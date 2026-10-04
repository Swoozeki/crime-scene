//! The findings engine: deterministic rules that turn metrics into a ranked, actionable list.

use crate::coupling::{By, CouplingQuery, coupling, sum_of_coupling};
use crate::dataset::Dataset;
use crate::hotspots::{Hotspot, hotspots};
use crate::scope::{Level, Scope};
use crate::social::ownership;
use crate::xray;
use csi_core::Db;
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize)]
pub struct Finding {
    pub id: String,
    pub kind: String,
    pub kind_label: String,
    pub severity: u32,
    pub title: String,
    /// entity/unit/file the finding is about
    pub subject: String,
    pub level: Level,
    pub key: u32,
    pub repo: String,
    pub unit: Option<String>,
    pub evidence: Value,
    pub recommendation: String,
    pub related: Vec<String>,
}

pub const KINDS: &[&str] = &[
    "hotspot",
    "deteriorating_hotspot",
    "xray_hotspot",
    "hidden_coupling",
    "shotgun_surgery",
    "knowledge_loss",
    "bus_factor",
    "coordination",
    "defect_magnet",
    "bloated_component",
];

const TITLE_PREFIXES: &[&str] = &[
    "Hotspot",
    "Deteriorating hotspot",
    "Function hotspot",
    "Hidden coupling",
    "Ripple effect",
    "Knowledge loss",
    "Bus factor 1",
    "Coordination bottleneck",
    "Defect magnet",
    "Bloated component",
];

pub fn kind_label(kind: &str) -> &'static str {
    match kind {
        "hotspot" => "Hotspot",
        "deteriorating_hotspot" => "Deteriorating hotspot",
        "xray_hotspot" => "Function hotspot",
        "hidden_coupling" => "Hidden coupling",
        "shotgun_surgery" => "Ripple effect",
        "knowledge_loss" => "Knowledge loss",
        "bus_factor" => "Bus factor",
        "coordination" => "Coordination bottleneck",
        "defect_magnet" => "Defect magnet",
        "bloated_component" => "Bloated component",
        _ => "Finding",
    }
}

fn sev(x: f64) -> u32 {
    x.clamp(1.0, 100.0).round() as u32
}

fn top_n(len: usize, pct: f64, min: usize) -> usize {
    ((len as f64 * pct).ceil() as usize).max(min).min(len)
}

pub fn findings(ds: &Dataset, db: Option<&Db>, scope: &Scope) -> Vec<Finding> {
    let mut out: Vec<Finding> = vec![];
    let ent = hotspots(ds, Level::Entity, scope);
    let ent_idx: HashMap<u32, &Hotspot> = ent.iter().map(|h| (h.key, h)).collect();
    let hot_n = top_n(ent.len(), 0.02, 5);
    let decile = top_n(ent.len(), 0.10, 10);

    // --- hotspots (capped so other kinds of finding still surface)
    for h in ent.iter().take(hot_n) {
        if out.len() >= 10 {
            break;
        }
        if h.test || !h.code || h.score < 0.35 || h.revisions < 3 || !(h.health <= 7.0 || h.norm_complexity >= 0.9) {
            continue;
        }
        let deteriorating = h.trend.as_ref().is_some_and(|t| t.direction == "deteriorating");
        let base = 40.0 + h.score * 50.0 + (10.0 - h.health) * 2.0;
        let reason = h.health_reasons.first().cloned();
        out.push(Finding {
            id: String::new(),
            kind_label: String::new(),
            kind: if deteriorating { "deteriorating_hotspot" } else { "hotspot" }.into(),
            severity: sev(base + if deteriorating { 10.0 } else { 0.0 }),
            title: format!(
                "{}{} — {} changes, complexity {:.0}{}, health {:.1}",
                if deteriorating { "Deteriorating hotspot: " } else { "Hotspot: " },
                h.display,
                h.revisions,
                h.complexity,
                match &h.trend {
                    Some(t) if deteriorating => format!(" (+{:.0}% in a year)", t.change * 100.0),
                    _ => String::new(),
                },
                h.health
            ),
            subject: h.name.clone(),
            level: Level::Entity,
            key: h.key,
            repo: h.repo.clone(),
            unit: h.unit.clone(),
            evidence: json!({
                "rank": h.rank, "score": h.score, "revisions": h.revisions, "rev_w": round2(h.rev_w),
                "complexity": h.complexity, "loc": h.loc, "health": h.health, "health_reasons": h.health_reasons,
                "main_dev": h.main_dev, "authors": h.authors, "defects": h.defects, "trend": h.trend,
            }),
            recommendation: match reason {
                Some(r) => format!(
                    "Prioritize refactoring here: {r}. Run `csi xray` on its main file to find the few functions that absorb most changes{}.",
                    if deteriorating { " — complexity is still growing, so act before it compounds" } else { "" }
                ),
                None => "High change rate on complex code. Run `csi xray` to find the functions that change most and split them along their change patterns.".into(),
            },
            related: vec![],
        });
    }

    // --- X-ray: complex functions that keep changing inside top files
    if let Some(db) = db {
        let files = hotspots(ds, Level::File, scope);
        let mut n = 0;
        for h in files.iter().filter(|h| ds.files[h.key as usize].lang.is_parsed()).take(15) {
            if h.test {
                continue;
            }
            let Ok(x) = xray::xray(ds, db, h.key) else { continue };
            if x.analyzed_commits == 0 {
                continue;
            }
            for func in x.functions.iter().filter(|f| f.cc > 15 && f.revisions >= 3 && f.score >= 0.4).take(2) {
                n += 1;
                out.push(Finding {
                    id: String::new(),
                    kind_label: String::new(),
                    kind: "xray_hotspot".into(),
                    severity: sev(35.0 + func.cc.min(40) as f64 * 0.6 + func.revisions.min(40) as f64 * 0.6 + h.score * 15.0),
                    title: format!(
                        "Function hotspot: `{}` in {} (cc {}, changed {}×)",
                        func.name, h.display, func.cc, func.revisions
                    ),
                    subject: h.name.clone(),
                    level: Level::File,
                    key: h.key,
                    repo: h.repo.clone(),
                    unit: h.unit.clone(),
                    evidence: json!({
                        "function": func.name, "cc": func.cc, "nesting": func.nesting, "loc": func.loc,
                        "revisions": func.revisions, "lines": [func.start, func.end], "file_rank": h.rank,
                    }),
                    recommendation: format!(
                        "Split `{}` (lines {}–{}): extract the branches that change together into named functions and cover them with tests first.",
                        func.name, func.start, func.end
                    ),
                    related: vec![],
                });
            }
            if n >= 8 {
                break;
            }
        }
    }

    // --- hidden coupling (across units within a repo, and across repos via tickets),
    // grouped around a hub entity so one duplicated file doesn't produce N findings
    let mut pairs: Vec<(crate::coupling::Coupling, By)> = vec![];
    for by in [By::Commit, By::Ticket] {
        let q = CouplingQuery { level: Level::Entity, by, cross_only: true, ..Default::default() };
        for c in coupling(ds, &q, scope) {
            let skip = |k: u32| ent_idx.get(&k).is_none_or(|h| h.test || !h.code);
            if c.degree() < 0.5 || c.test_pair || skip(c.a) || skip(c.b) || (by == By::Ticket && !c.cross_repo) {
                continue;
            }
            if !pairs.iter().any(|(p, _)| p.a == c.a && p.b == c.b) {
                pairs.push((c, by));
            }
        }
    }
    // connected components: copies that all change together form one cluster
    let mut degree: HashMap<u32, usize> = HashMap::new();
    let mut parent: HashMap<u32, u32> = HashMap::new();
    fn find(p: &mut HashMap<u32, u32>, x: u32) -> u32 {
        let mut r = x;
        while let Some(&n) = p.get(&r) {
            if n == r {
                break;
            }
            r = n;
        }
        p.insert(x, r);
        r
    }
    for (c, _) in &pairs {
        *degree.entry(c.a).or_default() += 1;
        *degree.entry(c.b).or_default() += 1;
        parent.entry(c.a).or_insert(c.a);
        parent.entry(c.b).or_insert(c.b);
        let (ra, rb) = (find(&mut parent, c.a), find(&mut parent, c.b));
        if ra != rb {
            parent.insert(rb, ra);
        }
    }
    let mut comps: HashMap<u32, Vec<usize>> = HashMap::new();
    for (i, (c, _)) in pairs.iter().enumerate() {
        let r = find(&mut parent, c.a);
        comps.entry(r).or_default().push(i);
    }
    let mut groups: Vec<(u32, Vec<(u32, crate::coupling::Coupling, By)>)> = comps
        .into_values()
        .map(|idxs| {
            let hub = idxs
                .iter()
                .flat_map(|&i| [pairs[i].0.a, pairs[i].0.b])
                .max_by_key(|k| (degree[k], std::cmp::Reverse(*k)))
                .unwrap();
            let mut members: Vec<(u32, crate::coupling::Coupling, By)> = vec![];
            let mut seen = std::collections::HashSet::new();
            let mut sorted = idxs.clone();
            sorted.sort_by(|&x, &y| pairs[y].0.degree().total_cmp(&pairs[x].0.degree()));
            for i in sorted {
                let (c, by) = &pairs[i];
                for k in [c.a, c.b] {
                    if k != hub && seen.insert(k) {
                        members.push((k, c.clone(), *by));
                    }
                }
            }
            (hub, members)
        })
        .collect();
    groups.sort_by(|a, b| {
        let best = |g: &Vec<(u32, crate::coupling::Coupling, By)>| g.iter().map(|m| m.1.degree() * m.1.support as f64).fold(0.0, f64::max);
        best(&b.1).total_cmp(&best(&a.1))
    });
    let label = |k: u32| -> String {
        let e = &ds.entities[k as usize];
        let clash = ds.entities.iter().filter(|x| x.name == e.name).count() > 1;
        if clash { format!("{} ({})", e.name, ds.unit_label(e.unit)) } else { e.name.clone() }
    };
    for (hub, members) in groups.into_iter().take(10) {
        let (_, best, by) = members
            .iter()
            .max_by(|x, y| x.1.degree().total_cmp(&y.1.degree()).then(x.1.support.cmp(&y.1.support)))
            .unwrap();
        let cross_repo = members.iter().any(|m| m.1.cross_repo);
        let hub_e = &ds.entities[hub as usize];
        let partners: Vec<String> = members.iter().map(|m| label(m.0)).collect();
        let title = if members.len() == 1 {
            format!(
                "Hidden coupling: {} ↔ {} change together {}× ({:.0}%){}",
                label(hub),
                partners[0],
                best.support,
                best.degree() * 100.0,
                if cross_repo { " across repos" } else { "" }
            )
        } else {
            format!(
                "Hidden coupling: {} changes together with {} entities in other {}: {}",
                label(hub),
                members.len(),
                if cross_repo { "repos/units" } else { "units" },
                crate::findings::list(&partners, 3)
            )
        };
        out.push(Finding {
            id: String::new(),
            kind_label: String::new(),
            kind: "hidden_coupling".into(),
            severity: sev(35.0
                + 45.0 * best.degree() * (best.support as f64 / 15.0).min(1.0)
                + if cross_repo { 8.0 } else { 0.0 }
                + (members.len() as f64 - 1.0).min(5.0)),
            title,
            subject: hub_e.key.clone(),
            level: Level::Entity,
            key: hub,
            repo: ds.repos[hub_e.repo as usize].name.clone(),
            unit: Some(ds.unit_label(hub_e.unit)),
            evidence: json!({
                "partners": members.iter().map(|(o, c, by)| json!({
                    "entity": ds.entities[*o as usize].key, "repo": ds.repos[ds.entities[*o as usize].repo as usize].name,
                    "support": c.support, "confidence": c.degree(), "lift": c.lift, "recent": c.recent, "by": by,
                })).collect::<Vec<_>>(),
                "by": by,
            }),
            recommendation: if cross_repo {
                "These live in different repos yet ship together. Make the contract explicit (shared API schema/types, consumer-driven contract tests) or move the shared concept to one side.".into()
            } else if members.len() > 1 && members.iter().all(|m| ds.entities[m.0 as usize].name == hub_e.name) {
                "The same file is maintained in several places and always changes in lockstep — likely copies. Generate them from one source or extract a shared package.".into()
            } else {
                "These live in different units but change together. Look for a shared concept (duplicated logic, leaky abstraction, misplaced responsibility) and co-locate or extract it.".into()
            },
            related: members.iter().map(|m| ds.entities[m.0 as usize].key.clone()).collect(),
        });
    }

    // --- shotgun surgery: entities whose changes ripple widely
    let soc = sum_of_coupling(ds, Level::Entity);
    let mut by_soc: Vec<&Hotspot> = ent.iter().filter(|h| h.revisions >= 10 && !h.test && h.code).collect();
    by_soc.sort_by(|a, b| soc[b.key as usize].cmp(&soc[a.key as usize]));
    let mut ripple = 0;
    for h in by_soc.iter().take(top_n(by_soc.len(), 0.02, 3)) {
        let s = soc[h.key as usize];
        let avg = s as f64 / h.revisions as f64;
        if avg < 5.0 || ripple >= 3 {
            continue;
        }
        ripple += 1;
        out.push(Finding {
            id: String::new(),
            kind_label: String::new(),
            kind: "shotgun_surgery".into(),
            severity: sev(25.0 + avg.min(15.0) * 2.0 + h.score * 15.0),
            title: format!("Ripple effect: changing {} touches {:.1} other entities on average", h.display, avg),
            subject: h.name.clone(),
            level: Level::Entity,
            key: h.key,
            repo: h.repo.clone(),
            unit: h.unit.clone(),
            evidence: json!({"sum_of_coupling": s, "revisions": h.revisions, "avg_coupled": round2(avg)}),
            recommendation: "Changes here rarely stay local. Check `csi coupling` for its partners and pull the shared responsibility into one place.".into(),
            related: vec![],
        });
    }

    // --- knowledge loss and bus factor
    let own_e = ownership(ds, Level::Entity, scope);
    let own_e: HashMap<u32, _> = own_e.into_iter().map(|o| (o.key, o)).collect();
    for h in ent.iter().filter(|h| !h.test).take(decile) {
        let Some(o) = own_e.get(&h.key) else { continue };
        if o.knowledge_loss >= 0.5 && h.score >= 0.3 {
            out.push(Finding {
                id: String::new(),
            kind_label: String::new(),
                kind: "knowledge_loss".into(),
                severity: sev(35.0 + 35.0 * o.knowledge_loss + 20.0 * h.score),
                title: format!(
                    "Knowledge loss: {:.0}% of {} was written by people no longer active",
                    o.knowledge_loss * 100.0,
                    h.display
                ),
                subject: h.name.clone(),
                level: Level::Entity,
                key: h.key,
                repo: h.repo.clone(),
                unit: h.unit.clone(),
                evidence: json!({"knowledge_loss": o.knowledge_loss, "authors": o.authors, "hotspot_rank": h.rank}),
                recommendation: "A hotspot nobody active knows well. Pair on the next change, add characterization tests, and document intent before refactoring.".into(),
                related: vec![],
            });
        }
    }
    let units = hotspots(ds, Level::Unit, scope);
    let own_u: HashMap<u32, _> = ownership(ds, Level::Unit, scope).into_iter().map(|o| (o.key, o)).collect();
    // bus factor 1, grouped per person so one maintainer doesn't fill the list
    let mut by_person: Vec<(String, Vec<(&Hotspot, f64)>)> = vec![];
    for h in units.iter().filter(|h| h.loc >= 1000 && h.revisions >= 10 && !h.test) {
        let Some(o) = own_u.get(&h.key) else { continue };
        if o.bus_factor != 1 || o.recent_authors == 0 || o.main_dev_share < 0.6 {
            continue;
        }
        let dev = o.main_dev.clone().unwrap_or_default();
        match by_person.iter_mut().find(|(d, _)| d == &dev) {
            Some((_, v)) => v.push((h, o.main_dev_share)),
            None => by_person.push((dev, vec![(h, o.main_dev_share)])),
        }
    }
    by_person.sort_by_key(|(_, v)| std::cmp::Reverse(v.iter().map(|(h, _)| h.loc).sum::<u32>()));
    for (dev, units_of) in by_person.into_iter().take(5) {
        let loc: u32 = units_of.iter().map(|(h, _)| h.loc).sum();
        let share = units_of.iter().map(|(_, s)| s).sum::<f64>() / units_of.len() as f64;
        let names: Vec<String> = units_of.iter().map(|(h, _)| h.display.clone()).collect();
        let (first, _) = units_of[0];
        out.push(Finding {
            id: String::new(),
            kind_label: String::new(),
            kind: "bus_factor".into(),
            severity: sev(30.0 + 25.0 * share + (loc as f64).log10() * 4.0 + (units_of.len() as f64).min(5.0)),
            title: if units_of.len() == 1 {
                format!("Bus factor 1: {dev} knows {:.0}% of {} ({loc} lines)", share * 100.0, names[0])
            } else {
                format!("Bus factor 1: {dev} holds most knowledge of {} units ({loc} lines): {}", units_of.len(), list(&names, 3))
            },
            subject: first.name.clone(),
            level: Level::Unit,
            key: first.key,
            repo: first.repo.clone(),
            unit: first.unit.clone(),
            evidence: json!({"main_dev": dev, "units": units_of.iter().map(|(h, s)| json!({"unit": h.name, "share": s, "loc": h.loc})).collect::<Vec<_>>()}),
            recommendation: "Spread knowledge: rotate reviewers, pair on changes in these units, and write down the non-obvious decisions.".into(),
            related: names,
        });
    }

    // --- coordination bottlenecks
    let mut coord: Vec<(&Hotspot, u32, usize)> = ent
        .iter()
        .filter(|h| !h.test)
        .filter_map(|h| {
            let o = own_e.get(&h.key)?;
            let teams = o.teams.iter().filter(|(_, s)| *s >= 0.1).count();
            (o.recent_authors >= 8 || teams >= 3).then_some((h, o.recent_authors, teams))
        })
        .collect();
    coord.sort_by(|a, b| (b.1 as usize + b.2 * 3).cmp(&(a.1 as usize + a.2 * 3)));
    for (h, authors, teams) in coord.into_iter().take(5) {
        out.push(Finding {
            id: String::new(),
            kind_label: String::new(),
            kind: "coordination".into(),
            severity: sev(25.0 + authors.min(20) as f64 * 1.5 + teams as f64 * 5.0 + h.score * 15.0),
            title: if teams >= 3 {
                format!("Coordination bottleneck: {} teams change {}", teams, h.display)
            } else {
                format!("Coordination bottleneck: {} people changed {} in the last year", authors, h.display)
            },
            subject: h.name.clone(),
            level: Level::Entity,
            key: h.key,
            repo: h.repo.clone(),
            unit: h.unit.clone(),
            evidence: json!({"recent_authors": authors, "teams": teams}),
            recommendation: "Many hands on one piece of code. Split it along the responsibilities different people change, or give it a clear owner.".into(),
            related: vec![],
        });
    }

    // --- defect magnets
    let mut magnets: Vec<&Hotspot> =
        ent.iter().filter(|h| !h.test && h.defects >= 5 && h.defect_density >= 0.4).collect();
    magnets.sort_by(|a, b| b.defects.cmp(&a.defects));
    for h in magnets.into_iter().take(5) {
        out.push(Finding {
            id: String::new(),
            kind_label: String::new(),
            kind: "defect_magnet".into(),
            severity: sev(30.0 + 40.0 * h.defect_density + h.defects.min(30) as f64 + h.score * 10.0),
            title: format!(
                "Defect magnet: {:.0}% of changes to {} are fixes ({} of {})",
                h.defect_density * 100.0,
                h.display,
                h.defects,
                h.revisions
            ),
            subject: h.name.clone(),
            level: Level::Entity,
            key: h.key,
            repo: h.repo.clone(),
            unit: h.unit.clone(),
            evidence: json!({"defects": h.defects, "revisions": h.revisions, "density": round2(h.defect_density)}),
            recommendation: "Bugs keep landing here. Strengthen tests around it before new features, and review recent fixes for a common root cause.".into(),
            related: vec![],
        });
    }

    // --- bloated Angular components
    for h in ent.iter().take(decile).filter(|h| h.kind == "component") {
        let e = &ds.entities[h.key as usize];
        let mut issues = vec![];
        for &f in &e.files {
            let file = &ds.files[f as usize];
            if !file.alive {
                continue;
            }
            if let Some(a) = &file.summary.angular {
                if a.deps > csi_lang::health::MANY_DEPS {
                    issues.push(format!("{} injected dependencies", a.deps));
                }
                if a.inputs > csi_lang::health::MANY_INPUTS {
                    issues.push(format!("{} inputs", a.inputs));
                }
            }
            if file.lang == csi_lang::Lang::Template && file.complexity > csi_lang::health::COMPLEX_TEMPLATE {
                issues.push(format!("template complexity {:.0}", file.complexity));
            }
        }
        if issues.is_empty() || ent_idx.get(&h.key).is_none() {
            continue;
        }
        out.push(Finding {
            id: String::new(),
            kind_label: String::new(),
            kind: "bloated_component".into(),
            severity: sev(30.0 + issues.len() as f64 * 10.0 + h.score * 25.0),
            title: format!("Bloated component: {} ({})", h.display, issues.join(", ")),
            subject: h.name.clone(),
            level: Level::Entity,
            key: h.key,
            repo: h.repo.clone(),
            unit: h.unit.clone(),
            evidence: json!({"issues": issues, "hotspot_rank": h.rank}),
            recommendation: "Split into a container + presentational components, move logic into a store/facade service, and break the template into child components.".into(),
            related: vec![],
        });
    }

    out.sort_by(|a, b| b.severity.cmp(&a.severity).then(a.title.cmp(&b.title)));
    for (i, f) in out.iter_mut().enumerate() {
        f.id = format!("F{:03}", i + 1);
        f.kind_label = kind_label(&f.kind).to_string();
        // the kind is shown separately; titles start with the subject
        if let Some((prefix, rest)) = f.title.split_once(": ") {
            if TITLE_PREFIXES.contains(&prefix) {
                f.title = rest.to_string();
            }
        }
    }
    out
}

fn round2(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}

/// "a, b, c and 2 more"
pub(crate) fn list(items: &[String], max: usize) -> String {
    if items.len() <= max {
        items.join(", ")
    } else {
        format!("{} and {} more", items[..max].join(", "), items.len() - max)
    }
}
