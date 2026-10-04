use super::*;
use std::collections::HashMap;

fn run(files: &[(&str, &str)], repo: &str) -> RepoArch {
    let paths: Vec<&str> = files.iter().map(|(p, _)| *p).collect();
    let content: HashMap<&str, &str> = files.iter().copied().collect();
    let mut read = |p: &str| content.get(p).map(|s| s.to_string());
    detect(&paths, &mut read, repo)
}

#[test]
fn angular_nx_and_federation() {
    let arch = run(
        &[
            ("nx.json", "{}"),
            ("package.json", r#"{"dependencies":{"@angular/core":"19"}}"#),
            ("apps/shell/project.json", r#"{"name":"shell","projectType":"application","tags":["scope:shell"]}"#),
            ("apps/shell/module-federation.config.ts", "export default { name: 'shell', remotes: ['checkout'] }"),
            ("apps/checkout/project.json", r#"{"name":"checkout","projectType":"application"}"#),
            ("apps/checkout/webpack.config.js", "module.exports = withModuleFederation({ exposes: {} })"),
            ("apps/checkout/src/app/cart/cart.routes.ts", ""),
            ("apps/checkout/src/app/cart/cart.component.ts", ""),
            ("libs/ui/project.json", r#"{"name":"ui","projectType":"library"}"#),
            ("libs/ui/src/button.ts", ""),
        ],
        "frontend",
    );
    let get = |n: &str| arch.units.iter().find(|u| u.name == n).unwrap_or_else(|| panic!("{n}: {:?}", arch.units));
    assert_eq!(get("shell").kind, "mfe");
    assert_eq!(get("shell").tags, vec!["scope:shell", "host"]);
    assert_eq!(get("checkout").kind, "mfe");
    assert!(get("checkout").tags.contains(&"remote".to_string()));
    assert_eq!(get("ui").kind, "lib");
    assert_eq!(get("cart").kind, "feature");
    assert!(arch.frameworks.contains(&"nx".to_string()));
    assert!(arch.frameworks.contains(&"module-federation".to_string()));

    let idx = UnitIndex::new(&arch, &[], "frontend", None);
    assert_eq!(idx.resolve("apps/checkout/src/app/cart/cart.component.ts").0, "cart");
    assert_eq!(idx.resolve("apps/checkout/src/main.ts").0, "checkout");
    assert_eq!(idx.resolve("libs/ui/src/button.ts").0, "ui");
    assert_eq!(idx.resolve("tools/scripts/x.js").0, "tools");
}

#[test]
fn angular_json_single_app_repo_is_mfe() {
    let arch = run(
        &[
            (
                "angular.json",
                r#"{"projects":{"orders-mfe":{"root":"","sourceRoot":"src","projectType":"application"}}}"#,
            ),
            ("federation.config.js", "module.exports = withNativeFederation({ exposes: {'./Routes': ''} })"),
            ("src/app/orders/orders.component.ts", ""),
        ],
        "orders",
    );
    let u = &arch.units[0];
    assert_eq!(u.root, "src");
    let mfe = arch.units.iter().find(|u| u.kind == "mfe").expect("mfe unit");
    assert!(mfe.tags.contains(&"remote".to_string()));
}

#[test]
fn laravel_layers() {
    let arch = run(
        &[
            ("composer.json", r#"{"require":{"laravel/framework":"^11"}}"#),
            ("artisan", ""),
            ("app/Models/Order.php", ""),
            ("app/Services/OrderService.php", ""),
            ("app/Http/Controllers/OrderController.php", ""),
            ("routes/api.php", ""),
        ],
        "orders-api",
    );
    let names: Vec<&str> = arch.units.iter().map(|u| u.name.as_str()).collect();
    assert!(names.contains(&"app/Models"), "{names:?}");
    assert!(names.contains(&"app/Http/Controllers"), "{names:?}");
    assert!(!names.contains(&"app/Http"), "{names:?}");
    let idx = UnitIndex::new(&arch, &[], "orders-api", None);
    assert_eq!(idx.resolve("app/Http/Controllers/OrderController.php").0, "app/Http/Controllers");
    assert_eq!(idx.resolve("routes/api.php").0, "routes");
}

#[test]
fn manual_units_win() {
    let arch = RepoArch::default();
    let manual =
        vec![UnitConfig { name: "payments".into(), repo: None, glob: "src/app/pay*/**".into(), kind: "mfe".into() }];
    let idx = UnitIndex::new(&arch, &manual, "r", None);
    assert_eq!(idx.resolve("src/app/payments/x.ts"), ("payments".into(), "mfe".into()));
}

#[test]
fn fallback_descends_containers() {
    assert_eq!(fallback_unit("src/app/orders/x.ts", None), "src/app/orders");
    assert_eq!(fallback_unit("tools/x.ts", None), "tools");
    assert_eq!(fallback_unit("README.md", None), "(root)");
    assert_eq!(fallback_unit("a/b/c/d.ts", Some(2)), "a/b");
}

#[test]
fn gitattributes_marks_generated_and_vendored() {
    let text = "# comment\n*.pbxproj binary\nsrc/api/client/** linguist-generated\n/legacy/ linguist-vendored=true\nschema.graphql.ts linguist-generated=true\ndocs/** linguist-documentation\nkeep.ts -linguist-generated\n";
    assert_eq!(crate::gitattributes_excludes("", text), vec!["src/api/client/**", "legacy/**", "**/schema.graphql.ts"]);
    assert_eq!(crate::gitattributes_excludes("web", "gen/*.ts linguist-generated\n"), vec!["web/gen/*.ts"]);
    let arch = run(&[(".gitattributes", "src/api/client/** linguist-generated\n")], "shop");
    let set = arch.exclude_set();
    assert!(set.is_match("src/api/client/orders.ts"));
    assert!(!set.is_match("src/app/orders.ts"));
}

#[test]
fn entity_keys() {
    let k = entity_key("src/app/cart/cart.component.ts");
    assert_eq!(k, "src/app/cart/cart.component");
    assert_eq!(entity_key("src/app/cart/cart.component.html"), k);
    assert_eq!(entity_key("src/app/cart/cart.component.scss"), k);
    assert_eq!(entity_key("src/app/cart/cart.component.spec.ts"), k);
    assert_eq!(entity_key("app/Models/Order.php"), "app/Models/Order");
    assert_eq!(entity_key("Makefile"), "Makefile");
    assert_eq!(entity_key(".gitignore"), ".gitignore");
}
