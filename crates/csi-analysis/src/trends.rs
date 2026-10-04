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
    /// relative change of complexity over the last year
    pub change: f64,
    pub refactored: bool,
    pub points: usize,
}

/// Trend over the last year: complexity now vs. ~one year ago (or the first sample if the file
/// is younger). A file that grew from nothing years ago is not "deteriorating" today.
pub fn classify(points: &[TrendPoint], now: i64) -> Option<Trend> {
    if points.len() < 3 {
        return None;
    }
    let year_ago = now - 365 * 86_400;
    let baseline = points.iter().rev().find(|p| p.ts <= year_ago).unwrap_or(&points[0]);
    let recent: Vec<&TrendPoint> = points.iter().filter(|p| p.ts >= baseline.ts).collect();
    let last = points[points.len() - 1].complexity;
    let delta = last - baseline.complexity;
    let change = delta / baseline.complexity.max(10.0);
    let refactored = recent.windows(2).any(|w| w[1].complexity < w[0].complexity * 0.8);
    let direction = if change > 0.25 && delta >= 5.0 {
        "deteriorating"
    } else if change < -0.15 {
        "improving"
    } else {
        "stable"
    };
    Some(Trend { direction: direction.into(), change: (change * 1000.0).round() / 1000.0, refactored, points: recent.len() })
}

/// Ensure sampled trend points exist for `file`. Returns the number of new points.
pub fn update(db: &Db, ds: &Dataset, file: u32, cat: &mut CatFile) -> Result<usize> {
    let f = &ds.files[file as usize];
    let samples = ds.cfg.analysis.trend_samples.max(3);
    let commits = &f.commits;
    if commits.len() < 2 {
        return Ok(0);
    }
    // evenly over the whole life, plus denser sampling of the last year
    let even = |cs: &[u32], n: usize| -> Vec<u32> {
        if cs.len() <= n { cs.to_vec() } else { (0..n).map(|i| cs[i * (cs.len() - 1) / (n - 1).max(1)]).collect() }
    };
    let year_ago = ds.now - 365 * 86_400;
    let recent: Vec<u32> = commits.iter().copied().filter(|&c| ds.commits[c as usize].ts >= year_ago).collect();
    let mut picks = even(commits, samples);
    picks.extend(even(&recent, samples / 2 + 1));
    if let Some(&before) = commits.iter().rev().find(|&&c| ds.commits[c as usize].ts < year_ago) {
        picks.push(before);
    }
    picks.sort_unstable();
    picks.dedup();
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
        assert_eq!(classify(&pts(&[10.0, 12.0, 15.0, 20.0, 25.0, 30.0]), 10).unwrap().direction, "deteriorating");
        assert_eq!(classify(&pts(&[30.0, 30.0, 31.0, 30.0]), 10).unwrap().direction, "stable");
        let t = classify(&pts(&[30.0, 32.0, 35.0, 12.0, 12.0, 13.0]), 10).unwrap();
        assert_eq!(t.direction, "improving");
        assert!(t.refactored);
        assert!(classify(&pts(&[1.0, 2.0]), 10).is_none());
    }

    #[test]
    fn old_growth_is_not_deterioration() {
        let day = 86_400;
        let now = 1000 * day;
        // grew years ago, flat over the last year
        let p: Vec<TrendPoint> = [(0, 5.0), (200, 40.0), (500, 60.0), (700, 61.0), (900, 62.0), (1000, 62.0)]
            .iter()
            .map(|&(d, c)| TrendPoint { ts: d * day, complexity: c, loc: 0, max_cc: 0 })
            .collect();
        assert_eq!(classify(&p, now).unwrap().direction, "stable");
    }
}
