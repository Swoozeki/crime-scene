//! Everything known about one file or entity, for drill-down views and the MCP `file_context` tool.

use crate::coupling::{By, CouplingQuery, coupling};
use crate::dataset::{Dataset, TrendPoint};
use crate::hotspots::{Hotspot, hotspots};
use crate::scope::{Level, Scope};
use crate::social::{Expert, Ownership, experts, ownership};
use crate::xray::{XRay, xray};
use anyhow::{Result, bail};
use csi_core::Db;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct FileRow {
    pub key: u32,
    pub path: String,
    pub lang: String,
    pub loc: u32,
    pub complexity: f64,
    pub max_cc: u32,
    pub health: f64,
    pub health_reasons: Vec<String>,
    pub test: bool,
    pub revisions: usize,
    pub angular: Option<csi_lang::AngularMeta>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Partner {
    pub name: String,
    pub repo: String,
    pub unit: Option<String>,
    pub support: u32,
    /// P(partner changes | this changes)
    pub confidence: f64,
    pub lift: f64,
    pub by: By,
    pub cross_repo: bool,
    pub cross_unit: bool,
    pub expected: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct CommitRow {
    pub sha: String,
    pub repo: String,
    pub ts: i64,
    pub author: String,
    pub subject: String,
    pub tickets: Vec<String>,
    pub defect: bool,
    pub added: u32,
    pub deleted: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct Detail {
    pub level: Level,
    pub key: u32,
    pub name: String,
    pub repo: String,
    pub hotspot: Option<Hotspot>,
    pub total_ranked: usize,
    pub files: Vec<FileRow>,
    pub ownership: Option<Ownership>,
    pub experts: Vec<Expert>,
    pub coupling: Vec<Partner>,
    pub trend: Vec<TrendPoint>,
    pub xray: Option<XRay>,
    pub commits: Vec<CommitRow>,
    pub tickets: Vec<String>,
}

pub fn detail(ds: &Dataset, db: &Db, level: Level, key: u32) -> Result<Detail> {
    if !matches!(level, Level::File | Level::Entity | Level::Unit) || key as usize >= ds.key_count(level) {
        bail!("no such {} #{key}", level.as_str());
    }
    let name = ds.key_name(key, level);
    let repo_idx = ds.key_repo(key, level);
    let repo = ds.repos[repo_idx as usize].name.clone();
    let scope = Scope { repo: Some(repo.clone()), ..Default::default() };
    let rows = hotspots(ds, level, &scope);
    let total_ranked = rows.len();
    let hotspot = rows.into_iter().find(|h| h.key == key);
    let files_of = ds.key_files(key, level);
    let mut files: Vec<FileRow> = files_of
        .iter()
        .map(|&f| &ds.files[f as usize])
        .zip(files_of.iter())
        .filter(|(f, _)| f.alive)
        .map(|(f, &k)| FileRow {
            key: k,
            path: f.path.clone(),
            lang: f.lang.as_str().into(),
            loc: f.loc,
            complexity: f.complexity,
            max_cc: f.max_cc,
            health: f.health(),
            health_reasons: f.summary.health_reasons.clone(),
            test: f.test,
            revisions: f.commits.len(),
            angular: f.summary.angular.clone(),
        })
        .collect();
    files.sort_by(|a, b| a.test.cmp(&b.test).then(b.complexity.total_cmp(&a.complexity)));

    let ownership = ownership(ds, level, &scope).into_iter().find(|o| o.key == key);
    let experts = experts(ds, &files_of, &[]).into_iter().take(5).collect();

    let mut partners = vec![];
    for by in [By::Commit, By::Ticket] {
        let q = CouplingQuery { level, by, include_expected: level == Level::File, min_lift: Some(1.0), ..Default::default() };
        for c in coupling(ds, &q, &Scope::default()) {
            let (other, conf) = if c.a == key {
                (c.b, c.conf_ab)
            } else if c.b == key {
                (c.a, c.conf_ba)
            } else {
                continue;
            };
            if by == By::Ticket && !c.cross_repo {
                continue;
            }
            partners.push(Partner {
                name: ds.key_name(other, level),
                repo: ds.repos[ds.key_repo(other, level) as usize].name.clone(),
                unit: ds.key_unit(other, level).map(|u| ds.unit_label(u)),
                support: c.support,
                confidence: conf,
                lift: c.lift,
                by,
                cross_repo: c.cross_repo,
                cross_unit: c.cross_unit,
                expected: c.same_entity,
            });
        }
    }
    partners.sort_by(|a, b| b.confidence.total_cmp(&a.confidence).then(b.support.cmp(&a.support)));
    partners.truncate(25);

    // main file: the most complex non-test file
    let main = files.iter().find(|f| !f.test).or(files.first()).map(|f| f.key);
    let trend = main.map(|m| ds.files[m as usize].trend.clone()).unwrap_or_default();
    let xray = match main {
        Some(m) if ds.files[m as usize].lang.is_parsed() && level != Level::Unit => xray(ds, db, m).ok(),
        _ => None,
    };

    // timeline
    let mut cis: Vec<u32> = files_of.iter().flat_map(|&f| ds.files[f as usize].commits.iter().copied()).collect();
    cis.sort_unstable();
    cis.dedup();
    let mut tickets: Vec<String> = vec![];
    let commits: Vec<CommitRow> = cis
        .iter()
        .rev()
        .take(30)
        .map(|&ci| {
            let c = &ds.commits[ci as usize];
            let (added, deleted) = c
                .changes
                .iter()
                .filter(|ch| files_of.contains(&ch.file))
                .fold((0, 0), |acc, ch| (acc.0 + ch.added, acc.1 + ch.deleted));
            let tks: Vec<String> = c.tickets.iter().map(|&t| ds.tickets[t as usize].clone()).collect();
            for t in &tks {
                if !tickets.contains(t) {
                    tickets.push(t.clone());
                }
            }
            CommitRow {
                sha: c.sha.clone(),
                repo: ds.repos[c.repo as usize].name.clone(),
                ts: c.ts,
                author: ds.authors[c.author as usize].name.clone(),
                subject: c.subject.clone(),
                tickets: tks,
                defect: c.defect,
                added,
                deleted,
            }
        })
        .collect();
    tickets.truncate(15);

    Ok(Detail {
        level,
        key,
        name,
        repo,
        hotspot,
        total_ranked,
        files,
        ownership,
        experts,
        coupling: partners,
        trend,
        xray,
        commits,
        tickets,
    })
}
