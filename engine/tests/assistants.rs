use graphify::assistants::{run, Opts};
use graphify::db::Db;
use rusqlite::Connection;
use serde_json::{json, Value};
use std::path::PathBuf;
use tempfile::TempDir;
use wiremock::matchers::{method, path, query_param_is_missing};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// The synthetic assistant and tool list captured from a probe, scrubbed. Same files the
/// spec's acceptance criteria name.
const ASSISTANT: &str = include_str!("fixtures/assistant.json");
const TOOLS: &str = include_str!("fixtures/tools.json");

fn assistant() -> Value {
    serde_json::from_str(ASSISTANT).unwrap()
}

fn tools() -> Value {
    serde_json::from_str(TOOLS).unwrap()
}

/// Answer each list endpoint with one fixed body. Every body here is shorter than a page,
/// so the walk stops after a single request per resource.
async fn serve(tools: Value, assistants: Value, squads: Value) -> MockServer {
    let server = MockServer::start().await;
    for (resource, body) in [
        ("/tool", tools),
        ("/assistant", assistants),
        ("/squad", squads),
    ] {
        Mock::given(method("GET"))
            .and(path(resource))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(&server)
            .await;
    }
    server
}

/// The common case: the two fixtures, no squads.
async fn serve_fixtures() -> MockServer {
    serve(tools(), json!([assistant()]), json!([])).await
}

struct Fixture {
    _dir: TempDir,
    path: PathBuf,
    db: Db,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("graphify.db");
    let db = Db::open(&path).unwrap();
    db.create_org("acme").unwrap();
    Fixture {
        _dir: dir,
        path,
        db,
    }
}

impl Fixture {
    /// One column of the single stored assistant, as a string, or `None` for NULL.
    fn col(&self, table: &str, column: &str) -> Option<String> {
        Connection::open(&self.path)
            .unwrap()
            .query_row(&format!("SELECT {column} FROM {table}"), [], |r| r.get(0))
            .unwrap()
    }

    fn json(&self, table: &str, column: &str) -> Value {
        serde_json::from_str(&self.col(table, column).unwrap()).unwrap()
    }
}

fn opts(server: &MockServer) -> Opts {
    Opts {
        org: "acme".into(),
        base: server.uri(),
        key: "k".into(),
    }
}

/// The spec's acceptance case for the assistant fixture.
#[tokio::test]
async fn the_assistant_lands_as_slim_columns() {
    let f = fixture();
    let server = serve_fixtures().await;

    let report = run(&f.db, &opts(&server)).await.unwrap();

    assert_eq!(
        report.to_string(),
        "org acme: 3 tools, 1 assistants written, 0 unchanged"
    );
    assert!(f
        .col("assistants", "system_prompt")
        .unwrap()
        .starts_with("You are a service-desk"));
    assert_eq!(f.col("assistants", "model").as_deref(), Some("gpt-4.1"));
    assert_eq!(
        f.col("assistants", "transcriber_model").as_deref(),
        Some("flux-general-multi")
    );
    assert_eq!(f.json("assistants", "tool_ids").as_array().unwrap().len(), 3);
    assert!(f.json("assistants", "structured_schema")["properties"]["call_intent"]["enum"]
        .as_array()
        .unwrap()
        .contains(&json!("transfer_request")));
}

/// The spec's acceptance case for the tools fixture.
#[tokio::test]
async fn the_transfer_tool_is_flagged() {
    let f = fixture();
    let server = serve_fixtures().await;

    run(&f.db, &opts(&server)).await.unwrap();

    let row: (String, i64) = Connection::open(&f.path)
        .unwrap()
        .query_row(
            "SELECT name, is_transfer FROM tools WHERE type = 'transferCall'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(row, ("acmeTransferAssistant_SpringfieldProd".into(), 1));
}

/// The whole reason tools are fetched first: this is what the extractor reads to call a
/// call transferred when `endedReason` does not say so.
#[tokio::test]
async fn the_stored_tools_feed_transfer_detection() {
    let f = fixture();
    let server = serve_fixtures().await;

    run(&f.db, &opts(&server)).await.unwrap();

    let names = f.db.transfer_tool_names(1).unwrap();
    assert_eq!(names.len(), 1);
    assert!(names.contains("acmeTransferAssistant_SpringfieldProd"));
}

#[tokio::test]
async fn an_unchanged_assistant_is_not_rewritten() {
    let f = fixture();
    let server = serve_fixtures().await;

    run(&f.db, &opts(&server)).await.unwrap();
    let second = run(&f.db, &opts(&server)).await.unwrap();

    assert_eq!(second.written, 0);
    assert_eq!(second.unchanged, 1);
}

#[tokio::test]
async fn a_new_version_rewrites_the_row() {
    let f = fixture();
    run(&f.db, &opts(&serve_fixtures().await)).await.unwrap();

    let mut edited = assistant();
    edited["latestVersion"] = json!("v8");
    let server = serve(tools(), json!([edited]), json!([])).await;
    let report = run(&f.db, &opts(&server)).await.unwrap();

    assert_eq!(report.written, 1);
    assert_eq!(f.col("assistants", "version").as_deref(), Some("v8"));
}

/// Vapi does not always bump the version for a prompt edit, and the prompt is the part
/// that changes what the brain sees, so the hash has to be watched separately.
#[tokio::test]
async fn a_changed_prompt_rewrites_the_row_at_the_same_version() {
    let f = fixture();
    run(&f.db, &opts(&serve_fixtures().await)).await.unwrap();
    let before = f.col("assistants", "prompt_sha256");

    let mut edited = assistant();
    edited["model"]["messages"][0]["content"] = json!("You are a service-desk robot. Rewritten.");
    let server = serve(tools(), json!([edited]), json!([])).await;
    let report = run(&f.db, &opts(&server)).await.unwrap();

    assert_eq!(report.written, 1);
    assert_ne!(f.col("assistants", "prompt_sha256"), before);
}

/// A disabled plan can still carry a leftover schema. Storing it would promise columns
/// that nothing ever fills.
#[tokio::test]
async fn a_disabled_structured_data_plan_stores_no_schema() {
    let f = fixture();
    let mut edited = assistant();
    edited["analysisPlan"]["structuredDataPlan"]["enabled"] = json!(false);
    let server = serve(tools(), json!([edited]), json!([])).await;

    run(&f.db, &opts(&server)).await.unwrap();

    assert_eq!(f.col("assistants", "structured_schema"), None);
}

/// Hashing the empty string would give every prompt-less assistant the same fingerprint.
#[tokio::test]
async fn an_assistant_with_no_system_prompt_has_no_hash() {
    let f = fixture();
    let server = serve(
        json!([]),
        json!([{ "id": "a1", "createdAt": "2026-09-01T00:00:00.000Z" }]),
        json!([]),
    )
    .await;

    run(&f.db, &opts(&server)).await.unwrap();

    assert_eq!(f.col("assistants", "system_prompt"), None);
    assert_eq!(f.col("assistants", "prompt_sha256"), None);
}

/// Squad members carry either an id or a whole inline assistant. Only the ones with an id
/// can be keyed on; the probe org had no squads at all, so this path stays small.
#[tokio::test]
async fn a_squad_member_with_an_id_is_stored_and_one_without_is_skipped() {
    let f = fixture();
    let squads = json!([{
        "id": "sq1",
        "createdAt": "2026-09-01T00:00:00.000Z",
        "members": [
            { "assistant": { "id": "a-squad", "name": "Overflow", "latestVersion": "v1" } },
            { "assistant": { "name": "Transient, no id" } },
            { "assistantId": "00000000-0000-4000-8000-00000000a001" },
        ],
    }]);
    let server = serve(json!([]), json!([assistant()]), squads).await;

    let report = run(&f.db, &opts(&server)).await.unwrap();

    assert_eq!(report.written, 2, "the fixture assistant and the squad one");
    let names: Vec<String> = Connection::open(&f.path)
        .unwrap()
        .prepare("SELECT id FROM assistants ORDER BY id")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(names, vec!["00000000-0000-4000-8000-00000000a001", "a-squad"]);
}

/// A full page means there may be more; a short one ends the walk.
#[tokio::test]
async fn a_full_page_is_followed_by_a_cursored_one() {
    let f = fixture();
    let server = MockServer::start().await;
    let full: Vec<Value> = (0..100)
        .map(|i| json!({ "id": format!("t{i:03}"), "type": "function",
                         "function": { "name": format!("f{i:03}") },
                         "createdAt": format!("2026-09-01T00:00:00.{:04}Z", 1000 - i) }))
        .collect();
    Mock::given(method("GET"))
        .and(path("/tool"))
        .and(query_param_is_missing("createdAtLt"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!(full)))
        .mount(&server)
        .await;
    // Anything with a cursor is the second page, and it is short, so the walk stops.
    Mock::given(method("GET"))
        .and(path("/tool"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            { "id": "t100", "type": "transferCall", "function": { "name": "handoff" },
              "createdAt": "2026-08-31T00:00:00.000Z" }
        ])))
        .mount(&server)
        .await;
    for resource in ["/assistant", "/squad"] {
        Mock::given(method("GET"))
            .and(path(resource))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
            .mount(&server)
            .await;
    }

    let report = run(&f.db, &opts(&server)).await.unwrap();

    assert_eq!(report.tools, 101);
    assert!(f.db.transfer_tool_names(1).unwrap().contains("handoff"));
}

#[tokio::test]
async fn an_unknown_org_is_an_error_before_any_request() {
    let f = fixture();
    let server = serve_fixtures().await;

    let err = run(
        &f.db,
        &Opts {
            org: "globex".into(),
            ..opts(&server)
        },
    )
    .await
    .unwrap_err();

    assert!(err.to_string().contains("globex"), "was: {err}");
    assert!(server.received_requests().await.unwrap().is_empty());
}
