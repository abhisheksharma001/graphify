//! The ask box, from the two things the engine is answerable for: what a question costs
//! before it is asked, and what happens to somebody who reads that price and walks away.
//!
//! The second one is the step's acceptance, and it is why the price is worked out here at
//! all instead of by the brain like every other quote in graphify. So the first test below
//! is about a row that must not exist.
//!
//! Nothing here reaches a provider: the brain is a shell script.

use graphify::ask;
use graphify::auth::Auth;
use graphify::db::{Call, Db};
use graphify::queries::Filters;
use graphify::secrets::Secrets;
use graphify::server::{router, App};
use regex::Regex;
use serde_json::{json, Value};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

// --- the price table, against the one a person actually maintains -----------------------

/// `graphify_brain.cost` is where the rates are read off a vendor's page and written down.
/// `engine::ask` mirrors them so a quote can be given without spawning anything.
const COST_PY: &str = include_str!("../../brain/src/graphify_brain/cost.py");

/// The two constants the estimate's arithmetic rests on live in `label.py`, and the ones
/// about how much a question may carry live in `ask.py`. Both are mirrored too.
const LABEL_PY: &str = include_str!("../../brain/src/graphify_brain/label.py");
const ASK_PY: &str = include_str!("../../brain/src/graphify_brain/ask.py");

/// A `NAME = 60_000` out of a Python module, with the underscores taken out.
fn constant(source: &str, name: &str) -> usize {
    let re = Regex::new(&format!(r"(?m)^{name} = ([\d_]+)")).unwrap();
    let found = re
        .captures(source)
        .unwrap_or_else(|| panic!("no {name} in the module"));
    found[1].replace('_', "").parse().unwrap()
}

#[test]
fn the_rates_are_the_brains_rates() {
    // The whole reason this module holds a price table at all is the acceptance below: a
    // quote that spawns nothing cannot ask the brain what things cost. This is what keeps
    // the copy honest — one place to edit a price, and a failing build if only one of the
    // two is edited.
    let re = Regex::new(r#""(\w+)": Price\("(\w+)", "([^"]+)", ([\d.]+), ([\d.]+)\)"#).unwrap();
    let brain: Vec<(String, String, String, f64, f64)> = re
        .captures_iter(COST_PY)
        .map(|c| {
            (
                c[1].to_string(),
                c[2].to_string(),
                c[3].to_string(),
                c[4].parse().unwrap(),
                c[5].parse().unwrap(),
            )
        })
        .collect();
    let ours: Vec<(String, String, String, f64, f64)> = ask::PRICES
        .iter()
        .map(|(name, r)| {
            (
                (*name).to_string(),
                r.provider.to_string(),
                r.model.to_string(),
                r.usd_in,
                r.usd_out,
            )
        })
        .collect();

    assert_eq!(brain.len(), 3, "found {} priced models in cost.py", brain.len());
    assert_eq!(ours, brain);
}

#[test]
fn the_constants_the_estimate_rests_on_are_the_brains() {
    assert_eq!(ask::CHARS_PER_TOKEN, constant(LABEL_PY, "CHARS_PER_TOKEN"));
    assert_eq!(ask::MAX_ANSWER_TOKENS, constant(LABEL_PY, "MAX_OUTPUT_TOKENS"));
    assert_eq!(ask::MAX_CONTEXT_TOKENS, constant(ASK_PY, "MAX_CONTEXT_TOKENS"));
    assert_eq!(ask::MAX_CALLS, constant(ASK_PY, "MAX_CALLS"));
    assert_eq!(ask::FIXED_PROMPT_CHARS, constant(ASK_PY, "FIXED_PROMPT_CHARS"));
}

#[test]
fn a_priced_call_costs_what_the_table_says() {
    let sonnet = ask::rate("sonnet").unwrap();
    assert_eq!(
        ask::usd(100_000, 2_000, "sonnet"),
        Some((100_000.0 * sonnet.usd_in + 2_000.0 * sonnet.usd_out) / 1_000_000.0)
    );
    assert_eq!(ask::usd(100, 100, "gemini"), None, "an unpriced model is not a free one");
    assert_eq!(ask::rate("  Sonnet "), ask::rate("sonnet"));
}

// --- quoting ---------------------------------------------------------------------------

fn calls(db: &Db, org: i64, lengths: &[usize]) {
    for (i, len) in lengths.iter().enumerate() {
        db.upsert_call(&Call {
            id: format!("c{i}"),
            org_id: org,
            assistant_id: Some("a-1".into()),
            created_at: Some(format!("2026-09-01T09:{i:02}:00.000Z")),
            transcript: Some("x".repeat(*len)),
            ..Call::default()
        })
        .unwrap();
    }
}

/// A database with one org and the calls a test asks for, by transcript length.
fn stored(lengths: &[usize]) -> (TempDir, Db, i64) {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(dir.path().join("graphify.db")).unwrap();
    let org = db.create_org("acme").unwrap();
    calls(&db, org, lengths);
    (dir, db, org)
}

fn filters(query: &str) -> Filters {
    Filters::from_query(query).unwrap()
}

fn quote(db: &Db, query: &str, question: &str) -> ask::Quote {
    ask::quote(db, &filters(query), question, "sonnet").unwrap()
}

fn refusal(db: &Db, question: &str, model: &str) -> String {
    match ask::quote(db, &filters("org=1"), question, model) {
        Err(ask::Error::Refused(why)) => why,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[test]
fn the_shortest_transcripts_go_in_first() {
    // The sample is skewed by construction, and this is the construction: the cheapest
    // calls first, so the most of them fit. `ask.baml` is what tells the model so.
    let (_dir, db, _org) = stored(&[9_000, 100, 4_000, 50]);

    let q = quote(&db, "org=1", "why do people call");

    assert_eq!(q.call_ids, ["c3", "c1", "c2", "c0"]);
    assert_eq!(q.readable, 4);
}

#[test]
fn no_more_transcripts_go_in_than_the_cap_allows() {
    let (_dir, db, _org) = stored(&[200; 30]);

    let q = quote(&db, "org=1", "why do people call");

    assert_eq!(q.call_ids.len(), ask::MAX_CALLS);
}

#[test]
fn the_token_cap_trims_the_sample_rather_than_the_price_growing_past_it() {
    // Each of these is a third of the whole context on its own, so three fit and the rest
    // do not — and the count that says how many *could* have gone in is still the sample's.
    let each = ask::MAX_CONTEXT_TOKENS * ask::CHARS_PER_TOKEN / 3;
    let (_dir, db, _org) = stored(&[each; 6]);

    let q = quote(&db, "org=1", "why do people call");

    assert!(q.call_ids.len() < q.readable, "{q:?}");
    assert!(q.tokens_in <= ask::MAX_CONTEXT_TOKENS, "{} tokens", q.tokens_in);
}

#[test]
fn a_call_with_nothing_to_read_is_not_in_the_sample() {
    let (_dir, db, org) = stored(&[100, 100]);
    db.upsert_call(&Call {
        id: "empty".into(),
        org_id: org,
        transcript: Some("   ".into()),
        ..Call::default()
    })
    .unwrap();
    db.upsert_call(&Call {
        id: "none".into(),
        org_id: org,
        transcript: None,
        ..Call::default()
    })
    .unwrap();

    let q = quote(&db, "org=1", "why do people call");

    assert_eq!(q.call_ids, ["c0", "c1"]);
}

#[test]
fn the_sample_is_taken_over_the_selection_the_filters_describe() {
    let (_dir, db, org) = stored(&[100, 100]);
    db.upsert_call(&Call {
        id: "other".into(),
        org_id: org,
        assistant_id: Some("a-2".into()),
        transcript: Some("y".repeat(10)),
        ..Call::default()
    })
    .unwrap();

    let q = quote(&db, "org=1&assistant_id=a-1", "why do people call");

    assert_eq!(q.call_ids, ["c0", "c1"], "the shorter call is another assistant's");
}

#[test]
fn a_bigger_sample_costs_more() {
    let (_dir, db, _org) = stored(&[3_000; 8]);

    let small = quote(&db, "org=1&last=2", "why do people call");
    let large = quote(&db, "org=1", "why do people call");

    assert!(large.usd > small.usd, "{} vs {}", large.usd, small.usd);
}

#[test]
fn the_same_question_costs_more_on_a_dearer_model() {
    let (_dir, db, _org) = stored(&[3_000; 4]);
    let f = filters("org=1");

    let sonnet = ask::quote(&db, &f, "why", "sonnet").unwrap();
    let opus = ask::quote(&db, &f, "why", "opus").unwrap();

    assert_eq!(sonnet.tokens_in, opus.tokens_in);
    assert!(opus.usd > sonnet.usd);
}

#[test]
fn an_empty_question_is_refused() {
    let (_dir, db, _org) = stored(&[100]);

    assert!(refusal(&db, "   ", "sonnet").contains("cannot be empty"));
}

#[test]
fn a_question_longer_than_a_question_is_refused() {
    let (_dir, db, _org) = stored(&[100]);

    let why = refusal(&db, &"why".repeat(ask::MAX_QUESTION_CHARS), "sonnet");

    assert!(why.contains(&ask::MAX_QUESTION_CHARS.to_string()), "{why}");
}

#[test]
fn a_model_nobody_prices_is_refused_by_name() {
    let (_dir, db, _org) = stored(&[100]);

    let why = refusal(&db, "why do people call", "gemini");

    assert!(why.contains("gemini") && why.contains("sonnet"), "{why}");
}

// --- the two routes ---------------------------------------------------------------------

/// A brain that reads the request, writes it down, quotes and answers. No `GO`: an ask job
/// never parks, because the price was agreed before the row existed.
const ANSWERS: &str = r###"#!/bin/sh
here=$(dirname "$0")
read -r request
printf '%s\n' "$request" > "$here/request.json"
echo "ESTIMATE 0.0421"
printf '%s\n' '{"answer":"## Why","calls":["c0"],"no_transcript":[],"usd":0.0032,"model":"sonnet","stopped":null}'
"###;

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

    fn db(&self) -> Db {
        Db::open(self.dir.path().join("graphify.db")).unwrap()
    }
}

async fn served(lengths: &[usize]) -> Server {
    let dir = tempfile::tempdir().unwrap();
    let brain = dir.path().join("brain.sh");
    std::fs::write(&brain, ANSWERS).unwrap();
    std::fs::set_permissions(&brain, std::fs::Permissions::from_mode(0o755)).unwrap();

    let db = Db::open(dir.path().join("graphify.db")).unwrap();
    let org = db.create_org("acme").unwrap();
    calls(&db, org, lengths);

    let store = Secrets::open(dir.path().join(".secret")).unwrap();
    let app = App::new(db, store, Auth::new(None)).with_brain(brain.to_string_lossy().to_string());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router(app)).await.unwrap() });
    Server {
        dir,
        base: format!("http://{addr}"),
    }
}

async fn post(url: &str, body: Value) -> (u16, Value) {
    let res = reqwest::Client::new()
        .post(url)
        .json(&body)
        .send()
        .await
        .unwrap();
    let status = res.status().as_u16();
    (status, res.json().await.unwrap_or(Value::Null))
}

async fn get(url: &str) -> (u16, Value) {
    let res = reqwest::get(url).await.unwrap();
    let status = res.status().as_u16();
    (status, res.json().await.unwrap_or(Value::Null))
}

fn jobs_in(db: &Db) -> i64 {
    db.conn()
        .query_row("SELECT count(*) FROM jobs", [], |r| r.get(0))
        .unwrap()
}

fn asked(dir: &Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(dir.join("request.json")).unwrap()).unwrap()
}

/// The step's acceptance.
#[tokio::test]
async fn a_price_somebody_walks_away_from_leaves_no_job_behind() {
    let s = served(&[400, 900]).await;

    let (status, quote) = post(
        &s.url("/api/ask/quote?org=1"),
        json!({ "question": "why do people call", "model": "sonnet" }),
    )
    .await;

    assert_eq!(status, 200);
    assert!(quote["usd"].as_f64().unwrap() > 0.0);
    assert_eq!(quote["call_ids"], json!(["c0", "c1"]));
    // Nothing was started, so there is nothing to cancel and nothing holding a child.
    assert_eq!(jobs_in(&s.db()), 0);
}

#[tokio::test]
async fn asking_starts_a_job_and_hands_the_brain_what_the_quote_priced() {
    let s = served(&[400, 900]).await;

    let (_, quote) = post(
        &s.url("/api/ask/quote?org=1"),
        json!({ "question": "why do people call", "model": "sonnet" }),
    )
    .await;
    let (status, started) = post(
        &s.url("/api/ask?org=1"),
        json!({ "question": "why do people call", "model": "sonnet", "max_usd": quote["usd"] }),
    )
    .await;

    assert_eq!(status, 202, "{started}");
    for _ in 0..100 {
        if s.path().join("request.json").exists() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    // What the child was handed on stdin: the request itself, which the engine built. The
    // org is not in it — that is the engine's own bookkeeping and rides in the `jobs` row.
    let body = asked(&s.path());
    assert_eq!(body["question"], "why do people call");
    assert_eq!(body["model"], "sonnet");
    assert_eq!(body["call_ids"], json!(["c0", "c1"]));
    assert_eq!(body["max_usd"], quote["usd"]);
    // The statistics travel as the string that was priced, not as an object somebody would
    // have to serialise again to show a model.
    let stats: Value = serde_json::from_str(body["stats"].as_str().unwrap()).unwrap();
    assert_eq!(stats["totals"]["calls"], 2);
}

#[tokio::test]
async fn an_ask_job_never_waits_for_a_go() {
    // The click already happened, at the price. A second approval on the child's stdin
    // would be asking the same person the same question twice — and would park a job
    // nobody is going to come back to.
    let s = served(&[400]).await;

    let (_, started) = post(
        &s.url("/api/ask?org=1"),
        json!({ "question": "why do people call", "model": "sonnet", "max_usd": 5.0 }),
    )
    .await;
    let id = started["id"].as_i64().unwrap();

    let mut job = Value::Null;
    for _ in 0..100 {
        let (_, got) = get(&s.url(&format!("/api/jobs/{id}"))).await;
        assert_ne!(got["status"], "waiting", "an ask job parked: {got}");
        if got["status"] == "done" || got["status"] == "failed" {
            job = got;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }

    assert_eq!(job["status"], "done", "{job}");
    assert_eq!(job["kind"], "ask");
    assert_eq!(job["estimate_usd"], 0.0421);
    assert_eq!(job["cost_usd"], 0.0032);
    assert!(job["output"]["answer"].as_str().unwrap().starts_with("## Why"));
}

#[tokio::test]
async fn what_an_answer_cost_is_booked_against_the_org() {
    let s = served(&[400]).await;

    let (_, started) = post(
        &s.url("/api/ask?org=1"),
        json!({ "question": "why do people call", "model": "sonnet", "max_usd": 5.0 }),
    )
    .await;
    let id = started["id"].as_i64().unwrap();
    for _ in 0..100 {
        let (_, got) = get(&s.url(&format!("/api/jobs/{id}"))).await;
        if got["status"] == "done" {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }

    let db = s.db();
    let today = graphify::now()[..10].to_string();
    assert_eq!(db.spend_on(&today, 1).unwrap(), 0.0032);
}

#[tokio::test]
async fn a_price_that_has_moved_since_the_quote_stops_at_the_figure_that_was_approved() {
    let s = served(&[400, 900]).await;

    let (status, body) = post(
        &s.url("/api/ask?org=1"),
        json!({ "question": "why do people call", "model": "sonnet", "max_usd": 0.000001 }),
    )
    .await;

    assert_eq!(status, 409, "{body}");
    assert!(
        body["error"].as_str().unwrap().contains("ask for the price again"),
        "{body}"
    );
    assert_eq!(jobs_in(&s.db()), 0, "a refused price must not start anything");
}

#[tokio::test]
async fn a_quote_needs_an_org_to_bill_and_a_question_to_price() {
    let s = served(&[400]).await;

    let (missing_org, _) = post(
        &s.url("/api/ask"),
        json!({ "question": "why", "model": "sonnet", "max_usd": 1.0 }),
    )
    .await;
    let (empty, why) = post(
        &s.url("/api/ask/quote?org=1"),
        json!({ "question": "  ", "model": "sonnet" }),
    )
    .await;
    let (unpriced, _) = post(
        &s.url("/api/ask/quote?org=1"),
        json!({ "question": "why", "model": "gemini" }),
    )
    .await;
    let (capless, _) = post(
        &s.url("/api/ask?org=1"),
        json!({ "question": "why", "model": "sonnet", "max_usd": 0 }),
    )
    .await;

    let (nobody, _) = post(
        &s.url("/api/ask/quote?org=99"),
        json!({ "question": "why", "model": "sonnet" }),
    )
    .await;

    assert_eq!(missing_org, 400);
    assert_eq!(nobody, 404, "a price for an org that is not there is a typo, not a figure");
    assert_eq!(empty, 400, "{why}");
    assert_eq!(unpriced, 400);
    assert_eq!(capless, 400);
    assert_eq!(jobs_in(&s.db()), 0);
}

#[tokio::test]
async fn a_question_carries_the_whole_filter_bar_and_not_only_the_org() {
    // Found in a browser: the ask box sends the filter bar's whole query string, because
    // that is what says which calls the question is about. A route that read the org with
    // a parser refusing every other key answered `unknown parameter window` to every
    // question asked over a window — which is all of them.
    let s = served(&[400, 900]).await;
    let bar = "org=1&window=7d&assistant_id=a-1&transferred=false";

    let (quoted, why) = post(
        &s.url(&format!("/api/ask/quote?{bar}")),
        json!({ "question": "why do people call", "model": "sonnet" }),
    )
    .await;
    let (asked, said) = post(
        &s.url(&format!("/api/ask?{bar}")),
        json!({ "question": "why do people call", "model": "sonnet", "max_usd": 5.0 }),
    )
    .await;

    assert_eq!(quoted, 200, "{why}");
    assert_eq!(asked, 202, "{said}");
}

#[tokio::test]
async fn a_quote_carries_no_field_it_was_not_given() {
    let s = served(&[400]).await;

    let (status, _) = post(
        &s.url("/api/ask/quote?org=1"),
        json!({ "question": "why", "model": "sonnet", "max_usd": 1.0 }),
    )
    .await;

    assert_eq!(status, 422, "a quote is not the place to approve a price");
}
