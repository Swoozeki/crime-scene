//! Incremental history ingestion into the SQLite cache.

use crate::Progress;
use crate::git::Git;
use crate::log::{RawCommit, stream_log};
use anyhow::{Result, bail};
use csi_core::{Config, Db, RepoConfig};
use rusqlite::{OptionalExtension, params};
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct TreeEntry {
    pub file_id: i64,
    pub path: String,
    pub blob: String,
    pub size: u64,
}

#[derive(Debug)]
pub struct IngestStats {
    pub repo_id: i64,
    pub head: String,
    pub branch: String,
    pub new_commits: usize,
    pub full: bool,
    pub tree: Vec<TreeEntry>,
}

pub const MAX_MESSAGE: usize = 4000;

pub fn ingest_repo(
    db: &Db,
    cfg: &Config,
    repo: &RepoConfig,
    full_rescan: bool,
    progress: Progress,
) -> Result<IngestStats> {
    let path = cfg.repo_path(repo);
    if !Git::is_repo(&path) {
        bail!("repo `{}`: {} is not a git repository", repo.name, path.display());
    }
    let git = Git::new(&path);
    let branch = git.resolve_branch(repo.branch.as_deref())?;
    let head = git.rev_parse(&branch)?;
    let path_s = path.to_string_lossy().to_string();
    let since = cfg.analysis.since.trim().to_string();

    let row: Option<(i64, String, String, Option<String>, String)> = db
        .conn
        .query_row("SELECT id, path, branch, last_sha, since FROM repos WHERE name=?1", [&repo.name], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
        })
        .optional()?;
    let (repo_id, last) = match row {
        Some((id, p, b, last, s)) => {
            if p != path_s || b != branch || s != since || full_rescan {
                db.clear_repo(id)?;
                db.conn.execute(
                    "UPDATE repos SET path=?2, branch=?3, since=?4 WHERE id=?1",
                    params![id, path_s, branch, since],
                )?;
                (id, None)
            } else {
                (id, last)
            }
        }
        None => {
            db.conn.execute(
                "INSERT INTO repos(name, path, branch, since) VALUES (?1, ?2, ?3, ?4)",
                params![repo.name, path_s, branch, since],
            )?;
            (db.conn.last_insert_rowid(), None)
        }
    };

    if last.as_deref() == Some(head.as_str()) {
        return Ok(IngestStats { repo_id, head, branch, new_commits: 0, full: false, tree: load_tree(db, repo_id)? });
    }
    let full = match &last {
        Some(l) if git.is_ancestor(l, &head) => false,
        Some(_) => {
            progress(&format!("{}: history was rewritten, re-ingesting", repo.name));
            db.clear_repo(repo_id)?;
            true
        }
        None => true,
    };
    let range = match (&last, full) {
        (Some(l), false) => format!("{l}..{head}"),
        _ => head.clone(),
    };
    let since_arg = (!since.is_empty() && since != "all").then(|| format!("{since} ago"));

    let tx = db.conn.unchecked_transaction()?;
    let mut paths: HashMap<String, i64> = {
        let mut st = tx.prepare("SELECT path, id FROM files WHERE repo_id=?1 ORDER BY id")?;
        st.query_map([repo_id], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<_, _>>()?
    };
    let mut idents: HashMap<(String, String), i64> = {
        let mut st = tx.prepare("SELECT name, email, id FROM identities")?;
        st.query_map([], |r| Ok(((r.get(0)?, r.get(1)?), r.get(2)?)))?.collect::<Result<_, _>>()?
    };
    let mut n = 0usize;
    {
        let mut ins_ident = tx.prepare_cached("INSERT INTO identities(name, email) VALUES (?1, ?2)")?;
        let mut ins_commit = tx.prepare_cached(
            "INSERT OR IGNORE INTO commits(repo_id, sha, author_id, ts, message) VALUES (?1, ?2, ?3, ?4, ?5)",
        )?;
        let mut ins_co = tx.prepare_cached("INSERT INTO commit_coauthors(commit_id, identity_id) VALUES (?1, ?2)")?;
        let mut ins_file = tx.prepare_cached("INSERT INTO files(repo_id, path) VALUES (?1, ?2)")?;
        let mut upd_path = tx.prepare_cached("UPDATE files SET path=?2 WHERE id=?1")?;
        let mut ins_change = tx.prepare_cached(
            "INSERT INTO changes(commit_id, file_id, path, added, deleted) VALUES (?1, ?2, ?3, ?4, ?5)",
        )?;

        let mut ident = |name: &str, email: &str| -> rusqlite::Result<i64> {
            let key = (name.to_string(), email.to_lowercase());
            if let Some(id) = idents.get(&key) {
                return Ok(*id);
            }
            ins_ident.execute(params![key.0, key.1])?;
            let id = tx.last_insert_rowid();
            idents.insert(key, id);
            Ok(id)
        };

        let mut handle = |c: RawCommit| -> Result<()> {
            let author = ident(&c.author_name, &c.author_email)?;
            let msg: String = c.message.chars().take(MAX_MESSAGE).collect();
            if ins_commit.execute(params![repo_id, c.sha, author, c.ts, msg])? == 0 {
                return Ok(());
            }
            let commit_id = tx.last_insert_rowid();
            for (name, email) in &c.coauthors {
                let id = ident(name, email)?;
                if id != author {
                    ins_co.execute(params![commit_id, id])?;
                }
            }
            for ch in &c.changes {
                let file_id = match &ch.old_path {
                    Some(old) if old != &ch.path => {
                        let id = match paths.remove(old) {
                            Some(id) => id,
                            None => {
                                ins_file.execute(params![repo_id, ch.path])?;
                                tx.last_insert_rowid()
                            }
                        };
                        upd_path.execute(params![id, ch.path])?;
                        paths.insert(ch.path.clone(), id);
                        id
                    }
                    _ => match paths.get(&ch.path) {
                        Some(id) => *id,
                        None => {
                            ins_file.execute(params![repo_id, ch.path])?;
                            let id = tx.last_insert_rowid();
                            paths.insert(ch.path.clone(), id);
                            id
                        }
                    },
                };
                ins_change.execute(params![
                    commit_id,
                    file_id,
                    ch.path,
                    ch.added.unwrap_or(0),
                    ch.deleted.unwrap_or(0)
                ])?;
            }
            n += 1;
            if n.is_multiple_of(5000) {
                progress(&format!("{}: {} commits", repo.name, n));
            }
            Ok(())
        };
        stream_log(&git, &range, since_arg.as_deref(), &mut handle)?;
    }

    // Current tree snapshot: authoritative for which files are alive.
    let entries = git.ls_tree(&head)?;
    let mut tree = Vec::with_capacity(entries.len());
    {
        let mut ins_file = tx.prepare_cached("INSERT INTO files(repo_id, path) VALUES (?1, ?2)")?;
        let mut alive = tx.prepare_cached("UPDATE files SET alive=1 WHERE id=?1")?;
        let mut ins_tree = tx.prepare_cached("INSERT INTO tree(file_id, repo_id, blob) VALUES (?1, ?2, ?3)")?;
        tx.execute("UPDATE files SET alive=0 WHERE repo_id=?1", [repo_id])?;
        tx.execute("DELETE FROM tree WHERE repo_id=?1", [repo_id])?;
        for (path, blob, size) in entries {
            let file_id = match paths.get(&path) {
                Some(id) => *id,
                None => {
                    ins_file.execute(params![repo_id, path])?;
                    let id = tx.last_insert_rowid();
                    paths.insert(path.clone(), id);
                    id
                }
            };
            alive.execute([file_id])?;
            ins_tree.execute(params![file_id, repo_id, blob])?;
            tree.push(TreeEntry { file_id, path, blob, size });
        }
    }
    tx.execute("UPDATE repos SET last_sha=?2, scanned_at=strftime('%s','now') WHERE id=?1", params![repo_id, head])?;
    tx.commit()?;
    Ok(IngestStats { repo_id, head, branch, new_commits: n, full, tree })
}

fn load_tree(db: &Db, repo_id: i64) -> Result<Vec<TreeEntry>> {
    let mut st = db
        .conn
        .prepare("SELECT t.file_id, f.path, t.blob FROM tree t JOIN files f ON f.id = t.file_id WHERE t.repo_id=?1")?;
    let rows = st
        .query_map([repo_id], |r| Ok(TreeEntry { file_id: r.get(0)?, path: r.get(1)?, blob: r.get(2)?, size: 0 }))?
        .collect::<Result<_, _>>()?;
    Ok(rows)
}
