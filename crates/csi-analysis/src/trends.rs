//! Complexity trends for hotspot files: sampled historical versions, classified by slope.

use crate::dataset::{Dataset, TrendPoint};
use anyhow::Result;
use csi_core::Db;
use csi_ingest::CatFile;
use csi_ingest::metrics::metrics_for;
use rusqlite::params;
use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Trend {
    /// deteriorating | stable | improving
    pub direction: String,
    /// relative change of complexity between the first and last third of samples
    pub change: f64,
    pub refactored: bool,
    pub points: usize,
}

pub fn classify(points: &[TrendPoint]) -> Option<Trend> {
    if points.len() < 3 {
        return None;
    }
    let third = (points.len() / 3).max(1);
    let mean = |s: &[TrendPoint]| s.iter().map(|p| p.complexity).sum::<f64>() / s.len() as f64;
    let first = mean(&points[..third]);
    let last = mean(&points[points.len() - third..]);
    let change = (last - first) / first.max(1.0);
    let refactored = points.windows(2).any(|w| w[1].complexity < w[0].complexity * 0.8);
    let direction = if change > 0.15 {
        "deteriorating"
    } else if change < -0.15 {
        "improving"
    } else {
        "stable"
    };
    Some(Trend { direction: direction.into(), change: (change * 1000.0).round() / 1000.0, refactored, points: points.len() })
}

/// Ensure sampled trend points exist for `file`. Returns the number of new points.
pub fn update(db: &Db, ds: &Dataset, file: u32, cat: &mut CatFile) -> Result<usize> {
    let f = &ds.files[file as usize];
    let samples = ds.cfg.analysis.trend_samples.max(3);
    let commits = &f.commits;
    if commits.len() < 2 {
        return Ok(0);
    }
    let picks: Vec<u32> = if commits.len() <= samples {
        commits.clone()
    } else {
        (0..samples).map(|i| commits[i * (commits.len() - 1) / (samples - 1)]).collect()
    };
    let mut path_st = db.conn.prepare_cached("SELECT path FROM changes WHERE commit_id=?1 AND file_id=?2 LIMIT 1")?;
    let mut exists = db.conn.prepare_cached("SELECT 1 FROM trend_points WHERE file_id=?1 AND commit_id=?2")?;
    let mut added = 0;
    for ci in picks {
        let c = &ds.commits[ci as usize];
        if exists.exists(params![f.db_id, c.db_id])? {
            continue;
        }
        let path: String = match path_st.query_row(params![c.db_id, f.db_id], |r| r.get(0)) {
            Ok(p) => p,
            Err(_) => continue,
        };
        if let Some((blob, _)) = metrics_for(db, cat, &format!("{}:{}", c.sha, path), &path, ds.cfg.analysis.max_file_bytes)? {
            db.conn.execute(
                "INSERT OR IGNORE INTO trend_points(file_id, commit_id, blob) VALUES (?1, ?2, ?3)",
                params![f.db_id, c.db_id, blob],
            )?;
            added += 1;
        }
    }
    Ok(added)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pts(v: &[f64]) -> Vec<TrendPoint> {
        v.iter().enumerate().map(|(i, &c)| TrendPoint { ts: i as i64, complexity: c, loc: 0, max_cc: 0 }).collect()
    }

    #[test]
    fn classifies() {
        assert_eq!(classify(&pts(&[10.0, 12.0, 15.0, 20.0, 25.0, 30.0])).unwrap().direction, "deteriorating");
        assert_eq!(classify(&pts(&[30.0, 30.0, 31.0, 30.0])).unwrap().direction, "stable");
        let t = classify(&pts(&[30.0, 32.0, 35.0, 12.0, 12.0, 13.0])).unwrap();
        assert_eq!(t.direction, "improving");
        assert!(t.refactored);
        assert!(classify(&pts(&[1.0, 2.0])).is_none());
    }
}
