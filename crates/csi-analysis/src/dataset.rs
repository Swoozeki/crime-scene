//! In-memory view of the cache: commits, files, entities, units and authors with all
//! configuration-dependent cleaning (bots, aliases, excludes, mega and format commits) applied.

use crate::identity::{self, Identity};
use crate::scope::Level;
use anyhow::Result;
use csi_arch::{RepoArch, UnitIndex, entity_key};
use csi_core::util::percentile_ranks;
use csi_core::{Config, Db};
use csi_lang::{Lang, Summary};
use regex::Regex;
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::LazyLock;

#[derive(Debug, Clone, Serialize)]
pub struct RepoInfo {
    pub db_id: i64,
    pub name: String,
    pub path: PathBuf,
    pub branch: String,
    pub head: Option<String>,
    pub frameworks: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Author {
    pub name: String,
    pub emails: Vec<String>,
    pub team: Option<String>,
    pub is_bot: bool,
    pub first_ts: i64,
    pub last_ts: i64,
    pub commits: u32,
}

#[derive(Debug, Clone, Copy)]
pub struct Change {
    pub file: u32,
    pub added: u32,
    pub deleted: u32,
}

#[derive(Debug, Clone)]
pub struct Commit {
    pub db_id: i64,
    pub repo: u32,
    pub sha: String,
    pub author: u32,
    pub coauthors: Vec<u32>,
    pub ts: i64,
    pub subject: String,
    pub tickets: Vec<u32>,
    pub defect: bool,
    /// formatting-only sweep
    pub format: bool,
    /// touches more than `max_commit_files`
    pub mega: bool,
    /// revision weight (1, 0.25 for mega, 0 for format)
    pub weight: f64,
    pub changes: Vec<Change>,
}

impl Commit {
    /// Eligible for change coupling.
    pub fn couples(&self) -> bool {
        !self.mega && !self.format
    }

    pub fn authors(&self) -> impl Iterator<Item = u32> + '_ {
        std::iter::once(self.author).chain(self.coauthors.iter().copied())
    }
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct TrendPoint {
    pub ts: i64,
    pub complexity: f64,
    pub loc: u32,
    pub max_cc: u32,
}

#[derive(Debug, Clone)]
pub struct File {
    pub db_id: i64,
    pub repo: u32,
    pub path: String,
    pub alive: bool,
    pub test: bool,
    pub blob: Option<String>,
    pub lang: Lang,
    pub loc: u32,
    pub complexity: f64,
    pub max_cc: u32,
    /// language-normalized complexity in 0..=1 (percentile within language × language weight)
    pub norm_cx: f64,
    pub summary: Summary,
    pub entity: u32,
    pub unit: u32,
    /// indexes into `Dataset::commits`, oldest first
    pub commits: Vec<u32>,
    pub trend: Vec<TrendPoint>,
}

impl File {
    pub fn health(&self) -> f64 {
        if self.summary.health == 0.0 { 10.0 } else { self.summary.health }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Entity {
    pub key: String,
    pub name: String,
    pub repo: u32,
    pub unit: u32,
    pub kind: String,
    pub files: Vec<u32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct UnitInfo {
    pub name: String,
    pub repo: u32,
    pub kind: String,
    pub root: String,
    pub tags: Vec<String>,
}

pub struct Dataset {
    pub cfg: Config,
    /// Reference time: the newest commit in the workspace.
    pub now: i64,
    pub repos: Vec<RepoInfo>,
    pub authors: Vec<Author>,
    pub commits: Vec<Commit>,
    pub files: Vec<File>,
    pub entities: Vec<Entity>,
    pub units: Vec<UnitInfo>,
    pub tickets: Vec<String>,
    pub ticket_commits: Vec<Vec<u32>>,
    pub file_by_db: HashMap<i64, u32>,
    pub file_by_path: HashMap<(u32, String), u32>,
    pub commit_by_sha: HashMap<(u32, String), u32>,
}

static FORMAT_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(format(ting|ted)?|prettier|lint(ing)?|eslint|reformat|whitespace|code ?style|indentation)\b")
        .unwrap()
});

impl Dataset {
    pub fn load(cfg: &Config, db: &Db) -> Result<Self> {
        let exclude = cfg.exclude_set()?;
        let ticket_re = cfg.ticket_regex();
        let defect_re = cfg.defect_regex();

        // --- repos (only those in the config, in config order)
        let mut repos = vec![];
        let mut repo_idx: HashMap<i64, u32> = HashMap::new();
        let mut arches: Vec<RepoArch> = vec![];
        for rc in &cfg.repos {
            let row = db.conn.query_row(
                "SELECT id, path, branch, last_sha, arch FROM repos WHERE name=?1",
                [&rc.name],
                |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, Option<String>>(3)?,
                        r.get::<_, Option<String>>(4)?,
                    ))
                },
            );
            let Ok((id, path, branch, head, arch)) = row else { continue };
            let arch: RepoArch = arch.and_then(|a| serde_json::from_str(&a).ok()).unwrap_or_default();
            repo_idx.insert(id, repos.len() as u32);
            repos.push(RepoInfo {
                db_id: id,
                name: rc.name.clone(),
                path: PathBuf::from(path),
                branch,
                head,
                frameworks: arch.frameworks.clone(),
            });
            arches.push(arch);
        }

        // --- authors
        let idents: Vec<Identity> = {
            let mut st = db.conn.prepare(
                "SELECT i.id, i.name, i.email, (SELECT count(*) FROM commits c WHERE c.author_id=i.id) FROM identities i",
            )?;
            st.query_map([], |r| Ok(Identity { id: r.get(0)?, name: r.get(1)?, email: r.get(2)?, commits: r.get(3)? }))?
                .collect::<Result<_, _>>()?
        };
        let (ident_map, resolved) = identity::resolve(&idents, cfg);
        let mut authors: Vec<Author> = resolved
            .into_iter()
            .map(|r| Author {
                name: r.name,
                emails: r.emails,
                team: r.team,
                is_bot: r.is_bot,
                first_ts: i64::MAX,
                last_ts: 0,
                commits: 0,
            })
            .collect();

        // --- files (excluded paths never enter the dataset)
        let mut files: Vec<File> = vec![];
        let mut file_by_db: HashMap<i64, u32> = HashMap::new();
        {
            let mut st = db.conn.prepare("SELECT id, repo_id, path, alive FROM files ORDER BY id")?;
            let rows = st.query_map([], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, String>(2)?, r.get::<_, bool>(3)?))
            })?;
            for row in rows {
                let (id, repo_id, path, alive) = row?;
                let Some(&repo) = repo_idx.get(&repo_id) else { continue };
                if exclude.is_match(&path) {
                    continue;
                }
                file_by_db.insert(id, files.len() as u32);
                files.push(File {
                    db_id: id,
                    repo,
                    test: csi_lang::is_test_path(&path),
                    lang: csi_lang::detect(&path),
                    path,
                    alive,
                    blob: None,
                    loc: 0,
                    complexity: 0.0,
                    max_cc: 0,
                    norm_cx: 0.0,
                    summary: Summary::default(),
                    entity: 0,
                    unit: 0,
                    commits: vec![],
                    trend: vec![],
                });
            }
        }
        // current metrics; drop generated and binary files
        {
            let mut st = db.conn.prepare(
                "SELECT t.file_id, t.blob, b.lang, b.loc, b.complexity, b.max_cc, b.generated, b.summary
                 FROM tree t LEFT JOIN blob_metrics b ON b.blob = t.blob",
            )?;
            let rows = st.query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, Option<u32>>(3)?,
                    r.get::<_, Option<f64>>(4)?,
                    r.get::<_, Option<u32>>(5)?,
                    r.get::<_, Option<bool>>(6)?,
                    r.get::<_, Option<String>>(7)?,
                ))
            })?;
            for row in rows {
                let (fid, blob, lang, loc, cx, max_cc, generated, summary) = row?;
                let Some(&i) = file_by_db.get(&fid) else { continue };
                let f = &mut files[i as usize];
                f.blob = Some(blob);
                if lang.as_deref() == Some("binary") || generated == Some(true) {
                    file_by_db.remove(&fid); // excluded below
                    continue;
                }
                if let Some(l) = lang {
                    f.lang = Lang::parse(&l);
                }
                f.loc = loc.unwrap_or(0);
                f.complexity = cx.unwrap_or(0.0);
                f.max_cc = max_cc.unwrap_or(0);
                f.summary = summary.and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
            }
        }
        // compact out files removed above
        if file_by_db.len() != files.len() {
            let keep: std::collections::HashSet<i64> = file_by_db.keys().copied().collect();
            files.retain(|f| keep.contains(&f.db_id));
            file_by_db = files.iter().enumerate().map(|(i, f)| (f.db_id, i as u32)).collect();
        }

        // --- commits + changes
        let mut commits: Vec<Commit> = vec![];
        let mut commit_by_db: HashMap<i64, u32> = HashMap::new();
        let mut tickets: Vec<String> = vec![];
        let mut ticket_idx: HashMap<String, u32> = HashMap::new();
        let coauthors: HashMap<i64, Vec<i64>> = {
            let mut m: HashMap<i64, Vec<i64>> = HashMap::new();
            let mut st = db.conn.prepare("SELECT commit_id, identity_id FROM commit_coauthors")?;
            for row in st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))? {
                let (c, i) = row?;
                m.entry(c).or_default().push(i);
            }
            m
        };
        {
            let mut st = db.conn.prepare("SELECT id, repo_id, sha, author_id, ts, message FROM commits ORDER BY ts, id")?;
            let rows = st.query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, i64>(3)?,
                    r.get::<_, i64>(4)?,
                    r.get::<_, String>(5)?,
                ))
            })?;
            for row in rows {
                let (id, repo_id, sha, author_id, ts, message) = row?;
                let Some(&repo) = repo_idx.get(&repo_id) else { continue };
                let Some(&author) = ident_map.get(&author_id) else { continue };
                if authors[author as usize].is_bot {
                    continue;
                }
                let subject = message.lines().next().unwrap_or("").trim().to_string();
                let mut tks: Vec<u32> = vec![];
                for m in ticket_re.find_iter(&message) {
                    let t = m.as_str().to_string();
                    let next = tickets.len() as u32;
                    let idx = *ticket_idx.entry(t.clone()).or_insert_with(|| {
                        tickets.push(t);
                        next
                    });
                    if !tks.contains(&idx) {
                        tks.push(idx);
                    }
                }
                let co: Vec<u32> = coauthors
                    .get(&id)
                    .map(|v| v.iter().filter_map(|i| ident_map.get(i).copied()).filter(|&a| a != author).collect())
                    .unwrap_or_default();
                commit_by_db.insert(id, commits.len() as u32);
                commits.push(Commit {
                    db_id: id,
                    repo,
                    sha,
                    author,
                    coauthors: co,
                    ts,
                    defect: defect_re.is_match(&subject),
                    format: FORMAT_RE.is_match(&subject),
                    subject,
                    tickets: tks,
                    mega: false,
                    weight: 1.0,
                    changes: vec![],
                });
            }
        }
        {
            let mut st = db.conn.prepare("SELECT commit_id, file_id, added, deleted FROM changes")?;
            let rows = st.query_map([], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, u32>(2)?, r.get::<_, u32>(3)?))
            })?;
            for row in rows {
                let (cid, fid, added, deleted) = row?;
                let (Some(&c), Some(&f)) = (commit_by_db.get(&cid), file_by_db.get(&fid)) else { continue };
                commits[c as usize].changes.push(Change { file: f, added, deleted });
            }
        }
        // drop commits that touch nothing analyzable; classify the rest
        let max_files = cfg.analysis.max_commit_files;
        commits.retain(|c| !c.changes.is_empty());
        let mut commit_by_sha = HashMap::new();
        for (ci, c) in commits.iter_mut().enumerate() {
            c.format = c.format && c.changes.len() >= 10;
            c.mega = c.changes.len() > max_files;
            c.weight = if c.format { 0.0 } else if c.mega { 0.25 } else { 1.0 };
            for ch in &c.changes {
                files[ch.file as usize].commits.push(ci as u32);
            }
            for a in c.authors().collect::<Vec<_>>() {
                let au = &mut authors[a as usize];
                au.commits += 1;
                au.first_ts = au.first_ts.min(c.ts);
                au.last_ts = au.last_ts.max(c.ts);
            }
            commit_by_sha.insert((c.repo, c.sha.clone()), ci as u32);
        }
        let mut ticket_commits = vec![vec![]; tickets.len()];
        for (ci, c) in commits.iter().enumerate() {
            for &t in &c.tickets {
                ticket_commits[t as usize].push(ci as u32);
            }
        }
        let now = commits.iter().map(|c| c.ts).max().unwrap_or_else(|| {
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
        });

        // --- units
        let mut units: Vec<UnitInfo> = vec![];
        let mut unit_idx: HashMap<(u32, String), u32> = HashMap::new();
        let indexes: Vec<UnitIndex> = repos
            .iter()
            .zip(&arches)
            .map(|(r, a)| UnitIndex::new(a, &cfg.architecture.units, &r.name, cfg.architecture.fallback_depth))
            .collect();
        for f in files.iter_mut() {
            let (name, kind) = indexes[f.repo as usize].resolve(&f.path);
            let key = (f.repo, name.clone());
            let next = units.len() as u32;
            f.unit = *unit_idx.entry(key).or_insert_with(|| {
                let arch_unit = arches[f.repo as usize].units.iter().find(|u| u.name == name);
                units.push(UnitInfo {
                    root: arch_unit.map(|u| u.root.clone()).unwrap_or_else(|| name.clone()),
                    tags: arch_unit.map(|u| u.tags.clone()).unwrap_or_default(),
                    name,
                    repo: f.repo,
                    kind,
                });
                next
            });
        }

        // --- entities: stem grouping plus Angular templateUrl/styleUrls links
        let mut file_by_path: HashMap<(u32, String), u32> = HashMap::new();
        for (i, f) in files.iter().enumerate() {
            if f.alive || !file_by_path.contains_key(&(f.repo, f.path.clone())) {
                file_by_path.insert((f.repo, f.path.clone()), i as u32);
            }
        }
        let mut key_of: Vec<String> = files.iter().map(|f| entity_key(&f.path)).collect();
        for (i, f) in files.iter().enumerate() {
            if !f.alive {
                continue;
            }
            let dir = f.path.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
            let linked = f.summary.template_url.iter().chain(f.summary.style_urls.iter());
            for url in linked {
                let target = normalize_join(dir, url);
                if let Some(&t) = file_by_path.get(&(f.repo, target)) {
                    key_of[t as usize] = key_of[i].clone();
                }
            }
        }
        let mut entities: Vec<Entity> = vec![];
        let mut entity_idx: HashMap<(u32, String), u32> = HashMap::new();
        for (i, f) in files.iter_mut().enumerate() {
            let key = key_of[i].clone();
            let next = entities.len() as u32;
            let e = *entity_idx.entry((f.repo, key.clone())).or_insert_with(|| {
                entities.push(Entity {
                    name: key.rsplit('/').next().unwrap_or(&key).to_string(),
                    key,
                    repo: f.repo,
                    unit: f.unit,
                    kind: "file".into(),
                    files: vec![],
                });
                next
            });
            f.entity = e;
            entities[e as usize].files.push(i as u32);
        }
        for e in entities.iter_mut() {
            let kind = e
                .files
                .iter()
                .filter_map(|&f| files[f as usize].summary.angular.as_ref().map(|a| a.kind.clone()))
                .next();
            e.kind = kind.unwrap_or_else(|| {
                let langs: Vec<Lang> = e.files.iter().map(|&f| files[f as usize].lang).collect();
                if langs.iter().all(|l| *l == Lang::Template) { "template".into() } else { "file".into() }
            });
            // an entity lives in the unit of its main (non-test) file
            if let Some(&main) = e.files.iter().find(|&&f| !files[f as usize].test) {
                e.unit = files[main as usize].unit;
            }
        }

        // --- language-normalized complexity
        let mut by_lang: BTreeMap<&'static str, Vec<usize>> = BTreeMap::new();
        for (i, f) in files.iter().enumerate() {
            if f.alive {
                by_lang.entry(f.lang.as_str()).or_default().push(i);
            }
        }
        for idxs in by_lang.values() {
            let vals: Vec<f64> = idxs.iter().map(|&i| files[i].complexity).collect();
            let pr = percentile_ranks(&vals);
            for (k, &i) in idxs.iter().enumerate() {
                // a lone file of its language isn't automatically "most complex"
                let p = if idxs.len() < 5 { (files[i].complexity / 50.0).min(1.0) } else { pr[k] };
                files[i].norm_cx = p * files[i].lang.weight();
            }
        }

        // --- trends
        {
            let mut st = db.conn.prepare(
                "SELECT tp.file_id, c.ts, b.complexity, b.loc, b.max_cc FROM trend_points tp
                 JOIN commits c ON c.id = tp.commit_id JOIN blob_metrics b ON b.blob = tp.blob ORDER BY c.ts",
            )?;
            let rows = st.query_map([], |r| {
                Ok((r.get::<_, i64>(0)?, TrendPoint { ts: r.get(1)?, complexity: r.get(2)?, loc: r.get(3)?, max_cc: r.get(4)? }))
            })?;
            for row in rows {
                let (fid, p) = row?;
                if let Some(&i) = file_by_db.get(&fid) {
                    files[i as usize].trend.push(p);
                }
            }
        }

        Ok(Dataset {
            cfg: cfg.clone(),
            now,
            repos,
            authors,
            commits,
            files,
            entities,
            units,
            tickets,
            ticket_commits,
            file_by_db,
            file_by_path,
            commit_by_sha,
        })
    }

    /// Key of a file at the given level.
    pub fn key(&self, file: u32, level: Level) -> u32 {
        let f = &self.files[file as usize];
        match level {
            Level::File => file,
            Level::Entity => f.entity,
            Level::Unit => f.unit,
            Level::Repo => f.repo,
        }
    }

    pub fn key_count(&self, level: Level) -> usize {
        match level {
            Level::File => self.files.len(),
            Level::Entity => self.entities.len(),
            Level::Unit => self.units.len(),
            Level::Repo => self.repos.len(),
        }
    }

    /// Human-readable name of a key: path for files, `dir/stem` for entities, unit/repo names.
    pub fn key_name(&self, key: u32, level: Level) -> String {
        match level {
            Level::File => self.files[key as usize].path.clone(),
            Level::Entity => self.entities[key as usize].key.clone(),
            Level::Unit => self.units[key as usize].name.clone(),
            Level::Repo => self.repos[key as usize].name.clone(),
        }
    }

    pub fn key_repo(&self, key: u32, level: Level) -> u32 {
        match level {
            Level::File => self.files[key as usize].repo,
            Level::Entity => self.entities[key as usize].repo,
            Level::Unit => self.units[key as usize].repo,
            Level::Repo => key,
        }
    }

    pub fn key_unit(&self, key: u32, level: Level) -> Option<u32> {
        match level {
            Level::File => Some(self.files[key as usize].unit),
            Level::Entity => Some(self.entities[key as usize].unit),
            Level::Unit => Some(key),
            Level::Repo => None,
        }
    }

    /// Files belonging to a key.
    pub fn key_files(&self, key: u32, level: Level) -> Vec<u32> {
        match level {
            Level::File => vec![key],
            Level::Entity => self.entities[key as usize].files.clone(),
            Level::Unit => (0..self.files.len() as u32).filter(|&f| self.files[f as usize].unit == key).collect(),
            Level::Repo => (0..self.files.len() as u32).filter(|&f| self.files[f as usize].repo == key).collect(),
        }
    }

    pub fn unit_label(&self, unit: u32) -> String {
        let u = &self.units[unit as usize];
        if self.repos.len() > 1 { format!("{}:{}", self.repos[u.repo as usize].name, u.name) } else { u.name.clone() }
    }

    pub fn is_active(&self, author: u32) -> bool {
        let a = &self.authors[author as usize];
        (self.now - a.last_ts) as f64 <= self.cfg.analysis.inactive_after_days * csi_core::util::DAY
    }

    /// Find a file by repo-relative path, an absolute path, or a unique suffix.
    pub fn find_file(&self, path: &str, repo: Option<&str>) -> Option<u32> {
        let candidates: Vec<u32> = (0..self.repos.len() as u32)
            .filter(|&r| repo.is_none_or(|n| self.repos[r as usize].name == n))
            .collect();
        let p = std::path::Path::new(path);
        if p.is_absolute() {
            for &r in &candidates {
                if let Ok(rel) = p.strip_prefix(&self.repos[r as usize].path) {
                    if let Some(&f) = self.file_by_path.get(&(r, rel.to_string_lossy().to_string())) {
                        return Some(f);
                    }
                }
            }
        }
        let rel = path.trim_start_matches("./");
        for &r in &candidates {
            if let Some(&f) = self.file_by_path.get(&(r, rel.to_string())) {
                return Some(f);
            }
        }
        let suffix = format!("/{rel}");
        let matches: Vec<u32> = (0..self.files.len() as u32)
            .filter(|&f| {
                let fl = &self.files[f as usize];
                fl.alive && candidates.contains(&fl.repo) && (fl.path.ends_with(&suffix) || fl.path == rel)
            })
            .collect();
        (matches.len() == 1).then(|| matches[0])
    }
}

/// Join `dir` and a relative URL like `./x.html` or `../y/z.scss`.
fn normalize_join(dir: &str, rel: &str) -> String {
    let mut parts: Vec<&str> = if dir.is_empty() { vec![] } else { dir.split('/').collect() };
    for seg in rel.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    parts.join("/")
}

#[cfg(test)]
mod tests {
    #[test]
    fn joins() {
        assert_eq!(super::normalize_join("src/app/a", "./a.component.html"), "src/app/a/a.component.html");
        assert_eq!(super::normalize_join("src/app/a", "../shared/x.scss"), "src/app/shared/x.scss");
    }
}
