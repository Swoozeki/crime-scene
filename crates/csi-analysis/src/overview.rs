//! Workspace overview: headline numbers plus the hotspot tree used by the circle-packing map.

use crate::coupling::{By, CouplingQuery, coupling};
use crate::dataset::Dataset;
use crate::hotspots::hotspots;
use crate::scope::{Level, Scope};
use crate::social::ownership;
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};

#[derive(Debug, Clone, Serialize)]
pub struct RepoSummary {
    pub name: String,
    pub branch: String,
    pub head: Option<String>,
    pub frameworks: Vec<String>,
    pub commits: usize,
    pub files: usize,
    pub loc: u64,
    pub units: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct Overview {
    pub workspace: String,
    pub repos: Vec<RepoSummary>,
    pub commits: usize,
    pub authors: usize,
    pub active_authors: usize,
    pub files: usize,
    pub entities: usize,
    pub units: usize,
    pub loc: u64,
    pub first_commit: Option<i64>,
    pub last_commit: Option<i64>,
    pub tickets: usize,
    /// share of all knowledge held by inactive authors
    pub knowledge_loss: f64,
    pub bus_factor_one_units: usize,
    /// strong (≥ 50% confidence) couplings across unit boundaries
    pub cross_unit_couplings: usize,
    pub cross_repo_couplings: usize,
    pub hotspot_concentration: f64,
    pub config: OverviewConfig,
}

#[derive(Debug, Clone, Serialize)]
pub struct OverviewConfig {
    pub since: String,
    pub half_life_days: f64,
    pub inactive_after_days: f64,
}

pub fn overview(ds: &Dataset) -> Overview {
    let alive: Vec<usize> = (0..ds.files.len()).filter(|&i| ds.files[i].alive).collect();
    let repos = ds
        .repos
        .iter()
        .enumerate()
        .map(|(ri, r)| RepoSummary {
            name: r.name.clone(),
            branch: r.branch.clone(),
            head: r.head.clone(),
            frameworks: r.frameworks.clone(),
            commits: ds.commits.iter().filter(|c| c.repo == ri as u32).count(),
            files: alive.iter().filter(|&&f| ds.files[f].repo == ri as u32).count(),
            loc: alive.iter().filter(|&&f| ds.files[f].repo == ri as u32).map(|&f| ds.files[f].loc as u64).sum(),
            units: ds.units.iter().filter(|u| u.repo == ri as u32).count(),
        })
        .collect();
    let used_authors: Vec<u32> =
        (0..ds.authors.len() as u32).filter(|&a| ds.authors[a as usize].commits > 0 && !ds.authors[a as usize].is_bot).collect();
    let all = Scope::default();
    let own_repo = ownership(ds, Level::Repo, &all);
    let total_loss = if own_repo.is_empty() {
        0.0
    } else {
        own_repo.iter().map(|o| o.knowledge_loss).sum::<f64>() / own_repo.len() as f64
    };
    let own_units = ownership(ds, Level::Unit, &all);
    let unit_hot = hotspots(ds, Level::Unit, &all);
    let unit_loc: HashMap<u32, u32> = unit_hot.iter().map(|h| (h.key, h.loc)).collect();
    let bus1 = own_units
        .iter()
        .filter(|o| o.bus_factor == 1 && unit_loc.get(&o.key).copied().unwrap_or(0) >= 1000 && o.recent_authors > 0)
        .count();
    let q = CouplingQuery { level: Level::Entity, cross_only: true, ..Default::default() };
    let strong = |c: &crate::coupling::Coupling| c.degree() >= 0.5 && !c.test_pair;
    let cross_unit = coupling(ds, &q, &all).iter().filter(|c| strong(c)).count();
    let cross_repo = if ds.repos.len() > 1 {
        coupling(ds, &CouplingQuery { by: By::Ticket, ..q }, &all).iter().filter(|c| c.cross_repo && strong(c)).count()
    } else {
        0
    };
    // share of recency-weighted change landing in the top 5% of entities
    let ent = hotspots(ds, Level::Entity, &all);
    let mut w: Vec<f64> = ent.iter().map(|h| h.rev_w).collect();
    w.sort_by(|a, b| b.total_cmp(a));
    let total_w: f64 = w.iter().sum();
    let top = ((w.len() as f64) * 0.05).ceil() as usize;
    let concentration = if total_w > 0.0 { w.iter().take(top).sum::<f64>() / total_w } else { 0.0 };

    Overview {
        workspace: ds.cfg.workspace.name.clone(),
        repos,
        commits: ds.commits.len(),
        authors: used_authors.len(),
        active_authors: used_authors.iter().filter(|&&a| ds.is_active(a)).count(),
        files: alive.len(),
        entities: ds.entities.iter().filter(|e| e.files.iter().any(|&f| ds.files[f as usize].alive)).count(),
        units: ds.units.len(),
        loc: alive.iter().map(|&f| ds.files[f].loc as u64).sum(),
        first_commit: ds.commits.first().map(|c| c.ts),
        last_commit: ds.commits.last().map(|c| c.ts),
        tickets: ds.tickets.len(),
        knowledge_loss: (total_loss * 1000.0).round() / 1000.0,
        bus_factor_one_units: bus1,
        cross_unit_couplings: cross_unit,
        cross_repo_couplings: cross_repo,
        hotspot_concentration: (concentration * 1000.0).round() / 1000.0,
        config: OverviewConfig {
            since: ds.cfg.analysis.since.clone(),
            half_life_days: ds.cfg.analysis.half_life_days,
            inactive_after_days: ds.cfg.analysis.inactive_after_days,
        },
    }
}

/// Hierarchical node for the circle-packing hotspot map: workspace → repo → unit → dirs → entity.
#[derive(Debug, Clone, Serialize, Default)]
pub struct TreeNode {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub loc: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revisions: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub health: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub age_days: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub main_dev: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub knowledge_loss: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub defects: Option<u32>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<TreeNode>,
}

pub fn hotspot_tree(ds: &Dataset, scope: &Scope) -> TreeNode {
    let ent = hotspots(ds, Level::Entity, scope);
    let own: HashMap<u32, f64> =
        ownership(ds, Level::Entity, scope).into_iter().map(|o| (o.key, o.knowledge_loss)).collect();
    // repo -> unit -> nested dirs (relative to unit root) -> leaves
    #[derive(Default)]
    struct Dir {
        dirs: BTreeMap<String, Dir>,
        leaves: Vec<TreeNode>,
    }
    fn into_nodes(d: Dir) -> Vec<TreeNode> {
        let mut v: Vec<TreeNode> = d
            .dirs
            .into_iter()
            .map(|(name, sub)| {
                let mut children = into_nodes(sub);
                // collapse single-child directory chains
                if children.len() == 1 && !children[0].children.is_empty() && children[0].key.is_none() {
                    let only = children.pop().unwrap();
                    return TreeNode { name: format!("{name}/{}", only.name), children: only.children, ..Default::default() };
                }
                TreeNode { name, children, ..Default::default() }
            })
            .collect();
        v.extend(d.leaves);
        v
    }
    let mut repos: BTreeMap<String, BTreeMap<String, Dir>> = BTreeMap::new();
    for h in ent.iter().filter(|h| h.loc > 0) {
        let e = &ds.entities[h.key as usize];
        let unit = &ds.units[e.unit as usize];
        let rel = e.key.strip_prefix(&unit.root).map(|s| s.trim_start_matches('/')).unwrap_or(&e.key);
        let mut parts: Vec<&str> = rel.split('/').collect();
        parts.pop();
        let mut d = repos
            .entry(ds.repos[e.repo as usize].name.clone())
            .or_default()
            .entry(unit.name.clone())
            .or_default();
        for p in parts {
            d = d.dirs.entry(p.to_string()).or_default();
        }
        d.leaves.push(TreeNode {
            name: h.display.clone(),
            key: Some(h.key),
            kind: Some(h.kind.clone()),
            loc: Some(h.loc.max(1)),
            score: Some(h.score),
            revisions: Some(h.revisions),
            health: Some(h.health),
            age_days: h.age_days.map(|a| a.round()),
            main_dev: h.main_dev.clone(),
            knowledge_loss: own.get(&h.key).copied(),
            defects: Some(h.defects),
            children: vec![],
        });
    }
    TreeNode {
        name: ds.cfg.workspace.name.clone(),
        children: repos
            .into_iter()
            .map(|(repo, units)| TreeNode {
                name: repo,
                kind: Some("repo".into()),
                children: units
                    .into_iter()
                    .map(|(u, d)| TreeNode { name: u, kind: Some("unit".into()), children: into_nodes(d), ..Default::default() })
                    .collect(),
                ..Default::default()
            })
            .collect(),
        ..Default::default()
    }
}
