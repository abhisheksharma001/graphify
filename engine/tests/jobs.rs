//! The engine running the brain: what reaches the child, what comes back, and the one
//! thing that has to happen before a single call is read.
//!
//! Every test here uses a fake brain — a shell script that records what it was handed and
//! prints what the test wants it to print. Nothing spawns Python, nothing needs a key that
//! works, and no test can reach a model even by accident.

use graphify::auth::Auth;
use graphify::db::Db;
use graphify::jobs;
use graphify::secrets::Secrets;
use graphify::server::{router, App};
use serde_json::{json, Value};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;
use tempfile::TempDir;
use tokio::sync::{Mutex, MutexGuard};

/// `Secrets::get` reads `ANTHROPIC_API_KEY` before it reads the store, and a spawned child
/// inherits whatever this process has. Anything asserting about what a key looks like on
/// the far side takes this first, and clears both.
async fn env_lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(())).lock().await
}

fn clear_env() {
    for var in ["ANTHROPIC_API_KEY", "OPENAI_API_KEY", "GRAPHIFY_SECRET"] {
        std::env::remove_var(var);
    }
}

// --- the fake brain -------------------------------------------------------------------

/// The preamble every fake brain starts with: write down the arguments and what the two
/// model-key variables looked like, so a test can ask afterwards.
const RECORD: &str = r#"#!/bin/sh
here=$(dirname "$0")
{
  echo "argv: $*"
  echo "anthropic: ${ANTHROPIC_API_KEY:-unset}"
  echo "openai: ${OPENAI_API_KEY:-unset}"
} >> "$here/seen.txt"
"#;

/// A brain that reads the request, prints a price, waits for the go, then reports.
const LABELS: &str = r#"
read -r request
printf '%s\n' "$request" > "$here/request.json"
echo "ESTIMATE 0.0428"
read -r go
if [ "$go" != "GO" ]; then
  echo "the engine said $go instead of GO" >&2
  exit 1
fi
echo "PROGRESS 1/3" >&2
echo "PROGRESS 2/3" >&2
echo "PROGRESS 3/3" >&2
echo '{"labels":[{"call_id":"c1","match":true}],"usd":0.0123,"stopped":null}'
"#;

/// A brain that quotes something which is not a price and then waits for a go it must
/// never be given. The `read` after the quote is what makes the test mean something: if
/// the engine parks anyway, this child is sitting there ready to be told to read.
fn bad_quote(quote: &str) -> String {
    format!(
        r#"
read -r request
echo "ESTIMATE {quote}"
read -r go
echo '{{"labels":[{{"call_id":"c1","match":true}}],"usd":0.0123,"stopped":null}}'
"#
    )
}

/// A brain that answers straight off, the way `plan` and `clarify` do.
const ANSWERS: &str = r#"
request=$(cat)
printf '%s\n' "$request" > "$here/request.json"
echo '{"confidence":0.91,"expressible":true}'
"#;

/// The same, but priced: since S-33 `plan` and `clarify` quote themselves before the call
/// and report what it cost after it, without ever parking on a go.
const PRICED_ANSWER: &str = r#"
request=$(cat)
printf '%s\n' "$request" > "$here/request.json"
echo "ESTIMATE 0.0438"
echo '{"confidence":0.91,"expressible":true,"usd":0.0031}'
"#;

/// Write a fake brain into `dir` and return its path.
fn fake(dir: &Path, body: &str) -> String {
    let path = dir.join("brain.sh");
    std::fs::write(&path, format!("{RECORD}{body}")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path.to_string_lossy().into_owned()
}

/// Everything the fake brain wrote down about how it was called.
fn seen(dir: &Path) -> String {
    std::fs::read_to_string(dir.join("seen.txt")).unwrap_or_default()
}

/// The request line the fake brain was handed, parsed.
fn request(dir: &Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(dir.join("request.json")).unwrap()).unwrap()
}

// --- the server -----------------------------------------------------------------------

struct Server {
    dir: TempDir,
    base: String,
}

impl Server {
    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }

    fn path(&self) -> PathBuf {
        self.dir.path().to_path_buf()
    }

    /// A second connection on the same file, for asking what actually landed.
    fn db(&self) -> Db {
        Db::open(self.dir.path().join("graphify.db")).unwrap()
    }
}

/// A live server over a fresh database holding one org, pointed at `brain`.
async fn boot(dir: TempDir, brain: &str, wait: Duration, fill: impl FnOnce(&Db, i64)) -> Server {
    let db = Db::open(dir.path().join("graphify.db")).unwrap();
    let org = db.create_org("acme").unwrap();
    fill(&db, org);

    let store = Secrets::open(dir.path().join(".secret")).unwrap();
    let app = App::new(db, store, Auth::new(None))
        .with_brain(brain)
        .with_go_wait(wait);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router(app)).await.unwrap() });
    Server {
        dir,
        base: format!("http://{addr}"),
    }
}

/// A server whose brain is the script `body`. The script is written into the server's own
/// directory, so `seen.txt` and `request.json` land where the test can read them and go
/// when the test does.
async fn served(body: &str) -> Server {
    served_for(body, GO_WAIT).await
}

/// The same, with the go-wait wound down to something a test can sit through.
async fn served_for(body: &str, wait: Duration) -> Server {
    let dir = tempfile::tempdir().unwrap();
    let brain = fake(dir.path(), body);
    boot(dir, &brain, wait, |_, _| {}).await
}

/// A server pointed at a binary of the test's choosing, over a database it fills itself.
async fn serve_with(brain: &str, fill: impl FnOnce(&Db, i64)) -> Server {
    boot(tempfile::tempdir().unwrap(), brain, GO_WAIT, fill).await
}

/// Long enough that no test reaches it by being slow.
const GO_WAIT: Duration = Duration::from_secs(600);

async fn get(url: &str) -> (u16, Value) {
    let res = reqwest::get(url).await.unwrap();
    let status = res.status().as_u16();
    (status, res.json().await.unwrap_or(Value::Null))
}

async fn send(method: reqwest::Method, url: &str, body: Value) -> (u16, Value) {
    let res = reqwest::Client::new()
        .request(method, url)
        .json(&body)
        .send()
        .await
        .unwrap();
    let status = res.status().as_u16();
    (status, res.json().await.unwrap_or(Value::Null))
}

async fn post(url: &str, body: Value) -> (u16, Value) {
    send(reqwest::Method::POST, url, body).await
}

async fn put(url: &str, body: Value) -> (u16, Value) {
    send(reqwest::Method::PUT, url, body).await
}

/// Poll one job until it reaches `want`, or give up and say where it got to instead.
///
/// Threads and subprocesses take a moment, and this file starts a shell for nearly every
/// test at once; a fixed sleep long enough to be safe on a loaded machine would be long
/// enough to make the suite tedious on an idle one. The budget is generous because the
/// whole file still finishes in seconds — nothing waits for it unless something is wrong.
async fn until(server: &Server, id: i64, want: &str) -> Value {
    let url = server.url(&format!("/api/jobs/{id}"));
    let mut last = Value::Null;
    let started = std::time::Instant::now();
    for _ in 0..1500 {
        let (status, body) = get(&url).await;
        assert_eq!(status, 200, "{body}");
        if body["status"] == want {
            return body;
        }
        last = body;
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("job {id} never reached {want} in {:?}; it is at {last}", started.elapsed());
}

/// Start a labelling job and wait until it is parked on its go.
async fn parked(server: &Server) -> i64 {
    let (status, body) = post(
        &server.url("/api/patterns/label?org=1"),
        json!({"criterion": "asked for a human", "call_ids": ["c1"], "model": "sonnet", "max_usd": 1.0}),
    )
    .await;
    assert_eq!(status, 202, "{body}");
    let id = body["id"].as_i64().unwrap();
    until(server, id, jobs::WAITING).await;
    id
}

// --- the acceptance -------------------------------------------------------------------

#[tokio::test]
async fn a_label_job_that_is_never_told_to_go_stays_waiting_and_spends_nothing() {
    let server = served(LABELS).await;
    let id = parked(&server).await;

    // Long enough that a brain which was going to read anything would have.
    tokio::time::sleep(Duration::from_millis(300)).await;
    let (_, body) = get(&server.url(&format!("/api/jobs/{id}"))).await;
    assert_eq!(body["status"], jobs::WAITING);
    assert_eq!(body["cost_usd"], 0.0);
    assert!(body["output"].is_null(), "nothing came back: {body}");

    let db = server.db();
    assert_eq!(db.spend_on(&graphify::now()[..10], 1).unwrap(), 0.0);
    let total: f64 = db
        .conn()
        .query_row("SELECT COALESCE(SUM(usd), 0) FROM spend", [], |r| r.get(0))
        .unwrap();
    assert_eq!(total, 0.0, "a job nobody approved booked spend");
}

#[tokio::test]
async fn the_go_is_what_starts_the_reading() {
    let server = served(LABELS).await;
    let id = parked(&server).await;

    let (status, body) = post(&server.url(&format!("/api/jobs/{id}/go")), json!({})).await;
    assert_eq!(status, 200, "{body}");

    let done = until(&server, id, jobs::DONE).await;
    assert_eq!(done["output"]["labels"][0]["call_id"], "c1");
    assert_eq!(done["cost_usd"], 0.0123);
    assert!(done["finished_at"].is_string());

    // What the job cost is booked against the org that asked for it, on the day it ran.
    let spent = server.db().spend_on(&graphify::now()[..10], 1).unwrap();
    assert!((spent - 0.0123).abs() < 1e-9, "spend was {spent}");
}

// --- what reaches the child -----------------------------------------------------------

#[tokio::test]
async fn the_model_keys_travel_in_the_environment_and_never_in_the_argv() {
    let _guard = env_lock().await;
    clear_env();

    let server = served(ANSWERS).await;
    {
        // Written through the same store the engine reads them back through, so what the
        // child sees is what a settings screen would have put there.
        let store = Secrets::open(server.path().join(".secret")).unwrap();
        let db = server.db();
        store.set(&db, None, "anthropic", "sk-ant-fake-0001").unwrap();
        store.set(&db, None, "openai", "sk-openai-fake-0002").unwrap();
    }

    let (status, body) = post(&server.url("/api/patterns/plan?org=1"), json!({"criterion": "x"})).await;
    assert_eq!(status, 202, "{body}");
    until(&server, body["id"].as_i64().unwrap(), jobs::DONE).await;

    let seen = seen(&server.path());
    assert!(seen.contains("anthropic: sk-ant-fake-0001"), "{seen}");
    assert!(seen.contains("openai: sk-openai-fake-0002"), "{seen}");
    let argv = seen.lines().find(|l| l.starts_with("argv:")).unwrap();
    assert!(!argv.contains("sk-ant"), "a key reached the command line: {argv}");
    assert!(!argv.contains("sk-openai"), "a key reached the command line: {argv}");
}

#[tokio::test]
async fn the_engine_never_says_yes_on_the_analysts_behalf() {
    let server = served(LABELS).await;
    parked(&server).await;
    let seen = seen(&server.path());
    let argv = seen.lines().find(|l| l.starts_with("argv:")).unwrap();
    // `--yes` is the brain's own escape hatch for a person at a terminal. An engine that
    // passed it would be the thing approving the spend, which is the whole point of the go.
    assert!(!argv.contains("--yes"), "{argv}");
    assert!(argv.starts_with("argv: label --db "), "{argv}");
}

#[tokio::test]
async fn the_brain_is_pointed_at_the_database_the_engine_is_holding() {
    let server = served(ANSWERS).await;
    let (_, body) = post(&server.url("/api/patterns/plan?org=1"), json!({"criterion": "x"})).await;
    until(&server, body["id"].as_i64().unwrap(), jobs::DONE).await;

    let seen = seen(&server.path());
    let argv = seen.lines().find(|l| l.starts_with("argv:")).unwrap();
    let db = server.path().join("graphify.db");
    assert!(argv.contains(db.to_str().unwrap()), "{argv}");
}

#[tokio::test]
async fn the_request_reaches_the_brain_exactly_as_it_arrived() {
    let server = served(ANSWERS).await;
    let sent = json!({"criterion": "asked for a human", "system_prompt": "you are a bot"});
    let (_, body) = post(&server.url("/api/patterns/plan?org=1"), sent.clone()).await;
    until(&server, body["id"].as_i64().unwrap(), jobs::DONE).await;

    // Byte for byte the object the caller sent — no `org`, no wrapper. The brain names the
    // keys it did not expect, and it can only do that if the engine has not added any.
    assert_eq!(request(&server.path()), sent);
}

// --- what comes back ------------------------------------------------------------------

#[tokio::test]
async fn the_price_is_readable_before_anything_is_spent() {
    let server = served(LABELS).await;
    let id = parked(&server).await;
    let (_, body) = get(&server.url(&format!("/api/jobs/{id}"))).await;
    assert_eq!(body["estimate_usd"], 0.0428);
    assert_eq!(body["cost_usd"], 0.0);
}

#[tokio::test]
async fn progress_lines_become_a_progress_reading() {
    let server = served(LABELS).await;
    let id = parked(&server).await;
    post(&server.url(&format!("/api/jobs/{id}/go")), json!({})).await;
    let done = until(&server, id, jobs::DONE).await;
    assert_eq!(done["progress"], json!({"done": 3, "of": 3}));
    assert!(done["log"].as_str().unwrap().contains("PROGRESS 1/3"));
}

#[tokio::test]
async fn a_job_that_has_reported_nothing_has_no_progress_rather_than_zero() {
    let server = served(ANSWERS).await;
    let (_, body) = post(&server.url("/api/patterns/plan?org=1"), json!({"criterion": "x"})).await;
    let done = until(&server, body["id"].as_i64().unwrap(), jobs::DONE).await;
    // Not `0/0`: a bar drawn at nought per cent says the job has got nowhere, and this one
    // simply never said.
    assert!(done["progress"].is_null(), "{done}");
    assert!(done["estimate_usd"].is_null(), "{done}");
}

#[tokio::test]
async fn a_brain_that_exits_one_is_a_failed_job_with_its_complaint_kept() {
    let server = served(
        r#"
read -r request
echo "label: has no field criterio" >&2
exit 1
"#,
    )
    .await;
    let (_, body) = post(
        &server.url("/api/patterns/label?org=1"),
        json!({"criterio": "typed wrong"}),
    )
    .await;
    let failed = until(&server, body["id"].as_i64().unwrap(), jobs::FAILED).await;
    assert!(
        failed["log"].as_str().unwrap().contains("has no field criterio"),
        "{failed}"
    );
    assert_eq!(failed["cost_usd"], 0.0);
    let total: f64 = server
        .db()
        .conn()
        .query_row("SELECT COALESCE(SUM(usd), 0) FROM spend", [], |r| r.get(0))
        .unwrap();
    assert_eq!(total, 0.0);
}

#[tokio::test]
async fn a_last_line_that_is_not_json_is_a_failed_job_and_not_a_half_read_one() {
    let server = served(
        r#"
read -r request
echo "Traceback (most recent call last)"
"#,
    )
    .await;
    let (_, body) = post(&server.url("/api/patterns/plan?org=1"), json!({"criterion": "x"})).await;
    let failed = until(&server, body["id"].as_i64().unwrap(), jobs::FAILED).await;
    assert!(
        failed["log"].as_str().unwrap().contains("was not JSON"),
        "{failed}"
    );
}

#[tokio::test]
async fn a_brain_that_is_not_installed_is_a_failed_job_and_not_a_crash() {
    let server = serve_with("/nonexistent/graphify-brain", |_, _| {}).await;
    let (status, body) = post(&server.url("/api/patterns/plan?org=1"), json!({"criterion": "x"})).await;
    assert_eq!(status, 202, "{body}");
    let failed = until(&server, body["id"].as_i64().unwrap(), jobs::FAILED).await;
    assert!(
        failed["log"]
            .as_str()
            .unwrap()
            .contains("could not start /nonexistent/graphify-brain"),
        "{failed}"
    );
}

// --- the go ---------------------------------------------------------------------------

#[tokio::test]
async fn a_go_for_a_job_that_is_not_waiting_is_refused() {
    let server = served(LABELS).await;
    let id = parked(&server).await;

    let (first, _) = post(&server.url(&format!("/api/jobs/{id}/go")), json!({})).await;
    assert_eq!(first, 200);
    // The second click finds nothing parked. One go per job, whatever the browser does.
    let (second, body) = post(&server.url(&format!("/api/jobs/{id}/go")), json!({})).await;
    assert_eq!(second, 409, "{body}");

    let (missing, _) = post(&server.url("/api/jobs/9999/go"), json!({})).await;
    assert_eq!(missing, 409);
}

#[tokio::test]
async fn a_function_that_does_not_wait_never_parks() {
    let server = served(ANSWERS).await;
    let (_, body) = post(&server.url("/api/patterns/clarify?org=1"), json!({"criterion": "x"})).await;
    let id = body["id"].as_i64().unwrap();
    let done = until(&server, id, jobs::DONE).await;
    assert_eq!(done["kind"], "clarify");
    assert_eq!(done["output"]["confidence"], 0.91);
    // It reached `done` without anyone approving anything, which is what "does not wait"
    // means. A go arriving now has nothing to wake.
    let (status, _) = post(&server.url(&format!("/api/jobs/{id}/go")), json!({})).await;
    assert_eq!(status, 409);
}

#[tokio::test]
async fn more_parked_jobs_than_the_cap_are_refused_rather_than_spawned() {
    let server = served(LABELS).await;
    for _ in 0..jobs::MAX_LIVE {
        parked(&server).await;
    }
    let (status, body) = post(
        &server.url("/api/patterns/label?org=1"),
        json!({"criterion": "one more"}),
    )
    .await;
    assert_eq!(status, 429, "{body}");
    assert!(body["error"].as_str().unwrap().contains("waiting for a go"));
}

#[tokio::test]
async fn a_job_for_an_org_that_does_not_exist_is_never_started() {
    let server = served(ANSWERS).await;
    let (status, body) = post(&server.url("/api/patterns/plan?org=77"), json!({"criterion": "x"})).await;
    assert_eq!(status, 404, "{body}");
    let (missing, _) = get(&server.url("/api/jobs/1")).await;
    assert_eq!(missing, 404, "a refused request still made a row");
}

#[tokio::test]
async fn a_brain_request_that_is_not_an_object_is_refused() {
    let server = served(ANSWERS).await;
    let (status, body) = post(&server.url("/api/patterns/plan?org=1"), json!(["criterion"])).await;
    assert_eq!(status, 400, "{body}");
}

// --- patterns -------------------------------------------------------------------------

/// One pattern with a rule, and three calls, one of which the rule matches. The one it
/// matches is the oldest, so a `last` that cuts the selection cuts it out.
fn a_pattern(db: &Db, org: i64) {
    db.conn()
        .execute(
            "INSERT INTO patterns (id, org_id, name, criterion, plan, rule, chart, model, mode,
                                   daily_cap_usd, sample_size, agreement, created_at)
             VALUES (1, ?1, 'asked for a human', 'caller asked for a human',
                     '{\"rows\":[]}', '{\"any_phrases\":[\"get me a human\"]}',
                     '{\"kind\":\"Line\",\"title\":\"Handoffs\"}', 'sonnet', 'free',
                     1.0, 250, 0.984, '2026-09-04T00:00:00.000Z')",
            rusqlite::params![org],
        )
        .unwrap();
    for (id, at, transcript) in [
        ("c1", "2026-09-01T09:00:00.000Z", "User: get me a human"),
        ("c2", "2026-09-02T09:00:00.000Z", "User: what are your hours"),
        ("c3", "2026-09-03T09:00:00.000Z", "User: thanks"),
    ] {
        db.conn()
            .execute(
                "INSERT INTO calls (id, org_id, created_at, transcript) VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![id, org, at, transcript],
            )
            .unwrap();
    }
}

#[tokio::test]
async fn the_patterns_list_parses_its_json_columns() {
    let server = serve_with("/nonexistent/brain", a_pattern).await;
    let (status, body) = get(&server.url("/api/patterns?org=1")).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body[0]["rule"]["any_phrases"][0], "get me a human");
    assert_eq!(body[0]["chart"]["title"], "Handoffs");
    assert_eq!(body[0]["agreement"], 0.984);
    assert_eq!(body[0]["mode"], "free");
}

#[tokio::test]
async fn a_rule_the_engine_refuses_is_never_stored() {
    let server = serve_with("/nonexistent/brain", a_pattern).await;
    let (status, body) = put(
        &server.url("/api/patterns/1"),
        json!({"rule": {"min_turns": 3}, "mode": "free", "daily_cap_usd": 1.0}),
    )
    .await;
    assert_eq!(status, 400, "{body}");
    assert!(body["error"].as_str().unwrap().contains("min_turns"), "{body}");

    // Still the rule it had. A refusal that half-wrote would be worse than no edit at all.
    let (_, list) = get(&server.url("/api/patterns?org=1")).await;
    assert_eq!(list[0]["rule"]["any_phrases"][0], "get me a human");
}

#[tokio::test]
async fn a_mode_or_a_cap_that_is_not_one_is_refused() {
    let server = serve_with("/nonexistent/brain", a_pattern).await;
    let bad_mode = json!({"rule": null, "mode": "sometimes", "daily_cap_usd": 1.0});
    assert_eq!(put(&server.url("/api/patterns/1"), bad_mode).await.0, 400);
    let no_cap = json!({"rule": null, "mode": "hybrid", "daily_cap_usd": 0.0});
    assert_eq!(put(&server.url("/api/patterns/1"), no_cap).await.0, 400);
    let missing = json!({"mode": "hybrid"});
    assert_eq!(put(&server.url("/api/patterns/1"), missing).await.0, 422);
}

#[tokio::test]
async fn a_rule_the_engine_accepts_comes_back_as_the_row_that_was_stored() {
    let server = serve_with("/nonexistent/brain", a_pattern).await;
    let (status, body) = put(
        &server.url("/api/patterns/1"),
        json!({
            "rule": {"any_phrases": ["speak to someone"], "speaker": "user"},
            "mode": "hybrid",
            "daily_cap_usd": 2.5
        }),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["rule"]["any_phrases"][0], "speak to someone");
    assert_eq!(body["mode"], "hybrid");
    assert_eq!(body["daily_cap_usd"], 2.5);
}

#[tokio::test]
async fn applying_one_pattern_counts_the_calls_its_rule_matches() {
    let server = serve_with("/nonexistent/brain", a_pattern).await;
    let (status, body) = post(&server.url("/api/patterns/1/apply"), json!({})).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body, json!({"matched": 1, "of": 3}));

    let matched: Vec<String> = {
        let db = server.db();
        let conn = db.conn();
        let mut stmt = conn
            .prepare("SELECT call_id FROM pattern_matches WHERE pattern_id = 1 AND source = 'rule'")
            .unwrap();
        let rows = stmt.query_map([], |r| r.get(0)).unwrap();
        rows.collect::<rusqlite::Result<Vec<_>>>().unwrap()
    };
    assert_eq!(matched, vec!["c1".to_string()]);
}

#[tokio::test]
async fn applying_a_pattern_with_no_rule_says_so_rather_than_matching_everything() {
    let server = serve_with("/nonexistent/brain", |db, org| {
        db.conn()
            .execute(
                "INSERT INTO patterns (id, org_id, name, mode) VALUES (1, ?1, 'half made', 'free')",
                rusqlite::params![org],
            )
            .unwrap();
    })
    .await;
    let (status, body) = post(&server.url("/api/patterns/1/apply"), json!({})).await;
    assert_eq!(status, 400, "{body}");
    assert!(body["error"].as_str().unwrap().contains("no rule"), "{body}");
}

#[tokio::test]
async fn a_pattern_that_does_not_exist_is_a_404_on_every_route_that_names_one() {
    let server = serve_with("/nonexistent/brain", |_, _| {}).await;
    assert_eq!(post(&server.url("/api/patterns/9/apply"), json!({})).await.0, 404);
    let edit = json!({"rule": null, "mode": "free", "daily_cap_usd": 1.0});
    assert_eq!(put(&server.url("/api/patterns/9"), edit).await.0, 404);
}

// --- what must not come back out ------------------------------------------------------

#[tokio::test]
async fn a_key_the_engine_handed_the_child_never_comes_back_in_the_log() {
    let _guard = env_lock().await;
    clear_env();

    // A brain that does the worst plausible thing with its key: prints it, in a traceback
    // and again in its result. Unlikely from `graphify-brain` itself, and entirely
    // ordinary from an HTTP library repeating a request it could not send.
    let server = served(
        r#"
read -r request
echo "httpx.ConnectError while sending Authorization: Bearer $ANTHROPIC_API_KEY" >&2
printf '{"confidence":0.5,"note":"key was %s"}\n' "$ANTHROPIC_API_KEY"
"#,
    )
    .await;
    {
        let store = Secrets::open(server.path().join(".secret")).unwrap();
        let db = server.db();
        store.set(&db, None, "anthropic", "sk-ant-fake-secret-value").unwrap();
    }

    let (_, body) = post(&server.url("/api/patterns/plan?org=1"), json!({"criterion": "x"})).await;
    let done = until(&server, body["id"].as_i64().unwrap(), jobs::DONE).await;

    let whole = done.to_string();
    assert!(!whole.contains("sk-ant-fake-secret-value"), "{whole}");
    assert!(done["log"].as_str().unwrap().contains("Bearer ***"), "{done}");
    assert_eq!(done["output"]["note"], "key was ***");

    // Not only out of the response: out of the row it was read from. A key written to the
    // database in clear is a key in clear whether or not anyone asks for it today.
    let stored: String = server
        .db()
        .conn()
        .query_row("SELECT log || COALESCE(output, '') FROM jobs WHERE id = 1", [], |r| r.get(0))
        .unwrap();
    assert!(!stored.contains("sk-ant-fake-secret-value"), "{stored}");
}

#[tokio::test]
async fn a_job_nobody_approves_is_killed_unspent_rather_than_held_for_ever() {
    // The engine waits half an hour for a go. What is under test is the end of that wait,
    // not its length, so this server's is a fifth of a second.
    let server = served_for(LABELS, Duration::from_millis(200)).await;
    let (_, body) = post(
        &server.url("/api/patterns/label?org=1"),
        json!({"criterion": "asked for a human"}),
    )
    .await;
    let id = body["id"].as_i64().unwrap();

    let gone = until(&server, id, jobs::EXPIRED).await;
    assert_eq!(gone["cost_usd"], 0.0);
    assert!(gone["output"].is_null(), "{gone}");
    assert!(
        gone["log"].as_str().unwrap().contains("nobody approved the price"),
        "{gone}"
    );
    // Expired, not failed: walking away from a quote is not a thing going wrong. And a
    // go arriving afterwards has nothing to wake.
    assert_eq!(post(&server.url(&format!("/api/jobs/{id}/go")), json!({})).await.0, 409);

    let total: f64 = server
        .db()
        .conn()
        .query_row("SELECT COALESCE(SUM(usd), 0) FROM spend", [], |r| r.get(0))
        .unwrap();
    assert_eq!(total, 0.0);
}

#[tokio::test]
async fn a_job_left_running_by_a_dead_engine_does_not_block_the_next_one() {
    let dir = tempfile::tempdir().unwrap();
    let brain = fake(dir.path(), ANSWERS);
    // Rows exactly like the ones a killed engine leaves: children that no longer exist,
    // and nothing in any registry that could ever finish them.
    let server = boot(dir, &brain, GO_WAIT, |db, _| {
        for _ in 0..jobs::MAX_LIVE {
            db.create_job("label", jobs::WAITING, "{}", "2026-09-03T00:00:00.000Z")
                .unwrap();
        }
    })
    .await;

    let (status, body) = post(&server.url("/api/patterns/plan?org=1"), json!({"criterion": "x"})).await;
    assert_eq!(status, 202, "the cap was still counting jobs from a dead process: {body}");
    until(&server, body["id"].as_i64().unwrap(), jobs::DONE).await;

    let (_, swept) = get(&server.url("/api/jobs/1")).await;
    assert_eq!(swept["status"], jobs::EXPIRED);
    assert!(swept["finished_at"].is_string(), "{swept}");
}

#[tokio::test]
async fn a_pattern_is_listed_with_how_many_calls_of_the_selection_it_matched() {
    let server = serve_with("/nonexistent/brain", a_pattern).await;

    // Nothing applied yet, so nothing matched. Zero, and not a missing field: the count was
    // taken and the answer is none.
    let (status, body) = get(&server.url("/api/patterns?org=1")).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body[0]["matched"], 0);

    post(&server.url("/api/patterns/1/apply"), json!({})).await;
    let (_, body) = get(&server.url("/api/patterns?org=1")).await;
    assert_eq!(body[0]["matched"], 1);
}

#[tokio::test]
async fn a_rule_edited_to_match_nothing_reads_zero_and_charts_nothing() {
    let server = serve_with("/nonexistent/brain", a_pattern).await;
    post(&server.url("/api/patterns/1/apply"), json!({})).await;

    let edit = json!({
        "rule": {"any_phrases": ["nobody has ever said this"]},
        "mode": "free",
        "daily_cap_usd": 1.0,
    });
    assert_eq!(put(&server.url("/api/patterns/1"), edit).await.0, 200);
    let (status, body) = post(&server.url("/api/patterns/1/apply"), json!({})).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body, json!({"matched": 0, "of": 3}));

    let (_, list) = get(&server.url("/api/patterns?org=1")).await;
    assert_eq!(list[0]["matched"], 0);

    // And the chart is empty rather than a flat line of zeroes: an axis with no instants
    // on it is no chart at all.
    let (_, stats) = get(&server.url("/api/stats?org=1&pattern=1")).await;
    assert_eq!(stats["totals"]["calls"], 0);
    assert_eq!(stats["per_bucket"], json!([]));
}

#[tokio::test]
async fn the_call_list_can_be_cut_to_one_patterns_matches() {
    let server = serve_with("/nonexistent/brain", a_pattern).await;
    post(&server.url("/api/patterns/1/apply"), json!({})).await;

    let (status, body) = get(&server.url("/api/calls?org=1&pattern=1")).await;
    assert_eq!(status, 200, "{body}");
    let ids: Vec<&str> = body
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, vec!["c1"]);
}

#[tokio::test]
async fn a_matched_call_carries_the_reason_it_was_labelled_only_under_its_own_pattern() {
    let server = serve_with("/nonexistent/brain", a_pattern).await;
    server
        .db()
        .conn()
        .execute(
            "INSERT INTO pattern_labels (pattern_id, call_id, llm_match, rule_match, evidence)
             VALUES (1, 'c1', 1, 1, 'the caller says \"get me a human\" at 0:14')",
            [],
        )
        .unwrap();
    post(&server.url("/api/patterns/1/apply"), json!({})).await;

    let (status, body) = get(&server.url("/api/calls?org=1&pattern=1")).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body[0]["evidence"], "the caller says \"get me a human\" at 0:14");

    // The same call, asked for without naming a pattern. A reason belongs to the pair, so
    // there is nothing to report here — and NULL, not the string, is what says so.
    let (_, all) = get(&server.url("/api/calls?org=1")).await;
    let c1 = all.as_array().unwrap().iter().find(|c| c["id"] == "c1").unwrap();
    assert!(c1["evidence"].is_null(), "{c1}");

    // And a call the model never read is in the list with no reason rather than out of it.
    let (_, drawer) = get(&server.url("/api/calls/c2")).await;
    assert!(drawer["evidence"].is_null(), "{drawer}");
}

#[tokio::test]
async fn a_pattern_narrows_the_selection_rather_than_paging_its_own_matches() {
    let server = serve_with("/nonexistent/brain", a_pattern).await;
    post(&server.url("/api/patterns/1/apply"), json!({})).await;

    // `last=1` is the newest call, which is `c3`. The only call this pattern matched is the
    // oldest, so the answer is none — not "the newest matched call", which is what folding
    // the pattern into the `WHERE` would have returned.
    let (_, body) = get(&server.url("/api/calls?org=1&last=1&pattern=1")).await;
    assert_eq!(body, json!([]));

    // And the count beside the pattern's name says the same thing about the same calls.
    let (_, list) = get(&server.url("/api/patterns?org=1&last=1")).await;
    assert_eq!(list[0]["matched"], 0);
}

#[tokio::test]
async fn a_pattern_list_without_an_org_says_which_parameter_is_missing() {
    let server = serve_with("/nonexistent/brain", a_pattern).await;
    let (status, body) = get(&server.url("/api/patterns")).await;
    assert_eq!(status, 400, "{body}");
    assert!(body["error"].as_str().unwrap().contains("org"), "{body}");
}

/// S-33. Before it, every message in the wizard's chat cost money and booked none: the one
/// place the register left the spec's "no model call without a shown cost" broken.
///
/// Nothing in the engine changed for it — which is the thing worth holding down. `plan`
/// reports what it spent the way `label` already did, so the row and the day's spend pick
/// it up through the path that was already there.
#[tokio::test]
async fn a_plan_books_what_it_cost_without_ever_parking() {
    let server = served(PRICED_ANSWER).await;

    let (status, body) = post(
        &server.url("/api/patterns/plan?org=1"),
        json!({"criterion": "asked for a person", "max_usd": 1.0}),
    )
    .await;
    assert_eq!(status, 202, "{body}");
    let id = body["id"].as_i64().unwrap();
    let done = until(&server, id, jobs::DONE).await;

    // The price it quoted, off the log, and the price it paid, off the row.
    assert_eq!(done["estimate_usd"], 0.0438);
    assert_eq!(done["cost_usd"], 0.0031);

    let spent = server.db().spend_on(&graphify::now()[..10], 1).unwrap();
    assert!((spent - 0.0031).abs() < 1e-9, "the day's spend was {spent}");
}

/// S-34. The wizard picks a model in step one and the brain prices the message at that
/// model's rate, so the picked model has to survive the trip. The engine knows nothing
/// about the field and that is the point of the test: the body is forwarded whole, so a
/// field the brain adds needs no change here.
#[tokio::test]
async fn a_plan_reaches_the_brain_with_the_model_the_wizard_picked() {
    let server = served(PRICED_ANSWER).await;

    let (status, body) = post(
        &server.url("/api/patterns/plan?org=1"),
        json!({"criterion": "asked for a person", "model": "opus", "max_usd": 1.0}),
    )
    .await;
    assert_eq!(status, 202, "{body}");
    until(&server, body["id"].as_i64().unwrap(), jobs::DONE).await;

    let sent = request(&server.path());
    assert_eq!(sent["model"], "opus");
    assert_eq!(sent["criterion"], "asked for a person");
    assert_eq!(sent["max_usd"], 1.0);
}

/// The go stays the Send button. A plan that parked would hold one of the four live slots
/// for half an hour over four tenths of a cent.
#[tokio::test]
async fn a_plan_is_never_parked_on_a_go() {
    let server = served(PRICED_ANSWER).await;

    let (_, body) = post(
        &server.url("/api/patterns/plan?org=1"),
        json!({"criterion": "asked for a person", "max_usd": 1.0}),
    )
    .await;
    let done = until(&server, body["id"].as_i64().unwrap(), jobs::DONE).await;

    assert_eq!(done["status"], jobs::DONE);
    // 409 and not 404: the row exists. What it is not, and never was, is parked on a go.
    let (status, why) = post(&server.url(&format!("/api/jobs/{}/go", body["id"])), json!({})).await;
    assert_eq!(status, 409, "a plan answered a go: {why}");
}

// --- reading a price back out of a log --------------------------------------------------

/// `estimate` is what the API answers `estimate_usd` with, and it is the same judgement
/// the supervisor parks on. Tested here rather than through a server because the guard
/// added by S-37 means a bad quote never reaches a log any more — so a log that already
/// has one, written before that guard existed, is the only way this is reached.
#[test]
fn a_price_read_back_out_of_a_log_is_one_that_can_be_shown_to_someone() {
    assert_eq!(jobs::estimate("ESTIMATE 0.1094\n"), Some(0.1094));
    // Free is not missing: a run with nothing to read costs nothing, and that is a figure.
    assert_eq!(jobs::estimate("ESTIMATE 0\n"), Some(0.0));
    assert_eq!(jobs::estimate("ESTIMATE abc\n"), None);
    assert_eq!(jobs::estimate("ESTIMATE nan\n"), None);
    assert_eq!(jobs::estimate("ESTIMATE inf\n"), None);
    assert_eq!(jobs::estimate("ESTIMATE -5\n"), None);
    // The last one wins, which is how it read before and how a re-quote should behave.
    assert_eq!(jobs::estimate("ESTIMATE 0.2\nPROGRESS 1/2\nESTIMATE 0.3\n"), Some(0.3));
    assert_eq!(jobs::estimate("PROGRESS 1/2\n"), None);
}

// --- the quote the engine will not park on ---------------------------------------------

/// Start a labelling job against a brain that quotes `quote`, and return the failed row.
///
/// `until` would sit through six hundred seconds of `waiting` before it gave up, so the
/// wait is wound down: a run that parks here is a bug, and the test should say so quickly.
async fn refused(quote: &str) -> (Server, Value) {
    let server = served_for(&bad_quote(quote), Duration::from_millis(200)).await;
    let (status, body) = post(
        &server.url("/api/patterns/label?org=1"),
        json!({"criterion": "asked for a human", "call_ids": ["c1"], "model": "sonnet", "max_usd": 1.0}),
    )
    .await;
    assert_eq!(status, 202, "{body}");
    let failed = until(&server, body["id"].as_i64().unwrap(), jobs::FAILED).await;
    (server, failed)
}

/// Nothing this job did can have cost anything: not the row, not the day, not the ledger.
fn spent_nothing(server: &Server, job: &Value) {
    assert_eq!(job["cost_usd"], 0.0);
    assert!(job["output"].is_null(), "something came back: {job}");
    let db = server.db();
    assert_eq!(db.spend_on(&graphify::now()[..10], 1).unwrap(), 0.0);
    let total: f64 = db
        .conn()
        .query_row("SELECT COALESCE(SUM(usd), 0) FROM spend", [], |r| r.get(0))
        .unwrap();
    assert_eq!(total, 0.0, "a job that never showed a price booked spend");
}

#[tokio::test]
async fn a_quote_that_is_not_a_number_fails_the_job_instead_of_parking_it() {
    let (server, failed) = refused("abc").await;
    assert_eq!(failed["status"], jobs::FAILED);
    // The text is in the reason because whoever reads this log needs to know what the
    // brain actually said, not that something unspecified was wrong with it.
    assert!(failed["log"].as_str().unwrap().contains(r#"quoted "abc""#), "{failed}");
    assert!(failed["estimate_usd"].is_null(), "{failed}");
    spent_nothing(&server, &failed);
}

#[tokio::test]
async fn a_quote_of_nan_fails_the_job_rather_than_reaching_the_browser_as_no_price() {
    // `"nan".parse::<f64>()` is `Ok(NaN)` and serde_json writes a non-finite float as
    // `null`, so without the check this parks and the browser is handed a waiting job
    // with no price on it — which is the state S-36 had to teach the wizard to draw.
    let (server, failed) = refused("nan").await;
    assert_eq!(failed["status"], jobs::FAILED);
    assert!(failed["log"].as_str().unwrap().contains(r#"quoted "nan""#), "{failed}");
    spent_nothing(&server, &failed);
}

#[tokio::test]
async fn a_quote_of_inf_fails_the_job_for_the_same_reason_nan_does() {
    let (server, failed) = refused("inf").await;
    assert_eq!(failed["status"], jobs::FAILED);
    assert!(failed["log"].as_str().unwrap().contains(r#"quoted "inf""#), "{failed}");
    spent_nothing(&server, &failed);
}

#[tokio::test]
async fn a_negative_quote_never_reaches_the_go_button() {
    // This one parses and serialises perfectly well. Left alone it would put `-$5.0000`
    // on the button that buys, which is worse than the dash a missing price gets: a dash
    // says nobody knows what this costs, and a number says somebody does.
    let (server, failed) = refused("-5").await;
    assert_eq!(failed["status"], jobs::FAILED);
    assert!(failed["log"].as_str().unwrap().contains(r#"quoted "-5""#), "{failed}");
    assert!(failed["estimate_usd"].is_null(), "{failed}");
    spent_nothing(&server, &failed);
}

#[tokio::test]
async fn the_text_of_a_bad_quote_is_scrubbed_before_it_reaches_the_log() {
    let _guard = env_lock().await;
    clear_env();

    // A brain broken enough to print something other than a price on its `ESTIMATE` line
    // is broken enough to print anything there, and it is holding the keys when it does.
    let server = served_for(&bad_quote("sk-ant-fake-0001"), Duration::from_millis(200)).await;
    {
        let store = Secrets::open(server.path().join(".secret")).unwrap();
        store.set(&server.db(), None, "anthropic", "sk-ant-fake-0001").unwrap();
    }
    let (_, body) = post(
        &server.url("/api/patterns/label?org=1"),
        json!({"criterion": "asked for a human", "call_ids": ["c1"], "model": "sonnet", "max_usd": 1.0}),
    )
    .await;
    let failed = until(&server, body["id"].as_i64().unwrap(), jobs::FAILED).await;

    let log = failed["log"].as_str().unwrap();
    assert!(!log.contains("sk-ant-fake-0001"), "a key reached the log: {log}");
    assert!(log.contains("***"), "the reason lost the text entirely: {log}");
}

#[tokio::test]
async fn a_bad_quote_is_never_parked_on_even_for_a_moment() {
    // The status the job is refused at matters as much as the one it ends at. A job that
    // touches `waiting` on its way to `failed` was, for that moment, a job a `go` could
    // have been sent to.
    let server = served_for(&bad_quote("nan"), Duration::from_millis(200)).await;
    let (_, body) = post(
        &server.url("/api/patterns/label?org=1"),
        json!({"criterion": "asked for a human", "call_ids": ["c1"], "model": "sonnet", "max_usd": 1.0}),
    )
    .await;
    let id = body["id"].as_i64().unwrap();

    let url = server.url(&format!("/api/jobs/{id}"));
    let mut seen = Vec::new();
    for _ in 0..1500 {
        let (_, job) = get(&url).await;
        let status = job["status"].as_str().unwrap_or_default().to_string();
        if seen.last() != Some(&status) {
            seen.push(status.clone());
        }
        if status == jobs::FAILED {
            break;
        }
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    assert!(seen.contains(&jobs::FAILED.to_string()), "never failed: {seen:?}");
    assert!(!seen.contains(&jobs::WAITING.to_string()), "it parked on a bad quote: {seen:?}");

    // And the one call that could have spent something is refused, because there is no
    // parked job for it to approve.
    let (status, why) = post(&server.url(&format!("/api/jobs/{id}/go")), json!({})).await;
    assert_eq!(status, 409, "a job that never showed a price answered a go: {why}");
}
