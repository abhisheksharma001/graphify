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

- **D-13 An agent analyst platform, Vapi first.** The product is analytics for AI agents,
  not a Vapi dashboard. Vapi is the first connector and not the subject: other voice
  platforms follow, and whether the unit ever widens from a call to a session — text and
  chat agents, n8n runs, assistants with no audio at all — is **left open here and not
  foreclosed**. What that costs today is already known and small. Three files are the
  connector — `vapi.rs` (the read-only HTTP client), `extract.rs` (their JSON to a row),
  `ended_reason.rs` (their vocabulary to the eleven groups of D-11) — and everything
  downstream reads the normalised row instead: `queries.rs`, `rules.rs`, the whole UI, the
  whole brain, none of which know where a call came from. `calls` is a generic call record,
  `orgs.provider` already exists and is already carried on `Org`, and `secrets` is keyed
  `(org_id, name)`, so a second provider's key is another name and not a schema change.
  **The constraint this puts on every step: no new provider assumption may enter
  `queries.rs`, the UI, or the brain.** Anything Vapi-shaped belongs in one of the three
  files above, and anything a chart needs belongs in a column whose meaning is normalised —
  `ended_reason` beside `ended_group` is the pattern. No provider trait until there is a
  second provider to shape it against; one implementation shapes an abstraction wrong.

## Must never (every step inherits these)
- Send anything but GET to a provider. Vapi is the one there is (`vapi.rs`, enforced by a
  test that greps the source); every connector after it inherits this.
- Call a model without a shown cost and an explicit go (`--yes` / click). Daily modes
  have a hard USD cap and stop when reached.
- Download or store audio. Recording URL only.
- Return a key to the browser, print one to a log, or write one to the DB in clear.
- Render a missing value as 0. NULL → "—".
- Put a provider's vocabulary or JSON shape anywhere but `vapi.rs`, `extract.rs` and
  `ended_reason.rs`. Downstream reads normalised columns only (D-13).

## Deliberately does not do (v1)
- No scoring / evals / "was this call good".
- No Retell, ElevenLabs, other providers (schema has `provider`, code has one). This is
  a v1 scope line, not the shape of the product — see D-13.
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

### S-20 — Brain scaffold with BAML clients and cost table ☑ (PR #20, c3f74ae; PR #21, 5a197c3)
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
**Learned:** the `claude-api` skill's price table is cached and was four months stale,
so the prices were read from the vendors' own pages instead and the module records the
day it read them (`PRICES_CHECKED`) and the URLs — the one failure mode no test can
catch. Clients are `Opus` → `claude-opus-5` ($5/$25 per MTok), `Sonnet` →
`claude-sonnet-5` ($2/$10), `GPT` → `gpt-5.6-terra` ($2/$12).

`clients.baml` and the price table are two files holding one fact, so `test_cost.py`
reads the BAML file and fails if a model it names is unpriced. Proved by repointing `GPT`
at `gpt-5.6-luna`: `Extra items in the right set: 'gpt-5.6-terra'`.

The estimate is deliberately the **ceiling** — every input token at the base rate, prompt
caching ignored — so a real call comes in under the approved number and never over it. A
cap built on an under-estimate is not a cap. An unpriced model raises rather than costing
zero, for the same reason a missing value renders "—" and not 0.

`estimate` keys on the *client name* (`"sonnet"`), because a job records which client it
ran on, not which model id that client was pointed at; the model id resolves too, since
`patterns.model` may hold either.

`baml_client/` is gitignored, so nothing under `src/` may import it and CI never
generated it — a syntax error in `baml_src/` would have shipped green. CI's brain job
gained `uv run baml-cli generate`; a malformed client file exits 4.

`db.py` uses `mode=ro` / `mode=rw`, never `rwc`: read-only is enforced by SQLite rather
than by convention, and a wrong `--db` path is an error at the point it is passed instead
of an empty database whose every later query fails with "no such table". `busy_timeout`
is set because the engine leaves SQLite in rollback-journal mode, where a `sync` writing
while a job reads is a normal collision.

**Beyond the file list:** `tests/test_db.py` (the module guaranteeing the brain cannot
edit a client's call history needs a test that tries) and the CI step above.

**Follow-up (PR #21, 5a197c3):** prices cannot be auto-updated — neither provider
publishes rates through an API. `GET /v1/models` returns ids, capabilities, context
windows and a release date, and no price at either. So `graphify-brain models` prints the
table and its age, and `--check` reads both providers' model lists with the key already in
the environment (no model call, no cost) to report a **retired** configured model (exit 1,
it breaks the brain) and any model released **after** the one a client points at (exit 0 —
news, not a failure). A provider with no key is named, never counted as a pass.
`STALE_AFTER_DAYS = 90` warns and never fails: a build that breaks on a calendar date with
no change to the code teaches everyone to skip the check. `Price` gained `provider`, which
let the drift guard tighten from loose `provider`/`model` lines to whole `client<llm> {}`
blocks matched against client name, provider and model id together.

**Not done:** no `Haiku` client — D-8's "cheap model confirms" will need one, and the step
named three. Cache-read and batch rates are not priced; the ceiling covers the caps but
over-states a cached run. `models --check` is not a CI step: it needs keys and CI holds
none. Nothing imports `baml_client` yet: the first function arrives in S-22.

### S-21 — Rule engine + `graphify apply` + `graphify rule-check` `[Rust]` ☑ (PR #22, b06003c)
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

**Learned:** the DSL needed three decisions the spec left open, and each one is a way a
count can be quietly wrong. **A list means "any of these"** everywhere, which makes
`tool_not_called` the plain negation of `tool_called` instead of a second rule to learn.
**Regexes compile case-insensitively** (`RegexBuilder::case_insensitive`, `(?-i)` to opt
out): a transcript is speech recognition output, and a rule written in lower case missing
a capitalised sentence would look like a rule that is merely too narrow. **NULL stays
unknown** — a call with no transcript answers no question about words, and a `transferred`
nobody recorded matches neither `true` nor `false`, the same rule as the dashboard's `—`
and never `0`.

`validate` takes the JSON and the pattern name and returns a `Checked`: parse, speaker
word, empty phrase, and every regex compiled once. The name opens every message, because
the caller that hits these is `apply`, running patterns it did not write. An empty phrase
is refused outright — it is a substring of every line, so it matches the whole org while
looking selective. `deny_unknown_fields` on `Rule` **and** on the `--calls` file: both are
contracts with the brain, and a `transcripts` where `transcript` was meant would answer
every question with "no".

The turn parser is `ui/src/CallDrawer.tsx` transliterated, deliberately word for word —
same six speaker words, same "anything else continues the previous turn". The drawer a
reader checks a match against and the rule that produced it have to agree on who said
what. `system` is a real speaker that is neither `user` nor `bot`, so a rule about the
user passes over the system line rather than guessing.

Signature deviates from what this step wrote down: `matches(&Checked, &Subject)`, not
`matches(&Rule, &CallRow, &[ToolCall])`. `Checked` exists so a broken rule fails once at
load rather than returning a quiet `false` on every call in the org, and so `apply`
compiles each regex once instead of once per call; `Subject` carries its own tool calls
because the `--calls` file does, and a second parallel type bought nothing. `Subject` is
also six columns, not a `calls` row: a rule has no business seeing costs or the slim blob,
and `apply` should not read them off disk to find that out. Calls are loaded once per org,
not once per pattern — the transcript column is the biggest thing in that query.

`apply` is free-mode only, by definition: those are the patterns a rule alone decides.
Hybrid and full have a model in the loop and a daily cap on it, and re-running one is
spending money. It replaces its own `source='rule'` rows and never touches `source='llm'`,
which was paid for once. A pattern with no rule or no org is skipped — a row someone is
halfway through creating is not a mistake — while a pattern whose rule is broken is an
error naming it.

`rule-check` reads two files and no database, which is what makes it usable by the brain
in S-24: the brain writes the calls it labelled, the engine says which ones the rule
agrees with, and the engine stays the only thing with an opinion about what a rule means.

Acceptance proved by breaking it: removing the speaker filter from `text_hit` fails the
acceptance test and two others, and restoring it returns 31 green. `regex` is the one new
dependency; it has no backreferences and no backtracking, so a rule a model wrote cannot
be made to run for ever, and a 1 MB `size_limit` caps what one can compile to.

**Not done:** phrases and regexes see one turn at a time, so neither can span a speaker
change. `apply` has no `--org` and re-runs everything, which is right at a few thousand
calls and will not be at a million. Nothing calls `apply` on a schedule or after a sync —
it is a command a person runs. The UI cannot see a pattern or its matches yet; the
`patterns` rows tested here were written by hand, because nothing creates one until S-24.

### S-22 — BAML `PlanPattern` + `ClarifyPattern` ☑ (PR #23, 825de65)
**PR:** one. **Depends on:** S-20.
**Files:** `brain/baml_src/plan.baml`, `brain/src/graphify_brain/plan.py`, `brain/tests/test_plan.py`.
**Today:** nothing.
**Change:** Inputs per Brain functions table. Output class `Plan { rows: Row[] {if_, then},
questions: string[], confidence: float, expressible: bool, reason: string }`. CLI
`graphify-brain plan` / `clarify` (stdin JSON → stdout JSON). Tests mock the BAML client.
**Acceptance:** WHEN a mocked model returns confidence 0.7 with 2 questions THEN the CLI SHALL output them unchanged and exit 0.
**Verify:** `uv run pytest -q tests/test_plan.py`.
**Must not:** call a model in tests.

**Learned:** **neither function grades its own answer.** A plan at 0.7 confidence with two
questions is printed as it stands and exits 0 — that is the acceptance test, and a brain
that withheld it would leave the analyst nothing to answer. The gate that will not spend
below 0.95 lives in the wizard, where the person spending can see it.

**Two deviations from the register's input table**, both because the model would otherwise
be grading an answer it cannot check. `ClarifyPattern` is given the **criterion**: a `Plan`
holds rows, questions and a reason, and none of those is the sentence an answer has to be
judged against. It is also given the **DSL**: an answer can add a row the DSL cannot
express, and `expressible` is the flag the wizard opens its spend button on, so a clarify
that could not see the DSL could only carry the old flag forward.

**The DSL description is a constant in `plan.py`, not an input.** There is one rule DSL and
`engine/src/rules.rs` decides what it means; a description somebody typed into a request
would promise conditions the engine cannot check. It is passed as a function argument
rather than baked into the prompt so a test can assert the model was shown it — and that
test earns its place: deleting `{{ dsl }}` from the prompt fails it while `baml-cli
generate` stays green. The description ends by naming what the DSL *cannot* do (turns,
silence, tone, comparing calls), because `expressible` means nothing unless the model has
been told where the edge is.

**`if_` the whole way through** — BAML class, JSON, engine, UI. `if` is a Python keyword,
so one hop would have to rename it, and a field with two names is a field somebody reads
under the wrong one. `_envelope` refuses an unknown key for the S-21 reason: a `criteria`
for `criterion` would reach the model as no criterion at all and it would invent one.

**Every model call goes through `plan.client()`, the one seam.** Tests that expect a call
install a `Recorder`; tests that expect none install `Never`, which raises on any attribute
access. `Never` caught two real bugs — `client().PlanPattern(...)` resolves the function
before evaluating its arguments, so validation now happens on its own lines first. Both
prompts are also rendered with no key and no network (`b.request.*` builds the request
without sending it), which is the only way to catch a template that no longer carries the
DSL.

**The generated client moved to `brain/src/baml_client/`** — the one directory the editable
install puts on `sys.path`. In `brain/` it was importable only from a process whose working
directory happened to be `brain/`, and S-25 spawns this program from wherever the engine
was started. Still gitignored, still generated in CI before pytest. `plan.py` imports it
inside the function, so `graphify-brain version` runs on a fresh checkout.

**Not done:** **neither call is metered** — `plan` and `clarify` spend real money per
message in the wizard's chat, no cost is reported, and no daily cap counts them; the
estimate/GO handshake arrives in S-23 for labelling, where the spend is large, and these
two are a few cents each with the send button as the explicit go. Extra keys *inside* a
submitted `plan` object are ignored rather than refused (pydantic's default; the envelope
around it does refuse them). `--db` is opened and closed by both commands but no row is
read — it is checked so a wrong path fails at the first step of the wizard instead of
after the labelling is paid for. No `Haiku` client yet.

### S-23 — BAML `LabelBatch` with batched loop + cost gate ☑ (PR #24, c840feb)
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

**Learned:** most of this step is not about labelling. It is four rules standing between a
criterion and a bill, and each one is a test. **The price is shown first** — `ESTIMATE
{usd}` on stdout before anything is read, and `--yes` skips the wait, not the showing.
**Nothing is spent without being told to** — `GO`, exactly, on its own line; EOF, silence,
`y` and `yes` are all a no, and the output says `"stopped": "declined"` with every id in
`not_reached`. Exit 0: a person declining to spend is not a failure. **The cap is checked
before each batch, never after**, because checking afterwards is finding out that the cap
was passed. `max_usd` is required and has no default — a cap a caller can leave out is a
cap that gets left out. **What is paid for is kept**: labels are written to
`pattern_labels` after every wave and after a wave that failed, using `as_completed`
rather than `map` so one batch falling over does not throw away the two beside it that
arrived and were charged for.

**The output half of the estimate is a bound, not a guess.** BAML sends `max_tokens` with
every call, so a batch cannot return more than that however much it wants to;
`MAX_OUTPUT_TOKENS` is that number and a test reads it back out of a rendered request, so
a change in BAML's default fails a test instead of quietly turning the cap into an
approximation. Input is priced at three characters per token against speech nearer four —
erring high, but an estimate and not a bound. The consequence is real and worth knowing
before S-26 draws it on a button: for short calls the estimate is dominated by the output
ceiling and reads several times what the run actually costs. Two calls estimate at
$0.0428 and cost a fraction of a cent. The true figure comes back in `usd`.

**Two deviations from the register's input table**, both so that the labels mean
something. `pattern_id` is **optional**: in the wizard there is no pattern at labelling
time — S-24 writes the `patterns` row out of these very labels — so a run without one
returns its labels and stores nothing, and S-28's daily runs pass an id. And each
transcript is introduced by **a line of what the system recorded** — duration, ended
reason and group, transfer, tools run and tools failed. A plan row can be about something
nobody says out loud ("calls where the booking tool failed", "calls under thirty
seconds"), and those are exactly the calls S-24 measures its rule against. Every unknown
is a dash, the prompt says a dash is not a zero, and `tool_calls = 0` reads as "none"
where `NULL` reads as "—" — the count column, not the emptiness of the list, is what
separates "no tool ran" from "nobody recorded whether one did".

**The model answers by position, never by call id.** Twenty UUIDs copied back is tokens
paid for nothing and one transposed character silently attaches a label to the wrong call.
A number returned twice labels the call once (`pop`, not a lookup); a number that was not
in the batch is dropped. Every call asked about lands in exactly one of four lists, each
with one cause: `labels`, `no_transcript`, `no_label`, `not_reached`. A call id that is
not in the database is **refused**, not skipped — S-24 divides by the number of labels,
and labelling forty-four of the forty-five calls somebody asked about makes that figure
quietly wrong about which calls it describes.

`plan._envelope` and `plan._text` became `plan.envelope` and `plan.required_text`:
`label.py` needs the same refusal of the same misspelled key, worded the same way, and a
second copy is a second place for the wording to drift.

**Not done:** the estimate is a ceiling and reads high, above. Cache-read and batch rates
are still unpriced, so a run that hits the prompt cache is charged at the base rate it was
never billed. A batch that fails after its wave-mates have been charged loses nothing
already written but the run then dies — there is no resume, so a re-run pays for the
batches it already bought. `PROGRESS` counts batches attempted, so the last line before a
failure overstates by one. Nothing is written to `jobs`; that is the engine's, in S-25.
`plan` and `clarify` are still unmetered (S-22).

### S-24 — BAML `SynthesizeRule` + `RefineRule` + agreement via `rule-check` ☑ (PR #25, 6b8ad87)
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

**Learned:** **nothing the model returns is executed, and that is a shape rather than a
promise.** The rule reaches the engine as a *file named by path* in an argument list with
no shell anywhere near it, `engine/src/rules.rs` is the only thing that ever reads one,
and its regexes are compiled by the `regex` crate — never by Python. A test hands the
synthesiser `'; rm -rf / #`, `$(whoami)` and `(a+)+$` and asserts none of it reaches an
argv. Round-tripped by hand against `target/debug/graphify`: the hostile rule matched
nothing and did nothing.

**The engine is the only thing that says what a rule means.** `agreement` is counted from
the ids `graphify rule-check` printed, never from a Python reimplementation of the DSL —
one that drifted would report a number about a rule nobody runs. A rule the engine refuses
comes back in the engine's own words, with the temp path scrubbed out because the file is
gone by the time anyone reads the message.

**Agreement is over the whole sample, not over the matches.** Two calls both sides said no
to are two calls they agree about — which is why the register's example is 246/250 and not
38/40. Reported next to `agreed`, `of`, `matched_by_rule` and `matched_by_model`, because a
rule can score 0.9 by matching nothing at all when only a tenth of the sample matched.

**The refinement has to earn its place.** Under 0.85, one `RefineRule` call gets at most
thirty disagreements — all of them would make the second call bigger than the first for a
rule that is badly wrong, which is exactly where the tokens buy least — and the rule it
returns is scored the same way and **kept only if it agrees on more calls**. The model is
not trusted to have improved anything; there is a number that says whether it did. One
deviation: `RefineRule` returns a `Refinement` (rule *and* reason) rather than a bare rule,
because the synthesis reason describes the draft the refinement replaced, and printing it
beside the new rule explains something nobody is running. Its criterion, plan and DSL
inputs are there for the S-22 and S-23 reason: without them it is fixing a rule against
nothing and will invent a key the DSL does not have.

**The `patterns` row is stored in `free` mode** — the whole point of having got here — and
S-23's labels are attached to it with `rule_match` filled in from the same `rule-check`
that produced the agreement figure, so the two can never come to disagree. `ESTIMATE`
covers **both** calls, because the refinement is the worst case and a cap is only a cap
against the worst case. No `GO`: one call on a few hundred short quotes, following a
labelling the analyst paid for in the same click, and a second confirmation inside one flow
trains people to click through both.

Two tests read `engine/src/rules.rs` and pin its `Rule`, `Subject` and `Tool` field lists
against what the brain sends, because both are `deny_unknown_fields` and the brain's CI job
has no Rust toolchain to find that out for real.

**Not done:** the cap must cover synthesize *plus* refine, so a cap sized for the typical
run refuses the job outright. A rule the engine refuses ends the run at exit 1 with the
money already spent and nothing stored — the engine's complaint would make a fine
`RefineRule` input and is not used as one. `PROGRESS` is a fixed `n/3` and does not grow
when a refinement happens. The chart is not re-suggested after a refinement. A pattern row
is written before S-26's "Save", so a wizard the analyst abandons leaves one behind.

### S-25 — Engine spawns brain jobs; `/api/patterns/*` with progress `[Rust]` ☑ (PR #26, e216d03)
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

**Learned:** the go is a word, and it travels. `label` prints its price and blocks on `GO`;
the engine parks the child there — alive, stdin still open, having read nothing — and the
only thing that writes `GO` into it is `POST /api/jobs/{id}/go`. The alternative was to let
the child exit at the price and re-spawn it with `--yes` on the click, which needs no parked
process and no timeout, and was rejected: `--yes` is the brain's escape hatch for a person
at a terminal, and an engine passing it becomes the thing approving the spend. As built, the
word goes from the click to the child's stdin in one hop and can be followed by hand.

Keys in the environment is half the rule; the other half is that they must not come back.
A job's log is its brain's stderr, stored in a column and served to the browser, and a
traceback out of an HTTP library is an ordinary way for an `Authorization` header to end up
in one. The engine knows the exact strings it handed the child, so it replaces those with
`***` on everything the child prints — stderr and result line both. Exact values rather
than a pattern for what a key looks like: provider prefixes change, and a guess is a guess.

The price and the progress are read back out of the log rather than kept in columns beside
it. One account of what the job said, and no second copy to drift from the lines the brain
actually printed. `PROGRESS` with nothing reported is `null`, not `0/0` — a bar drawn at
nought per cent says the job has got nowhere, and it simply never said.

Two things the register did not ask for, both of which the design needs. A job nobody
approves is killed unspent after half an hour, as `expired` rather than `failed`: walking
away from a quote is not a thing going wrong. And every `running` or `waiting` row is closed
out at startup, because those children died with the process that spawned them — without the
sweep, four abandoned `waiting` rows count against the live cap for ever and no job ever
starts again. Both have tests; the go-wait is a field on `Jobs` so one of them can watch a
job expire in a fifth of a second.

`stderr` gets a thread of its own. A supervisor reading stdout while the child fills its
stderr pipe is a deadlock with both sides waiting politely, and `PROGRESS` lines and
tracebacks both come down stderr.

The routes forward the request body to the brain byte for byte. The engine does not know
what a plan looks like and has no business editing one — the brain names the key it did not
expect, in its own words, and that message reaches the log. The org rides in `?org=` for the
same reason: it is the engine's own bookkeeping, `jobs` has no org column and the spend it
books is keyed by one, and putting it in the body would add a key the brain would refuse.

`PUT /api/patterns/{id}` runs the rule through `rules::validate` before storing it, so a rule
the engine will not run is refused while the analyst is looking at it rather than at the next
unattended `apply`, naming a pattern nobody is in front of any more.

Verified beyond the suite by a round trip against the real `graphify-brain`, with real call
rows and no key in the environment: it quoted `ESTIMATE 0.0428`, parked at `waiting`, and was
killed without a `GO`. `spend` empty, `pattern_labels` empty, and `ps` showing its argv as
`label --db /tmp/gtrip/graphify.db` — no key, no `--yes`.

**Not done:** `plan` and `clarify` are still unmetered, so their jobs finish at `cost_usd`
0 whatever they cost. There is no resume and no cancel: a job cannot be stopped once its go
is given, and an engine restarted mid-labelling marks the row `expired` while the batches it
already bought stay bought. The daily cap in D-8 is stored per pattern and read by nobody
yet — `spend` is written, and S-28 is what checks it. A job's log is capped at 64 KB by
dropping later lines, which is the wrong end of a traceback to keep. And a job row is never
deleted, so `jobs` grows for ever.

### S-26 — UI pattern wizard: config step, chat step, plan table, ≥95% gate, cost go ☑ (PR #27, 98e6f83)
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

**Learned.** **The go is two clicks on one button, and the price is on it before the one
that costs.** The register asks for a button reading "Read N calls · ~$X", and the only
honest source of that `$X` is the brain — the estimate is arithmetic over the transcripts
and the model's rates, and reproducing it in TypeScript would be a second copy free to
drift. So the first click starts the labelling job and lets it park on its own quote: the
child is alive with its stdin open, having read nothing and bought nothing. The button then
carries that figure and the second click is the go. **Quoting on a deliberate click rather
than the moment the plan clears the gate is the whole reason there are two.** A parked job
holds a live child for half an hour, there is no cancel, and `MAX_LIVE` is four — so
quoting automatically on every clarify that happened to land above 0.95 would wedge the
engine with jobs nobody asked for, after which nothing starts at all. The go is sent behind
a `useRef` and not a piece of state, because a re-render is too late for the second click of
a double-click.

**A client polling a job has to treat `waiting` as in flight.** `POST /api/jobs/{id}/go`
answers as soon as the word is on its way to the child; the row goes back to `running` in
the thread that was parked on it, after that. Six goes in a row here were all `running` by
the next request, so the window is small — but a client that read `waiting` as an ending
would fail a labelling run that was about to succeed.

**"Read the agent's prompt" needed an engine route**, and that is the one thing here that is
not a UI file. `/api/assistants` deliberately leaves the prompt behind — the list is a
picker and the prompts run to tens of kilobytes each — so there was nowhere to go and read
one. `GET /api/assistants/{id}/prompt` reads exactly the assistant it names, and the org is
part of the lookup rather than a filter on the answer: one client's assistant id must never
read another's prompt.

**A run is one object, tagged with what it is about.** The quote, the calls it was priced
against, the labels it bought and the pattern written from them are never separately true,
so they travel together under the settings string they belong to, compared during render —
the dashboard's trick with its query string. There is no effect racing the edit that caused
it, and a stale run is simply not shown; its parked job is left to expire, unspent.

**The failure headline is a guess, and had to be.** The brain's own refusals are one tidy
line. Anything it did not expect is a Python traceback whose last line is the tail of a
wrapped sentence: "to be set but it is not" is a true last line and tells nobody that no
model key is configured, while `BamlError: LLM client 'Sonnet' requires environment variable
'ANTHROPIC_API_KEY'` is four lines above it. So the headline is the last line that *starts*
with an exception's name, the last non-empty line when there is none, and the whole log is
one disclosure away underneath. Showing it is safe: the engine scrubbed the keys on the way
into that column.

**Verified without spending anything.** The wizard's exact bodies were sent to the real
`graphify-brain` through the engine: `plan` with a `system_prompt` and `synthesize` with
labels were both accepted and both died at the missing provider key — which is the proof
they passed validation — and `label` parked at `ESTIMATE 0.0431` with argv
`label --db …`, no key and no `--yes`. The engine was killed with no `GO` ever sent, and
`spend`, `pattern_labels` and `patterns` were all empty.

**Not done:** `plan` and `clarify` are unmetered, so the wizard cannot say what the
conversation cost. There is no cancel — a quote the analyst walks away from holds a child
until the engine expires it, and three abandoned quotes plus one real job is the cap. The
Save button shows the cap rather than an estimate, because `synthesize` prints its price and
does not wait for a go. The wizard cannot change org: that is the filter bar's, on the
dashboard. And there is still no UI test runner, so the two acceptances are held by the
shape of the code and a manual walkthrough, not by a test.

### S-27 — Patterns list, pattern chart, edit rule, mode + cap, re-apply ☑ (PR #28, 9c59f49)
**PR:** one. **Depends on:** S-26.
**Files:** `ui/src/patterns/List.tsx`, `ui/src/charts/pattern.tsx`.
**Today:** patterns created but not browsable.
**Change:** Sidebar list with counts under current filters; per-pattern chart (type from
the suggestion, default line per bucket); click filters the call table; edit rule JSON
with validate; mode select free / hybrid / full with `daily_cap_usd`; "Re-apply".
**Acceptance:** WHEN a rule is edited to match nothing and re-applied THEN its count SHALL read 0 and the chart SHALL be empty.
**Verify:** `pnpm build`; manual.
**Must not:** spend on re-apply in `free` mode.

**Learned:** *the count has to be a count of the calls on screen, and that decides where the
filter goes.* `pattern=` is a new filter, and the tempting place for it is `conditions()`
with all the others — one line, no new SQL. It is the wrong place. `selection()` cuts the
newest `last` calls out of the range, and a pattern folded into that `WHERE` pages the
*matches* instead: 250 matched calls drawn from a span the count beside them was never taken
over. So the CTE grew a second half — `page` is the cut, `sel` is the page narrowed to the
pattern — and every read downstream inherits it for nothing, `/api/stats` included. The
sharp test is `last=1`: the only matched call is the oldest, so both the list and the count
say none, which is what folding it inwards would have got wrong.

**The chart needed no new engine code at all.** `Totals.calls` per bucket, counted with
`pattern=` on the request, *is* the number the chart draws — so `charts/pattern.tsx` is
fifty lines that hand `stats.per_bucket` to the `Bucketed` the dashboard already had, `Bar`
to its stack and `Line` to its lines. The one change was widening `Field` from
`keyof Omit<Totals, 'calls'>` to `keyof Totals`; `calls` had been left out only because
nothing had wanted it yet. A bucket with no match draws a real 0 here rather than a gap,
which is the one place on this dashboard where zero is the right mark: the calls were read
and none of them matched.

**`GET /api/patterns` takes the whole filter set now, and `matched` is null where there is
no selection.** One statement counts every pattern — a list whose counts arrive one at a
time is a list that fills in. `DISTINCT call_id`, because a hybrid pattern can have matched
the same call by its rule and by the model, and that is one call. The list itself is never
filtered: a pattern this window holds nothing for reads 0 rather than vanishing, because a
pattern that disappears when you narrow the range looks deleted. And `update_pattern`'s
response carries `matched: null` rather than 0 — it was not counted against anything, and 0
there is a number the analyst could believe.

**Two checks, in two places, for two different mistakes.** A missing brace is caught in the
browser as it is typed, because that answer needs no server. Whether the JSON is a *rule* is
the engine's, and `PUT` already refused an unknown key naming it — so "validate" is the save
button, and a rule that saves is one that will still run at three in the morning with nobody
watching. Nothing on the panel spends: `apply_one` is the rule half in every mode, which the
copy says out loud and a live check confirmed — no `spend` rows, no `jobs` rows, no brain
process, on a `hybrid` pattern.

**A stale query and a stale pattern are not the same staleness.** The house idiom is to
leave the last render on screen, dimmed, while the next loads. That is right for a filter
that moved — the same pattern a moment ago — and wrong for a pattern that changed, where it
would put one pattern's chart and one pattern's table under another's name. So the detail is
tagged twice: with the query it came from and with the pattern it is about. Same pattern,
stale query → dimmed. Different pattern → gone, and "Loading…". Proved with a throttled
route rather than asserted.

**The filter bar is handed to the screen, not drawn above it.** The wizard picks its own
calls, so a window and a `last` over the top of it are controls it ignores. `Patterns` takes
the bar as a prop and puts it above the list only; the filters still belong to the page.

**Not done:** no delete and no rename. The call table under a pattern still does not sort and
does not page. `daily_cap_usd` is stored and read by nobody until S-28. `pattern_counts` with
`pattern=` also set answers a coherent but useless question, and nothing guards it — the UI
never sends both. And there is still no UI test runner, so the screen is held by a browser
walkthrough and by the engine's tests, not by a test of its own.

### S-28 — Daily hybrid/full modes with spend cap ☑ (PR #29, 5c176e3)
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

**Learned:** the step is three decisions and one of them is not in the register.

**Where the rule half runs, and why it is inside `sync`.** In hybrid the rule chooses which
calls a model is paid to read, so a prefilter that has not seen this morning's calls is a
model that reads none of them. `graphify apply` is free-only on purpose — re-counting a
model-backed pattern from the outside is somebody asking for a number and getting a bill —
so `sync` grew `rules::apply_org`, which runs the rule half for every pattern in one org in
whatever mode it is in. That is still arithmetic and still free; it is the same pass the
Re-apply button does. The order in S-31's crontab line — sync, assistants, apply, daily —
is now literally what `graphify sync` does, in that order, in one command.

**What a verdict does to a rule's row.** This is the decision the register does not name and
the mode is incoherent without it. A confirmed call gets a `source='llm'` match of its own,
which in hybrid is a second row for one call — exactly what S-27's `count(DISTINCT
call_id)` was written for. A **rejected** call loses the rule's row, and `run_one` will not
put it back the next time the rule is run. Leave it in and the count beside the pattern
reads the same before and after the model was asked: hybrid mode becomes a bill for a number
that did not move, and Re-apply quietly undoes every confirmation that was paid for — which
the next daily run cannot buy back, because those calls have been read once and a model is
not asked the same question twice. Free patterns are deliberately exempt: the wizard stores
its sample against every pattern, and a free pattern is one whose rule was chosen to
disagree with part of that sample by a measured amount. That figure is `agreement`, and a
rule quietly edited to agree with it would make it meaningless.

**Two caps, and neither of them is a click.** The pattern's `daily_cap_usd` bounds one
pattern in one run; what is left of the org's day — `GRAPHIFY_DAILY_CAP_USD`, default $5,
less what `spend` already holds — bounds the whole run, so a first pattern that eats the
budget leaves the second unread rather than doubling the bill. Both go through
`label._affordable`, which prices a wave and refuses it *before* it is sent; checking
afterwards is finding out that the cap was passed. A cap that will not parse is an error,
not a fall back to five dollars — somebody who wrote `2,50` meant to set a cap. A cap of
zero is allowed and means zero, which is how the daily modes are turned off on a machine
without editing every pattern on it.

**The spend is reported even when the run is not clean.** The engine books a job's cost off
the brain's last line, so a traceback out of `daily` is money spent with nothing to book it
from. A pattern that falls over is caught, recorded against that pattern with its error,
and the run carries on. The job ends `done` with the failure in its log and its output,
because `failed` would book $0.

**A blocking spawn.** `jobs::start` answers an HTTP request while the child works; a `sync`
at six in the morning has nobody to answer, and a command that returned early would exit
and take its child with it. `jobs::run_blocking` is the same supervisor on the calling
thread — no `Jobs` map, because `daily` is a kind that never parks.

**Not done:** `spend` is keyed `(day, org)`, so there is no per-pattern record of a day —
the pattern's cap bounds one run and the global cap is what bounds the day. A batch that
was charged inside a run that later raises still loses its `usd`; that is S-23's own path
and it is unchanged. `apply_org` is reachable only through `sync` and the Re-apply button:
there is no `graphify apply --org`. The candidate list is capped at 500 calls per pattern
per run, which is memory and not money, and a full-mode pattern on a busier org than that
falls behind by a day at a time. And `daily` reports its progress per pattern while `label`
reports it per batch down the same pipe, so a job's last `PROGRESS` line is the pattern one
and the two interleave.

### S-29 — Ask box (BAML `AskAnalysis`) ☑ (PR #30, ae50bf0)
**PR:** one. **Depends on:** S-25, S-17.
**Files:** `brain/baml_src/ask.baml`, `brain/src/graphify_brain/ask.py`, `engine/src/server.rs` (`POST /api/ask`), `ui/src/Ask.tsx`.
**Today:** no free-form analysis.
**Change:** Input: current filters → engine builds `stats` + up to 20 sample transcripts
(shortest first, capped at 60k tokens) → estimate → confirm → answer markdown. No history.
**Acceptance:** WHEN the user cancels at the cost step THEN no job SHALL be created.
**Verify:** tests for the three layers with mocks; manual once live.
**Must not:** send more than the cap.


**Learned:** *Where the price comes from decided the whole step.* The acceptance — a
cancel at the cost step creates no job — cannot be met by the shape every other spend in
graphify uses, where the brain prints `ESTIMATE` from a child that has already been
started and parked on its stdin. A question is a thing people try, reword and abandon, and
`MAX_LIVE` is four: four questions read and walked away from would hold the engine's whole
job budget for half an hour each. So `POST /api/ask/quote` prices the question on the
request, starting no process and writing no row, and `POST /api/ask` is the click. The ask
job never parks, because the approval is behind it rather than ahead of it.

*Pricing in the engine means the rates live in two files, and a test is what makes that
one fact.* `engine/src/ask.rs` mirrors `graphify_brain.cost`'s table and the constants the
estimate rests on; `engine/tests/ask.rs` parses `cost.py`, `label.py` and `ask.py` and
fails if any of them disagree — the same trick `brain/tests/test_cost.py` plays on
`clients.baml`. One place to edit a price, and drift is a failing build rather than a
wrong number on a button.

*Every count on the engine's side errs high, and that is what makes the two caps agree.*
The engine counts the question and the statistics in bytes where the brain counts
characters, and allows a flat `FACTS_CHARS` per call for a line it does not build. Its
figure is therefore a ceiling the brain's own estimate comes in under — measured at about
8% on the live run. The allowance is a trade, not a margin to maximise: too small and the
engine quotes a context the brain then refuses as over the 60k cap, which is a question
that cannot be asked at all; too large and the number on the button is visibly bigger than
what the same question costs, which teaches people to stop reading it.

*The sample is skewed by construction and everything has to say so.* Shortest transcripts
first is how the most calls fit under one cap, and it makes the sample biased in exactly
the dimension people generalise along. So the prompt separates the two kinds of evidence —
numbers come from the statistics, which describe every call in the selection; quotes come
from the sample, which is not typical — and the line under the box says the same to the
reader.

*The browser found two things the tests did not.* The route read the org with `org_param`,
which refuses every query key but `org`, so a question asked over a window came back
`unknown parameter window` — and every question is asked over a window. And the answer's
footer showed `job.estimate_usd`, the brain's own quote, under the word "quoted": a figure
the person never saw, beside a button that had said something else.

**Not done:** the quote is taken twice per question and the statistics are built both
times, which is two full passes over the selection for one answer. A question over a wide
enough window is refused rather than answered from the statistics alone — `per_bucket`
grows with the span and can fill the context on its own. `MAX_QUESTION_CHARS` is 2,000 and
there is no history, so a follow-up is a new question that re-reads and re-pays. The
Markdown subset is drawn by hand and anything outside it renders literally. And nothing
stores an answer: it lives on the screen until the filters move.

### S-30 — PDF downloads: dashboard, per pattern, call list ☑ (PR #31, 32fa48f)
**PR:** one. **Depends on:** S-27, S-18.
**Files:** `ui/src/pdf/*.ts`, `ui/package.json` (`jspdf`, `html-to-image`).
**Today:** none.
**Change:** Three buttons. Dashboard PDF = header (org, assistants, window, filters) +
every enabled chart as image. Pattern PDF = criterion, plan table, rule, agreement,
count, 20 matched calls with evidence. Call-list PDF = the current table.
**Acceptance:** WHEN "Download dashboard PDF" is clicked THEN a PDF SHALL download containing one image per enabled chart.
**Verify:** `pnpm build`; open the file.
**Must not:** need a server dependency.

**Learned:** jsPDF draws and does not lay out. Every `text` call is an absolute
coordinate, so without one file owning the arithmetic each of the three reports would be
doing its own and getting a different answer — one wrapping at a different width, another
running off the bottom because nothing counted the lines. `ui/src/pdf/doc.ts` is that file:
a cursor, a margin, and the half-dozen marks a report is made of, with `need` called before
every one of them so a table row is never half on one page and a table repeats its header
when it runs on.

Nothing in the PDF layer formats a value. The numbers in a file are the numbers that were
on screen, through `format.ts` — dash included. `yesNo` and `tools` moved out of
`CallTable.tsx` into `format.ts` for that reason: a downloaded call list *is* the table,
and two spellings of "3 · 1 failed" would be two documents disagreeing about one call.

A capture is whatever theme the reader had. On a dark screen that is white ink on
near-black, which prints as a slab. `capture.ts` pins `data-theme` to light for the length
of the capture and restores it in a `finally` — the stylesheet already declares its dark
steps twice, once for the OS setting and once for an explicit `data-theme`, precisely so an
explicit choice wins in both directions, and this is that choice held for two frames.
Verified from a dark-mode browser: the file came out light and the page was still dark
afterwards.

The layout mistake worth remembering is scaling every picture to the page width. A
half-width card captured at 670px and stretched to 180mm renders its 15px heading at the
size of the document's title — the same picture, and a lie about how much of the dashboard
it is. Drawing each card at the share of the width it had on screen keeps one scale across
the file and puts two narrow cards on a row the way the pack does; twelve charts went from
eleven pages to three.

Images are deflated rather than stored (`addImage(..., 'FAST')`). A twelve-chart dashboard
is 20 MB without it and 477 KB with it. Charts are flat colour and straight lines, which is
the case PNG's own filter was built for.

`jspdf` and `html-to-image` weigh about 350 kB — a third again on top of the page — for a
button most readers never press, so `ui/src/pdf/index.ts` is a dynamic-import seam and the
main chunk is unchanged. `import type` across it is erased, so the shapes are still checked
at build time.

`pattern_labels.evidence` had been stored since the first migration and never read by
anything. `/api/calls` returns it now when the request names a pattern. A reason belongs to
the (pattern, call) pair — the same call carries a different one under a different pattern
— so without a pattern there is nothing to join on and the column is NULL, asserted both
ways.

Two small things the environment taught: `pnpm` exits 1 on `ERR_PNPM_IGNORED_BUILDS`, which
would have failed CI, and the answer is `allowBuilds` in `ui/pnpm-workspace.yaml` rather
than a `pnpm` field in `package.json`. And `new Date().toISOString()` for a filename names
the UTC day while the footer stamps the local one, so a file downloaded in the evening is
named for yesterday.

**Not done:** the captures are raster, so text inside a chart does not select or search.
Twenty is a fixed cut on the pattern's call list. The header is the filter bar as it stood
at the click, and nothing re-checks it against the numbers if the two drift. Nothing
watermarks a file or records that one was taken.

### S-31 — `graphify schedule --print` + README daily section ☑ (PR #32, 19fa68e)
**PR:** one. **Depends on:** S-28.
**Files:** `engine/src/cli.rs`, `README.md`.
**Today:** manual sync.
**Change:** Prints a crontab line and a launchd plist for `graphify sync --org all` at
06:00 (sync → assistants → apply → daily). `--install` asks y/n before writing.
**Acceptance:** WHEN `--print` runs THEN the crontab line SHALL contain the absolute binary path.
**Verify:** run it; install on Abhishek's machine; next-day log.
**Must not:** install without confirm.

**Learned.** A scheduled job starts with none of the environment a shell hands you, and
each absence breaks the morning differently. There is no working directory: cron runs from
`/`, and `data/graphify.db` is a relative path, so an unqualified line does not fail — it
makes an empty database somewhere else and syncs into that, which is the worst kind of
wrong because nothing says so. There is no PATH worth having: cron's is `/usr/bin:/bin`,
and neither `graphify` nor the `graphify-brain` it spawns is in either. And there is no
environment at all. So everything is resolved while a shell that knows the answers is
still running, and the acceptance for this step — the absolute binary path — is really the
smallest case of that rule.

The one thing that cannot be resolved is `GRAPHIFY_SECRET`, because a key is never
printed. Leaving it out silently would have been the trap: the run falls back to the
`.secret` file beside the database, which is a different key, and every stored Vapi key
fails to decrypt under it. The consequence is printed instead of the value. That is the
shape to reuse — when a rule stops you from emitting something, emit what its absence will
do.

`engine/src/schedule.rs` splits at the line between text and the world: `Plan::crontab` and
`Plan::plist` are pure, and `install` is the only part that writes. That split is what
makes the step testable at all, because a scheduler cannot be checked by running it — the
line does nothing until tomorrow, and the only thing to look at today is what was written.
Both strings are quoted for the reader that gets them: single quotes for `/bin/sh`, with
the one character they cannot hold closed, escaped and reopened, and `&`/`<`/`>` escaped
for the plist's XML parser. A directory called `a b & c` breaks each of them differently.

Two small shapes worth keeping. The crontab entry ends in `# graphify schedule`, which is
nothing to `/bin/sh` and is the marker `--install` uses to replace its own previous line
rather than double the morning — one line that is both the job and its own name. And the
launchd job is a `StartCalendarInterval` rather than an interval, so a Mac asleep at six
runs it on waking instead of skipping the day.

`confirm` treats end of input as no. A `--install` in a pipe or a CI job reads a closed
stdin, and "no answer" must never be an answer — that is what "must not install without
confirm" actually costs in code.

`sync --org all` is the line's argument, so it had to exist. One org without a key does not
stop the ones after it, and the run still exits non-zero: a morning log full of zeroes that
exits 0 reads as a quiet day. A single named org still returns its own error unwrapped, so
nothing reading stderr had to change.

Verified past the suite, because none of the suite proves the thing this step is for: the
printed command was run from `/` under `env -i` against a scratch database and exited 0
into the log it had named; `plutil -lint` accepted the plist; and the whole `--install`
path was answered `y` against a throwaway `HOME`, at which launchd took the plist and
registered the label, and `bootout` removed it again.

**Not done:** `--install` knows macOS and Linux, and points anywhere else at `--print`. It
writes the job and never checks the next day that it ran — the log beside the database is
the only record, and nothing rotates it. An org literally named `all` is shadowed by the
keyword. Nothing has been installed on Abhishek's machine yet; that is his `--install` to
answer.

### S-32 — Dockerfile + docker-compose + password mode ☑ (PR #33, c63d301)
**PR:** one. **Depends on:** S-13, S-20, S-31.
**Files:** `Dockerfile`, `docker-compose.yml`, `README.md`.
**Today:** local only.
**Change:** Multi-stage: ui build → cargo build → python slim with `uv` + brain +
engine binary. Volume `/data`. Env: `GRAPHIFY_PASSWORD`, `GRAPHIFY_SECRET`,
`GRAPHIFY_BIND=0.0.0.0:3737`. Cron inside the container via `supercronic`.
**Acceptance:** WHEN `docker compose up` runs with `GRAPHIFY_PASSWORD` THEN `http://localhost:3737` SHALL show the login page.
**Verify:** build + run locally; screenshot.
**Must not:** bake any key into the image.

**Files (as built):** `Dockerfile`, `.dockerignore`, `docker-compose.yml`,
`docker/entrypoint.sh`, `README.md`, `engine/src/db.rs`, `engine/tests/db.rs`.

**Learned.**

*Build order is not a preference here.* `engine/build.rs` embeds `../ui/dist` at compile
time, so the node stage has to come before the cargo stage. Getting it wrong does not
fail: it produces a working binary that serves a page saying the UI was never built. The
same shape as S-31's relative database path — the container's characteristic failure is
not an error, it is a plausible wrong answer.

*Three toolchains build it and none of them ships.* node, cargo and curl each live in a
stage that contributes one file to the final image. What ships is `python:3.12-slim` with
two binaries in it, 437 MB, running as uid 10001 with `/data` as the only writable thing.
Debian and not Alpine, because `rusqlite`'s bundled SQLite is C and the runtime image is
glibc: a musl binary would build fine and then not run.

*A lockfile is not the whole install.* `pnpm install --frozen-lockfile` failed in the
image and passes in CI, because `pnpm-workspace.yaml` carries the `allowBuilds` answer for
`core-js` and the manifest-only COPY had left it behind. pnpm refuses to install at all
when an ignored build script has no recorded answer and nobody to ask. Anything that
configures the installer belongs in the dependency layer beside the lockfile.

*What "must not bake a key into the image" actually costs.* `docker history` prints every
ENV and every ARG of every layer, so a baked key is readable by anyone who can pull the
image and stays readable after it is rotated. Two halves: nothing in the Dockerfile is a
value, and `.dockerignore` keeps `data/`, `.env` and `.secret` out of the build context so
no COPY can take one by accident. Verified by grepping the built history.

*Named, not assigned.* Compose's `- VAR` passes a variable through only when the shell has
it; `- VAR=${VAR:-}` sets it to the empty string. For `GRAPHIFY_DAILY_CAP_USD` that is the
difference between "no cap given, use the default" and a value that will not parse, which
by S-29's rule stops the run. The one deliberate exception is
`${GRAPHIFY_PASSWORD:?…}`: this is the mode that publishes a port, so compose refuses to
start without it.

*The container's crontab is the short line, and that is the point.* S-31 spells out every
path because a laptop's cron has no working directory and a PATH of `/usr/bin:/bin`. An
image has both by construction — PATH, `GRAPHIFY_DB` and `GRAPHIFY_BIND` are set in it —
so the entrypoint writes `0 6 * * * graphify sync --org all` and nothing more. supercronic
rather than Debian's `cron`: it does not want to be PID 1, it hands each job the
environment it inherited (which is how `GRAPHIFY_SECRET` reaches six o'clock, since the
printed line cannot carry it), and it logs to stderr where `docker logs` already is. The
crontab is written only when the command is `serve`, so
`docker compose run graphify sync --org acme` is one command that ends rather than a
second scheduler.

*The image is the first configuration with two writers by default,* and it exposed a bug
that was always latent: `Db::open` set no busy timeout, so SQLite's answer to the second
process finding the file locked was to fail it immediately with "database is locked". Five
seconds of waiting is the whole fix — every write here is one short statement or one short
transaction. This is the one change outside the step's named files, and it is the step's
own doing.

*Verified past the suite, against a real daemon:* build clean on arm64; `up` without a
password refused by name; with one, the login page rendered (screenshot), wrong password
401, right password 200 with a `Set-Cookie`, then `/api/orgs` 200; `docker history`
carrying nothing of ours; `GRAPHIFY_CRON='* * * * *'` firing the sync on the minute with
its stdout in `docker logs` and the job reported succeeded; `GRAPHIFY_CRON=off` reading no
crontab; a one-off `run` starting no scheduler; the volume keeping the database across a
recreate and `down -v` removing it.

**Not done:** no TLS, so the port is published on `127.0.0.1` and anything further needs a
reverse proxy with a certificate. No healthcheck. supercronic is backgrounded by the
entrypoint, so if it dies mid-life the container stays up and the mornings quietly stop —
nothing notices. Only `amd64` and `arm64` have a recorded supercronic checksum. Nothing
prunes `jobs` rows or rotates anything inside the volume.

---

### S-33 — Meter `plan` and `clarify` ☑ (PR #34, af61bb7)
**PR:** one. **Depends on:** S-22, S-25, S-26.
**Files:** `brain/src/graphify_brain/plan.py`, `ui/src/patterns/Wizard.tsx`, tests either
side. The engine is expected to need nothing: `jobs.rs` already reads `ESTIMATE` off
stdout and books the output's `usd` through `add_spend`.
**Today:** every message in the wizard's chat calls Sonnet, reports `cost_usd` 0 whatever
it cost, and is counted by no cap. Four Not-done paragraphs say so (S-22, S-23, S-25,
S-26), and it is the one place the register leaves a **Must never** broken: *no model
call without a shown cost and an explicit go.*
**Change:** `plan` and `clarify` take a required `max_usd`, like `label` and
`synthesize`. Each prices its own call at the ceiling before the model is touched, prints
`ESTIMATE {usd}`, and refuses rather than sends when the ceiling is over the cap. The
call itself runs under a `baml_py.Collector`, so what comes back is what was actually
spent, and that goes out as a top-level `usd` beside the plan's fields — the same shape
`label` already returns and the same field `jobs.rs` already books.

The go stays the Send button, which is what S-22 argued and is still right: these are a
few cents and they read no transcript, so parking them on a second click would buy
nothing. The shown cost is the per-step cap on the button, plus what the last message
cost and what the chat has cost so far, read from `jobs.cost_usd` rather than recomputed
in the browser. Nothing mirrors the pricing arithmetic into TypeScript — S-29 paid for a
second copy in Rust because a quote had to exist before a process did, and there is no
such requirement here.
**Acceptance:** WHEN a plan or a clarify finishes THEN its job row SHALL carry the USD it
actually cost and that amount SHALL appear in the day's `spend`; AND WHEN a message's
ceiling is over `max_usd` THEN no model call SHALL be made.
**Verify:** unit tests over the seam (`client()` replaced, `Collector` faked); one live
walkthrough in the browser showing a non-zero cost on a plan and on a clarify.
**Must not:** park either function on a go — the Send button is the go. Charge the
ceiling: the ceiling is what the cap is checked against, the collector's number is what
is booked. Let a plan carry `usd` back into `clarify` as part of the plan object.

**Files (as built):** `brain/src/graphify_brain/plan.py`, `brain/src/graphify_brain/cli.py`,
`brain/tests/test_plan.py`, `ui/src/api.ts`, `ui/src/patterns/Wizard.tsx`,
`engine/tests/jobs.rs`. No engine source at all — tagged `[Rust]` on the way in, and it
turned out not to be one.

**Learned.**

*"A shown cost and an explicit go" is two requirements, and they can be met by two
different things.* The go was already there — the Send button — and S-22 was right about
that. What was missing was only the cost. Reading the rule as one requirement is what kept
it unfixed for eleven steps: every attempt to picture the fix ended at a parked child and a
second click, which for four tenths of a cent is worse than nothing, because a person made
to approve a price they cannot care about stops reading prices.

*The ceiling and the charge answer different questions and must not be the same number.*
`label` had already worked this out; it is worth stating plainly. The ceiling is a bound
computed before the call, and it is what a cap can be checked against precisely because it
exists before there is anything to check. The collector's figure is what the provider
actually billed, and it is what gets booked, because a day's spend built out of ceilings is
a day's spend that is wrong. Here the two are far apart — `MAX_OUTPUT_TOKENS` is BAML's
4,096 and a plan is nearer 300 — so booking the ceiling would have overstated by ten times.

*A fake that answers cannot also invoice.* `test_plan.py` replaced `client` and that was
sufficient while nothing cost anything. The moment a price came off the call one seam was
not enough: a canned answer carries no usage, and `charged()` reading `collector.last.usage`
on a fake fails with `None.usage`, which reads exactly like a bug in the code under test.
Two seams, and the second one autouse — because the failure mode of forgetting it is a
confusing error rather than a wrong number.

*The prediction that the engine needed nothing was worth writing down before checking it.*
It held. `jobs.rs` reads `ESTIMATE` off stdout and books the output's `usd`, and it does
that for every kind, so a function that starts reporting money is picked up by machinery
that was already there. The two engine tests added are not changes; they are that claim
held down.

*Where a number is displayed decides where it is computed.* The wizard shows what the last
message cost and what the conversation has cost, and both come off `cost_usd` — the figure
the engine booked. S-29 paid for a second copy of the pricing arithmetic in Rust because a
quote had to exist before a process did; there is no such requirement here, and a third
copy in TypeScript would have been a number free to disagree with the booked one, with
nobody able to say which was right.

*The one thing no assertion can hold is `FIXED_PROMPT_CHARS`,* so it is held by rendering:
`test_plan.py` builds both prompts through `b.request` without sending them and fails if
either has grown past the constant. A measurement that nothing re-measures stops being one.

*Verified live, through the real engine and the real brain over HTTP, with a deliberately
invalid `ANTHROPIC_API_KEY` so nothing could be spent:* `max_usd: 0.0001` failed with
`ESTIMATE 0.0438` and then the refusal, and the provider was never reached; `max_usd: 1`
gave the same quote and then did reach the provider and fail on the key, so nothing but the
cap stands between the estimate and the call — and the key stayed absent from the log; no
`max_usd` at all was refused by name.

**Not done:** `plan.baml` declares `client Sonnet` for both functions, so neither honours
the model the wizard picked in step one. `MODEL` is pinned to that declaration and a test
fails if it moves, but making them follow the picker is its own change. The ceiling is
loose — roughly ten times a typical message — because `MAX_OUTPUT_TOKENS` is BAML's default
rather than something `plan.baml` sets. `plan` and `clarify` book against the org's day like
everything else, so an afternoon in the wizard and the next morning's sync draw on one
number and neither knows about the other. And the two figures in the wizard are held by the
type checker and by reading: there is still no UI test runner.

---

### S-34 — The wizard's model picker reaches the chat ☑ (PR #35, 9f26d55)
**PR:** one. **Depends on:** S-33, S-26, S-22.
**Files:** `brain/src/graphify_brain/cost.py`, `brain/src/graphify_brain/plan.py`,
`brain/src/graphify_brain/label.py`, `brain/tests/test_plan.py`, `ui/src/api.ts`,
`ui/src/patterns/Wizard.tsx`. No engine source is expected: the wizard builds the request
body in the browser and `jobs.rs` forwards it whole.
**Today:** step 1 of the wizard has a Model select. `model` rides along on the quote, on
`label` and on `synthesize`, and `ask` takes one too. `plan` and `clarify` ignore it:
`plan.py` pins `MODEL = "sonnet"` and `plan.baml` declares `client Sonnet` on both
functions. Until S-33 that was a wrong model on a call nobody was told the price of.
Now it is worse than that — the chat quotes a price, and it quotes Sonnet's price for a
call the person asked to be Opus. S-33's own Not-done paragraph is the one that says so.
**Change:** `plan` and `clarify` take a required `model`, validated against the same list
`label` and `ask` validate against, and run under `.with_options(client=CLIENTS[model])`
exactly as `label`, `synthesize` and `ask` already do. Both the ceiling and the charge are
priced at that model, so the quote, the call and the booked spend are the same model or
the request is refused. The wizard sends the model it is already holding in step 1.

`CLIENTS` and `_model` move from `label.py` to `cost.py`. This is not tidying: `label.py`
imports `envelope` from `plan.py`, so `plan.py` cannot import from `label.py` without
closing a cycle, and the only other way is a second copy of the one list that decides
which models may be asked for at all. `cost.py` imports nothing from the package and
already keys `PRICES` by exactly those names, which is the argument for putting the list
there rather than anywhere else — a model that can be asked for is a model whose spend
can be counted.
**Acceptance:** WHEN a person picks a model in step 1 and sends a message in step 2 THEN
that model SHALL be the model called, AND the USD quoted before the call and the USD
booked after it SHALL both be that model's rate.
**Verify:** unit tests over the same seam S-33 built, asserting the client name handed to
`with_options` and the rate the ceiling was computed at, for each of the three models; one
live walkthrough picking a model that is not the default.
**Must not:** fall back to a default when `model` is missing or unknown — the request
names a model or it is refused, exactly as `label` refuses. Quote one model's rate and
call another.

**Files (as built):** `brain/src/graphify_brain/cost.py`, `plan.py`, `label.py`,
`synth.py`, `ask.py`, `cli.py`; `brain/tests/test_plan.py`, `test_cost.py`, `test_ask.py`,
`test_label.py`; `engine/tests/jobs.rs`; `ui/src/api.ts`, `ui/src/patterns/Wizard.tsx`.
No engine source, again.

**Learned.**

*A control that is read by four callers out of five is not a control, and S-33 turned it
from a wrong answer into a wrong price.* Picking Opus and being quoted Sonnet is not a
smaller version of picking Opus and getting Sonnet — it is a different failure, because a
person who reads the price has been told something false about the call they are about to
make. It is worth noticing that the step before this one is what made this one urgent: a
figure that was merely absent became a figure that was wrong. Adding a number to a screen
puts the burden on that number being right.

*The cycle decided where the shared list goes, and the answer was better than the
constraint.* `plan.py` could not import from `label.py`, because `label` already imports
`envelope` from `plan`. The cheap way out was a fifth copy of the model validator, which
is what `max_usd` got in S-33 for exactly the same reason. The right way out was to notice
that `CLIENTS` had no business in `label` at all: the set of models that may be asked for
is the set whose spend can be counted, and the price table is what does the counting. In
`cost.py` it is reachable by all four callers with no cycle, and `set(CLIENTS) ==
set(PRICES)` — an assertion that `test_label.py` and `test_ask.py` each held a copy of
because the list lived in neither of their modules — becomes one test in `test_cost.py`,
where both objects now are. A duplicate that will not go away is usually a thing living in
the wrong module.

*The provider proved it, which no fake can.* The unit tests assert the client name handed
to `with_options`, and that is worth having, but it is a test of what this code passes
down. Running it live against a deliberately bad key produced
`BamlClientHttpError(client_name=Opus, ...)` from Anthropic's own 401 — the far end saying
which client it was, at no cost. A `gpt` clarify went further and failed asking for
`OPENAI_API_KEY`, a variable that is not set: a wrong dispatch could not have produced
that error. Two-and-a-half times exactly (`0.0438` → `0.1094`) is Opus over Sonnet on both
halves of the rate, so the quote is not merely different, it is right.

*The engine needed nothing for the second step running,* and this is now a property worth
naming rather than a coincidence: `jobs.rs` forwards the request body whole and books the
output's `usd`, so any field the brain learns to read is a field the engine already sends.
The test added to `jobs.rs` asserts exactly that and nothing more.

**Not done:** `plan.baml` still declares `client Sonnet` on both functions, as
`label.baml` and `ask.baml` declare theirs — always overridden per call, never removed, so
the declaration is now decoration that a reader could mistake for the answer. The ceiling
is still loose for S-33's reason. The picker lives on step 1 and the chat is step 2, so
nothing stops a person going back and changing it mid-conversation and paying a different
rate for the next message than the last; the price line names the model for that reason,
but naming is all it does. And the wizard's two figures are still held by the type checker
and by reading: there is still no UI test runner.

### S-35 — A test runner for the UI, and the money screen goes first ☑ (PR #36, 1e8c4e8)
**PR:** one. **Depends on:** S-34, S-33, S-22, S-13.
**Files:** `ui/package.json`, `ui/vite.config.ts`, `ui/tsconfig.app.json`, new
`ui/src/*.test.ts(x)`, `.github/workflows/ci.yml`. No engine and no brain source.
**Today:** the engine has 235 tests and the brain has 185. The UI has none. CI runs
`pnpm build` and `oxlint`, which between them prove that the TypeScript compiles and that
nobody left an unused variable — neither of which is a claim about what the screen says.
Thirty-three source files are held by the type checker and by reading.

Two of the last three steps changed `Wizard.tsx`. S-33 put a price on it; S-34 made that
price the picked model's rate. Both were verified by one person driving one browser once,
and the Not-done paragraph under each of them says so — the last line of S-34's is that
there is still no UI test runner.

The gap is not even across the tree. Three things in `ui/` are rules rather than
rendering, and a rule enforced only by reading is a rule that is already half broken.
`format.ts` is where "NULL → —, never 0" is actually decided for every number on the
dashboard, and it is one of five Must-nevers. `jobs.ts` picks the headline out of a
Python traceback with a regex its own comment calls "a guess, and a load-bearing one".
And `Wizard.tsx` builds the request bodies that spend money.
**Change:** vitest, jsdom and Testing Library in `ui/`; a `pnpm test` that runs once and
exits rather than watching; `pnpm test` added to CI's `ui` job, making it four checks and
five. Then the first tests, and only these three subjects: the Must-never in `format.ts`,
the headline in `jobs.ts`, and the wizard's spend surface — that the model and the cap
chosen in step 1 are on the `plan` and `clarify` request bodies, and that the price line
names the model that was chosen. `api.ts`'s comment on `MODELS` still points at
`label.py`, where `CLIENTS` lived until S-34 moved it; that is an orphan the last step
made and it is corrected here.
**Acceptance:** WHEN `pnpm test` runs THEN it SHALL exit non-zero for a `format.ts` that
answers `0` where a value is missing, for a `Wizard.tsx` that drops `model` or `max_usd`
from a request body, and for a `jobs.ts` that reports the last line of a traceback over
the line that names the fault.
**Verify:** every new assertion broken on purpose once and seen to fail — an assertion
that has never failed is a sentence, not a test. Then CI red on one break and green on
its repair.
**Must not:** grow this into tests for all thirty-three files, or snapshot anything. A
snapshot asserts that a screen has not changed, which is a different claim from that it
is right, and it is the assertion that gets re-recorded rather than read. Stub `fetch`
and nothing above it: a test that replaced `api.ts` would pass over a `Wizard.tsx` that
sends the wrong body, and sending the wrong body is exactly what S-34 shipped.

**Files (as built):** `ui/package.json`, `ui/pnpm-lock.yaml`, `ui/vite.config.ts`,
`ui/src/api.ts`; new `ui/src/format.test.ts`, `ui/src/jobs.test.ts`,
`ui/src/patterns/Wizard.test.tsx`; `.github/workflows/ci.yml`. No engine and no brain
source, and `ui/tsconfig.app.json` needed no change — it already includes `src`, so the
tests are type-checked with everything else for free.

**Learned.**

*An assertion that has never failed is a sentence, not a test.* Thirty-three tests went
green the first time they were run, which proves nothing at all about whether any of them
is about anything. So each was broken on purpose once: `money(null)` made to answer
`$0.00`, `model` dropped from the plan body, the cap sent as the string it was typed in,
the last line of a traceback made to win, an unreported cost counted as nothing, the
price line's model name removed, and the plan's `usd` carried back into `clarify`. Seven
breaks, seven reds — two, three, four, one, one, two and one — and the counts matter as
much as the colours: a break that reddens more tests than it should is a suite where
several tests are asserting the same thing under different names. Then repaired and
green. That loop took about as long as writing the tests and is the only part of this
step that establishes anything.

*Stubbing `fetch` rather than `api.ts` is the whole difference between a test and a
tautology.* The failure S-34 fixed was that `Wizard.tsx` never put `model` on the request
body. A test that replaced `api.ts` and asserted `startPlan` was called would have been
green on the day that bug shipped, because `startPlan` *was* called — with the wrong
body. The seam has to be below the thing that could be wrong. That is a general rule and
it is easy to get backwards, because the higher seam is always the more comfortable one
to write against.

*The three things worth testing first were not the three biggest files.* Thirty-three
source files, and the ones that earned tests are the two smallest and the one that
spends. `format.ts` is sixty lines and is where a Must-never is actually decided for
every number on the dashboard; `jobs.ts` picks a headline out of a Python traceback with
a regex its own comment already called "a guess, and a load-bearing one" — a comment
that is a request for a test, written by someone who could not write one. A rule enforced
only by reading is a rule that is already half broken, and rules are cheaper to test than
rendering because they have no screen in them.

*`pnpm test` goes after `pnpm build`, not before.* The tests live under `src`, so
`tsc -b` type-checks them along with the source. A test file that stopped compiling
should fail as a compile error rather than as a run that quietly covers one file fewer,
and that ordering is what makes the difference visible.

**Not done:** three files of thirty-three. The dashboard, the charts, the call table, the
ask box, the settings screen and every PDF path are still held by the type checker and by
reading, and the runner existing is not the same as them being tested — the next screen
to grow a bug will be one of those. Nothing asserts the wizard's *second* click, the one
that actually spends: `priceIt` parks a job and `spend` sends the go, and both were left
alone here because a test of them is a test of the two-click rule and deserves its own
step rather than a corner of this one. There is no coverage figure and deliberately no
threshold: a number that must go up is a number that gets gone up. And everything S-34
left open is still open — `plan.baml` still declares a client that is always overridden,
the plan ceiling is still roughly ten times a real message, and a person can still walk
back to step 1 mid-conversation and pay a different rate for the next message than the
last.

### S-36 — No price, no go ☑ (PR #37, 0d94d91)
**PR:** one. **Depends on:** S-35, S-33, S-22, S-13.
**Files:** `ui/src/patterns/Wizard.tsx`, `ui/src/patterns/Wizard.test.tsx`,
`docs/backlog/bugs.md`. No engine and no brain source.
**Today:** the two-click rule is the second Must-never — never call a model without a
shown cost and an explicit go — and it is the only one of the five that needs two
programs to agree. The engine holds up its end and is tested for it: a label job that is
never told to go stays waiting and spends nothing, a go for a job that is not waiting is
refused, a job nobody approves is killed unspent rather than held for ever, and a plan is
never parked on a go at all. Four tests in `engine/tests/jobs.rs`, and they are about the
half of the rule that lives in Rust.

The other half lives in the browser and nothing tests it. `priceIt` starts the labelling
job and lets it park, having read nothing; `spend` sends `POST /api/jobs/{id}/go`, which
is the only call in the system that lets a model read a call. Between them sits a `went`
ref whose comment says it exists so that a double-click on a slow network is not two
goes, and a `settings` tag whose job is to take the price off the button the moment the
plan, the model or the selection changes underneath it. Both are load-bearing and both
are held by reading.

And there is a bug in the middle of it, which is why this step is a fix and not only
tests. The button draws its price as `money(live.job.estimate_usd ?? 0)`. `money` renders
NULL as an em dash and S-35 spent three assertions establishing that it never renders a
missing number as zero; the `?? 0` at the call site means the formatter is never given
the chance. A parked job with no price on it draws as `up to $0.00`, and the click behind
that button is the go. It is reachable: `append` in `engine/src/jobs.rs` discards the
error from `append_job_log`, then parks regardless, so a failed log write leaves a job
waiting with no `ESTIMATE` line for `estimate()` to find. The type in `api.ts` already
says `number | null`, and the comment on the field directly above it says a job that has
said nothing "does not have a progress of zero". The caller overrode both.
**Change:** the price on the button is `money(live.job.estimate_usd)` with no coalesce,
so an unpriced job reads as a dash. The go is disabled while the price is unknown, with a
line saying that nothing will be read until one comes back — a shown cost is a
precondition of the go, not decoration beside it, and the button that cannot say what
something costs must not be the button that buys it. Then the tests for the browser half
of the rule: that the first click sends no go, that the price is on the button before the
click that costs and not before the one that does not, that a double-click sends exactly
one go, that a setting edited after a quote takes the price off the button and turns the
next click back into a price rather than a go, and that an unpriced parked job offers no
go at all.
**Acceptance:** WHEN a labelling job is parked with a null `estimate_usd` THEN the wizard
SHALL show a dash rather than `$0.00` and SHALL NOT offer the go. WHEN the spend button
is double-clicked THEN exactly one `POST /api/jobs/{id}/go` SHALL be sent. WHEN a
setting changes after a quote THEN the next click SHALL start a new price and SHALL NOT
send a go for the job priced against the old settings.
**Verify:** every new assertion broken on purpose once and seen to fail, as in S-35, and
the fix demonstrated by a test that is red against the current `?? 0` and green after it
goes.
**Must not:** change the engine. The invariant that a waiting job has a price is the
engine's to keep and this step does not reach into it; what is being fixed here is a
browser that assumes it. Do not add a cancel for a parked job, do not touch the `save`
button, and do not let this grow into the rest of the wizard's screens — this step is the
two-click rule and nothing else.

**Files (as built):** `ui/src/patterns/Wizard.tsx`, `ui/src/patterns/Wizard.test.tsx`, and
the bug log entry that came with the spec entry. No engine and no brain source, and
`format.ts` was read but not changed.

**Learned.**

*The coalesce was load-bearing, and that is why it was there.* `?? 0` looks like
carelessness until you notice that `Wizard.tsx` keeps its own `money` — four decimals
rather than `format.ts`'s two, because these are fractions of a cent and rounding one to
two would print `$0.00` for a run that cost money — and that it took a `number`. Someone
reached for the coalesce because the alternative was widening a formatter, and widening a
formatter is a bigger-looking change than adding two characters at a call site. The fix
is the bigger-looking one: the null branch belongs in the formatter, where the rest of the
app keeps it, and `DASH` comes from `format.ts` so there is one em dash in this codebase
and not two. A rule that is enforced in one file and worked around in another is a rule
with an exception, and the exception was on the most expensive button in the product.

*S-35's tests walked straight past this.* Three assertions established that `money(null)`
is a dash and is not a zero, and they were about `format.ts`'s `money`, which the wizard
does not use. Testing a rule at the place it is written is not the same as testing it
everywhere it is relied on, and the gap between those two is exactly where this bug was
sitting while its tests were green. The lesson is not that the S-35 tests were wrong; it
is that "the rule has a test" and "the rule holds" are different claims.

*A test can name the right thing and assert a different one.* The double-click test was
written the obvious way — two `fireEvent` clicks in a row — and it passed. Then the `went`
ref was deleted and it still passed, along with everything else. React flushes a discrete
event synchronously, so the first click had already re-rendered the button into its
`disabled` state before the second one landed: the test was asserting an attribute, not
the ref whose comment says it exists for precisely this case. Batching both clicks inside
one `act` stops the re-render happening between them, and the ref then carries its own
weight — without it, two goes are sent. Six breaks, six reds, and the one that came back
green was worth more than the five that did not.

*The engine's invariants are not the browser's guarantees.* A parked job has a price
because the engine appends the `ESTIMATE` line before it parks — except that `append`
discards the error from `append_job_log` and parks anyway. The browser was written against
the invariant rather than against the type, and `api.ts` had the type right all along:
`number | null`, with the field above it explaining that a job which has said nothing does
not have a progress of zero. Two programs in two languages agreeing on a rule is the
shape of this whole feature, and it is where the rule will keep breaking.

**Not done:** the engine end is untouched, by the step's own Must-not. `append` still
swallows its write error and `park` still runs regardless, so the state this step now
draws honestly can still be reached; making a parked job's price a column rather than a
line in a log would end it, and that is a step about the engine and not about a button.
There is still no cancel for a parked job: the way out of an unpriced quote is to change a
setting and price again, which leaves the first job holding one of the engine's four slots
until it expires. The `save` button still shows the cap rather than a quote, which is the
right figure but a different kind of promise from the one on the go. And everything S-35
left open is still open — twenty-eight of thirty-three UI files untested, no coverage
figure, and the plan ceiling still roughly ten times a real message.

### S-37 — The engine does not park on a price it cannot read ☑ [Rust] (PR #38, dcb266d)
**PR:** one. **Depends on:** S-36, S-33, S-22, S-13.
**Files:** `engine/src/jobs.rs`, `engine/tests/jobs.rs`, `docs/backlog/bugs.md`. No brain
and no UI.
**Today:** S-36 taught the browser to draw a parked job with no price honestly — a dash,
and no go behind it. This step stops the engine making one. The supervisor's rule for
parking is `line.starts_with("ESTIMATE ")` and nothing more: it appends the line and parks
on the strength of the prefix. The number is read much later and somewhere else, by
`estimate()`, when the browser asks for the job. Between the two sits a whole state the
engine believes cannot exist — a job waiting for a go with no price for anyone to
approve — and it is reachable four ways, only one of which needs anything to fail.

`append` discards the error from `append_job_log` and `park` runs regardless, which is the
one already in the bug log and the one S-36 named in its Not-done. The other three need
no failure at all, only a line: `ESTIMATE abc` does not parse, so `estimate()` finds
nothing; `ESTIMATE nan` and `ESTIMATE inf` do parse — Rust's `f64` accepts both — and
serde_json writes a non-finite float as `null`, so the browser is handed the same missing
price by a different road; and `ESTIMATE -5` parses cleanly and puts a negative number on
the go button, which is worse than a dash, because a dash says the price is unknown and
`-$5.0000` says something false. All four end at the same place: a waiting job the browser
cannot price. The brain prints `{usd:.4f}` and would have to be broken to send any of
them, which is the point — the engine's job is not to assume the brain is well, and the
one line in this system that stands between a model and a call is not the place to start.
**Change:** one function, `price(rest) -> Option<f64>`, which parses the text after
`ESTIMATE ` and returns it only when it is finite and not negative. `estimate()` reads
through it, so a log already carrying a bad line reads as no price rather than a wrong
one. The park site reads through it too, before it writes anything: a line whose price
`price` refuses is not appended and not parked on — it fails the job, quoting what the
brain actually said, and `supervise` kills the child on the way out, so nothing is read.
Only when there is a price does the log line go in, and that write is checked rather than
discarded — if the quote cannot be recorded, the job fails instead of parking, because a
price nobody can read back is not a shown cost. `append` keeps its swallowed error where
`drain` uses it: a dropped line of stderr is a dropped line of stderr.
**Acceptance:** WHEN the brain prints an `ESTIMATE` line carrying anything but a finite,
non-negative number THEN the job SHALL end `failed` with the offending text in its reason,
SHALL NOT reach `waiting`, and SHALL book no spend. WHEN the quote cannot be written to
the job's log THEN the job SHALL fail rather than park. WHEN the price is good THEN the
job SHALL park exactly as it does today.
**Verify:** a fake brain per bad line — `abc`, `nan`, `inf`, `-5` — each asserted to reach
`failed` and never `waiting`, and asserted to have spent nothing; the existing park and go
tests still green, which is what says the good path did not move; every new assertion
broken on purpose once and seen to fail.
**Must not:** move the price into a column of its own. The log is where the brain's own
words live and `estimate()` reading them back is the design, not the bug; what is being
fixed is that the engine never looked at them before acting on them. Do not touch the UI —
S-36 already draws this state correctly and will keep working when it stops arriving. Do
not change what the brain prints, do not add a validation to the Python side, and do not
touch `drain` or the stderr path.

**Files (as built):** `engine/src/jobs.rs`, `engine/tests/jobs.rs`, and the bug log entry
that came with the spec entry. No brain and no UI, and 36 tests in `jobs.rs` became 43.

**Learned.**

*Three of the four ways in needed nothing to go wrong.* The bug log had one entry and it
was about a discarded write error, which is the kind of bug that needs a bad day to
happen. Reading for the fix turned up three more that need only a line: a number that does
not parse, a number that parses to something serde_json cannot write, and a number that
parses and serialises perfectly and is simply wrong. The one in the log was the rarest of
them. A bug found by reading the code and a bug found by watching it fail are different
sizes, and the second is usually a corner of the first.

*`nan` is caught by the comparison, not by the finiteness check.* Breaking `price` down to
`usd >= 0.0` alone reddened one test out of the four bad quotes — `inf` — because `NaN >=
0.0` is already false, and so is `-inf >= 0.0`. The two halves of that condition overlap
almost entirely and neither is redundant: `is_finite()` alone lets `-5` through and `>=
0.0` alone lets `inf` through. Someone tidying that line to one comparison would keep
three of the four cases and lose the fourth in silence. The break said so; reading it
would not have.

*The self-review found the one that mattered.* The failure reason quotes what the brain
printed, and `finish` appends that reason to the job's log — so the first version of this
step wrote unscrubbed brain output into the log, on the exact path taken when the brain is
behaving strangely enough to print something other than a price on a line it holds the
keys to write. Every test was green when that was true. The fourth Must-never says never
print a key to a log, and it was the diff that said it, not the suite.

*A control can be right and untestable at the same time.* The checked log write is the
half of this step that closes the original bug-log entry, and breaking it back to the
discarded error left all forty-three tests green. Nothing in this repository can make
SQLite fail, so there is no test to write without a fault-injecting `Db` first. It stays
in — a quote that is not on record is not a shown cost — and it stays unproven, which is
a different thing from untrue and is written here so nobody reads the green suite as
covering it.

**Not done:** the checked write has no test, as above; proving it needs a `Db` that can be
told to fail, which is a step of its own and touches every handler that swallows a write.
The price is still a line in a log rather than a column, by this step's own Must-not — the
state is now unreachable rather than impossible, and the difference is one guard. Nothing
validates the `ESTIMATE` line on the Python side, so a brain that computes a bad price
still gets as far as the engine before anything says so; that is the right side for the
check to live on and the wrong side for the bug to be found on. And there is still no
cancel for a parked job, which S-36 left open and this step did not reach.

### S-38 — The second click has a no ☑ [Rust] (PR #39, 70bd768)
**PR:** one. **Depends on:** S-37, S-36, S-33, S-22, S-13.
**Files (as built):** `engine/src/jobs.rs`, `engine/src/server.rs`,
`engine/tests/jobs.rs`, `ui/src/api.ts`, `ui/src/index.css`,
`ui/src/patterns/Wizard.tsx`, `ui/src/patterns/Wizard.test.tsx`,
`docs/backlog/bugs.md`. No brain.
**Today:** the two-click rule has a yes and no no. S-22 built the park, S-36 made the
browser refuse to draw a go it cannot price, and S-37 made the engine refuse to park on a
price it cannot read — all three about the click that spends. None of them is about the
person who looks at the figure and decides against it. That person's only move is to close
the tab, and closing the tab does nothing: the job stays `waiting`, holding one of the
engine's four slots, until `GO_WAIT` kills it half an hour later. The engine already knows
this is wrong and says so twice. `POST /api/patterns/label` refuses the fifth job with
*"finish or abandon one first"*, naming a remedy that does not exist; and the wizard's own
hint tells the analyst that "the engine drops it within the half hour", which is the
product asking someone to wait out a timer rather than offering them a button. Four closed
tabs is a labelling feature that is refused for thirty minutes having read nothing.
**Change:** the parked job's channel carries a verdict rather than a signal. `Jobs::go`
sends the yes it always sent; a new `Jobs::stop` sends a no, and both take the sender out
of the map under the same lock, so a go and a stop race the way two gos already do — one
of them wins and the other finds nothing. `park` reports which of the three happened, and
a no ends the same way a timeout does: the child is killed with its stdin still open, it
has read nothing, and the row is `expired` — whose own definition is already *"killed
unspent. Not a failure: there is nothing wrong with walking away from a quote."* Only the
reason line differs, because "nobody approved the price in time" is not what happened when
somebody declined it in four seconds. `POST /api/jobs/{id}/stop` mirrors `/go` exactly,
409 and all, and the wizard grows a second button beside the go that calls it and puts the
step back where it was before the quote.
**Acceptance:** WHEN a parked job is sent a stop THEN it SHALL end `expired` with a reason
saying it was turned down, SHALL book no spend, and SHALL free its slot at once. WHEN a
stop names a job that is not parked THEN it SHALL answer 409, as `/go` does. WHEN a go and
a stop reach the same job THEN exactly one SHALL take effect. WHEN the analyst declines a
quote in the wizard THEN the run SHALL return to the state it was in before the quote, and
a new quote SHALL be startable without waiting.
**Verify:** engine — a parked job stopped and seen to reach `expired` with nothing booked
on the row, the day, or the ledger; a stop against a finished job answered 409; a stop
after a go, and a go after a stop, each refused; `live_jobs` seen to fall back below
`MAX_LIVE` immediately after a stop, which is the whole point of the step. UI — the
decline button asserted to appear only while a job is parked, to call the route, and to
leave the wizard able to price again; and asserted **not** to be the button that spends.
Every new assertion broken on purpose once and seen to fail.
**Must not:** add a `stopped` status. `expired` is documented as a walk-away that cost
nothing, which is exactly what this is; a second status that every reader would treat
identically and that differs only in how fast it arrived is a column for a sentence, and
the reason line is where that sentence belongs. Do not make the stop kill anything but a
parked job — a `running` job has already spent, and stopping it would be a refund the
engine cannot give. Do not touch the brain, which never learns the difference: it is
killed with its stdin open either way. Do not add a resume; a declined quote is priced
again from the top, because the selection may have moved and S-33 exists for that reason.

### S-39 — A cost that is not counted ☑ [Rust] (PR #40, e96a59e)

**PR:** one. **Depends on:** S-38, S-37, S-22, S-13, D-8.
**Files:** `engine/src/db.rs`, `engine/src/jobs.rs`, `engine/tests/jobs.rs`. No brain, no UI.

**Why:** `finish` closes every job out, and the comment above it states the invariant its
ordering exists to hold: *"the spend is written before the status, so a job that reads
`done` is a job whose cost has already been counted against its org."* The ordering is
right and the next three lines throw it away — `let _ = db.add_spend(...)` and then `let _
= db.finish_job(...)`, unconditionally. A failed spend does not stop the status being
written, so the row reads `done`, carries its `cost_usd`, and the ledger never hears about
it. That would be a wrong figure on a report if the `spend` table were read by a report.
It is read by one caller: `sync.rs:222` computes `let left = opts.cap_usd -
db.spend_on(&now()[..10], org.id)?`, which is the whole of the daily cap. One line writes
that table and one line reads it, and the write's error is discarded. So a lost spend
means every later run that day sizes itself against a ledger that is short, and the hard
USD cap is exceeded by exactly the money already spent — quietly, with the job that spent
it reading `done` and looking correct. That is the second Must-never, and the same
discarded error also leaves a job whose close fails sitting `running`, holding one of
`MAX_LIVE` slots that S-38 just went to the trouble of handing back.

**Change:** one database call closes a job. `Db::finish_job` grows the booking it was
always paired with and does both writes inside a transaction, so *done implies booked*
holds by construction rather than by the order of two statements that can fail
separately — either the ledger and the row both move or neither does. `finish` checks the
result. A close that fails leaves the row exactly as it was, which is `running`: an
unfinished job is a truthful thing to be, it is swept to `expired` by
`abandon_live_jobs` at the next boot, and it is never a `done` that nobody paid for. The
failure is reported the only two ways left when the database is the thing that is broken —
appended to the job's log on a best-effort basis, and printed to stderr, which is where
`cli.rs` already sends what the operator needs to see.

**Test seam:** none is built. A trait-shaped `Db` that can be told to fail was named as a
step of its own in S-37 and again in S-38, and it is not needed: SQLite can be told to
fail by SQL. `CREATE TRIGGER … BEFORE INSERT ON spend BEGIN SELECT RAISE(ABORT, …); END;`
makes exactly one table's writes fail, on a real connection, with a real `rusqlite::Error`
on the real path — and `serve_with(brain, fill)` already hands a test the `Db` before the
server boots. The fault is one statement in a closure the harness already has.

**Acceptance:** WHEN the spend write fails THEN the job SHALL NOT be recorded `done`, and
the row SHALL carry no cost the ledger has not counted. WHEN the spend write succeeds and
the status write fails THEN neither SHALL be visible, so the ledger never runs ahead of the
row either. WHEN a job is closed normally THEN the ledger and the row SHALL move together
and `spend_on` SHALL report the cost, as today. WHEN any close fails THEN the reason SHALL
reach the operator.

**Verify:** engine — a labelling job run to completion against a `spend` table that
refuses inserts, asserted to leave a row that is not `done` and a `spend_on` of 0.0, with
the two never disagreeing; the mirror case, a `jobs` table that refuses the close, asserted
to leave `spend_on` at 0.0 rather than booked-and-unrecorded; the ordinary path asserted
unchanged, both figures present; and S-37's checked log write finally proved with the same
trigger on `jobs`, retiring the "right and unproven" note that step and S-38 both carry.
Every new assertion broken on purpose once and seen to fail.

**Must not:** add a retry. A database that is refusing writes is not fixed by being asked
twice, and a retry loop inside `finish` holds the `Db` mutex while it sleeps, which stops
every other handler. Do not invent a status for this — S-38's lesson stands, and `running`
is already the true word for a job that was not closed. Do not widen to the other
swallowed writes in the tree: `server.rs:71`'s `abandon_live_jobs` is the recovery path
for this failure and deserves its own reading, not a rider on this one. Do not make
`spend_on` fail closed when it suspects a gap; it cannot suspect one, and a cap that
refuses to spend on a hunch is a different feature.
**Files (as built):** `engine/src/db.rs`, `engine/src/jobs.rs`, `engine/tests/jobs.rs`. No
brain, no UI. 246 → 250 engine tests; `tests/jobs.rs` 47 → 51.

**Learned:**

*The comment was the bug report, for the second step running.* S-38 found a missing feature
by reading the strings the product printed at people. This one was found by reading a doc
comment the code printed at itself: *"the spend is written before the status, so a job that
reads `done` is a job whose cost has already been counted."* The ordering it describes is
real and deliberate, and the two `let _ =` beneath it make the conclusion false. A comment
that states an invariant is a claim that can be checked against the lines under it, and
checking it is cheaper than deriving the invariant from scratch. Both of the last two steps
came out of somewhere the codebase had already written down what it was supposed to do.

*One writer, one reader, and a discarded error between them.* What made this a Must-never
break rather than a reporting bug is that the `spend` table has exactly one writer
(`jobs.rs`) and exactly one reader (`sync.rs:222`, the daily cap). Nothing else would ever
notice the gap, so nothing else would ever correct it, and the failure mode is a cap that
is silently higher than the operator set — the quietest possible form of the second
Must-never. Counting the writers and readers of a piece of state is a fast way to tell a
wrong number from an unsafe one.

*The fault seam did not need building, and had been priced as a step twice.* S-37 said
proving its checked write needed "a `Db` that can be told to fail, which is a step of its
own and touches every handler that swallows a write," and S-38 repeated it. Both were
wrong about the size. SQLite can be told to fail by SQL — `CREATE TRIGGER … BEFORE INSERT
ON spend BEGIN SELECT RAISE(ABORT, …); END;` fails one table's writes on a real connection
with a real `rusqlite::Error` on the real path — and `boot`'s `fill` closure already handed
a test the database before the server started. The seam was one statement in a hook that
had existed since S-22. Twice an estimate was made by imagining the shape of the solution
(a trait, a mock, every call site) rather than by asking what the smallest thing that could
produce the failure was, and twice it came out an order of magnitude high.

*Ordering two writes is not the same as making them one.* The original code had the right
idea — spend first, status second, so a crash between them leaves the ledger ahead rather
than behind — and that reasoning only holds if the first write cannot fail on its own.
Inside a transaction the invariant needs no reasoning at all: both land or neither does,
and nobody maintaining this has to remember which order was the safe one.

*A green break is not always an untested control.* Break 5 dropped the `cost_usd > 0.0`
guard and nothing reddened, which looks like the finding S-37 and S-38 both ended on. It
is not: a zero-dollar spend row changes no sum, no `spend_on`, and no cap, so the guard
saves a pointless row and guards nothing. The distinction matters, because "unproven
control" is a debt and "untested optimisation" is not, and treating the second as the first
is how a test suite fills up with assertions about things that cannot go wrong.

**Not done:** `server.rs:71`'s `let _ = db.abandon_live_jobs(...)` is still swallowed, and
it is now the recovery path for the failure this step created a truthful state for — a row
left `running` is only rescued if that sweep works, and if it does not the job holds one of
`MAX_LIVE` slots forever, which is the bug S-38 fixed arriving by another road. It has a
seam now, so it is a small step rather than a large one. S-38's other unproven control —
`park` marking a declined job `running` for the millisecond before `finish` overwrites it —
is untouched, because that one needs a clock the test owns rather than a database that can
fail; it is the last of the two, and it is the smaller. Nothing prunes the `jobs` table or
the `spend` table. And the operator only learns about a failed close from stderr and a job
log nobody is watching; there is no place in the UI where "this job did not close" is
visible, which is the difference between recording a failure and reporting one.

### S-40 — The sweep that could not run says so ☑ [Rust] (PR #41, c261fd6)

**PR:** one. **Depends on:** S-39, S-38, S-22.
**Files:** `engine/src/server.rs`, `engine/tests/jobs.rs`. No `db.rs`:
`abandon_live_jobs` already returns the thing this step stops throwing away. No brain, no
UI.

**Why:** `App::new` clears whatever the last process left live, and the comment above the
call already states what happens if it does not run — *"Left alone, four abandoned
`waiting` rows would count against the live cap for ever and no job would ever start
again."* The line beneath that comment is `let _ = db.abandon_live_jobs(...)`. The one
operation whose failure this file has already written down as permanent is the one whose
failure nobody is told about.

It is permanent because `live_jobs` counts rows and not children: `server.rs:667` refuses
a fifth job by asking the `jobs` table how many read `running` or `waiting`, so four rows
left by a dead process are four slots gone. They stay gone across restarts, because the
next boot runs the same sweep against the same database and discards the same error. S-38
spent a whole step giving a stuck job a way out, and a ghost row has none: `stop` reaches
the in-memory registry, and a row whose process is gone is in no registry. What the
operator sees is every plan, clarify, label, synthesize and ask answering 429 with
*"finish or abandon one first"* — advice that cannot be taken, about jobs that do not
exist. Everything else goes on working, which is exactly why it would not be noticed.

This is also the last one. Every other `let _ =` in the tree is deliberate and stays:
opening a browser (`server.rs:137`), `launchctl bootout` before an install that has not
happened yet (`schedule.rs:224`), killing a child that may already be dead
(`jobs.rs:387`), joining a logger thread (`jobs.rs:507`), and the three job-log appends
S-39 documented as best-effort because the log lives in the database that just failed.
After this step, nothing is being ignored that costs anything.

**Change:** the sweep is checked and what its failure costs is said. A named function does
it — `sweep_abandoned(&db) -> Option<String>`, returning the sentence the operator needs
rather than a `Result`, because the thing worth proving here is not that SQLite can fail
but what the product says when it does. `App::new` prints that sentence to stderr, which
is where `cli.rs` already sends what the operator has to see, and goes on to serve. The
sentence names the consequence and not only the error: the rows were not cleared, they
still count against the limit, and new jobs may be refused until the database is writable
and graphify is started again.

Serving anyway is the existing policy and it is kept, because it is right. The failure
costs one feature — the job queue — and charts, sync, patterns and settings all read other
tables and are unharmed; refusing to boot would turn a partial outage into a total one on
the evidence of a single `UPDATE`. The comment was right about the policy and wrong only
in being silent about it.

**Test seam:** the one S-39 built, unchanged. `boot`'s `fill` closure runs before
`App::new`, so a trigger installed there is already in place when the sweep runs. It must
fire on a column the sweep writes and S-39's triggers do not: `abandon_live_jobs` sets
both `status` and `finished_at`, `NO_CLOSE` already takes `finished_at`, so this one takes
`status` and the two stay separable.

**Acceptance:** WHEN the sweep fails THEN the server SHALL still serve every route that
does not depend on it. WHEN the sweep fails THEN the operator SHALL be told in one
sentence that names the consequence — slots held, jobs refused — and not only the SQLite
error. WHEN the sweep fails THEN the rows it could not clear SHALL still be readable
through `GET /api/jobs/{id}`, so the old comment's claim that they are "visible in the API
either way" is proved rather than assumed. WHEN the sweep succeeds THEN nothing SHALL
change, including the absence of any new output.

**Verify:** engine — `sweep_abandoned` over a `jobs` table that refuses the update,
asserted to return a sentence naming the live limit; the same database booted into a
server and asked for `/api/orgs`, asserted 200; a stale row asserted still `running` and
still readable through the API after that boot; the ordinary case asserted to return
`None`, with S-22's existing dead-engine sweep test left green. Every new assertion broken
on purpose once and seen to fail.

**Must not:** refuse to boot — see above; the sweep is one feature's recovery, not the
product's. Do not retry: S-39's reasoning holds, a database that is refusing writes is not
fixed by being asked twice. Do not print on success — a line on every restart with a live
job is noise, and the rows are already visible as `expired`. Do not touch the other
`let _ =` sites listed above; they are deliberate and the survey exists so that stays
decided. Do not make `live_jobs` cleverer by cross-checking the registry — a count that
second-guesses the table is a different feature and a worse one. Do not add a
`#[cfg(test)]` module to `src/`; there are none, and the library target exists so
`engine/tests/` can reach what it needs.
**Files (as built):** `engine/src/server.rs`, `engine/tests/jobs.rs`. No `db.rs`, no
brain, no UI. 250 → 253 engine tests; `tests/jobs.rs` 51 → 54.

**Learned:**

*The comment was the bug report, for the third step running.* S-38 read the strings the
product printed at people, S-39 read a doc comment the code printed at itself, and this one
read the comment above the call — which named the exact consequence of the failure on the
very next line it then discarded. Three steps out of one habit. What the habit actually is:
a comment that states a consequence is a claim, and the code beneath it either upholds the
claim or it does not, and checking which is cheaper than deriving the consequence from
scratch. The codebase had already done the hard thinking each time; it just had not
finished the sentence.

*A survey is what makes a step complete rather than arbitrary.* Before writing this, every
`let _ =` in the tree was listed: eight of them, and after this one, seven that are
deliberate — a browser that will not open, a `launchctl bootout` for something not yet
installed, a child that may already be dead, a logger thread being joined, and three
job-log appends whose log lives in the database that just failed. Without that list this
step is "fix one more swallowed error and probably find another next week." With it, the
step has an end: nothing is being ignored that costs anything. The survey took one `grep`
and it is written into the spec so it stays decided.

*Returning the sentence instead of the `Result` is what made the report testable.* The
easy shape is `Result<usize>` at the call site and a `format!` inside `App::new`, and then
the only thing a test can reach is that SQLite failed — which was never in doubt. Moving
the sentence into a named function put the part that actually matters, what the operator
is told, somewhere an assertion can read it. Break 2 proves the difference: a version that
prints the rusqlite error and nothing else passes every behavioural test and fails the one
that asks whether anyone would understand it.

*The failure had to be tested for what it costs, not only for what it breaks.* The boot
test asserts three things at once — the 429 arrives, `/api/orgs` still answers, and the
rows the sweep could not clear are still readable. Together they are the argument for the
policy: the queue is gone, the product is not, and the state is inspectable. Any one of
them alone would have been a weaker test of a decision that was really about proportion.

*Break 5 reddened a test from S-22.* Pointing the sweep at a status that does not exist
broke the new assertions and the eighteen-step-old `a_job_left_running_by_a_dead_engine_
does_not_block_the_next_one`. That is the useful signal that the new tests are not a
restatement of the old one: S-22 proves the sweep works, S-40 proves what happens when it
does not, and only a break that removes the sweep entirely hits both.

**Not done:** the wiring itself is unproven. The tests reach `sweep_abandoned` and they
reach the server, but nothing asserts that `App::new` actually prints — capturing an
in-process `eprintln!` from an integration test is not worth what it would cost, and break
3 shows the boot test catches a wrong policy there, which is the part that matters. The
report still only reaches stderr: there is no place in the UI where "the queue is held by
rows from a dead run" is visible, which is S-39's unfinished sentence arriving again and is
now the same gap for two different failures — it wants one notice mechanism, not two
messages. S-38's last unproven control, `park` marking a declined job `running` for the
millisecond before `finish` overwrites it, is still untouched; it needs a clock the test
owns rather than a database that can fail, and it is now the only one left. Nothing prunes
the `jobs` table or the `spend` table. And the 429's own wording is still *"finish or
abandon one first"* without saying where — true when the queue is genuinely full, and it
is the sentence a person reads in both cases.

### S-41 — One place the failures are visible ☑ [Rust] (PR #42, 35cb904)

**PR:** one. **Depends on:** S-40, S-39, S-38.
**Files:** `engine/src/notices.rs` (new), `engine/src/lib.rs`, `engine/src/server.rs`,
`engine/src/jobs.rs`, `engine/tests/jobs.rs`, `ui/src/api.ts`, `ui/src/Notices.tsx` (new),
`ui/src/Notices.test.tsx` (new), `ui/src/App.tsx`, `ui/src/index.css`. Not `sync.rs`: the
daily keeps the signature it has, for the reason below.

**Why:** two steps running have ended on the same unfinished sentence. S-39 made a failed
close say so and S-40 made a failed sweep say so, and both say so to stderr — which is
nowhere, in a product whose whole shape is a browser tab somebody left open. The person who
needs to know that the queue is held by rows from a dead run is the person looking at the
screen, and what that person is shown instead is a 429 saying *"finish or abandon one
first"* about jobs that do not exist.

They are one class, not two. Both are the database refusing a write to `jobs`, and that
settles where a notice can live: **a notice about a failed write cannot be written to the
database that failed the write.** There is no table for this. The board is in memory and
lasts as long as the process.

Which has a second consequence, and it is the good one. Notices do not survive a restart —
and that is right rather than a limitation, because the remedy the sweep's own sentence
names *is* a restart, and a restart re-runs the sweep, which either clears the rows and the
notice is gone truthfully or fails again and the notice comes back. A board that cannot go
stale needs no dismiss button. The restart is the dismiss button.

The one thing that makes any of this reachable: both failures are *write* failures. Login,
charts and this route all read, so they all still answer while the queue is dead. That is
why the notice can be shown at all, and it is the same asymmetry S-40 leaned on when it
decided to serve anyway.

**Change:** `notices.rs` holds `Notices`, a `Mutex<VecDeque<Notice>>` with `push(&self,
text)` and `all(&self) -> (Vec<Notice>, usize)` — the kept list newest first, and how many
were dropped. `Notice` is `{ at, text }` and nothing else: no id, no severity, no source. A
database refusing every write closes every job with a failure, so the board is bounded at
twenty and reports what it dropped rather than truncating in silence; a swallowed count is
the exact thing this register has spent three steps taking out of the tree.

`App` gains `notices: Arc<Notices>`. `App::new` pushes the sweep sentence as well as
printing it. `start` takes the handle and hands it to `supervise` and on to `finish`, which
pushes the close failure. `run_blocking` keeps its signature and makes its own board, the
way it already makes its own `Jobs`: it is `graphify sync` on a schedule, no browser is
attached to that process, nobody can read a board it takes with it when it exits, and
`cli.rs` already prints what that operator sees.

`GET /api/notices` goes behind the same gate as everything else and answers
`{"notices": [...], "dropped": n}`.

The close's own sentence gains the consequence S-40 says it should have had, because it is
being put in front of a person now: *"could not close this job out: {e}"* names the error
and not the cost. The cost is that the job stays `running`, holds one of the four slots,
and holds it until a restart sweeps it.

On the UI side, `Notices` is a banner under the header and above the error line, on every
screen, because an outage is not a property of the screen you happen to be on. It polls
every thirty seconds: a close that fails while somebody is watching has to arrive without a
reload, and the condition is not urgent to the second — it lasts until a restart either way.

**Test seam:** S-39's triggers, a third time and unchanged. `NO_SWEEP` for the boot notice,
`NO_CLOSE` for the job notice. Nothing new is needed, which is the third step in a row that
has been true.

**Acceptance:** WHEN the sweep fails THEN `GET /api/notices` SHALL answer with a notice
naming the consequence and not only the error. WHEN a close fails THEN a notice SHALL name
the job and what holding it costs. WHEN neither has failed THEN the route SHALL answer an
empty list and a zero. WHEN more notices arrive than the board keeps THEN the response SHALL
say how many it did not keep. WHEN there are notices THEN the banner SHALL be on screen
whichever screen is showing, and what was dropped SHALL be on it. A notice SHALL never carry
a key or a line the brain wrote.

**Verify:** engine — the board's cap and its dropped count directly; a server booted over a
`jobs` table that refuses `status` writes, then asked for `/api/notices`; a job run to a
close that refuses `finished_at`, then the same route; a clean boot asserted empty. ui — the
banner with none, with one, and with more than the board kept. Every new assertion broken on
purpose once and seen to fail.

**Must not:** put the board in the database; that is the one storage the failure has already
taken away. No dismiss, no acknowledge, no read-and-unread — the condition is true until it
is not, and the restart ends it. No severity levels, no notice ids, no source field, and no
third caller: two failures have a mechanism, and a general place to log anything is a
different product and a worse one. Nothing the brain wrote and no secret goes into a notice;
both producers pass rusqlite's own words about the engine's own statements, and that is all
they pass. No websocket and no SSE behind a banner that changes twice a year. Do not fail
the page on it: a notices request that does not answer leaves the banner as it was and puts
nothing on the error line — the route being unreachable is not itself news, and that line is
for requests a person made.
**Files (as built):** as listed, plus `engine/tests/serve.rs` — the gate test belongs where
a server with a password already exists, not where one is invented for it. 253 → 258 engine
tests; `tests/jobs.rs` 54 → 59. 41 → 46 ui tests.

**Learned:**

*The cause decided the storage, and the storage turned out to be the feature.* The board
could not be a table, because both things that push to it are the database refusing a
write; that is a constraint, and it was accepted rather than worked around. What fell out
of it is the part worth keeping: a board that lives for the life of the process cannot go
stale, because the remedy its own sentence names is a restart and a restart re-runs the
sweep. So there is no dismiss button, no acknowledged flag, no read-and-unread, and none of
them are missing. The restart is the dismissal. Half the features this step did not build
were ruled out by one sentence about where the failure was.

*Clippy's argument limit found the abstraction, and S-39's precedent said which way to go.*
Adding the board pushed `start` and `finish` to eight arguments each. The cheap answer is
`#[allow(clippy::too_many_arguments)]`; S-39 had already refused it once, deriving the day
from `finished_at` rather than passing both. The same answer worked here and is better than
either: `Records` bundles the row and the board because they are one idea — the two places
a job's ending is written down — and it names that idea in a file that was carrying it
implicitly. A lint about argument count is sometimes a lint about a missing noun.

*The third step running that needed no new test harness.* S-39 built the `RAISE(ABORT)`
trigger seam, S-40 reused it, and this step used two of its triggers unchanged. The
estimate at the time was a trait-based mock of `Db`; what it took was four lines of SQL.
The seam has now paid for itself three times, which is the argument for spending a step's
budget on one rather than on the feature it was in the way of.

*The one hazard in the diff was not in the feature.* Self-review found `finish` pushing to
the board while still holding the database lock — safe, because nothing anywhere takes the
board first and the database second, but safe by an invariant nothing had written down. The
comment is the change; the code was already right. Worth noticing that the review caught a
lock order and not a missing assertion, which is the kind of thing the tests could not have
found.

*One break reddened the run without reddening a test, and it is recorded that way.* Break 8
— rethrowing instead of swallowing a failed notices request — fails CI through an unhandled
rejection, not an assertion, because the assertion's claim (the page is unaffected) is true
either way. It is a real red and a weaker one than the other seven, and saying so is worth
more than a break table with eight tidy rows.

**Not done:** the banner polls, which is the crudest thing that works and is the right
crudeness for something that changes twice a year; nothing pushes. The `eprintln!` wiring
S-40 left unproven is still unproven, and the board now covers what that gap actually cost,
so it is less worth doing than it was. Nothing prunes the `jobs` or `spend` tables. S-38's
`park`/`running` control is still the last unproven one and still needs a clock the test
owns. The 429's own wording still says *"finish or abandon one first"* without saying
where — and now there is a banner it could point at, which makes that a smaller step than
it was yesterday. `run_blocking` fills a board nobody reads: honest, documented, and still a
shape that would mislead a reader who found it before the comment.

### S-42 — What a declined job never did [Rust]
**PR:** one. **Depends on:** S-38, S-39. **Files:** `engine/tests/jobs.rs`. No engine
source, no brain, no UI: the control this step is about is already written and already
right, and what is missing is the sentence that says so.
**Today:** `park` carries a comment — *"Only a go changes the row"* — and nothing checks
it. Three steps running have called it the last unproven control here and deferred it for
the same stated reason: that it "needs a clock the test owns", the window in question being
the millisecond between a decline and the `expired` written over it. That reason is wrong
twice. It is wrong about the window, because `begin` passes `RUNNING` to `create_job`, so
every job is `running` from the moment it exists and stays that way for as long as the brain
takes to quote — not a millisecond, and not a defect either, because `running` is documented
as *"the job is alive and its subprocess is working"* and it is. And it is wrong about the
clock, because the claim is not about what the row said at some instant. It is about which
statements ran, and a database can be asked that directly.
**Change:** nothing in `engine/src`. A trail on the test side — one table and two triggers
installed by `boot`'s `fill`, an `AFTER INSERT` and an `AFTER UPDATE OF status` on `jobs` —
records every status a job is ever written, in order, and the tests assert the whole history
instead of polling the current one. That is S-39's seam a fourth time and for the opposite
purpose: `RAISE(ABORT)` makes a write fail, and this makes a write remembered.
**Acceptance:** WHEN a parked job is turned down THEN its history SHALL be `running`,
`waiting`, `expired` — `running` exactly once, and before the price was ever shown. WHEN a
parked job is left to time out THEN its history SHALL be those same three. WHEN a parked job
is told to go THEN its history SHALL be `running`, `waiting`, `running`, `done`. WHEN a job
of a kind that never parks runs THEN its history SHALL be `running`, `done`.
**Verify:** the go case is not decoration and is named as the guard it is — a trail that
records nothing would pass all three of the others, so the one history that must contain a
`running` after a `waiting` is what proves the recorder can see one at all. Every new
assertion broken on purpose once and seen to fail; the file green afterwards.
**Must not:** add a seam to `engine/src/` for this. The trail belongs in the test file
because the claim is about statements SQLite already sees, and a production hook that exists
only so a test can watch is the mock this seam was built to avoid. Do not turn the
`recv_timeout`/`try_recv` race in `park` into an assertion: a go landing in the channel while
the wait is timing out does need a clock the test owns, and it is a different thing from the
control this step closes.

---

**The register is complete through S-42.** Anything after that is a new step appended
here, or a bug in `docs/backlog/bugs.md` promoted to one.
