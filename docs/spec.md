# graphify — feature spec v1 (Vapi only)

## What a person sees today
Vapi's dashboard: a call list, one call at a time. To know "how many calls ended in a
failed transfer this week" or "how often does a caller ask for a human" you open calls
one by one, or pay Roark / Hamming / Sherlock. Nothing open-source does this for Vapi.

## What they must see instead
One local web page. Paste `VAPI_API_KEY`, run `graphify sync --last 250`, open
`graphify serve`. Charts: calls over time, how calls ended (grouped), tool-call
failures, transfers, latency. Filter by time window (1d / 5h / 7h / custom), by
"last N calls", by date, by assistant, by call ID. Click a call, see transcript and
every tool call with its result.

Then a **pattern**: type "calls where the caller asks for a human". graphify sends a
sample of calls to the model you picked (Opus / Sonnet / GPT), gets back a rule
(phrases, regex, endedReason codes, tool conditions) plus a label per sample call. It
checks the rule against those labels, shows agreement, saves the rule. From then on
every `sync` re-counts the pattern over all calls with no model call. A daily cron
keeps it fresh. Data older than 14 days is purged.

## Acceptance sentence
WHEN a user sets `VAPI_API_KEY`, runs `graphify sync --last 250` then `graphify serve`,
THEN within 2 minutes the browser SHALL show why each of those calls ended and which
tool calls failed, AND WHEN they add a pattern in plain English THEN graphify SHALL
show its count over the same calls and re-count it on the next sync without a model
call.

## Decisions
- **D-1 Python + React, no Rust in v1.** The work is HTTP, JSON, text and one LLM call.
  Rust buys compile-time checks but triples iteration time and blocks reuse of the
  proven rush-audit code. Single-binary distribution is a v2 concern.
- **D-2 Patterns are data, not code.** The model returns a JSON rule spec; graphify
  runs it with its own rule engine. Never execute model-written Python. Reason: safety,
  and a weaker model can read and fix a JSON rule.
- **D-3 API pull, not webhook, in v1.** One API key is the whole setup. Webhook needs a
  public URL. Webhook ingestion is v2.
- **D-4 Vapi is read-only. LLM spend needs explicit go.**
- **D-5 endedReason is grouped**, not raw: customer · assistant · llm-error · tts-error ·
  stt-error · transfer-error · transport · timeout · start-error · other. Raw code kept.

## Deliberately does not do (v1)
- No scoring, evals, or "was this call good". Counts and charts only.
- No Retell / ElevenLabs / other providers. Table has a `provider` column set to `vapi`.
- No webhook receiver. No auth. No multi-tenant. No cloud deploy.
- No recording download. No audio.
- No automatic rule refinement loop. One model pass, then the user edits the JSON.

## Data model (SQLite, `data/graphify.db`)
```
calls(id TEXT PK, provider TEXT, assistant_id TEXT, created_at TEXT, started_at TEXT,
      ended_at TEXT, duration_s REAL, ended_reason TEXT, ended_group TEXT, cost REAL,
      transferred INTEGER, tool_calls INTEGER, tool_failures INTEGER,
      transcript TEXT, recording_url TEXT, raw JSON, synced_at TEXT)
tool_calls(call_id TEXT, name TEXT, seconds_from_start REAL, failed INTEGER,
           result_excerpt TEXT)
patterns(id INTEGER PK, name TEXT, prompt TEXT, rule JSON, model TEXT,
         sample_size INTEGER, agreement REAL, created_at TEXT)
pattern_labels(pattern_id INTEGER, call_id TEXT, llm_match INTEGER, rule_match INTEGER,
               evidence TEXT)
pattern_matches(pattern_id INTEGER, call_id TEXT)
```

## Rule spec (what the model must return)
```json
{
  "any_phrases": ["speak to a person", "real human", "representative"],
  "regex": ["\\btalk to (a|an|the) (agent|human|person)\\b"],
  "ended_reasons": [],
  "ended_groups": [],
  "tool_failed": null,
  "transferred": null,
  "speaker": "user"
}
```
Match = any phrase OR any regex hits in transcript lines from `speaker` (user/bot/any)
AND every non-null structural condition holds. Empty list = no constraint.

## Repo layout (target)
```
pyproject.toml
src/graphify/__init__.py
src/graphify/cli.py           # typer app: sync, purge, serve, pattern, apply
src/graphify/vapi.py          # GET /call pagination, retries, read-only
src/graphify/db.py            # schema, upsert, queries
src/graphify/ended_reason.py  # code -> group
src/graphify/extract.py       # raw call -> row + tool_calls
src/graphify/server.py        # FastAPI, serves ui/dist
src/graphify/patterns/llm.py  # cost estimate, model call, JSON parse
src/graphify/patterns/rules.py# rule engine
tests/
ui/                           # Vite + React + TS + Recharts
docs/spec.md  docs/backlog/bugs.md
```

---

# Step register

Legend: ☐ todo · ☐→ in progress · ☑ done (with what was learned).

### S-1 — Python project scaffold with a `graphify` CLI that prints its version ☐

**PR:** one.
**Depends on:** nothing.
**Files:** `pyproject.toml`, `src/graphify/__init__.py`, `src/graphify/cli.py`, `tests/test_cli.py`.
**Today:** repo has docs only.
**Change:** `uv init --package` style project, Python 3.11, deps `typer`, `httpx`, `fastapi`,
`uvicorn`, dev dep `pytest`. `cli.py` is a typer app with one command `version` printing
`graphify 0.1.0`. Console script `graphify = "graphify.cli:app"`.
**Acceptance:** WHEN `uv run graphify version` runs THEN it SHALL print `graphify 0.1.0` and exit 0.
**Verify:** `uv sync && uv run graphify version && uv run pytest -q` → version line printed, 1 test passed.
**Must not:** touch `ui/`, call any network.

### S-2 — SQLite schema and `db.py` with upsert ☐

**PR:** one.
**Depends on:** S-1.
**Files:** `src/graphify/db.py`, `tests/test_db.py`.
**Today:** no storage.
**Change:** `db.connect(path)` creates tables from the data model above if missing
(`CREATE TABLE IF NOT EXISTS`). `db.upsert_calls(rows)` inserts or replaces by `id`.
`db.upsert_tool_calls(call_id, rows)` deletes then inserts for that call. Default path
`data/graphify.db`, overridable by `GRAPHIFY_DB`. Indexes on `created_at`, `assistant_id`, `ended_group`.
**Acceptance:** WHEN `upsert_calls` is called twice with the same `id` THEN the table SHALL hold one row with the second values.
**Verify:** `uv run pytest -q tests/test_db.py` → passes; test uses a tmp path.
**Must not:** touch Vapi, add an ORM.

### S-3 — `ended_reason.py`: map every Vapi endedReason to a group ☐

**PR:** one.
**Depends on:** S-1.
**Files:** `src/graphify/ended_reason.py`, `tests/test_ended_reason.py`.
**Today:** nothing.
**Change:** `group(code: str | None) -> str` using prefix/substring rules from
https://docs.vapi.ai/calls/call-ended-reason. Groups: `customer`, `assistant`, `llm-error`,
`tts-error`, `stt-error`, `transfer-error`, `transport`, `timeout`, `start-error`, `other`.
`None` → `unknown`. Rules, in order: `silence-timed-out|exceeded-max-duration` → timeout;
`call.start.error*|assistant-not-*|assistant-request-*` → start-error; `*transfer*` →
transfer-error; `*transcriber*|*-returning-4*|*-returning-5*` → stt-error;
`*voice*|*-out-of-credits|*quota*` → tts-error; `*llm*|*-4[0-9][0-9]-*|*-5[0-9][0-9]-*|pipeline-*`
→ llm-error; `*sip*|*twilio*|*vonage*|*transport*|*worker*|*websocket*` → transport;
`customer-*|voicemail` → customer; `assistant-*` → assistant; else other.
**Acceptance:** WHEN `group("call.in-progress.error-transfer-failed")` THEN it SHALL return `transfer-error`, AND WHEN `group(None)` THEN `unknown`.
**Verify:** `uv run pytest -q tests/test_ended_reason.py` with at least 12 cases across all groups → pass.
**Must not:** network.

### S-4 — `vapi.py`: read-only paginated fetch of the last N calls ☐

**PR:** one.
**Depends on:** S-1.
**Files:** `src/graphify/vapi.py`, `tests/test_vapi.py`.
**Today:** nothing.
**Change:** `fetch_calls(api_key, *, last=None, since=None, until=None, assistant_id=None)`
yields raw call dicts newest-first from `GET https://api.vapi.ai/call` with
`limit=100`, cursoring with `createdAtLt=<oldest createdAt of previous page>` (this is how
`~/rush-audit/fetch_new_calls.py` does it and it works). Stop when `last` reached, or
page shorter than limit, or `createdAt <= since`. Retry 429/5xx up to 5 times with
backoff. Header `Authorization: Bearer <key>`. Tests mock `httpx` with `respx` or a
transport stub; no real calls.
**Acceptance:** WHEN two mocked pages of 100 and 30 calls are served THEN `fetch_calls(last=250)` SHALL yield 130 calls and make exactly 2 requests.
**Verify:** `uv run pytest -q tests/test_vapi.py` → pass.
**Must not:** send any method other than GET; log the key.

### S-5 — `extract.py`: raw call → `calls` row + `tool_calls` rows ☐

**PR:** one.
**Depends on:** S-2, S-3.
**Files:** `src/graphify/extract.py`, `tests/test_extract.py`, `tests/fixtures/call_sample.json`.
**Today:** nothing.
**Change:** `extract(raw) -> (call_row, tool_rows)`. Fields: `duration_s` from
`endedAt - startedAt` (NULL if either missing), `ended_group` via S-3, `transferred` =
1 if `endedReason == "assistant-forwarded-call"` or any message `role == "transfer"`
or any tool call named like `transferCall`, `tool_calls` = count of
`artifact.messages[].role == "tool_calls"` entries, `tool_failures` = count of
`tool_call_result` messages whose result contains `"error"` (case-insensitive) or is empty,
`transcript` = `artifact.transcript`, `recording_url` = `artifact.recordingUrl` or
`artifact.recording.url`. Missing → NULL, never 0 (see CLAUDE.md "absent is not zero").
Fixture file: one anonymised real Vapi call payload (strip phone numbers).
**Acceptance:** WHEN the fixture call has 3 tool calls and 1 error result THEN `extract` SHALL return `tool_calls=3, tool_failures=1`, AND WHEN `endedAt` is missing THEN `duration_s` SHALL be NULL.
**Verify:** `uv run pytest -q tests/test_extract.py` → pass.
**Must not:** download recordings.

### S-6 — `graphify sync --last N | --since DATE`, incremental, then purge ☐

**PR:** one.
**Depends on:** S-4, S-5.
**Files:** `src/graphify/cli.py`, `src/graphify/sync.py`, `tests/test_sync.py`.
**Today:** CLI only prints version.
**Change:** `sync` reads `VAPI_API_KEY` (exit 2 with a plain message if unset), fetches
via S-4, extracts via S-5, upserts via S-2. Incremental: if `--last` and DB non-empty,
default `since` = newest `created_at` in DB, so re-runs add only new calls; `--last 500`
on a DB holding 250 fetches 250 more, never replaces. After upsert, delete rows with
`created_at` older than `GRAPHIFY_KEEP_DAYS` (default 14). Print
`synced N new, M total, purged P`.
**Acceptance:** WHEN `sync --last 250` runs twice against the same mocked API THEN the second run SHALL report `0 new` and the table SHALL still hold 250 rows.
**Verify:** `uv run pytest -q tests/test_sync.py` → pass. Then live: `VAPI_API_KEY=... uv run graphify sync --last 25` prints `synced 25 new, 25 total, purged 0`.
**Must not:** any non-GET request; delete rows newer than the keep window.

### S-7 — `graphify serve`: FastAPI with `/api/calls`, `/api/stats`, `/api/calls/{id}` ☐

**PR:** one.
**Depends on:** S-6.
**Files:** `src/graphify/server.py`, `src/graphify/queries.py`, `tests/test_server.py`.
**Today:** no API.
**Change:** Query params shared by `/api/calls` and `/api/stats`: `since`, `until` (ISO),
`window` (`1d|5h|7h|<n>h|<n>d`, overrides since), `last` (int), `assistant_id`,
`ended_group`, `call_id`, `tool_failed=1`, `transferred=1`. `/api/stats` returns
`{by_ended_group, by_ended_reason, calls_per_bucket, tool_failures_per_bucket,
transfers_per_bucket, latency_p50_p95_per_bucket, totals}`; bucket = 1h when window ≤ 2d
else 1d. `/api/calls/{id}` returns the row plus `raw` and its `tool_calls`. Serves
`ui/dist` at `/` if present. `serve` opens `http://127.0.0.1:3737`.
**Acceptance:** WHEN the DB holds 10 calls, 3 in `transfer-error`, and `/api/stats?window=1d` is called THEN `by_ended_group["transfer-error"]` SHALL equal 3.
**Verify:** `uv run pytest -q tests/test_server.py` (TestClient, seeded tmp DB) → pass; `uv run graphify serve` then `curl localhost:3737/api/stats` returns JSON.
**Must not:** bind to 0.0.0.0; call Vapi.

### S-8 — UI scaffold: Vite + React + TS + Recharts, one page, filter bar, four charts ☐

**PR:** one.
**Depends on:** S-7.
**Files:** `ui/` (new), `src/graphify/server.py` (static mount only).
**Today:** API only.
**Change:** `pnpm create vite ui --template react-ts`; add `recharts`. Load the
`dataviz` skill before writing chart code. Filter bar: window presets 1d / 5h / 7h,
custom since/until, "last N" (250 / 500 / custom), assistant select, ended group
select, call ID box. Charts: calls per bucket (bar), ended group (horizontal bar,
raw reason on hover), tool failures per bucket (line), transfers per bucket (line).
Every chart reads from `/api/stats` with the current filters. "—" for NULL.
**Acceptance:** WHEN `pnpm build` completes and `graphify serve` runs THEN `http://127.0.0.1:3737` SHALL render four charts that change when the window preset changes.
**Verify:** `cd ui && pnpm i && pnpm build` → exit 0; open the page, switch 1d → 7h, bucket count changes. Screenshot in PR.
**Must not:** add a UI framework beyond React; call Vapi from the browser.

### S-9 — Call table + call detail drawer ☐

**PR:** one.
**Depends on:** S-8.
**Files:** `ui/src/CallTable.tsx`, `ui/src/CallDrawer.tsx`.
**Today:** charts only.
**Change:** Table under the charts from `/api/calls`: created, assistant, duration, ended
reason (group colour), tools / failed, transferred, cost. Row click opens a drawer
from `/api/calls/{id}`: transcript by speaker, tool calls list with name, time, failed
flag, result excerpt; recording link (opens Vapi URL, no download).
**Acceptance:** WHEN a row with `tool_failures=1` is clicked THEN the drawer SHALL show exactly one tool call marked failed with its result excerpt.
**Verify:** `pnpm build` clean; manual check on live data, screenshot in PR.
**Must not:** embed or download audio.

### S-10 — `patterns/rules.py`: rule engine over stored calls ☐

**PR:** one.
**Depends on:** S-5.
**Files:** `src/graphify/patterns/__init__.py`, `src/graphify/patterns/rules.py`, `tests/test_rules.py`.
**Today:** nothing.
**Change:** `matches(rule: dict, call_row: dict, tool_rows: list) -> bool` per the rule
spec above. Phrase match is case-insensitive substring on transcript lines of the
chosen speaker (transcript lines are `User: ...` / `AI: ...`). Regex compiled once,
invalid regex → `ValueError` naming the pattern. `validate_rule(rule)` rejects unknown
keys.
**Acceptance:** WHEN rule `{"any_phrases":["real human"],"speaker":"user"}` runs on a call whose only "real human" line is spoken by the AI THEN `matches` SHALL return False.
**Verify:** `uv run pytest -q tests/test_rules.py` → pass, including one "prove by breaking" test: remove the speaker filter, that test fails.
**Must not:** `eval`/`exec` anything.

### S-11 — `patterns/llm.py`: cost estimate + one model call → rule + labels ☐

**PR:** one.
**Depends on:** S-10.
**Files:** `src/graphify/patterns/llm.py`, `src/graphify/patterns/prompt.md`, `tests/test_llm.py`.
**Today:** nothing.
**Change:** Load the `claude-api` skill first. `estimate(transcripts, model) -> {tokens, usd}`
using current per-model prices in a small table in the file. `learn(prompt, calls, model)`
sends `prompt.md` (system) + the user's criterion + numbered transcripts, asks for JSON
`{"rule": <rule spec>, "labels": [{"n": 1, "match": true, "evidence": "..."}]}`; parses
strictly; retries once on bad JSON. Models: `claude-opus-4-1`, `claude-sonnet-4-5`,
`gpt-5` (OpenAI via `openai` SDK). Keys from env; missing key → clear error, no call.
Tests mock the SDKs.
**Acceptance:** WHEN `learn` receives a mocked model reply with a valid rule and 5 labels THEN it SHALL return them parsed, AND WHEN `ANTHROPIC_API_KEY` is unset THEN it SHALL raise before any network call.
**Verify:** `uv run pytest -q tests/test_llm.py` → pass.
**Must not:** make a real model call in tests; call any model without a key.

### S-12 — `graphify pattern add "<criterion>" --sample 250 --model X [--yes]` ☐

**PR:** one.
**Depends on:** S-11, S-6.
**Files:** `src/graphify/cli.py`, `src/graphify/patterns/service.py`, `tests/test_pattern_cli.py`.
**Today:** no user-facing patterns.
**Change:** Takes the newest `--sample` calls from DB, prints estimate
`~N tokens, ~$X on <model>`, waits for `y` unless `--yes`. Calls `learn`, runs S-10 rule
over the same sample, computes `agreement` = fraction of sample where rule == label,
stores `patterns`, `pattern_labels`, prints `pattern #id saved, agreement 0.93,
matches 41/250`. Then runs the rule over ALL calls → `pattern_matches`.
**Acceptance:** WHEN the command runs without `--yes` and the user answers `n` THEN no model call SHALL be made and nothing SHALL be stored.
**Verify:** `uv run pytest -q tests/test_pattern_cli.py` (mocked model) → pass; live once with `--sample 25` on Sonnet after Abhishek says go.
**Must not:** spend without confirmation; exceed `--sample`.

### S-13 — `graphify apply` and auto-apply after every sync ☐

**PR:** one.
**Depends on:** S-12.
**Files:** `src/graphify/cli.py`, `src/graphify/sync.py`, `tests/test_apply.py`.
**Today:** patterns only counted at creation.
**Change:** `apply` re-runs every stored rule over every call, rebuilds `pattern_matches`.
`sync` calls it at the end. No model call ever.
**Acceptance:** WHEN 10 new calls are synced and 2 match a stored rule THEN `pattern_matches` for that rule SHALL grow by exactly 2 with no model call.
**Verify:** `uv run pytest -q tests/test_apply.py` → pass (assert the LLM module is never imported/called).
**Must not:** import `patterns.llm`.

### S-14 — Patterns in the UI: list, count chart, add form with cost confirm ☐

**PR:** one.
**Depends on:** S-13, S-9.
**Files:** `src/graphify/server.py` (`/api/patterns`, `POST /api/patterns/estimate`,
`POST /api/patterns`), `ui/src/Patterns.tsx`.
**Today:** patterns are CLI-only.
**Change:** Sidebar list of patterns with match count under current filters; one chart
"pattern matches per bucket"; add form: criterion, sample size, model select
(Opus / Sonnet / GPT-5). Submit → estimate shown → explicit Confirm → create. Show
agreement and the rule JSON, editable, "re-apply" button. Clicking a pattern filters the
call table to its matches.
**Acceptance:** WHEN the add form is submitted THEN no model call SHALL happen until Confirm is clicked, AND after Confirm the pattern SHALL appear with its count.
**Verify:** `pnpm build` clean; server tests for the three endpoints; manual run, screenshot.
**Must not:** send API keys from the browser (server reads env).

### S-15 — Daily run docs + `graphify schedule` helper ☐

**PR:** one.
**Depends on:** S-13.
**Files:** `README.md`, `src/graphify/cli.py`.
**Today:** user must remember to sync.
**Change:** `graphify schedule --print` prints a ready cron line and a macOS launchd
plist for `graphify sync --last 500` daily at 06:00. README gets Install / Sync /
Serve / Patterns / Daily sections.
**Acceptance:** WHEN `graphify schedule --print` runs THEN the output SHALL contain a valid crontab line invoking the absolute path of the `graphify` binary.
**Verify:** run it; paste line into `crontab -e` on Abhishek's machine; next morning `graphify sync` log shows a run.
**Must not:** install the cron itself without `--install` and a confirm.
