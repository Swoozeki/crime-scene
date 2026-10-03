//! "Code health lite": a 1–10 score (10 = healthy) built from explicit, documented penalties.

use crate::{FileMetrics, Lang};

pub const BRAIN_CC: u32 = 15;
pub const DEEP_NESTING: u32 = 4;
pub const LONG_FUNCTION: u32 = 80;
pub const LARGE_FILE: u32 = 600;
pub const HUGE_FILE: u32 = 1500;
pub const MANY_DEPS: u32 = 7;
pub const MANY_INPUTS: u32 = 10;
pub const COMPLEX_TEMPLATE: f64 = 25.0;
pub const MANY_METHODS: u32 = 20;

/// Returns (health, reasons). Non-code files are always 10.
pub fn health(m: &FileMetrics) -> (f64, Vec<String>) {
    let mut p = 0.0f64;
    let mut reasons = vec![];
    if !m.lang.is_parsed() || m.generated {
        return (10.0, reasons);
    }
    let code = matches!(m.lang, Lang::TypeScript | Lang::Tsx | Lang::JavaScript | Lang::Php);

    if code {
        let brain: Vec<_> = m.functions.iter().filter(|f| f.cc > BRAIN_CC).collect();
        if !brain.is_empty() {
            let pen: f64 = brain.iter().map(|f| (1.0 + (f.cc - BRAIN_CC) as f64 / 10.0).min(3.0)).sum();
            p += pen.min(5.0);
            let worst = brain.iter().max_by_key(|f| f.cc).unwrap();
            reasons.push(format!(
                "{} complex function{} (cc > {BRAIN_CC}); worst `{}` cc {}",
                brain.len(),
                if brain.len() == 1 { "" } else { "s" },
                worst.name,
                worst.cc
            ));
        }
        let long: Vec<_> = m.functions.iter().filter(|f| f.loc > LONG_FUNCTION).collect();
        if !long.is_empty() {
            p += (long.len() as f64 * 0.5).min(2.0);
            let worst = long.iter().max_by_key(|f| f.loc).unwrap();
            reasons.push(format!(
                "{} long function{} (> {LONG_FUNCTION} lines); longest `{}` {} lines",
                long.len(),
                if long.len() == 1 { "" } else { "s" },
                worst.name,
                worst.loc
            ));
        }
    }
    let nesting = m.summary.max_nesting;
    if nesting > DEEP_NESTING {
        p += if nesting > DEEP_NESTING + 2 { 1.5 } else { 1.0 };
        reasons.push(format!("deep nesting (depth {nesting})"));
    }
    if m.loc > HUGE_FILE {
        p += 2.0;
        reasons.push(format!("very large file ({} lines)", m.loc));
    } else if m.loc > LARGE_FILE && code {
        p += 1.0;
        reasons.push(format!("large file ({} lines)", m.loc));
    }
    if let Some(a) = &m.summary.angular {
        if a.deps > MANY_DEPS {
            p += 1.0;
            reasons.push(format!("{} injected dependencies (> {MANY_DEPS})", a.deps));
        }
        if a.inputs > MANY_INPUTS {
            p += 1.0;
            reasons.push(format!("{} inputs (> {MANY_INPUTS})", a.inputs));
        }
    }
    if m.lang == Lang::Template && m.complexity > COMPLEX_TEMPLATE {
        p += ((m.complexity - COMPLEX_TEMPLATE) / 15.0 + 1.0).min(3.0);
        reasons.push(format!("complex template (complexity {})", m.complexity));
    }
    if m.lang == Lang::Php && m.summary.methods > MANY_METHODS {
        p += 1.0;
        reasons.push(format!("{} methods in one file (> {MANY_METHODS})", m.summary.methods));
    }
    let h = (10.0 - p.min(9.0)).max(1.0);
    ((h * 10.0).round() / 10.0, reasons)
}
