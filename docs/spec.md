# graphify — feature spec v2 (Vapi only)

> **Resume after compaction:** read this file top to bottom, then
> `python3 ~/.claude/MEM0/MEMOS/memo.py show graphify`, then `git status && git log -5`.
> The next step is the first `☐` in the register. Nothing else needs to be remembered.

## What a person sees today
Vapi's dashboard: a call list, one call at a time. To know "how many calls ended in a
failed transfer this week" or "how often does a caller ask for a human" you open calls
one by one, or pay Roark / Hamming / Sherlock. Nothing open-source does this for Vapi.

## What they must see instead
One web page (local by default, Docker for a team). Add a Vapi key in Settings with
the "+" button, pick assistants, pull the last N calls (14-day hard cap). Every chart,
each one toggleable: how calls ended (grouped) over time, tool-call failures by tool,
transfers, latency, cost with breakdown, duration, calls per assistant, Vapi's own
successEvaluation and structuredData fields. Filters: org, assistant(s), window
(1d / 5h / 7h / custom), last N, date range, call ID, ended group, tool failed,
transferred. Click a call → transcript + every tool call with result.

**Patterns** (the differentiator). Configure: provider Vapi → assistants (multi) →
calls (last N or explicit IDs) → "read the agent's prompt" on/off. Then a chat wizard:
user types a criterion in plain English ("calls where someone tried to book but
couldn't"). The brain reads the assistant's system prompt from Vapi, builds an
if-this-then-this plan table, asks follow-up questions until confidence ≥ 95%, shows
the cost, and only on click reads the sample calls in batches of ~20, labels each,
synthesizes one **rule** (JSON DSL, not code), validates the rule against its own
labels (agreement score), stores it, and suggests a chart. From then on every daily
`sync` re-counts the rule with no model call (mode `free`), or with a capped daily
model spend (`hybrid`, `full`). PDF downloads: dashboard, each pattern, call list.
An **Ask** box sends the current filter + sample calls to the chosen model for a
free-form answer ("how is it going?"), cost-confirmed.

## Acceptance sentence
WHEN a user adds a Vapi key, selects an assistant, and syncs the last 250 calls,
THEN within 2 minutes the dashboard SHALL show why each call ended, which tool calls
failed, and the cost per call; AND WHEN they create a pattern in plain English through
the wizard THEN graphify SHALL show its count over those calls and re-count it on the
next sync without a model call.

## Decisions
- **D-1 Three languages, one job each.** Rust `engine/` (sync, SQLite, rule engine,
  HTTP API, serves UI, Docker entry) — compiled, runs unattended daily, one binary.
  Python `brain/` with BAML (plan, clarify, label, synthesize, ask) — talks to models,
  BAML guarantees output shape. React `ui/`. Engine and brain share one SQLite file;
  engine spawns brain as a subprocess for jobs. **Rust steps are Fable/Opus only.**
- **D-2 Patterns are data, not code.** Model output is a JSON rule in our DSL; the
  Rust engine runs it. Never execute model-written code. If the DSL can't express the
  criterion, the brain says so in the plan instead of guessing.
- **D-3 Batched learning.** Labeling loops over calls ≤ 20 per model call. One
  synthesis call turns all evidence into the rule. Accuracy over speed.
- **D-4 API pull in v1, webhook v2.** GET only, ever.
- **D-5 Retention: 14 days hard cap** (`keep_days` ≤ 14, default 14), plus optional
  `max_calls` per org. Both configurable, neither can exceed 14 days.
- **D-6 Keys live server-side, added in the UI.** Stored encrypted in SQLite with a
  secret from `GRAPHIFY_SECRET` or auto-generated `data/.secret` (mode 0600). Env vars
  override. The API never returns a key, only `set: true, last4`.
- **D-7 Multi-org.** Every data table has `org_id`. UI has an org switcher.
- **D-8 Pattern modes:** `free` (default, zero spend), `hybrid` (rule prefilters, cheap
  model confirms new candidates, daily cap), `full` (model reads every new call, daily
  cap). Chosen per pattern.
- **D-9 Docker + optional password from day one.** `GRAPHIFY_PASSWORD` set → login
  page + cookie session. Unset → localhost only, no login.
- **D-10 PDFs are built in the browser** (jsPDF + chart canvas capture). No server-side
  PDF dependency.
- **D-11 endedReason is grouped:** customer · assistant · llm-error · tts-error ·
  stt-error · transfer-error · transport · timeout · start-error · other · unknown.
  Raw code kept.
- **D-12 Store slim, not raw.** Parse everything the charts need into columns, keep a
  de-duplicated `slim` JSON for the call drawer, never the 137 KB raw. Prompts live once
  per assistant version. See "Slimming rule".

## Must never (every step inherits these)
- Send anything but GET to Vapi.
- Call a model without a shown cost and an explicit go (`--yes` / click). Daily modes
  have a hard USD cap and stop when reached.
- Download or store audio. Recording URL only.
- Return a key to the browser, print one to a log, or write one to the DB in clear.
- Render a missing value as 0. NULL → "—".

## Deliberately does not do (v1)
- No scoring / evals / "was this call good".
- No Retell, ElevenLabs, other providers (schema has `provider`, code has one).
- No webhook receiver. No placing test calls (v2). No chat history for Ask.
- No squad handoff tracking: a squad is shown as its member assistants.

## Data model (SQLite, `data/graphify.db`, migrations in `engine/migrations/`)
```
orgs(id INTEGER PK, name TEXT, provider TEXT DEFAULT 'vapi', keep_days INTEGER DEFAULT 14,
     max_calls INTEGER NULL, created_at TEXT)
secrets(org_id INTEGER NULL, name TEXT, ciphertext BLOB, last4 TEXT, updated_at TEXT,
        PRIMARY KEY(org_id, name))            -- name: vapi | anthropic | openai
assistants(id TEXT PK, org_id INTEGER, name TEXT, version TEXT, model_provider TEXT,
           model TEXT, voice_provider TEXT, transcriber_provider TEXT, transcriber_model TEXT,
           system_prompt TEXT, prompt_sha256 TEXT, first_message TEXT, tool_ids JSON,
           structured_schema JSON, fetched_at TEXT)      -- slim: ~40 KB, no raw
tools(id TEXT PK, org_id INTEGER, name TEXT, type TEXT, is_transfer INTEGER, fetched_at TEXT)
calls(id TEXT PK, org_id INTEGER, assistant_id TEXT, assistant_version TEXT,
      phone_number_id TEXT, call_type TEXT, status TEXT,
      created_at TEXT, started_at TEXT, ended_at TEXT, duration_s REAL,
      ended_reason TEXT, ended_group TEXT,
      cost REAL, cost_stt REAL, cost_llm REAL, cost_tts REAL, cost_vapi REAL,
      cost_transport REAL, cost_analysis REAL, llm_prompt_tokens INTEGER,
      llm_completion_tokens INTEGER, llm_cached_tokens INTEGER, tts_characters INTEGER,
      transferred INTEGER, transfer_destination TEXT,
      tool_calls INTEGER, tool_failures INTEGER,
      turns INTEGER, lat_turn_avg_ms REAL, lat_turn_p50_ms REAL, lat_turn_p95_ms REAL,
      lat_model_avg_ms REAL, lat_voice_avg_ms REAL, lat_transcriber_avg_ms REAL,
      lat_endpointing_avg_ms REAL, turn_latencies JSON,
      success_eval TEXT, summary TEXT, structured JSON,
      transcript TEXT, recording_url TEXT, slim JSON, synced_at TEXT)
tool_calls(call_id TEXT, name TEXT, seconds_from_start REAL, failed INTEGER,
           arguments TEXT, result_excerpt TEXT)
patterns(id INTEGER PK, org_id INTEGER, name TEXT, criterion TEXT, assistant_ids JSON,
         plan JSON, rule JSON, chart JSON, model TEXT, mode TEXT DEFAULT 'free',
         daily_cap_usd REAL DEFAULT 1.0, sample_size INTEGER, agreement REAL,
         created_at TEXT)
pattern_labels(pattern_id INTEGER, call_id TEXT, llm_match INTEGER, rule_match INTEGER,
               evidence TEXT)
pattern_matches(pattern_id INTEGER, call_id TEXT, source TEXT)  -- rule | llm
jobs(id INTEGER PK, kind TEXT, status TEXT, input JSON, output JSON, cost_usd REAL,
     log TEXT, created_at TEXT, finished_at TEXT)
spend(day TEXT, org_id INTEGER, usd REAL, PRIMARY KEY(day, org_id))
dashboard(org_id INTEGER PK, config JSON)      -- which charts are enabled, order
```

### Verified Vapi field paths (live probe 2026-09-03, fixture `engine/tests/fixtures/call_ended_transfer.json`)
- `GET /call?limit=100` returns the **full** call incl. `artifact`; only presigned URLs are
  missing vs `GET /call/{id}`. No per-call GET needed. In-progress calls have empty artifacts.
- Latency: `artifact.performanceMetrics.turnLatencies[] = {modelLatency, voiceLatency,
  transcriberLatency, endpointingLatency, turnLatency}` (ms) plus `modelLatencyAverage`,
  `voiceLatencyAverage`, `transcriberLatencyAverage`, `endpointingLatencyAverage`,
  `turnLatencyAverage`, `fromTransportLatencyAverage`, `toTransportLatencyAverage`.
- Cost: `cost` (total), `costBreakdown.{stt, llm, tts, vapi, transport, total,
  llmPromptTokens, llmCompletionTokens, llmCachedPromptTokens, ttsCharacters,
  analysisCostBreakdown.{summary, structuredData, successEvaluation}}`; `costs[]` per
  provider with `type` and model name.
- Analysis: `analysis.summary`, `analysis.successEvaluation` (string "true"/"false"),
  `analysis.structuredData` (object; keys from the assistant's
  `analysisPlan.structuredDataPlan.schema`).
- Messages: `artifact.messages[]` roles `system | bot | user | tool_calls |
  tool_call_result`; timing `time`, `endTime`, `secondsFromStart`, `duration` (ms);
  `tool_calls[].toolCalls[].function.{name, arguments}`; `tool_call_result.{name, result,
  toolCallId}`. `messages` (top level) and `artifact.messagesOpenAIFormatted` are
  duplicates — drop both.
- Transfer: `endedReason == "assistant-forwarded-call"`, `destination.{type, number}`,
  `forwardedPhoneNumber`, `artifact.transfers[]`, and a tool call whose tool `type ==
  "transferCall"` (resolve via `tools`).
- Assistant (`GET /assistant?limit=100`, ~49 KB each): `name`, `latestVersion`,
  `model.{provider, model, toolIds[], messages[role=system].content}`, `voice.provider`,
  `transcriber.{provider, model}`, `firstMessage`, `analysisPlan.structuredDataPlan.schema`.
  Tools are referenced by id → `GET /tool?limit=100` gives `{id, type, function.name}`.
- `artifact.assistantActivations[] = {assistantId, assistantName, assistantVersion}` ties
  a call to the prompt version. `GET /squad` returned `[]` on the probe org.

### Slimming rule (D-12)
Raw call 137 KB → stored `slim` ≈ 6 KB: keep `artifact.messages` with the system
message content replaced by `{"role":"system","prompt_sha256":…}`; drop top-level
`messages`, `messagesOpenAIFormatted`, `variables`/`variableValues`, `monitor`,
`transport`, presigned URLs, `assistant`/`squad` inline copies. Keep `costs[]`,
`performanceMetrics`, `analysis`, `destination`, `assistantActivations`, `recording`
URLs (URLs only). Assistant stored once per `(id, version)` with the prompt; calls
reference it by `assistant_version`.

## Rule DSL (what SynthesizeRule must return)
```json
{
  "any_phrases": ["speak to a person", "real human"],
  "regex": ["\\btalk to (a|an|the) (agent|human|person)\\b"],
  "speaker": "user",
  "ended_reasons": [], "ended_groups": [],
  "tool_called": [], "tool_not_called": ["bookAppointment"],
  "tool_failed": null, "transferred": null,
  "min_duration_s": null, "max_duration_s": null
}
```
Match = (any phrase OR any regex on lines from `speaker` ∈ user|bot|any; empty both =
true) AND every non-null / non-empty structural condition holds.

## Brain functions (BAML, `brain/baml_src/`)
| Function | In | Out |
|---|---|---|
| `PlanPattern` | criterion, assistant system prompt, DSL description | plan rows `{if, then}`, questions[], confidence 0-1, expressible bool |
| `ClarifyPattern` | plan + user answers | updated plan, remaining questions, confidence |
| `LabelBatch` | criterion, plan, ≤20 transcripts | `[{n, match, evidence}]` |
| `SynthesizeRule` | criterion, plan, all labels+evidence, DSL | rule JSON, chart `{type, title}` |
| `RefineRule` | rule, disagreements | rule JSON |
| `AskAnalysis` | stats JSON, sample transcripts, question | answer markdown |

Engine ↔ brain contract: `graphify-brain <fn> --db PATH` reads JSON on stdin, writes
JSON on stdout, exit 0/1, progress lines on stderr as `PROGRESS n/m`. Engine records
everything in `jobs`.

## Repo layout (target)
```
engine/        Rust crate, binary `graphify`  (Cargo.toml, src/, migrations/, tests/)
brain/         Python package `graphify-brain` (pyproject.toml, src/, baml_src/, tests/)
ui/            Vite + React + TS + Recharts
docs/spec.md   docs/backlog/bugs.md
.github/workflows/ci.yml   Dockerfile   docker-compose.yml
```

---

# Step register

Legend: ☐ todo · ☐→ in progress · ☑ done (with what was learned).
Tag `[Rust]` = Fable/Opus only. Untagged = any model.

### S-1 — Python project scaffold with a `graphify` CLI that prints its version ☑ (PR #1, 5162347)
**Learned:** typer collapses a single `@app.command()` into the root command; an empty
`@app.callback()` keeps `version` a subcommand. Superseded by S-2: this package becomes
`brain/` and the Rust engine takes the `graphify` binary name.

### S-2 — Move the Python package to `brain/` as `graphify-brain` ☑ (PR #2, 5a29ae2)
**Learned:** hatchling refuses to build without the `readme` file existing on disk —
added `brain/README.md`. `uv sync` doesn't reprint already-resolved deps on a repeat
run; check `uv.lock` or `uv run python -c "import x"`, not sync's stdout, to confirm
a new dependency landed.
**PR:** one. **Depends on:** S-1.
**Files:** `brain/pyproject.toml`, `brain/src/graphify_brain/__init__.py`,
`brain/src/graphify_brain/cli.py`, `brain/tests/test_cli.py`; delete root
`pyproject.toml`, `src/`, `tests/`, `uv.lock`.
**Today:** Python lives at repo root under the name `graphify`.
**Change:** `git mv` into `brain/`, rename package to `graphify_brain`, script
`graphify-brain = "graphify_brain.cli:app"`, output `graphify-brain 0.1.0`. Add
`baml-py` dependency (no BAML code yet). Root `.gitignore` gains `brain/.venv/`,
`brain/baml_client/`.
**Acceptance:** WHEN `cd brain && uv sync && uv run graphify-brain version` runs THEN it SHALL print `graphify-brain 0.1.0`, AND the repo root SHALL contain no `pyproject.toml`.
**Verify:** `cd brain && uv run pytest -q` → 1 passed. `ls ~/graphify` shows no `src/`.
**Must not:** touch `engine/`, `ui/`.

### S-3 — Rust engine scaffold: `graphify version` `[Rust]` ☑ (PR #3, 42fc647)
**Learned:** `engine/` already held `tests/fixtures/`, so `cargo new` was wrong — the
files were written by hand instead (cargo ignores a `tests/` subdir with no `.rs` in it,
so the fixtures and the integration tests coexist). `assert_cmd` takes a `&str` as an
exact stdout predicate, so no `predicates` dep is needed. Clippy must be run as
`cargo clippy --all-targets -- -D warnings` or it skips the test targets.
**PR:** one. **Depends on:** nothing.
**Files:** `engine/Cargo.toml`, `engine/src/main.rs`, `engine/src/cli.rs`, `engine/tests/cli.rs`.
**Today:** no Rust.
**Change:** `cargo new engine --name graphify`. Deps: `clap` (derive), `anyhow`. Subcommand
`version` prints `graphify 0.1.0`. Integration test with `assert_cmd`. Edition 2021.
**Acceptance:** WHEN `cargo run -q -- version` runs in `engine/` THEN it SHALL print `graphify 0.1.0`.
**Verify:** `cd engine && cargo test -q` → pass; `cargo clippy -- -D warnings` clean.
**Must not:** network; touch `brain/`.

### S-4 — GitHub Actions CI: cargo test, pytest, ui build ☑ (PR #4, 6271a12)
**PR:** one. **Depends on:** S-2, S-3.
**Files:** `.github/workflows/ci.yml`.
**Today:** no CI; merges are unchecked.
**Change:** On PR and push to main: job `engine` (cargo test + clippy), job `brain`
(uv sync + pytest), job `ui` (skipped with `if: hashFiles('ui/package.json') != ''` until
S-16). Cache cargo and uv.
**Acceptance:** WHEN a PR is opened THEN two green checks `engine` and `brain` SHALL appear on it.
**Verify:** open this step's PR, watch `gh pr checks`.
**Must not:** run anything that needs a secret.
**Learned:** the spec's `if: hashFiles('ui/package.json') != ''` cannot gate a *job* —
`hashFiles()` reads `GITHUB_WORKSPACE`, which is empty before `actions/checkout` runs, so the
job-level `if` is always `''` and the job never turns on, even after S-16. Guard moved to a step
after checkout that writes `$GITHUB_OUTPUT`; the four real ui steps carry
`if: steps.check.outputs.exists == 'true'` and switch themselves on when `ui/` appears.
Action tags in the wild are far newer than habit suggests — pinned `actions/checkout@v5`,
`astral-sh/setup-uv@v7`, `actions/setup-node@v5` after resolving every ref with
`gh api repos/<a>/commits/<ref>` instead of guessing `@v4`. CI also greens a `CodeRabbit`
check that self-skips on this repo; only `engine` and `brain` are real gates.

### S-5 — SQLite schema + migrations + `orgs` `[Rust]` ☑ (PR #5, fdd8736)
**PR:** one. **Depends on:** S-3.
**Files:** `engine/src/db.rs`, `engine/migrations/0001_init.sql`, `engine/tests/db.rs`.
**Today:** no storage.
**Change:** `rusqlite` (bundled) + `rusqlite_migration`. Tables exactly as in Data model.
`Db::open(path)` runs migrations; default path `data/graphify.db`, env `GRAPHIFY_DB`.
Helpers: `upsert_call`, `replace_tool_calls`, `create_org`, `list_orgs`. Indexes on
`calls(org_id, created_at)`, `calls(assistant_id)`, `calls(ended_group)`.
**Acceptance:** WHEN `upsert_call` runs twice with the same id THEN one row SHALL remain with the second values, AND WHEN `Db::open` runs on an existing file THEN it SHALL not fail.
**Verify:** `cargo test -q --test db` → pass on a tempfile DB.
**Must not:** add an ORM; touch Vapi.
**Learned:** `cargo test -q db` filters by test *name*, not by test target — it matched
nothing and exited 0 having run zero tests, a false green. Verify is now
`cargo test -q --test db`. `engine/tests/` is an integration-test dir, so it can only see a
*library* target; `main.rs` modules are invisible to it. Added `engine/src/lib.rs`
(`pub mod db;`) — every later engine module goes there, `cli.rs` stays in the binary.
`upsert_call` is `INSERT OR REPLACE`, safe only because nothing references `calls` by
foreign key (REPLACE deletes the old row first, which would fire ON DELETE CASCADE);
it overwrites wholesale, so a value that goes missing is written back as NULL, not left
stale. Two schema additions beyond the Data model, both needed: `orgs.name` is
`NOT NULL UNIQUE` (S-9 resolves an org by name) and `idx_tool_calls_call` exists because
`replace_tool_calls` deletes by `call_id` on every sync. `rusqlite 0.40` + `rusqlite_migration 2`
resolve to a single shared `rusqlite`, so no duplicate-type mismatch; bundled SQLite makes
the engine CI job ~1m instead of ~25s.

### S-6 — endedReason → group `[Rust]` ☑ (PR #6, c7eb477)
**PR:** one. **Depends on:** S-3.
**Files:** `engine/src/ended_reason.rs`, `engine/tests/ended_reason.rs`.
**Today:** nothing.
**Change:** `pub fn group(code: Option<&str>) -> &'static str`. Ordered rules:
`silence-timed-out|exceeded-max-duration` → timeout; `call.start.error*|assistant-not-*|
assistant-request-*|scheduled-call-deleted` → start-error; contains `transfer` →
transfer-error; contains `transcriber` or `-returning-` → stt-error; contains `voice` or
`out-of-credits` or `quota` → tts-error; contains `llm` or `pipeline` or `-4dd-`/`-5dd-`
→ llm-error; contains `sip|twilio|vonage|transport|worker|websocket` → transport;
starts `customer-` or `voicemail` → customer; starts `assistant-` → assistant; None →
unknown; else other. Source: https://docs.vapi.ai/calls/call-ended-reason.
**Acceptance:** WHEN `group(Some("call.in-progress.error-transfer-failed"))` THEN `transfer-error`; WHEN `group(None)` THEN `unknown`.
**Verify:** `cargo test -q --test ended_reason` with ≥ 15 cases covering all 11 groups → pass.
**Must not:** network.
**Learned:** the ordered rules have one collision the spec did not see: `voicemail`
contains `voice`, so the tts rule swallowed it before the customer rule could claim it —
every voicemail call would have charted as a TTS provider failure, invisibly. The tts rule
now skips codes starting with `voicemail`. Three readings the spec left open, all
implemented and reversible: `-4dd-`/`-5dd-` means an embedded HTTP status (`-4`/`-5` +
two digits + `-`), because taken as four literal characters no real Vapi code contains it
and the rule would be dead; codes are trimmed and lowercased before matching, since a case
variant landing in `other` corrupts a chart with no error; blank (`Some("")`) is `unknown`,
not `other`. Verify is `cargo test -q --test ended_reason` — the bare-name form runs zero
tests, same false green as S-5. Test is a 31-case table plus a guard test asserting all
eleven group names appear, so a future rule edit that drops a group fails loudly.

### S-7 — Vapi client: read-only paginated `GET /call` `[Rust]` ☑ (PR #7, de39422)
**PR:** one. **Depends on:** S-3.
**Files:** `engine/src/vapi.rs`, `engine/tests/vapi.rs`.
**Today:** nothing.
**Change:** `reqwest` (rustls) + `tokio`. `fetch_calls(key, FetchOpts{last, since, until,
assistant_id})` → `Vec<serde_json::Value>` newest-first; `limit=100`; cursor
`createdAtLt=<oldest createdAt of previous page>`; stop at `last`, short page, or
`createdAt <= since`. Retry 429/5xx ×5 with backoff. Tests use `wiremock`. Only
`reqwest::Client::get` exists in this file — enforce with a unit test that greps the
source for `.post(` / `.patch(` / `.delete(`.
**Acceptance:** WHEN mocked pages of 100 and 30 are served THEN `fetch_calls(last: 250)` SHALL return 130 and make exactly 2 requests.
**Verify:** `cargo test -q --test vapi` → pass.
**Must not:** any non-GET; log the key.
**Learned:** `cargo test -q vapi` is a false green a third time — the name filter matches no
test fn, so it exits 0 having run nothing. Every remaining `cargo test -q <word>` Verify
line in this register has the same bug; use `--test <file>`. The GET-only guard is
`include_str!("../src/vapi.rs")` + a substring scan, and it also asserts `.get(` is still
present so deleting the requests can't pass it. `reqwest` needs `default-features = false`
or it drags in native-tls alongside rustls — `cargo tree -i rustls` and a grep for openssl
are the proof. Two seams the spec didn't name: `fetch_calls_at(base, key, opts, retry)`
carries the base URL and retry policy so wiremock can be pointed at and the backoff zeroed
(a real 5×500ms-doubling exhaustion test would sleep 15.5s in CI); `fetch_calls` keeps the
spec's signature on top. `since` is filtered client-side as the spec words it, not sent as
`createdAtGt`. The page loop can't spin forever because a full page always grows the
result, so `out.len() < last` bounds it even if a server ignores the cursor.

### S-8 — Extract: raw call → `calls` row + `tool_calls` + slim JSON `[Rust]` ☑ (PR #8, 5fc1c6a)
**PR:** one. **Depends on:** S-5, S-6.
**Files:** `engine/src/extract.rs`, `engine/tests/extract.rs`, uses
`engine/tests/fixtures/call_ended_transfer.json` (synthetic replica of a real payload, same keys and numbers).
**Today:** nothing.
**Change:** Map per "Verified Vapi field paths": `duration_s = endedAt - startedAt`
(NULL if either missing); `ended_group` via S-6; cost columns from `costBreakdown`
(`cost_analysis` = sum of `analysisCostBreakdown.{summary, structuredData,
successEvaluation}`); `transferred` = endedReason `assistant-forwarded-call` OR
`destination.number` present OR any tool call whose name is in `tools` with
`is_transfer=1`; `transfer_destination = destination.number`; `tool_calls` = count of
`toolCalls[]` across `tool_calls` messages; `tool_failures` = `tool_call_result` whose
`result` is empty or contains `error`/`failed` (case-insensitive); `turns` = count of
`turnLatencies`; `lat_*_avg_ms` from the `*LatencyAverage` fields; `lat_turn_p50/p95`
computed from `turnLatencies[].turnLatency`; `turn_latencies` = the array as JSON;
`success_eval`, `summary`, `structured` from `analysis`; `transcript`, `recording_url`
(`artifact.recordingUrl`); `slim` per the Slimming rule; `assistant_version` from
`artifact.assistantActivations[0].assistantVersion`. Missing → NULL, never 0.
**Acceptance:** WHEN the fixture is extracted THEN `tool_calls=1, tool_failures=0, transferred=1, turns=2, lat_turn_avg_ms=4553.5, lat_turn_p95_ms=6030, cost_vapi=0.0248, success_eval="true", structured.call_intent="general_info"`, AND `slim` SHALL be under 10 KB and contain no `messagesOpenAIFormatted` key.
**Verify:** `cargo test -q --test extract` → pass.
**Must not:** store raw; download recordings; coerce missing to 0.
**Learned:** `extract(raw, org_id, transfer_tools) -> (Call, Vec<ToolCall>)` is pure — no
network, no clock. `synced_at` stays NULL for S-9 to fill, which is what lets the tests
assert exact values with no time control. "Missing → NULL" needed a sharper edge than the
spec words it: a call with **no** `artifact.messages` gets NULL tool counts, a call with an
**empty** one gets 0, and both cases are tested. Same shape for `transferred` — no evidence
means `false` once `endedReason` exists and NULL while the call is still running. `slim` is
built by *removing* keys, not allow-listing, so a field Vapi adds later reaches the drawer
instead of vanishing; it lands at 7.7 KB from an 11 KB fixture. Percentiles are nearest-rank,
which is why the fixture's p95 of two turns is exactly `6030`. `chrono` needs
`default-features = false` or it drags in `iana-time-zone` for a clock this never uses.
Watch out: `assistant-forwarded-call` groups as `assistant`, not `transfer-error` — a
forward that worked is not an error, and I wrote that expectation wrong before the test
corrected me.

### S-9 — `graphify sync --org NAME --last N | --since DATE`, incremental + purge `[Rust]` ☑ (PR #9, 9c7960e)
**PR:** one. **Depends on:** S-7, S-8.
**Files:** `engine/src/cli.rs`, `engine/src/sync.rs`, `engine/tests/sync.rs`.
**Today:** CLI prints version only.
**Change:** Key from `secrets` (S-11) or env `VAPI_API_KEY` (env wins). If DB has calls for
the org, default `since` = newest `created_at` → re-runs add only new calls; `--last 500`
on 250 stored fetches up to 250 more, never replaces. After upsert: delete calls older
than `orgs.keep_days` (≤ 14, reject higher) and beyond `orgs.max_calls` if set. Print
`org X: synced N new, M total, purged P`.
**Acceptance:** WHEN `sync --last 250` runs twice against the same mock THEN the second run SHALL print `0 new` and the table SHALL hold 250 rows; WHEN `keep_days` is 20 THEN sync SHALL refuse with a message naming the 14-day cap.
**Verify:** `cargo test -q --test sync` → pass; live once with a real key after Abhishek's go: `graphify sync --org test --last 25`.
**Must not:** non-GET; delete rows inside the keep window.
**Learned:** `--last N` is a *target size* for the org, so stored rows count against it
(budget = `last - stored`, which is the spec's own "250 stored, `--last 500` fetches 250
more" arithmetic); `--since DATE` is a *range* and does not subtract, or an org already at
its target could never be asked for an older window. Consequence to remember: once
`--last` is met sync makes **no request at all**, so a daily job needs a `--last` above its
steady-state row count, or `--since`. `new` is the row-count delta around the write, never
the fetch size. Unknown age is not old age — the age sweep skips calls with no
`created_at`, and the `max_calls` sweep sorts them last. Purging a call must take its
`tool_calls` rows or every tool chart counts rows no call points at. Compare ages with
`julianday(created_at) < julianday('now', '-N days')`: Vapi's `…T…Z` and SQLite's
`datetime('now')` are different shapes and a text compare purges the wrong side.
`keep_days` and the missing key are both checked *before* the fetch, so a bad setting
costs neither a request nor a DELETE. chrono needs feature `now` (SystemTime) for
`Utc::now()`; `clock` would drag `iana-time-zone` back in for a local zone nothing wants.

### S-10 — Assistants + tools: slim fetch into `assistants` and `tools` `[Rust]` ☑ (PR #10, 1762a65)
**PR:** one. **Depends on:** S-9.
**Files:** `engine/src/vapi.rs`, `engine/src/assistants.rs`, `engine/tests/assistants.rs`,
uses `engine/tests/fixtures/assistant.json` and `engine/tests/fixtures/tools.json`.
**Today:** calls only; no prompt, tool ids unresolved.
**Change:** `graphify assistants --org NAME`: `GET /tool?limit=100` (paginate) →
`tools(id, name=function.name, type, is_transfer = type == "transferCall")`; then
`GET /assistant?limit=100` (paginate) → slim columns per data model; `system_prompt` =
first `model.messages[]` with `role == "system"` → `.content`, `prompt_sha256` of it;
`structured_schema = analysisPlan.structuredDataPlan.schema` (NULL if disabled). Do not
store the raw 49 KB. Skip write when `prompt_sha256` and `version` unchanged. `sync`
runs this first. `GET /squad` members flattened (probe org had none; keep the code path
small).
**Acceptance:** WHEN the assistant fixture is parsed THEN `system_prompt` SHALL start with `You are a service-desk`, `model="gpt-4.1"`, `transcriber_model="flux-general-multi"`, `tool_ids` SHALL have 3 entries, and `structured_schema.properties.call_intent.enum` SHALL contain `"transfer_request"`; WHEN the tools fixture is parsed THEN the `transferCall` tool SHALL have `is_transfer=1`.
**Verify:** `cargo test -q --test assistants` → pass; live: `graphify assistants --org rush` prints 100 names.
**Must not:** non-GET; store the raw assistant.
**Learned:** `sync` runs this first *inside `sync::run`*, not in the CLI — `tools.is_transfer`
is what lets the extractor recognise a transfer `endedReason` does not name, and a stale
tools table does not error, it silently under-counts transfers on every call written after
it. Wiring it in the CLI would have given that ordering to one subcommand instead of every
caller. Cost: every mock server in `tests/sync.rs` now has to answer `/tool`, `/assistant`
and `/squad`, and the request-count assertions there filter on `/call`. A disabled
`structuredDataPlan` still carries its old schema — store NULL, or the dashboard is
promised columns nothing fills. An assistant with no system prompt gets a NULL
`prompt_sha256`, never the hash of `""`, which would give every prompt-less assistant one
fingerprint. Staleness needs `version` **and** `prompt_sha256`: Vapi does not always bump
`latestVersion` for a prompt edit. Squad members carry either an `assistantId` (already in
`GET /assistant`) or a whole inline assistant; only inline ones with an `id` are storable.
`fetch_all_at` is the paginator for list endpoints with no `last` to stop them, so it stops
itself — short page, no usable `createdAt`, or a cursor that did not move; without that
last guard a page of untimestamped rows loops forever.

### S-11 — Secrets store: encrypted at rest, env override, never returned `[Rust]` ☑ (PR #11, 0837ef0)
**PR:** one. **Depends on:** S-5.
**Files:** `engine/src/secrets.rs`, `engine/tests/secrets.rs`.
**Today:** env vars only.
**Change:** `chacha20poly1305` with key from `GRAPHIFY_SECRET` (32 bytes, base64) else
`data/.secret` created with mode 0600. `set(org, name, value)`, `get(org, name) ->
Option<String>` (env `VAPI_API_KEY` / `ANTHROPIC_API_KEY` / `OPENAI_API_KEY` override),
`status(org) -> [{name, set, last4}]`. `Debug` impl for the secret type prints `***`.
**Acceptance:** WHEN a key is set THEN the DB file SHALL not contain its plaintext bytes, AND `status` SHALL show `set: true` with the last 4 chars only.
**Verify:** `cargo test -q --test secrets` → pass (test greps the DB file for the plaintext and asserts absent).
**Must not:** log values.
**Learned:** `get` returns `Option<Secret>`, not `Option<String>` — the spec asks for both
a `String` and a `Debug` that prints `***`, and only the wrapper can be both. `Secret`
prints `***` under `{}`, `{:?}`, `dbg!` and panics alike, so `expose` is the single word
to grep when asking whether a key can reach a log; `Secrets` prints `key: ***` for the
same reason. AEAD associated data is `org_id:name`, so a ciphertext lifted into another
row fails to decrypt instead of returning the wrong org's key. A variable set to
whitespace is **not** an override — an empty `VAPI_API_KEY=` in a compose file must not
mask a good stored key. `status` reports what `get` would return, so an env-only key reads
as set rather than leaving the settings screen claiming "not set" while sync works. The
key file is opened `create_new` **with** mode 0600, never created then chmodded: a key
must not exist readable even for an instant. A value under 8 characters gets no `last4` —
four characters of a short secret is most of it. Scope kept to the spec's Files line, so
`sync` and `assistants` still read the env directly; S-12 is the step that first gives an
org a stored key and is where they get wired, and the env winning means that change cannot
alter today's behaviour. Crates added are pure-Rust RustCrypto; `chacha20` and `rand_core`
each end up at two versions because `chacha20poly1305 0.10` pins the older ones.

### S-12 — HTTP API (axum): orgs, assistants, calls, stats, optional password `[Rust]` ☑ (PR #12, 9ff68d2)
**PR:** one. **Depends on:** S-9, S-10, S-11.
**Files:** `engine/src/server.rs`, `engine/src/queries.rs`, `engine/src/auth.rs`, `engine/tests/server.rs`.
**Today:** CLI only.
**Change:** `axum`. Routes: `GET /api/orgs`, `POST /api/orgs`, `GET /api/orgs/{id}/secrets`,
`PUT /api/orgs/{id}/secrets/{name}` (body `{value}`), `POST /api/orgs/{id}/test`
(one GET /assistant with that key → ok/err), `GET /api/assistants?org=`,
`GET /api/calls?…`, `GET /api/calls/{id}`, `GET /api/stats?…`, and — added in S-17 —
`GET|PUT /api/dashboard?org=` (body `{charts:[{id, on}]}`, which takes `org` and
nothing else). Shared filters: `org`,
`assistant_id` (repeatable), `since`, `until`, `window` (`1d|5h|7h|<n>h|<n>d`), `last`,
`ended_group`, `call_id`, `tool_failed`, `transferred`. `/api/stats` returns
`{by_ended_group, by_ended_reason, per_bucket:{calls, tool_failures, transfers,
cost, duration_avg, latency_p50, latency_p95}, tool_failures_by_name,
by_assistant, success_eval_counts, structured_fields, totals}`; bucket 1h ≤ 2d else 1d.
`GRAPHIFY_PASSWORD` set → `POST /api/login`, cookie session, all `/api/*` 401 without it.
Bind `127.0.0.1:3737` unless `GRAPHIFY_BIND` set.
**Acceptance:** WHEN 10 calls are seeded, 3 in `transfer-error`, THEN `/api/stats?window=1d` SHALL report `by_ended_group["transfer-error"] == 3`; WHEN `GRAPHIFY_PASSWORD` is set THEN `/api/stats` without a session SHALL return 401.
**Verify:** `cargo test -q --test server` → pass; `graphify serve` + `curl localhost:3737/api/stats?org=1`.
**Must not:** return secret values; bind 0.0.0.0 by default.
**Learned:** filters are parsed from the raw query string, not through serde:
`serde_urlencoded` cannot express a repeated key and `assistant_id` has to repeat, and
parsing by hand is what makes an **unknown key a 400** rather than a shrug — a typo'd
filter that silently answered with the whole org is a wrong chart nobody can spot. `org`
is an org **id**, matching the spec's own `?org=1`; `?org=` with no value means "no choice
made", which is what a browser sends from an unset select. NULL survives to the browser:
`sum()` over no priced call is NULL, so an empty bucket reports `cost: null` and
`calls: 0` — a count of none is a number, a cost of none is not — and `duration_avg`
divides by the calls that carried a duration. Buckets are filled across the whole span
including the empty ones, because a gap in a line chart has to be drawn as a gap, and the
axis runs from the requested `since` so two charts with the same `window` line up. Bucket
size follows the **span**, however it was asked for, so an explicit two-hour `since` is as
hourly as `window=2h`; `bucket_size` is returned so the axis needs no re-deriving. Bucket
percentiles are percentiles over per-call percentiles — the raw turns live in
`turn_latencies` and re-parsing them every refresh costs more than the precision is worth.
A NULL `ended_group` buckets as `unknown` (the call has not ended, which is what
`ended_reason::group(None)` already calls that); a NULL `success_eval` is dropped, because
"no evaluation ran" is not a verdict, and the same rule drops a structured key that came
back null. The connectivity test runs with **retries off** and reports a rejected key as
`{ok: false}` with a 200 — "your key does not work" is the answer that was asked for, not
a server error. `Db` sits behind a `std::sync::Mutex` because `Connection` is not `Sync`;
the one handler that waits on the network drops the lock first, and a poisoned lock is
recovered rather than propagated. Sessions live in memory only — a restart signs everyone
out, which beats a session table to leak — and passwords compare over SHA-256 digests so
the comparison cannot leak a length. This is also where S-11's deferred wiring landed:
`sync` and `assistants` read the org's key from the store, the environment still winning,
which means the DB is now opened before the key is looked for and a missing org is the
first thing that goes wrong. The plaintext still leaves the `Secret` wrapper at that
boundary because `sync::Opts.key` is a `String`; neither `Opts` derives `Debug`, so
nothing prints it. `engine/tests/cli.rs` needed a `GRAPHIFY_DB` temp path per test — two
CLI tests migrating the same new file is a race as well as a stray `data/` in the repo.
Only five crates are new to the lockfile (axum, axum-core, matchit, mime,
serde_path_to_error); everything else axum wants was already there via reqwest. The server
suite holds a **`tokio::sync::Mutex`**, not a `std` one, around every env-touching test:
the environment is read on the far side of an `await`, and clippy's `await_holding_lock`
is right that a `std` guard must not survive one.

### S-13 — `graphify serve` serves `ui/dist` and opens the browser `[Rust]` ☑ (PR #13, 1633c61)
**PR:** one. **Depends on:** S-12.
**Files:** `engine/src/server.rs`, `engine/build.rs`, `engine/Cargo.toml`.
**Today:** API only.
**Change:** `rust-embed` embeds `ui/dist` at build time when present (empty page with
"UI not built" otherwise). `serve --no-open` flag; default opens the browser with `open`.
**Acceptance:** WHEN `ui/dist/index.html` exists at build time THEN `GET /` SHALL return it; otherwise a 200 placeholder.
**Verify:** `cargo test -q --test serve`; manual curl.
**Must not:** call Vapi.
**Learned:** `rust-embed` was named here and not used: it costs fourteen new crates —
`mime_guess`, `walkdir`, `unicase`, a second copy of the whole sha2/digest tree — to do
what a recursive `read_dir` and `include_bytes!` do in thirty lines, and it cannot name a
folder that does not exist yet, which is the case this step is mostly about. `engine/build.rs`
walks `ui/dist` and writes an `include_bytes!` table into `OUT_DIR`; an absent folder is an
empty table, which `src/ui.rs` reads as "no UI was built". **The diff adds no dependencies at
all.** The paths in that table are absolute, because `include_bytes!` resolves against the
generated file and that file lives in `OUT_DIR`, which has no idea where `ui/dist` is.
`rerun-if-changed` points at a folder that usually is not there, so cargo re-runs the build
script every build until it appears — about a second, and exactly the moment the answer is
about to change. Once it exists, edits to embedded files are caught by rustc through
`include_bytes!` instead.
The placeholder is a **200, not a 404**: the API really is up, and a 404 on `/` reads like a
broken route rather than a UI that was never built. Assets sit **outside the password gate** —
the login form has to render before there is a session to render it with, and a bundle holds
nothing worth protecting. `/api` and `/api/...` are the one thing the fallback refuses to
answer with a page, because a typo'd endpoint returning HTML and a 200 is a bug that looks
like a working request; everything else falls through to the shell, including a missing
asset, since the fallback cannot tell a stale asset URL from a client route. Path traversal
is impossible by construction: nothing reads the filesystem at request time, so a path either
matches a key the compiler put in the table or matches nothing. Content types come from a
twelve-line match rather than a crate. `--no-open` rather than `--open`, because opening the
dashboard is what a laptop wants and the flag is for ssh and containers; the open fires after
the bind, best effort, so a box with no browser still serves. Tests cannot change what was
compiled, so `engine/tests/serve.rs` asserts against `ui::built()` and is meaningful in both
worlds — a checkout with no UI, and one with a dashboard in it.

### S-14 — UI scaffold + org switcher + filter bar + first chart ☑ (PR #14, 61b7300)
**PR:** one. **Depends on:** S-12.
**Files:** `ui/` (new), `.github/workflows/ci.yml` (enable ui job).
**Today:** no UI.
**Change:** `pnpm create vite ui --template react-ts`, add `recharts`. Load the `dataviz`
skill before chart code. Header: org switcher, assistant multi-select, window presets
1d / 5h / 7h, custom since/until, last N (250 / 500 / custom), call ID box. First chart:
stacked bars per bucket, one colour per ended group, raw reason on hover. "—" for NULL.
**Acceptance:** WHEN the window preset changes THEN the chart SHALL refetch `/api/stats` and re-render.
**Verify:** `cd ui && pnpm i && pnpm build` clean; screenshot in PR.
**Must not:** call Vapi from the browser; hold keys in the browser.
**Learned:** The counts do not come from `/api/stats`. It counts ended groups over the whole
selection — `by_ended_group` is one flat map and `per_bucket` carries no breakdown — so it
structurally cannot draw a stack. Fetching stats once per group with `&ended_group=` is up to
twelve requests, gives only selection-wide reasons, and is **wrong** under `last`: the newest
250 calls *of a group* is not that group's share of the newest 250 calls. `/api/calls` returns
the same selection row by row with each row's group and reason on it — one request, exact
under `last`, and the only way to put a bucket's own raw reasons in that bucket's tooltip.
`/api/stats` still supplies the axis (bucket size, every bucket including empty ones, cost per
bucket), so the acceptance holds and the two can never disagree. That is also why the filter
bar **always sends a `last`**: `/api/calls` is a page, and a page with no size is a page of
200 — a cap the reader never asked for and would never see. With one named, the chart can say
when the cap is what ended the selection.
Eleven ended groups, eight hues. `transport`, `start-error` and `other` fold into a grey
residual bucket rather than take a generated ninth colour no colourblind reader could tell
from the eight; grey is chart chrome, not a ninth hue. The fold is fixed, never data-driven,
so a filter that changes which groups are on screen never repaints the ones that stay, and
nothing is hidden by it — the residual is named in the legend, counted in the table, and
hovering it lists the reasons it covers. The hue order came out of running the `dataviz`
palette validator over candidate orderings and keeping one that clears every adjacent-pair
gate in both modes (light CVD ΔE 9.1 / normal 19.6; dark 8.4 / 19.3). Three light-mode hues
sit under 3:1 against the surface; the table view under the chart is the required relief, so
it is not optional decoration. Green went on `customer` on purpose — it is the one hue read as
a verdict, and the normal ending is the only place that verdict is true.
Marks: a 2px surface gap taken off each segment's **own top** so it separates from the one
above; a 4px cap on the topmost segment only, square at the baseline; bars capped at 24px with
the band's leftover left as air; solid hairline gridlines. Interior stacked segments get no
direct labels — they cannot fit them — so the legend, axis, tooltip and table carry the
values. On refetch the previous render holds at reduced opacity, and "stale" is derived by
tagging the chart with the query that produced it rather than kept in a second state.
CI: `pnpm/action-setup` reads `packageManager` from a `package.json`, and this repo has none
at the root, so the job passes `package_json_file: ui/package.json`. The old existence gate is
gone. And `ui/dist` existing at last ran `engine/tests/serve.rs`'s `ui::built() == true` branch
for the first time, against a real Vite build rather than an empty table.
Not done: the chart's hover has no keyboard equivalent — recharts bars are not focusable. The
table view is the reachable twin, and carries every value the tooltip does.

### S-15 — Chart pack A: tools, transfers, latency, cost, duration, per-assistant ☑ (PR #15, 54da7c2)
**PR:** one. **Depends on:** S-14.
**Files:** `ui/src/charts/*.tsx`.
**Today:** one chart.
**Change:** Tool failures by tool name (bar) and per bucket (line); transfers per bucket;
turn latency p50/p95 per bucket with model / voice / transcriber / endpointing averages as a stacked breakdown; cost per bucket with stt/llm/tts/vapi/analysis stack; tokens per call; duration avg;
calls per assistant. All from `/api/stats`.
**Acceptance:** WHEN stats contain `tool_failures_by_name` with 2 tools THEN the tools chart SHALL show 2 bars with those names.
**Verify:** `pnpm build` clean; screenshot.
**Must not:** invent a value for a NULL series.
**Learned:** The engine had the columns and was not aggregating them. `calls` already stored
`cost_stt`…`cost_analysis`, the three token counts and the four `lat_*_avg_ms` components, but
`Totals` carried none of them, so three of the charts this step names could not be drawn from
`/api/stats` at all. `Totals` now carries all fourteen plus `latency_avg`. The step's file list
said `ui/src/charts/*.tsx`; the data it asks for was not there yet.
Averages go through a `Mean` that counts only the calls that carried the number. Averaging a
missing latency in as a zero would drag every component towards a figure nothing measured —
and it would read **lowest** for exactly the calls that went wrong. `by_assistant` is built
from a `GROUP BY`, which yields no percentiles and none of the new breakdowns, so those stay
NULL there with a comment saying so: a chart that starts reading one will show that nothing
measured it, not that it measured nothing.
Six of the seven charts are one component. `Bucketed` is a `ComposedChart` where bars stack
and lines overlay, which covers a pure stack (cost), a pure line (tool failures) and the
latency chart, which is both — the four components add up to the average turn and the p50/p95
are lines over them, in the same milliseconds. One axis throughout. Lines are `type="linear"`,
never `monotone`: a spline between two buckets passes through values nothing measured.
The 2px surface ring goes on a line's dots **only when the chart also has bars**. A p50 drawn
straight onto a stacked fill of nearly its own value disappears into it; on a chart that is
nothing but the line, the same ring chops the stroke into dashes at every bucket.
Palette: the S-14 order with green removed, as `--s-1`…`--s-7`, so a cost slice or a latency
component never wears the one hue read as a verdict — green stays reserved for `customer`.
Dropping the first colour of a validated ordering creates no new adjacent pair; re-validated
anyway (light CVD ΔE 9.1 / normal 19.6, dark 8.4 / 19.3). Each chart maps its own members onto
slots in a fixed order of its own, so an empty slice never repaints the others. `Ranked` has no
colour encoding at all — length is the whole answer, and colouring by rank would repaint the
chart whenever a filter changed the order.
`Ranked` folds past ten names into a grey `other` row that is counted, labelled and says how
many names it covers. A recharts `<Cell>` composes with a custom `shape`, so the fold can wear
its own fill — verified with a fourteen-tool database: 11 bars, the last `var(--g-other)`.
A chart whose measures are all empty says so in words rather than drawing a floor, and the
message is per chart where the reason is specific: a selection can have a cost and no
breakdown of it, and "Vapi reported no cost breakdown for these calls" is a different fact
from "this cost nothing".
`.pack` uses `align-items: start`. Stretching each card to match its neighbours put a field of
empty surface under the short ones, which reads as a chart that failed to draw.
Not done: recharts marks are still not focusable, so the pack inherits S-14's missing keyboard
equivalent for hover. Every chart's table twin carries the same numbers and is reachable.

### S-16 — Chart pack B: Vapi analysis fields ☑ (PR #16, 22396d5)
**PR:** one. **Depends on:** S-15.
**Files:** `ui/src/charts/analysis.tsx`, `engine/src/queries.rs` (structured key counts).
**Today:** successEvaluation and structuredData stored, not shown.
**Change:** successEvaluation counts (pie or bar), and for each `structured_keys` entry
whose values are strings/booleans, a small count chart; numeric keys get avg per bucket.
**Acceptance:** WHEN 5 calls have `structured.intent` values THEN a chart titled `intent` SHALL show their counts.
**Verify:** `pnpm build`; screenshot on seeded data.
**Must not:** spend any model tokens.
**Learned:** `structured_keys` counted calls per key and stopped there, which is enough to
decide a key is worth offering and not enough to draw anything. It was **replaced** by
`structured_fields`: one entry per key, carrying the classification and the one chart that
key can honestly hold. Two fields saying the same thing would have been worse than one
changed field, and nothing consumed the old one yet.
**The engine classifies, the UI draws.** A key is `number` only when *every* value it
carried was one, `text` when they were all scalars, `other` when any was an object or a
list. Classification is over the whole selection, never per value: one string among the
numbers and an average would cover only some of the calls, which is not the average of
anything — so a mixed key is counted instead, which is still true. An `other` key is still
reported, with its call count and words saying its values are not counts or numbers;
dropping it would tell the user the data is not there when it is.
**The tail is folded in the engine, at exactly what the chart shows.** `VALUES_SHOWN = 10`
matches `Ranked`'s `TOP`, so the ten commonest plus one summed remainder arrive as eleven
rows and `Ranked`'s own fold (`length <= TOP + 1`) leaves them alone — no double fold. The
remainder has to be summed here: a chart folding its own tail could only ever sum the
values it was sent, so a key with 200 cities would draw an "other" bar that is wrong. Live:
17 cities → 10 bars + `other (7 more) = 14`, and 22 + 14 = the 36 calls in the selection.
**A numeric key rides the page's axis, not one of its own.** The structured pass runs
*after* `per_bucket` is built and walks its bucket stamps, so alignment is structural
rather than a rule someone has to keep. Buckets no call carried the key in stay NULL — a
gap in the line and a "—" in the table, never a zero. A call with no `created_at` counts
towards the key but sits in no bucket, which is the same rule the time axis already uses.
**`Bucketed` took one new field name, not a generic.** A structured key has no compile-time
name, so its series arrives under `avg` and `Field` gained that one member. Making the
component generic over its field name would have bought nothing and cost the guarantee that
a chart and its table read the same field.
**Every chart here wears slot one.** They are one series each on a card that names them, so
numbering them by list position would mean a key appearing or disappearing repainted its
neighbours — colour following order, the one thing it must never do. `successEvaluation`
gets no colour verdict either: the rubric is the assistant's, so "true", "8" or a sentence
all arrive as strings and painting a guess at which is good would be the dashboard
inventing an opinion. `.pack` moved from `auto-fit` to `auto-fill` so a pack with one card
in it keeps a card's width instead of stretching across the page.
**Not done:** the same keyboard gap as S-14 and S-15 — recharts marks are not focusable, so
hover has no keyboard equivalent and the table twin is the reachable relief.

### S-17 — Chart toggles + saved dashboard layout ☑ (PR #17, a67e762)
**PR:** one. **Depends on:** S-16.
**Files:** `ui/src/Dashboard.tsx`, `engine/src/server.rs` (`GET/PUT /api/dashboard?org=`).
**Today:** all charts always on.
**Change:** "Charts" menu: enable/disable each, drag order; saved to `dashboard` per org.
**Acceptance:** WHEN a chart is disabled and the page reloads THEN it SHALL stay hidden.
**Verify:** `cargo test -q --test dashboard`; `pnpm build`; manual reload.
**Must not:** lose the config on sync.
**Learned:** every card is now one `Entry` — `{id, title, wide?, node}` — and `Pack` and
`Analysis` return lists of them rather than packs of their own. `Dashboard.tsx` draws the
one pack, and is the only file that knows which charts exist. `card(id, title, node)` in
`ui/src/charts/entry.ts` writes each title once, so the menu and the card's heading cannot
come to disagree.

The layout is a preference and nothing else: it names ids and says nothing about what they
draw. That is what lets the two kinds of chart coexist — the fixed ones, always there, and
the structured keys, which exist only while a call in the selection carries them. Two rules
follow, and they are the whole file. **An id the layout has never seen is new, and is
drawn**: a chart added by an upgrade, or a key that arrived with the last sync. Hiding it
would be the dashboard deciding for the reader that a number they have never seen is not
worth seeing. **An id with no chart behind it is remembered anyway**: it shows in the menu
marked "not in this range", is not drawn, and is still settable — narrowing a filter must
not quietly delete a preference. Nothing is written until the reader actually changes
something, so a fresh org's layout stays empty and empty means "draw everything".

The engine checks the layout's shape, never its contents: it cannot know a structured key's
name at compile time, so there is no closed set of ids to check against. It refuses a blank
or over-long id, more than `MAX_CHARTS` (200) of them, and — the one worth refusing outright
— the same id twice, because the dashboard keys its charts by id and two rows claiming one
leave the order of the page undefined. A stored layout that will not parse comes back as the
default rather than a 500: a preference that cannot be honoured must not take every chart
down with it. `?org=` is read by hand rather than through `Filters` — a layout belongs to an
org, not to a selection, and accepting `?window=7h` would say it could differ per range.

Ids are namespaced: a structured key is `structured:<key>`, so a schema with a key called
`cost` in it can never be mistaken for the cost chart.

Dragging is a pointer gesture and nothing else, so every row also carries a pair of real ↑/↓
buttons. Both paths were verified live; the drag by dispatching the drag events the handlers
are wired to, because the browser's own gesture does not fire them under a synthetic mouse.

The must-not is pinned where it would actually break: `a_sync_does_not_lose_the_layout`
writes a layout through the API, then upserts and purges through a second connection the way
a sync does, and reads it back unchanged. The `dashboard` table has been in `0001_init.sql`
since the start, so this step needed no migration.

`.pack > .wide` gives the ended-group chart the full width wherever the reader puts it.

**Not done:** the menu closes only by its own button — no click-outside, no Escape; it is a
disclosure panel above the charts, not a modal. Recharts marks are still not focusable, the
same gap as S-14/S-15/S-16, with the table twin as the reachable relief.

### S-18 — Call table + call drawer ☑ (PR #18, c03837b)
**PR:** one. **Depends on:** S-14.
**Files:** `ui/src/CallTable.tsx`, `ui/src/CallDrawer.tsx`.
**Today:** charts only.
**Change:** Table: created, assistant, duration, ended reason (group colour), tools /
failed, transferred, cost. Row click → drawer with transcript by speaker, tool calls
(name, time, failed, result excerpt), summary, successEvaluation, recording link.
**Acceptance:** WHEN a row with `tool_failures=1` is clicked THEN the drawer SHALL show exactly one tool call marked failed.
**Verify:** `pnpm build`; manual on live data.
**Must not:** embed audio.

**Learned:** every chart on the page was a summary of calls the reader could not see; this
is the calls themselves.

One selection, loaded once. `series.ts` already fetched `/api/calls` to bucket the ended
groups, so `Chart` carries `rows` instead of a count of them and the table reads those.
Two requests for one selection is two chances for the charts and the table to describe
different calls.

The drawer is a `<dialog>` opened with `showModal()`. Escape, the focus trap and the inert
page behind it are all things the browser already implements correctly, and none of them
was worth reimplementing. A backdrop click is tested as a point outside the panel's box,
not as `e.target === dialog`: the backdrop and the panel's own padding report the same
target, so the usual target comparison closes the drawer when the reader clicks its
margin. Both were verified.

The transcript arrives as one string of `Speaker: line` rows. A line whose head is a known
speaker starts a turn and **anything else continues the previous one**, so a colon inside a
sentence never invents a speaker and no line is ever dropped.

A failed tool call is marked twice over — the word `failed` beside its name and a rule down
its edge — never by colour alone. Same rule in the table: the ended-group swatch carries the
colour, the reason carries the words, and text never wears a data hue.

The clickable thing in a row is a real `<button>` carrying the start time, so a keyboard
reaches it while a pointer can hit anywhere along the row.

`showModal()` throws on an already-open dialog and an effect runs twice under StrictMode, so
the open state is checked rather than assumed. The drawer is keyed by call id upstream, so a
second call is a second drawer — the same remount trick S-17 used to keep `setState` out of
an effect.

The must-not held end to end: `audio elements in drawer: 0`, and the recording is an
external link under a line saying graphify stores the link and never the audio.

**Not done:** columns do not sort and the table is not paged — it draws the selection the
filter bar asked for, and `last` is what bounds it. Recharts marks are still not focusable,
unchanged from S-14/S-15/S-16/S-17.

### S-19 — Settings page: "+" add org and keys, test connection ☑ (PR #19, 318a449)
**PR:** one. **Depends on:** S-12, S-14.
**Files:** `ui/src/Settings.tsx`.
**Today:** keys only via env / CLI.
**Change:** Orgs list; "+" → name + Vapi key → `POST /api/orgs` + `PUT secrets/vapi` →
`POST test`. Anthropic / OpenAI keys (global, `org_id NULL`). Shows `set · ••••last4`
only. `keep_days` (≤14) and `max_calls` editable.
**Acceptance:** WHEN a Vapi key is saved THEN the page SHALL show `set` with its last 4 chars and never the full key in any response or DOM.
**Verify:** `pnpm build`; devtools network tab shows no key in any GET.
**Must not:** keep the key in React state after submit.

**Learned:** the step was written as one UI file, and two of the four things it promised
had no engine behind them. The screen was built against what the engine could do plus
what it had to grow, not against what the Files line guessed.

*The key never comes back.* A key goes up and a `Status` comes back — name, flag, four
characters — so there is no response on this page whose *shape* could carry a value. The
field is uncontrolled and is cleared **before** the request leaves, not after it returns,
so the value is in one local variable for the length of one `await` and in neither React
state nor the DOM afterwards. A failed save costs a retype; that is the cheaper half.

*Global secrets, and the two NULL traps.* The model keys are billed to one account and
spent on every org's calls, so they belong to the install: `org_id NULL`, behind
`GET /api/secrets` and `PUT /api/secrets/{name}`. SQLite counts NULLs in a `PRIMARY KEY`
as **distinct**, so `PRIMARY KEY (org_id, name)` does not constrain those rows at all —
`0002_global_secrets.sql` adds `CREATE UNIQUE INDEX … ON secrets (name) WHERE org_id IS
NULL`, without which replacing a global key leaves its predecessor behind and `get` may
return either. And the lookup is `org_id IS ?1`: `= NULL` is NULL, so the query could
never find the row the same code had just written. The AAD keeps spelling an org's
binding exactly as before (`{id}:{name}`, and `global:{name}` for the new scope), so
every key already in a store keeps decrypting.

*Each name has one scope.* `ORG_NAMES` and `GLOBAL_NAMES` replaced the single `NAMES`
list; `status` reports the names of the scope it was asked about, and a name put at the
wrong scope is a 400 rather than a row nothing will look for.

*Retention.* `PUT /api/orgs/{id}` writes both `keep_days` and `max_calls` every time —
both are nullable, so a request carrying one could not tell "leave it" from "clear it".
D-5 is a cap: above 14 is refused by the field's own `max` before anything is sent, and
by the engine again if it were.

Verified in Chrome against a live server on a fresh database: `stored reads: "set ·
••••1234"`, `key in DOM: false`, `password input values: ["","",""]`, `responses that
carried a key: 0` over eight API responses, `console errors: []`. The plaintext keys are
absent from the database file.

**Not done:** no delete and no rename — a name is what an org is known by everywhere
else. The "+" flow leaves an org created even when its key fails the test, which is
right, but there is no second chance at the key inside that form.

### S-20 — Brain scaffold with BAML clients and cost table ☐
**PR:** one. **Depends on:** S-2.
**Files:** `brain/baml_src/clients.baml`, `brain/baml_src/generators.baml`,
`brain/src/graphify_brain/cost.py`, `brain/src/graphify_brain/db.py`, `brain/tests/test_cost.py`.
**Today:** empty Python package.
**Change:** Load the `claude-api` skill for current model ids and prices. Clients:
`Opus`, `Sonnet`, `GPT` with keys from env (engine passes them as env when spawning).
`cost.estimate(tokens_in, tokens_out, model) -> usd`. `db.py` opens the same SQLite
read-only for calls, read-write for `jobs`/`patterns`. `baml-cli generate` in `uv run`.
**Acceptance:** WHEN `estimate(100_000, 2_000, "sonnet")` runs THEN it SHALL return a positive USD matching the price table in the file.
**Verify:** `cd brain && uv run baml-cli generate && uv run pytest -q` → pass.
**Must not:** make a network call in tests.

### S-21 — Rule engine + `graphify apply` + `graphify rule-check` `[Rust]` ☐
**PR:** one. **Depends on:** S-8.
**Files:** `engine/src/rules.rs`, `engine/src/cli.rs`, `engine/tests/rules.rs`.
**Today:** nothing.
**Change:** `Rule` struct = the DSL; `validate` rejects unknown keys and bad regex with
the pattern name; `matches(&Rule, &CallRow, &[ToolCall]) -> bool`. `apply` re-runs all
`mode=free` patterns over all calls into `pattern_matches(source='rule')`. `rule-check
--rule FILE --calls FILE` prints matched ids (brain uses it for agreement).
**Acceptance:** WHEN rule `{"any_phrases":["real human"],"speaker":"user"}` runs on a call where only the bot says it THEN `matches` SHALL be false; prove by removing the speaker filter → exactly that test fails.
**Verify:** `cargo test -q --test rules` → pass.
**Must not:** eval anything; import the brain.

### S-22 — BAML `PlanPattern` + `ClarifyPattern` ☐
**PR:** one. **Depends on:** S-20.
**Files:** `brain/baml_src/plan.baml`, `brain/src/graphify_brain/plan.py`, `brain/tests/test_plan.py`.
**Today:** nothing.
**Change:** Inputs per Brain functions table. Output class `Plan { rows: Row[] {if_, then},
questions: string[], confidence: float, expressible: bool, reason: string }`. CLI
`graphify-brain plan` / `clarify` (stdin JSON → stdout JSON). Tests mock the BAML client.
**Acceptance:** WHEN a mocked model returns confidence 0.7 with 2 questions THEN the CLI SHALL output them unchanged and exit 0.
**Verify:** `uv run pytest -q tests/test_plan.py`.
**Must not:** call a model in tests.

### S-23 — BAML `LabelBatch` with batched loop + cost gate ☐
**PR:** one. **Depends on:** S-22.
**Files:** `brain/baml_src/label.baml`, `brain/src/graphify_brain/label.py`, `brain/tests/test_label.py`.
**Today:** nothing.
**Change:** `label` CLI: input `{criterion, plan, call_ids, model, batch_size=20,
max_usd}`; estimates first (`ESTIMATE {usd}` on stdout then waits for `GO` on stdin unless
`--yes`); loops batches with concurrency 3, `PROGRESS n/m` on stderr; writes
`pattern_labels`; stops if running cost > `max_usd`. Output: labels + total cost.
**Acceptance:** WHEN 45 calls are given with batch 20 THEN exactly 3 model calls SHALL be made; WHEN stdin never says GO THEN zero model calls.
**Verify:** `uv run pytest -q tests/test_label.py` (mocked client counts calls).
**Must not:** exceed `max_usd`.

### S-24 — BAML `SynthesizeRule` + `RefineRule` + agreement via `rule-check` ☐
**PR:** one. **Depends on:** S-23, S-21.
**Files:** `brain/baml_src/rule.baml`, `brain/src/graphify_brain/synth.py`, `brain/tests/test_synth.py`.
**Today:** labels only.
**Change:** `synthesize` CLI: labels+evidence → rule + chart suggestion; runs
`graphify rule-check` on the sample; `agreement` = matches == labels fraction; if < 0.85
one `RefineRule` call with the disagreements; stores `patterns` row. Output: rule,
agreement, chart.
**Acceptance:** WHEN labels have 40 matches and the rule matches 38 of those plus 2 others THEN agreement SHALL be reported as 0.984 (246/250).
**Verify:** `uv run pytest -q tests/test_synth.py` with a fake `rule-check` on PATH.
**Must not:** execute anything returned by the model.

### S-25 — Engine spawns brain jobs; `/api/patterns/*` with progress `[Rust]` ☐
**PR:** one. **Depends on:** S-24, S-12.
**Files:** `engine/src/jobs.rs`, `engine/src/server.rs`, `engine/tests/jobs.rs`.
**Today:** brain is CLI-only.
**Change:** `jobs.rs` spawns `graphify-brain <fn>` (path from `GRAPHIFY_BRAIN` or
`PATH`) with keys in env, streams stderr `PROGRESS` into `jobs.log`, stores output.
Routes: `POST /api/patterns/plan`, `/clarify`, `/label` (returns job id; `GO` sent only
after `POST /api/jobs/{id}/go`), `/synthesize`, `GET /api/jobs/{id}` (status, progress,
cost), `GET /api/patterns?org=`, `PUT /api/patterns/{id}` (rule, mode, cap),
`POST /api/patterns/{id}/apply`. Two things (spawn + routes) in one step because the
routes cannot be verified without the spawn.
**Acceptance:** WHEN `/label` is called and `/go` never is THEN the job SHALL stay `waiting` and `spend` SHALL be unchanged.
**Verify:** `cargo test -q --test jobs` with a fake brain script.
**Must not:** pass keys as argv (env only).

### S-26 — UI pattern wizard: config step, chat step, plan table, ≥95% gate, cost go ☐
**PR:** one. **Depends on:** S-25, S-18.
**Files:** `ui/src/patterns/Wizard.tsx`, `ui/src/patterns/PlanTable.tsx`.
**Today:** no pattern UI.
**Change:** Step 1 config (Vapi): assistants multi-select, calls = last N or pasted IDs,
"read the agent's prompt" toggle, model select. Step 2 chat: criterion → plan table on
the right updates with each answer; "Read N calls · ~$X" button enabled only when
confidence ≥ 0.95 and `expressible`; progress bar from the job; result: rule JSON,
agreement, suggested chart, "Save".
**Acceptance:** WHEN confidence is below 0.95 THEN the read button SHALL be disabled; WHEN it is clicked THEN `/go` SHALL be called exactly once.
**Verify:** `pnpm build`; manual walkthrough with a real key after Abhishek's go (Sonnet, sample 25).
**Must not:** show a key; start spend without the click.

### S-27 — Patterns list, pattern chart, edit rule, mode + cap, re-apply ☐
**PR:** one. **Depends on:** S-26.
**Files:** `ui/src/patterns/List.tsx`, `ui/src/charts/pattern.tsx`.
**Today:** patterns created but not browsable.
**Change:** Sidebar list with counts under current filters; per-pattern chart (type from
the suggestion, default line per bucket); click filters the call table; edit rule JSON
with validate; mode select free / hybrid / full with `daily_cap_usd`; "Re-apply".
**Acceptance:** WHEN a rule is edited to match nothing and re-applied THEN its count SHALL read 0 and the chart SHALL be empty.
**Verify:** `pnpm build`; manual.
**Must not:** spend on re-apply in `free` mode.

### S-28 — Daily hybrid/full modes with spend cap ☐
**PR:** one. **Depends on:** S-27, S-23.
**Files:** `brain/src/graphify_brain/daily.py`, `engine/src/sync.rs`.
**Today:** free mode only.
**Change:** After `apply`, engine spawns `graphify-brain daily` once: for `hybrid`, calls
that the rule matched since last run and are unlabeled → `LabelBatch` confirm; for
`full`, all new calls → `LabelBatch`. Writes `pattern_matches(source='llm')`, adds to
`spend`, stops at the pattern's cap and at a global `GRAPHIFY_DAILY_CAP_USD` (default 5).
**Acceptance:** WHEN the cap is $0.01 THEN at most one batch SHALL run and the job log SHALL say `cap reached`.
**Verify:** `uv run pytest -q tests/test_daily.py`; `cargo test -q --test sync_daily`.
**Must not:** run without a cap.

### S-29 — Ask box (BAML `AskAnalysis`) ☐
**PR:** one. **Depends on:** S-25, S-17.
**Files:** `brain/baml_src/ask.baml`, `brain/src/graphify_brain/ask.py`, `engine/src/server.rs` (`POST /api/ask`), `ui/src/Ask.tsx`.
**Today:** no free-form analysis.
**Change:** Input: current filters → engine builds `stats` + up to 20 sample transcripts
(shortest first, capped at 60k tokens) → estimate → confirm → answer markdown. No history.
**Acceptance:** WHEN the user cancels at the cost step THEN no job SHALL be created.
**Verify:** tests for the three layers with mocks; manual once live.
**Must not:** send more than the cap.

### S-30 — PDF downloads: dashboard, per pattern, call list ☐
**PR:** one. **Depends on:** S-27, S-18.
**Files:** `ui/src/pdf/*.ts`, `ui/package.json` (`jspdf`, `html-to-image`).
**Today:** none.
**Change:** Three buttons. Dashboard PDF = header (org, assistants, window, filters) +
every enabled chart as image. Pattern PDF = criterion, plan table, rule, agreement,
count, 20 matched calls with evidence. Call-list PDF = the current table.
**Acceptance:** WHEN "Download dashboard PDF" is clicked THEN a PDF SHALL download containing one image per enabled chart.
**Verify:** `pnpm build`; open the file.
**Must not:** need a server dependency.

### S-31 — `graphify schedule --print` + README daily section ☐
**PR:** one. **Depends on:** S-28.
**Files:** `engine/src/cli.rs`, `README.md`.
**Today:** manual sync.
**Change:** Prints a crontab line and a launchd plist for `graphify sync --org all` at
06:00 (sync → assistants → apply → daily). `--install` asks y/n before writing.
**Acceptance:** WHEN `--print` runs THEN the crontab line SHALL contain the absolute binary path.
**Verify:** run it; install on Abhishek's machine; next-day log.
**Must not:** install without confirm.

### S-32 — Dockerfile + docker-compose + password mode ☐
**PR:** one. **Depends on:** S-13, S-20, S-31.
**Files:** `Dockerfile`, `docker-compose.yml`, `README.md`.
**Today:** local only.
**Change:** Multi-stage: ui build → cargo build → python slim with `uv` + brain +
engine binary. Volume `/data`. Env: `GRAPHIFY_PASSWORD`, `GRAPHIFY_SECRET`,
`GRAPHIFY_BIND=0.0.0.0:3737`. Cron inside the container via `supercronic`.
**Acceptance:** WHEN `docker compose up` runs with `GRAPHIFY_PASSWORD` THEN `http://localhost:3737` SHALL show the login page.
**Verify:** build + run locally; screenshot.
**Must not:** bake any key into the image.
