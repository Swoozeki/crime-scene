//! Small numeric and string helpers shared by the analysis crates.

pub const DAY: f64 = 86_400.0;

/// FNV-1a 64-bit hash; stable across runs (used for cache directory names).
pub fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// Exponential decay weight for an event `age_secs` old: 1.0 now, 0.5 after one half-life.
pub fn decay(age_secs: f64, half_life_days: f64) -> f64 {
    if half_life_days <= 0.0 {
        return 1.0;
    }
    let age_days = (age_secs / DAY).max(0.0);
    (-(age_days / half_life_days) * std::f64::consts::LN_2).exp()
}

/// Percentile ranks (0..=1) of `values`; ties share the average rank. Single values rank 1.0.
pub fn percentile_ranks(values: &[f64]) -> Vec<f64> {
    let n = values.len();
    if n == 0 {
        return vec![];
    }
    if n == 1 {
        return vec![1.0];
    }
    let mut idx: Vec<usize> = (0..n).collect();
    idx.sort_by(|&a, &b| values[a].total_cmp(&values[b]));
    let mut ranks = vec![0.0; n];
    let mut i = 0;
    while i < n {
        let mut j = i;
        while j + 1 < n && values[idx[j + 1]] == values[idx[i]] {
            j += 1;
        }
        // average 0-based position of the tie group, scaled to 0..=1
        let avg = (i + j) as f64 / 2.0 / (n - 1) as f64;
        for k in i..=j {
            ranks[idx[k]] = avg;
        }
        i = j + 1;
    }
    ranks
}

/// Fold a name for identity matching: lowercase, strip accents and non-alphanumerics.
pub fn normalize_name(name: &str) -> String {
    name.chars()
        .filter_map(|c| {
            let c = fold_accent(c);
            c.is_alphanumeric().then(|| c.to_lowercase().next().unwrap_or(c))
        })
        .collect()
}

fn fold_accent(c: char) -> char {
    match c {
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'À' | 'Á' | 'Â' | 'Ã' | 'Ä' | 'Å' => 'a',
        'è' | 'é' | 'ê' | 'ë' | 'È' | 'É' | 'Ê' | 'Ë' => 'e',
        'ì' | 'í' | 'î' | 'ï' | 'Ì' | 'Í' | 'Î' | 'Ï' => 'i',
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'Ò' | 'Ó' | 'Ô' | 'Õ' | 'Ö' | 'Ø' => 'o',
        'ù' | 'ú' | 'û' | 'ü' | 'Ù' | 'Ú' | 'Û' | 'Ü' => 'u',
        'ñ' | 'Ñ' => 'n',
        'ç' | 'Ç' => 'c',
        'ý' | 'ÿ' | 'Ý' => 'y',
        _ => c,
    }
}

/// Parse "3 years", "18 months", "90 days", "2y", "6m" into seconds. Empty → None (all history).
pub fn parse_duration_secs(s: &str) -> Option<f64> {
    let s = s.trim().to_lowercase();
    if s.is_empty() || s == "all" {
        return None;
    }
    let num: String = s.chars().take_while(|c| c.is_ascii_digit() || *c == '.').collect();
    let unit = s[num.len()..].trim().trim_end_matches(" ago");
    let n: f64 = num.parse().ok()?;
    let days = match unit {
        "y" | "yr" | "yrs" | "year" | "years" => 365.25,
        "m" | "mo" | "month" | "months" => 30.44,
        "w" | "week" | "weeks" => 7.0,
        "d" | "day" | "days" => 1.0,
        _ => return None,
    };
    Some(n * days * DAY)
}

/// Truncate to `max` chars, appending an ellipsis.
pub fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut t: String = s.chars().take(max.saturating_sub(1)).collect();
        t.push('…');
        t
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decay_halves_after_half_life() {
        assert!((decay(0.0, 180.0) - 1.0).abs() < 1e-9);
        assert!((decay(180.0 * DAY, 180.0) - 0.5).abs() < 1e-9);
        assert!((decay(360.0 * DAY, 180.0) - 0.25).abs() < 1e-9);
    }

    #[test]
    fn percentiles_handle_ties() {
        let r = percentile_ranks(&[1.0, 5.0, 5.0, 10.0]);
        assert_eq!(r[0], 0.0);
        assert_eq!(r[3], 1.0);
        assert_eq!(r[1], r[2]);
        assert!((r[1] - 0.5).abs() < 1e-9);
    }

    #[test]
    fn durations() {
        assert_eq!(parse_duration_secs("90 days"), Some(90.0 * DAY));
        assert!(parse_duration_secs("3 years").unwrap() > 1000.0 * DAY);
        assert_eq!(parse_duration_secs(""), None);
        assert_eq!(parse_duration_secs("2y").map(|s| (s / DAY) as i64), Some(730));
    }

    #[test]
    fn names_fold() {
        assert_eq!(normalize_name("José  García"), normalize_name("jose garcia"));
    }
}
