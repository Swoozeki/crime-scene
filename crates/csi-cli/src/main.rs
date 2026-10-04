//! `csi` — Crime Scene Investigator: behavioral code analysis from git history.

mod init;
mod out;

use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand, ValueEnum};
use csi_analysis::coupling::{By, CouplingQuery, coupling};
use csi_analysis::diff::{DiffRequest, diff};
use csi_analysis::findings::findings;
use csi_analysis::hotspots::hotspots;
use csi_analysis::scan::ScanOptions;
use csi_analysis::social::{experts, ownership};
use csi_analysis::{Engine, Level, Scope, detail, trends, xray};
use csi_core::Config;
use indicatif::{ProgressBar, ProgressStyle};
use out::{Fmt, bar, c, health_cell, rel_time, sparkline};
use std::io::IsTerminal;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Parser)]
#[command(
    name = "csi",
    version,
    about = "Crime Scene Investigator — find the code that matters, from your git history"
)]
struct Cli {
    /// Path to crimescene.toml (default: search upwards from the current directory)
    #[arg(long, global = true)]
    config: Option<PathBuf>,
    /// Output format
    #[arg(long, short, global = true, value_enum, default_value_t = Fmt::Table)]
    format: Fmt,
    /// Don't rescan automatically when repos have new commits
    #[arg(long, global = true)]
    no_scan: bool,
    /// No progress output
    #[arg(long, short, global = true)]
    quiet: bool,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Args, Clone, Default)]
struct ScopeArgs {
    /// Limit to one repository
    #[arg(long)]
    repo: Option<String>,
    /// Limit to one architectural unit (name or repo:name)
    #[arg(long)]
    unit: Option<String>,
    /// Limit to a path prefix inside the repo
    #[arg(long)]
    path: Option<String>,
    /// Only consider the last N days of history
    #[arg(long)]
    days: Option<f64>,
}

impl ScopeArgs {
    fn scope(&self) -> Scope {
        Scope { repo: self.repo.clone(), unit: self.unit.clone(), path: self.path.clone(), days: self.days }
    }
}

#[derive(Clone, Copy, ValueEnum)]
enum LevelArg {
    File,
    Entity,
    Unit,
    Repo,
}

impl From<LevelArg> for Level {
    fn from(l: LevelArg) -> Level {
        match l {
            LevelArg::File => Level::File,
            LevelArg::Entity => Level::Entity,
            LevelArg::Unit => Level::Unit,
            LevelArg::Repo => Level::Repo,
        }
    }
}

#[derive(Subcommand)]
enum Cmd {
    /// Create crimescene.toml for the git repositories under DIR
    Init {
        #[arg(default_value = ".")]
        dir: PathBuf,
        #[arg(long)]
        force: bool,
    },
    /// Ingest history and measure code (incremental)
    Scan {
        /// Re-ingest everything
        #[arg(long)]
        full: bool,
        #[arg(long)]
        repo: Option<String>,
        /// Skip X-ray and trend sampling
        #[arg(long)]
        shallow: bool,
    },
    /// Ranked findings with evidence and recommendations
    Report {
        #[arg(long, default_value_t = 20)]
        top: usize,
        /// Only findings of this kind (hotspot, hidden_coupling, knowledge_loss, ...)
        #[arg(long)]
        kind: Option<String>,
        #[command(flatten)]
        scope: ScopeArgs,
    },
    /// Hotspots: frequently changed, complex code
    Hotspots {
        #[arg(long, value_enum, default_value_t = LevelArg::Entity)]
        level: LevelArg,
        #[arg(long, short = 'n', default_value_t = 25)]
        limit: usize,
        #[command(flatten)]
        scope: ScopeArgs,
    },
    /// Change coupling: what changes together
    Coupling {
        /// Only pairs involving this path/name
        focus: Option<String>,
        #[arg(long, value_enum, default_value_t = LevelArg::Entity)]
        level: LevelArg,
        /// Group changes by commit (PR) or ticket (crosses repos)
        #[arg(long, default_value = "commit")]
        by: String,
        /// Only pairs crossing unit or repo boundaries
        #[arg(long)]
        cross: bool,
        /// Include pairs where one side is test code
        #[arg(long)]
        tests: bool,
        #[arg(long)]
        min_support: Option<u32>,
        #[arg(long, short = 'n', default_value_t = 30)]
        limit: usize,
        #[command(flatten)]
        scope: ScopeArgs,
    },
    /// Function-level hotspots inside a file
    Xray {
        path: String,
        #[arg(long)]
        repo: Option<String>,
    },
    /// Complexity trend of a file
    Trend {
        path: String,
        #[arg(long)]
        repo: Option<String>,
    },
    /// Ownership, knowledge loss and experts for a path, unit or the workspace
    Owners {
        /// File or directory (default: whole workspace)
        path: Option<String>,
        #[arg(long, value_enum, default_value_t = LevelArg::Unit)]
        level: LevelArg,
        #[arg(long, short = 'n', default_value_t = 25)]
        limit: usize,
        #[command(flatten)]
        scope: ScopeArgs,
    },
    /// Code health of a file
    Health {
        path: String,
        #[arg(long)]
        repo: Option<String>,
    },
    /// Everything about one file or entity
    Show {
        path: String,
        #[arg(long)]
        repo: Option<String>,
    },
    /// Risk report for a branch/PR (`base..head`) or a ticket across repos
    Diff {
        /// `base..head`, `base`, or nothing (merge-base with the analyzed branch .. HEAD)
        range: Option<String>,
        #[arg(long)]
        ticket: Option<String>,
        #[arg(long)]
        repo: Option<String>,
        /// Exit with status 2 when risk is above this
        #[arg(long)]
        fail_above: Option<u32>,
    },
    /// Local web app
    Serve {
        #[arg(long, default_value_t = 7777)]
        port: u16,
        #[arg(long)]
        open: bool,
    },
    /// Write a single-file HTML report to share
    Export {
        #[arg(long, default_value = "csi-report.html")]
        html: PathBuf,
    },
    /// MCP server on stdio for coding agents
    Mcp,
}

fn main() {
    let cli = Cli::parse();
    if let Err(e) = run(cli) {
        eprintln!("{} {e:#}", c::red("error:"));
        std::process::exit(1);
    }
}

fn load_config(cli: &Cli) -> Result<Config> {
    match &cli.config {
        Some(p) => Config::load(p),
        None => Config::discover(&std::env::current_dir()?),
    }
}

fn spinner(quiet: bool) -> ProgressBar {
    if quiet || !std::io::stderr().is_terminal() {
        return ProgressBar::hidden();
    }
    let pb = ProgressBar::new_spinner();
    pb.set_style(ProgressStyle::with_template("{spinner:.cyan} {msg}").unwrap());
    pb.enable_steady_tick(Duration::from_millis(100));
    pb
}

fn open_engine(cli: &Cli, auto_scan: bool) -> Result<Engine> {
    let cfg = load_config(cli)?;
    let pb = spinner(cli.quiet);
    let progress = |m: &str| pb.set_message(m.to_string());
    let engine = Engine::open(cfg, auto_scan && !cli.no_scan, &progress)?;
    pb.finish_and_clear();
    Ok(engine)
}

fn run(cli: Cli) -> Result<()> {
    out::init_color(cli.format);
    match &cli.cmd {
        Cmd::Init { dir, force } => init::run(dir, *force),
        Cmd::Scan { full, repo, shallow } => {
            let cfg = load_config(&cli)?;
            let pb = spinner(cli.quiet);
            let progress = |m: &str| pb.set_message(m.to_string());
            let mut engine = Engine::open(cfg, false, &progress)?;
            let r = engine.scan(&ScanOptions { full: *full, repo: repo.clone(), shallow: *shallow }, &progress)?;
            pb.finish_and_clear();
            if cli.format == Fmt::Json {
                return out::json(&r);
            }
            for repo in &r.repos {
                println!(
                    "{} {}  {} new commits{}  {} files parsed  {}",
                    c::green("✓"),
                    c::bold(&repo.name),
                    repo.new_commits,
                    if repo.full { " (full)" } else { "" },
                    repo.parsed_files,
                    c::dim(&format!("[{}] {}", repo.branch, repo.frameworks.join(", ")))
                );
            }
            let o = csi_analysis::overview::overview(&engine.ds);
            println!(
                "{}",
                c::dim(&format!(
                    "{} commits · {} authors ({} active) · {} files · {} units · x-ray {} commits · {:.1}s",
                    o.commits, o.authors, o.active_authors, o.files, o.units, r.xray_commits, r.seconds
                ))
            );
            println!(
                "Next: {} for ranked findings, {} for the web app",
                c::bold("csi report"),
                c::bold("csi serve --open")
            );
            Ok(())
        }
        Cmd::Report { top, kind, scope } => {
            let engine = open_engine(&cli, true)?;
            let mut f = findings(&engine.ds, Some(&engine.db), &scope.scope());
            if let Some(k) = kind {
                f.retain(|x| &x.kind == k);
            }
            f.truncate(*top);
            match cli.format {
                Fmt::Json => out::json(&f),
                Fmt::Csv => {
                    out::csv(
                        &["id", "severity", "kind", "repo", "subject", "title"],
                        f.iter().map(|x| {
                            vec![
                                x.id.clone(),
                                x.severity.to_string(),
                                x.kind.clone(),
                                x.repo.clone(),
                                x.subject.clone(),
                                x.title.clone(),
                            ]
                        }),
                    );
                    Ok(())
                }
                _ => {
                    let o = csi_analysis::overview::overview(&engine.ds);
                    println!(
                        "{}  {}",
                        c::bold(&format!("Crime scene report — {}", o.workspace)),
                        c::dim(&format!(
                            "{} repos · {} commits · {} files · history {}",
                            o.repos.len(),
                            o.commits,
                            o.files,
                            o.first_commit.map(|t| format!("since {}", out::date(t))).unwrap_or_default()
                        ))
                    );
                    println!();
                    if f.is_empty() {
                        println!("No findings — nothing stands out. Try `csi hotspots` for the full ranking.");
                    }
                    for x in &f {
                        let sev = format!("{:>3}", x.severity);
                        let sev = if x.severity >= 70 {
                            c::red(&sev)
                        } else if x.severity >= 45 {
                            c::yellow(&sev)
                        } else {
                            c::dim(&sev)
                        };
                        println!(
                            "{} {}  {} {}",
                            sev,
                            c::dim(&x.id),
                            c::yellow(&format!("{}:", x.kind_label)),
                            c::bold(&x.title)
                        );
                        let loc = match &x.unit {
                            Some(u) => format!("{u} · {}", x.subject),
                            None => x.subject.clone(),
                        };
                        println!("      {}", c::dim(&loc));
                        println!("      {}", x.recommendation);
                        println!();
                    }
                    Ok(())
                }
            }
        }
        Cmd::Hotspots { level, limit, scope } => {
            let engine = open_engine(&cli, true)?;
            let level: Level = (*level).into();
            let rows: Vec<_> = hotspots(&engine.ds, level, &scope.scope()).into_iter().take(*limit).collect();
            match cli.format {
                Fmt::Json => out::json(&rows),
                Fmt::Csv => {
                    out::csv(
                        &[
                            "rank",
                            "score",
                            "name",
                            "repo",
                            "revisions",
                            "rev_w",
                            "complexity",
                            "loc",
                            "health",
                            "main_dev",
                            "trend",
                        ],
                        rows.iter().map(|h| {
                            vec![
                                h.rank.to_string(),
                                format!("{:.3}", h.score),
                                h.name.clone(),
                                h.repo.clone(),
                                h.revisions.to_string(),
                                format!("{:.2}", h.rev_w),
                                format!("{:.0}", h.complexity),
                                h.loc.to_string(),
                                format!("{:.1}", h.health),
                                h.main_dev.clone().unwrap_or_default(),
                                h.trend.as_ref().map(|t| t.direction.clone()).unwrap_or_default(),
                            ]
                        }),
                    );
                    Ok(())
                }
                _ => {
                    let multi = engine.ds.repos.len() > 1;
                    let mut t = out::table(&[
                        "#",
                        "score",
                        "",
                        "name",
                        "changes",
                        "complexity",
                        "loc",
                        "health",
                        "main dev",
                        "trend",
                    ]);
                    for h in &rows {
                        let name = if multi { format!("{}:{}", h.repo, h.name) } else { h.name.clone() };
                        t.add_row(vec![
                            h.rank.to_string(),
                            format!("{:.2}", h.score),
                            bar(h.score, 8),
                            name,
                            format!("{} ({:.1})", h.revisions, h.rev_w),
                            format!("{:.0}", h.complexity),
                            h.loc.to_string(),
                            health_cell(h.health),
                            h.main_dev
                                .clone()
                                .map(|d| format!("{d} {:.0}%", h.main_dev_share * 100.0))
                                .unwrap_or_default(),
                            h.trend.as_ref().map(|t| out::trend_cell(&t.direction)).unwrap_or_default(),
                        ]);
                    }
                    println!("{t}");
                    println!(
                        "{}",
                        c::dim(
                            "changes = commits (recency-weighted) · score = change frequency × complexity percentile"
                        )
                    );
                    Ok(())
                }
            }
        }
        Cmd::Coupling { focus, level, by, cross, tests, min_support, limit, scope } => {
            let engine = open_engine(&cli, true)?;
            let by = By::parse(by).context("--by must be commit or ticket")?;
            let level: Level = (*level).into();
            let focus = focus.as_ref().map(|f| resolve_name(&engine, f, level));
            let q = CouplingQuery {
                level,
                by,
                focus,
                min_support: *min_support,
                cross_only: *cross,
                include_tests: *tests,
                ..Default::default()
            };
            let rows: Vec<_> = coupling(&engine.ds, &q, &scope.scope()).into_iter().take(*limit).collect();
            match cli.format {
                Fmt::Json => out::json(&rows),
                Fmt::Csv => {
                    out::csv(
                        &["a", "b", "support", "conf_ab", "conf_ba", "lift", "cross_unit", "cross_repo"],
                        rows.iter().map(|r| {
                            vec![
                                r.a_name.clone(),
                                r.b_name.clone(),
                                r.support.to_string(),
                                r.conf_ab.to_string(),
                                r.conf_ba.to_string(),
                                r.lift.to_string(),
                                r.cross_unit.to_string(),
                                r.cross_repo.to_string(),
                            ]
                        }),
                    );
                    Ok(())
                }
                _ => {
                    let multi = engine.ds.repos.len() > 1;
                    let mut t = out::table(&["A", "B", "together", "A→B", "B→A", "lift", "boundary"]);
                    for r in &rows {
                        let name = |n: &str, repo: &str| if multi { format!("{repo}:{n}") } else { n.to_string() };
                        let boundary = if r.cross_repo {
                            c::red("cross-repo")
                        } else if r.cross_unit {
                            c::yellow("cross-unit")
                        } else if r.test_pair {
                            c::dim("test")
                        } else {
                            String::new()
                        };
                        t.add_row(vec![
                            name(&r.a_name, &r.a_repo),
                            name(&r.b_name, &r.b_repo),
                            format!("{} ({} recent)", r.support, r.recent),
                            format!("{:.0}%", r.conf_ab * 100.0),
                            format!("{:.0}%", r.conf_ba * 100.0),
                            format!("{:.1}", r.lift),
                            boundary,
                        ]);
                    }
                    println!("{t}");
                    if rows.is_empty() {
                        println!(
                            "{}",
                            c::dim("No coupling above the thresholds. Lower --min-support or try --by ticket.")
                        );
                    } else {
                        println!(
                            "{}",
                            c::dim(
                                "A→B = share of A's changes that also changed B · lift > 1 = more often than chance"
                            )
                        );
                    }
                    Ok(())
                }
            }
        }
        Cmd::Xray { path, repo } => {
            let mut engine = open_engine(&cli, true)?;
            let f = resolve_file(&engine, path, repo.as_deref())?;
            engine.ensure_deep(f)?;
            let x = xray::xray(&engine.ds, &engine.db, f)?;
            if cli.format == Fmt::Json {
                return out::json(&x);
            }
            println!(
                "{}  {}",
                c::bold(&format!("X-ray {}", x.path)),
                c::dim(&format!("{} commits analyzed", x.analyzed_commits))
            );
            let mut t =
                out::table(&["score", "", "function", "changes", "cc", "nesting", "lines", "authors", "last change"]);
            for func in x.functions.iter().take(30) {
                t.add_row(vec![
                    format!("{:.2}", func.score),
                    bar(func.score, 8),
                    func.name.clone(),
                    format!("{} ({:.1})", func.revisions, func.rev_w),
                    if func.cc > 15 { c::red(&func.cc.to_string()) } else { func.cc.to_string() },
                    func.nesting.to_string(),
                    format!("{}–{}", func.start, func.end),
                    func.authors.to_string(),
                    func.last_change.map(|t| rel_time(engine.ds.now, t)).unwrap_or_default(),
                ]);
            }
            println!("{t}");
            if !x.coupling.is_empty() {
                println!("\n{}", c::bold("Functions that change together"));
                for p in x.coupling.iter().take(10) {
                    println!(
                        "  {} ↔ {}  {}",
                        p.a,
                        p.b,
                        c::dim(&format!("{}× · {:.0}%", p.support, p.confidence * 100.0))
                    );
                }
            }
            Ok(())
        }
        Cmd::Trend { path, repo } => {
            let mut engine = open_engine(&cli, true)?;
            let f = resolve_file(&engine, path, repo.as_deref())?;
            engine.ensure_deep(f)?;
            let points = engine.ds.files[f as usize].trend.clone();
            let cls = trends::classify(&points, engine.ds.now);
            if cli.format == Fmt::Json {
                return out::json(
                    &serde_json::json!({"path": engine.ds.files[f as usize].path, "points": points, "trend": cls}),
                );
            }
            println!("{}", c::bold(&format!("Complexity trend {}", engine.ds.files[f as usize].path)));
            if points.is_empty() {
                println!("Not enough history.");
                return Ok(());
            }
            let cx: Vec<f64> = points.iter().map(|p| p.complexity).collect();
            println!("  {}  {:.0} → {:.0}", sparkline(&cx), cx[0], cx[cx.len() - 1]);
            if let Some(t) = cls {
                println!(
                    "  {} ({:+.0}%){}",
                    out::trend_cell(&t.direction),
                    t.change * 100.0,
                    if t.refactored { c::dim(" · refactoring detected") } else { String::new() }
                );
            }
            let mut t = out::table(&["date", "complexity", "loc", "max fn cc"]);
            for p in &points {
                t.add_row(vec![
                    out::date(p.ts),
                    format!("{:.0}", p.complexity),
                    p.loc.to_string(),
                    p.max_cc.to_string(),
                ]);
            }
            println!("{t}");
            Ok(())
        }
        Cmd::Owners { path, level, limit, scope } => {
            let engine = open_engine(&cli, true)?;
            let ds = &engine.ds;
            let mut scope = scope.scope();
            let mut level: Level = (*level).into();
            let mut focus_files: Vec<u32> = vec![];
            if let Some(p) = path {
                if let Ok(f) = resolve_file(&engine, p, scope.repo.as_deref()) {
                    level = Level::File;
                    scope.repo = Some(ds.repos[ds.files[f as usize].repo as usize].name.clone());
                    scope.path = Some(ds.files[f as usize].path.clone());
                    focus_files.push(f);
                } else {
                    let (repo, rel) = resolve_dir(&engine, p)?;
                    scope.repo = Some(repo);
                    scope.path = Some(rel);
                    if matches!(level, Level::Unit | Level::Repo) {
                        level = Level::Entity;
                    }
                }
            }
            let mut rows = ownership(ds, level, &scope);
            csi_analysis::social::sort_by_importance(ds, level, &scope, &mut rows);
            if focus_files.is_empty() {
                focus_files = (0..ds.files.len() as u32)
                    .filter(|&f| scope.file(ds, &ds.files[f as usize]) && ds.files[f as usize].alive)
                    .collect();
            }
            let ex = experts(ds, &focus_files, &[]);
            if cli.format == Fmt::Json {
                return out::json(
                    &serde_json::json!({"ownership": rows.iter().take(*limit).collect::<Vec<_>>(), "experts": ex.iter().take(10).collect::<Vec<_>>()}),
                );
            }
            let mut t = out::table(&[
                "name",
                "main dev",
                "bus factor",
                "knowledge loss",
                "fragmentation",
                "active (1y)",
                "top authors",
            ]);
            for o in rows.iter().take(*limit) {
                let top: Vec<String> = o
                    .authors
                    .iter()
                    .take(3)
                    .map(|a| format!("{}{} {:.0}%", a.name, if a.active { "" } else { "†" }, a.share * 100.0))
                    .collect();
                t.add_row(vec![
                    o.name.clone(),
                    o.main_dev.clone().map(|d| format!("{d} {:.0}%", o.main_dev_share * 100.0)).unwrap_or_default(),
                    if o.bus_factor == 1 { c::yellow("1") } else { o.bus_factor.to_string() },
                    if o.knowledge_loss >= 0.5 {
                        c::red(&format!("{:.0}%", o.knowledge_loss * 100.0))
                    } else {
                        format!("{:.0}%", o.knowledge_loss * 100.0)
                    },
                    format!("{:.2}", o.fragmentation),
                    o.recent_authors.to_string(),
                    top.join(", "),
                ]);
            }
            println!("{t}");
            println!("{}", c::dim("† = inactive author · knowledge loss = share of code written by inactive authors"));
            if !ex.is_empty() {
                let names: Vec<String> =
                    ex.iter().take(5).map(|e| format!("{} ({:.0}%)", e.name, e.score * 100.0)).collect();
                println!("\n{} {}", c::bold("Experts (active, recent knowledge):"), names.join(", "));
            }
            Ok(())
        }
        Cmd::Health { path, repo } => {
            let engine = open_engine(&cli, true)?;
            let f = resolve_file(&engine, path, repo.as_deref())?;
            let file = &engine.ds.files[f as usize];
            let fns = match &file.blob {
                Some(b) => csi_ingest::metrics::load_functions(&engine.db, b)?,
                None => vec![],
            };
            if cli.format == Fmt::Json {
                return out::json(&serde_json::json!({
                    "path": file.path, "lang": file.lang.as_str(), "loc": file.loc, "complexity": file.complexity,
                    "health": file.health(), "reasons": file.summary.health_reasons, "angular": file.summary.angular, "functions": fns,
                }));
            }
            println!(
                "{}  health {}  {}",
                c::bold(&file.path),
                health_cell(file.health()),
                c::dim(&format!("{} · {} loc · complexity {:.0}", file.lang.as_str(), file.loc, file.complexity))
            );
            for r in &file.summary.health_reasons {
                println!("  • {r}");
            }
            if let Some(a) = &file.summary.angular {
                println!(
                    "  {}",
                    c::dim(&format!(
                        "angular {} · {} deps · {} inputs · {} outputs{}",
                        a.kind,
                        a.deps,
                        a.inputs,
                        a.outputs,
                        if a.standalone { " · standalone" } else { "" }
                    ))
                );
            }
            let mut sorted = fns.clone();
            sorted.sort_by_key(|f| std::cmp::Reverse(f.cc));
            if !sorted.is_empty() {
                let mut t = out::table(&["function", "cc", "nesting", "loc", "lines"]);
                for func in sorted.iter().take(15) {
                    t.add_row(vec![
                        func.name.clone(),
                        if func.cc > 15 { c::red(&func.cc.to_string()) } else { func.cc.to_string() },
                        func.nesting.to_string(),
                        func.loc.to_string(),
                        format!("{}–{}", func.start, func.end),
                    ]);
                }
                println!("{t}");
            }
            Ok(())
        }
        Cmd::Show { path, repo } => {
            let mut engine = open_engine(&cli, true)?;
            let f = resolve_file(&engine, path, repo.as_deref())?;
            engine.ensure_deep(f)?;
            let e = engine.ds.files[f as usize].entity;
            let d = detail::detail(&engine.ds, &engine.db, Level::Entity, e)?;
            if cli.format == Fmt::Json {
                return out::json(&d);
            }
            out::print_detail(&engine, &d);
            Ok(())
        }
        Cmd::Diff { range, ticket, repo, fail_above } => {
            let engine = open_engine(&cli, true)?;
            let (base, head) = match range.as_deref() {
                Some(r) if r.contains("...") => {
                    let (a, b) = r.split_once("...").unwrap();
                    (Some(a.to_string()), Some(if b.is_empty() { "HEAD".to_string() } else { b.to_string() }))
                }
                Some(r) if r.contains("..") => {
                    let (a, b) = r.split_once("..").unwrap();
                    (Some(a.to_string()), Some(if b.is_empty() { "HEAD".to_string() } else { b.to_string() }))
                }
                Some(r) => (Some(r.to_string()), None),
                None => (None, None),
            };
            let req = DiffRequest {
                repo: repo.clone(),
                base,
                head,
                ticket: ticket.clone(),
                cwd: std::env::current_dir().ok().map(|p| p.to_string_lossy().to_string()),
            };
            let r = diff(&engine.ds, &engine.db, &req)?;
            match cli.format {
                Fmt::Json => out::json(&r)?,
                Fmt::Md => print!("{}", r.to_markdown()),
                _ => out::print_diff(&r),
            }
            if let Some(max) = fail_above
                && r.risk > *max
            {
                std::process::exit(2);
            }
            Ok(())
        }
        Cmd::Serve { port, open } => {
            let engine = open_engine(&cli, true)?;
            let rt = tokio::runtime::Runtime::new()?;
            rt.block_on(csi_server::serve(engine, *port, *open))
        }
        Cmd::Export { html } => {
            let engine = open_engine(&cli, true)?;
            let page = csi_server::export_html(&engine)?;
            std::fs::write(html, page).with_context(|| format!("writing {}", html.display()))?;
            println!("{} wrote {}", c::green("✓"), html.display());
            Ok(())
        }
        Cmd::Mcp => {
            let cfg = load_config(&cli)?;
            let rt = tokio::runtime::Runtime::new()?;
            rt.block_on(csi_mcp::run(cfg, !cli.no_scan))
        }
    }
}

/// Resolve a user path (relative to cwd, absolute, repo-relative, or unique suffix) to a file.
fn resolve_file(engine: &Engine, path: &str, repo: Option<&str>) -> Result<u32> {
    let ds = &engine.ds;
    let abs = std::env::current_dir()?.join(path);
    if let Ok(canon) = abs.canonicalize()
        && let Some(f) = ds.find_file(&canon.to_string_lossy(), repo)
    {
        return Ok(f);
    }
    if let Some(f) = ds.find_file(path, repo) {
        return Ok(f);
    }
    bail!("`{path}` is not a tracked file in this workspace (or the name is ambiguous; pass --repo or a longer path)")
}

/// Directory argument → (repo name, repo-relative prefix).
fn resolve_dir(engine: &Engine, path: &str) -> Result<(String, String)> {
    let ds = &engine.ds;
    let abs = std::env::current_dir()?.join(path);
    let canon = abs.canonicalize().unwrap_or(abs);
    for r in &ds.repos {
        if let Ok(rel) = canon.strip_prefix(&r.path) {
            return Ok((r.name.clone(), rel.to_string_lossy().to_string()));
        }
    }
    if ds.repos.len() == 1 {
        return Ok((ds.repos[0].name.clone(), path.trim_start_matches("./").to_string()));
    }
    bail!("`{path}` is not inside any repo of this workspace")
}

/// For `coupling <focus>`: turn a file path into the key name used at `level`.
fn resolve_name(engine: &Engine, focus: &str, level: Level) -> String {
    match resolve_file(engine, focus, None) {
        Ok(f) => engine.ds.key_name(engine.ds.key(f, level), level),
        Err(_) => focus.to_string(),
    }
}
