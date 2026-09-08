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

// S-51: retention. A purge used to take the call and its tool rows and leave its
// `pattern_labels` row — with the sentence the model quoted out of the transcript on it.

/// The child tables, read out of the schema rather than written here, so a table added
/// later is covered without this file being edited.
fn call_children(c: &Connection) -> Vec<String> {
    let mut stmt = c
        .prepare(
            "SELECT m.name FROM sqlite_master m
               JOIN pragma_table_info(m.name) p
              WHERE m.type = 'table' AND m.name <> 'calls' AND p.name = 'call_id'
              ORDER BY m.name",
        )
        .unwrap();
    let rows = stmt.query_map([], |r| r.get::<_, String>(0)).unwrap();
    rows.collect::<rusqlite::Result<Vec<_>>>().unwrap()
}

/// Two calls, one older than any `keep_days` this suite uses, each with a row in every
/// child table. Returns the org.
fn org_with_children(db: &mut Db, c: &Connection) -> i64 {
    let org = db.create_org("acme").unwrap();
    for (id, created) in [
        ("old", "2020-01-01T00:00:00.000Z"),
        ("new", "2026-09-07T00:00:00.000Z"),
    ] {
        db.upsert_call(&Call {
            id: id.to_string(),
            org_id: org,
            created_at: Some(created.to_string()),
            ..Call::default()
        })
        .unwrap();
        db.replace_tool_calls(
            id,
            &[ToolCall {
                name: Some("lookup".into()),
                ..Default::default()
            }],
        )
        .unwrap();
        c.execute(
            "INSERT INTO pattern_labels (pattern_id, call_id, llm_match, rule_match, evidence)
             VALUES (1, ?1, 1, 1, 'user: I want to talk to a person please')",
            [id],
        )
        .unwrap();
        c.execute(
            "INSERT INTO pattern_matches (pattern_id, call_id, source) VALUES (1, ?1, 'llm')",
            [id],
        )
        .unwrap();
    }
    org
}

fn count(c: &Connection, sql: &str) -> i64 {
    c.query_row(sql, [], |r| r.get(0)).unwrap()
}

#[test]
fn purging_a_call_by_age_takes_every_row_keyed_to_it() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("graphify.db");
    let mut db = Db::open(&path).unwrap();
    let c = Connection::open(&path).unwrap();
    let org = org_with_children(&mut db, &c);

    assert_eq!(db.purge_calls(org, 30, None).unwrap(), 1);

    // The call is gone and so is everything that pointed at it. The other call is untouched.
    for table in ["calls", "tool_calls", "pattern_labels", "pattern_matches"] {
        let column = if table == "calls" { "id" } else { "call_id" };
        assert_eq!(
            count(&c, &format!("SELECT count(*) FROM {table} WHERE {column} = 'old'")),
            0,
            "{table} kept a row for the purged call"
        );
        assert_eq!(
            count(&c, &format!("SELECT count(*) FROM {table} WHERE {column} = 'new'")),
            1,
            "{table} lost a row for a call that was not purged"
        );
    }
}

#[test]
fn the_sentence_the_caller_said_does_not_outlive_the_call() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("graphify.db");
    let mut db = Db::open(&path).unwrap();
    let c = Connection::open(&path).unwrap();
    let org = org_with_children(&mut db, &c);

    db.purge_calls(org, 30, None).unwrap();

    // `evidence` is quoted out of the transcript. Retention deleted the transcript.
    assert_eq!(
        count(
            &c,
            "SELECT count(*) FROM pattern_labels
              WHERE evidence LIKE '%talk to a person%' AND call_id = 'old'"
        ),
        0
    );
}

#[test]
fn purging_a_call_by_max_calls_takes_its_children_too() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("graphify.db");
    let mut db = Db::open(&path).unwrap();
    let c = Connection::open(&path).unwrap();
    let org = org_with_children(&mut db, &c);

    // Nothing is old enough for the age sweep; the cap is what removes the row.
    assert_eq!(db.purge_calls(org, 36_500, Some(1)).unwrap(), 1);

    assert_eq!(count(&c, "SELECT count(*) FROM calls"), 1);
    assert_eq!(count(&c, "SELECT count(*) FROM pattern_labels"), 1);
    assert_eq!(count(&c, "SELECT count(*) FROM pattern_matches"), 1);
}

#[test]
fn every_table_keyed_to_a_call_is_swept_by_the_purge() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("graphify.db");
    let mut db = Db::open(&path).unwrap();
    let c = Connection::open(&path).unwrap();
    let org = org_with_children(&mut db, &c);

    // The schema is the list, not this file. A table added later with a `call_id` column
    // is covered the day it is created, and named here if the purge does not sweep it.
    let children = call_children(&c);
    assert!(
        children.len() >= 3,
        "expected the schema to hold at least three child tables, found {children:?}"
    );
    for table in &children {
        c.execute(
            &format!("INSERT INTO {table} (call_id) VALUES ('ghost')"),
            [],
        )
        .unwrap();
    }

    db.purge_calls(org, 30, None).unwrap();

    for table in &children {
        assert_eq!(
            count(
                &c,
                &format!(
                    "SELECT count(*) FROM {table}
                      WHERE call_id NOT IN (SELECT id FROM calls)"
                )
            ),
            0,
            "{table} holds rows for calls that are gone; add it to CALL_CHILDREN in db.rs"
        );
    }
}

#[test]
fn the_count_a_purge_returns_is_calls_not_rows() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("graphify.db");
    let mut db = Db::open(&path).unwrap();
    let c = Connection::open(&path).unwrap();
    let org = org_with_children(&mut db, &c);

    // One call goes, and three child rows go with it. The answer is one: the caller reports
    // how many calls retention removed, and `sync` logs that number.
    assert_eq!(db.purge_calls(org, 30, None).unwrap(), 1);
}
