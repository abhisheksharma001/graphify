use graphify::db::{Call, Db, ToolCall};
use rusqlite::Connection;
use tempfile::tempdir;

fn call(id: &str, summary: &str, cost: Option<f64>) -> Call {
    Call {
        id: id.to_string(),
        org_id: 1,
        summary: Some(summary.to_string()),
        cost,
        ..Call::default()
    }
}

#[test]
fn upsert_call_twice_keeps_one_row_with_the_second_values() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("graphify.db");
    let db = Db::open(&path).unwrap();

    db.upsert_call(&call("c1", "first", Some(1.5))).unwrap();
    db.upsert_call(&call("c1", "second", None)).unwrap();

    let conn = Connection::open(&path).unwrap();
    let rows: i64 = conn
        .query_row("SELECT count(*) FROM calls", [], |r| r.get(0))
        .unwrap();
    assert_eq!(rows, 1);

    let (summary, cost): (String, Option<f64>) = conn
        .query_row("SELECT summary, cost FROM calls WHERE id = 'c1'", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .unwrap();
    assert_eq!(summary, "second");
    assert_eq!(cost, None, "a missing value must overwrite as NULL, not 0");
}

#[test]
fn open_on_an_existing_file_does_not_fail() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("nested").join("graphify.db");

    let db = Db::open(&path).unwrap();
    db.upsert_call(&call("c1", "kept", None)).unwrap();
    drop(db);

    let db = Db::open(&path).unwrap();
    let orgs = db.list_orgs().unwrap();
    assert!(orgs.is_empty());

    let conn = Connection::open(&path).unwrap();
    let rows: i64 = conn
        .query_row("SELECT count(*) FROM calls", [], |r| r.get(0))
        .unwrap();
    assert_eq!(rows, 1, "re-opening must migrate in place, not wipe");
}

#[test]
fn create_org_defaults_then_lists() {
    let dir = tempdir().unwrap();
    let db = Db::open(dir.path().join("graphify.db")).unwrap();

    let id = db.create_org("acme").unwrap();
    db.create_org("globex").unwrap();

    let orgs = db.list_orgs().unwrap();
    assert_eq!(orgs.len(), 2);
    assert_eq!(orgs[0].id, id);
    assert_eq!(orgs[0].name, "acme");
    assert_eq!(orgs[0].provider.as_deref(), Some("vapi"));
    assert_eq!(orgs[0].keep_days, Some(14));
    assert_eq!(orgs[0].max_calls, None);
    assert!(orgs[0].created_at.is_some());
}

#[test]
fn replace_tool_calls_swaps_rather_than_appends() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("graphify.db");
    let mut db = Db::open(&path).unwrap();
    db.upsert_call(&call("c1", "with tools", None)).unwrap();

    let first = vec![
        ToolCall {
            name: Some("lookup".into()),
            seconds_from_start: Some(3.0),
            failed: Some(false),
            ..ToolCall::default()
        },
        ToolCall {
            name: Some("transfer".into()),
            failed: Some(true),
            ..ToolCall::default()
        },
    ];
    db.replace_tool_calls("c1", &first).unwrap();
    db.replace_tool_calls(
        "c1",
        &[ToolCall {
            name: Some("lookup".into()),
            ..ToolCall::default()
        }],
    )
    .unwrap();

    let conn = Connection::open(&path).unwrap();
    let names: Vec<String> = conn
        .prepare("SELECT name FROM tool_calls WHERE call_id = 'c1'")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(names, vec!["lookup".to_string()]);
}

/// The container runs a server and a six o'clock sync against one file, so two processes
/// hold it open. Without this SQLite fails the second one on the spot with "database is
/// locked" — a morning that does not happen and says nothing about why.
#[test]
fn an_open_database_waits_for_a_lock_rather_than_failing_on_it() {
    let dir = tempdir().unwrap();
    let db = Db::open(dir.path().join("graphify.db")).unwrap();

    let ms: i64 = db
        .conn()
        .query_row("PRAGMA busy_timeout", [], |r| r.get(0))
        .unwrap();
    assert!(ms >= 1000, "busy timeout is {ms}ms, which is not a wait");
}
