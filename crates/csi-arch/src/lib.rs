//! Architecture model: maps every file to a logical entity and an architectural unit.
//!
//! Units come from framework plugins (Angular workspaces, Nx projects, module federation,
//! Node packages, PHP frameworks), manual config globs, and finally a directory fallback.

use csi_core::config::UnitConfig;
use globset::{Glob, GlobMatcher};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::LazyLock;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Unit {
    pub name: String,
    /// app | lib | mfe | feature | module | package | layer | service | dir
    pub kind: String,
    /// Path prefix without trailing slash; "" is the repository root.
    pub root: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    pub source: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct RepoArch {
    pub units: Vec<Unit>,
    pub frameworks: Vec<String>,
}

/// Files whose content plugins need to read.
pub fn wants_content(path: &str) -> bool {
    let name = basename(path);
    matches!(name, "angular.json" | "nx.json" | "project.json" | "package.json" | "composer.json")
        || FEDERATION_FILE.is_match(name)
}

static FEDERATION_FILE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^((module-)?federation\.config|webpack(\.[a-z]+)?\.config)\.(js|ts|mjs|cjs|json)$").unwrap()
});
static FEDERATION_CONTENT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"ModuleFederationPlugin|withModuleFederation(Plugin)?|withNativeFederation|@module-federation/|shareAll\s*\(")
        .unwrap()
});

fn basename(p: &str) -> &str {
    p.rsplit('/').next().unwrap_or(p)
}

fn dirname(p: &str) -> &str {
    p.rsplit_once('/').map(|(d, _)| d).unwrap_or("")
}

fn is_ignored_dir(p: &str) -> bool {
    p.split('/').any(|c| matches!(c, "node_modules" | "vendor" | "dist" | ".angular" | ".nx" | "coverage"))
}

/// Detect units for one repository. `read` returns the content of a path (only called for
/// paths where [`wants_content`] is true). `repo_name` names a unit rooted at the repo root.
pub fn detect(paths: &[&str], read: &mut dyn FnMut(&str) -> Option<String>, repo_name: &str) -> RepoArch {
    let mut units: Vec<Unit> = vec![];
    let mut frameworks: BTreeSet<String> = BTreeSet::new();
    let path_set: HashSet<&str> = paths.iter().copied().collect();
    let add = |units: &mut Vec<Unit>, u: Unit| {
        if let Some(existing) = units.iter_mut().find(|x| x.root == u.root) {
            // keep the more specific kind from framework plugins over generic packages
            if existing.source == "node" && u.source != "node" {
                *existing = u;
            }
            return;
        }
        units.push(u);
    };
    let unit_name = |root: &str, fallback: &str| -> String {
        if root.is_empty() { repo_name.to_string() } else { fallback.to_string() }
    };

    // --- Angular workspace
    if path_set.contains("angular.json") {
        frameworks.insert("angular".into());
        if let Some(json) = read("angular.json").and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok()) {
            if let Some(projects) = json.get("projects").and_then(|p| p.as_object()) {
                for (name, p) in projects {
                    let root = p.get("root").and_then(|r| r.as_str()).unwrap_or("");
                    let root = if root.is_empty() {
                        p.get("sourceRoot").and_then(|r| r.as_str()).unwrap_or("")
                    } else {
                        root
                    };
                    let kind = match p.get("projectType").and_then(|t| t.as_str()) {
                        Some("library") => "lib",
                        _ => "app",
                    };
                    add(&mut units, Unit {
                        name: name.clone(),
                        kind: kind.into(),
                        root: root.trim_end_matches('/').to_string(),
                        tags: vec![],
                        source: "angular".into(),
                    });
                }
            }
        }
    }

    // --- Nx projects
    if path_set.contains("nx.json") {
        frameworks.insert("nx".into());
    }
    for p in paths.iter().filter(|p| basename(p) == "project.json" && !is_ignored_dir(p)) {
        let Some(json) = read(p).and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok()) else { continue };
        let root = dirname(p).to_string();
        let name = json.get("name").and_then(|n| n.as_str()).map(str::to_string).unwrap_or_else(|| {
            if root.is_empty() { repo_name.to_string() } else { basename(&root).to_string() }
        });
        let kind = match json.get("projectType").and_then(|t| t.as_str()) {
            Some("library") => "lib",
            _ => "app",
        };
        let tags = json
            .get("tags")
            .and_then(|t| t.as_array())
            .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
            .unwrap_or_default();
        frameworks.insert("nx".into());
        add(&mut units, Unit { name, kind: kind.into(), root, tags, source: "nx".into() });
    }

    // --- Node packages (and NestJS)
    for p in paths.iter().filter(|p| basename(p) == "package.json" && !is_ignored_dir(p)) {
        let Some(text) = read(p) else { continue };
        let json: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
        let root = dirname(p).to_string();
        if text.contains("\"@nestjs/core\"") {
            frameworks.insert("nestjs".into());
        }
        if text.contains("\"@angular/core\"") {
            frameworks.insert("angular".into());
        }
        frameworks.insert("node".into());
        if root.is_empty() {
            continue; // the repo itself; not a useful boundary
        }
        let name = json.get("name").and_then(|n| n.as_str()).unwrap_or(basename(&root)).to_string();
        add(&mut units, Unit { name, kind: "package".into(), root, tags: vec![], source: "node".into() });
    }

    // --- PHP / composer
    for p in paths.iter().filter(|p| basename(p) == "composer.json" && !is_ignored_dir(p)) {
        let text = read(p).unwrap_or_default();
        let root = dirname(p).to_string();
        frameworks.insert("php".into());
        let prefix = if root.is_empty() { String::new() } else { format!("{root}/") };
        let laravel = text.contains("laravel/framework") || path_set.contains(format!("{prefix}artisan").as_str());
        let symfony = text.contains("symfony/framework-bundle");
        if laravel {
            frameworks.insert("laravel".into());
            for base in ["app", "app/Http"] {
                let base_p = format!("{prefix}{base}/");
                let mut dirs: BTreeSet<String> = BTreeSet::new();
                for f in paths.iter().filter(|f| f.starts_with(&base_p)) {
                    let rest = &f[base_p.len()..];
                    if let Some((d, _)) = rest.split_once('/') {
                        if !(base == "app" && d == "Http") {
                            dirs.insert(d.to_string());
                        }
                    }
                }
                for d in dirs {
                    add(&mut units, Unit {
                        name: format!("{base}/{d}"),
                        kind: "layer".into(),
                        root: format!("{prefix}{base}/{d}"),
                        tags: vec![],
                        source: "laravel".into(),
                    });
                }
            }
        } else if symfony {
            frameworks.insert("symfony".into());
            let base_p = format!("{prefix}src/");
            let dirs: BTreeSet<String> = paths
                .iter()
                .filter_map(|f| f.strip_prefix(base_p.as_str()).and_then(|r| r.split_once('/')).map(|(d, _)| d.to_string()))
                .collect();
            for d in dirs {
                add(&mut units, Unit {
                    name: format!("src/{d}"),
                    kind: "layer".into(),
                    root: format!("{prefix}src/{d}"),
                    tags: vec![],
                    source: "symfony".into(),
                });
            }
        }
        if !root.is_empty() {
            let name = serde_json::from_str::<serde_json::Value>(&text)
                .ok()
                .and_then(|j| j.get("name").and_then(|n| n.as_str()).map(str::to_string))
                .unwrap_or_else(|| basename(&root).to_string());
            add(&mut units, Unit { name, kind: "service".into(), root, tags: vec![], source: "php".into() });
        }
    }

    // --- Feature modules: Angular NgModules / route files, NestJS modules
    let mut feature_dirs: BTreeSet<String> = BTreeSet::new();
    for p in paths.iter().filter(|p| !is_ignored_dir(p) && !csi_lang_is_test(p)) {
        let name = basename(p);
        let is_feature = (name.ends_with(".module.ts") && !name.ends_with("-routing.module.ts") && name != "app.module.ts")
            || (name.ends_with(".routes.ts") && name != "app.routes.ts");
        if is_feature {
            feature_dirs.insert(dirname(p).to_string());
        }
    }
    for d in feature_dirs {
        if d.is_empty() || units.iter().any(|u| u.root == d) {
            continue;
        }
        add(&mut units, Unit {
            name: basename(&d).to_string(),
            kind: "feature".into(),
            root: d,
            tags: vec![],
            source: "angular".into(),
        });
    }

    // --- Module federation: mark the owning unit as a micro-frontend
    for p in paths.iter().filter(|p| FEDERATION_FILE.is_match(basename(p)) && !is_ignored_dir(p)) {
        let Some(text) = read(p) else { continue };
        if !FEDERATION_CONTENT.is_match(&text) && !basename(p).contains("federation") {
            continue;
        }
        frameworks.insert("module-federation".into());
        let role = if text.contains("exposes") { "remote" } else if text.contains("remotes") { "host" } else { "mfe" };
        let dir = dirname(p).to_string();
        // the unit that owns this config: exact root, else nearest ancestor that isn't a feature
        let owner = units
            .iter_mut()
            .filter(|u| u.kind != "feature" && (u.root == dir || dir.starts_with(&format!("{}/", u.root)) || u.root.is_empty()))
            .max_by_key(|u| u.root.len());
        match owner {
            Some(u) if !u.root.is_empty() || dir.is_empty() => {
                u.kind = "mfe".into();
                if !u.tags.contains(&role.to_string()) {
                    u.tags.push(role.into());
                }
            }
            _ => add(&mut units, Unit {
                name: unit_name(&dir, basename(&dir)),
                kind: "mfe".into(),
                root: dir,
                tags: vec![role.into()],
                source: "module-federation".into(),
            }),
        }
    }

    // Unique names within the repo.
    let mut seen: HashMap<String, usize> = HashMap::new();
    for u in units.iter_mut() {
        let n = seen.entry(u.name.clone()).or_insert(0);
        *n += 1;
        if *n > 1 {
            u.name = if u.root.is_empty() { format!("{}#{n}", u.name) } else { u.root.clone() };
        }
    }
    RepoArch { units, frameworks: frameworks.into_iter().collect() }
}

// Avoid a dependency on csi-lang for one helper.
fn csi_lang_is_test(p: &str) -> bool {
    let l = p.to_ascii_lowercase();
    l.contains(".spec.") || l.contains("/test/") || l.contains("/tests/") || l.contains("/e2e/") || l.starts_with("tests/")
}

/// Resolves files to units for one repo: manual globs, then longest plugin root, then directory fallback.
pub struct UnitIndex {
    manual: Vec<(GlobMatcher, Unit)>,
    by_root: Vec<Unit>,
    fallback_depth: Option<usize>,
}

const CONTAINERS: &[&str] = &[
    "apps", "libs", "packages", "projects", "services", "modules", "src", "app", "lib", "components", "features",
    "pages", "domains", "Http", "Controllers", "resources", "assets", "js", "ts",
];

impl UnitIndex {
    pub fn new(arch: &RepoArch, manual: &[UnitConfig], repo_name: &str, fallback_depth: Option<usize>) -> Self {
        let manual = manual
            .iter()
            .filter(|u| u.repo.as_deref().is_none_or(|r| r == repo_name))
            .filter_map(|u| {
                let g = Glob::new(&u.glob).ok()?.compile_matcher();
                Some((g, Unit {
                    name: u.name.clone(),
                    kind: u.kind.clone(),
                    root: u.glob.clone(),
                    tags: vec![],
                    source: "config".into(),
                }))
            })
            .collect();
        let mut by_root = arch.units.clone();
        by_root.sort_by(|a, b| b.root.len().cmp(&a.root.len()));
        Self { manual, by_root, fallback_depth }
    }

    /// (unit name, kind) for a path.
    pub fn resolve(&self, path: &str) -> (String, String) {
        for (g, u) in &self.manual {
            if g.is_match(path) {
                return (u.name.clone(), u.kind.clone());
            }
        }
        for u in &self.by_root {
            if u.root.is_empty() {
                continue; // repo-root units only catch what the fallback cannot name
            }
            if path.starts_with(&u.root) && path.as_bytes().get(u.root.len()) == Some(&b'/') {
                return (u.name.clone(), u.kind.clone());
            }
        }
        // Files outside every plugin unit (including single-project repos rooted at "")
        // are subdivided by directory.
        (fallback_unit(path, self.fallback_depth), "dir".into())
    }
}

/// Directory-based unit: descends through container dirs (`src`, `apps`, `app`, ...).
pub fn fallback_unit(path: &str, depth: Option<usize>) -> String {
    let dirs: Vec<&str> = path.split('/').collect();
    let dirs = &dirs[..dirs.len().saturating_sub(1)];
    if dirs.is_empty() {
        return "(root)".into();
    }
    if let Some(d) = depth {
        return dirs[..d.min(dirs.len()).max(1)].join("/");
    }
    let mut n = 1;
    while n < dirs.len() && CONTAINERS.contains(&dirs[n - 1]) && n < 5 {
        n += 1;
    }
    dirs[..n].join("/")
}

/// Grouping key for logical entities: `dir/stem` where the stem drops the extension and
/// test/story suffixes, so `x.component.{ts,html,scss,spec.ts}` share one entity.
pub fn entity_key(path: &str) -> String {
    let (dir, name) = match path.rsplit_once('/') {
        Some((d, n)) => (d, n),
        None => ("", path),
    };
    let mut stem = match name.rsplit_once('.') {
        Some((s, _)) if !s.is_empty() => s,
        _ => name,
    };
    for suffix in [".spec", ".test", ".stories", ".cy", ".e2e"] {
        if let Some(s) = stem.strip_suffix(suffix) {
            stem = s;
            break;
        }
    }
    if dir.is_empty() { stem.to_string() } else { format!("{dir}/{stem}") }
}

#[cfg(test)]
mod tests;
