//! Language layer: per-function complexity via tree-sitter, with an indentation fallback.

mod angular;
pub mod health;
mod indent;
mod treewalk;

use serde::{Deserialize, Serialize};

pub use health::health;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Lang {
    TypeScript,
    Tsx,
    JavaScript,
    Template,
    Scss,
    Css,
    Php,
    Data,
    Doc,
    Other,
}

impl Lang {
    pub fn as_str(self) -> &'static str {
        match self {
            Lang::TypeScript => "typescript",
            Lang::Tsx => "tsx",
            Lang::JavaScript => "javascript",
            Lang::Template => "template",
            Lang::Scss => "scss",
            Lang::Css => "css",
            Lang::Php => "php",
            Lang::Data => "data",
            Lang::Doc => "doc",
            Lang::Other => "other",
        }
    }

    pub fn parse(s: &str) -> Lang {
        match s {
            "typescript" => Lang::TypeScript,
            "tsx" => Lang::Tsx,
            "javascript" => Lang::JavaScript,
            "template" => Lang::Template,
            "scss" => Lang::Scss,
            "css" => Lang::Css,
            "php" => Lang::Php,
            "data" => Lang::Data,
            "doc" => Lang::Doc,
            _ => Lang::Other,
        }
    }

    /// How much this language's complexity counts towards hotspot scores.
    pub fn weight(self) -> f64 {
        match self {
            Lang::TypeScript | Lang::Tsx | Lang::JavaScript | Lang::Php => 1.0,
            Lang::Template => 0.8,
            Lang::Other => 0.7,
            Lang::Scss | Lang::Css => 0.5,
            Lang::Data => 0.3,
            Lang::Doc => 0.15,
        }
    }

    /// True when complexity comes from a real parser rather than indentation.
    pub fn is_parsed(self) -> bool {
        !matches!(self, Lang::Data | Lang::Doc | Lang::Other)
    }
}

/// Map a path to a language. Binary detection happens on content in [`analyze`].
pub fn detect(path: &str) -> Lang {
    let name = path.rsplit('/').next().unwrap_or(path).to_ascii_lowercase();
    if name.ends_with(".blade.php") {
        return Lang::Other;
    }
    let ext = name.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
    match ext {
        "ts" | "mts" | "cts" => Lang::TypeScript,
        "tsx" => Lang::Tsx,
        "js" | "mjs" | "cjs" | "jsx" => Lang::JavaScript,
        "html" | "htm" => Lang::Template,
        "scss" => Lang::Scss,
        "css" => Lang::Css,
        "php" | "phtml" => Lang::Php,
        "json" | "jsonc" | "yaml" | "yml" | "xml" | "toml" | "ini" | "env" | "csv" | "properties"
        | "neon" | "lock" => Lang::Data,
        "md" | "mdx" | "txt" | "rst" | "adoc" => Lang::Doc,
        _ => Lang::Other,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Function {
    pub name: String,
    pub start: u32,
    pub end: u32,
    pub cc: u32,
    pub nesting: u32,
    pub loc: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct AngularMeta {
    /// component | directive | pipe | service | module | guard | interceptor | resolver
    pub kind: String,
    pub deps: u32,
    pub inputs: u32,
    pub outputs: u32,
    pub standalone: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Summary {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub angular: Option<AngularMeta>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template_url: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub style_urls: Vec<String>,
    pub functions: u32,
    pub methods: u32,
    pub max_nesting: u32,
    pub health: f64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub health_reasons: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FileMetrics {
    pub lang: Lang,
    /// non-blank lines
    pub loc: u32,
    pub complexity: f64,
    pub max_cc: u32,
    pub generated: bool,
    pub summary: Summary,
    pub functions: Vec<Function>,
}

/// Analyze one file's content. Returns `None` for binary content.
pub fn analyze(path: &str, content: &[u8], max_bytes: usize) -> Option<FileMetrics> {
    if content.iter().take(8000).any(|&b| b == 0) {
        return None;
    }
    let text = String::from_utf8_lossy(content);
    let lang = detect(path);
    let loc = text.lines().filter(|l| !l.trim().is_empty()).count() as u32;
    let generated = is_generated(&text, loc);

    let mut m = FileMetrics {
        lang,
        loc,
        complexity: 1.0,
        max_cc: 0,
        generated,
        summary: Summary::default(),
        functions: vec![],
    };

    let parsed = lang.is_parsed() && !generated && content.len() <= max_bytes;
    if parsed {
        if let Some(r) = treewalk::walk(lang, &text) {
            m.complexity = r.complexity.max(1) as f64;
            m.max_cc = r.functions.iter().map(|f| f.cc).max().unwrap_or(0);
            m.summary.functions = r.functions.len() as u32;
            m.summary.methods = r.methods;
            m.summary.max_nesting = r.functions.iter().map(|f| f.nesting).max().unwrap_or(0).max(r.top_nesting);
            m.functions = r.functions;
            if matches!(lang, Lang::TypeScript | Lang::Tsx | Lang::JavaScript) {
                angular::enrich(path, &text, r.ctor_params, &mut m.summary);
            }
        } else {
            m.complexity = indent::complexity(&text);
        }
    } else {
        m.complexity = indent::complexity(&text).max(1.0);
    }
    let (h, reasons) = health(&m);
    m.summary.health = h;
    m.summary.health_reasons = reasons;
    Some(m)
}

fn is_generated(text: &str, loc: u32) -> bool {
    let head: String = text.lines().take(10).collect::<Vec<_>>().join("\n").to_lowercase();
    if head.contains("@generated")
        || head.contains("do not edit")
        || head.contains("auto-generated")
        || head.contains("autogenerated")
        || head.contains("this file was generated")
        || head.contains("code generated by")
    {
        return true;
    }
    // minified / bundled: very long average lines
    loc > 0 && text.len() / loc as usize > 300
}

/// The innermost function containing `line` (1-based).
pub fn function_at(functions: &[Function], line: u32) -> Option<&Function> {
    functions
        .iter()
        .filter(|f| f.start <= line && line <= f.end)
        .min_by_key(|f| f.end - f.start)
}

/// Test, spec, story and e2e files.
pub fn is_test_path(path: &str) -> bool {
    let p = path.to_ascii_lowercase();
    let name = p.rsplit('/').next().unwrap_or(&p);
    name.contains(".spec.")
        || name.contains(".test.")
        || name.contains(".stories.")
        || name.contains(".cy.")
        || name.contains(".e2e.")
        || name.ends_with("_test.php")
        || name.ends_with("test.php")
        || p.starts_with("test/")
        || p.starts_with("tests/")
        || p.starts_with("e2e/")
        || p.contains("/test/")
        || p.contains("/tests/")
        || p.contains("/__tests__/")
        || p.contains("/e2e/")
        || p.contains("/cypress/")
}

#[cfg(test)]
mod tests;
