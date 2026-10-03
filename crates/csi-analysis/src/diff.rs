//! PR / branch risk: touched hotspots, complexity deltas, likely-missed coupled changes,
//! suggested reviewers. Works on a git range or on a ticket ID across repositories.

use crate::coupling::{By, CouplingQuery, coupling};
use crate::dataset::Dataset;
use crate::hotspots::{hotspots, index};
use crate::scope::{Level, Scope};
use crate::social::{Expert, experts, ownership};
use anyhow::{Result, bail};
use csi_core::Db;
use csi_ingest::metrics::metrics_for;
use csi_ingest::{CatFile, Git};
use csi_lang::Function;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct DiffRequest {
    pub repo: Option<String>,
    pub base: Option<String>,
    pub head: Option<String>,
    pub ticket: Option<String>,
    /// Used to infer the repo when `repo` is not given.
    pub cwd: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiffFn {
    pub name: String,
    pub cc_before: Option<u32>,
    pub cc_after: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiffFile {
    pub path: String,
    pub repo: String,
    pub added: u32,
    pub deleted: u32,
    pub status: String,
    pub hotspot_rank: Option<usize>,
    pub hotspot_score: f64,
    pub complexity_before: Option<f64>,
    pub complexity_after: Option<f64>,
    pub health_before: Option<f64>,
    pub health_after: Option<f64>,
    pub functions: Vec<DiffFn>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MissedChange {
    pub path: String,
    pub repo: String,
    pub because_of: String,
    pub confidence: f64,
    pub support: u32,
    pub kind: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiffReport {
    pub title: String,
    pub repos: Vec<String>,
    pub base: Option<String>,
    pub head: Option<String>,
    pub authors: Vec<String>,
    pub files: Vec<DiffFile>,
    pub missed: Vec<MissedChange>,
    pub reviewers: Vec<Expert>,
    pub risk: u32,
    pub level: String,
    pub reasons: Vec<String>,
}

pub fn diff(ds: &Dataset, db: &Db, req: &DiffRequest) -> Result<DiffReport> {
    if let Some(t) = &req.ticket {
        return ticket_diff(ds, t);
    }
    let repo = pick_repo(ds, req)?;
    let r = &ds.repos[repo as usize];
    let git = Git::new(&r.path);
    let head_ref = req.head.clone().unwrap_or_else(|| "HEAD".into());
    let base_ref = match &req.base {
        Some(b) => b.clone(),
        None => {
            let b = if r.branch.is_empty() { "HEAD".to_string() } else { r.branch.clone() };
            git.run(&["merge-base", &b, &head_ref])?.trim().to_string()
        }
    };
    let base = git.rev_parse(&base_ref)?;
    let head = git.rev_parse(&head_ref)?;
    if base == head {
        bail!("nothing to compare: {base_ref} and {head_ref} are the same commit (pass a base, e.g. `csi diff main..HEAD`)");
    }
    let numstat = git.run(&["diff", "--numstat", "-M", &base, &head])?;
    let patch = git.run(&["diff", "-U0", "--full-index", "-M", &base, &head])?;
    let hunks = parse_patch(&patch);
    let file_hot = hotspots(ds, Level::File, &Scope::default());
    let hot_idx = index(&file_hot);
    let mut cat = CatFile::new(&git)?;
    let max = ds.cfg.analysis.max_file_bytes;

    let mut files = vec![];
    let mut touched: Vec<u32> = vec![];
    for line in numstat.lines() {
        let mut it = line.splitn(3, '\t');
        let (a, d, p) = (it.next().unwrap_or("0"), it.next().unwrap_or("0"), it.next().unwrap_or(""));
        let (old, path) = csi_ingest::log::split_rename(p);
        let status = if old.is_some() { "renamed" } else { "modified" };
        let fidx = ds.file_by_path.get(&(repo, old.clone().unwrap_or_else(|| path.clone()))).copied();
        if let Some(f) = fidx {
            touched.push(f);
        }
        let h = fidx.and_then(|f| hot_idx.get(&f)).map(|&i| &file_hot[i]);
        let before = metrics_for(db, &mut cat, &format!("{base}:{}", old.as_deref().unwrap_or(&path)), &path, max)?;
        let after = metrics_for(db, &mut cat, &format!("{head}:{path}"), &path, max)?;
        let status = match (&before, &after) {
            (None, Some(_)) => "added",
            (Some(_), None) => "deleted",
            _ => status,
        };
        let fns_before: HashMap<&str, &Function> =
            before.as_ref().map(|(_, m)| m.functions.iter().map(|f| (f.name.as_str(), f)).collect()).unwrap_or_default();
        let mut functions = vec![];
        if let (Some((_, m)), Some(hs)) = (&after, hunks.get(&path)) {
            let mut names: Vec<&str> = vec![];
            for &(start, count) in hs {
                for l in start..start + count.max(1) {
                    if let Some(f) = csi_lang::function_at(&m.functions, l) {
                        if !names.contains(&f.name.as_str()) {
                            names.push(&f.name);
                        }
                    }
                }
            }
            for n in names {
                let after_cc = m.functions.iter().find(|f| f.name == n).map(|f| f.cc);
                functions.push(DiffFn { name: n.to_string(), cc_before: fns_before.get(n).map(|f| f.cc), cc_after: after_cc });
            }
        }
        files.push(DiffFile {
            path: path.clone(),
            repo: r.name.clone(),
            added: a.parse().unwrap_or(0),
            deleted: d.parse().unwrap_or(0),
            status: status.into(),
            hotspot_rank: h.map(|h| h.rank),
            hotspot_score: h.map(|h| h.score).unwrap_or(0.0),
            complexity_before: before.as_ref().map(|(_, m)| m.complexity),
            complexity_after: after.as_ref().map(|(_, m)| m.complexity),
            health_before: before.as_ref().map(|(_, m)| m.summary.health),
            health_after: after.as_ref().map(|(_, m)| m.summary.health),
            functions,
        });
    }

    // authors of the change
    let log = git.run(&["log", "--format=%aN%x1f%aE", &format!("{base}..{head}")])?;
    let mut author_names: Vec<String> = vec![];
    let mut author_ids: Vec<u32> = vec![];
    for l in log.lines() {
        let (name, email) = l.split_once('\x1f').unwrap_or((l, ""));
        if !author_names.iter().any(|n| n == name) {
            author_names.push(name.to_string());
        }
        let email = email.to_lowercase();
        if let Some(a) = ds.authors.iter().position(|a| a.emails.contains(&email) || a.name == name) {
            author_ids.push(a as u32);
        }
    }
    let changed: HashSet<String> = files.iter().map(|f| f.path.clone()).collect();
    let missed = missed_changes(ds, &touched, |f| ds.files[f as usize].repo == repo && changed.contains(&ds.files[f as usize].path));
    let reviewers: Vec<Expert> = experts(ds, &touched, &author_ids).into_iter().take(5).collect();
    let (risk, reasons) = risk(ds, &files, &missed, &touched, &author_ids);
    Ok(DiffReport {
        title: format!("{} {}..{}", r.name, short(&base), short(&head)),
        repos: vec![r.name.clone()],
        base: Some(base),
        head: Some(head),
        authors: author_names,
        files,
        missed,
        reviewers,
        level: level(risk).into(),
        risk,
        reasons,
    })
}

fn ticket_diff(ds: &Dataset, ticket: &str) -> Result<DiffReport> {
    let Some(t) = ds.tickets.iter().position(|x| x.eq_ignore_ascii_case(ticket)) else {
        bail!("no commits mention ticket {ticket}");
    };
    let file_hot = hotspots(ds, Level::File, &Scope::default());
    let hot_idx = index(&file_hot);
    let mut per_file: BTreeMap<u32, (u32, u32)> = BTreeMap::new();
    let mut authors: Vec<u32> = vec![];
    let mut repos: Vec<String> = vec![];
    for &ci in &ds.ticket_commits[t] {
        let c = &ds.commits[ci as usize];
        for a in c.authors() {
            if !authors.contains(&a) {
                authors.push(a);
            }
        }
        let rn = ds.repos[c.repo as usize].name.clone();
        if !repos.contains(&rn) {
            repos.push(rn);
        }
        for ch in &c.changes {
            let e = per_file.entry(ch.file).or_default();
            e.0 += ch.added;
            e.1 += ch.deleted;
        }
    }
    let touched: Vec<u32> = per_file.keys().copied().collect();
    let files: Vec<DiffFile> = per_file
        .iter()
        .map(|(&f, &(a, d))| {
            let file = &ds.files[f as usize];
            let h = hot_idx.get(&f).map(|&i| &file_hot[i]);
            DiffFile {
                path: file.path.clone(),
                repo: ds.repos[file.repo as usize].name.clone(),
                added: a,
                deleted: d,
                status: if file.alive { "modified" } else { "deleted" }.into(),
                hotspot_rank: h.map(|h| h.rank),
                hotspot_score: h.map(|h| h.score).unwrap_or(0.0),
                complexity_before: None,
                complexity_after: file.alive.then_some(file.complexity),
                health_before: None,
                health_after: file.alive.then(|| file.health()),
                functions: vec![],
            }
        })
        .collect();
    let set: HashSet<u32> = touched.iter().copied().collect();
    let missed = missed_changes(ds, &touched, |f| set.contains(&f));
    let reviewers = experts(ds, &touched, &authors).into_iter().take(5).collect();
    let (risk, reasons) = risk(ds, &files, &missed, &touched, &authors);
    Ok(DiffReport {
        title: format!("ticket {}", ds.tickets[t]),
        repos,
        base: None,
        head: None,
        authors: authors.iter().map(|&a| ds.authors[a as usize].name.clone()).collect(),
        files,
        missed,
        reviewers,
        level: level(risk).into(),
        risk,
        reasons,
    })
}

fn missed_changes(ds: &Dataset, touched: &[u32], in_change: impl Fn(u32) -> bool) -> Vec<MissedChange> {
    let touched_set: HashSet<u32> = touched.iter().copied().collect();
    let mut best: HashMap<u32, MissedChange> = HashMap::new();
    for by in [By::Commit, By::Ticket] {
        let q = CouplingQuery { level: Level::File, by, include_expected: true, min_lift: Some(1.0), ..Default::default() };
        for c in coupling(ds, &q, &Scope::default()) {
            for (src, dst, conf) in [(c.a, c.b, c.conf_ab), (c.b, c.a, c.conf_ba)] {
                if !touched_set.contains(&src) || in_change(dst) || conf < 0.6 || c.support < 5 {
                    continue;
                }
                let f = &ds.files[dst as usize];
                let kind = if c.test_pair {
                    "test"
                } else if c.cross_repo {
                    "cross-repo"
                } else if c.same_entity {
                    "same component"
                } else {
                    "coupled"
                };
                let m = MissedChange {
                    path: f.path.clone(),
                    repo: ds.repos[f.repo as usize].name.clone(),
                    because_of: ds.files[src as usize].path.clone(),
                    confidence: conf,
                    support: c.support,
                    kind: kind.into(),
                };
                if best.get(&dst).is_none_or(|b| b.confidence < conf) {
                    best.insert(dst, m);
                }
            }
        }
    }
    let mut v: Vec<MissedChange> = best.into_values().collect();
    v.sort_by(|a, b| b.confidence.total_cmp(&a.confidence).then(b.support.cmp(&a.support)));
    v.truncate(15);
    v
}

fn risk(ds: &Dataset, files: &[DiffFile], missed: &[MissedChange], touched: &[u32], authors: &[u32]) -> (u32, Vec<String>) {
    let mut score = 0.0f64;
    let mut reasons = vec![];
    let max_hot = files.iter().map(|f| f.hotspot_score).fold(0.0, f64::max);
    let hot: Vec<&DiffFile> = files.iter().filter(|f| f.hotspot_score >= 0.5).collect();
    if max_hot > 0.0 {
        score += 40.0 * max_hot;
    }
    if !hot.is_empty() {
        score += (hot.len() as f64 * 5.0).min(15.0);
        let names: Vec<String> = hot
            .iter()
            .take(3)
            .map(|f| format!("{} (#{})", f.path.rsplit('/').next().unwrap_or(&f.path), f.hotspot_rank.unwrap_or(0)))
            .collect();
        reasons.push(format!("touches {} hotspot file{}: {}", hot.len(), if hot.len() == 1 { "" } else { "s" }, names.join(", ")));
    }
    let delta: i64 = files
        .iter()
        .flat_map(|f| &f.functions)
        .map(|f| f.cc_after.unwrap_or(0) as i64 - f.cc_before.unwrap_or(0) as i64)
        .filter(|d| *d > 0)
        .sum();
    if delta > 0 {
        score += (delta as f64).min(15.0);
        reasons.push(format!("adds {delta} cyclomatic complexity to touched functions"));
    }
    let worsened: Vec<&DiffFile> = files
        .iter()
        .filter(|f| matches!((f.health_before, f.health_after), (Some(b), Some(a)) if a + 0.5 <= b))
        .collect();
    if !worsened.is_empty() {
        score += 5.0;
        reasons.push(format!("code health drops in {} file(s)", worsened.len()));
    }
    let real_missed = missed.iter().filter(|m| m.kind != "test").count();
    if !missed.is_empty() {
        score += (real_missed as f64 * 7.0 + (missed.len() - real_missed) as f64 * 3.0).min(20.0);
        reasons.push(format!("{} file(s) that usually change with these were not touched", missed.len()));
    }
    let lines: u32 = files.iter().map(|f| f.added + f.deleted).sum();
    if lines > 1500 {
        score += 15.0;
        reasons.push(format!("large change ({lines} lines)"));
    } else if lines > 500 {
        score += 10.0;
        reasons.push(format!("sizeable change ({lines} lines)"));
    }
    // author familiarity with the touched code
    if !authors.is_empty() && !touched.is_empty() {
        let known: f64 = touched
            .iter()
            .filter(|&&f| ds.files[f as usize].alive)
            .filter_map(|&f| {
                let scope = Scope { repo: Some(ds.repos[ds.files[f as usize].repo as usize].name.clone()), path: Some(ds.files[f as usize].path.clone()), days: None, unit: None };
                ownership(ds, Level::File, &scope).into_iter().find(|o| o.key == f).map(|o| {
                    o.authors.iter().filter(|a| authors.iter().any(|&x| ds.authors[x as usize].name == a.name)).map(|a| a.share).sum::<f64>()
                })
            })
            .fold(0.0, f64::max);
        if known < 0.1 && touched.iter().any(|&f| ds.files[f as usize].commits.len() >= 5) {
            score += 10.0;
            reasons.push("author(s) have little history with the touched code".into());
        }
    }
    if reasons.is_empty() {
        reasons.push("no risk signals: code outside hotspots, no missed coupled changes".into());
    }
    (score.clamp(0.0, 100.0).round() as u32, reasons)
}

fn level(risk: u32) -> &'static str {
    match risk {
        0..=29 => "low",
        30..=59 => "medium",
        _ => "high",
    }
}

fn short(sha: &str) -> &str {
    &sha[..sha.len().min(8)]
}

fn pick_repo(ds: &Dataset, req: &DiffRequest) -> Result<u32> {
    if let Some(n) = &req.repo {
        return ds
            .repos
            .iter()
            .position(|r| &r.name == n)
            .map(|i| i as u32)
            .ok_or_else(|| anyhow::anyhow!("unknown repo `{n}`"));
    }
    if ds.repos.len() == 1 {
        return Ok(0);
    }
    if let Some(cwd) = &req.cwd {
        let cwd = Path::new(cwd).canonicalize().unwrap_or_else(|_| cwd.into());
        if let Some(i) = ds.repos.iter().position(|r| cwd.starts_with(&r.path)) {
            return Ok(i as u32);
        }
    }
    bail!("several repos in this workspace; pass --repo <name> (or run inside the repo)")
}

/// New-side hunks per path: (start line, line count).
fn parse_patch(patch: &str) -> HashMap<String, Vec<(u32, u32)>> {
    let mut out: HashMap<String, Vec<(u32, u32)>> = HashMap::new();
    let mut cur: Option<String> = None;
    for l in patch.lines() {
        if let Some(p) = l.strip_prefix("+++ ") {
            cur = (p != "/dev/null").then(|| csi_ingest::log::unquote(p).trim_start_matches("b/").to_string());
        } else if l.starts_with("@@ ") {
            if let (Some(c), Some(new)) = (&cur, l.split_whitespace().nth(2)) {
                let new = new.trim_start_matches('+');
                let (s, n) = match new.split_once(',') {
                    Some((s, n)) => (s.parse().unwrap_or(0), n.parse().unwrap_or(0)),
                    None => (new.parse().unwrap_or(0), 1),
                };
                out.entry(c.clone()).or_default().push((s, n));
            }
        }
    }
    out
}

impl DiffReport {
    pub fn to_markdown(&self) -> String {
        let mut s = String::new();
        let icon = match self.level.as_str() {
            "high" => "🔴",
            "medium" => "🟠",
            _ => "🟢",
        };
        s.push_str(&format!("### {icon} Change risk {}/100 ({}) — {}\n\n", self.risk, self.level, self.title));
        for r in &self.reasons {
            s.push_str(&format!("- {r}\n"));
        }
        let hot: Vec<&DiffFile> = self.files.iter().filter(|f| f.hotspot_rank.is_some() && f.hotspot_score > 0.0).collect();
        if !hot.is_empty() {
            s.push_str("\n**Touched hotspots**\n\n| File | Rank | Score | Complexity | Health | Functions touched |\n|---|---|---|---|---|---|\n");
            let mut hot = hot.clone();
            hot.sort_by(|a, b| b.hotspot_score.total_cmp(&a.hotspot_score));
            for f in hot.iter().take(10) {
                let cx = match (f.complexity_before, f.complexity_after) {
                    (Some(b), Some(a)) => format!("{b:.0} → {a:.0}"),
                    (None, Some(a)) => format!("{a:.0}"),
                    _ => "–".into(),
                };
                let h = match (f.health_before, f.health_after) {
                    (Some(b), Some(a)) if (a - b).abs() >= 0.1 => format!("{b:.1} → {a:.1}"),
                    (_, Some(a)) => format!("{a:.1}"),
                    _ => "–".into(),
                };
                let fns: Vec<String> = f
                    .functions
                    .iter()
                    .take(4)
                    .map(|x| match (x.cc_before, x.cc_after) {
                        (Some(b), Some(a)) if a != b => format!("`{}` cc {b}→{a}", x.name),
                        (_, Some(a)) => format!("`{}` cc {a}", x.name),
                        _ => format!("`{}`", x.name),
                    })
                    .collect();
                s.push_str(&format!(
                    "| `{}` | #{} | {:.2} | {} | {} | {} |\n",
                    f.path,
                    f.hotspot_rank.unwrap_or(0),
                    f.hotspot_score,
                    cx,
                    h,
                    fns.join(", ")
                ));
            }
        }
        if !self.missed.is_empty() {
            s.push_str("\n**Usually changed together but not in this change**\n\n");
            for m in &self.missed {
                s.push_str(&format!(
                    "- `{}`{} — {:.0}% of changes to `{}` also touch it ({}×, {})\n",
                    m.path,
                    if self.repos.len() > 1 || m.kind == "cross-repo" { format!(" ({})", m.repo) } else { String::new() },
                    m.confidence * 100.0,
                    m.because_of.rsplit('/').next().unwrap_or(&m.because_of),
                    m.support,
                    m.kind
                ));
            }
        }
        if !self.reviewers.is_empty() {
            let r: Vec<String> = self.reviewers.iter().map(|e| e.name.clone()).collect();
            s.push_str(&format!("\n**Suggested reviewers:** {}\n", r.join(", ")));
        }
        s.push_str("\n<sub>Generated by csi — behavioral code analysis</sub>\n");
        s
    }
}
