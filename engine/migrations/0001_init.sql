-- Initial schema. Column set is the Data model in docs/spec.md, verbatim.
-- Everything a chart needs is a column; `slim` holds the trimmed payload for the
-- call drawer. Raw Vapi payloads are never stored.

CREATE TABLE orgs (
  id         INTEGER PRIMARY KEY,
  name       TEXT NOT NULL UNIQUE,
  provider   TEXT DEFAULT 'vapi',
  keep_days  INTEGER DEFAULT 14,
  max_calls  INTEGER NULL,
  created_at TEXT
);

-- name: vapi | anthropic | openai. Ciphertext only; the plaintext key never lands here.
CREATE TABLE secrets (
  org_id     INTEGER NULL,
  name       TEXT,
  ciphertext BLOB,
  last4      TEXT,
  updated_at TEXT,
  PRIMARY KEY (org_id, name)
);

CREATE TABLE assistants (
  id                   TEXT PRIMARY KEY,
  org_id               INTEGER,
  name                 TEXT,
  version              TEXT,
  model_provider       TEXT,
  model                TEXT,
  voice_provider       TEXT,
  transcriber_provider TEXT,
  transcriber_model    TEXT,
  system_prompt        TEXT,
  prompt_sha256        TEXT,
  first_message        TEXT,
  tool_ids             JSON,
  structured_schema    JSON,
  fetched_at           TEXT
);

CREATE TABLE tools (
  id          TEXT PRIMARY KEY,
  org_id      INTEGER,
  name        TEXT,
  type        TEXT,
  is_transfer INTEGER,
  fetched_at  TEXT
);

CREATE TABLE calls (
  id                       TEXT PRIMARY KEY,
  org_id                   INTEGER,
  assistant_id             TEXT,
  assistant_version        TEXT,
  phone_number_id          TEXT,
  call_type                TEXT,
  status                   TEXT,
  created_at               TEXT,
  started_at               TEXT,
  ended_at                 TEXT,
  duration_s               REAL,
  ended_reason             TEXT,
  ended_group              TEXT,
  cost                     REAL,
  cost_stt                 REAL,
  cost_llm                 REAL,
  cost_tts                 REAL,
  cost_vapi                REAL,
  cost_transport           REAL,
  cost_analysis            REAL,
  llm_prompt_tokens        INTEGER,
  llm_completion_tokens    INTEGER,
  llm_cached_tokens        INTEGER,
  tts_characters           INTEGER,
  transferred              INTEGER,
  transfer_destination     TEXT,
  tool_calls               INTEGER,
  tool_failures            INTEGER,
  turns                    INTEGER,
  lat_turn_avg_ms          REAL,
  lat_turn_p50_ms          REAL,
  lat_turn_p95_ms          REAL,
  lat_model_avg_ms         REAL,
  lat_voice_avg_ms         REAL,
  lat_transcriber_avg_ms   REAL,
  lat_endpointing_avg_ms   REAL,
  turn_latencies           JSON,
  success_eval             TEXT,
  summary                  TEXT,
  structured               JSON,
  transcript               TEXT,
  recording_url            TEXT,
  slim                     JSON,
  synced_at                TEXT
);

CREATE INDEX idx_calls_org_created ON calls (org_id, created_at);
CREATE INDEX idx_calls_assistant   ON calls (assistant_id);
CREATE INDEX idx_calls_ended_group ON calls (ended_group);

CREATE TABLE tool_calls (
  call_id             TEXT,
  name                TEXT,
  seconds_from_start  REAL,
  failed              INTEGER,
  arguments           TEXT,
  result_excerpt      TEXT
);

CREATE INDEX idx_tool_calls_call ON tool_calls (call_id);

CREATE TABLE patterns (
  id            INTEGER PRIMARY KEY,
  org_id        INTEGER,
  name          TEXT,
  criterion     TEXT,
  assistant_ids JSON,
  plan          JSON,
  rule          JSON,
  chart         JSON,
  model         TEXT,
  mode          TEXT DEFAULT 'free',
  daily_cap_usd REAL DEFAULT 1.0,
  sample_size   INTEGER,
  agreement     REAL,
  created_at    TEXT
);

CREATE TABLE pattern_labels (
  pattern_id INTEGER,
  call_id    TEXT,
  llm_match  INTEGER,
  rule_match INTEGER,
  evidence   TEXT
);

-- source: rule | llm
CREATE TABLE pattern_matches (
  pattern_id INTEGER,
  call_id    TEXT,
  source     TEXT
);

CREATE TABLE jobs (
  id          INTEGER PRIMARY KEY,
  kind        TEXT,
  status      TEXT,
  input       JSON,
  output      JSON,
  cost_usd    REAL,
  log         TEXT,
  created_at  TEXT,
  finished_at TEXT
);

CREATE TABLE spend (
  day    TEXT,
  org_id INTEGER,
  usd    REAL,
  PRIMARY KEY (day, org_id)
);

-- which charts are enabled, and their order
CREATE TABLE dashboard (
  org_id INTEGER PRIMARY KEY,
  config JSON
);
