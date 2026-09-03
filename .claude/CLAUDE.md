# graphify — project conventions

Open-source Vapi call analytics. Add a Vapi key, pull calls, see every chart, define
patterns in plain English through a wizard, re-count them daily for free.

**Read `docs/spec.md` first, always.** It has the decisions, the data model, the rule
DSL, and the step register. The next step is the first `☐`. After a context compaction
that file plus `python3 ~/.claude/MEM0/MEMOS/memo.py show graphify` is the whole memory.

## Stack (decided in spec D-1, do not re-litigate)
- `engine/` Rust binary `graphify`: sync, SQLite (rusqlite), rule engine, axum API,
  serves `ui/dist`, spawns the brain. **Rust steps (tagged `[Rust]`) are Fable/Opus only.**
- `brain/` Python 3.11 + BAML, package `graphify-brain`: plan, clarify, label,
  synthesize, ask. Reads the same SQLite. JSON on stdin/stdout, `PROGRESS n/m` on stderr.
- `ui/` React + TypeScript + Vite + Recharts. Load the `dataviz` skill before chart code.
- One SQLite file `data/graphify.db`. No ORM. Docker optional. Password optional.

## Hard rules (spec "Must never")
- Vapi is GET-only. A unit test greps `engine/src/vapi.rs` for non-GET verbs.
- No model call without a shown cost and an explicit go. Daily modes have a USD cap.
- No audio download. URL only.
- Keys: encrypted in SQLite or env. Never in a response, log, argv, or the browser.
- Missing value → NULL → "—". Never 0.

## Commands
```
cd engine && cargo test -q && cargo clippy -- -D warnings
cd brain  && uv sync && uv run baml-cli generate && uv run pytest -q
cd ui     && pnpm i && pnpm build
```

## Shipping a step
Branch `s<n>-<slug>` from `main` → do only that step → verify commands green →
self-review diff → PR titled with the step → CI green → squash-merge, delete branch →
mark the step `☑` in `docs/spec.md` with what was learned → update the memo → report,
ending with **Master Abhishek**. Bugs go to `docs/backlog/bugs.md` first, then a step.

See `~/.claude/skills/mystandard/SKILL.md` for the full standard.
