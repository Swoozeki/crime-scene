//! Terminal output helpers.

use anyhow::Result;
use clap::ValueEnum;
use comfy_table::{ContentArrangement, Table, presets};
use csi_analysis::Engine;
use csi_analysis::detail::Detail;
use csi_analysis::diff::DiffReport;
use serde::Serialize;
use std::io::IsTerminal;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Fmt {
    Table,
    Json,
    Csv,
    Md,
}

static COLOR: AtomicBool = AtomicBool::new(false);

pub fn init_color(fmt: Fmt) {
    let on = fmt == Fmt::Table && std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none();
    COLOR.store(on, Ordering::Relaxed);
}

pub mod c {
    use super::COLOR;
    use owo_colors::OwoColorize;
    use std::sync::atomic::Ordering;

    fn on() -> bool {
        COLOR.load(Ordering::Relaxed)
    }
    pub fn red(s: &str) -> String {
        if on() { s.red().to_string() } else { s.to_string() }
    }
    pub fn yellow(s: &str) -> String {
        if on() { s.yellow().to_string() } else { s.to_string() }
    }
    pub fn green(s: &str) -> String {
        if on() { s.green().to_string() } else { s.to_string() }
    }
    pub fn dim(s: &str) -> String {
        if on() { s.dimmed().to_string() } else { s.to_string() }
    }
    pub fn bold(s: &str) -> String {
        if on() { s.bold().to_string() } else { s.to_string() }
    }
}

pub fn json<T: Serialize>(v: &T) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(v)?);
    Ok(())
}

pub fn csv(header: &[&str], rows: impl Iterator<Item = Vec<String>>) {
    let esc = |s: &str| {
        if s.contains([',', '"', '\n']) { format!("\"{}\"", s.replace('"', "\"\"")) } else { s.to_string() }
    };
    println!("{}", header.join(","));
    for r in rows {
        println!("{}", r.iter().map(|s| esc(s)).collect::<Vec<_>>().join(","));
    }
}

pub fn table(header: &[&str]) -> Table {
    let mut t = Table::new();
    t.load_preset(presets::UTF8_HORIZONTAL_ONLY).set_content_arrangement(ContentArrangement::Dynamic);
    t.set_header(header.iter().map(|h| c::bold(h)).collect::<Vec<_>>());
    t
}

pub fn bar(x: f64, width: usize) -> String {
    let filled = (x.clamp(0.0, 1.0) * width as f64).round() as usize;
    let s = format!("{}{}", "█".repeat(filled), "·".repeat(width - filled));
    if x >= 0.7 {
        c::red(&s)
    } else if x >= 0.4 {
        c::yellow(&s)
    } else {
        c::dim(&s)
    }
}

pub fn health_cell(h: f64) -> String {
    let s = format!("{h:.1}");
    if h < 4.0 {
        c::red(&s)
    } else if h < 7.0 {
        c::yellow(&s)
    } else {
        c::green(&s)
    }
}

pub fn trend_cell(d: &str) -> String {
    match d {
        "deteriorating" => c::red("↗ worse"),
        "improving" => c::green("↘ better"),
        _ => c::dim("→ stable"),
    }
}

pub fn sparkline(v: &[f64]) -> String {
    const BLOCKS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    let (min, max) = v.iter().fold((f64::MAX, f64::MIN), |(a, b), &x| (a.min(x), b.max(x)));
    v.iter()
        .map(|&x| {
            let i = if max > min { ((x - min) / (max - min) * 7.0).round() as usize } else { 3 };
            BLOCKS[i.min(7)]
        })
        .collect()
}

pub fn date(ts: i64) -> String {
    // civil-from-days (Howard Hinnant), no chrono dependency
    let days = ts.div_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    format!("{}-{:02}-{:02}", if m <= 2 { y + 1 } else { y }, m, d)
}

pub fn rel_time(now: i64, ts: i64) -> String {
    let d = (now - ts) as f64 / 86_400.0;
    if d < 1.0 {
        "today".into()
    } else if d < 60.0 {
        format!("{d:.0}d ago")
    } else if d < 730.0 {
        format!("{:.0}mo ago", d / 30.4)
    } else {
        format!("{:.1}y ago", d / 365.25)
    }
}

pub fn print_detail(engine: &Engine, d: &Detail) {
    println!("{}  {}", c::bold(&d.name), c::dim(&d.repo));
    if let Some(h) = &d.hotspot {
        println!(
            "  hotspot #{} of {} · score {:.2} · {} changes · complexity {:.0} · {} loc · health {}{}",
            h.rank,
            d.total_ranked,
            h.score,
            h.revisions,
            h.complexity,
            h.loc,
            health_cell(h.health),
            h.trend.as_ref().map(|t| format!(" · {}", trend_cell(&t.direction))).unwrap_or_default()
        );
        for r in &h.health_reasons {
            println!("  • {r}");
        }
    }
    println!("\n{}", c::bold("Files"));
    for f in &d.files {
        println!(
            "  {} {}",
            f.path,
            c::dim(&format!(
                "{} · {} loc · cc {:.0} · health {:.1}{}",
                f.lang,
                f.loc,
                f.complexity,
                f.health,
                if f.test { " · test" } else { "" }
            ))
        );
    }
    if let Some(x) = &d.xray {
        let hot: Vec<_> = x.functions.iter().filter(|f| f.revisions > 0).take(8).collect();
        if !hot.is_empty() {
            println!("\n{}", c::bold("X-ray (functions by change × complexity)"));
            for f in hot {
                println!(
                    "  {} {}  {}",
                    bar(f.score, 6),
                    f.name,
                    c::dim(&format!("{}× · cc {} · L{}–{}", f.revisions, f.cc, f.start, f.end))
                );
            }
        }
    }
    if !d.coupling.is_empty() {
        println!("\n{}", c::bold("Changes together with"));
        for p in d.coupling.iter().take(10) {
            let tag = if p.cross_repo {
                c::red(" cross-repo")
            } else if p.cross_unit {
                c::yellow(" cross-unit")
            } else {
                String::new()
            };
            println!(
                "  {:>4.0}%  {}{}{}  {}",
                p.confidence * 100.0,
                if engine.ds.repos.len() > 1 { format!("{}:", p.repo) } else { String::new() },
                p.name,
                tag,
                c::dim(&format!("{}× by {:?}", p.support, p.by).to_lowercase())
            );
        }
    }
    if let Some(o) = &d.ownership {
        println!("\n{}", c::bold("Knowledge"));
        let a: Vec<String> = o
            .authors
            .iter()
            .take(5)
            .map(|a| format!("{}{} {:.0}%", a.name, if a.active { "" } else { "†" }, a.share * 100.0))
            .collect();
        println!("  {}", a.join(", "));
        println!(
            "  {}",
            c::dim(&format!(
                "bus factor {} · knowledge loss {:.0}% · {} authors in the last year",
                o.bus_factor,
                o.knowledge_loss * 100.0,
                o.recent_authors
            ))
        );
    }
    if !d.experts.is_empty() {
        let e: Vec<String> = d.experts.iter().map(|e| e.name.clone()).collect();
        println!("  experts: {}", e.join(", "));
    }
    if !d.commits.is_empty() {
        println!("\n{}", c::bold("Recent commits"));
        for cm in d.commits.iter().take(8) {
            println!(
                "  {} {} {}  {}",
                c::dim(&cm.sha[..8]),
                c::dim(&date(cm.ts)),
                if cm.defect { c::red(&cm.subject) } else { cm.subject.clone() },
                c::dim(&cm.author)
            );
        }
    }
}

pub fn print_diff(r: &DiffReport) {
    let lvl = match r.level.as_str() {
        "high" => c::red("HIGH"),
        "medium" => c::yellow("MEDIUM"),
        _ => c::green("LOW"),
    };
    println!("{} risk {}/100 — {}", lvl, r.risk, c::bold(&r.title));
    for reason in &r.reasons {
        println!("  • {reason}");
    }
    let mut files: Vec<_> = r.files.iter().collect();
    files.sort_by(|a, b| b.hotspot_score.total_cmp(&a.hotspot_score));
    println!();
    let mut t = table(&["file", "+/-", "hotspot", "complexity", "health", "functions"]);
    for f in files.iter().take(25) {
        let cx = match (f.complexity_before, f.complexity_after) {
            (Some(b), Some(a)) if (a - b).abs() >= 1.0 => {
                let s = format!("{b:.0}→{a:.0}");
                if a > b { c::yellow(&s) } else { c::green(&s) }
            }
            (_, Some(a)) => format!("{a:.0}"),
            _ => "–".into(),
        };
        let h = match (f.health_before, f.health_after) {
            (Some(b), Some(a)) if (a - b).abs() >= 0.1 => format!("{b:.1}→{a:.1}"),
            (_, Some(a)) => format!("{a:.1}"),
            _ => "–".into(),
        };
        let fns: Vec<String> = f.functions.iter().take(3).map(|x| x.name.clone()).collect();
        t.add_row(vec![
            if r.repos.len() > 1 { format!("{}:{}", f.repo, f.path) } else { f.path.clone() },
            format!("+{} -{}", f.added, f.deleted),
            f.hotspot_rank
                .filter(|_| f.hotspot_score > 0.0)
                .map(|x| format!("#{x} ({:.2})", f.hotspot_score))
                .unwrap_or_default(),
            cx,
            h,
            fns.join(", "),
        ]);
    }
    println!("{t}");
    if !r.missed.is_empty() {
        println!("\n{}", c::bold("Usually changed together, but not here:"));
        for m in &r.missed {
            println!(
                "  {:>4.0}%  {}{}  {}",
                m.confidence * 100.0,
                if m.kind == "cross-repo" { format!("{}:", m.repo) } else { String::new() },
                m.path,
                c::dim(&format!(
                    "with {} · {}× · {}",
                    m.because_of.rsplit('/').next().unwrap_or(&m.because_of),
                    m.support,
                    m.kind
                ))
            );
        }
    }
    if !r.reviewers.is_empty() {
        let names: Vec<String> = r.reviewers.iter().map(|e| e.name.clone()).collect();
        println!("\n{} {}", c::bold("Suggested reviewers:"), names.join(", "));
    }
}
