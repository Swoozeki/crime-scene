use crate::fixture::{FixtureRepo, Op};
use crate::{CatFile, Git, analyze_tree, ingest_repo};
use csi_core::{Config, Db};

fn count(db: &Db, sql: &str) -> i64 {
    db.conn.query_row(sql, [], |r| r.get(0)).unwrap()
}

#[test]
fn ingests_renames_incrementally() {
    let repo = FixtureRepo::new();
    repo.commit("Jane <jane@x.com>", 30.0, "ABC-1 add cart", &[Op::Write("src/cart.ts", "export const a = 1;\n")]);
    repo.commit(
        "Jane <jane@x.com>",
        20.0,
        "ABC-2 move\n\nCo-authored-by: Raj <raj@x.com>",
        &[Op::Move("src/cart.ts", "src/shop/cart.ts")],
    );
    let cfg = Config::implicit(&repo.dir);
    let db = Db::open_in_memory().unwrap();
    let noop = |_: &str| {};
    let s = ingest_repo(&db, &cfg, &cfg.repos[0], false, &noop).unwrap();
    assert_eq!(s.new_commits, 2);
    assert!(s.full);
    // the rename keeps one file identity
    assert_eq!(count(&db, "SELECT count(*) FROM files"), 1);
    assert_eq!(count(&db, "SELECT count(DISTINCT file_id) FROM changes"), 1);
    assert_eq!(count(&db, "SELECT count(*) FROM commit_coauthors"), 1);
    let path: String = db.conn.query_row("SELECT path FROM files", [], |r| r.get(0)).unwrap();
    assert_eq!(path, "src/shop/cart.ts");

    // no new commits → nothing ingested
    let s = ingest_repo(&db, &cfg, &cfg.repos[0], false, &noop).unwrap();
    assert_eq!(s.new_commits, 0);
    assert_eq!(s.tree.len(), 1);

    repo.commit("Raj <raj@x.com>", 10.0, "edit", &[
        Op::Write("src/shop/cart.ts", "export function f(x: number) { if (x) { return 1; } return 2; }\n"),
        Op::Write("src/old.ts", "x\n"),
    ]);
    repo.commit("Raj <raj@x.com>", 5.0, "delete", &[Op::Delete("src/old.ts")]);
    let s = ingest_repo(&db, &cfg, &cfg.repos[0], false, &noop).unwrap();
    assert_eq!(s.new_commits, 2);
    assert!(!s.full);
    assert_eq!(count(&db, "SELECT count(*) FROM commits"), 4);
    assert_eq!(count(&db, "SELECT count(*) FROM files WHERE alive=1"), 1);

    let mut cat = CatFile::new(&Git::new(&repo.dir)).unwrap();
    let n = analyze_tree(&db, &cfg, &mut cat, &s.tree, &noop).unwrap();
    assert_eq!(n, 1);
    let cx: f64 = db.conn.query_row("SELECT complexity FROM blob_metrics", [], |r| r.get(0)).unwrap();
    assert_eq!(cx, 2.0);
    // cached on the second pass
    assert_eq!(analyze_tree(&db, &cfg, &mut cat, &s.tree, &noop).unwrap(), 0);
}

#[test]
fn rewritten_history_triggers_full_reingest() {
    let repo = FixtureRepo::new();
    repo.commit("A <a@x.com>", 10.0, "one", &[Op::Write("a.ts", "1\n")]);
    repo.commit("A <a@x.com>", 9.0, "two", &[Op::Write("a.ts", "2\n")]);
    let cfg = Config::implicit(&repo.dir);
    let db = Db::open_in_memory().unwrap();
    let noop = |_: &str| {};
    ingest_repo(&db, &cfg, &cfg.repos[0], false, &noop).unwrap();
    repo.git(&["reset", "-q", "--hard", "HEAD~1"]);
    repo.commit("A <a@x.com>", 8.0, "three", &[Op::Write("a.ts", "3\n")]);
    let s = ingest_repo(&db, &cfg, &cfg.repos[0], false, &noop).unwrap();
    assert!(s.full);
    assert_eq!(count(&db, "SELECT count(*) FROM commits"), 2);
}
