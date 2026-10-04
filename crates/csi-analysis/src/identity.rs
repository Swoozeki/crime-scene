//! Author identity resolution: mailmap (applied by git) → config aliases → shared email or
//! normalized name. Bots are flagged by regex.

use csi_core::Config;
use csi_core::util::normalize_name;
use std::collections::HashMap;

pub struct Identity {
    pub id: i64,
    pub name: String,
    pub email: String,
    pub commits: u32,
}

pub struct Resolved {
    pub name: String,
    pub emails: Vec<String>,
    pub is_bot: bool,
    pub team: Option<String>,
}

/// Returns (identity db id -> author index, authors).
pub fn resolve(idents: &[Identity], cfg: &Config) -> (HashMap<i64, u32>, Vec<Resolved>) {
    let n = idents.len();
    let mut parent: Vec<usize> = (0..n).collect();
    fn find(p: &mut [usize], mut x: usize) -> usize {
        while p[x] != x {
            p[x] = p[p[x]];
            x = p[x];
        }
        x
    }
    fn union(p: &mut [usize], a: usize, b: usize) {
        let (ra, rb) = (find(p, a), find(p, b));
        if ra != rb {
            p[rb] = ra;
        }
    }

    // config aliases: alias (normalized name or lowercase email) -> canonical
    let mut alias_of: HashMap<String, String> = HashMap::new();
    for (canonical, aliases) in &cfg.authors.aliases {
        alias_of.insert(normalize_name(canonical), canonical.clone());
        for a in aliases {
            let key = if a.contains('@') { a.to_lowercase() } else { normalize_name(a) };
            alias_of.insert(key, canonical.clone());
        }
    }

    let mut by_key: HashMap<String, usize> = HashMap::new();
    let mut canonical_of_root: HashMap<usize, String> = HashMap::new();
    let mut canonical_idx: Vec<Option<String>> = vec![None; n];
    for (i, ident) in idents.iter().enumerate() {
        let email = ident.email.to_lowercase();
        let norm = normalize_name(&ident.name);
        let mut keys = vec![];
        if !email.is_empty() && email.contains('@') {
            keys.push(format!("e:{email}"));
        }
        if norm.chars().count() >= 3 {
            keys.push(format!("n:{norm}"));
        }
        let canon = alias_of.get(&email).or_else(|| alias_of.get(&norm)).cloned();
        if let Some(c) = &canon {
            keys.push(format!("c:{c}"));
        }
        canonical_idx[i] = canon;
        for k in keys {
            match by_key.get(&k) {
                Some(&j) => union(&mut parent, j, i),
                None => {
                    by_key.insert(k, i);
                }
            }
        }
    }
    for (i, c) in canonical_idx.iter().enumerate() {
        if let Some(c) = c.clone() {
            let r = find(&mut parent, i);
            canonical_of_root.insert(r, c);
        }
    }

    let bots = cfg.bot_regexes();
    let teams: Vec<(String, Vec<String>)> = cfg
        .teams
        .iter()
        .map(|(t, members)| {
            (
                t.clone(),
                members.iter().map(|m| if m.contains('@') { m.to_lowercase() } else { normalize_name(m) }).collect(),
            )
        })
        .collect();

    let mut groups: HashMap<usize, Vec<usize>> = HashMap::new();
    for i in 0..n {
        let r = find(&mut parent, i);
        groups.entry(r).or_default().push(i);
    }
    let mut roots: Vec<usize> = groups.keys().copied().collect();
    roots.sort();
    let mut map = HashMap::new();
    let mut out = vec![];
    for r in roots {
        let members = &groups[&r];
        let display = canonical_of_root.get(&r).cloned().unwrap_or_else(|| {
            members
                .iter()
                .max_by_key(|&&i| (idents[i].commits, std::cmp::Reverse(idents[i].id)))
                .map(|&i| idents[i].name.clone())
                .unwrap_or_default()
        });
        let mut emails: Vec<String> = members.iter().map(|&i| idents[i].email.to_lowercase()).collect();
        emails.sort();
        emails.dedup();
        let is_bot =
            members.iter().any(|&i| bots.iter().any(|b| b.is_match(&idents[i].name) || b.is_match(&idents[i].email)));
        let mut keys: Vec<String> = emails.clone();
        keys.push(normalize_name(&display));
        keys.extend(members.iter().map(|&i| normalize_name(&idents[i].name)));
        let team = teams.iter().find(|(_, ms)| ms.iter().any(|m| keys.contains(m))).map(|(t, _)| t.clone());
        let idx = out.len() as u32;
        for &i in members {
            map.insert(idents[i].id, idx);
        }
        out.push(Resolved { name: display, emails, is_bot, team });
    }
    (map, out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ident(id: i64, name: &str, email: &str, commits: u32) -> Identity {
        Identity { id, name: name.into(), email: email.into(), commits }
    }

    #[test]
    fn merges_by_email_name_and_alias() {
        let mut cfg = Config::default();
        cfg.authors.aliases.insert("Jane Doe".into(), vec!["jd@old.com".into()]);
        cfg.teams.insert("payments".into(), vec!["Jane Doe".into()]);
        let ids = vec![
            ident(1, "Jane Doe", "jane@new.com", 10),
            ident(2, "jane doe", "jane@laptop.local", 2),
            ident(3, "JD", "jd@old.com", 5),
            ident(4, "Raj", "raj@x.com", 3),
            ident(5, "dependabot[bot]", "49699333+dependabot[bot]@users.noreply.github.com", 40),
        ];
        let (map, authors) = resolve(&ids, &cfg);
        assert_eq!(map[&1], map[&2]);
        assert_eq!(map[&1], map[&3]);
        assert_ne!(map[&1], map[&4]);
        let jane = &authors[map[&1] as usize];
        assert_eq!(jane.name, "Jane Doe");
        assert_eq!(jane.team.as_deref(), Some("payments"));
        assert!(authors[map[&5] as usize].is_bot);
        assert!(!jane.is_bot);
    }
}
