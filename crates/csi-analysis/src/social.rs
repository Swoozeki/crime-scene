//! Social analyses: ownership, fragmentation, bus factor, knowledge loss, experts, coordination.

use crate::dataset::Dataset;
use crate::scope::{Level, Scope};
use csi_core::util::{DAY, decay};
use serde::Serialize;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Serialize)]
pub struct AuthorShare {
    pub name: String,
    pub team: Option<String>,
    pub share: f64,
    pub active: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Ownership {
    pub key: u32,
    pub name: String,
    pub repo: String,
    pub unit: Option<String>,
    pub main_dev: Option<String>,
    pub main_dev_share: f64,
    /// 1 - Σ share²: 0 = single owner, → 1 = many small contributors
    pub fragmentation: f64,
    /// share of knowledge held by authors inactive for `inactive_after_days`
    pub knowledge_loss: f64,
    /// fewest authors holding > 50% of the knowledge
    pub bus_factor: u32,
    pub authors: Vec<AuthorShare>,
    pub teams: Vec<(String, f64)>,
    /// distinct authors in the last 365 days
    pub recent_authors: u32,
}

/// Knowledge per key per author: lines added (+1 per change so edits that only delete count).
fn knowledge(ds: &Dataset, level: Level, scope: &Scope, alive_only: bool) -> HashMap<u32, HashMap<u32, f64>> {
    let mut m: HashMap<u32, HashMap<u32, f64>> = HashMap::new();
    let file_ok: Vec<bool> = ds.files.iter().map(|f| (!alive_only || f.alive) && scope.file(ds, f)).collect();
    for c in &ds.commits {
        if !scope.commit(ds, c) || c.format {
            continue;
        }
        let share = 1.0 / (c.coauthors.len() as f64 + 1.0);
        for ch in &c.changes {
            if !file_ok[ch.file as usize] {
                continue;
            }
            let k = ds.key(ch.file, level);
            let e = m.entry(k).or_default();
            for a in c.authors() {
                *e.entry(a).or_default() += (ch.added as f64 + 1.0) * share;
            }
        }
    }
    m
}

pub fn ownership(ds: &Dataset, level: Level, scope: &Scope) -> Vec<Ownership> {
    let k = knowledge(ds, level, scope, true);
    let recent_cut = ds.now - (365.0 * DAY) as i64;
    let mut recent: HashMap<u32, HashSet<u32>> = HashMap::new();
    for c in ds.commits.iter().filter(|c| c.ts >= recent_cut) {
        for ch in &c.changes {
            if ds.files[ch.file as usize].alive {
                recent.entry(ds.key(ch.file, level)).or_default().extend(c.authors());
            }
        }
    }
    let mut out: Vec<Ownership> = k
        .into_iter()
        .map(|(key, authors)| {
            let total: f64 = authors.values().sum();
            let mut shares: Vec<(u32, f64)> = authors.into_iter().map(|(a, v)| (a, v / total)).collect();
            shares.sort_by(|x, y| y.1.total_cmp(&x.1).then(x.0.cmp(&y.0)));
            let mut teams: HashMap<String, f64> = HashMap::new();
            for (a, s) in &shares {
                if let Some(t) = &ds.authors[*a as usize].team {
                    *teams.entry(t.clone()).or_default() += s;
                }
            }
            let mut teams: Vec<(String, f64)> = teams.into_iter().map(|(t, s)| (t, round3(s))).collect();
            teams.sort_by(|a, b| b.1.total_cmp(&a.1));
            let mut cum = 0.0;
            let mut bus = 0;
            for (_, s) in &shares {
                cum += s;
                bus += 1;
                if cum > 0.5 {
                    break;
                }
            }
            Ownership {
                key,
                name: ds.key_name(key, level),
                repo: ds.repos[ds.key_repo(key, level) as usize].name.clone(),
                unit: ds.key_unit(key, level).map(|u| ds.unit_label(u)),
                main_dev: shares.first().map(|(a, _)| ds.authors[*a as usize].name.clone()),
                main_dev_share: shares.first().map(|(_, s)| round3(*s)).unwrap_or(0.0),
                fragmentation: round3(1.0 - shares.iter().map(|(_, s)| s * s).sum::<f64>()),
                knowledge_loss: round3(shares.iter().filter(|(a, _)| !ds.is_active(*a)).map(|(_, s)| s).sum()),
                bus_factor: bus,
                authors: shares
                    .iter()
                    .take(8)
                    .map(|(a, s)| AuthorShare {
                        name: ds.authors[*a as usize].name.clone(),
                        team: ds.authors[*a as usize].team.clone(),
                        share: round3(*s),
                        active: ds.is_active(*a),
                    })
                    .collect(),
                teams,
                recent_authors: recent.get(&key).map(|s| s.len() as u32).unwrap_or(0),
            }
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

#[derive(Debug, Clone, Serialize)]
pub struct Expert {
    pub name: String,
    pub email: Option<String>,
    pub team: Option<String>,
    /// recency-weighted knowledge share among active authors
    pub score: f64,
    pub commits: u32,
    pub last_commit: i64,
}

/// Active authors ranked by recency-weighted contribution to `files`.
pub fn experts(ds: &Dataset, files: &[u32], exclude: &[u32]) -> Vec<Expert> {
    let set: HashSet<u32> = files.iter().copied().collect();
    let half = ds.cfg.analysis.half_life_days * 2.0;
    let mut acc: HashMap<u32, (f64, u32, i64)> = HashMap::new();
    for &f in files {
        for &ci in &ds.files[f as usize].commits {
            let c = &ds.commits[ci as usize];
            let w = decay((ds.now - c.ts) as f64, half);
            let lines: f64 = c.changes.iter().filter(|ch| set.contains(&ch.file)).map(|ch| ch.added as f64 + 1.0).sum();
            for a in c.authors() {
                let e = acc.entry(a).or_default();
                e.0 += w * lines.ln_1p();
                e.1 += 1;
                e.2 = e.2.max(c.ts);
            }
        }
    }
    let total: f64 = acc.iter().filter(|(a, _)| ds.is_active(**a)).map(|(_, v)| v.0).sum();
    let mut v: Vec<Expert> = acc
        .into_iter()
        .filter(|(a, _)| ds.is_active(*a) && !exclude.contains(a))
        .map(|(a, (s, n, last))| Expert {
            name: ds.authors[a as usize].name.clone(),
            email: ds.authors[a as usize].emails.first().cloned(),
            team: ds.authors[a as usize].team.clone(),
            score: if total > 0.0 { round3(s / total) } else { 0.0 },
            commits: n,
            last_commit: last,
        })
        .collect();
    v.sort_by(|a, b| b.score.total_cmp(&a.score).then(a.name.cmp(&b.name)));
    v.dedup_by(|a, b| a.name == b.name);
    v
}

fn round3(x: f64) -> f64 {
    (x * 1000.0).round() / 1000.0
}
