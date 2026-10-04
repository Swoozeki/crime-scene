//! Streaming parser for `git log --numstat --summary` output.

use crate::git::Git;
use anyhow::{Context, Result, bail};
use std::io::{BufRead, BufReader};
use std::process::Stdio;

pub const RS: u8 = 0x1e;
pub const US: char = '\x1f';

#[derive(Debug, Clone, PartialEq, Default)]
pub struct RawChange {
    pub path: String,
    pub old_path: Option<String>,
    /// `None` for binary files
    pub added: Option<u32>,
    pub deleted: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct RawCommit {
    pub sha: String,
    pub author_name: String,
    pub author_email: String,
    pub ts: i64,
    pub message: String,
    pub coauthors: Vec<(String, String)>,
    pub changes: Vec<RawChange>,
    pub created: Vec<String>,
    pub removed: Vec<String>,
}

pub const LOG_FORMAT: &str = "--format=%x1e%H%x1f%aN%x1f%aE%x1f%at%x1f%B%x1f";

/// Stream commits in `range` (oldest first) to `f`.
pub fn stream_log(
    git: &Git,
    range: &str,
    since: Option<&str>,
    mut f: impl FnMut(RawCommit) -> Result<()>,
) -> Result<usize> {
    let mut cmd = git.command();
    cmd.args(["log", "--no-merges", "--reverse", "-M", "--numstat", "--summary", "--date=unix", LOG_FORMAT]);
    if let Some(s) = since {
        cmd.arg(format!("--since={s}"));
    }
    cmd.arg(range).arg("--");
    let mut child =
        cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().context("spawning git log")?;
    let mut reader = BufReader::with_capacity(1 << 20, child.stdout.take().unwrap());
    let mut buf = Vec::with_capacity(1 << 16);
    let mut n = 0;
    loop {
        buf.clear();
        let read = reader.read_until(RS, &mut buf)?;
        if read == 0 {
            break;
        }
        if buf.last() == Some(&RS) {
            buf.pop();
        }
        if buf.iter().all(|b| b.is_ascii_whitespace()) {
            continue;
        }
        let rec = String::from_utf8_lossy(&buf);
        if let Some(c) = parse_record(&rec) {
            f(c)?;
            n += 1;
        }
    }
    let out = child.wait_with_output()?;
    if !out.status.success() {
        bail!("git log failed: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(n)
}

pub fn parse_record(rec: &str) -> Option<RawCommit> {
    let mut parts = rec.splitn(6, US);
    let sha = parts.next()?.trim().to_string();
    let author_name = parts.next()?.to_string();
    let author_email = parts.next()?.to_string();
    let ts = parts.next()?.trim().parse().ok()?;
    let message = parts.next()?.trim().to_string();
    let rest = parts.next().unwrap_or("");
    if sha.len() < 7 {
        return None;
    }
    let mut c = RawCommit {
        sha,
        author_name,
        author_email,
        ts,
        coauthors: parse_coauthors(&message),
        message,
        ..Default::default()
    };
    for line in rest.lines() {
        if line.is_empty() {
            continue;
        }
        if let Some(s) = line.strip_prefix(" create mode ") {
            if let Some((_, p)) = s.split_once(' ') {
                c.created.push(unquote(p));
            }
        } else if let Some(s) = line.strip_prefix(" delete mode ") {
            if let Some((_, p)) = s.split_once(' ') {
                c.removed.push(unquote(p));
            }
        } else if line.starts_with(' ') {
            // rename/copy/mode change summaries: renames come from numstat
        } else if let Some(ch) = parse_numstat(line) {
            c.changes.push(ch);
        }
    }
    Some(c)
}

fn parse_numstat(line: &str) -> Option<RawChange> {
    let mut it = line.splitn(3, '\t');
    let a = it.next()?;
    let d = it.next()?;
    let p = it.next()?;
    let added = if a == "-" { None } else { Some(a.parse().ok()?) };
    let deleted = if d == "-" { None } else { Some(d.parse().ok()?) };
    let (old_path, path) = split_rename(&unquote(p));
    Some(RawChange { path, old_path, added, deleted })
}

/// Expand numstat rename notation: `a/{b => c}/d` or `old => new`.
pub fn split_rename(p: &str) -> (Option<String>, String) {
    if let (Some(open), Some(close)) = (p.find('{'), p.rfind('}'))
        && open < close
    {
        let inner = &p[open + 1..close];
        if let Some((from, to)) = inner.split_once(" => ") {
            let pre = &p[..open];
            let post = &p[close + 1..];
            let join = |mid: &str| {
                let s = format!("{pre}{mid}{post}");
                s.replace("//", "/").trim_start_matches('/').to_string()
            };
            return (Some(join(from)), join(to));
        }
    }
    if let Some((from, to)) = p.split_once(" => ") {
        return (Some(from.to_string()), to.to_string());
    }
    (None, p.to_string())
}

/// Undo git's C-style quoting of unusual paths.
pub fn unquote(p: &str) -> String {
    let p = p.trim_end();
    if !(p.starts_with('"') && p.ends_with('"') && p.len() >= 2) {
        return p.to_string();
    }
    let inner = &p[1..p.len() - 1];
    let mut bytes = Vec::with_capacity(inner.len());
    let mut chars = inner.bytes().peekable();
    while let Some(b) = chars.next() {
        if b != b'\\' {
            bytes.push(b);
            continue;
        }
        match chars.next() {
            Some(b'n') => bytes.push(b'\n'),
            Some(b't') => bytes.push(b'\t'),
            Some(b'"') => bytes.push(b'"'),
            Some(b'\\') => bytes.push(b'\\'),
            Some(d @ b'0'..=b'7') => {
                let mut v = (d - b'0') as u32;
                for _ in 0..2 {
                    if let Some(&n @ b'0'..=b'7') = chars.peek() {
                        v = v * 8 + (n - b'0') as u32;
                        chars.next();
                    }
                }
                bytes.push(v as u8);
            }
            Some(o) => bytes.push(o),
            None => {}
        }
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

fn parse_coauthors(message: &str) -> Vec<(String, String)> {
    message
        .lines()
        .filter_map(|l| {
            let l = l.trim();
            let rest = l.get(..15).filter(|p| p.eq_ignore_ascii_case("co-authored-by:")).map(|_| &l[15..])?;
            let rest = rest.trim();
            let (name, email) = match rest.split_once('<') {
                Some((n, e)) => (n.trim().to_string(), e.trim_end_matches('>').trim().to_string()),
                None => (rest.to_string(), String::new()),
            };
            (!name.is_empty()).then_some((name, email))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renames() {
        assert_eq!(split_rename("src/{old => new}/x.ts"), (Some("src/old/x.ts".into()), "src/new/x.ts".into()));
        assert_eq!(split_rename("src/{ => sub}/x.ts"), (Some("src/x.ts".into()), "src/sub/x.ts".into()));
        assert_eq!(split_rename("{a => b}/x.ts"), (Some("a/x.ts".into()), "b/x.ts".into()));
        assert_eq!(split_rename("a.ts => b.ts"), (Some("a.ts".into()), "b.ts".into()));
        assert_eq!(split_rename("plain.ts"), (None, "plain.ts".into()));
    }

    #[test]
    fn quoted_paths() {
        assert_eq!(unquote("\"a\\tb.ts\""), "a\tb.ts");
        assert_eq!(unquote("\"caf\\303\\251.ts\""), "café.ts");
    }

    #[test]
    fn record() {
        let rec = "abc1234def\x1fJane\x1fjane@x.com\x1f1700000000\x1fABC-12 fix cart\n\nCo-authored-by: Raj K <raj@x.com>\x1f\n\n3\t1\tsrc/a.ts\n-\t-\tlogo.png\n0\t0\tsrc/{x => y}/b.ts\n create mode 100644 src/a.ts\n delete mode 100644 old.ts\n";
        let c = parse_record(rec).unwrap();
        assert_eq!(c.sha, "abc1234def");
        assert_eq!(c.ts, 1700000000);
        assert_eq!(c.coauthors, vec![("Raj K".into(), "raj@x.com".into())]);
        assert_eq!(c.changes.len(), 3);
        assert_eq!(c.changes[1].added, None);
        assert_eq!(c.changes[2].old_path.as_deref(), Some("src/x/b.ts"));
        assert_eq!(c.created, vec!["src/a.ts"]);
        assert_eq!(c.removed, vec!["old.ts"]);
    }
}
