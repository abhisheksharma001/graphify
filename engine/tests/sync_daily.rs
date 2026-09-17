//! The daily run: the first thing the engine starts that spends money with nobody
//! watching it. So these tests are about what stops it, and what it is told before it
//! begins.
//!
//! The brain is a shell script, as it is in `jobs.rs`. Nothing here spawns Python and
//! nothing can reach a model, which is what lets a test say "no process was started at
//! all" and mean it.

use graphify::db::Db;
use graphify::jobs;
use graphify::rules;
use graphify::secrets::Secrets;
use graphify::sync::{self, Daily, DailyOpts};
use serde_json::Value;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use tempfile::TempDir;

/// A brain that writes down its request and reports one pattern read at a price.
const READS: &str = r#"#!/bin/sh
here=$(dirname "$0")
read -r request
printf '%s\n' "$request" > "$here/request.json"
echo "PROGRESS 1/1" >&2
echo '{"usd":0.0250,"patterns":[{"pattern":1,"read":3,"matched":2,"usd":0.025}],"stopped":null}'
"#;

/// A path no brain lives at. What a test points at when it is asserting that nothing was
/// started: if one ever were, the job would fail on the spawn and say so.
const NO_BRAIN: &str = "/nonexistent/graphify-brain";

fn fake(dir: &Path, body: &str) -> String {
    let path = dir.join("brain.sh");
    std::fs::write(&path, body).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path.to_string_lossy().into_owned()
}

/// The request line the fake brain was handed, parsed.
fn request(dir: &Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(dir.join("request.json")).unwrap()).unwrap()
}

// --- a database ------------------------------------------------------------------------

struct Fixture {
    dir: TempDir,
    db: Arc<Mutex<Db>>,
    store: Secrets,
}

impl Fixture {
    fn new() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(dir.path().join("graphify.db")).unwrap();
        db.create_org("acme").unwrap();
        let store = Secrets::open(dir.path().join(".secret")).unwrap();
        Fixture {
            dir,
            db: Arc::new(Mutex::new(db)),
            store,
        }
    }

    fn db(&self) -> MutexGuard<'_, Db> {
        self.db.lock().unwrap()
    }

    /// `n` calls, `c1`…`cn`, all of them asking for a person.
    fn calls(&self, n: usize) {
        let db = self.db();
        for i in 1..=n {
            db.conn()
                .execute(
                    "INSERT INTO calls (id, org_id, created_at, transcript)
                     VALUES (?1, 1, ?2, 'User: get me a human')",
                    rusqlite::params![format!("c{i}"), format!("2026-09-01T09:{i:02}:00.000Z")],
                )
                .unwrap();
        }
    }

    fn pattern(&self, mode: &str, rule: &str) -> i64 {
        let db = self.db();
        db.conn()
            .execute(
                "INSERT INTO patterns (org_id, name, criterion, plan, rule, model, mode,
                                       daily_cap_usd)
                 VALUES (1, 'Handoffs', 'asked for a person', '{}', ?1, 'sonnet', ?2, 1.0)",
                rusqlite::params![rule, mode],
            )
            .unwrap();
        db.conn().last_insert_rowid()
    }

    /// A verdict a model was paid for.
    fn labelled(&self, pattern: i64, call: &str, matched: bool) {
        self.db()
            .conn()
            .execute(
                "INSERT INTO pattern_labels (pattern_id, call_id, llm_match, rule_match, evidence)
                 VALUES (?1, ?2, ?3, NULL, 'read')",
                rusqlite::params![pattern, call, i64::from(matched)],
            )
            .unwrap();
    }

    fn matches(&self, pattern: i64) -> Vec<(String, String)> {
        let db = self.db();
        let conn = db.conn();
        let mut stmt = conn
            .prepare(
                "SELECT call_id, source FROM pattern_matches WHERE pattern_id = ?1
                  ORDER BY call_id, source",
            )
            .unwrap();
        let rows = stmt
            .query_map([pattern], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap();
        rows.collect::<rusqlite::Result<Vec<_>>>().unwrap()
    }

    /// A job row for an org, in whatever state and with whatever the brain had said by
    /// then. `log` is where a `SPENT` line lives until the row closes and the ledger
    /// hears about it.
    fn job(&self, status: &str, org: i64, created_at: &str, log: &str) {
        self.db()
            .conn()
            .execute(
                "INSERT INTO jobs (kind, status, org_id, input, cost_usd, log, created_at)
                 VALUES ('label', ?1, ?2, '{}', 0, ?3, ?4)",
                rusqlite::params![status, org, log, created_at],
            )
            .unwrap();
    }

    /// Run the daily half against a brain of the test's choosing.
    fn daily(&self, brain: &str, cap_usd: f64) -> anyhow::Result<Daily> {
        sync::daily(
            &self.db,
            &self.store,
            &DailyOpts {
                org: "acme".to_string(),
                brain: brain.to_string(),
                cap_usd,
            },
        )
    }

    /// The same, against the fake brain that reads and charges.
    fn reading(&self, cap_usd: f64) -> Daily {
        let brain = fake(self.dir.path(), READS);
        self.daily(&brain, cap_usd).unwrap()
    }
}

/// `daily_cap_from_env` reads a process-wide variable, so the tests that move it take this
/// first. Everything else passes its cap in and never touches the environment.
fn env_lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

// --- the order of the two halves ---------------------------------------------------------

#[test]
fn the_rule_runs_before_the_model_does() {
    // The acceptance in miniature: in hybrid the rule chooses which calls a model is paid
    // to read, so a rule that has not seen this morning's calls is a model that reads
    // none of them. Nothing but `daily` has touched this database.
    let f = Fixture::new();
    f.calls(3);
    let p = f.pattern("hybrid", r#"{"any_phrases": ["human"]}"#);

    let report = f.reading(5.0);

    assert_eq!(report.applied, 1);
    assert_eq!(
        f.matches(p),
        vec![
            ("c1".to_string(), "rule".to_string()),
            ("c2".to_string(), "rule".to_string()),
            ("c3".to_string(), "rule".to_string()),
        ]
    );
    assert!(report.job.is_some(), "{report:?}");
}

#[test]
fn the_brain_is_told_what_is_left_of_the_day_and_not_the_whole_cap() {
    // Two runs on one morning must not be two caps. What has already been booked against
    // the org today comes off before the brain is told what it may spend.
    let f = Fixture::new();
    f.calls(1);
    f.pattern("full", r#"{}"#);
    f.db()
        .add_spend(&graphify::now()[..10], 1, 1.5)
        .unwrap();

    f.reading(5.0);

    let asked = request(f.dir.path());
    assert_eq!(asked["org"], 1);
    assert_eq!(asked["max_usd"], 3.5);
}

#[test]
fn what_the_daily_run_spent_is_booked_against_the_org() {
    let f = Fixture::new();
    f.calls(1);
    f.pattern("full", r#"{}"#);

    let report = f.reading(5.0);

    assert_eq!(report.usd, 0.025);
    assert_eq!(f.db().spend_on(&graphify::now()[..10], 1).unwrap(), 0.025);
}

// --- what stops it before a process starts -----------------------------------------------

#[test]
fn an_org_with_no_model_backed_pattern_starts_nothing() {
    // The ordinary case, and the one that has to stay free: an org whose patterns are all
    // decided by their rules never starts an interpreter, never mind spends.
    let f = Fixture::new();
    f.calls(3);
    let p = f.pattern("free", r#"{"any_phrases": ["human"]}"#);

    let report = f.daily(NO_BRAIN, 5.0).unwrap();

    assert_eq!(report.job, None);
    assert_eq!(report.usd, 0.0);
    assert!(
        report.note.as_deref().unwrap().contains("model in the loop"),
        "{report:?}"
    );
    // The rule half still ran. It costs nothing, and it is the count the dashboard reads.
    assert_eq!(f.matches(p).len(), 3);
}

#[test]
fn a_day_that_is_already_spent_starts_nothing() {
    let f = Fixture::new();
    f.calls(1);
    f.pattern("full", r#"{}"#);
    f.db().add_spend(&graphify::now()[..10], 1, 5.0).unwrap();

    let report = f.daily(NO_BRAIN, 5.0).unwrap();

    assert_eq!(report.job, None);
    assert!(
        report.note.as_deref().unwrap().contains("already spent"),
        "{report:?}"
    );
}

#[test]
fn a_cap_of_nothing_turns_the_daily_modes_off() {
    // The escape hatch on a machine that must not spend: one variable, rather than editing
    // the mode of every pattern in the database.
    let f = Fixture::new();
    f.calls(1);
    f.pattern("full", r#"{}"#);

    let report = f.daily(NO_BRAIN, 0.0).unwrap();

    assert_eq!(report.job, None);
    assert!(report.note.is_some(), "{report:?}");
}

#[test]
fn a_negative_cap_is_refused_rather_than_read_as_zero() {
    let f = Fixture::new();
    f.pattern("full", r#"{}"#);

    let error = f.daily(NO_BRAIN, -1.0).unwrap_err().to_string();

    assert!(error.contains("cannot be negative"), "{error}");
}

#[test]
fn a_brain_that_is_not_installed_is_reported_and_not_a_failed_sync() {
    // The pull worked. A cron that treated a provider being down — or a brain that was
    // never installed — as a failed sync would re-pull the whole org tomorrow, and the
    // line it prints is what says which half of the morning went wrong.
    let f = Fixture::new();
    f.calls(1);
    f.pattern("full", r#"{}"#);

    let report = f.daily(NO_BRAIN, 5.0).unwrap();

    assert_eq!(report.status, "failed");
    assert_eq!(report.usd, 0.0);
    assert!(format!("{report}").contains("failed"), "{report}");
}

// --- the cap in the environment -----------------------------------------------------------

#[test]
fn the_default_cap_is_five_dollars() {
    let _lock = env_lock();
    std::env::remove_var(sync::DAILY_CAP_VAR);

    assert_eq!(sync::daily_cap_from_env().unwrap(), sync::DEFAULT_DAILY_CAP_USD);
}

#[test]
fn a_cap_that_is_not_a_number_is_an_error_and_not_the_default() {
    // Somebody who wrote `2,50` meant to set a cap. Handing them the five-dollar default
    // instead is the one mistake this number exists to prevent.
    let _lock = env_lock();
    std::env::set_var(sync::DAILY_CAP_VAR, "2,50");

    let error = sync::daily_cap_from_env().unwrap_err().to_string();
    std::env::remove_var(sync::DAILY_CAP_VAR);

    assert!(error.contains("not a number of dollars"), "{error}");
}

#[test]
fn a_cap_from_the_environment_is_read_as_dollars() {
    let _lock = env_lock();
    std::env::set_var(sync::DAILY_CAP_VAR, " 0.75 ");

    let cap = sync::daily_cap_from_env().unwrap();
    std::env::remove_var(sync::DAILY_CAP_VAR);

    assert_eq!(cap, 0.75);
}

// --- the model has the last word ----------------------------------------------------------

#[test]
fn a_call_the_model_rejected_does_not_come_back_when_the_rule_is_re_run() {
    // What makes a hybrid confirmation worth paying for. Left alone, Re-apply would undo
    // every rejection the analyst had bought, and the next daily run would not buy them
    // back: those calls have been read once, and a model is not asked twice.
    let f = Fixture::new();
    f.calls(3);
    let p = f.pattern("hybrid", r#"{"any_phrases": ["human"]}"#);
    f.labelled(p, "c2", false);

    rules::apply_org(&mut f.db(), 1).unwrap();

    assert_eq!(
        f.matches(p),
        vec![
            ("c1".to_string(), "rule".to_string()),
            ("c3".to_string(), "rule".to_string()),
        ]
    );
}

#[test]
fn a_free_patterns_rule_is_not_overruled_by_a_label() {
    // The wizard stores its sample against every pattern, and a free pattern is one whose
    // rule was chosen to disagree with part of that sample by a measured amount. That
    // figure is `agreement`; a rule quietly edited to match it would make it meaningless.
    let f = Fixture::new();
    f.calls(3);
    let p = f.pattern("free", r#"{"any_phrases": ["human"]}"#);
    f.labelled(p, "c2", false);

    rules::apply_org(&mut f.db(), 1).unwrap();

    assert_eq!(f.matches(p).len(), 3);
}

#[test]
fn a_rule_re_run_leaves_the_models_own_matches_alone() {
    // `source='rule'` rows are derived and are replaced outright. `source='llm'` rows were
    // paid for, and nothing arithmetic gets to throw one away.
    let f = Fixture::new();
    f.calls(2);
    let p = f.pattern("full", r#"{"any_phrases": ["nothing here"]}"#);
    f.db()
        .conn()
        .execute(
            "INSERT INTO pattern_matches (pattern_id, call_id, source) VALUES (?1, 'c1', 'llm')",
            rusqlite::params![p],
        )
        .unwrap();

    rules::apply_org(&mut f.db(), 1).unwrap();

    assert_eq!(f.matches(p), vec![("c1".to_string(), "llm".to_string())]);
}

// --- money in the air (S-66) -------------------------------------------------------------

/// Two waves of a labelling run that is still going: twenty calls read, $3.00 announced.
const IN_FLIGHT: &str = "PROGRESS 10/60\nSPENT 1.400000\nPROGRESS 20/60\nSPENT 3.000000";

/// A day old enough that nothing about it is today, whatever today is.
const LONG_AGO: &str = "2026-01-01T09:00:00.000Z";

/// What this org's unclosed jobs have announced spending today.
fn in_flight(f: &Fixture, org: i64) -> f64 {
    jobs::spend_in_flight(&f.db(), &graphify::now()[..10], org).unwrap()
}

#[test]
fn a_run_that_starts_beside_one_that_is_still_spending_is_told_what_is_left() {
    // The defect. Four jobs may be live at once, and a wizard's labelling run overlapping
    // cron's daily is an ordinary morning. Nothing reaches the ledger until a job closes,
    // so the day looked untouched and the brain was handed the whole $5.
    let f = Fixture::new();
    f.calls(1);
    f.pattern("full", r#"{}"#);
    f.job("running", 1, &graphify::now(), IN_FLIGHT);

    f.reading(5.0);

    assert_eq!(request(f.dir.path())["max_usd"], 2.0);
}

#[test]
fn what_is_in_the_air_comes_off_on_top_of_the_ledger_and_not_instead_of_it() {
    // Two halves of one figure and neither replaces the other: $1.50 booked by a job that
    // closed, $1.00 announced by one that has not.
    let f = Fixture::new();
    f.calls(1);
    f.pattern("full", r#"{}"#);
    f.db().add_spend(&graphify::now()[..10], 1, 1.5).unwrap();
    f.job("running", 1, &graphify::now(), "SPENT 1.000000");

    f.reading(5.0);

    assert_eq!(request(f.dir.path())["max_usd"], 2.5);
}

#[test]
fn a_day_whose_cap_is_in_the_air_starts_nothing_and_says_so() {
    // The whole cap is committed to a run that has not closed. No process is started, and
    // the refusal does not call it spent — it has not been booked, and a run that fails
    // before it books gives the day its room back.
    let f = Fixture::new();
    f.calls(1);
    f.pattern("full", r#"{}"#);
    f.job("running", 1, &graphify::now(), "SPENT 5.000000");

    let report = f.daily(NO_BRAIN, 5.0).unwrap();

    assert_eq!(report.job, None);
    let note = report.note.as_deref().unwrap();
    assert!(note.contains("$5.0000 on jobs that have not closed"), "{note}");
    assert!(note.contains("$0.0000 booked"), "{note}");
}

// The four below ask `spend_in_flight` directly rather than through a daily run. What they
// are about is which rows the figure is made of, and a shell brain in the middle would only
// turn every one of those answers into the same subtraction.

#[test]
fn another_orgs_live_job_does_not_move_this_ones_cap() {
    // The cap is per-org and so is the ledger. A neighbour spending its own money is not
    // this org's morning getting shorter.
    let f = Fixture::new();
    f.job("running", 1, &graphify::now(), "SPENT 1.000000");
    f.job("running", 2, &graphify::now(), IN_FLIGHT);

    assert_eq!(in_flight(&f, 1), 1.0);
    assert_eq!(in_flight(&f, 2), 3.0);
}

#[test]
fn a_live_job_from_another_day_does_not_move_todays_cap() {
    // `RUN_LIMIT` is ten minutes, so a row still claiming to be running months later is a
    // corpse of a process that died. Its money was spent on a day this cap does not
    // govern, and counting it would shrink every cap until a server boots and sweeps it.
    let f = Fixture::new();
    f.job("running", 1, &graphify::now(), "SPENT 1.000000");
    f.job("running", 1, LONG_AGO, IN_FLIGHT);

    assert_eq!(in_flight(&f, 1), 1.0);
}

#[test]
fn a_job_that_has_closed_is_counted_by_the_ledger_and_not_again_by_its_log() {
    // A closed job keeps every `SPENT` line it printed. The ledger is the record for it,
    // and reading both would charge the day twice for one run.
    let f = Fixture::new();
    f.job("running", 1, &graphify::now(), "SPENT 1.000000");
    f.job("done", 1, &graphify::now(), IN_FLIGHT);

    assert_eq!(in_flight(&f, 1), 1.0);
}

#[test]
fn a_parked_quote_is_not_money_and_does_not_move_the_cap() {
    // A waiting job has been priced and has read nothing: the go is what it is parked in
    // front of, and it may never come. A quote is not a spend.
    let f = Fixture::new();
    f.job("running", 1, &graphify::now(), "SPENT 1.000000");
    f.job("waiting", 1, &graphify::now(), "ESTIMATE 4.0000");

    assert_eq!(in_flight(&f, 1), 1.0);
}
