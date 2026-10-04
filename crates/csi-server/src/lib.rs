//! Local web app: JSON API over the engine plus the embedded Svelte UI, and the
//! single-file static export.

use anyhow::Result;
use axum::extract::{Query, State};
use axum::http::{StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use csi_analysis::coupling::{By, CouplingQuery, coupling};
use csi_analysis::diff::{DiffRequest, diff};
use csi_analysis::findings::findings;
use csi_analysis::hotspots::hotspots;
use csi_analysis::overview::{hotspot_tree, overview};
use csi_analysis::scan::ScanOptions;
use csi_analysis::social::ownership;
use csi_analysis::{Engine, Level, Scope, detail};
use rust_embed::RustEmbed;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

#[derive(RustEmbed)]
#[folder = "../../ui/dist"]
struct Assets;

type Shared = Arc<Mutex<Engine>>;

#[derive(Debug, Default, Deserialize, Clone)]
#[serde(default)]
pub struct Params {
    pub level: Option<String>,
    pub repo: Option<String>,
    pub unit: Option<String>,
    pub path: Option<String>,
    pub days: Option<f64>,
    pub limit: Option<usize>,
    pub by: Option<String>,
    pub focus: Option<String>,
    pub cross: Option<bool>,
    pub min_support: Option<u32>,
    pub key: Option<u32>,
    pub base: Option<String>,
    pub head: Option<String>,
    pub ticket: Option<String>,
}

impl Params {
    fn scope(&self) -> Scope {
        Scope {
            repo: self.repo.clone().filter(|s| !s.is_empty()),
            unit: self.unit.clone().filter(|s| !s.is_empty()),
            path: self.path.clone().filter(|s| !s.is_empty()),
            days: self.days,
        }
    }
    fn level(&self, default: Level) -> Level {
        self.level.as_deref().and_then(Level::parse).unwrap_or(default)
    }
}

/// All API endpoints as plain functions so the static export can call them too.
pub fn api(engine: &mut Engine, endpoint: &str, p: &Params) -> Result<Value> {
    let ds = &engine.ds;
    Ok(match endpoint {
        "meta" => json!({
            "workspace": ds.cfg.workspace.name,
            "repos": ds.repos.iter().map(|r| json!({"name": r.name, "branch": r.branch, "frameworks": r.frameworks})).collect::<Vec<_>>(),
            "units": ds.units.iter().map(|u| json!({"name": u.name, "repo": ds.repos[u.repo as usize].name, "kind": u.kind})).collect::<Vec<_>>(),
            "now": ds.now,
            "version": env!("CARGO_PKG_VERSION"),
        }),
        "overview" => serde_json::to_value(overview(ds))?,
        "findings" => {
            let mut f = findings(ds, Some(&engine.db), &p.scope());
            f.truncate(p.limit.unwrap_or(100));
            serde_json::to_value(f)?
        }
        "hotspots" => {
            let mut h = hotspots(ds, p.level(Level::Entity), &p.scope());
            h.truncate(p.limit.unwrap_or(1000));
            serde_json::to_value(h)?
        }
        "tree" => serde_json::to_value(hotspot_tree(ds, &p.scope()))?,
        "coupling" => {
            let q = CouplingQuery {
                level: p.level(Level::Entity),
                by: p.by.as_deref().and_then(By::parse).unwrap_or_default(),
                focus: p.focus.clone().filter(|s| !s.is_empty()),
                min_support: p.min_support,
                cross_only: p.cross.unwrap_or(false),
                include_expected: false,
                ..Default::default()
            };
            let mut c = coupling(ds, &q, &p.scope());
            c.truncate(p.limit.unwrap_or(500));
            serde_json::to_value(c)?
        }
        "owners" => {
            let mut o = ownership(ds, p.level(Level::Unit), &p.scope());
            o.sort_by(|a, b| b.knowledge_loss.total_cmp(&a.knowledge_loss).then(a.name.cmp(&b.name)));
            o.truncate(p.limit.unwrap_or(1000));
            serde_json::to_value(o)?
        }
        "architecture" => architecture(engine)?,
        "entity" => {
            let level = p.level(Level::Entity);
            let key = p.key.ok_or_else(|| anyhow::anyhow!("missing key"))?;
            if level == Level::Entity && (key as usize) < engine.ds.entities.len() {
                // deep data for the main file on first view
                let main = engine.ds.entities[key as usize]
                    .files
                    .iter()
                    .copied()
                    .filter(|&f| engine.ds.files[f as usize].alive)
                    .find(|&f| !engine.ds.files[f as usize].test && engine.ds.files[f as usize].lang.is_parsed());
                if let Some(f) = main {
                    engine.ensure_deep(f)?;
                }
            } else if level == Level::File && (key as usize) < engine.ds.files.len() {
                engine.ensure_deep(key)?;
            }
            serde_json::to_value(detail::detail(&engine.ds, &engine.db, level, key)?)?
        }
        "diff" => {
            let req = DiffRequest {
                repo: p.repo.clone().filter(|s| !s.is_empty()),
                base: p.base.clone().filter(|s| !s.is_empty()),
                head: p.head.clone().filter(|s| !s.is_empty()),
                ticket: p.ticket.clone().filter(|s| !s.is_empty()),
                cwd: None,
            };
            let r = diff(ds, &engine.db, &req)?;
            let md = r.to_markdown();
            let mut v = serde_json::to_value(r)?;
            v["markdown"] = Value::String(md);
            v
        }
        other => anyhow::bail!("unknown endpoint {other}"),
    })
}

/// Units with their metrics and the coupling edges between them (commit and ticket).
fn architecture(engine: &Engine) -> Result<Value> {
    let ds = &engine.ds;
    let all = Scope::default();
    let hot = hotspots(ds, Level::Unit, &all);
    let own: HashMap<u32, _> = ownership(ds, Level::Unit, &all).into_iter().map(|o| (o.key, o)).collect();
    let units: Vec<Value> = hot
        .iter()
        .map(|h| {
            let o = own.get(&h.key);
            let u = &ds.units[h.key as usize];
            json!({
                "key": h.key, "name": u.name, "label": ds.unit_label(h.key), "repo": h.repo, "kind": u.kind, "tags": u.tags,
                "loc": h.loc, "files": h.files, "score": h.score, "revisions": h.revisions, "health": h.health,
                "bus_factor": o.map(|o| o.bus_factor), "knowledge_loss": o.map(|o| o.knowledge_loss),
                "main_dev": o.and_then(|o| o.main_dev.clone()), "rank": h.rank,
            })
        })
        .collect();
    let mut edges: BTreeMap<(u32, u32), Value> = BTreeMap::new();
    for by in [By::Commit, By::Ticket] {
        let q = CouplingQuery {
            level: Level::Unit,
            by,
            min_support: Some(3),
            min_confidence: Some(0.1),
            min_lift: Some(1.0),
            ..Default::default()
        };
        for c in coupling(ds, &q, &all).into_iter().take(400) {
            if by == By::Ticket && !c.cross_repo {
                continue;
            }
            edges.entry((c.a, c.b)).or_insert_with(|| {
                json!({"a": c.a, "b": c.b, "support": c.support, "conf_ab": c.conf_ab, "conf_ba": c.conf_ba,
                       "lift": c.lift, "cross_repo": c.cross_repo, "by": by})
            });
        }
    }
    Ok(json!({"units": units, "edges": edges.into_values().collect::<Vec<_>>()}))
}

fn err(e: anyhow::Error) -> Response {
    (StatusCode::BAD_REQUEST, Json(json!({"error": format!("{e:#}")}))).into_response()
}

async fn handle(State(s): State<Shared>, uri: Uri, Query(p): Query<Params>) -> Response {
    let endpoint = uri.path().trim_start_matches("/api/").to_string();
    let res = tokio::task::spawn_blocking(move || {
        let mut engine = s.lock().unwrap_or_else(|e| e.into_inner());
        api(&mut engine, &endpoint, &p)
    })
    .await;
    match res {
        Ok(Ok(v)) => Json(v).into_response(),
        Ok(Err(e)) => err(e),
        Err(e) => err(anyhow::anyhow!("{e}")),
    }
}

async fn rescan(State(s): State<Shared>) -> Response {
    let res = tokio::task::spawn_blocking(move || {
        let mut engine = s.lock().unwrap_or_else(|e| e.into_inner());
        let noop = |_: &str| {};
        engine.scan(&ScanOptions::default(), &noop)
    })
    .await;
    match res {
        Ok(Ok(r)) => Json(serde_json::to_value(r).unwrap_or_default()).into_response(),
        Ok(Err(e)) => err(e),
        Err(e) => err(anyhow::anyhow!("{e}")),
    }
}

async fn static_file(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };
    match Assets::get(path).or_else(|| Assets::get("index.html")) {
        Some(f) => {
            let mime = mime_guess::from_path(if Assets::get(path).is_some() { path } else { "index.html" })
                .first_or_octet_stream();
            ([(header::CONTENT_TYPE, mime.as_ref().to_string())], f.data.into_owned()).into_response()
        }
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

pub fn router(engine: Engine) -> Router {
    let shared: Shared = Arc::new(Mutex::new(engine));
    Router::new()
        .route("/api/scan", post(rescan))
        .route("/api/{*endpoint}", get(handle))
        .fallback(static_file)
        .with_state(shared)
}

pub async fn serve(engine: Engine, port: u16, open: bool) -> Result<()> {
    let app = router(engine);
    let mut listener = None;
    for p in port..port.saturating_add(20) {
        if let Ok(l) = tokio::net::TcpListener::bind(("127.0.0.1", p)).await {
            listener = Some(l);
            break;
        }
    }
    let listener = listener.ok_or_else(|| anyhow::anyhow!("no free port from {port}"))?;
    let url = format!("http://{}", listener.local_addr()?);
    eprintln!("csi web app: {url}  (Ctrl+C to stop)");
    if open {
        let cmd = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
        let _ = std::process::Command::new(cmd).arg(&url).spawn();
    }
    axum::serve(listener, app).await?;
    Ok(())
}

/// A self-contained HTML report: the UI with API responses baked in.
pub fn export_html(engine: &Engine) -> Result<String> {
    // The export needs `&mut` only for on-demand deep data; work on a reloaded engine.
    let mut engine = Engine::open(engine.cfg.clone(), false, &|_: &str| {})?;
    let mut data: BTreeMap<String, Value> = BTreeMap::new();
    let p = Params::default();
    for ep in ["meta", "overview", "findings", "tree", "architecture"] {
        data.insert(ep.to_string(), api(&mut engine, ep, &p)?);
    }
    for level in ["file", "entity", "unit", "repo"] {
        let lp = Params { level: Some(level.into()), ..Default::default() };
        data.insert(format!("hotspots?level={level}"), api(&mut engine, "hotspots", &lp)?);
        data.insert(format!("owners?level={level}"), api(&mut engine, "owners", &lp)?);
    }
    for level in ["file", "entity", "unit"] {
        for by in ["commit", "ticket"] {
            let cp = Params { level: Some(level.into()), by: Some(by.into()), ..Default::default() };
            data.insert(format!("coupling?by={by}&level={level}"), api(&mut engine, "coupling", &cp)?);
        }
    }
    // details for the entities a reader is most likely to open
    let mut keys: Vec<u32> =
        hotspots(&engine.ds, Level::Entity, &Scope::default()).iter().take(60).map(|h| h.key).collect();
    if let Some(Value::Array(fs)) = data.get("findings") {
        for f in fs {
            if f["level"] == "entity"
                && let Some(k) = f["key"].as_u64()
            {
                keys.push(k as u32);
            }
        }
    }
    keys.sort();
    keys.dedup();
    for k in keys {
        let ep = Params { level: Some("entity".into()), key: Some(k), ..Default::default() };
        if let Ok(v) = api(&mut engine, "entity", &ep) {
            data.insert(format!("entity?key={k}&level=entity"), v);
        }
    }
    let index = Assets::get("index.html").ok_or_else(|| anyhow::anyhow!("UI not embedded"))?;
    let html = String::from_utf8_lossy(&index.data).into_owned();
    let payload = serde_json::to_string(&data)?.replace("</", "<\\/");
    let script = format!("<script>window.__CSI_STATIC__={payload};</script>");
    Ok(match html.find("</head>") {
        Some(i) => format!("{}{}{}", &html[..i], script, &html[i..]),
        None => format!("{script}{html}"),
    })
}
