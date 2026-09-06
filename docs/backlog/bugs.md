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
