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
find. · Fixed by S-36.
