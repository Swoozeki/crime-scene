//! `csi init`: discover repositories and write a commented crimescene.toml.

use crate::out::c;
use anyhow::{Result, bail};
use csi_core::config::CONFIG_FILE;
use std::path::{Path, PathBuf};

pub fn run(dir: &Path, force: bool) -> Result<()> {
    let dir = dir.canonicalize()?;
    let target = dir.join(CONFIG_FILE);
    if target.exists() && !force {
        bail!("{} already exists (use --force to overwrite)", target.display());
    }
    let repos = discover(&dir);
    if repos.is_empty() {
        bail!("no git repositories found under {}", dir.display());
    }
    let name = dir.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "workspace".into());
    let mut s = format!("# csi workspace — see `csi --help`\n\n[workspace]\nname = \"{name}\"\n\n");
    let mut used = std::collections::HashSet::new();
    for r in &repos {
        let rel = r.strip_prefix(&dir).unwrap_or(r);
        let rel_s = if rel.as_os_str().is_empty() { ".".to_string() } else { rel.to_string_lossy().to_string() };
        let mut rname = r.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "repo".into());
        while !used.insert(rname.clone()) {
            rname.push('_');
        }
        s.push_str(&format!("[[repo]]\nname = \"{rname}\"\npath = \"{rel_s}\"\n# branch = \"main\"   # default: origin's default branch\n\n"));
    }
    s.push_str(TEMPLATE);
    std::fs::write(&target, s)?;
    println!("{} wrote {} with {} repo{}", c::green("✓"), target.display(), repos.len(), if repos.len() == 1 { "" } else { "s" });
    for r in &repos {
        println!("  {}", c::dim(&r.display().to_string()));
    }
    println!("Next: {}", c::bold("csi scan"));
    let gi = dir.join(".gitignore");
    if dir.join(".git").exists() && !std::fs::read_to_string(&gi).unwrap_or_default().contains(".csi") {
        println!("{}", c::dim("Tip: add `.csi/` (the cache) to .gitignore"));
    }
    Ok(())
}

fn discover(dir: &Path) -> Vec<PathBuf> {
    if dir.join(".git").exists() {
        return vec![dir.to_path_buf()];
    }
    let mut out = vec![];
    for e in walkdir::WalkDir::new(dir)
        .min_depth(1)
        .max_depth(3)
        .into_iter()
        .filter_entry(|e| {
            let n = e.file_name().to_string_lossy();
            e.file_type().is_dir() && !n.starts_with('.') && n != "node_modules" && n != "vendor"
        })
        .flatten()
    {
        if e.path().join(".git").exists() && !out.iter().any(|p: &PathBuf| e.path().starts_with(p)) {
            out.push(e.path().to_path_buf());
        }
    }
    out.sort();
    out
}

const TEMPLATE: &str = r#"[analysis]
since = "3 years"            # history window ("all" for everything)
half_life_days = 180         # recency decay for change frequency
max_commit_files = 60        # bigger commits are excluded from coupling
coupling_min_support = 5
coupling_min_confidence = 0.3
coupling_min_lift = 1.5
inactive_after_days = 180    # authors silent this long count as knowledge loss
top_n_xray = 50              # files that get function-level X-ray on scan
trend_samples = 12

[exclude]
globs = []                   # added to built-in excludes (lock files, dist, vendor, binaries, ...)

[tickets]
pattern = '\b[A-Z][A-Z0-9]+-\d+\b'

[defects]
pattern = '(?i)\b(fix(es|ed|ing)?|bug(fix)?|hotfix|defect|regression|broken|crash(es)?)\b'

[authors]
# bots = ['(?i)\[bot\]', '(?i)dependabot']   # replaces the defaults
[authors.aliases]
# "Jane Doe" = ["jdoe@old-corp.com", "Jane D"]

[teams]
# payments = ["Jane Doe", "raj@corp.com"]

# [[architecture.unit]]       # optional: name units yourself (globs win over auto-detection)
# name = "checkout-mfe"
# repo = "shell"
# glob = "apps/checkout/**"
# kind = "mfe"
"#;
