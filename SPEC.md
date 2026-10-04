# csi — Crime Scene Investigator

A behavioral code analysis tool inspired by *Your Code as a Crime Scene* (Adam Tornhill), rebuilt to fix
the weaknesses of the method and of Code Maat. Written from scratch (no Code Maat code; it is GPL-3).

Primary target: a large, multi-repo Angular estate (shell + micro-frontends + PHP/Node microservices) with
squash merges and ticket IDs in commit messages. Must also work well on any personal repo with zero config.

---

## 1. Principles

1. **Behavior first, structure second.** Git history is the primary signal; code parsing ranks and explains it.
2. **Clean data or nothing.** Renames, bots, aliases, generated files and mega-commits are handled before any metric.
3. **Logical entities, not files.** An Angular component is `.ts + .html + .scss + .spec.ts`. Analyses run on
   files, logical entities, and architectural units (MFE, lib, service, repo) — user picks the level.
4. **Actionable over pretty.** The headline output is a ranked list of findings with evidence and a recommendation.
5. **Workspace = many repos.** Cross-repo coupling via ticket IDs is first-class, not an afterthought.
6. **Local only.** No outbound network calls, ever. AI integration is via an MCP server the user's own agent queries.
7. **Incremental and fast.** One binary, SQLite cache, re-scans in seconds.

## 2. Stack

| Concern | Choice | Why |
|---|---|---|
| Language | Rust (stable, 2024 edition) | Performance, single binary, native tree-sitter |
| Git history | `git log` subprocess, streamed | git's rename detection (`-M`) is the most mature; streaming keeps memory flat |
| Historical blobs | long-lived `git cat-file --batch` subprocess | fast, no gitoxide dependency/compile weight |
| Parsing | tree-sitter: typescript, tsx, javascript, angular (templates), html, scss, css, php | per-function complexity |
| Parallelism | rayon | parse files across cores; repos ingested in parallel |
| Storage | rusqlite (bundled), WAL | single cache file per workspace |
| CLI | clap (derive) | |
| Web server | axum + tokio, UI embedded via rust-embed | `csi serve` |
| MCP | rmcp (stdio transport) | `csi mcp` |
| UI | Svelte 5 + Vite + D3 v7, TypeScript | one UI codebase for both served app and single-file static export (vite-plugin-singlefile) |
| Terminal output | comfy-table + owo-colors | |

Prereqs: Rust toolchain (rustup), Node ≥ 20 + pnpm (UI build only; the built UI is embedded in the binary).

## 3. Repository layout (`~/crime-scene`)

```
Cargo.toml                 # workspace
crates/
  csi-core/                # config, domain model, DB schema + migrations, time/decay utils
  csi-ingest/              # git log parser, rename map, identity resolution, filters, incremental sync
  csi-lang/                # tree-sitter complexity per language, indentation fallback, function ranges
  csi-arch/                # architecture model: plugins (angular, nx, module-federation, node, php) + manual rules
  csi-analysis/            # hotspots, coupling, xray, trends, social, age, defects, health, findings, diff-risk
  csi-server/              # axum JSON API + embedded UI
  csi-mcp/                 # MCP tools over csi-analysis
  csi-cli/                 # binary `csi`
ui/                        # Svelte app -> ui/dist embedded into csi-server
tests/fixtures/            # scripted synthetic repos (built at test time)
SPEC.md README.md
```

## 4. Workspace & configuration

`csi init [dir]` discovers git repos under `dir` (depth ≤ 3) and writes `crimescene.toml`. Cache lives at
`.csi/cache.db` next to the config. Running `csi` inside a single repo with no config uses an implicit
one-repo workspace with defaults (cache under `~/.cache/csi/<hash>`).

```toml
[workspace]
name = "work"

[[repo]]
name = "shell"
path = "../shell-app"
# branch = "main"            # default: the remote's default branch, else HEAD

[analysis]
since = "3 years"              # history window
half_life_days = 180           # recency decay for change frequency
max_commit_files = 60          # commits touching more files are "mega" (see 5.4)
coupling_min_support = 5       # min shared changes
coupling_min_confidence = 0.3
inactive_after_days = 180      # author considered departed after this
top_n_xray = 50                # files that get function-level X-ray
trend_samples = 12

[exclude]
globs = ["**/package-lock.json", "**/pnpm-lock.yaml", "**/composer.lock", "**/*.min.js",
         "**/dist/**", "**/vendor/**", "**/node_modules/**", "**/*.generated.*", "**/*.snap"]
                               # defaults always on; this list extends them

[tickets]
pattern = '\b[A-Z][A-Z0-9]+-\d+\b'
[defects]
pattern = '(?i)\b(fix(es|ed)?|bug|hotfix|defect|regression)\b'

[authors]
bots = ['(?i)\[bot\]', '(?i)dependabot', '(?i)renovate']
[authors.aliases]               # canonical = [aliases...]; .mailmap is also honored
"Jane Doe" = ["jdoe@old-corp.com", "Jane D"]

[teams]                         # optional, enables Conway analysis
payments = ["Jane Doe", "Raj K"]

[[architecture.unit]]           # optional manual units; plugins add auto-detected ones
name = "checkout-mfe"
repo = "shell"
glob = "apps/checkout/**"
kind = "mfe"                    # mfe | app | lib | service | module | layer
```

## 5. Ingest (csi-ingest)

### 5.1 History extraction
Per repo, stream:
```
git log <since-or-last>..<branch> --no-merges -M --numstat --date=unix
        --format=%x1e%H%x1f%P%x1f%an%x1f%ae%x1f%at%x1f%cn%x1f%ce%x1f%B
```
Squash merges mean each commit ≈ one PR; `--no-merges` drops merge commits (squash produces none, but
personal repos may). Parse `Co-authored-by:` trailers as additional authors (weight split).
Binary files (`-\t-`) recorded with zero churn. Numstat rename syntax (`a/{x => y}/b`, `old => new`) parsed.

### 5.2 Rename tracking
Walk commits oldest→newest maintaining `path → file_id`. A rename re-points the new path to the same file_id.
Every file_id resolves to its **current** path (or last known path if deleted; deleted files are kept but
excluded from current-state views).

### 5.3 Identity resolution
Order: `.mailmap` (via `git check-mailmap` batch) → config aliases → automatic merge of identities sharing an
email, or same normalized name (case/diacritics/whitespace). Bots flagged by regex; bot commits are stored but
excluded from all analyses by default.

### 5.4 Noise handling
- Excluded files (default + config globs, plus auto-detected generated: header markers like `@generated`,
  `DO NOT EDIT`; checked when parsing current tree).
- Mega-commits (> `max_commit_files`): kept for revisions with weight 0.25, **excluded** from coupling.
- Coupling weight per commit = 1 / log2(n_files + 1) (diminishes broad commits smoothly).
- Whitespace/format-only commits: detected cheaply when churn is high and message matches
  `(?i)format|prettier|lint|eslint --fix|reformat`; weight 0 for revisions/coupling.
- Ticket IDs extracted from message → `commit_tickets`. Defect flag from message regex.

### 5.5 Incremental sync
Store `last_sha` per repo. On scan: if `last_sha` is an ancestor of branch head → ingest `last_sha..head`;
otherwise (force-push/rebase) re-ingest that repo fully. Derived metrics are recomputed (cheap); per-blob
complexity results cached by blob SHA so unchanged files are never re-parsed.

## 6. Language layer (csi-lang)

For each supported file: list of functions `{name, kind, start_line, end_line, cyclomatic, cognitive_nesting_max, loc}`
plus file totals.

| Lang | Grammar | Functions | Decision points |
|---|---|---|---|
| TS/JS/TSX | typescript, tsx, javascript | function decl/expr, arrow fns assigned to names, methods, class property arrows | if, else-if, ternary, `&&`/`||`/`??` in conditions, case, for/while/do, catch, optional chaining excluded |
| Angular template | angular | whole template as one unit + each `@if/@for/@switch/@defer` block | `@if`, `@else if`, `@for`, `@case`, `*ngIf`, `*ngFor`, `[ngSwitch]`, ternaries/pipes in bindings |
| PHP | php | functions, methods, closures | if/elseif, case, loops, catch, ternary, `&&`/`||`/`??`, match arms |
| SCSS/CSS | scss, css | rule blocks | complexity = max nesting depth + selector count (scored lower weight) |
| Other text | — | none | indentation complexity (Tornhill): sum of logical indents, tabs=4 |

Complexity used for hotspots = sum of cyclomatic over functions (or indentation complexity fallback),
normalized per language percentile so SCSS doesn't compete with TS on raw numbers.

**Function changes over history (X-ray):** for each X-ray file, `git log -M --follow -U0 -p` on that path;
for each commit, parse the post-image blob (cached by blob sha) to get function ranges; map changed hunk
lines to function names; deleted-line hunks map via pre-image ranges. Result: per-function revisions,
recency-weighted revisions, churn, last change, authors, defect commits.

## 7. Architecture model (csi-arch)

Entity hierarchy: `file → logical entity → unit → repo → workspace`.

Plugins auto-detect per repo; all are pure functions over the current tree:

- **angular**: `angular.json` projects → units (app/lib). Component grouping: files sharing the stem
  `x.component|directive|pipe|service|guard|resolver|interceptor|store|effects|reducer|selectors|facade`
  across `.ts/.html/.scss/.css/.spec.ts` → one logical entity named `x.component` etc.
  Template linkage also via `templateUrl`/`styleUrl(s)` parsing when names differ.
  Collects: injected deps count (constructor params + `inject()` calls), `@Input/@Output/input()/output()`
  counts, standalone vs NgModule, lazy routes (`loadChildren`/`loadComponent` targets).
- **nx**: `nx.json` + `project.json` → units with `tags`; boundaries from tags.
- **module-federation**: `module-federation.config.*`, `federation.config.*`, `webpack.config.*` with
  `ModuleFederationPlugin`, `@angular-architects/native-federation` → unit kind `mfe` (remote/host) and
  exposed modules.
- **node**: `package.json` (workspaces) → units.
- **php**: `composer.json` → unit; Laravel (`app/Http/Controllers`, `app/Models`…) and Symfony (`src/Controller`…)
  layers → units of kind `layer`; PHP class = logical entity (one class per file assumption).
- **manual**: config `[[architecture.unit]]` globs override/extend.

Fallback when nothing detected: top-level directories (depth configurable) become units.

## 8. Analyses (csi-analysis)

All analyses take a scope `{repos?, unit?, path_prefix?, level: file|entity|unit|repo, since?, until?}`.

### 8.1 Hotspots
- `rev_w` = Σ commit_weight · 2^(−age_days / half_life_days)
- `complexity` = language-normalized complexity of current version
- `score` = pct_rank(rev_w) · pct_rank(complexity) → 0..1, plus raw values. Also churn (added+deleted, decayed).
- Aggregation to entity/unit = sum of rev_w (deduped per commit), sum of complexity.

### 8.2 Change coupling
- Change sets: **commit** (default, = PR under squash) and **ticket** (all commits sharing a ticket ID,
  across all repos — cross-repo coupling). Fallback grouping for ticketless commits: same author, same
  repo set, within 4h (only used for cross-repo).
- For pair (A,B): `shared` (weighted), `support`, `conf(A→B)`, `conf(B→A)`, `lift = P(A∧B)/(P(A)P(B))`,
  `jaccard`. Keep pairs with support ≥ min_support, max conf ≥ min_confidence, lift > 1.5.
- Pair generation capped: skip change sets > max_commit_files (they're excluded anyway).
- Flags: `cross_unit`, `cross_repo`, `expected` (same logical entity — hidden at entity level), `test_pair`
  (src↔spec — reported separately as "test coupling", not as a smell).
- Sum of coupling (SOC) per entity.
- Coupling trend: compare last 6 months vs before → strengthening/weakening.

### 8.3 X-ray
Function-level hotspots inside the top `top_n_xray` files (and on demand for any file). Also intra-file
function coupling (functions changed in the same commits).

### 8.4 Complexity trends
For top hotspots (and on demand): sample `trend_samples` commits evenly over the window, fetch blob via
cat-file, compute total complexity, LOC, max function complexity. Slope classification:
`deteriorating | stable | improving`. Also a "refactoring detected" marker when complexity drops > 20%.

### 8.5 Social
- Ownership per entity: share of decayed added lines per author; `main_dev`, `main_dev_share`.
- Fragmentation (Tornhill fractal value): 1 − Σ(share²).
- Bus factor per unit/repo: minimum number of authors covering > 50% of knowledge (added lines of
  current-surviving files, decayed).
- Knowledge loss: share of knowledge owned by authors inactive > `inactive_after_days`; per entity and unit.
- Teams (if configured): team ownership; Conway check = entities/units with significant changes from ≥ 3
  teams (coordination bottleneck); cross-team coupling pairs.
- Experts lookup: for a path/entity, ranked active authors by recent knowledge.

### 8.6 Code age
Months since last change per entity; age distribution per unit.

### 8.7 Defects
Defect-flagged commits per entity (decayed), defect density = defect revs / total revs; combined in findings.

### 8.8 Code health (lite)
Per file, 1–10 (10 = healthy), from penalties: functions with cyclomatic > 15 (scaled), max nesting > 4,
functions > 80 LOC, file > 600 LOC, Angular: > 7 injected deps, > 10 inputs, template complexity > 25;
PHP: class > 20 methods. Explicit, documented, configurable thresholds; each penalty becomes a reason string.

### 8.9 Findings engine (the headline)
Deterministic rules generate findings `{id, kind, severity (0–100), title, entities, evidence{metrics}, recommendation}`:

| Kind | Trigger (defaults) |
|---|---|
| `hotspot` | top 2% by score and health ≤ 6 |
| `deteriorating_hotspot` | hotspot with deteriorating trend |
| `xray_hotspot` | function with high rev_w and cyclomatic > 15 inside a hotspot |
| `hidden_coupling` | coupled pair cross_unit or cross_repo, conf ≥ 0.5, not src↔spec |
| `shotgun_surgery` | entity with SOC in top 2% |
| `knowledge_loss` | hotspot/unit with ≥ 50% knowledge by inactive authors |
| `bus_factor` | unit with bus factor 1 and recent activity |
| `coordination` | entity changed by ≥ 3 teams (if teams configured), or ≥ 8 active authors |
| `defect_magnet` | defect density ≥ 0.4 with ≥ 5 defect revs |
| `bloated_component` | Angular entity with deps/inputs/template thresholds breached and is a hotspot |

Severity = base weight × score percentile. Recommendations are templated text referencing the evidence
(e.g., "Split `OrderService.calculate()` (CC 41, changed 38× in 6mo)"). Output top N (default 20).

### 8.10 Diff risk (`csi diff`)
Input: repo + `base..head` (default `origin/<default>...HEAD`), or a ticket ID (collects commits across repos).
Output:
- touched hotspots (+ functions touched via hunk→function map)
- complexity delta of touched functions (base vs head blobs), health delta
- **likely missed changes**: files with conf(touched→X) ≥ 0.6 and support ≥ 5 not in diff, including other repos
  (via ticket coupling)
- suggested reviewers: top active experts of touched entities, excluding the author
- risk score 0–100 with reasons
Formats: terminal, markdown (for PR comments), json. Exit code `--fail-above <score>` for CI.

## 9. Storage (csi-core)

SQLite, WAL, schema versioned with embedded migrations. Tables:
`repos, authors, author_aliases, commits(id, repo_id, sha, author_id, ts, msg_subject, n_files, weight,
is_defect, is_bot, is_format), commit_coauthors, commit_tickets(commit_id, ticket), files(id, repo_id,
current_path, deleted, excluded_reason, lang), file_paths(file_id, path, from_commit), changes(commit_id,
file_id, added, deleted), blob_metrics(blob_sha, lang, json), file_metrics(file_id, blob_sha, complexity,
loc, health, json), units(id, repo_id, name, kind, json), entities(id, repo_id, unit_id, name, kind),
entity_files(entity_id, file_id), function_changes(file_id, commit_id, fn_name, added, deleted),
trend_points(file_id, commit_id, ts, complexity, loc, max_fn), findings_cache(json), meta(key, value)`.
Indexes on changes(file_id), changes(commit_id), commits(repo_id, ts), commit_tickets(ticket).

Analyses query SQLite into in-memory structures; coupling uses hashmap of (min_id,max_id) → accumulators.

## 10. Interfaces

### 10.1 CLI (`csi`)
```
csi init [dir]                     discover repos, write crimescene.toml
csi scan [--full] [--repo X]       ingest + parse + derive (incremental)
csi report [--top 20]              ranked findings (terminal)
csi hotspots [--level entity] [--unit X] [--limit 30]
csi coupling [path] [--cross-repo] [--by ticket|commit]
csi xray <path>
csi trend <path>
csi owners <path|unit>             ownership, experts, knowledge loss
csi health <path>
csi diff [base..head] [--ticket ABC-123] [--format md|json|text] [--fail-above 70]
csi serve [--port 7777] [--open]   local web app
csi export --html report.html      single-file shareable report (data baked in)
csi mcp                            MCP server on stdio
```
Global: `--config`, `--format table|json|csv`, `--since`, `--quiet`. All read commands auto-run an incremental
scan if the cache is older than the repo heads (skip with `--no-scan`). Progress bars via indicatif on stderr.

### 10.2 MCP tools (`csi mcp`)
Designed for a coding agent working in one of the repos (repo inferred from cwd/path):
- `file_context(path)` → hotspot rank/score, health + reasons, coupled files (incl. cross-repo) with
  confidence, top functions by change, experts, defect density, recent tickets touching it. **Primary tool.**
- `hotspots(scope, level, limit)`
- `coupling(path, by)`
- `xray(path)`
- `experts(path)`
- `review_diff(base, head | ticket)` → same as `csi diff --format json`
- `findings(limit, kind?)`
Tool descriptions instruct: call `file_context` before modifying a file in a hotspot-heavy area.
README includes the `claude mcp add csi -- csi mcp --config ...` line.

### 10.3 Web app (`csi serve`) and static export
Svelte SPA, light/dark, URL-routed, filters (repo, unit, time window, level) persisted in URL.
Views:
1. **Overview** — KPIs (hotspots, knowledge-loss %, bus factor 1 units, hidden coupling count), findings list
   with expandable evidence.
2. **Hotspot map** — zoomable D3 circle packing: workspace → repo → unit → dir → entity; size = LOC,
   color = hotspot score (sequential), toggle color by health / age / main dev / knowledge loss.
3. **Architecture** — units as nodes (grouped by repo), edges = coupling between units (width = shared changes),
   cross-repo edges highlighted.
4. **Coupling** — sortable table + per-entity ego graph; filter by cross-unit/cross-repo/ticket.
5. **Entity detail** — metrics, health reasons, trend chart, X-ray table/bar (functions), owners donut,
   coupled entities, commit timeline.
6. **Knowledge** — knowledge map (circle packing colored by main dev/team), bus factor table, knowledge loss.
7. **Diff** — paste/select range, renders the diff-risk report.
JSON API: `/api/{overview,findings,hotspots,coupling,entity/:id,xray,trend,owners,units,diff}`. The static export
inlines responses for all views except diff and on-demand xray/trend beyond precomputed ones.

## 11. Performance targets
- Initial ingest: ≥ 20k commits/s parse rate (git is the bottleneck); 100k-commit repo < 60s.
- Incremental re-scan with no new commits: < 1s; with 50 new commits: < 3s.
- Current-tree parse: 20k files < 20s on 8 cores; cached by blob SHA afterwards.
- Analyses for workspace of 5 repos / 200k commits: report < 3s.
- Memory: < 1 GB for the above.

## 12. Testing & verification
- **Unit**: numstat/rename parsing, identity merging, decay math, coupling metrics, function extraction per
  language (fixtures of real TS/Angular/PHP snippets), Angular grouping, MF detection.
- **Integration**: fixture builder creates temp git repos from a scripted DSL (author, date, files, contents,
  message) — scenarios: renames, squash/ticket coupling across two repos, bots/aliases, mega-commit,
  inactive author, deteriorating file. Golden assertions on findings.
- **Real-world smoke**: run on public repos covering the target shapes — an Angular/Nx repo
  (e.g. nrwl/nx-examples), a Laravel app, a Node service, plus a 3-repo workspace — check timing and sanity.
- **UI**: Playwright smoke test of each view against a fixture workspace; light + dark.
- `cargo clippy -D warnings`, `cargo test`, `pnpm build` must be green.

## 13. Build order (one-shot execution)
1. Scaffold workspace, core config + schema, fixture builder.
2. Ingest (parse, renames, identities, filters, incremental) + tests.
3. Language layer (all grammars, complexity, function ranges) + tests.
4. Architecture plugins + tests.
5. Analyses: hotspots → coupling → social/age/defects → health → X-ray → trends → findings → diff + tests.
6. CLI commands with table/json/csv output.
7. MCP server.
8. UI + server API + static export.
9. Real-world smoke runs, perf tuning, README.

## 14. Explicit non-goals (v1)
Duplication detection, IDE plugin, hosted/multi-user server, issue-tracker API integration (ticket types),
LLM calls of any kind, languages beyond TS/JS/Angular/HTML/SCSS/CSS/PHP (fallback still covers them).

---

## 15. Implementation notes (as built, after calibration on real repos)

Calibrated against koel (Laravel + Vue, 3.8k commits), ngrx/platform (Angular Nx monorepo, 2.3k
commits) and angular/angular (10.5k commits in the 3-year window). Changes from the plan above:

- **Hotspot score** uses log-scaled frequency relative to the busiest *code* key (not a percentile):
  percentiles flattened 5 and 80 changes into the same bucket. Test-only keys count half.
- **Trends** compare complexity now vs. ~one year ago (denser sampling in the last year).
  Comparing against a file's first version marked every growing file as "deteriorating".
- **Release / version-bump commits** (`chore: release`, `v1.2.3`, …) are neutralized like formatting
  sweeps — they created fake coupling between every `package.json` in a monorepo.
- **AI co-authors** (`Co-authored-by: Claude/Copilot/…`) are bots; bot co-authors are dropped
  without dropping the commit.
- **Defect magnets** are relative to the repo's own fix rate (≥ 1.5×) — conventional-commit repos
  label most changes `fix:`.
- **Hidden coupling** findings are clustered (connected components) so N copies of one duplicated
  file produce one finding; coupling/ripple findings require code (not config) on both sides.
- **X-ray ranking**: frequency-led, scaled by complexity up to cc 15.
- **Default branch** without a remote: `main`/`master`/`develop`/`trunk` before `HEAD`, so checking
  out a feature branch doesn't change what is analyzed.
- **Ticketless cross-repo sessions** (§8.2 fallback): one author's ticketless commits starting within
  `session_hours` (4h) of the session's first commit form one change set when they span ≥ 2 repos.
- **Clutter**: built-in excludes also cover more build caches (`.next`, `.turbo`, `out`, …), third-party
  dirs, generated code (`generated/`, `*.gen.ts`, `*.pb.ts`), translations (`*.xlf`, `*.po`) and
  changelogs; each repo's `.gitattributes` `linguist-generated` / `linguist-vendored` patterns are
  excluded too. Dependency-bump, "prepare … release" and framework-upgrade commits are neutralized
  like releases. `spec/`, `__mocks__/`, `fixtures/` and `*.mock.*` count as test code. Coupling hides
  pairs involving config/data files by default (`--config-files`).
- **Test noise**: coupling pairs involving test-only code are hidden by default (`--tests`); source↔test
  pairs are matched by stem even when tests live in a separate `spec/` tree. Ownership lists rank by
  hotspot score with tests last. X-ray drops one-line anonymous fragments.
- **Static export** bakes in all list views plus detail pages for the top ~60 entities and every
  entity named in a finding; change-risk needs the live app.

Measured: angular/angular, 10.5k commits / 10k files, first scan 40 s (git log itself ≈ 18 s),
no-change rescan ≈ 2 s, `csi report` < 1 s, peak memory ≈ 500 MB.
