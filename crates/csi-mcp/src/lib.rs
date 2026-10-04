//! MCP server (stdio): lets a coding agent ask about hotspots, coupling and experts for the
//! code it is about to change. Never writes to stdout except protocol messages.

use anyhow::Result;
use csi_analysis::coupling::{By, CouplingQuery, coupling};
use csi_analysis::diff::{DiffRequest, diff};
use csi_analysis::findings::findings;
use csi_analysis::hotspots::hotspots;
use csi_analysis::social::experts;
use csi_analysis::{Engine, Level, Scope, detail, xray};
use csi_core::Config;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerConfig};
use rmcp::{ErrorData, ServerHandler, ServiceExt, schemars, tool, tool_handler, tool_router};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const INSTRUCTIONS: &str = "csi analyzes git history (behavioral code analysis). Before modifying a file, \
call `file_context` with its path: it tells you whether the file is a hotspot (changes often and is complex), \
which other files usually change together with it — including in other repositories — that you may need to \
update too, which functions are the riskiest, and who the experts are. Use `review_diff` before finishing a \
change to check for touched hotspots and forgotten coupled files.";

#[derive(Clone)]
pub struct CsiMcp {
    engine: Arc<Mutex<Engine>>,
    last_check: Arc<Mutex<Instant>>,
    auto_scan: bool,
    tool_router: ToolRouter<Self>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct PathArgs {
    /// File path: absolute, relative to the working directory, or repo-relative.
    pub path: String,
    /// Repository name when the workspace has several and the path is ambiguous.
    #[serde(default)]
    pub repo: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct HotspotArgs {
    /// file | entity (default; Angular components group .ts/.html/.scss) | unit | repo
    #[serde(default)]
    pub level: Option<String>,
    #[serde(default)]
    pub repo: Option<String>,
    /// Architectural unit name (micro-frontend, lib, feature, service, ...)
    #[serde(default)]
    pub unit: Option<String>,
    /// Path prefix inside the repo
    #[serde(default)]
    pub path: Option<String>,
    /// Max rows (default 15)
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CouplingArgs {
    pub path: String,
    #[serde(default)]
    pub repo: Option<String>,
    /// commit (default; = PR with squash merges) or ticket (all commits sharing a ticket ID, across repos)
    #[serde(default)]
    pub by: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct DiffArgs {
    #[serde(default)]
    pub repo: Option<String>,
    /// Base revision (default: merge-base with the analyzed branch)
    #[serde(default)]
    pub base: Option<String>,
    /// Head revision (default: HEAD)
    #[serde(default)]
    pub head: Option<String>,
    /// Analyze a ticket ID across all repos instead of a git range
    #[serde(default)]
    pub ticket: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct FindingsArgs {
    #[serde(default)]
    pub limit: Option<usize>,
    /// hotspot | deteriorating_hotspot | xray_hotspot | hidden_coupling | shotgun_surgery | knowledge_loss | bus_factor | coordination | defect_magnet | bloated_component
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub repo: Option<String>,
}

fn ok(v: Value) -> Result<CallToolResult, ErrorData> {
    Ok(CallToolResult::success(vec![ContentBlock::text(serde_json::to_string_pretty(&v).unwrap_or_default())]))
}

fn fail(msg: impl std::fmt::Display) -> Result<CallToolResult, ErrorData> {
    Ok(CallToolResult::error(vec![ContentBlock::text(msg.to_string())]))
}

impl CsiMcp {
    fn new(engine: Engine, auto_scan: bool) -> Self {
        Self {
            engine: Arc::new(Mutex::new(engine)),
            last_check: Arc::new(Mutex::new(Instant::now())),
            auto_scan,
            tool_router: Self::tool_router(),
        }
    }

    /// Run `f` on the engine in a blocking task, rescanning first if repos moved (≤ once a minute).
    async fn with_engine<F>(&self, f: F) -> Result<CallToolResult, ErrorData>
    where
        F: FnOnce(&mut Engine) -> Result<CallToolResult, ErrorData> + Send + 'static,
    {
        let engine = self.engine.clone();
        let last = self.last_check.clone();
        let auto = self.auto_scan;
        tokio::task::spawn_blocking(move || {
            let mut e = engine.lock().unwrap_or_else(|p| p.into_inner());
            if auto {
                let mut l = last.lock().unwrap_or_else(|p| p.into_inner());
                if l.elapsed() > Duration::from_secs(60) {
                    *l = Instant::now();
                    let _ = e.refresh(&|_: &str| {});
                }
            }
            f(&mut e)
        })
        .await
        .map_err(|e| ErrorData::internal_error(e.to_string(), None))?
    }
}

fn resolve(engine: &Engine, path: &str, repo: Option<&str>) -> Option<u32> {
    if let Ok(cwd) = std::env::current_dir()
        && let Ok(abs) = cwd.join(path).canonicalize()
        && let Some(f) = engine.ds.find_file(&abs.to_string_lossy(), repo)
    {
        return Some(f);
    }
    engine.ds.find_file(path, repo)
}

fn not_found(path: &str) -> Result<CallToolResult, ErrorData> {
    fail(format!(
        "`{path}` is not a tracked file in the analyzed workspace (it may be new, excluded, or ambiguous — pass `repo`)."
    ))
}

#[tool_router(router = tool_router)]
impl CsiMcp {
    #[tool(
        description = "Everything csi knows about a file before you change it: hotspot rank and score, code health with reasons, \
the riskiest functions (X-ray), files that usually change together with it (including other repos via ticket IDs), \
experts to ask, defect history and recent tickets. Call this before editing a file."
    )]
    async fn file_context(&self, Parameters(a): Parameters<PathArgs>) -> Result<CallToolResult, ErrorData> {
        self.with_engine(move |engine| {
            let Some(f) = resolve(engine, &a.path, a.repo.as_deref()) else { return not_found(&a.path) };
            let _ = engine.ensure_deep(f);
            let ds = &engine.ds;
            let file = &ds.files[f as usize];
            let fh = hotspots(ds, Level::File, &Scope { repo: Some(ds.repos[file.repo as usize].name.clone()), ..Default::default() });
            let total = fh.len();
            let row = fh.into_iter().find(|h| h.key == f);
            let d = match detail::detail(ds, &engine.db, Level::Entity, file.entity) {
                Ok(d) => d,
                Err(e) => return fail(e),
            };
            let x = xray::xray(ds, &engine.db, f).ok();
            let partners: Vec<Value> = d
                .coupling
                .iter()
                .filter(|p| p.confidence >= 0.3)
                .take(10)
                .map(|p| json!({"entity": p.name, "repo": p.repo, "changes_together_pct": (p.confidence * 100.0).round(), "times": p.support, "by": p.by, "cross_repo": p.cross_repo}))
                .collect();
            let file_partners: Vec<Value> = coupling(
                ds,
                &CouplingQuery { level: Level::File, include_expected: true, include_tests: true, min_lift: Some(1.0), focus: Some(file.path.clone()), ..Default::default() },
                &Scope::default(),
            )
            .into_iter()
            .filter_map(|c| {
                let (other, conf) = if c.a == f { (c.b_name.clone(), c.conf_ab) } else if c.b == f { (c.a_name.clone(), c.conf_ba) } else { return None };
                (conf >= 0.3).then(|| json!({"file": other, "changes_together_pct": (conf * 100.0).round(), "times": c.support}))
            })
            .take(10)
            .collect();
            let risky_fns: Vec<Value> = x
                .as_ref()
                .map(|x| {
                    x.functions
                        .iter()
                        .filter(|f| f.revisions > 0)
                        .take(5)
                        .map(|f| json!({"function": f.name, "lines": [f.start, f.end], "cc": f.cc, "changes": f.revisions}))
                        .collect()
                })
                .unwrap_or_default();
            let hot = row.as_ref().map(|h| h.score >= 0.5).unwrap_or(false);
            let summary = match &row {
                Some(h) if hot => format!(
                    "HOTSPOT #{} of {} files (score {:.2}): changed {}× with complexity {:.0} and health {:.1}/10. Change carefully, keep the change small, and check the coupled files.",
                    h.rank, total, h.score, h.revisions, h.complexity, h.health
                ),
                Some(h) => format!("Not a hotspot (rank {} of {}, score {:.2}, health {:.1}/10).", h.rank, total, h.score, h.health),
                None => "No history for this file in the analyzed window.".into(),
            };
            ok(json!({
                "summary": summary,
                "file": file.path,
                "repo": ds.repos[file.repo as usize].name,
                "entity": d.name,
                "unit": ds.unit_label(file.unit),
                "hotspot": row.as_ref().map(|h| json!({"rank": h.rank, "of": total, "score": h.score, "changes": h.revisions, "complexity": h.complexity, "loc": h.loc, "trend": h.trend, "defect_fixes": h.defects})),
                "health": {"score": file.health(), "reasons": file.summary.health_reasons},
                "angular": file.summary.angular,
                "riskiest_functions": risky_fns,
                "changes_together_with_files": file_partners,
                "changes_together_with_entities": partners,
                "experts": d.experts.iter().map(|e| json!({"name": e.name, "email": e.email})).collect::<Vec<_>>(),
                "knowledge": d.ownership.as_ref().map(|o| json!({"main_dev": o.main_dev, "bus_factor": o.bus_factor, "knowledge_loss_pct": (o.knowledge_loss * 100.0).round()})),
                "recent_tickets": d.tickets,
            }))
        })
        .await
    }

    #[tool(
        description = "Ranked hotspots (change frequency × complexity) for the workspace, a repo, a unit or a path."
    )]
    async fn hotspots(&self, Parameters(a): Parameters<HotspotArgs>) -> Result<CallToolResult, ErrorData> {
        self.with_engine(move |engine| {
            let level = a.level.as_deref().and_then(Level::parse).unwrap_or(Level::Entity);
            let scope = Scope { repo: a.repo, unit: a.unit, path: a.path, days: None };
            let rows: Vec<Value> = hotspots(&engine.ds, level, &scope)
                .into_iter()
                .take(a.limit.unwrap_or(15))
                .map(|h| json!({"rank": h.rank, "name": h.name, "repo": h.repo, "unit": h.unit, "score": h.score, "changes": h.revisions, "complexity": h.complexity, "loc": h.loc, "health": h.health, "main_dev": h.main_dev, "trend": h.trend.map(|t| t.direction)}))
                .collect();
            ok(json!(rows))
        })
        .await
    }

    #[tool(
        description = "Files that change together with the given file (change coupling), with confidence and support. by=ticket finds coupling across repositories."
    )]
    async fn coupling(&self, Parameters(a): Parameters<CouplingArgs>) -> Result<CallToolResult, ErrorData> {
        self.with_engine(move |engine| {
            let Some(f) = resolve(engine, &a.path, a.repo.as_deref()) else { return not_found(&a.path) };
            let ds = &engine.ds;
            let by = a.by.as_deref().and_then(By::parse).unwrap_or_default();
            let q = CouplingQuery { level: Level::File, by, include_expected: true, include_tests: true, min_lift: Some(1.0), focus: Some(ds.files[f as usize].path.clone()), ..Default::default() };
            let rows: Vec<Value> = coupling(ds, &q, &Scope::default())
                .into_iter()
                .filter_map(|c| {
                    let (other, repo, conf) = if c.a == f { (c.b_name, c.b_repo, c.conf_ab) } else if c.b == f { (c.a_name, c.a_repo, c.conf_ba) } else { return None };
                    Some(json!({"file": other, "repo": repo, "changes_together_pct": (conf * 100.0).round(), "times": c.support, "lift": c.lift, "cross_repo": c.cross_repo, "test_pair": c.test_pair}))
                })
                .take(20)
                .collect();
            ok(json!(rows))
        })
        .await
    }

    #[tool(
        description = "Function-level hotspots inside one file: which functions change most and how complex they are."
    )]
    async fn xray(&self, Parameters(a): Parameters<PathArgs>) -> Result<CallToolResult, ErrorData> {
        self.with_engine(move |engine| {
            let Some(f) = resolve(engine, &a.path, a.repo.as_deref()) else { return not_found(&a.path) };
            if let Err(e) = engine.ensure_deep(f) {
                return fail(e);
            }
            match xray::xray(&engine.ds, &engine.db, f) {
                Ok(mut x) => {
                    x.functions.truncate(25);
                    ok(serde_json::to_value(x).unwrap_or_default())
                }
                Err(e) => fail(e),
            }
        })
        .await
    }

    #[tool(description = "Active people who know a file best (recency-weighted), to ask or request review from.")]
    async fn experts(&self, Parameters(a): Parameters<PathArgs>) -> Result<CallToolResult, ErrorData> {
        self.with_engine(move |engine| {
            let Some(f) = resolve(engine, &a.path, a.repo.as_deref()) else { return not_found(&a.path) };
            let ds = &engine.ds;
            let files = ds.entities[ds.files[f as usize].entity as usize].files.clone();
            ok(serde_json::to_value(experts(ds, &files, &[]).into_iter().take(8).collect::<Vec<_>>())
                .unwrap_or_default())
        })
        .await
    }

    #[tool(
        description = "Risk review of a change: a git range in one repo (default: current branch vs. its merge-base) or a ticket ID across repos. Reports touched hotspots, complexity deltas, likely-forgotten coupled files and reviewers."
    )]
    async fn review_diff(&self, Parameters(a): Parameters<DiffArgs>) -> Result<CallToolResult, ErrorData> {
        self.with_engine(move |engine| {
            let req = DiffRequest {
                repo: a.repo,
                base: a.base,
                head: a.head,
                ticket: a.ticket,
                cwd: std::env::current_dir().ok().map(|p| p.to_string_lossy().to_string()),
            };
            match diff(&engine.ds, &engine.db, &req) {
                Ok(r) => {
                    let md = r.to_markdown();
                    let mut v = serde_json::to_value(&r).unwrap_or_default();
                    v["markdown"] = Value::String(md);
                    ok(v)
                }
                Err(e) => fail(format!("{e:#}")),
            }
        })
        .await
    }

    #[tool(
        description = "Ranked findings for the workspace: hotspots, function hotspots, hidden coupling, knowledge loss, bus factor, coordination bottlenecks, defect magnets, bloated Angular components."
    )]
    async fn findings(&self, Parameters(a): Parameters<FindingsArgs>) -> Result<CallToolResult, ErrorData> {
        self.with_engine(move |engine| {
            let scope = Scope { repo: a.repo, ..Default::default() };
            let mut f = findings(&engine.ds, Some(&engine.db), &scope);
            if let Some(k) = &a.kind {
                f.retain(|x| &x.kind == k);
            }
            f.truncate(a.limit.unwrap_or(15));
            let rows: Vec<Value> = f
                .into_iter()
                .map(|x| json!({"id": x.id, "severity": x.severity, "kind": x.kind, "title": x.title, "subject": x.subject, "repo": x.repo, "recommendation": x.recommendation, "evidence": x.evidence}))
                .collect();
            ok(json!(rows))
        })
        .await
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for CsiMcp {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("csi", env!("CARGO_PKG_VERSION")))
            .with_instructions(INSTRUCTIONS)
    }
}

/// Serve MCP over stdio until the client disconnects.
pub async fn run(cfg: Config, auto_scan: bool) -> Result<()> {
    let engine = tokio::task::spawn_blocking(move || Engine::open(cfg, auto_scan, &|_: &str| {})).await??;
    let server = CsiMcp::new(engine, auto_scan);
    let service = server.serve(rmcp::transport::stdio()).await?;
    service.waiting().await?;
    Ok(())
}
