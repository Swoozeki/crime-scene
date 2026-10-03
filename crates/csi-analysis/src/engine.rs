//! Engine: config + cache + loaded dataset, shared by the CLI, web server and MCP server.

use crate::dataset::Dataset;
use crate::scan::{ScanOptions, ScanReport, scan, stale_repos};
use crate::{trends, xray};
use anyhow::Result;
use csi_core::{Config, Db};
use csi_ingest::{CatFile, Git, Progress};

pub struct Engine {
    pub cfg: Config,
    pub db: Db,
    pub ds: Dataset,
}

impl Engine {
    /// Open the cache; scan first when `auto_scan` and any repo is stale or never scanned.
    pub fn open(cfg: Config, auto_scan: bool, progress: Progress) -> Result<Self> {
        let db = Db::open(&cfg.cache_path)?;
        if auto_scan && !stale_repos(&cfg, &db).is_empty() {
            scan(&cfg, &db, &ScanOptions::default(), progress)?;
        }
        let ds = Dataset::load(&cfg, &db)?;
        Ok(Self { cfg, db, ds })
    }

    pub fn stale(&self) -> Vec<String> {
        stale_repos(&self.cfg, &self.db)
    }

    pub fn scan(&mut self, opts: &ScanOptions, progress: Progress) -> Result<ScanReport> {
        let r = scan(&self.cfg, &self.db, opts, progress)?;
        self.ds = Dataset::load(&self.cfg, &self.db)?;
        Ok(r)
    }

    /// Rescan incrementally if anything moved. Returns true when the dataset was reloaded.
    pub fn refresh(&mut self, progress: Progress) -> Result<bool> {
        if self.stale().is_empty() {
            return Ok(false);
        }
        self.scan(&ScanOptions::default(), progress)?;
        Ok(true)
    }

    /// Compute X-ray and trend data for one file on demand (not just the top hotspots).
    pub fn ensure_deep(&mut self, file: u32) -> Result<()> {
        let repo = self.ds.files[file as usize].repo;
        let mut cat = CatFile::new(&Git::new(&self.ds.repos[repo as usize].path))?;
        let x = xray::update(&self.db, &self.ds, file, &mut cat)?;
        let t = trends::update(&self.db, &self.ds, file, &mut cat)?;
        if t > 0 {
            self.ds.files[file as usize].trend = load_trend(&self.db, self.ds.files[file as usize].db_id)?;
        }
        let _ = x;
        Ok(())
    }
}

pub fn load_trend(db: &Db, file_db_id: i64) -> Result<Vec<crate::dataset::TrendPoint>> {
    let mut st = db.conn.prepare_cached(
        "SELECT c.ts, b.complexity, b.loc, b.max_cc FROM trend_points tp
         JOIN commits c ON c.id = tp.commit_id JOIN blob_metrics b ON b.blob = tp.blob
         WHERE tp.file_id=?1 ORDER BY c.ts",
    )?;
    let v = st
        .query_map([file_db_id], |r| {
            Ok(crate::dataset::TrendPoint { ts: r.get(0)?, complexity: r.get(1)?, loc: r.get(2)?, max_cc: r.get(3)? })
        })?
        .collect::<Result<_, _>>()?;
    Ok(v)
}
