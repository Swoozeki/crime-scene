//! Change coupling with support / confidence / lift, per commit (= PR under squash merges)
//! or per ticket (all commits sharing a ticket ID, across repositories).

use crate::dataset::Dataset;
use crate::scope::{Level, Scope};
use csi_core::util::DAY;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum By {
    #[default]
    Commit,
    Ticket,
}

impl By {
    pub fn parse(s: &str) -> Option<By> {
        match s {
            "commit" | "commits" | "pr" => Some(By::Commit),
            "ticket" | "tickets" => Some(By::Ticket),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Coupling {
    pub a: u32,
    pub b: u32,
    pub a_name: String,
    pub b_name: String,
    pub a_repo: String,
    pub b_repo: String,
    pub a_unit: Option<String>,
    pub b_unit: Option<String>,
    pub support: u32,
    pub n_a: u32,
    pub n_b: u32,
    pub conf_ab: f64,
    pub conf_ba: f64,
    pub lift: f64,
    pub jaccard: f64,
    /// changes together in the last 180 days
    pub recent: u32,
    pub cross_unit: bool,
    pub cross_repo: bool,
    /// same logical entity (only at file level; expected coupling)
    pub same_entity: bool,
    /// source ↔ its own test (same stem, wherever the test lives)
    pub test_pair: bool,
    /// one side is test-only code
    pub involves_test: bool,
}

impl Coupling {
    pub fn degree(&self) -> f64 {
        self.conf_ab.max(self.conf_ba)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CouplingQuery {
    pub level: Level,
    pub by: By,
    /// Only pairs where one side's name contains this (a path or entity).
    pub focus: Option<String>,
    pub min_support: Option<u32>,
    pub min_confidence: Option<f64>,
    pub min_lift: Option<f64>,
    pub cross_only: bool,
    pub include_expected: bool,
    /// Keep pairs where one side is test-only code (hidden by default: "a test changes with
    /// its subject" says little about the design).
    pub include_tests: bool,
}

/// Build change sets: each is a deduped list of keys.
pub fn change_sets(ds: &Dataset, level: Level, by: By, scope: &Scope) -> Vec<(Vec<u32>, i64)> {
    let max = ds.cfg.analysis.max_commit_files;
    let mut sets = vec![];
    let keys_of = |ci: u32, out: &mut Vec<u32>| {
        for ch in &ds.commits[ci as usize].changes {
            let k = ds.key(ch.file, level);
            if !out.contains(&k) {
                out.push(k);
            }
        }
    };
    let eligible = |ci: u32| {
        let c = &ds.commits[ci as usize];
        c.couples() && scope.commit(ds, c)
    };
    match by {
        By::Commit => {
            for ci in 0..ds.commits.len() as u32 {
                if !eligible(ci) {
                    continue;
                }
                let mut ks = vec![];
                keys_of(ci, &mut ks);
                sets.push((ks, ds.commits[ci as usize].ts));
            }
        }
        By::Ticket => {
            for commits in &ds.ticket_commits {
                let mut ks = vec![];
                let mut ts = 0;
                let mut files = 0;
                for &ci in commits {
                    if eligible(ci) {
                        keys_of(ci, &mut ks);
                        files += ds.commits[ci as usize].changes.len();
                        ts = ts.max(ds.commits[ci as usize].ts);
                    }
                }
                if !ks.is_empty() && files <= max * 2 {
                    sets.push((ks, ts));
                }
            }
            let ticketless: Vec<u32> = (0..ds.commits.len() as u32)
                .filter(|&ci| ds.commits[ci as usize].tickets.is_empty() && eligible(ci))
                .collect();
            for session in sessions(ds, &ticketless) {
                let files: usize = session.iter().map(|&ci| ds.commits[ci as usize].changes.len()).sum();
                let repos = session.iter().map(|&ci| ds.commits[ci as usize].repo).collect::<HashSet<_>>().len();
                if repos > 1 && files <= max * 2 {
                    let mut ks = vec![];
                    for &ci in &session {
                        keys_of(ci, &mut ks);
                    }
                    let ts = session.iter().map(|&ci| ds.commits[ci as usize].ts).max().unwrap_or(0);
                    sets.push((ks, ts));
                } else {
                    for ci in session {
                        let mut ks = vec![];
                        keys_of(ci, &mut ks);
                        sets.push((ks, ds.commits[ci as usize].ts));
                    }
                }
            }
        }
    }
    sets
}

/// Groups commits into work sessions: one author, starting within `session_hours` of the
/// session's first commit. Without tickets, this is how a change that spans repos shows up.
fn sessions(ds: &Dataset, commits: &[u32]) -> Vec<Vec<u32>> {
    let window = (ds.cfg.analysis.session_hours * 3600.0) as i64;
    if window <= 0 || ds.repos.len() < 2 {
        return commits.iter().map(|&c| vec![c]).collect();
    }
    let mut by_author: HashMap<u32, Vec<u32>> = HashMap::new();
    for &ci in commits {
        by_author.entry(ds.commits[ci as usize].author).or_default().push(ci);
    }
    let mut out = vec![];
    for (_, mut cs) in by_author {
        cs.sort_by_key(|&ci| ds.commits[ci as usize].ts);
        let mut cur: Vec<u32> = vec![];
        for ci in cs {
            let ts = ds.commits[ci as usize].ts;
            if cur.first().is_some_and(|&s| ts - ds.commits[s as usize].ts > window) {
                out.push(std::mem::take(&mut cur));
            }
            cur.push(ci);
        }
        if !cur.is_empty() {
            out.push(cur);
        }
    }
    out
}

pub fn coupling(ds: &Dataset, q: &CouplingQuery, scope: &Scope) -> Vec<Coupling> {
    let level = q.level;
    let a = &ds.cfg.analysis;
    let min_support = q.min_support.unwrap_or(a.coupling_min_support);
    let min_conf = q.min_confidence.unwrap_or(a.coupling_min_confidence);
    let min_lift = q.min_lift.unwrap_or(a.coupling_min_lift);
    let recent_cutoff = ds.now - (180.0 * DAY) as i64;

    let sets = change_sets(ds, level, q.by, &Scope { days: scope.days, ..Default::default() });
    let total = sets.len() as f64;
    let mut n: HashMap<u32, u32> = HashMap::new();
    let mut pairs: HashMap<(u32, u32), (u32, u32)> = HashMap::new();
    for (ks, ts) in &sets {
        for &k in ks {
            *n.entry(k).or_default() += 1;
        }
        if ks.len() < 2 {
            continue;
        }
        for i in 0..ks.len() {
            for j in i + 1..ks.len() {
                let key = if ks[i] < ks[j] { (ks[i], ks[j]) } else { (ks[j], ks[i]) };
                let e = pairs.entry(key).or_default();
                e.0 += 1;
                if *ts >= recent_cutoff {
                    e.1 += 1;
                }
            }
        }
    }

    let alive_key = alive_keys(ds, level);
    let in_scope = |k: u32| -> bool {
        if scope.repo.is_none() && scope.unit.is_none() && scope.path.is_none() {
            return true;
        }
        ds.key_files(k, level).iter().any(|&f| scope.file(ds, &ds.files[f as usize]))
    };
    let focus = q.focus.as_deref();
    let mut out: Vec<Coupling> = pairs
        .into_iter()
        .filter(|(_, (s, _))| *s >= min_support)
        .filter(|((x, y), _)| alive_key[*x as usize] && alive_key[*y as usize])
        .filter_map(|((x, y), (s, recent))| {
            let (na, nb) = (n[&x], n[&y]);
            let conf_ab = s as f64 / na as f64;
            let conf_ba = s as f64 / nb as f64;
            let lift = s as f64 * total / (na as f64 * nb as f64);
            if conf_ab.max(conf_ba) < min_conf || lift < min_lift {
                return None;
            }
            let (ua, ub) = (ds.key_unit(x, level), ds.key_unit(y, level));
            let (ra, rb) = (ds.key_repo(x, level), ds.key_repo(y, level));
            let same_entity = level == Level::File && ds.files[x as usize].entity == ds.files[y as usize].entity;
            let (ta, tb) = (ds.key_is_test(x, level), ds.key_is_test(y, level));
            let (a_name, b_name) = (ds.key_name(x, level), ds.key_name(y, level));
            let test_pair = ta != tb && stem(&a_name) == stem(&b_name);
            Some(Coupling {
                a: x,
                b: y,
                a_name,
                b_name,
                a_repo: ds.repos[ra as usize].name.clone(),
                b_repo: ds.repos[rb as usize].name.clone(),
                a_unit: ua.map(|u| ds.unit_label(u)),
                b_unit: ub.map(|u| ds.unit_label(u)),
                support: s,
                n_a: na,
                n_b: nb,
                conf_ab: round3(conf_ab),
                conf_ba: round3(conf_ba),
                lift: round3(lift),
                jaccard: round3(s as f64 / (na + nb - s) as f64),
                recent,
                cross_unit: ua != ub,
                cross_repo: ra != rb,
                same_entity,
                test_pair,
                involves_test: ta || tb,
            })
        })
        .filter(|c| q.include_expected || !c.same_entity)
        .filter(|c| q.include_tests || !c.involves_test)
        .filter(|c| !q.cross_only || c.cross_unit || c.cross_repo)
        .filter(|c| in_scope(c.a) || in_scope(c.b))
        .filter(|c| focus.is_none_or(|f| c.a_name.contains(f) || c.b_name.contains(f)))
        .collect();
    out.sort_by(|x, y| {
        rank_score(y).total_cmp(&rank_score(x)).then(y.support.cmp(&x.support)).then(x.a_name.cmp(&y.a_name))
    });
    out
}

/// `src/app/cart.service.spec.ts` and `src/app/cart.service` → `cart.service`.
fn stem(name: &str) -> String {
    let key = csi_arch::entity_key(name);
    key.rsplit('/').next().unwrap_or(&key).to_string()
}

fn rank_score(c: &Coupling) -> f64 {
    c.degree() * (1.0 - 1.0 / (c.support as f64).sqrt())
}

fn round3(x: f64) -> f64 {
    (x * 1000.0).round() / 1000.0
}

pub fn alive_keys(ds: &Dataset, level: Level) -> Vec<bool> {
    let mut alive = vec![false; ds.key_count(level)];
    for (i, f) in ds.files.iter().enumerate() {
        if f.alive {
            alive[ds.key(i as u32, level) as usize] = true;
        }
    }
    alive
}

/// Sum of coupling (Tornhill): for each change set containing a key, add the number of other keys.
pub fn sum_of_coupling(ds: &Dataset, level: Level) -> Vec<u32> {
    let mut soc = vec![0u32; ds.key_count(level)];
    for (ks, _) in change_sets(ds, level, By::Commit, &Scope::default()) {
        let others = ks.len() as u32 - 1;
        for k in ks {
            soc[k as usize] += others;
        }
    }
    soc
}
