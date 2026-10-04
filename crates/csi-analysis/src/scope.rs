//! Filters shared by every analysis.

use crate::dataset::{Commit, Dataset, File};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    File,
    #[default]
    Entity,
    Unit,
    Repo,
}

impl Level {
    pub fn parse(s: &str) -> Option<Level> {
        Some(match s {
            "file" | "files" => Level::File,
            "entity" | "entities" => Level::Entity,
            "unit" | "units" => Level::Unit,
            "repo" | "repos" => Level::Repo,
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Level::File => "file",
            Level::Entity => "entity",
            Level::Unit => "unit",
            Level::Repo => "repo",
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Scope {
    /// Repository name.
    pub repo: Option<String>,
    /// Unit name (optionally `repo:unit`).
    pub unit: Option<String>,
    /// Path prefix within the repo.
    pub path: Option<String>,
    /// Only consider commits from the last N days.
    pub days: Option<f64>,
}

impl Scope {
    pub fn is_empty(&self) -> bool {
        self.repo.is_none() && self.unit.is_none() && self.path.is_none() && self.days.is_none()
    }

    pub fn file(&self, ds: &Dataset, f: &File) -> bool {
        if let Some(r) = &self.repo
            && &ds.repos[f.repo as usize].name != r
        {
            return false;
        }
        if let Some(u) = &self.unit {
            let unit = &ds.units[f.unit as usize];
            let qualified = format!("{}:{}", ds.repos[unit.repo as usize].name, unit.name);
            if &unit.name != u && &qualified != u {
                return false;
            }
        }
        if let Some(p) = &self.path {
            let p = p.trim_start_matches("./");
            if !p.is_empty() && !f.path.starts_with(p) {
                return false;
            }
        }
        true
    }

    pub fn commit(&self, ds: &Dataset, c: &Commit) -> bool {
        match self.days {
            Some(d) => c.ts as f64 >= ds.now as f64 - d * csi_core::util::DAY,
            None => true,
        }
    }
}
