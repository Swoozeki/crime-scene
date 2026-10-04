//! `crimescene.toml` workspace configuration.

use anyhow::{Context, Result, bail};
use globset::{Glob, GlobSet, GlobSetBuilder};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const CONFIG_FILE: &str = "crimescene.toml";

/// Globs that are always excluded from analysis (lock files, vendored and built output, binaries).
pub const DEFAULT_EXCLUDES: &[&str] = &[
    "**/package-lock.json",
    "**/pnpm-lock.yaml",
    "**/yarn.lock",
    "**/composer.lock",
    "**/Cargo.lock",
    "**/bun.lockb",
    "**/*.min.js",
    "**/*.min.css",
    "**/*.map",
    "**/dist/**",
    "**/build/**",
    "**/coverage/**",
    "**/vendor/**",
    "**/node_modules/**",
    "**/.angular/**",
    "**/.nx/**",
    "**/*.generated.*",
    "**/*.snap",
    "**/__snapshots__/**",
    "**/*.{png,jpg,jpeg,gif,ico,webp,avif,bmp,svg,woff,woff2,ttf,otf,eot,pdf,zip,gz,tgz,jar,war,mp3,mp4,mov,webm,wav,psd,sketch,fig,exe,dll,so,dylib,bin,dat,db,sqlite}",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub workspace: WorkspaceConfig,
    #[serde(rename = "repo")]
    pub repos: Vec<RepoConfig>,
    pub analysis: AnalysisConfig,
    pub exclude: ExcludeConfig,
    pub tickets: PatternConfig,
    pub defects: PatternConfig,
    pub authors: AuthorsConfig,
    /// team name -> author names or emails
    pub teams: BTreeMap<String, Vec<String>>,
    pub architecture: ArchitectureConfig,

    /// Directory the config was loaded from; repo paths are relative to it.
    #[serde(skip)]
    pub root: PathBuf,
    /// Where the SQLite cache lives.
    #[serde(skip)]
    pub cache_path: PathBuf,
    /// Path of the config file, if one exists on disk.
    #[serde(skip)]
    pub file: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct WorkspaceConfig {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepoConfig {
    pub name: String,
    pub path: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AnalysisConfig {
    /// History window passed to `git log --since`, e.g. "3 years". Empty = all history.
    pub since: String,
    pub half_life_days: f64,
    pub max_commit_files: usize,
    pub coupling_min_support: u32,
    pub coupling_min_confidence: f64,
    pub coupling_min_lift: f64,
    pub inactive_after_days: f64,
    pub top_n_xray: usize,
    pub trend_samples: usize,
    pub max_file_bytes: usize,
}

impl Default for AnalysisConfig {
    fn default() -> Self {
        Self {
            since: "3 years".into(),
            half_life_days: 180.0,
            max_commit_files: 60,
            coupling_min_support: 5,
            coupling_min_confidence: 0.3,
            coupling_min_lift: 1.5,
            inactive_after_days: 180.0,
            top_n_xray: 50,
            trend_samples: 12,
            max_file_bytes: 512 * 1024,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct ExcludeConfig {
    pub globs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[derive(Default)]
pub struct PatternConfig {
    pub pattern: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AuthorsConfig {
    pub bots: Vec<String>,
    /// canonical name -> aliases (names or emails)
    pub aliases: BTreeMap<String, Vec<String>>,
}

impl Default for AuthorsConfig {
    fn default() -> Self {
        Self {
            bots: vec![
                r"(?i)\[bot\]".into(),
                r"(?i)dependabot".into(),
                r"(?i)renovate".into(),
                r"(?i)github-actions".into(),
                r"(?i)^(ci|build|jenkins|gitlab-ci|semantic-release)(-bot)?\b".into(),
                r"(?i)noreply@github\.com$".into(),
                // AI assistants credited via Co-authored-by trailers
                r"(?i)noreply@anthropic\.com$".into(),
                r"(?i)^(claude|copilot|github copilot|cursor( agent)?|devin|codex|chatgpt|gemini|aider)\b".into(),
            ],
            aliases: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct ArchitectureConfig {
    #[serde(rename = "unit")]
    pub units: Vec<UnitConfig>,
    /// Depth of the directory fallback when no plugin detects units.
    pub fallback_depth: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnitConfig {
    pub name: String,
    #[serde(default)]
    pub repo: Option<String>,
    pub glob: String,
    #[serde(default = "default_unit_kind")]
    pub kind: String,
}

fn default_unit_kind() -> String {
    "module".into()
}

pub const DEFAULT_TICKET_PATTERN: &str = r"\b[A-Z][A-Z0-9]+-\d+\b";
pub const DEFAULT_DEFECT_PATTERN: &str =
    r"(?i)\b(fix(es|ed|ing)?|bug(fix)?|hotfix|defect|regression|broken|crash(es)?)\b";

impl Default for Config {
    fn default() -> Self {
        Self {
            workspace: WorkspaceConfig::default(),
            repos: vec![],
            analysis: AnalysisConfig::default(),
            exclude: ExcludeConfig::default(),
            tickets: PatternConfig { pattern: DEFAULT_TICKET_PATTERN.into() },
            defects: PatternConfig { pattern: DEFAULT_DEFECT_PATTERN.into() },
            authors: AuthorsConfig::default(),
            teams: BTreeMap::new(),
            architecture: ArchitectureConfig::default(),
            root: PathBuf::new(),
            cache_path: PathBuf::new(),
            file: None,
        }
    }
}

impl Config {
    /// Load an explicit config file.
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let mut cfg: Config = toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        let root = path.canonicalize()?.parent().map(Path::to_path_buf).unwrap_or_default();
        if cfg.workspace.name.is_empty() {
            cfg.workspace.name = dir_name(&root);
        }
        cfg.cache_path = root.join(".csi").join("cache.db");
        cfg.root = root;
        cfg.file = Some(path.to_path_buf());
        cfg.finish()?;
        Ok(cfg)
    }

    /// Find a config by walking up from `start`; otherwise build an implicit single-repo
    /// workspace if `start` is inside a git repository.
    pub fn discover(start: &Path) -> Result<Self> {
        let start = start.canonicalize().unwrap_or_else(|_| start.to_path_buf());
        for dir in start.ancestors() {
            let candidate = dir.join(CONFIG_FILE);
            if candidate.is_file() {
                return Self::load(&candidate);
            }
        }
        let Some(repo_root) = start.ancestors().find(|d| d.join(".git").exists()) else {
            bail!(
                "no {CONFIG_FILE} found and {} is not inside a git repository.\n\
                 Run `csi init <dir>` to create a workspace.",
                start.display()
            );
        };
        Ok(Self::implicit(repo_root))
    }

    /// A zero-config workspace containing a single repository.
    pub fn implicit(repo_root: &Path) -> Self {
        let name = dir_name(repo_root);
        let mut cfg = Config {
            workspace: WorkspaceConfig { name: name.clone() },
            repos: vec![RepoConfig { name, path: PathBuf::from("."), branch: None }],
            root: repo_root.to_path_buf(),
            ..Default::default()
        };
        let base = dirs::cache_dir().unwrap_or_else(std::env::temp_dir);
        cfg.cache_path = base
            .join("csi")
            .join(format!("{:016x}", crate::util::fnv1a(repo_root.to_string_lossy().as_bytes())))
            .join("cache.db");
        cfg.finish().expect("default config is valid");
        cfg
    }

    fn finish(&mut self) -> Result<()> {
        if self.tickets.pattern.is_empty() {
            self.tickets.pattern = DEFAULT_TICKET_PATTERN.into();
        }
        if self.defects.pattern.is_empty() {
            self.defects.pattern = DEFAULT_DEFECT_PATTERN.into();
        }
        let mut seen = std::collections::HashSet::new();
        for r in &self.repos {
            if !seen.insert(r.name.clone()) {
                bail!("duplicate repo name `{}` in config", r.name);
            }
        }
        // Validate patterns early so errors point at the config.
        Regex::new(&self.tickets.pattern).context("invalid [tickets].pattern")?;
        Regex::new(&self.defects.pattern).context("invalid [defects].pattern")?;
        for b in &self.authors.bots {
            Regex::new(b).with_context(|| format!("invalid bot pattern `{b}`"))?;
        }
        self.exclude_set()?;
        Ok(())
    }

    pub fn repo_path(&self, repo: &RepoConfig) -> PathBuf {
        let p = if repo.path.is_absolute() { repo.path.clone() } else { self.root.join(&repo.path) };
        p.canonicalize().unwrap_or(p)
    }

    pub fn exclude_set(&self) -> Result<GlobSet> {
        let mut b = GlobSetBuilder::new();
        for g in DEFAULT_EXCLUDES.iter().copied().chain(self.exclude.globs.iter().map(String::as_str)) {
            b.add(Glob::new(g).with_context(|| format!("invalid exclude glob `{g}`"))?);
            // `**/x/**` should also match a top-level `x/...`; globset already handles `**/` as optional.
        }
        Ok(b.build()?)
    }

    pub fn ticket_regex(&self) -> Regex {
        Regex::new(&self.tickets.pattern).expect("validated")
    }

    pub fn defect_regex(&self) -> Regex {
        Regex::new(&self.defects.pattern).expect("validated")
    }

    pub fn bot_regexes(&self) -> Vec<Regex> {
        self.authors.bots.iter().map(|b| Regex::new(b).expect("validated")).collect()
    }

    /// Serialize to TOML for `csi init`.
    pub fn to_toml(&self) -> Result<String> {
        Ok(toml::to_string_pretty(self)?)
    }
}

fn dir_name(p: &Path) -> String {
    p.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "workspace".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_full_config() {
        let dir = std::env::temp_dir().join(format!("csi-cfg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(CONFIG_FILE);
        std::fs::write(
            &path,
            r#"
[workspace]
name = "work"
[[repo]]
name = "shell"
path = "../shell"
[analysis]
half_life_days = 90
[authors.aliases]
"Jane Doe" = ["jd@x.com"]
[teams]
payments = ["Jane Doe"]
[[architecture.unit]]
name = "checkout"
glob = "apps/checkout/**"
kind = "mfe"
"#,
        )
        .unwrap();
        let cfg = Config::load(&path).unwrap();
        assert_eq!(cfg.workspace.name, "work");
        assert_eq!(cfg.analysis.half_life_days, 90.0);
        assert_eq!(cfg.analysis.max_commit_files, 60);
        assert_eq!(cfg.repos[0].name, "shell");
        assert_eq!(cfg.architecture.units[0].kind, "mfe");
        assert!(cfg.cache_path.ends_with(".csi/cache.db"));
        let ex = cfg.exclude_set().unwrap();
        assert!(ex.is_match("package-lock.json"));
        assert!(ex.is_match("apps/a/node_modules/x/index.js"));
        assert!(ex.is_match("src/assets/logo.png"));
        assert!(!ex.is_match("src/app/app.component.ts"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn rejects_unknown_keys() {
        assert!(toml::from_str::<Config>("[analysis]\nhalf_lif = 3").is_err());
    }
}
