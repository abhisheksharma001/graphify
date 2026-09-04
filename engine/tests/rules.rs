//! What a rule means. The pure half needs no database, so most of this file is a rule, a
//! transcript, and the answer.

use assert_cmd::Command;
use graphify::db::{Call, Db, ToolCall};
use graphify::rules::{apply, matches, validate, Subject, Tool};
use rusqlite::Connection;
use std::fs;
use tempfile::{tempdir, TempDir};

fn call(transcript: &str) -> Subject {
    Subject {
        id: "c1".to_string(),
        transcript: Some(transcript.to_string()),
        ..Subject::default()
    }
}

fn tool(name: &str, failed: bool) -> Tool {
    Tool {
        name: Some(name.to_string()),
        failed: Some(failed),
    }
}

/// True or false for one rule against one call, with the rule written the way a model
/// returns it.
fn hit(rule: &str, call: &Subject) -> bool {
    matches(&validate(rule, "test").unwrap(), call)
}

// --- who said it -----------------------------------------------------------

/// The acceptance test. The bot offering to fetch a real human is the assistant working;
/// counting it as a customer asking for one is the whole failure this DSL exists to avoid.
#[test]
fn a_phrase_the_bot_said_does_not_match_a_rule_about_the_user() {
    let transcript = "AI: I can get you a real human if you like.\nUser: no, go on.";
    assert!(!hit(
        r#"{"any_phrases": ["real human"], "speaker": "user"}"#,
        &call(transcript)
    ));
}

/// The same transcript and the same phrase, asked about the bot. Without this the test
/// above would pass just as well on a rule engine that never matches anything.
#[test]
fn the_same_phrase_matches_when_the_rule_asks_about_the_bot() {
    let transcript = "AI: I can get you a real human if you like.\nUser: no, go on.";
    assert!(hit(
        r#"{"any_phrases": ["real human"], "speaker": "bot"}"#,
        &call(transcript)
    ));
    assert!(hit(
        r#"{"any_phrases": ["real human"]}"#,
        &call(transcript)
    ));
}

#[test]
fn a_line_with_no_speaker_continues_the_previous_turn() {
    let transcript = "User: can you put me through\nto a real human please";
    assert!(hit(
        r#"{"any_phrases": ["real human"], "speaker": "user"}"#,
        &call(transcript)
    ));
}

/// A colon in the middle of a sentence is not a speaker, and the line it is on still
/// belongs to whoever was talking.
#[test]
fn a_colon_inside_a_sentence_does_not_invent_a_speaker() {
    let transcript = "User: two things: a real human, and soon.";
    assert!(hit(
        r#"{"any_phrases": ["real human"], "speaker": "user"}"#,
        &call(transcript)
    ));
}

#[test]
fn the_system_line_is_neither_the_user_nor_the_bot() {
    let transcript = "System: offer a real human when asked.";
    assert!(!hit(
        r#"{"any_phrases": ["real human"], "speaker": "user"}"#,
        &call(transcript)
    ));
    assert!(!hit(
        r#"{"any_phrases": ["real human"], "speaker": "bot"}"#,
        &call(transcript)
    ));
    assert!(hit(r#"{"any_phrases": ["real human"]}"#, &call(transcript)));
}

// --- phrases and regexes ---------------------------------------------------

#[test]
fn a_phrase_is_matched_without_case() {
    assert!(hit(
        r#"{"any_phrases": ["Real Human"]}"#,
        &call("User: a real human, please")
    ));
}

/// A transcriber capitalises the first word of a sentence and a rule is written in lower
/// case. If that were a miss, every rule would need its own `(?i)`.
#[test]
fn a_regex_is_matched_without_case() {
    assert!(hit(
        r#"{"regex": ["\\btalk to (a|an|the) (agent|human|person)\\b"]}"#,
        &call("User: Talk to a person.")
    ));
}

#[test]
fn a_regex_can_ask_for_case_back() {
    assert!(!hit(
        r#"{"regex": ["(?-i)talk to a person"]}"#,
        &call("User: Talk to a person.")
    ));
}

#[test]
fn a_phrase_and_a_regex_are_alternatives_not_requirements() {
    let c = call("User: a real human, please");
    assert!(hit(
        r#"{"any_phrases": ["real human"], "regex": ["never appears"]}"#,
        &c
    ));
    assert!(hit(
        r#"{"any_phrases": ["never appears"], "regex": ["real human"]}"#,
        &c
    ));
}

#[test]
fn a_rule_with_no_words_in_it_is_about_the_structure_only() {
    assert!(hit(r#"{}"#, &call("User: hello")));
    assert!(hit(r#"{}"#, &Subject::default()));
}

/// A rule that asks about words cannot be answered about a call with no transcript, and
/// counting it as a match would invent the evidence.
#[test]
fn a_call_with_no_transcript_matches_no_rule_about_words() {
    assert!(!hit(r#"{"any_phrases": ["real human"]}"#, &Subject::default()));
}

// --- the structural conditions ---------------------------------------------

#[test]
fn a_list_of_ended_reasons_means_any_of_them() {
    let c = Subject {
        ended_reason: Some("customer-ended-call".to_string()),
        ..Subject::default()
    };
    assert!(hit(
        r#"{"ended_reasons": ["silence-timed-out", "customer-ended-call"]}"#,
        &c
    ));
    assert!(!hit(r#"{"ended_reasons": ["silence-timed-out"]}"#, &c));
}

#[test]
fn an_ended_group_that_was_never_set_is_in_no_list() {
    assert!(!hit(
        r#"{"ended_groups": ["customer"]}"#,
        &Subject::default()
    ));
}

#[test]
fn tool_not_called_is_the_negation_of_tool_called() {
    let booked = Subject {
        tool_calls: vec![tool("bookAppointment", false)],
        ..Subject::default()
    };
    assert!(hit(r#"{"tool_called": ["bookAppointment"]}"#, &booked));
    assert!(!hit(r#"{"tool_not_called": ["bookAppointment"]}"#, &booked));

    let didnt = Subject::default();
    assert!(!hit(r#"{"tool_called": ["bookAppointment"]}"#, &didnt));
    assert!(hit(r#"{"tool_not_called": ["bookAppointment"]}"#, &didnt));
}

#[test]
fn tool_failed_asks_about_every_tool_call_not_one() {
    let mixed = Subject {
        tool_calls: vec![tool("lookup", false), tool("bookAppointment", true)],
        ..Subject::default()
    };
    assert!(hit(r#"{"tool_failed": true}"#, &mixed));
    assert!(!hit(r#"{"tool_failed": false}"#, &mixed));

    let clean = Subject {
        tool_calls: vec![tool("lookup", false)],
        ..Subject::default()
    };
    assert!(hit(r#"{"tool_failed": false}"#, &clean));
    assert!(!hit(r#"{"tool_failed": true}"#, &clean));
}

/// NULL is unknown, and unknown answers neither question. The same rule the dashboard
/// follows when it renders a missing value as "—" rather than 0.
#[test]
fn a_transferred_nobody_recorded_matches_neither_true_nor_false() {
    let unknown = Subject::default();
    assert!(!hit(r#"{"transferred": true}"#, &unknown));
    assert!(!hit(r#"{"transferred": false}"#, &unknown));

    let no = Subject {
        transferred: Some(false),
        ..Subject::default()
    };
    assert!(hit(r#"{"transferred": false}"#, &no));
}

#[test]
fn the_duration_bounds_are_inclusive_and_combine() {
    let sixty = Subject {
        duration_s: Some(60.0),
        ..Subject::default()
    };
    assert!(hit(r#"{"min_duration_s": 60}"#, &sixty));
    assert!(hit(r#"{"max_duration_s": 60}"#, &sixty));
    assert!(hit(r#"{"min_duration_s": 30, "max_duration_s": 90}"#, &sixty));
    assert!(!hit(r#"{"min_duration_s": 61}"#, &sixty));
    assert!(!hit(
        r#"{"min_duration_s": 60}"#,
        &Subject::default(),
    ));
}

/// Both halves have to hold. A rule that gets the words right and the structure wrong is
/// a rule that did not match.
#[test]
fn the_words_and_the_structure_are_both_required() {
    let c = Subject {
        transcript: Some("User: a real human, please".to_string()),
        transferred: Some(false),
        ..Subject::default()
    };
    assert!(hit(
        r#"{"any_phrases": ["real human"], "transferred": false}"#,
        &c
    ));
    assert!(!hit(
        r#"{"any_phrases": ["real human"], "transferred": true}"#,
        &c
    ));
}

// --- what validate refuses -------------------------------------------------

/// The key is in the message. A model that invented one has to be told which one, and a
/// dropped condition would otherwise leave a rule that quietly matches too much.
#[test]
fn an_unknown_key_is_refused_and_named_along_with_the_pattern() {
    let err = format!(
        "{:#}",
        validate(r#"{"min_turns": 3}"#, "asks for a human").unwrap_err()
    );
    assert!(err.contains("asks for a human"), "error was: {err}");
    assert!(err.contains("min_turns"), "error was: {err}");
}

#[test]
fn a_regex_that_does_not_compile_is_refused_and_quoted() {
    let err = format!(
        "{:#}",
        validate(r#"{"regex": ["(unclosed"]}"#, "asks for a human").unwrap_err()
    );
    assert!(err.contains("asks for a human"), "error was: {err}");
    assert!(err.contains("(unclosed"), "error was: {err}");
}

#[test]
fn a_speaker_that_is_not_a_speaker_is_refused() {
    let err = validate(r#"{"speaker": "caller"}"#, "asks for a human")
        .unwrap_err()
        .to_string();
    assert!(err.contains("caller"), "error was: {err}");
    assert!(err.contains("user, bot or any"), "error was: {err}");
}

/// An empty phrase is a substring of every line, so a rule holding one matches the whole
/// org while looking like it selects something.
#[test]
fn an_empty_phrase_is_refused_rather_than_matching_everything() {
    let err = validate(r#"{"any_phrases": ["  "]}"#, "asks for a human")
        .unwrap_err()
        .to_string();
    assert!(err.contains("every call"), "error was: {err}");
}

// --- apply -----------------------------------------------------------------

fn store() -> (TempDir, Db) {
    let dir = tempdir().unwrap();
    let db = Db::open(dir.path().join("graphify.db")).unwrap();
    db.create_org("acme").unwrap();
    (dir, db)
}

fn stored_call(db: &Db, id: &str, transcript: &str) {
    db.upsert_call(&Call {
        id: id.to_string(),
        org_id: 1,
        transcript: Some(transcript.to_string()),
        ..Call::default()
    })
    .unwrap();
}

fn pattern(db: &Db, id: i64, mode: &str, rule: &str) {
    db.conn()
        .execute(
            "INSERT INTO patterns (id, org_id, name, mode, rule) VALUES (?1, 1, ?2, ?3, ?4)",
            rusqlite::params![id, format!("p{id}"), mode, rule],
        )
        .unwrap();
}

fn matched(path: &std::path::Path, pattern_id: i64) -> Vec<String> {
    let conn = Connection::open(path).unwrap();
    let mut stmt = conn
        .prepare("SELECT call_id FROM pattern_matches WHERE pattern_id = ?1 AND source = 'rule' ORDER BY call_id")
        .unwrap();
    let rows = stmt
        .query_map([pattern_id], |r| r.get::<_, String>(0))
        .unwrap();
    rows.map(Result::unwrap).collect()
}

#[test]
fn apply_writes_one_rule_match_row_per_matching_call() {
    let (dir, mut db) = store();
    stored_call(&db, "c1", "User: get me a real human");
    stored_call(&db, "c2", "User: thanks, bye");
    pattern(&db, 1, "free", r#"{"any_phrases": ["real human"], "speaker": "user"}"#);

    let report = apply(&mut db).unwrap();
    assert_eq!(report.len(), 1);
    assert_eq!(report[0].matched, 1);
    assert_eq!(report[0].of, 2);
    assert_eq!(matched(&dir.path().join("graphify.db"), 1), vec!["c1"]);
}

/// Running twice is running once. The rule decides these rows, so a second run replaces
/// them rather than adding a second copy.
#[test]
fn apply_replaces_its_own_rows_and_leaves_a_models_alone() {
    let (dir, mut db) = store();
    stored_call(&db, "c1", "User: get me a real human");
    pattern(&db, 1, "free", r#"{"any_phrases": ["real human"]}"#);
    db.conn()
        .execute(
            "INSERT INTO pattern_matches (pattern_id, call_id, source) VALUES (1, 'c9', 'llm')",
            [],
        )
        .unwrap();

    apply(&mut db).unwrap();
    apply(&mut db).unwrap();

    let path = dir.path().join("graphify.db");
    assert_eq!(matched(&path, 1), vec!["c1"]);
    let llm: i64 = Connection::open(&path)
        .unwrap()
        .query_row(
            "SELECT count(*) FROM pattern_matches WHERE source = 'llm'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(llm, 1, "a model's answers were paid for and are not derived");
}

/// A hybrid or full pattern has a model in the loop and a daily cap on it. `graphify
/// apply` is the free command, and it does not touch them.
#[test]
fn apply_runs_free_patterns_only() {
    let (_dir, mut db) = store();
    stored_call(&db, "c1", "User: get me a real human");
    pattern(&db, 1, "hybrid", r#"{"any_phrases": ["real human"]}"#);

    assert!(apply(&mut db).unwrap().is_empty());
}

/// Half-created rows are ordinary. A pattern someone is still writing is skipped; a
/// pattern whose rule cannot work is an error that names it.
#[test]
fn a_pattern_with_no_rule_is_skipped_and_a_broken_one_is_an_error() {
    let (_dir, mut db) = store();
    stored_call(&db, "c1", "User: hello");
    db.conn()
        .execute(
            "INSERT INTO patterns (id, org_id, name, mode) VALUES (1, 1, 'half written', 'free')",
            [],
        )
        .unwrap();
    db.conn()
        .execute(
            "INSERT INTO patterns (id, name, mode, rule) VALUES (9, 'no org', 'free', '{}')",
            [],
        )
        .unwrap();
    assert!(apply(&mut db).unwrap().is_empty());

    pattern(&db, 2, "free", r#"{"regex": ["(unclosed"]}"#);
    let err = apply(&mut db).unwrap_err().to_string();
    assert!(err.contains("p2"), "error was: {err}");
}

// --- rule-check ------------------------------------------------------------

/// The brain has no database handle and no opinion about what a rule means. It writes the
/// calls it labelled to a file, and the engine says which ones the rule agrees with.
#[test]
fn rule_check_prints_the_ids_that_match_and_nothing_else() {
    let dir = tempdir().unwrap();
    let rule = dir.path().join("rule.json");
    let calls = dir.path().join("calls.json");
    fs::write(
        &rule,
        r#"{"any_phrases": ["real human"], "speaker": "user", "tool_not_called": ["bookAppointment"]}"#,
    )
    .unwrap();
    fs::write(
        &calls,
        r#"[
             {"id": "c1", "transcript": "User: get me a real human"},
             {"id": "c2", "transcript": "AI: I can get you a real human"},
             {"id": "c3", "transcript": "User: a real human please",
              "tool_calls": [{"name": "bookAppointment", "failed": false}]}
           ]"#,
    )
    .unwrap();

    Command::cargo_bin("graphify")
        .unwrap()
        .args(["rule-check", "--rule"])
        .arg(&rule)
        .arg("--calls")
        .arg(&calls)
        .assert()
        .success()
        .stdout("c1\n");
}

/// It reads two files and no database, so it must work with `GRAPHIFY_DB` pointing at
/// nothing at all — the brain runs this from wherever it happens to be.
#[test]
fn rule_check_needs_no_database() {
    let dir = tempdir().unwrap();
    let rule = dir.path().join("rule.json");
    let calls = dir.path().join("calls.json");
    fs::write(&rule, r#"{}"#).unwrap();
    fs::write(&calls, r#"[{"id": "c1"}]"#).unwrap();

    Command::cargo_bin("graphify")
        .unwrap()
        .env("GRAPHIFY_DB", dir.path().join("nowhere/graphify.db"))
        .args(["rule-check", "--rule"])
        .arg(&rule)
        .arg("--calls")
        .arg(&calls)
        .assert()
        .success()
        .stdout("c1\n");
}

#[test]
fn rule_check_on_a_bad_rule_names_the_file() {
    let dir = tempdir().unwrap();
    let rule = dir.path().join("rule.json");
    let calls = dir.path().join("calls.json");
    fs::write(&rule, r#"{"speaker": "caller"}"#).unwrap();
    fs::write(&calls, r#"[]"#).unwrap();

    let out = Command::cargo_bin("graphify")
        .unwrap()
        .args(["rule-check", "--rule"])
        .arg(&rule)
        .arg("--calls")
        .arg(&calls)
        .assert()
        .failure();
    let stderr = String::from_utf8_lossy(&out.get_output().stderr).to_string();
    assert!(stderr.contains("rule.json"), "stderr was: {stderr}");
}

/// The file the brain writes is a contract. A key nobody reads is a key somebody meant to
/// be read, and answering "no match" to a misspelt transcript would look exactly like a
/// rule that is merely too narrow.
#[test]
fn rule_check_refuses_a_call_with_a_key_it_does_not_know() {
    let dir = tempdir().unwrap();
    let rule = dir.path().join("rule.json");
    let calls = dir.path().join("calls.json");
    fs::write(&rule, r#"{}"#).unwrap();
    fs::write(&calls, r#"[{"id": "c1", "transcripts": "User: hello"}]"#).unwrap();

    let out = Command::cargo_bin("graphify")
        .unwrap()
        .args(["rule-check", "--rule"])
        .arg(&rule)
        .arg("--calls")
        .arg(&calls)
        .assert()
        .failure();
    let stderr = String::from_utf8_lossy(&out.get_output().stderr).to_string();
    assert!(stderr.contains("transcripts"), "stderr was: {stderr}");
}

/// `ToolCall` is the engine's row and `Tool` is what a rule may see. This is here so the
/// import above is not the only thing holding the two together.
#[test]
fn the_stored_tool_call_row_and_the_rule_view_agree_on_the_two_fields() {
    let row = ToolCall {
        name: Some("bookAppointment".to_string()),
        failed: Some(true),
        ..ToolCall::default()
    };
    let seen = Tool {
        name: row.name.clone(),
        failed: row.failed,
    };
    assert!(hit(
        r#"{"tool_called": ["bookAppointment"], "tool_failed": true}"#,
        &Subject {
            tool_calls: vec![seen],
            ..Subject::default()
        }
    ));
}
