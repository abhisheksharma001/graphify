# Bug log

Format per entry: date · where seen · what was seen · how to reproduce · step that fixes it.

2026-09-06 · `ui/src/patterns/Wizard.tsx:544` · A parked labelling job whose price never
reached its log draws the spend button as `Read 25 calls · up to $0.00`, and the click
behind that button is the go. `money` renders NULL as `—` correctly and S-35 has three
tests saying so; the call site passes `estimate_usd ?? 0`, so the formatter never sees
the null. The button's `disabled` says nothing about whether a price is known. ·
Reproduce: answer `GET /api/jobs/{id}` with `status: "waiting"` and
`estimate_usd: null`. The engine reaches that state whenever `append_job_log` fails,
because `append` (`engine/src/jobs.rs:545`) discards the write error and `park` runs
regardless — the job is parked and waiting with no `ESTIMATE` line for `estimate()` to
find. · Fixed by S-36 (PR #37, 0d94d91).

2026-09-06 · `engine/src/jobs.rs:418` · The engine parks a labelling job on any line that
starts with `ESTIMATE `, without ever reading the number off it. The price is parsed much
later and somewhere else — `estimate()` reads it back out of the log when the browser asks
— so a line that begins right and ends wrong parks a job that no one can price. Four ways
in, and only the first needs anything to fail: a discarded `append_job_log` error (logged
above, fixed in the browser by S-36 and still open here); `ESTIMATE abc`, which does not
parse; `ESTIMATE nan` or `ESTIMATE inf`, which parse to non-finite floats that serde_json
writes as `null`; and `ESTIMATE -5`, which parses fine and puts a negative price on the go
button. · Reproduce: a brain that prints any of those four lines and then waits. Verified:
`"nan".parse::<f64>()` is `Ok(NaN)`, `json!({"estimate_usd": Some(f64::NAN)})` is
`{"estimate_usd":null}`, and `"-5".parse::<f64>()` is `Ok(-5.0)`. · Fixed by S-37
(PR #38, dcb266d), except the discarded write error, which is now checked at the quote and
still discarded in `drain` where a dropped stderr line is all it costs.

2026-09-06 · `engine/src/server.rs:670` · The engine refuses a fifth job with *"4 jobs are
already running or waiting for a go; finish or abandon one first"*, and there is no way to
abandon one. A parked labelling job holds its slot until `GO_WAIT` expires it, which is
thirty minutes (`engine/src/jobs.rs:66`), and the only inputs the wizard offers are the go
and the back of the browser. So the message names a remedy the product does not have, and
`ui/src/patterns/Wizard.tsx:558` tells the analyst the same thing more plainly: *"the
engine drops it within the half hour."* Four abandoned quotes — four closed tabs — and
labelling is refused for up to half an hour, having read nothing and spent nothing. ·
Reproduce: price a run in the wizard, close the tab without clicking the go, four times.
The fifth `POST /api/patterns/label` answers 429. · Fixed by S-38 (PR #39).

2026-09-06 · `engine/src/jobs.rs:615` · `finish` books a job's cost with `let _ =
db.add_spend(...)` and then writes the job's status regardless. Its own doc comment states
the invariant the ordering was written for — *"the spend is written before the status, so
a job that reads `done` is a job whose cost has already been counted against its org"* —
and the discarded error is what breaks it: a failed `add_spend` still reaches
`finish_job`, so the row says `done` and carries `cost_usd`, and the ledger never hears
about it. That one line is the only writer of the `spend` table, and `sync.rs:222` — `let
left = opts.cap_usd - db.spend_on(&now()[..10], org.id)?;` — is the only reader the daily
cap has. So a lost spend is not a lost figure on a report: every later run that day
computes its remaining budget from a ledger that is short, and the hard USD cap is
exceeded by exactly the money that was already spent, silently, with the row that spent it
reading `done`. The same `let _ =` covers `finish_job`, so a job whose close fails is left
`running` and keeps holding one of `MAX_LIVE` slots. · Reproduce: make the `spend` insert
fail — `CREATE TRIGGER t BEFORE INSERT ON spend BEGIN SELECT RAISE(ABORT, 'x'); END;` —
and run a labelling job through to `done`. The job row shows a cost; `spend_on` returns
0.0. · Second Must-never: *"Daily modes have a hard USD cap and stop when reached."* · Fixed by S-39 (PR #40).

2026-09-06 · `engine/src/jobs.rs:177` · `tell` takes a parked job's sender out of the map
and sends it the verdict, and its own comment states an invariant the code does not hold —
*"the removal and the send are the same decision"*. They are two. `self.lock().remove(&id)`
drops the guard at the end of that statement, so the send happens with the map unlocked.
`park` gives up on a timeout by taking the sender back and looking in the channel once more,
and that salvage runs under the same lock — so a click that has removed the sender but not
yet sent leaves `park` a map with nothing in it and a channel with nothing in it either.
`park` returns `None`, `supervise` writes `expired` and kills the child, and `go_job` has
already answered 200 `{"status": "running"}` (`engine/src/server.rs:891`). The analyst is
told the run started and it never does. Nothing is read and nothing is spent, so the money
is safe; what is lost is the answer to a question the product said yes to. · Reproduce: not
by hand — the window is the handful of instructions between `MutexGuard::drop` and
`SyncSender::send`. Under contention it is reachable: park a job, race a `go` against a
reader that takes the lock the instant it is free, and the reader can see the map without
the job while the channel is still empty. · Fixed by S-43 (PR #44, 45fec76).

2026-09-10 · `engine/src/cli.rs:181` · `Command::Schedule` destructures its `print` flag as
`print: _` and never reads it, so `--print` is not a flag — it is the absence of
`--install`. The help for it says *"Print both and write nothing. What happens anyway with
no flags."* `graphify schedule --print --install` is therefore byte-identical to `graphify
schedule --install` — measured under S-53, 1,478 bytes each and an empty `diff`. It does not
print and then install: the crontab line is never printed at all, and what happens instead is
the offer to write `~/Library/LaunchAgents/ai.graphify.daily.plist` and load it, or to replace
a line in the user's crontab, having been told in the same breath to write nothing. The confirm
prompt is still asked, so nothing lands without a `y`, which is what keeps this small; what
is wrong is that a flag documented as "write nothing" does not prevent a write, and the two
flags are silently resolved in favour of the destructive one. · Reproduce: `graphify
schedule --print --install` and answer `y`. · Fix shape: clap `conflicts_with`, so the pair
is refused at parse time and neither flag has to win. Found while auditing `schedule.rs`
for S-52; out of that step's scope because it is a `cli.rs` defect. · Fixed by S-53 (PR #54,
8849cd9): clap refuses the pair at parse time.

2026-09-17 · `engine/tests/jobs.rs:250` · Six tests in this file wait on a labelling job
reaching `waiting`, and the brain they wait on is a shell script given a six-second silence
budget by the deadline the test sets. `cargo test` runs the test binaries in parallel and
this one starts about a hundred servers and shells, so on a loaded machine the shell does
not get scheduled inside six seconds, the watchdog stops it, and the test times out thirty
seconds later on a job that reads `failed` — *"the brain said nothing for 6s and was
stopped"*. Nothing about the product is wrong when this happens: the run under test is one
the engine correctly gave up on. · Reproduce: `cargo test -q` in `engine/` on a busy
machine, repeatedly. Measured over eleven whole-suite runs while shipping S-66, on `main`
and on the step's branch alike: four of them failed this way, two to four tests each, never
the same set twice, and every one of those tests passed on its own
(`cargo test -q --test jobs`, 102 passed). CI has not hit it. · Not a load problem this
suite can be spared by trimming — the budget is the thing that is wrong, not the number of
tests. Fix shape: the silence budget these tests set is a product clock borrowed for a
test, and the test wants "the child did not answer" rather than "the child did not answer
within six seconds of wall clock on a machine doing something else". · Found while
verifying S-66, and out of its scope: it is in `tests/jobs.rs` and S-66 is the daily cap. ·
Fixed by S-67 (PR #68, a2d1ab1): `parked` reads the row instead of panicking on it, and a job
that ended without ever quoting is started again rather than reported. Measured at the engine
first: `Command::spawn` returns in under a millisecond and the child's first word arrives at
a median of 2.2-4.0s, p90 up to 6.0s, worst seen 6.8s, against the 6s budget.

2026-09-21 · `brain/src/graphify_brain/label.py:362` · `brain/baml_src/label.baml` instructs
the model *"Never quote a line that is not in the transcript you were given"* and nothing
holds it to it. `synth.py:476` checks that `evidence` is a `str` and stops there; `_attach`
writes whatever came back straight into `pattern_labels`. An invented quote then reaches
three readers that each treat it as what a person said: the row in `pattern_labels`, the
call drawer (`engine/src/queries.rs:342` selects `l.evidence` into `CallRow`), and — the
expensive one — `SynthesizeRule`, whose prompt tells it to *"work from the evidence, not
from the criterion"* and that the quotes are *"what people actually said"*. A sentence
nobody said can therefore shape a rule that runs unattended on every call forever. ·
Reproduce: answer a batch with a `Label` whose `evidence` appears nowhere in that call's
transcript. It is stored, served and synthesised from. No test covered it. · Found while
reading the labelling path to assess Jev (`docs/prd-jev.md`, §10), and independent of it. ·
Fixed by S-68 (PR #69, 11595fb): the check goes in `_attach`, the one place holding both
a quote and the transcript it claims to come from, and what is not there is replaced
rather than dropped — the judgement was paid for and is not what was in doubt.
