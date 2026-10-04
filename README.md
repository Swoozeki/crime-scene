# csi — Crime Scene Investigator

Behavioral code analysis from git history, inspired by Adam Tornhill's *Your Code as a Crime Scene*,
rebuilt to fix the method's known weak spots and Code Maat's clunky pipeline.

**One binary. One command. Local only.**

```
csi init ~/work          # find the repos, write crimescene.toml
csi scan                 # ingest history, parse code, x-ray the top hotspots (incremental)
csi report               # ranked findings with evidence and a recommendation each
csi serve --open         # interactive web app
```

## What it finds

| Finding | Meaning |
|---|---|
| **Hotspot** / **Deteriorating hotspot** | Changes often *and* is complex — and (if deteriorating) got >25% more complex in the last year |
| **Function hotspot** | The few complex functions inside a hotspot that absorb most changes (X-ray) |
| **Hidden coupling** | Code in different units — or different repos, via shared ticket IDs — that changes together |
| **Ripple effect** | Changing it usually means changing many other things |
| **Knowledge loss** | A hotspot mostly written by people who are no longer active |
| **Bus factor** | Units where one person holds most of the knowledge |
| **Coordination bottleneck** | Code changed by many people or teams |
| **Defect magnet** | A high share of changes are bug fixes |
| **Bloated component** | Angular component with too many injected deps / inputs / a complex template — and a hotspot |

## How it improves on Code Maat and the book's method

- **Clean data first.** Rename-aware history (`git log -M`), `.mailmap` + alias + email/name merging,
  bots and AI co-authors filtered, lock files / vendored / generated / minified files excluded,
  formatting sweeps and release/version-bump commits neutralized, mega-commits kept out of coupling.
- **Recency-weighted change frequency** (exponential decay, 180-day half-life by default), so last
  year's fire counts more than a 2019 refactor.
- **Real complexity.** tree-sitter cyclomatic complexity and nesting per function for TypeScript,
  JavaScript, Angular templates, SCSS/CSS and PHP; indentation complexity (Tornhill) as fallback.
- **Function-level X-ray** from `git log -p -U0` hunks mapped onto function ranges of each version.
- **Statistically sound coupling**: support, confidence both ways, and lift (instead of a raw %).
  By commit (= PR with squash merges) or **by ticket ID across repositories**.
- **Logical entities.** `cart.component.{ts,html,scss,spec.ts}` is one component (stem grouping plus
  `templateUrl`/`styleUrls` links), so trivial template↔class coupling doesn't drown real signals.
- **Architecture-aware.** Units come from Angular workspaces, Nx `project.json` (with tags), module
  federation / native federation configs (micro-frontends: host/remote), NgModules and route files,
  Node packages, Laravel and Symfony layers, plus your own globs.
- **Trends that mean something:** complexity now vs. a year ago, not vs. a file's first commit.
- **Incremental** SQLite cache keyed by commit and blob sha; re-scans take seconds; force-pushes are detected.
- **Where you work:** a ranked report, a PR risk check for CI, an MCP server for your coding agent,
  a local web app and a single-file HTML export.

## Commands

```
csi init [dir]                    discover git repos under dir, write crimescene.toml
csi scan [--full] [--repo X] [--shallow]
csi report [--top 20] [--kind hotspot] [--repo X] [--unit U]
csi hotspots [--level file|entity|unit|repo] [-n 25] [--repo X] [--unit U] [--path P] [--days N]
csi coupling [path] [--by commit|ticket] [--cross] [--level ...]
csi xray <file>                   function-level hotspots
csi trend <file>                  complexity over time
csi owners [path] [--level ...]   ownership, bus factor, knowledge loss, experts
csi health <file>                 code health and the reasons
csi show <file>                   everything about the file's component
csi diff [base..head] [--ticket ABC-123] [--repo X] [-f md|json] [--fail-above 70]
csi serve [--port 7777] [--open]  local web app
csi export --html report.html     self-contained report to share
csi mcp                           MCP server (stdio) for coding agents
```

Global flags: `--config <file>`, `-f table|json|csv|md`, `--no-scan`, `-q`.
Read commands rescan incrementally when a repo's analyzed branch moved.

Inside a single repo, no config is needed: `csi report` just works (cache in `~/Library/Caches/csi`).

## Multi-repo workspaces

`csi init ~/work` writes:

```toml
[workspace]
name = "work"

[[repo]]
name = "shell"
path = "shell-app"
# branch = "main"     # default: origin's default branch, else main/master, else HEAD

[[repo]]
name = "orders-api"
path = "services/orders-api"
```

Cross-repo coupling needs ticket IDs in commit messages (default pattern `ABC-123`; configure
`[tickets].pattern`). With squash merges, put the ticket in the PR title.

The analyzed branch is the remote's default branch, so **`git fetch` your repos** before scanning to
see the latest history. csi itself never touches the network.

## Configuration

All keys are optional — see the generated `crimescene.toml`:

| Key | Default | |
|---|---|---|
| `analysis.since` | `3 years` | history window (`all` for everything) |
| `analysis.half_life_days` | 180 | recency decay |
| `analysis.max_commit_files` | 60 | larger commits are excluded from coupling, weighted 0.25 |
| `analysis.coupling_min_support` / `_confidence` / `_lift` | 5 / 0.3 / 1.5 | coupling thresholds |
| `analysis.inactive_after_days` | 180 | knowledge-loss cut-off |
| `analysis.top_n_xray` | 50 | files that get X-ray + trend sampling on scan |
| `exclude.globs` | | added to built-in excludes |
| `tickets.pattern` / `defects.pattern` | | regexes over commit messages |
| `authors.bots` / `authors.aliases` | | bot regexes; canonical name → aliases |
| `teams` | | team → members, enables coordination-by-team |
| `[[architecture.unit]]` | | name/glob/kind units that override detection |

## Using it with Claude Code (MCP)

```
claude mcp add csi -- csi mcp --config ~/work/crimescene.toml
```

Tools: `file_context` (call before editing a file: hotspot rank, health, riskiest functions,
co-changing files incl. other repos, experts), `hotspots`, `coupling`, `xray`, `experts`,
`review_diff`, `findings`. All computed locally from the cache.

## CI: PR risk

```
csi diff origin/main...HEAD -f md > risk.md      # post as a PR comment
csi diff origin/main...HEAD --fail-above 80      # exit 2 on very risky changes
```

CI needs full history (`fetch-depth: 0`). The cache (`.csi/`) can be restored between runs to keep it fast.

## Scoring, briefly

- **Hotspot score** = log-scaled recency-weighted change frequency (relative to the busiest code)
  × complexity percentile within its language × language weight (TS/JS/PHP 1.0, templates 0.8,
  styles 0.5, config 0.3, docs 0.15). Test-only code counts half.
- **Code health** (1–10) starts at 10 and subtracts documented penalties: functions over cc 15, over
  80 lines, nesting over 4, files over 600/1500 lines, >7 injected deps, >10 inputs, template
  complexity over 25, >20 methods per PHP class. Each penalty becomes a reason you can read.
- **Coupling** keeps pairs with support ≥ 5, max confidence ≥ 0.3 and lift ≥ 1.5.

## Build from source

```
pnpm -C ui install && pnpm -C ui build     # the UI is embedded into the binary
cargo build --release                      # → target/release/csi
cargo test --workspace
```

Layout: `crates/csi-core` (config, cache), `csi-ingest` (git), `csi-lang` (tree-sitter),
`csi-arch` (architecture plugins), `csi-analysis` (all analyses), `csi-server` (web), `csi-mcp`,
`csi-cli`; `ui/` (Svelte + D3).
