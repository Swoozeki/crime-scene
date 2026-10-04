//! Tornhill's indentation complexity: the sum of logical indentation levels of non-blank lines.

pub fn complexity(text: &str) -> f64 {
    let indents: Vec<usize> = text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.chars().take_while(|c| *c == ' ' || *c == '\t').map(|c| if c == '\t' { 4 } else { 1 }).sum())
        .collect();
    // Detect the indentation unit (2 or 4 spaces usually) from the smallest positive indent.
    let unit = indents.iter().copied().filter(|&i| i > 0).min().unwrap_or(4).clamp(2, 8) as f64;
    indents.iter().map(|&i| (i as f64 / unit).floor()).sum::<f64>().max(1.0)
}
