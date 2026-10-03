//! Angular-specific metadata extracted from TypeScript sources.

use crate::{AngularMeta, Summary};
use regex::Regex;
use std::sync::LazyLock;

static DECORATOR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"@(Component|Directive|Pipe|Injectable|NgModule)\s*\(").unwrap());
static INJECT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\binject\s*[<(]").unwrap());
static INPUT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"@Input\s*\(|\binput(\.required)?\s*[<(]|\bmodel(\.required)?\s*[<(]").unwrap());
static OUTPUT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"@Output\s*\(|\boutput\s*[<(]").unwrap());
static STANDALONE_FALSE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"standalone\s*:\s*false").unwrap());
static STANDALONE_TRUE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"standalone\s*:\s*true").unwrap());
static TEMPLATE_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"templateUrl\s*:\s*['"`]([^'"`]+)['"`]"#).unwrap());
static STYLE_URLS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"styleUrls?\s*:\s*(\[[^\]]*\]|['"`][^'"`]+['"`])"#).unwrap());
static QUOTED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"['"`]([^'"`]+)['"`]"#).unwrap());
static FUNCTIONAL_GUARD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r":\s*(CanActivateFn|CanMatchFn|CanDeactivateFn|ResolveFn|HttpInterceptorFn)\b").unwrap());

pub fn enrich(path: &str, text: &str, ctor_params: u32, summary: &mut Summary) {
    let name = path.rsplit('/').next().unwrap_or(path);
    let decorator = DECORATOR.captures(text).map(|c| c[1].to_string());
    let functional = FUNCTIONAL_GUARD.captures(text).map(|c| c[1].to_string());
    let kind = match (decorator.as_deref(), functional.as_deref()) {
        (Some("Component"), _) => "component",
        (Some("Directive"), _) => "directive",
        (Some("Pipe"), _) => "pipe",
        (Some("NgModule"), _) => "module",
        (Some("Injectable"), _) if name.contains(".guard.") => "guard",
        (Some("Injectable"), _) if name.contains(".interceptor.") => "interceptor",
        (Some("Injectable"), _) if name.contains(".resolver.") => "resolver",
        (Some("Injectable"), _) => "service",
        (None, Some("HttpInterceptorFn")) => "interceptor",
        (None, Some("ResolveFn")) => "resolver",
        (None, Some(_)) => "guard",
        _ => return,
    };
    let deps = ctor_params + INJECT.find_iter(text).count() as u32;
    let standalone = if STANDALONE_FALSE.is_match(text) {
        false
    } else {
        // Angular 19+ defaults to standalone; older code says so explicitly.
        STANDALONE_TRUE.is_match(text) || matches!(kind, "component" | "directive" | "pipe")
    };
    summary.angular = Some(AngularMeta {
        kind: kind.to_string(),
        deps,
        inputs: INPUT.find_iter(text).count() as u32,
        outputs: OUTPUT.find_iter(text).count() as u32,
        standalone,
    });
    summary.template_url = TEMPLATE_URL.captures(text).map(|c| c[1].to_string());
    if let Some(c) = STYLE_URLS.captures(text) {
        summary.style_urls = QUOTED.captures_iter(&c[1]).map(|q| q[1].to_string()).collect();
    }
}
