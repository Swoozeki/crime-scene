//! End-to-end scenario: a two-repo workspace scripted with known behavior.

use crate::coupling::{By, CouplingQuery, coupling};
use crate::diff::{DiffRequest, diff};
use crate::findings::findings;
use crate::hotspots::hotspots;
use crate::scan::{ScanOptions, scan};
use crate::scope::{Level, Scope};
use crate::social::ownership;
use crate::{Dataset, detail, overview, xray};
use csi_core::{Config, Db};
use csi_ingest::fixture::{FixtureRepo, Op, ts_with_branches};

const ALICE: &str = "Alice <alice@shop.io>";
const ALICE_OLD: &str = "alice <alice@old-laptop.local>";
const BOB: &str = "Bob <bob@shop.io>";
const CARL: &str = "Carl <carl@shop.io>";
const BOT: &str = "dependabot[bot] <49699333+dependabot[bot]@users.noreply.github.com>";

fn cart_ts(n: usize) -> String {
    let mut s = String::from(
        "import { Component } from '@angular/core';\n@Component({ selector: 'app-cart', templateUrl: './cart.component.html' })\nexport class CartComponent {\n  total(x: number) {\n    let r = 0;\n",
    );
    for i in 0..n {
        s.push_str(&format!("    if (x > {i} && x < {}) {{ r += {i}; }}\n", i + 100));
    }
    s.push_str("    return r;\n  }\n  name() { return 'cart'; }\n}\n");
    s
}

struct World {
    _tmp: tempfile::TempDir,
    shop: FixtureRepo,
    cfg: Config,
}

fn world() -> World {
    let tmp = tempfile::tempdir().unwrap();
    let shop = FixtureRepo::at(&tmp.path().join("shop"));
    let api = FixtureRepo::at(&tmp.path().join("api"));

    // legacy code by Bob, who left over a year ago
    shop.commit(
        BOB,
        600.0,
        "initial",
        &[
            Op::Write("src/app/legacy/legacy.ts", &ts_with_branches("legacy", 30, 0)),
            Op::Write("src/app/util/util.ts", "export const u = 1;\n"),
            Op::Write("src/app/cart/cart.component.ts", &cart_ts(5)),
            Op::Write("src/app/cart/cart.component.html", "<div>@if (a) { <p>x</p> }</div>\n"),
            Op::Write("src/app/pricing/pricing.service.ts", &ts_with_branches("price", 3, 0)),
        ],
    );
    for i in 0..6 {
        shop.commit(
            BOB,
            590.0 - i as f64 * 20.0,
            "tweak legacy",
            &[Op::Write("src/app/legacy/legacy.ts", &ts_with_branches("legacy", 30 + i, i))],
        );
    }
    // the cart: changes a lot, recently, mostly together with pricing (cross-unit coupling)
    for i in 0..14 {
        let author = if i % 4 == 0 {
            ALICE_OLD
        } else if i % 3 == 0 {
            CARL
        } else {
            ALICE
        };
        let msg = format!("SHOP-{} {} cart pricing", 100 + i, if i % 3 == 0 { "fix" } else { "feat" });
        let mut ops = vec![Op::Write("src/app/cart/cart.component.ts", Box::leak(cart_ts(6 + i).into_boxed_str()))];
        if i % 7 != 6 {
            ops.push(Op::Write(
                "src/app/pricing/pricing.service.ts",
                Box::leak(ts_with_branches("price", 3 + i, i).into_boxed_str()),
            ));
        }
        if i % 2 == 0 {
            ops.push(Op::Write(
                "src/app/cart/cart.component.html",
                Box::leak(format!("<div>@if (a) {{ <p>{i}</p> }}</div>\n").into_boxed_str()),
            ));
        }
        shop.commit(author, 200.0 - i as f64 * 10.0, &msg, &ops);
        // the backend half of the same ticket
        if i % 2 == 0 {
            api.commit(
                CARL,
                199.5 - i as f64 * 10.0,
                &format!("SHOP-{} api", 100 + i),
                &[Op::Write(
                    "app/Http/Controllers/CartController.php",
                    Box::leak(
                        format!("<?php\nclass CartController {{ public function show() {{ return {i}; }} }}\n")
                            .into_boxed_str(),
                    ),
                )],
            );
        }
    }
    // a mega commit and a bot commit must not create coupling or authorship
    let many: Vec<(String, String)> =
        (0..70).map(|i| (format!("src/gen/f{i}.ts"), format!("export const x{i} = {i};\n"))).collect();
    let ops: Vec<Op> = many
        .iter()
        .map(|(p, c)| Op::Write(p, c))
        .chain([Op::Write("src/app/util/util.ts", "export const u = 2;\n")])
        .collect();
    shop.commit(CARL, 15.0, "add generated constants", &ops);
    shop.commit(BOT, 10.0, "bump deps", &[Op::Write("src/app/util/util.ts", "export const u = 3;\n")]);

    std::fs::write(
        tmp.path().join(csi_core::config::CONFIG_FILE),
        "[workspace]\nname = \"fixture\"\n[[repo]]\nname = \"shop\"\npath = \"shop\"\n[[repo]]\nname = \"api\"\npath = \"api\"\n[analysis]\nsince = \"all\"\ncoupling_min_lift = 1.0\n",
    )
    .unwrap();
    let cfg = Config::load(&tmp.path().join(csi_core::config::CONFIG_FILE)).unwrap();
    World { _tmp: tmp, shop, cfg }
}

#[test]
fn end_to_end() {
    let w = world();
    let db = Db::open(&w.cfg.cache_path).unwrap();
    let noop = |_: &str| {};
    let report = scan(&w.cfg, &db, &ScanOptions::default(), &noop).unwrap();
    assert_eq!(report.repos.len(), 2);
    assert!(report.xray_commits > 0, "{report:?}");
    let ds = Dataset::load(&w.cfg, &db).unwrap();

    // identities: Alice's two identities merge; the bot is gone
    let names: Vec<&str> = ds.authors.iter().filter(|a| a.commits > 0).map(|a| a.name.as_str()).collect();
    assert_eq!(names.iter().filter(|n| n.eq_ignore_ascii_case("alice")).count(), 1, "{names:?}");
    assert!(!names.iter().any(|n| n.contains("dependabot")), "{names:?}");

    // generated mega commit files are mega, not coupled
    let mega = ds.commits.iter().find(|c| c.subject.starts_with("add generated")).unwrap();
    assert!(mega.mega && !mega.couples());

    // entity hotspots: the cart component (ts + html grouped via templateUrl) is #1
    let ent = hotspots(&ds, Level::Entity, &Scope::default());
    assert_eq!(
        ent[0].name,
        "src/app/cart/cart.component",
        "{:?}",
        ent.iter().take(3).map(|h| &h.name).collect::<Vec<_>>()
    );
    assert_eq!(ent[0].files, 2); // ts + html grouped via templateUrl
    let cart_entity = &ds.entities[ent[0].key as usize];
    assert_eq!(cart_entity.files.len(), 2);
    assert_eq!(cart_entity.kind, "component");

    // coupling: cart ↔ pricing across units; template coupling hidden at entity level
    let q = CouplingQuery { level: Level::Entity, ..Default::default() };
    let c = coupling(&ds, &q, &Scope::default());
    let pair = c
        .iter()
        .find(|c| {
            c.a_name.contains("cart.component") && c.b_name.contains("pricing")
                || c.b_name.contains("cart.component") && c.a_name.contains("pricing")
        })
        .expect("cart/pricing coupling");
    assert_eq!(pair.support, 12); // 13 co-changes minus iteration 0, where pricing content is unchanged
    assert!(pair.cross_unit);
    // file level shows the expected component/template pair only when asked
    let fq = CouplingQuery { level: Level::File, include_expected: true, min_lift: Some(1.0), ..Default::default() };
    assert!(coupling(&ds, &fq, &Scope::default()).iter().any(|c| c.same_entity));

    // ticket coupling crosses repos
    let tq = CouplingQuery { level: Level::Entity, by: By::Ticket, min_lift: Some(1.0), ..Default::default() };
    let tc = coupling(&ds, &tq, &Scope::default());
    assert!(
        tc.iter().any(|c| c.cross_repo && (c.a_name.contains("CartController") || c.b_name.contains("CartController"))),
        "{tc:#?}"
    );

    // knowledge loss on Bob's legacy code
    let own = ownership(&ds, Level::Entity, &Scope::default());
    let legacy = own.iter().find(|o| o.name.contains("legacy")).unwrap();
    assert!(legacy.knowledge_loss > 0.99);
    assert_eq!(legacy.main_dev.as_deref(), Some("Bob"));

    // x-ray finds the method that changes
    let file = ds.find_file("src/app/cart/cart.component.ts", Some("shop")).unwrap();
    let x = xray::xray(&ds, &db, file).unwrap();
    assert_eq!(x.functions[0].name, "CartComponent.total");
    assert_eq!(x.functions[0].revisions, 15);
    assert!(x.functions.iter().any(|f| f.name == "CartComponent.name" && f.revisions <= 1));

    // trends: cart keeps growing
    assert!(ds.files[file as usize].trend.len() >= 3);
    let fh = hotspots(&ds, Level::File, &Scope::default());
    let cart_row = fh.iter().find(|h| h.key == file).unwrap();
    assert_eq!(cart_row.trend.as_ref().unwrap().direction, "deteriorating");
    assert!(cart_row.defects >= 4);

    // findings: a hotspot and hidden coupling surface
    let f = findings(&ds, Some(&db), &Scope::default());
    let kinds: Vec<&str> = f.iter().map(|f| f.kind.as_str()).collect();
    assert!(kinds.contains(&"deteriorating_hotspot") || kinds.contains(&"hotspot"), "{kinds:?}");
    assert!(kinds.contains(&"hidden_coupling"), "{kinds:?}");
    assert!(f.windows(2).all(|w| w[0].severity >= w[1].severity));

    // overview + detail don't panic and see both repos
    let o = overview::overview(&ds);
    assert_eq!(o.repos.len(), 2);
    assert!(o.cross_repo_couplings >= 1);
    let tree = overview::hotspot_tree(&ds, &Scope::default());
    assert_eq!(tree.children.len(), 2);
    let d = detail::detail(&ds, &db, Level::Entity, ent[0].key).unwrap();
    assert!(d.coupling.iter().any(|p| p.name.contains("pricing")));
    assert!(!d.commits.is_empty());

    // diff: change the cart without pricing on a branch → pricing flagged as missed
    w.shop.git(&["checkout", "-q", "-b", "feature"]);
    w.shop.commit(ALICE, 1.0, "SHOP-999 tweak cart", &[Op::Write("src/app/cart/cart.component.ts", &cart_ts(25))]);
    let r = diff(
        &ds,
        &db,
        &DiffRequest {
            repo: Some("shop".into()),
            base: Some("main".into()),
            head: Some("feature".into()),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(r.files.len(), 1);
    assert!(r.missed.iter().any(|m| m.path.contains("pricing")), "{:#?}", r.missed);
    assert!(r.files[0].functions.iter().any(|f| f.name == "CartComponent.total" && f.cc_after > f.cc_before));
    assert!(r.risk >= 30, "risk {} {:?}", r.risk, r.reasons);
    assert!(r.reviewers.iter().all(|e| e.name != "Alice"));
    assert!(r.to_markdown().contains("pricing.service.ts"));

    // ticket mode spans repos
    let t = diff(&ds, &db, &DiffRequest { ticket: Some("SHOP-100".into()), ..Default::default() }).unwrap();
    assert_eq!(t.repos.len(), 2);

    // incremental rescan picks up nothing new on main
    let again = scan(&w.cfg, &db, &ScanOptions::default(), &noop).unwrap();
    assert!(again.repos.iter().all(|r| r.new_commits == 0));
}
