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
**Verify:** `cargo test -q db` → pass on a tempfile DB.
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

### S-6 — endedReason → group `[Rust]` ☐
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
**Verify:** `cargo test -q ended_reason` with ≥ 15 cases covering all 11 groups → pass.
**Must not:** network.

### S-7 — Vapi client: read-only paginated `GET /call` `[Rust]` ☐
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
**Verify:** `cargo test -q vapi` → pass.
**Must not:** any non-GET; log the key.

### S-8 — Extract: raw call → `calls` row + `tool_calls` + slim JSON `[Rust]` ☐
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
**Verify:** `cargo test -q extract` → pass.
**Must not:** store raw; download recordings; coerce missing to 0.

### S-9 — `graphify sync --org NAME --last N | --since DATE`, incremental + purge `[Rust]` ☐
**PR:** one. **Depends on:** S-7, S-8.
**Files:** `engine/src/cli.rs`, `engine/src/sync.rs`, `engine/tests/sync.rs`.
**Today:** CLI prints version only.
**Change:** Key from `secrets` (S-11) or env `VAPI_API_KEY` (env wins). If DB has calls for
the org, default `since` = newest `created_at` → re-runs add only new calls; `--last 500`
on 250 stored fetches up to 250 more, never replaces. After upsert: delete calls older
than `orgs.keep_days` (≤ 14, reject higher) and beyond `orgs.max_calls` if set. Print
`org X: synced N new, M total, purged P`.
**Acceptance:** WHEN `sync --last 250` runs twice against the same mock THEN the second run SHALL print `0 new` and the table SHALL hold 250 rows; WHEN `keep_days` is 20 THEN sync SHALL refuse with a message naming the 14-day cap.
**Verify:** `cargo test -q sync` → pass; live once with a real key after Abhishek's go: `graphify sync --org test --last 25`.
**Must not:** non-GET; delete rows inside the keep window.

### S-10 — Assistants + tools: slim fetch into `assistants` and `tools` `[Rust]` ☐
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
**Verify:** `cargo test -q assistants` → pass; live: `graphify assistants --org rush` prints 100 names.
**Must not:** non-GET; store the raw assistant.

### S-11 — Secrets store: encrypted at rest, env override, never returned `[Rust]` ☐
**PR:** one. **Depends on:** S-5.
**Files:** `engine/src/secrets.rs`, `engine/tests/secrets.rs`.
**Today:** env vars only.
**Change:** `chacha20poly1305` with key from `GRAPHIFY_SECRET` (32 bytes, base64) else
`data/.secret` created with mode 0600. `set(org, name, value)`, `get(org, name) ->
Option<String>` (env `VAPI_API_KEY` / `ANTHROPIC_API_KEY` / `OPENAI_API_KEY` override),
`status(org) -> [{name, set, last4}]`. `Debug` impl for the secret type prints `***`.
**Acceptance:** WHEN a key is set THEN the DB file SHALL not contain its plaintext bytes, AND `status` SHALL show `set: true` with the last 4 chars only.
**Verify:** `cargo test -q secrets` → pass (test greps the DB file for the plaintext and asserts absent).
**Must not:** log values.

### S-12 — HTTP API (axum): orgs, assistants, calls, stats, optional password `[Rust]` ☐
**PR:** one. **Depends on:** S-9, S-10, S-11.
**Files:** `engine/src/server.rs`, `engine/src/queries.rs`, `engine/src/auth.rs`, `engine/tests/server.rs`.
**Today:** CLI only.
**Change:** `axum`. Routes: `GET /api/orgs`, `POST /api/orgs`, `GET /api/orgs/{id}/secrets`,
`PUT /api/orgs/{id}/secrets/{name}` (body `{value}`), `POST /api/orgs/{id}/test`
(one GET /assistant with that key → ok/err), `GET /api/assistants?org=`,
`GET /api/calls?…`, `GET /api/calls/{id}`, `GET /api/stats?…`. Shared filters: `org`,
`assistant_id` (repeatable), `since`, `until`, `window` (`1d|5h|7h|<n>h|<n>d`), `last`,
`ended_group`, `call_id`, `tool_failed`, `transferred`. `/api/stats` returns
`{by_ended_group, by_ended_reason, per_bucket:{calls, tool_failures, transfers,
cost, duration_avg, latency_p50, latency_p95}, tool_failures_by_name,
by_assistant, success_eval_counts, structured_keys, totals}`; bucket 1h ≤ 2d else 1d.
`GRAPHIFY_PASSWORD` set → `POST /api/login`, cookie session, all `/api/*` 401 without it.
Bind `127.0.0.1:3737` unless `GRAPHIFY_BIND` set.
**Acceptance:** WHEN 10 calls are seeded, 3 in `transfer-error`, THEN `/api/stats?window=1d` SHALL report `by_ended_group["transfer-error"] == 3`; WHEN `GRAPHIFY_PASSWORD` is set THEN `/api/stats` without a session SHALL return 401.
**Verify:** `cargo test -q server` → pass; `graphify serve` + `curl localhost:3737/api/stats?org=1`.
**Must not:** return secret values; bind 0.0.0.0 by default.

### S-13 — `graphify serve` serves `ui/dist` and opens the browser `[Rust]` ☐
**PR:** one. **Depends on:** S-12.
**Files:** `engine/src/server.rs`, `engine/build.rs`, `engine/Cargo.toml`.
**Today:** API only.
**Change:** `rust-embed` embeds `ui/dist` at build time when present (empty page with
"UI not built" otherwise). `serve --no-open` flag; default opens the browser with `open`.
**Acceptance:** WHEN `ui/dist/index.html` exists at build time THEN `GET /` SHALL return it; otherwise a 200 placeholder.
**Verify:** `cargo test -q serve`; manual curl.
**Must not:** call Vapi.

### S-14 — UI scaffold + org switcher + filter bar + first chart ☐
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

### S-15 — Chart pack A: tools, transfers, latency, cost, duration, per-assistant ☐
**PR:** one. **Depends on:** S-14.
**Files:** `ui/src/charts/*.tsx`.
**Today:** one chart.
**Change:** Tool failures by tool name (bar) and per bucket (line); transfers per bucket;
turn latency p50/p95 per bucket with model / voice / transcriber / endpointing averages as a stacked breakdown; cost per bucket with stt/llm/tts/vapi/analysis stack; tokens per call; duration avg;
calls per assistant. All from `/api/stats`.
**Acceptance:** WHEN stats contain `tool_failures_by_name` with 2 tools THEN the tools chart SHALL show 2 bars with those names.
**Verify:** `pnpm build` clean; screenshot.
**Must not:** invent a value for a NULL series.

### S-16 — Chart pack B: Vapi analysis fields ☐
**PR:** one. **Depends on:** S-15.
**Files:** `ui/src/charts/analysis.tsx`, `engine/src/queries.rs` (structured key counts).
**Today:** successEvaluation and structuredData stored, not shown.
**Change:** successEvaluation counts (pie or bar), and for each `structured_keys` entry
whose values are strings/booleans, a small count chart; numeric keys get avg per bucket.
**Acceptance:** WHEN 5 calls have `structured.intent` values THEN a chart titled `intent` SHALL show their counts.
**Verify:** `pnpm build`; screenshot on seeded data.
**Must not:** spend any model tokens.

### S-17 — Chart toggles + saved dashboard layout ☐
**PR:** one. **Depends on:** S-16.
**Files:** `ui/src/Dashboard.tsx`, `engine/src/server.rs` (`GET/PUT /api/dashboard?org=`).
**Today:** all charts always on.
**Change:** "Charts" menu: enable/disable each, drag order; saved to `dashboard` per org.
**Acceptance:** WHEN a chart is disabled and the page reloads THEN it SHALL stay hidden.
**Verify:** `cargo test -q dashboard`; `pnpm build`; manual reload.
**Must not:** lose the config on sync.

### S-18 — Call table + call drawer ☐
**PR:** one. **Depends on:** S-14.
**Files:** `ui/src/CallTable.tsx`, `ui/src/CallDrawer.tsx`.
**Today:** charts only.
**Change:** Table: created, assistant, duration, ended reason (group colour), tools /
failed, transferred, cost. Row click → drawer with transcript by speaker, tool calls
(name, time, failed, result excerpt), summary, successEvaluation, recording link.
**Acceptance:** WHEN a row with `tool_failures=1` is clicked THEN the drawer SHALL show exactly one tool call marked failed.
**Verify:** `pnpm build`; manual on live data.
**Must not:** embed audio.

### S-19 — Settings page: "+" add org and keys, test connection ☐
**PR:** one. **Depends on:** S-12, S-14.
**Files:** `ui/src/Settings.tsx`.
**Today:** keys only via env / CLI.
**Change:** Orgs list; "+" → name + Vapi key → `POST /api/orgs` + `PUT secrets/vapi` →
`POST test`. Anthropic / OpenAI keys (global, `org_id NULL`). Shows `set · ••••last4`
only. `keep_days` (≤14) and `max_calls` editable.
**Acceptance:** WHEN a Vapi key is saved THEN the page SHALL show `set` with its last 4 chars and never the full key in any response or DOM.
**Verify:** `pnpm build`; devtools network tab shows no key in any GET.
**Must not:** keep the key in React state after submit.

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
**Verify:** `cargo test -q rules` → pass.
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
**Verify:** `cargo test -q jobs` with a fake brain script.
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
**Verify:** `uv run pytest -q tests/test_daily.py`; `cargo test -q sync_daily`.
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
