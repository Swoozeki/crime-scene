//! Hotspots: recency-weighted change frequency × language-normalized complexity.

use crate::dataset::Dataset;
use crate::scope::{Level, Scope};
use crate::trends::{self, Trend};
use csi_core::util::{DAY, decay, percentile_ranks};
use serde::Serialize;
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize)]
pub struct Hotspot {
    pub key: u32,
    pub level: Level,
    pub name: String,
    pub display: String,
    pub repo: String,
    pub unit: Option<String>,
    pub kind: String,
    pub lang: Option<String>,
    pub rank: usize,
    pub score: f64,
    pub revisions: u32,
    pub rev_w: f64,
    pub churn: u32,
    pub loc: u32,
    pub complexity: f64,
    pub norm_complexity: f64,
    pub health: f64,
    pub health_reasons: Vec<String>,
    pub authors: u32,
    pub active_authors: u32,
    pub main_dev: Option<String>,
    pub main_dev_share: f64,
    pub defects: u32,
    pub defect_density: f64,
    pub last_change: Option<i64>,
    pub age_days: Option<f64>,
    pub trend: Option<Trend>,
    pub files: u32,
    /// every alive file is a test/spec/story
    pub test: bool,
    /// has at least one alive source-code file (not only config/docs)
    pub code: bool,
}

/// All alive keys at `level` within `scope`, sorted by score (rank 1 = hottest).
pub fn hotspots(ds: &Dataset, level: Level, scope: &Scope) -> Vec<Hotspot> {
    let n = ds.key_count(level);
    let half_life = ds.cfg.analysis.half_life_days;

    #[derive(Default, Clone)]
    struct Acc {
        revs: u32,
        rev_w: f64,
        churn: u32,
        defects: u32,
        last: Option<i64>,
        authors: HashMap<u32, f64>,
    }
    let mut acc: Vec<Acc> = vec![Acc::default(); n];
    let mut in_scope = vec![false; n];
    let mut alive = vec![false; n];
    let file_ok: Vec<bool> = ds.files.iter().map(|f| scope.file(ds, f)).collect();
    for (i, f) in ds.files.iter().enumerate() {
        if file_ok[i] {
            let k = ds.key(i as u32, level) as usize;
            in_scope[k] = true;
            alive[k] |= f.alive;
        }
    }

    let mut seen: Vec<u32> = Vec::new();
    for c in &ds.commits {
        if !scope.commit(ds, c) {
            continue;
        }
        let w = c.weight * decay((ds.now - c.ts) as f64, half_life);
        seen.clear();
        for ch in &c.changes {
            if !file_ok[ch.file as usize] {
                continue;
            }
            let k = ds.key(ch.file, level);
            let a = &mut acc[k as usize];
            a.churn += ch.added + ch.deleted;
            let lines = (ch.added as f64 + 1.0) / (c.coauthors.len() as f64 + 1.0);
            for au in c.authors() {
                *a.authors.entry(au).or_default() += lines;
            }
            if !seen.contains(&k) {
                seen.push(k);
                a.revs += 1;
                a.rev_w += w;
                if c.defect {
                    a.defects += 1;
                }
                a.last = Some(a.last.map_or(c.ts, |l| l.max(c.ts)));
            }
        }
    }

    let keys: Vec<u32> = (0..n as u32).filter(|&k| in_scope[k as usize] && alive[k as usize]).collect();
    // complexity per key from alive files (tests excluded unless the key is only tests)
    let cx_files = |k: u32| -> Vec<u32> {
        let fs: Vec<u32> = ds.key_files(k, level).into_iter().filter(|&f| ds.files[f as usize].alive).collect();
        let non_test: Vec<u32> = fs.iter().copied().filter(|&f| !ds.files[f as usize].test).collect();
        if non_test.is_empty() { fs } else { non_test }
    };
    let unit_files: Option<HashMap<u32, Vec<u32>>> = matches!(level, Level::Unit | Level::Repo).then(|| {
        let mut m: HashMap<u32, Vec<u32>> = HashMap::new();
        for (i, f) in ds.files.iter().enumerate() {
            if f.alive && file_ok[i] {
                m.entry(ds.key(i as u32, level)).or_default().push(i as u32);
            }
        }
        m
    });
    let files_of = |k: u32| -> Vec<u32> {
        match &unit_files {
            Some(m) => {
                let fs = m.get(&k).cloned().unwrap_or_default();
                let nt: Vec<u32> = fs.iter().copied().filter(|&f| !ds.files[f as usize].test).collect();
                if nt.is_empty() { fs } else { nt }
            }
            None => cx_files(k),
        }
    };

    let mut rows: Vec<Hotspot> = keys
        .iter()
        .map(|&k| {
            let fs = files_of(k);
            let a = &acc[k as usize];
            let loc: u32 = fs.iter().map(|&f| ds.files[f as usize].loc).sum();
            let complexity: f64 = fs.iter().map(|&f| ds.files[f as usize].complexity).sum();
            let worst =
                fs.iter().min_by(|&&x, &&y| ds.files[x as usize].health().total_cmp(&ds.files[y as usize].health()));
            let (health, reasons) = worst
                .map(|&f| (ds.files[f as usize].health(), ds.files[f as usize].summary.health_reasons.clone()))
                .unwrap_or((10.0, vec![]));
            let total: f64 = a.authors.values().sum();
            let main = a.authors.iter().max_by(|x, y| x.1.total_cmp(y.1).then(y.0.cmp(x.0)));
            let (display, kind, lang) = match level {
                Level::File => {
                    let f = &ds.files[k as usize];
                    (short_name(&f.path), "file".to_string(), Some(f.lang.as_str().to_string()))
                }
                Level::Entity => {
                    let e = &ds.entities[k as usize];
                    (short_name(&e.key), e.kind.clone(), None)
                }
                Level::Unit => (ds.units[k as usize].name.clone(), ds.units[k as usize].kind.clone(), None),
                Level::Repo => (ds.repos[k as usize].name.clone(), "repo".into(), None),
            };
            let trend = match level {
                Level::File => trends::classify(&ds.files[k as usize].trend, ds.now),
                Level::Entity => fs.iter().find_map(|&f| trends::classify(&ds.files[f as usize].trend, ds.now)),
                _ => None,
            };
            Hotspot {
                key: k,
                level,
                name: ds.key_name(k, level),
                display,
                repo: ds.repos[ds.key_repo(k, level) as usize].name.clone(),
                unit: ds.key_unit(k, level).map(|u| ds.unit_label(u)),
                kind,
                lang,
                rank: 0,
                score: 0.0,
                revisions: a.revs,
                rev_w: a.rev_w,
                churn: a.churn,
                loc,
                complexity,
                norm_complexity: 0.0,
                health,
                health_reasons: reasons,
                authors: a.authors.len() as u32,
                active_authors: a.authors.keys().filter(|&&au| ds.is_active(au)).count() as u32,
                main_dev: main.map(|(au, _)| ds.authors[*au as usize].name.clone()),
                main_dev_share: main.map(|(_, v)| if total > 0.0 { v / total } else { 0.0 }).unwrap_or(0.0),
                defects: a.defects,
                defect_density: if a.revs > 0 { a.defects as f64 / a.revs as f64 } else { 0.0 },
                last_change: a.last,
                age_days: a.last.map(|l| (ds.now - l) as f64 / DAY),
                trend,
                test: !fs.is_empty() && fs.iter().all(|&f| ds.files[f as usize].test),
                code: fs.iter().any(|&f| ds.files[f as usize].lang.weight() >= 0.5),
                files: fs.len() as u32,
            }
        })
        .collect();

    // normalized complexity per level
    match level {
        Level::File => {
            for r in rows.iter_mut() {
                r.norm_complexity = ds.files[r.key as usize].norm_cx;
            }
        }
        Level::Entity => {
            for r in rows.iter_mut() {
                r.norm_complexity = cx_files(r.key).iter().map(|&f| ds.files[f as usize].norm_cx).fold(0.0, f64::max);
            }
        }
        Level::Unit | Level::Repo => {
            let weighted: Vec<f64> = rows
                .iter()
                .map(|r| {
                    files_of(r.key)
                        .iter()
                        .map(|&f| ds.files[f as usize].complexity * ds.files[f as usize].lang.weight())
                        .sum()
                })
                .collect();
            let pr = percentile_ranks(&weighted);
            for (r, p) in rows.iter_mut().zip(pr) {
                r.norm_complexity = p;
            }
        }
    }
    // Frequency is log-scaled against the busiest key so the top of the long tail stays
    // discriminating (percentiles would flatten 5 and 80 changes into the same bucket).
    // measured against code only, so CHANGELOGs and package.json don't set the scale
    let max_rw = match rows.iter().filter(|r| r.code && !r.test).map(|r| r.rev_w).fold(0.0, f64::max) {
        m if m > 0.0 => m,
        _ => rows.iter().map(|r| r.rev_w).fold(0.0, f64::max),
    };
    for r in rows.iter_mut() {
        if r.test {
            r.norm_complexity *= 0.5; // test code matters, but less than the code it tests
        }
        let freq = if max_rw > 0.0 { ((1.0 + r.rev_w).ln() / (1.0 + max_rw).ln()).min(1.0) } else { 0.0 };
        r.score = if r.revisions == 0 { 0.0 } else { (freq * r.norm_complexity * 1000.0).round() / 1000.0 };
    }
    rows.sort_by(|a, b| b.score.total_cmp(&a.score).then(b.rev_w.total_cmp(&a.rev_w)).then(a.name.cmp(&b.name)));
    for (i, r) in rows.iter_mut().enumerate() {
        r.rank = i + 1;
    }
    rows
}

/// Rank lookup: key -> hotspot row index.
pub fn index(rows: &[Hotspot]) -> HashMap<u32, usize> {
    rows.iter().enumerate().map(|(i, r)| (r.key, i)).collect()
}

/// Last path segment, prefixed with its directory when the name alone is too generic to recognize.
pub fn short_name(path: &str) -> String {
    let mut parts = path.rsplit('/');
    let name = parts.next().unwrap_or(path);
    let stem = name.split('.').next().unwrap_or(name).to_ascii_lowercase();
    const GENERIC: &[&str] = &[
        "index",
        "util",
        "utils",
        "types",
        "type",
        "helpers",
        "helper",
        "main",
        "api",
        "constants",
        "model",
        "models",
        "node",
        "common",
        "shared",
        "config",
        "mod",
        "lib",
        "core",
        "base",
        "handler",
        "service",
        "routes",
        "module",
        "component",
        "app",
        "init",
        "__init__",
        "default",
        "interfaces",
        "public_api",
    ];
    match parts.next() {
        Some(dir) if GENERIC.contains(&stem.as_str()) => format!("{dir}/{name}"),
        _ => name.to_string(),
    }
}
