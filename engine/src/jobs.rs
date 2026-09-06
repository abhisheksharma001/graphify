//! Running the brain. One brain function, one subprocess, one `jobs` row.
//!
//! The contract is the one in `docs/spec.md`: `graphify-brain <fn> --db PATH` reads JSON
//! on stdin, writes JSON on stdout, exits 0 or 1, and reports progress on stderr as
//! `PROGRESS n/m`. This module is the only thing in the engine that starts a process, and
//! three rules shape it.
//!
//! **Keys go in the environment, never in an argument.** An argv is readable by every
//! process on the box; an environment is the child's own. `Secret::expose` appears once in
//! this file, on the line that sets the variable, and nowhere else.
//!
//! **Nothing is spent until somebody says so.** A labelling job prints its price and then
//! blocks on `GO`. The engine parks the child there — alive, with its stdin still open,
//! having read nothing — and the only thing that writes `GO` into it is a
//! `POST /api/jobs/{id}/go`. The engine never passes `--yes`: the word that starts the
//! spending travels from the click to the child's stdin, and can be followed by hand.
//!
//! **What the brain printed is what gets stored** — with one exception. The output column
//! holds the line the brain wrote and the log holds its stderr as it arrived, except that
//! any key this module handed the child is replaced by `***` on the way in. A job's log is
//! read back by the browser, and a traceback out of an HTTP library is a plausible way for
//! a key to end up in one. The engine knows exactly which strings it passed, so it does not
//! have to guess at what a key looks like.

use crate::db::Db;
use crate::secrets::{self, Secret, Secrets};
use anyhow::{anyhow, bail, Context, Result};
use serde_json::Value;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStderr, Command, Stdio};
use std::sync::mpsc::{sync_channel, SyncSender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::Duration;

/// The job is alive and its subprocess is working.
pub const RUNNING: &str = "running";
/// The price has been printed and the child is blocked on its `GO`. Nothing has been read
/// and nothing has been spent.
pub const WAITING: &str = "waiting";
pub const DONE: &str = "done";
pub const FAILED: &str = "failed";
/// Nobody approved the price in time, so the child was killed unspent. Not a failure:
/// there is nothing wrong with walking away from a quote.
pub const EXPIRED: &str = "expired";

/// The brain binary when `GRAPHIFY_BRAIN` does not name one. Found on `PATH`, which is
/// where `uv sync` and `pip install` both put it.
pub const DEFAULT_BIN: &str = "graphify-brain";

/// Which brain to spawn, as the environment names it. Both callers read it once and keep
/// the answer — the server at boot, `sync` at the top of a run — so a variable that
/// changes under a running process cannot point half its children somewhere else.
pub fn binary_from_env() -> String {
    std::env::var("GRAPHIFY_BRAIN")
        .ok()
        .filter(|b| !b.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_BIN.to_string())
}

/// How long a job may sit at its price before the child is killed. Long enough for an
/// analyst to read a plan table and think about it; short enough that a wizard abandoned
/// at lunchtime is not still holding an interpreter at five.
const GO_WAIT: Duration = Duration::from_secs(30 * 60);

/// How many jobs may hold a subprocess at once. A waiting job holds a parked interpreter,
/// so a button pressed ten times is refused rather than answered with ten of them.
pub const MAX_LIVE: i64 = 4;

/// Ceiling on one job's log. A traceback is worth keeping; a child looping on stderr is
/// not worth the disk. Past this the pipe is still drained — a child blocked on a full
/// stderr is a child that never exits — and the lines are dropped instead of stored.
const LOG_BYTES: usize = 64 * 1024;

/// Ceiling on one line of it, so a single enormous line cannot get past the one above.
const LOG_LINE_CHARS: usize = 4_000;

const ESTIMATE: &str = "ESTIMATE ";
const PROGRESS: &str = "PROGRESS ";

/// The brain functions the API can start. The name is the subcommand, and it is also what
/// lands in `jobs.kind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Plan,
    Clarify,
    Label,
    Synthesize,
    /// The one function nobody starts by clicking. `sync` runs it, and D-8's two caps
    /// stand where the click stands everywhere else.
    Daily,
    /// One free-form question. Started by a click like the rest, but with the price
    /// already agreed: `ask::quote` shows it without spawning anything, so the approval
    /// happens before this row exists rather than against a parked child.
    Ask,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Plan => "plan",
            Kind::Clarify => "clarify",
            Kind::Label => "label",
            Kind::Synthesize => "synthesize",
            Kind::Daily => "daily",
            Kind::Ask => "ask",
        }
    }

    /// Only labelling parks. `synthesize` prints a price too, but it is one call over
    /// quotes already paid for, following a click the analyst has already made.
    fn waits_for_go(self) -> bool {
        self == Kind::Label
    }
}

/// What a parked job was told. There are two ways for a person to answer a price, and
/// the engine has always carried only one of them.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Verdict {
    /// Go. Read the calls.
    Go,
    /// No. Nothing has been read, and nothing will be.
    No,
}

/// The jobs that are parked on their go, by id.
///
/// Only the live ones: a job's row says what happened to it, and this says which of them
/// a `POST /go` can still reach. Empty after a restart, which is correct — the children
/// died with the engine, and their rows are `waiting` against a process that is gone.
pub struct Jobs {
    waiting: Mutex<HashMap<i64, SyncSender<Verdict>>>,
    /// How long a parked job waits. A field rather than a constant so a test can watch a
    /// job expire without sitting through half an hour of it.
    wait: Duration,
}

impl Default for Jobs {
    fn default() -> Self {
        Jobs {
            waiting: Mutex::new(HashMap::new()),
            wait: GO_WAIT,
        }
    }
}

impl Jobs {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn waiting_for(wait: Duration) -> Self {
        Jobs {
            wait,
            ..Self::default()
        }
    }

    /// Send one job its go. `false` means nothing was parked under that id: a job that has
    /// finished, one that expired, one that was turned down, one of a kind that never
    /// waits, or the second of two clicks on the same button.
    pub fn go(&self, id: i64) -> bool {
        self.tell(id, Verdict::Go)
    }

    /// Turn one job's price down. Same `false` for the same reasons, and a job that has
    /// already been told to go is one of them: the go and the no are the same decision
    /// asked twice, so the second of them finds an empty map either way.
    pub fn stop(&self, id: i64) -> bool {
        self.tell(id, Verdict::No)
    }

    fn tell(&self, id: i64, verdict: Verdict) -> bool {
        let sender = self.lock().remove(&id);
        // The removal and the send are the same decision: whoever takes the sender out of
        // the map is the one call that gets to answer this job, so a second click finds
        // nothing rather than a second answer.
        sender.is_some_and(|tx| tx.send(verdict).is_ok())
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<i64, SyncSender<Verdict>>> {
        self.waiting.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Everything one child needs, gathered before the thread starts so nothing it does
/// depends on a lock the caller is still holding.
struct Spawn {
    kind: Kind,
    binary: String,
    db_path: PathBuf,
    /// `(variable, key)`, ready to set on the child. Still `Secret`s: the value is exposed
    /// on one line of `command`, and on no other line in the engine.
    keys: Vec<(&'static str, Secret)>,
    /// The request, as one line of JSON. Forwarded to the brain unchanged: the engine does
    /// not know what a `plan` looks like and has no business editing one.
    body: String,
    /// Which org pays for this. `jobs` has no org column, so it also goes into `input`.
    org: i64,
    redact: Redact,
}

/// The exact key strings that went into the child's environment, so that nothing coming
/// back out can carry one into the log or the browser.
///
/// Exact values rather than a pattern for what a key looks like: a guess at the shape of
/// an API key is a guess, and provider prefixes change. What the engine passed in it can
/// recognise on the way out with certainty.
#[derive(Clone, Default)]
struct Redact(Vec<String>);

impl Redact {
    /// Values short enough that replacing them would mangle ordinary text are left alone.
    /// A key that short is not a key.
    const SHORTEST: usize = 8;

    fn new(keys: &[(&'static str, Secret)]) -> Self {
        Redact(
            keys.iter()
                .map(|(_, key)| key.expose().to_string())
                .filter(|k| k.len() >= Self::SHORTEST)
                .collect(),
        )
    }

    fn scrub(&self, text: &str) -> String {
        let mut out = text.to_string();
        for key in &self.0 {
            if out.contains(key.as_str()) {
                out = out.replace(key.as_str(), "***");
            }
        }
        out
    }
}

/// Start one brain function and return the new job's id. Returns as soon as the row is
/// written; the work happens on a thread of its own.
pub fn start(
    jobs: &Arc<Jobs>,
    db: &Arc<Mutex<Db>>,
    secrets: &Secrets,
    binary: &str,
    kind: Kind,
    org: i64,
    request: &Value,
) -> Result<i64> {
    let (id, spawn) = begin(db, secrets, binary, kind, org, request)?;
    let jobs = Arc::clone(jobs);
    let db = Arc::clone(db);
    thread::spawn(move || supervise(&jobs, &db, id, spawn));
    Ok(id)
}

/// Run one brain function on this thread, and return its id once the row is closed out.
///
/// `start` is for the server, where an HTTP request has to be answered while the child is
/// still working. A `sync` at six in the morning has nobody waiting on an answer, and a
/// command that returned before its child had read anything would exit and take the child
/// down with it.
///
/// No `Jobs` map: this is only ever used for a kind that does not park, so there is
/// nothing for a `POST /go` to reach and nothing to put in one.
pub fn run_blocking(
    db: &Arc<Mutex<Db>>,
    secrets: &Secrets,
    binary: &str,
    kind: Kind,
    org: i64,
    request: &Value,
) -> Result<i64> {
    let (id, spawn) = begin(db, secrets, binary, kind, org, request)?;
    supervise(&Jobs::new(), db, id, spawn);
    Ok(id)
}

/// Check the request, gather the keys, and write the `jobs` row. Everything both callers
/// do before they differ over which thread the child runs on.
fn begin(
    db: &Arc<Mutex<Db>>,
    secrets: &Secrets,
    binary: &str,
    kind: Kind,
    org: i64,
    request: &Value,
) -> Result<(i64, Spawn)> {
    let body = serde_json::to_string(request)?;
    if body.contains('\n') {
        // The brain reads its request as one line. Serialising a `Value` never produces a
        // newline, so this is a guard on a thing that cannot happen rather than a check on
        // the caller — and if it ever does, a truncated request is worth refusing.
        bail!("a job request has to be one line of JSON");
    }

    let db = lock(db);
    let mut keys = Vec::new();
    for name in secrets::GLOBAL_NAMES {
        let var = secrets::env_var(name)
            .ok_or_else(|| anyhow!("no environment variable is defined for {name}"))?;
        if let Some(key) = secrets.get(&db, None, name)? {
            keys.push((var, key));
        }
    }
    let input = serde_json::json!({ "org": org, "body": request });
    let id = db.create_job(kind.as_str(), RUNNING, &input.to_string(), &crate::now())?;
    let spawn = Spawn {
        kind,
        binary: binary.to_string(),
        db_path: db.path().to_path_buf(),
        redact: Redact::new(&keys),
        keys,
        body,
        org,
    };
    Ok((id, spawn))
}

/// The last `PROGRESS n/m` the brain reported, read back out of the log.
///
/// Parsed rather than stored in a column of its own, so there is one place progress lives
/// and no second copy to fall out of step with the lines the job actually printed.
pub fn progress(log: &str) -> Option<(u64, u64)> {
    log.lines()
        .rev()
        .find_map(|line| line.trim().strip_prefix(PROGRESS).and_then(split_fraction))
}

fn split_fraction(rest: &str) -> Option<(u64, u64)> {
    let (done, of) = rest.trim().split_once('/')?;
    Some((done.trim().parse().ok()?, of.trim().parse().ok()?))
}

/// The price the brain quoted, read back the same way. `None` for a function that does not
/// quote one, for a job that died before it got that far, and for a line that begins like
/// a quote and does not carry one.
pub fn estimate(log: &str) -> Option<f64> {
    log.lines()
        .rev()
        .find_map(|line| price(line.trim().strip_prefix(ESTIMATE)?))
}

/// The number on an `ESTIMATE` line, if it is one that can be shown to someone.
///
/// `f64` will read `nan` and `inf` out of a string quite happily, and serde_json writes
/// both as `null`, so a quote that parses is not yet a quote the browser can price. A
/// negative one is worse than either: it parses, it serialises, and it reaches the go
/// button looking like money. This is the only place either question is asked, so that
/// what the supervisor parks on and what the API answers with are the same judgement.
fn price(rest: &str) -> Option<f64> {
    let usd: f64 = rest.trim().parse().ok()?;
    (usd.is_finite() && usd >= 0.0).then_some(usd)
}

// --- the supervisor -------------------------------------------------------------------

/// What came back from the conversation with the child, before its exit status is known.
enum Outcome {
    /// It ran to the end of its stdout. The string is the last non-empty line, which is
    /// where the brain puts its result.
    Ran(Option<String>),
    /// Nobody approved the price in time, so the child was killed.
    Expired,
    /// Somebody looked at the price and said no, so the child was killed. The same cost as
    /// `Expired` — nothing — reached faster and on purpose.
    Declined,
}

/// Own one child from spawn to row. Runs on its own thread; every failure here ends as a
/// `failed` job carrying the reason, because a thread has nowhere else to put one.
fn supervise(jobs: &Jobs, db: &Arc<Mutex<Db>>, id: i64, spawn: Spawn) {
    let mut child = match command(&spawn).spawn() {
        Ok(child) => child,
        Err(e) => {
            // The common one, and worth naming precisely: the brain is not installed, or
            // `GRAPHIFY_BRAIN` points at something that is not there.
            let said = format!("could not start {}: {e}", spawn.binary);
            finish(db, id, FAILED, None, 0.0, spawn.org, &said);
            return;
        }
    };

    let outcome = converse(jobs, db, id, &spawn, &mut child);
    if outcome.is_err() {
        let _ = child.kill();
    }
    // Reaped whatever happened: a child nobody waits on is a zombie, including the one
    // just killed for expiring.
    let status = child.wait();

    match outcome {
        Err(e) => finish(db, id, FAILED, None, 0.0, spawn.org, &format!("{e:#}")),
        Ok(Outcome::Expired) => finish(
            db,
            id,
            EXPIRED,
            None,
            0.0,
            spawn.org,
            "nobody approved the price in time, so this was stopped before it read anything",
        ),
        // `expired` and not a status of its own: the row already means killed unspent, and
        // a second word for that would differ only in how fast it arrived. The reason line
        // is where the difference belongs.
        Ok(Outcome::Declined) => finish(
            db,
            id,
            EXPIRED,
            None,
            0.0,
            spawn.org,
            "the price was turned down, so this was stopped before it read anything",
        ),
        Ok(Outcome::Ran(last)) => {
            let ok = matches!(status, Ok(s) if s.success());
            classify(db, id, &spawn, ok, last.as_deref())
        }
    }
}

fn command(spawn: &Spawn) -> Command {
    let mut cmd = Command::new(&spawn.binary);
    cmd.arg(spawn.kind.as_str())
        .arg("--db")
        .arg(&spawn.db_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (var, key) in &spawn.keys {
        // The one place a key leaves the store. It goes into the child's environment and
        // never onto the command line above: `ps` shows an argv to everyone on the box.
        cmd.env(var, key.expose());
    }
    cmd
}

/// Write the request, read the answer, and stop at the price if this kind stops there.
fn converse(
    jobs: &Jobs,
    db: &Arc<Mutex<Db>>,
    id: i64,
    spawn: &Spawn,
    child: &mut Child,
) -> Result<Outcome> {
    let mut stdin = child.stdin.take().context("the child has no stdin")?;
    let stdout = child.stdout.take().context("the child has no stdout")?;
    let stderr = child.stderr.take().context("the child has no stderr")?;

    // stderr gets a thread of its own. `PROGRESS` lines and tracebacks both come down it,
    // and a supervisor blocked on stdout while the child fills its stderr pipe is a
    // deadlock with both sides politely waiting.
    let log = Arc::clone(db);
    let redact = spawn.redact.clone();
    let logger = thread::spawn(move || drain(&log, id, stderr, &redact));

    writeln!(stdin, "{}", spawn.body).context("writing the request to the brain")?;
    stdin.flush()?;
    // Everything but a labelling job has said all it is going to say. Closing stdin is how
    // `plan` and `clarify`, which read to end-of-file, learn that.
    let mut stdin = spawn.kind.waits_for_go().then_some(stdin);

    let mut last = None;
    for line in BufReader::new(stdout).lines() {
        let line = line.context("reading the brain's answer")?;
        if let Some(rest) = line.strip_prefix(ESTIMATE) {
            // Read before anything is written or parked on. A quote nobody can read back
            // is not a shown cost, and the go is what it stands in front of.
            price(rest).with_context(|| {
                // Scrubbed and cut short, because this reason is appended to the job's log
                // by `finish` and the text it quotes was written by something holding the
                // keys. Whatever the brain printed here, it was not a price, so there is
                // no telling what it was.
                let said: String = spawn
                    .redact
                    .scrub(rest.trim())
                    .chars()
                    .take(LOG_LINE_CHARS)
                    .collect();
                format!("the brain quoted {said:?}, which is not a price")
            })?;
            // The price goes in the log rather than a column: it is something the brain
            // said, and `estimate` reads it back from there. Which is why this one write
            // is checked where `append` does not bother — if the quote is not on record,
            // the job that would park on it has nothing to show.
            lock(db)
                .append_job_log(id, &spawn.redact.scrub(&line))
                .context("writing the brain's quote to the job's log")?;
            if let Some(stdin) = stdin.take() {
                match park(jobs, db, id)? {
                    Some(Verdict::Go) => go(stdin)?,
                    // Both of these drop `stdin` unwritten and return, and `supervise`
                    // kills the child on the way out. It is holding a request it has read
                    // and a price it has quoted, and it has read no call to make either of
                    // them cost anything.
                    Some(Verdict::No) => return Ok(Outcome::Declined),
                    None => return Ok(Outcome::Expired),
                }
            }
            continue;
        }
        if !line.trim().is_empty() {
            last = Some(line);
        }
    }
    let _ = logger.join();
    Ok(Outcome::Ran(last))
}

/// Block until this job is told to go, or until nobody has told it for [`GO_WAIT`].
fn park(jobs: &Jobs, db: &Arc<Mutex<Db>>, id: i64) -> Result<Option<Verdict>> {
    let (tx, rx) = sync_channel(1);
    jobs.lock().insert(id, tx);
    lock(db).set_job_status(id, WAITING)?;

    let mut said = rx.recv_timeout(jobs.wait).ok();
    if said.is_none() {
        // Take the sender back before giving up, then look once more. An answer that
        // arrived while the wait was timing out took the sender out of the map under that
        // lock and put its verdict in the channel, and it would be a strange thing to drop.
        jobs.lock().remove(&id);
        said = rx.try_recv().ok();
    }
    // Only a go changes the row. A no leaves it `waiting` for the moment it takes
    // `supervise` to write `expired` over it, which is the same moment a timeout takes.
    if said == Some(Verdict::Go) {
        lock(db).set_job_status(id, RUNNING)?;
    }
    Ok(said)
}

/// Say the word, then close stdin so the child knows there is no more of it coming.
fn go(mut stdin: std::process::ChildStdin) -> Result<()> {
    writeln!(stdin, "GO").context("sending the go to the brain")?;
    stdin.flush()?;
    Ok(())
}

/// Copy the child's stderr into its log, line by line, while it is still running.
fn drain(db: &Arc<Mutex<Db>>, id: i64, stderr: ChildStderr, redact: &Redact) {
    let mut written = 0usize;
    for line in BufReader::new(stderr).lines() {
        let Ok(line) = line else { return };
        // Past the cap the pipe is still read and the line is dropped. Stopping the read
        // instead would fill the pipe and block the child for ever.
        if written >= LOG_BYTES {
            continue;
        }
        let line: String = redact.scrub(&line).chars().take(LOG_LINE_CHARS).collect();
        written += line.len() + 1;
        append(db, id, &line);
    }
}

/// Turn an exit status and a last line into a finished row.
fn classify(db: &Arc<Mutex<Db>>, id: i64, spawn: &Spawn, ok: bool, last: Option<&str>) {
    // Scrubbed here as well as in the log: a result the brain printed goes into a column
    // the browser reads, and it was written by something that had the keys.
    let last = last.map(|text| spawn.redact.scrub(text));
    let last = last.as_deref();
    if !ok {
        // The brain's own complaint is already in the log — this is the stderr it wrote on
        // the way down — so there is nothing to add but the verdict.
        finish(db, id, FAILED, last, 0.0, spawn.org, "");
        return;
    }
    let Some(text) = last else {
        finish(
            db,
            id,
            FAILED,
            None,
            0.0,
            spawn.org,
            "the brain exited cleanly and printed no result",
        );
        return;
    };
    match serde_json::from_str::<Value>(text) {
        // `usd` is what the brain says it spent, and the two functions that spend both
        // report it. `plan` and `clarify` do not, and are unmetered until they do.
        Ok(value) => {
            let usd = value.get("usd").and_then(Value::as_f64).unwrap_or(0.0);
            finish(db, id, DONE, Some(text), usd, spawn.org, "");
        }
        Err(e) => finish(
            db,
            id,
            FAILED,
            Some(text),
            0.0,
            spawn.org,
            &format!("the brain exited cleanly and its last line was not JSON: {e}"),
        ),
    }
}

/// Close the row out and book what it cost. The spend is written before the status, so a
/// job that reads `done` is a job whose cost has already been counted against its org.
fn finish(
    db: &Arc<Mutex<Db>>,
    id: i64,
    status: &str,
    output: Option<&str>,
    usd: f64,
    org: i64,
    note: &str,
) {
    let db = lock(db);
    if !note.is_empty() {
        let _ = db.append_job_log(id, note);
    }
    if usd > 0.0 {
        let _ = db.add_spend(&crate::now()[..10], org, usd);
    }
    let _ = db.finish_job(id, status, output, usd, &crate::now());
}

fn append(db: &Arc<Mutex<Db>>, id: i64, line: &str) {
    let _ = lock(db).append_job_log(id, line);
}

/// A poisoned lock means another handler panicked mid-statement, which SQLite survives —
/// the same reasoning `server::App::db` gives.
fn lock(db: &Mutex<Db>) -> MutexGuard<'_, Db> {
    db.lock().unwrap_or_else(|e| e.into_inner())
}
