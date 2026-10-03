//! Scripted git repositories for tests: deterministic authors, dates and file operations.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Fixed "now" for fixtures (2025-06-15) so decay maths is deterministic.
pub const BASE_TS: i64 = 1_750_000_000;

pub enum Op<'a> {
    Write(&'a str, &'a str),
    Delete(&'a str),
    Move(&'a str, &'a str),
}

pub struct FixtureRepo {
    _tmp: Option<tempfile::TempDir>,
    pub dir: PathBuf,
}

impl FixtureRepo {
    pub fn new() -> Self {
        let tmp = tempfile::tempdir().expect("tempdir");
        let dir = tmp.path().join("repo");
        let mut r = Self::at(&dir);
        r._tmp = Some(tmp);
        r
    }

    /// Create a repo inside an existing directory (for multi-repo workspaces).
    pub fn at(dir: &Path) -> Self {
        std::fs::create_dir_all(dir).unwrap();
        let r = Self { _tmp: None, dir: dir.to_path_buf() };
        r.git(&["init", "-q", "-b", "main"]);
        r.git(&["config", "user.name", "Fixture"]);
        r.git(&["config", "user.email", "fixture@example.com"]);
        r.git(&["config", "commit.gpgsign", "false"]);
        r
    }

    pub fn git(&self, args: &[&str]) -> String {
        let out = Command::new("git").arg("-C").arg(&self.dir).args(args).output().expect("git");
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    /// Commit `ops` as `author` ("Name <email>") `days_ago` days before [`BASE_TS`].
    pub fn commit(&self, author: &str, days_ago: f64, message: &str, ops: &[Op]) -> String {
        for op in ops {
            match op {
                Op::Write(p, content) => {
                    let full = self.dir.join(p);
                    std::fs::create_dir_all(full.parent().unwrap()).unwrap();
                    std::fs::write(full, content).unwrap();
                    self.git(&["add", "--", p]);
                }
                Op::Delete(p) => {
                    self.git(&["rm", "-q", "--", p]);
                }
                Op::Move(from, to) => {
                    let full = self.dir.join(to);
                    std::fs::create_dir_all(full.parent().unwrap()).unwrap();
                    self.git(&["mv", from, to]);
                }
            }
        }
        let ts = BASE_TS - (days_ago * 86400.0) as i64;
        let date = format!("@{ts} +0000");
        let (name, email) = author.split_once('<').map(|(n, e)| (n.trim(), e.trim_end_matches('>'))).unwrap();
        let out = Command::new("git")
            .arg("-C")
            .arg(&self.dir)
            .args(["commit", "-q", "--allow-empty", "-m", message])
            .env("GIT_AUTHOR_NAME", name)
            .env("GIT_AUTHOR_EMAIL", email)
            .env("GIT_AUTHOR_DATE", &date)
            .env("GIT_COMMITTER_NAME", name)
            .env("GIT_COMMITTER_EMAIL", email)
            .env("GIT_COMMITTER_DATE", &date)
            .output()
            .unwrap();
        assert!(out.status.success(), "commit: {}", String::from_utf8_lossy(&out.stderr));
        self.git(&["rev-parse", "HEAD"]).trim().to_string()
    }
}

impl Default for FixtureRepo {
    fn default() -> Self {
        Self::new()
    }
}

/// Generate TypeScript with `n` if-branches so complexity is controllable.
pub fn ts_with_branches(fn_name: &str, n: usize, salt: usize) -> String {
    let mut s = format!("export function {fn_name}(x: number) {{\n  let r = {salt};\n");
    for i in 0..n {
        s.push_str(&format!("  if (x === {i}) {{ r += {i}; }}\n"));
    }
    s.push_str("  return r;\n}\n");
    s
}
