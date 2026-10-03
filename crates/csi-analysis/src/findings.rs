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

    // --- hotspots
    for h in ent.iter().take(hot_n) {
        if h.score < 0.4 || h.revisions < 3 || !(h.health <= 7.0 || h.norm_complexity >= 0.9) {
            continue;
        }
        let deteriorating = h.trend.as_ref().is_some_and(|t| t.direction == "deteriorating");
        let base = h.score * 70.0 + (10.0 - h.health) * 3.0;
        let reason = h.health_reasons.first().cloned();
        out.push(Finding {
            id: String::new(),
            kind: if deteriorating { "deteriorating_hotspot" } else { "hotspot" }.into(),
            severity: sev(base + if deteriorating { 10.0 } else { 0.0 }),
            title: format!(
                "{}{} changed {}× ({} recently weighted), complexity {:.0}, health {:.1}",
                if deteriorating { "Deteriorating hotspot: " } else { "Hotspot: " },
                h.display,
                h.revisions,
                format_args!("{:.1}", h.rev_w),
                h.complexity,
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
            let Ok(x) = xray::xray(ds, db, h.key) else { continue };
            if x.analyzed_commits == 0 {
                continue;
            }
            for func in x.functions.iter().filter(|f| f.cc > 15 && f.revisions >= 3 && f.score >= 0.5).take(2) {
                n += 1;
                out.push(Finding {
                    id: String::new(),
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

    // --- hidden coupling (across units within a repo, and across repos via tickets)
    let mut coupled = 0;
    for by in [By::Commit, By::Ticket] {
        let q = CouplingQuery { level: Level::Entity, by, cross_only: true, ..Default::default() };
        for c in coupling(ds, &q, scope).into_iter().filter(|c| c.degree() >= 0.5 && !c.test_pair) {
            if by == By::Ticket && !c.cross_repo {
                continue; // already covered by commit coupling
            }
            coupled += 1;
            if coupled > 10 {
                break;
            }
            let (ea, eb) = (&ds.entities[c.a as usize], &ds.entities[c.b as usize]);
            let where_ = if c.cross_repo {
                format!("repos {} and {}", c.a_repo, c.b_repo)
            } else {
                format!("units {} and {}", c.a_unit.clone().unwrap_or_default(), c.b_unit.clone().unwrap_or_default())
            };
            out.push(Finding {
                id: String::new(),
                kind: "hidden_coupling".into(),
                severity: sev(35.0 + 45.0 * c.degree() * (c.support as f64 / 15.0).min(1.0) + if c.cross_repo { 8.0 } else { 0.0 }),
                title: format!(
                    "Hidden coupling: {} ↔ {} change together {}× ({:.0}%){}",
                    ea.name,
                    eb.name,
                    c.support,
                    c.degree() * 100.0,
                    if c.cross_repo { " across repos" } else { "" }
                ),
                subject: c.a_name.clone(),
                level: Level::Entity,
                key: c.a,
                repo: c.a_repo.clone(),
                unit: c.a_unit.clone(),
                evidence: json!({
                    "a": c.a_name, "b": c.b_name, "a_repo": c.a_repo, "b_repo": c.b_repo,
                    "support": c.support, "conf_ab": c.conf_ab, "conf_ba": c.conf_ba, "lift": c.lift,
                    "recent": c.recent, "by": by,
                }),
                recommendation: if c.cross_repo {
                    format!("These live in {where_} yet ship together. Make the contract between them explicit (shared API schema/types, consumer-driven contract tests) or move the shared concept to one side.")
                } else {
                    format!("These live in {where_} but change together. Look for a shared concept (duplicated logic, leaky abstraction, misplaced responsibility) and co-locate or extract it.")
                },
                related: vec![c.b_name.clone()],
            });
        }
    }

    // --- shotgun surgery: entities whose changes ripple widely
    let soc = sum_of_coupling(ds, Level::Entity);
    let mut by_soc: Vec<&Hotspot> = ent.iter().filter(|h| h.revisions >= 10).collect();
    by_soc.sort_by(|a, b| soc[b.key as usize].cmp(&soc[a.key as usize]));
    for h in by_soc.iter().take(top_n(by_soc.len(), 0.02, 3)) {
        let s = soc[h.key as usize];
        let avg = s as f64 / h.revisions as f64;
        if avg < 4.0 {
            continue;
        }
        out.push(Finding {
            id: String::new(),
            kind: "shotgun_surgery".into(),
            severity: sev(30.0 + avg.min(15.0) * 2.5 + h.score * 15.0),
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
    for h in ent.iter().take(decile) {
        let Some(o) = own_e.get(&h.key) else { continue };
        if o.knowledge_loss >= 0.5 && h.score >= 0.3 {
            out.push(Finding {
                id: String::new(),
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
    let mut bus = 0;
    for h in units.iter().filter(|h| h.loc >= 1000 && h.revisions >= 10) {
        let Some(o) = own_u.get(&h.key) else { continue };
        if o.bus_factor == 1 && o.recent_authors >= 1 && o.main_dev_share >= 0.6 && bus < 5 {
            bus += 1;
            out.push(Finding {
                id: String::new(),
                kind: "bus_factor".into(),
                severity: sev(25.0 + 30.0 * o.main_dev_share + 15.0 * h.score),
                title: format!(
                    "Bus factor 1: {} knows {:.0}% of {} ({} lines)",
                    o.main_dev.clone().unwrap_or_default(),
                    o.main_dev_share * 100.0,
                    h.display,
                    h.loc
                ),
                subject: h.name.clone(),
                level: Level::Unit,
                key: h.key,
                repo: h.repo.clone(),
                unit: h.unit.clone(),
                evidence: json!({"main_dev_share": o.main_dev_share, "authors": o.authors, "loc": h.loc}),
                recommendation: "Spread knowledge: rotate reviewers and pair on changes in this unit.".into(),
                related: vec![],
            });
        }
    }

    // --- coordination bottlenecks
    let mut coord: Vec<(&Hotspot, u32, usize)> = ent
        .iter()
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
        ent.iter().filter(|h| h.defects >= 5 && h.defect_density >= 0.4).collect();
    magnets.sort_by(|a, b| b.defects.cmp(&a.defects));
    for h in magnets.into_iter().take(5) {
        out.push(Finding {
            id: String::new(),
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
    }
    out
}

fn round2(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}
