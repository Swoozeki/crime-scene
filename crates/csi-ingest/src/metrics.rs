//! Complexity metrics for the current tree, cached by blob sha so unchanged files are never re-parsed.

use crate::Progress;
use crate::git::CatFile;
use crate::ingest::TreeEntry;
use anyhow::Result;
use csi_core::{Config, Db};
use csi_lang::{FileMetrics, Function, Lang, Summary};
use rayon::prelude::*;
use rusqlite::{OptionalExtension, params};

const CHUNK: usize = 1000;

/// Parse every alive, non-excluded file whose blob has no cached metrics.
pub fn analyze_tree(db: &Db, cfg: &Config, cat: &mut CatFile, tree: &[TreeEntry], progress: Progress) -> Result<usize> {
    let exclude = cfg.exclude_set()?;
    let max = cfg.analysis.max_file_bytes;
    let missing: Vec<&TreeEntry> = {
        let mut st = db.conn.prepare_cached("SELECT 1 FROM blob_metrics WHERE blob=?1")?;
        let mut v = vec![];
        for e in tree.iter().filter(|e| !exclude.is_match(&e.path)) {
            if st.query_row([&e.blob], |_| Ok(())).optional()?.is_none() {
                v.push(e);
            }
        }
        v.sort_by(|a, b| a.blob.cmp(&b.blob));
        v.dedup_by(|a, b| a.blob == b.blob);
        v
    };
    let total = missing.len();
    let mut done = 0;
    for chunk in missing.chunks(CHUNK) {
        let mut contents = Vec::with_capacity(chunk.len());
        for e in chunk {
            let data = if e.size as usize > max * 4 { None } else { cat.get(&e.blob)?.map(|(_, d)| d) };
            contents.push((e, data));
        }
        let results: Vec<(&str, Option<FileMetrics>, bool)> = contents
            .par_iter()
            .map(|(e, data)| match data {
                Some(d) => (e.blob.as_str(), csi_lang::analyze(&e.path, d, max), false),
                None => (e.blob.as_str(), None, e.size as usize > max * 4),
            })
            .collect();
        let tx = db.conn.unchecked_transaction()?;
        for (blob, m, too_large) in results {
            store(&tx, blob, m.as_ref(), too_large)?;
        }
        tx.commit()?;
        done += chunk.len();
        if total > CHUNK {
            progress(&format!("parsed {done}/{total} files"));
        }
    }
    Ok(total)
}

fn store(conn: &rusqlite::Connection, blob: &str, m: Option<&FileMetrics>, too_large: bool) -> Result<()> {
    let mut st = conn.prepare_cached(
        "INSERT OR REPLACE INTO blob_metrics(blob, lang, loc, complexity, max_cc, generated, summary, functions)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
    )?;
    match m {
        Some(m) => st.execute(params![
            blob,
            m.lang.as_str(),
            m.loc,
            m.complexity,
            m.max_cc,
            m.generated,
            serde_json::to_string(&m.summary)?,
            serde_json::to_string(&m.functions)?
        ])?,
        // binary or oversized: remember it so we never fetch it again
        None => st.execute(params![
            blob,
            if too_large { "other" } else { "binary" },
            0,
            0.0,
            0,
            true,
            serde_json::to_string(&Summary { health: 10.0, ..Default::default() })?,
            "[]"
        ])?,
    };
    Ok(())
}

/// Metrics for an object (`sha` or `rev:path`), using and filling the blob cache.
pub fn metrics_for(
    db: &Db,
    cat: &mut CatFile,
    object: &str,
    path: &str,
    max_bytes: usize,
) -> Result<Option<(String, FileMetrics)>> {
    if object.len() == 40
        && !object.contains(':')
        && let Some(m) = load_metrics(db, object)?
    {
        return Ok(Some((object.to_string(), m)));
    }
    let Some((sha, data)) = cat.get(object)? else { return Ok(None) };
    if let Some(m) = load_metrics(db, &sha)? {
        return Ok(Some((sha, m)));
    }
    let m = csi_lang::analyze(path, &data, max_bytes);
    store(&db.conn, &sha, m.as_ref(), false)?;
    Ok(m.map(|m| (sha, m)))
}

pub fn load_metrics(db: &Db, blob: &str) -> Result<Option<FileMetrics>> {
    let row = db
        .conn
        .prepare_cached(
            "SELECT lang, loc, complexity, max_cc, generated, summary, functions FROM blob_metrics WHERE blob=?1",
        )?
        .query_row([blob], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, u32>(1)?,
                r.get::<_, f64>(2)?,
                r.get::<_, u32>(3)?,
                r.get::<_, bool>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, String>(6)?,
            ))
        })
        .optional()?;
    let Some((lang, loc, complexity, max_cc, generated, summary, functions)) = row else { return Ok(None) };
    if lang == "binary" {
        return Ok(None);
    }
    Ok(Some(FileMetrics {
        lang: Lang::parse(&lang),
        loc,
        complexity,
        max_cc,
        generated,
        summary: serde_json::from_str(&summary).unwrap_or_default(),
        functions: serde_json::from_str(&functions).unwrap_or_default(),
    }))
}

pub fn load_functions(db: &Db, blob: &str) -> Result<Vec<Function>> {
    Ok(load_metrics(db, blob)?.map(|m| m.functions).unwrap_or_default())
}
