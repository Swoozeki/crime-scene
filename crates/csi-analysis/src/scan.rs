//! Scan pipeline: ingest → parse current tree → detect architecture → deep analyses
//! (X-ray and complexity trends) for the top hotspots.

use crate::dataset::Dataset;
use crate::hotspots::hotspots;
use crate::scope::{Level, Scope};
use crate::{trends, xray};
use anyhow::{Result, bail};
use csi_core::{Config, Db};
use csi_ingest::{CatFile, Git, Progress, analyze_tree, ingest_repo};
use serde::Serialize;
use std::collections::HashMap;
use std::time::Instant;

#[derive(Debug, Clone, Default)]
pub struct ScanOptions {
    pub full: bool,
    pub repo: Option<String>,
    /// Skip X-ray and trend sampling.
    pub shallow: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct RepoScan {
    pub name: String,
    pub branch: String,
    pub head: String,
    pub new_commits: usize,
    pub full: bool,
    pub parsed_files: usize,
    pub frameworks: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ScanReport {
    pub repos: Vec<RepoScan>,
    pub xray_commits: usize,
    pub trend_points: usize,
    pub seconds: f64,
}

pub fn scan(cfg: &Config, db: &Db, opts: &ScanOptions, progress: Progress) -> Result<ScanReport> {
    let start = Instant::now();
    if cfg.repos.is_empty() {
        bail!("the workspace has no repositories; add [[repo]] entries to crimescene.toml");
    }
    if let Some(r) = &opts.repo
        && !cfg.repos.iter().any(|x| &x.name == r)
    {
        bail!("unknown repo `{r}`");
    }
    let mut reports = vec![];
    for rc in cfg.repos.iter().filter(|r| opts.repo.as_ref().is_none_or(|n| n == &r.name)) {
        progress(&format!("{}: reading history", rc.name));
        let stats = ingest_repo(db, cfg, rc, opts.full, progress)?;
        let git = Git::new(cfg.repo_path(rc));
        let mut cat = CatFile::new(&git)?;
        progress(&format!("{}: measuring {} files", rc.name, stats.tree.len()));
        let parsed = analyze_tree(db, cfg, &mut cat, &stats.tree, progress)?;

        // architecture: plugins read a handful of manifest files from the tree
        let paths: Vec<&str> = stats.tree.iter().map(|e| e.path.as_str()).collect();
        let blobs: HashMap<&str, &str> = stats.tree.iter().map(|e| (e.path.as_str(), e.blob.as_str())).collect();
        let mut read = |p: &str| -> Option<String> {
            let blob = blobs.get(p)?;
            cat.get(blob).ok().flatten().map(|(_, d)| String::from_utf8_lossy(&d).into_owned())
        };
        let arch = csi_arch::detect(&paths, &mut read, &rc.name);
        db.conn.execute(
            "UPDATE repos SET arch=?2 WHERE id=?1",
            rusqlite::params![stats.repo_id, serde_json::to_string(&arch)?],
        )?;
        progress(&format!(
            "{}: {} new commits{}, {} files parsed",
            rc.name,
            stats.new_commits,
            if stats.full { " (full)" } else { "" },
            parsed
        ));
        reports.push(RepoScan {
            name: rc.name.clone(),
            branch: stats.branch,
            head: stats.head,
            new_commits: stats.new_commits,
            full: stats.full,
            parsed_files: parsed,
            frameworks: arch.frameworks,
        });
    }

    let (mut xc, mut tp) = (0, 0);
    if !opts.shallow {
        let ds = Dataset::load(cfg, db)?;
        let files = hotspots(&ds, Level::File, &Scope::default());
        let top: Vec<u32> = files
            .iter()
            .filter(|h| h.revisions >= 2 && ds.files[h.key as usize].lang.is_parsed())
            .take(cfg.analysis.top_n_xray)
            .map(|h| h.key)
            .collect();
        if !top.is_empty() {
            progress(&format!("x-raying the top {} hotspot files", top.len()));
        }
        let mut cats: HashMap<u32, CatFile> = HashMap::new();
        for f in top {
            let repo = ds.files[f as usize].repo;
            if let std::collections::hash_map::Entry::Vacant(e) = cats.entry(repo) {
                e.insert(CatFile::new(&Git::new(&ds.repos[repo as usize].path))?);
            }
            let cat = cats.get_mut(&repo).unwrap();
            xc += xray::update(db, &ds, f, cat)?;
            tp += trends::update(db, &ds, f, cat)?;
        }
    }
    db.meta_set("last_scan", &chrono_now().to_string())?;
    Ok(ScanReport { repos: reports, xray_commits: xc, trend_points: tp, seconds: start.elapsed().as_secs_f64() })
}

fn chrono_now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// Repos whose analyzed branch moved since the last scan (or that were never scanned).
pub fn stale_repos(cfg: &Config, db: &Db) -> Vec<String> {
    cfg.repos
        .iter()
        .filter(|rc| {
            let last: Option<String> =
                db.conn.query_row("SELECT last_sha FROM repos WHERE name=?1", [&rc.name], |r| r.get(0)).ok().flatten();
            let git = Git::new(cfg.repo_path(rc));
            let head = git.resolve_branch(rc.branch.as_deref()).and_then(|b| git.rev_parse(&b)).ok();
            last.is_none() || head.is_none() || last != head
        })
        .map(|r| r.name.clone())
        .collect()
}
