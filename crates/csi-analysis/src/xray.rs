//! X-ray: function-level hotspots inside a file, from `git log -p -U0` hunks mapped onto
//! function ranges of the pre- and post-image blobs.

use crate::dataset::Dataset;
use anyhow::{Context, Result};
use csi_core::Db;
use csi_core::util::{decay, percentile_ranks};
use csi_ingest::metrics::{load_functions, metrics_for};
use csi_ingest::{CatFile, Git};
use csi_lang::{Function, function_at};
use rusqlite::params;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader};
use std::process::Stdio;

const TOP_LEVEL: &str = "(top level)";
const NULL_SHA: &str = "0000000000000000000000000000000000000000";

/// Ensure function-level change data exists for every commit touching `file`.
pub fn update(db: &Db, ds: &Dataset, file: u32, cat: &mut CatFile) -> Result<usize> {
    let f = &ds.files[file as usize];
    if !f.alive || !f.lang.is_parsed() {
        return Ok(0);
    }
    let repo = &ds.repos[f.repo as usize];
    let done: HashSet<i64> = {
        let mut st = db.conn.prepare_cached("SELECT commit_id FROM xray_done WHERE file_id=?1")?;
        st.query_map([f.db_id], |r| r.get(0))?.collect::<Result<_, _>>()?
    };
    let needed: HashSet<i64> =
        f.commits.iter().map(|&c| ds.commits[c as usize].db_id).filter(|id| !done.contains(id)).collect();
    if needed.is_empty() {
        return Ok(0);
    }
    let head = repo.head.clone().unwrap_or_else(|| "HEAD".into());
    let git = Git::new(&repo.path);
    let mut cmd = git.command();
    cmd.args(["log", "--no-merges", "-M", "--follow", "-p", "-U0", "--full-index", "--format=%x1e%H"]);
    let since = ds.cfg.analysis.since.trim();
    if !since.is_empty() && since != "all" {
        cmd.arg(format!("--since={since} ago"));
    }
    cmd.arg(&head).arg("--").arg(&f.path);
    let mut child = cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().context("git log -p")?;
    let mut reader = BufReader::new(child.stdout.take().unwrap());
    let max = ds.cfg.analysis.max_file_bytes;

    let tx = db.conn.unchecked_transaction()?;
    let mut processed = 0;
    let mut buf = Vec::new();
    loop {
        buf.clear();
        if reader.read_until(0x1e, &mut buf)? == 0 {
            break;
        }
        if buf.last() == Some(&0x1e) {
            buf.pop();
        }
        let rec = String::from_utf8_lossy(&buf);
        let mut lines = rec.lines();
        let Some(sha) = lines.next().map(str::trim).filter(|s| s.len() == 40) else { continue };
        let Some(&ci) = ds.commit_by_sha.get(&(f.repo, sha.to_string())) else { continue };
        let commit_id = ds.commits[ci as usize].db_id;
        if !needed.contains(&commit_id) {
            continue;
        }
        let (mut old_blob, mut new_blob) = (String::new(), String::new());
        let mut hunks: Vec<(u32, u32, u32, u32)> = vec![];
        for l in lines {
            if let Some(idx) = l.strip_prefix("index ") {
                if let Some((o, rest)) = idx.split_once("..") {
                    old_blob = o.to_string();
                    new_blob = rest.split_whitespace().next().unwrap_or("").to_string();
                }
            } else if l.starts_with("@@ ") {
                if let Some(h) = parse_hunk(l) {
                    hunks.push(h);
                }
            }
        }
        let fns = |blob: &str, cat: &mut CatFile| -> Result<Vec<Function>> {
            if blob.is_empty() || blob == NULL_SHA {
                return Ok(vec![]);
            }
            Ok(metrics_for(db, cat, blob, &f.path, max)?.map(|(_, m)| m.functions).unwrap_or_default())
        };
        let new_fns = fns(&new_blob, cat)?;
        let old_fns = if hunks.iter().any(|h| h.1 > 0) { fns(&old_blob, cat)? } else { vec![] };
        let mut acc: HashMap<String, (u32, u32)> = HashMap::new();
        for (a, b, c, d) in hunks {
            for line in c..c + d {
                let name = function_at(&new_fns, line).map(|f| f.name.as_str()).unwrap_or(TOP_LEVEL);
                acc.entry(name.to_string()).or_default().0 += 1;
            }
            for line in a..a + b {
                let name = function_at(&old_fns, line).map(|f| f.name.as_str()).unwrap_or(TOP_LEVEL);
                acc.entry(name.to_string()).or_default().1 += 1;
            }
        }
        for (name, (added, deleted)) in acc {
            tx.execute(
                "INSERT INTO function_changes(file_id, commit_id, fn, added, deleted) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![f.db_id, commit_id, name, added, deleted],
            )?;
        }
        tx.execute("INSERT OR IGNORE INTO xray_done(file_id, commit_id) VALUES (?1, ?2)", params![f.db_id, commit_id])?;
        processed += 1;
    }
    let _ = child.wait();
    // commits git didn't report for this path (e.g. outside --follow's view) are marked done too
    for id in &needed {
        tx.execute("INSERT OR IGNORE INTO xray_done(file_id, commit_id) VALUES (?1, ?2)", params![f.db_id, id])?;
    }
    tx.commit()?;
    Ok(processed)
}

/// `@@ -a,b +c,d @@` → (a, b, c, d); counts default to 1.
fn parse_hunk(l: &str) -> Option<(u32, u32, u32, u32)> {
    let mut parts = l.split_whitespace();
    parts.next()?;
    let old = parts.next()?.strip_prefix('-')?;
    let new = parts.next()?.strip_prefix('+')?;
    let pair = |s: &str| -> Option<(u32, u32)> {
        match s.split_once(',') {
            Some((x, y)) => Some((x.parse().ok()?, y.parse().ok()?)),
            None => Some((s.parse().ok()?, 1)),
        }
    };
    let (a, b) = pair(old)?;
    let (c, d) = pair(new)?;
    Some((a, b, c, d))
}

#[derive(Debug, Clone, Serialize)]
pub struct FnHotspot {
    pub name: String,
    pub start: u32,
    pub end: u32,
    pub cc: u32,
    pub nesting: u32,
    pub loc: u32,
    pub revisions: u32,
    pub rev_w: f64,
    pub churn: u32,
    pub defects: u32,
    pub authors: u32,
    pub last_change: Option<i64>,
    pub score: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct FnCoupling {
    pub a: String,
    pub b: String,
    pub support: u32,
    pub confidence: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct XRay {
    pub path: String,
    pub repo: String,
    pub analyzed_commits: u32,
    pub functions: Vec<FnHotspot>,
    pub coupling: Vec<FnCoupling>,
}

/// Function hotspots for an alive file, using data stored by [`update`].
pub fn xray(ds: &Dataset, db: &Db, file: u32) -> Result<XRay> {
    let f = &ds.files[file as usize];
    let current: Vec<Function> = match &f.blob {
        Some(b) => load_functions(db, b)?,
        None => vec![],
    };
    let commit_idx: HashMap<i64, u32> = f.commits.iter().map(|&c| (ds.commits[c as usize].db_id, c)).collect();
    let mut st = db.conn.prepare_cached("SELECT commit_id, fn, added, deleted FROM function_changes WHERE file_id=?1")?;
    let rows: Vec<(i64, String, u32, u32)> =
        st.query_map([f.db_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?.collect::<Result<_, _>>()?;
    let analyzed: u32 = db.conn.query_row("SELECT count(*) FROM xray_done WHERE file_id=?1", [f.db_id], |r| r.get(0))?;

    #[derive(Default)]
    struct Acc {
        revs: u32,
        rev_w: f64,
        churn: u32,
        defects: u32,
        authors: HashSet<u32>,
        last: Option<i64>,
    }
    let mut acc: HashMap<String, Acc> = HashMap::new();
    let mut by_commit: HashMap<i64, Vec<String>> = HashMap::new();
    for (cid, name, added, deleted) in rows {
        let Some(&ci) = commit_idx.get(&cid) else { continue };
        let c = &ds.commits[ci as usize];
        let a = acc.entry(name.clone()).or_default();
        a.revs += 1;
        a.rev_w += c.weight * decay((ds.now - c.ts) as f64, ds.cfg.analysis.half_life_days);
        a.churn += added + deleted;
        a.defects += c.defect as u32;
        a.authors.extend(c.authors());
        a.last = Some(a.last.map_or(c.ts, |l| l.max(c.ts)));
        by_commit.entry(cid).or_default().push(name);
    }
    let mut functions: Vec<FnHotspot> = current
        .iter()
        .map(|func| {
            let a = acc.get(&func.name);
            FnHotspot {
                name: func.name.clone(),
                start: func.start,
                end: func.end,
                cc: func.cc,
                nesting: func.nesting,
                loc: func.loc,
                revisions: a.map(|a| a.revs).unwrap_or(0),
                rev_w: a.map(|a| (a.rev_w * 1000.0).round() / 1000.0).unwrap_or(0.0),
                churn: a.map(|a| a.churn).unwrap_or(0),
                defects: a.map(|a| a.defects).unwrap_or(0),
                authors: a.map(|a| a.authors.len() as u32).unwrap_or(0),
                last_change: a.and_then(|a| a.last),
                score: 0.0,
            }
        })
        .collect();
    let freq = percentile_ranks(&functions.iter().map(|f| f.rev_w).collect::<Vec<_>>());
    let cx = percentile_ranks(&functions.iter().map(|f| f.cc as f64).collect::<Vec<_>>());
    for (i, func) in functions.iter_mut().enumerate() {
        func.score = if func.revisions == 0 { 0.0 } else { ((freq[i] * cx[i]) * 1000.0).round() / 1000.0 };
    }
    functions.sort_by(|a, b| b.score.total_cmp(&a.score).then(b.revisions.cmp(&a.revisions)));

    // intra-file function coupling
    let live: HashSet<&str> = current.iter().map(|f| f.name.as_str()).collect();
    let mut pairs: HashMap<(String, String), u32> = HashMap::new();
    for names in by_commit.values() {
        let mut ns: Vec<&String> = names.iter().filter(|n| live.contains(n.as_str())).collect();
        ns.sort();
        ns.dedup();
        for i in 0..ns.len() {
            for j in i + 1..ns.len() {
                *pairs.entry((ns[i].clone(), ns[j].clone())).or_default() += 1;
            }
        }
    }
    let mut coupling: Vec<FnCoupling> = pairs
        .into_iter()
        .filter(|(_, s)| *s >= 3)
        .map(|((a, b), s)| {
            let na = acc.get(&a).map(|x| x.revs).unwrap_or(1);
            let nb = acc.get(&b).map(|x| x.revs).unwrap_or(1);
            FnCoupling { confidence: ((s as f64 / na.min(nb) as f64) * 1000.0).round() / 1000.0, a, b, support: s }
        })
        .filter(|c| c.confidence >= 0.5)
        .collect();
    coupling.sort_by(|a, b| b.support.cmp(&a.support).then(b.confidence.total_cmp(&a.confidence)));
    coupling.truncate(20);

    Ok(XRay {
        path: f.path.clone(),
        repo: ds.repos[f.repo as usize].name.clone(),
        analyzed_commits: analyzed,
        functions,
        coupling,
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn hunks() {
        assert_eq!(super::parse_hunk("@@ -10,2 +12,3 @@ fn"), Some((10, 2, 12, 3)));
        assert_eq!(super::parse_hunk("@@ -5 +5 @@"), Some((5, 1, 5, 1)));
        assert_eq!(super::parse_hunk("@@ -0,0 +1,20 @@"), Some((0, 0, 1, 20)));
    }
}
