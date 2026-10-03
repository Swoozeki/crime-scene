//! Thin wrappers around the git CLI. All history access goes through git itself so that
//! rename detection, mailmap and object lookup behave exactly like the user's git.

use anyhow::{Context, Result, bail};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

#[derive(Clone, Debug)]
pub struct Git {
    pub dir: PathBuf,
}

impl Git {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    pub fn command(&self) -> Command {
        let mut c = Command::new("git");
        c.arg("-C")
            .arg(&self.dir)
            .args(["-c", "core.quotepath=off", "-c", "diff.renameLimit=10000", "-c", "log.showSignature=false"])
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("LC_ALL", "C");
        c
    }

    /// Run git and return stdout as a string; fails with stderr on non-zero exit.
    pub fn run(&self, args: &[&str]) -> Result<String> {
        let out = self
            .command()
            .args(args)
            .stdin(Stdio::null())
            .output()
            .with_context(|| format!("running git {}", args.join(" ")))?;
        if !out.status.success() {
            bail!(
                "git {} failed in {}: {}",
                args.join(" "),
                self.dir.display(),
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }

    pub fn ok(&self, args: &[&str]) -> bool {
        self.command()
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    pub fn is_repo(dir: &Path) -> bool {
        Git::new(dir).ok(&["rev-parse", "--git-dir"])
    }

    /// The ref to analyze: configured branch, else origin's default branch, else a local default branch, else HEAD.
    pub fn resolve_branch(&self, configured: Option<&str>) -> Result<String> {
        if let Some(b) = configured {
            return Ok(b.to_string());
        }
        if let Ok(r) = self.run(&["symbolic-ref", "--quiet", "--short", "refs/remotes/origin/HEAD"]) {
            let r = r.trim();
            if !r.is_empty() && self.ok(&["rev-parse", "--verify", "--quiet", r]) {
                return Ok(r.to_string());
            }
        }
        // No remote: prefer a stable local default branch over whatever is checked out.
        for b in ["main", "master", "develop", "trunk"] {
            if self.ok(&["rev-parse", "--verify", "--quiet", &format!("refs/heads/{b}")]) {
                return Ok(b.to_string());
            }
        }
        Ok("HEAD".to_string())
    }

    pub fn rev_parse(&self, rev: &str) -> Result<String> {
        Ok(self.run(&["rev-parse", "--verify", &format!("{rev}^{{commit}}")])?.trim().to_string())
    }

    pub fn is_ancestor(&self, ancestor: &str, descendant: &str) -> bool {
        self.ok(&["merge-base", "--is-ancestor", ancestor, descendant])
    }

    /// `(path, blob sha, size)` for every regular file at `rev`.
    pub fn ls_tree(&self, rev: &str) -> Result<Vec<(String, String, u64)>> {
        let out = self.run(&["ls-tree", "-r", "-l", "-z", rev])?;
        let mut v = vec![];
        for entry in out.split('\0').filter(|e| !e.is_empty()) {
            // "<mode> SP <type> SP <object> SP+ <size> TAB <path>"
            let Some((meta, path)) = entry.split_once('\t') else { continue };
            let mut parts = meta.split_whitespace();
            let (_mode, kind, sha, size) = (parts.next(), parts.next(), parts.next(), parts.next());
            if kind != Some("blob") {
                continue; // submodules
            }
            let size = size.and_then(|s| s.parse().ok()).unwrap_or(0);
            v.push((path.to_string(), sha.unwrap_or_default().to_string(), size));
        }
        Ok(v)
    }
}

/// A long-lived `git cat-file --batch` process for fast blob reads.
pub struct CatFile {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl CatFile {
    pub fn new(git: &Git) -> Result<Self> {
        let mut child = git
            .command()
            .args(["cat-file", "--batch"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .context("spawning git cat-file")?;
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        Ok(Self { child, stdin, stdout })
    }

    /// Read an object by sha or `rev:path`. Returns `(sha, bytes)`; `None` if missing.
    pub fn get(&mut self, object: &str) -> Result<Option<(String, Vec<u8>)>> {
        if object.contains('\n') {
            return Ok(None);
        }
        writeln!(self.stdin, "{object}")?;
        self.stdin.flush()?;
        let mut header = String::new();
        self.stdout.read_line(&mut header)?;
        let header = header.trim_end();
        let mut parts = header.split(' ');
        let sha = parts.next().unwrap_or_default().to_string();
        let kind = parts.next().unwrap_or_default();
        if kind == "missing" || kind == "ambiguous" || header.is_empty() {
            return Ok(None);
        }
        let size: usize = parts.next().and_then(|s| s.parse().ok()).context("bad cat-file header")?;
        let mut buf = vec![0u8; size + 1]; // trailing newline
        self.stdout.read_exact(&mut buf)?;
        buf.pop();
        if kind != "blob" {
            return Ok(None);
        }
        Ok(Some((sha, buf)))
    }
}

impl Drop for CatFile {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
