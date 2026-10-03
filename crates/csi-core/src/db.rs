//! SQLite cache. The cache is disposable: a schema version bump simply rebuilds it.

use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, params};
use std::path::Path;

pub const SCHEMA_VERSION: i64 = 3;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS repos (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    path TEXT NOT NULL,
    branch TEXT NOT NULL DEFAULT '',
    last_sha TEXT,
    since TEXT NOT NULL DEFAULT '',
    scanned_at INTEGER,
    arch TEXT
);
CREATE TABLE IF NOT EXISTS identities (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    email TEXT NOT NULL,
    UNIQUE(name, email)
);
CREATE TABLE IF NOT EXISTS commits (
    id INTEGER PRIMARY KEY,
    repo_id INTEGER NOT NULL,
    sha TEXT NOT NULL,
    author_id INTEGER NOT NULL,
    ts INTEGER NOT NULL,
    message TEXT NOT NULL,
    UNIQUE(repo_id, sha)
);
CREATE INDEX IF NOT EXISTS commits_repo_ts ON commits(repo_id, ts);
CREATE TABLE IF NOT EXISTS commit_coauthors (commit_id INTEGER NOT NULL, identity_id INTEGER NOT NULL);
CREATE INDEX IF NOT EXISTS coauthors_commit ON commit_coauthors(commit_id);
CREATE TABLE IF NOT EXISTS files (
    id INTEGER PRIMARY KEY,
    repo_id INTEGER NOT NULL,
    path TEXT NOT NULL,
    alive INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS files_repo_path ON files(repo_id, path);
CREATE TABLE IF NOT EXISTS changes (
    commit_id INTEGER NOT NULL,
    file_id INTEGER NOT NULL,
    path TEXT NOT NULL,
    added INTEGER NOT NULL,
    deleted INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS changes_file ON changes(file_id);
CREATE INDEX IF NOT EXISTS changes_commit ON changes(commit_id);
CREATE TABLE IF NOT EXISTS tree (
    file_id INTEGER PRIMARY KEY,
    repo_id INTEGER NOT NULL,
    blob TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS blob_metrics (
    blob TEXT PRIMARY KEY,
    lang TEXT NOT NULL,
    loc INTEGER NOT NULL,
    complexity REAL NOT NULL,
    max_cc INTEGER NOT NULL,
    generated INTEGER NOT NULL,
    summary TEXT NOT NULL,
    functions TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS xray_done (
    file_id INTEGER NOT NULL,
    commit_id INTEGER NOT NULL,
    PRIMARY KEY (file_id, commit_id)
);
CREATE TABLE IF NOT EXISTS function_changes (
    file_id INTEGER NOT NULL,
    commit_id INTEGER NOT NULL,
    fn TEXT NOT NULL,
    added INTEGER NOT NULL,
    deleted INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS fnchanges_file ON function_changes(file_id);
CREATE TABLE IF NOT EXISTS trend_points (
    file_id INTEGER NOT NULL,
    commit_id INTEGER NOT NULL,
    blob TEXT NOT NULL,
    PRIMARY KEY (file_id, commit_id)
);
"#;

pub struct Db {
    pub conn: Connection,
}

impl Db {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        let conn = Connection::open(path).with_context(|| format!("opening {}", path.display()))?;
        Self::init(conn)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA temp_store=MEMORY;
             PRAGMA cache_size=-65536; PRAGMA busy_timeout=10000;",
        )?;
        let db = Db { conn };
        let version: Option<i64> = db
            .conn
            .query_row("SELECT value FROM meta WHERE key='schema_version'", [], |r| {
                r.get::<_, String>(0)
            })
            .optional()
            .unwrap_or(None)
            .and_then(|v| v.parse().ok());
        if version != Some(SCHEMA_VERSION) {
            db.reset()?;
        }
        Ok(db)
    }

    /// Drop everything and recreate the schema.
    pub fn reset(&self) -> Result<()> {
        let tables: Vec<String> = {
            let mut st = self
                .conn
                .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'")?;
            st.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?
        };
        for t in tables {
            self.conn.execute_batch(&format!("DROP TABLE IF EXISTS \"{t}\""))?;
        }
        self.conn.execute_batch(SCHEMA)?;
        self.conn.execute(
            "INSERT OR REPLACE INTO meta(key, value) VALUES ('schema_version', ?1)",
            params![SCHEMA_VERSION.to_string()],
        )?;
        Ok(())
    }

    /// Remove all history for one repo (used for full rescans and force-pushes).
    pub fn clear_repo(&self, repo_id: i64) -> Result<()> {
        self.conn.execute_batch(&format!(
            "DELETE FROM changes WHERE commit_id IN (SELECT id FROM commits WHERE repo_id={repo_id});
             DELETE FROM commit_coauthors WHERE commit_id IN (SELECT id FROM commits WHERE repo_id={repo_id});
             DELETE FROM function_changes WHERE file_id IN (SELECT id FROM files WHERE repo_id={repo_id});
             DELETE FROM xray_done WHERE file_id IN (SELECT id FROM files WHERE repo_id={repo_id});
             DELETE FROM trend_points WHERE file_id IN (SELECT id FROM files WHERE repo_id={repo_id});
             DELETE FROM tree WHERE repo_id={repo_id};
             DELETE FROM commits WHERE repo_id={repo_id};
             DELETE FROM files WHERE repo_id={repo_id};
             UPDATE repos SET last_sha=NULL WHERE id={repo_id};"
        ))?;
        Ok(())
    }

    pub fn meta_get(&self, key: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row("SELECT value FROM meta WHERE key=?1", [key], |r| r.get(0))
            .optional()?)
    }

    pub fn meta_set(&self, key: &str, value: &str) -> Result<()> {
        self.conn
            .execute("INSERT OR REPLACE INTO meta(key, value) VALUES (?1, ?2)", params![key, value])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_schema() {
        let db = Db::open_in_memory().unwrap();
        assert_eq!(db.meta_get("schema_version").unwrap().as_deref(), Some("3"));
        db.conn.execute("INSERT INTO repos(name, path) VALUES ('a', '/a')", []).unwrap();
        db.reset().unwrap();
        let n: i64 = db.conn.query_row("SELECT count(*) FROM repos", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 0);
    }
}
