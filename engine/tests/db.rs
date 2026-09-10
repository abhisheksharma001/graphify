use graphify::db::{Call, Db, ToolCall};
use graphify::jobs::{DONE, RUNNING, WAITING};
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

// S-54: retention, one layer out. `purge_calls` reaches every table that names a call.
// `jobs` names no call and holds the call's words anyway — the request naming it, the
// verdicts with the sentence the model quoted out of the transcript, every line the brain
// printed — so a purge that emptied `calls` left all of it behind, in the one table with no
// list route to read it back.

/// The tables an org's retention deletes from.
const SWEPT: [&str; 5] = [
    "calls",
    "tool_calls",
    "pattern_labels",
    "pattern_matches",
    "jobs",
];

/// And the tables it does not, each with the reason. Checked against the schema below, so a
/// table added later is in neither list and this test names it. The lists are not the point:
/// the point is that putting a table in the database makes somebody write one sentence about
/// what happens to it when an org's `keep_days` runs out. `jobs` went eleven steps without
/// one.
const EXEMPT: [(&str, &str); 7] = [
    ("orgs", "the org itself, which retention is a setting on"),
    (
        "secrets",
        "keys, which go when their org does and not on a clock",
    ),
    (
        "assistants",
        "configuration the provider holds too, replaced by every sync",
    ),
    ("tools", "the same, keyed to an assistant and not to a call"),
    (
        "patterns",
        "the analyst's own questions, not the provider's data",
    ),
    ("dashboard", "which charts are on, one row per org"),
    (
        "spend",
        "a ledger the daily cap reads: one row per org per day, and losing one raises that \
         day's cap by exactly what it held",
    ),
];

fn tables(c: &Connection) -> Vec<String> {
    let mut stmt = c
        .prepare(
            "SELECT name FROM sqlite_master
              WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
        )
        .unwrap();
    let rows = stmt.query_map([], |r| r.get::<_, String>(0)).unwrap();
    rows.collect::<rusqlite::Result<Vec<_>>>().unwrap()
}

fn job(db: &Db, org: i64, status: &str, created_at: &str) -> i64 {
    db.create_job("label", status, org, r#"{"body":{}}"#, created_at)
        .unwrap()
}

#[test]
fn every_table_is_one_somebody_decided_retention_for() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("graphify.db");
    Db::open(&path).unwrap();
    let c = Connection::open(&path).unwrap();

    for table in tables(&c) {
        let swept = SWEPT.contains(&table.as_str());
        let exempt = EXEMPT.iter().any(|(name, _)| *name == table);
        assert!(
            swept != exempt,
            "`{table}` is in neither list, or in both: say whether an org's keep_days \
             deletes from it, and if not, why not"
        );
    }
    // And the other way, so a table that is dropped does not leave a sentence about it
    // standing here as if it were still true.
    let schema = tables(&c);
    for name in SWEPT.iter().chain(EXEMPT.iter().map(|(name, _)| name)) {
        assert!(
            schema.contains(&name.to_string()),
            "`{name}` is written down here and is not in the schema"
        );
    }
}

#[test]
fn purging_an_org_takes_the_jobs_that_read_its_calls() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("graphify.db");
    let db = Db::open(&path).unwrap();
    let c = Connection::open(&path).unwrap();
    let org = db.create_org("acme").unwrap();

    let old = job(&db, org, DONE, "2020-01-01T00:00:00.000Z");
    let new = job(&db, org, DONE, "2026-09-07T00:00:00.000Z");
    db.finish_job(
        old,
        DONE,
        Some(r#"{"evidence":"User: I want a refund on order 5512"}"#),
        0.0,
        org,
        "2020-01-01T00:00:00.000Z",
    )
    .unwrap();

    assert_eq!(db.purge_jobs(org, 30, RUNNING, WAITING).unwrap(), 1);
    assert!(db.job(old).unwrap().is_none(), "the old job is still there");
    assert!(db.job(new).unwrap().is_some(), "a job inside keep_days went");
    assert_eq!(
        count(
            &c,
            "SELECT count(*) FROM jobs WHERE output LIKE '%order 5512%'"
        ),
        0,
        "the sentence the model quoted out of a purged call's transcript is still stored"
    );
}

/// The two statuses that mean a subprocess is alive. A `running` row is not only a record,
/// it is one of `MAX_LIVE` slots — deleting it frees a slot that is not free — and a job old
/// enough to purge that is still running is the one shape where age says nothing at all.
#[test]
fn a_job_a_process_is_still_writing_to_is_never_old_enough() {
    let dir = tempdir().unwrap();
    let db = Db::open(dir.path().join("graphify.db")).unwrap();
    let org = db.create_org("acme").unwrap();

    for status in [RUNNING, WAITING] {
        job(&db, org, status, "2020-01-01T00:00:00.000Z");
    }
    assert_eq!(db.purge_jobs(org, 1, RUNNING, WAITING).unwrap(), 0);
    assert_eq!(db.live_jobs(RUNNING, WAITING).unwrap(), 2);
}

/// One org's retention is one org's. The column is what makes that a `WHERE` rather than a
/// parse of the request stored on the row.
#[test]
fn purging_one_orgs_jobs_leaves_anothers() {
    let dir = tempdir().unwrap();
    let db = Db::open(dir.path().join("graphify.db")).unwrap();
    let acme = db.create_org("acme").unwrap();
    let other = db.create_org("other").unwrap();

    let theirs = job(&db, other, DONE, "2020-01-01T00:00:00.000Z");
    job(&db, acme, DONE, "2020-01-01T00:00:00.000Z");

    assert_eq!(db.purge_jobs(acme, 30, RUNNING, WAITING).unwrap(), 1);
    assert!(
        db.job(theirs).unwrap().is_some(),
        "purging one org took another org's job"
    );
}

/// A database written before this step has its jobs' orgs inside a JSON blob and nowhere
/// else. The migration moves that fact to a column, so the rows already in the file are
/// covered by retention rather than only the ones written after it.
#[test]
fn a_database_from_before_the_column_comes_out_with_its_orgs() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("graphify.db");

    // The schema as it stood at migration 2, and a job row as `create_job` wrote one then.
    let old = Connection::open(&path).unwrap();
    old.execute_batch(include_str!("../migrations/0001_init.sql"))
        .unwrap();
    old.execute_batch(include_str!("../migrations/0002_global_secrets.sql"))
        .unwrap();
    old.execute_batch(
        r#"PRAGMA user_version = 2;
           INSERT INTO orgs (id, name) VALUES (7, 'acme');
           INSERT INTO jobs (kind, status, input, cost_usd, log, created_at)
             VALUES ('label', 'done', '{"org":7,"body":{}}', 0, '',
                     '2020-01-01T00:00:00.000Z');"#,
    )
    .unwrap();
    drop(old);

    let db = Db::open(&path).unwrap();
    assert_eq!(
        db.purge_jobs(7, 30, RUNNING, WAITING).unwrap(),
        1,
        "a job written before the column has no org and no retention"
    );
}

/// The backfill reads a column this code has only ever written JSON to. If it were ever
/// handed something else, `json_extract` would raise, the migration would fail, and the
/// failure would repeat on every open: a database that does not start, in exchange for a
/// column. The row is left without an org instead — which is the retention it had before
/// this step, and is a thing an operator can be told about a database they can still open.
#[test]
fn a_job_whose_request_is_not_json_does_not_cost_the_database_its_next_start() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("graphify.db");

    let old = Connection::open(&path).unwrap();
    old.execute_batch(include_str!("../migrations/0001_init.sql"))
        .unwrap();
    old.execute_batch(include_str!("../migrations/0002_global_secrets.sql"))
        .unwrap();
    old.execute_batch(
        r#"PRAGMA user_version = 2;
           INSERT INTO jobs (kind, status, input, cost_usd, log, created_at)
             VALUES ('label', 'done', 'not json at all', 0, '',
                     '2020-01-01T00:00:00.000Z');"#,
    )
    .unwrap();
    drop(old);

    Db::open(&path).expect("one unparseable row stopped the database from opening");
    // And again, because a migration that failed would be retried on the next open.
    let db = Db::open(&path).expect("the second open is the one a cron job does");
    assert_eq!(
        count(
            db.conn(),
            "SELECT count(*) FROM jobs WHERE org_id IS NULL"
        ),
        1,
        "the row was given an org it does not have"
    );
}
