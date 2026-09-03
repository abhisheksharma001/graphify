# graphify — project conventions

Open-source Vapi call analytics. Pull calls with an API key, see charts, define
patterns in plain English, re-count them daily for free. Read `docs/spec.md` before
touching anything; the step register there is the only source of "what next".

## Stack (decided, do not re-litigate)
- Python 3.11, managed by `uv`. Package at `src/graphify/`. CLI entry `graphify`.
- SQLite via stdlib `sqlite3`. One file: `data/graphify.db` (gitignored).
- FastAPI for the local API (`graphify serve`). `httpx` for Vapi calls.
- UI: React + TypeScript + Vite + Recharts at `ui/`. Built assets are served by FastAPI.
- No Rust in v1. No ORM. No Docker. No auth (localhost only).

## Rules
- Vapi is READ-ONLY. Only `GET` requests to `https://api.vapi.ai`. Never create,
  update, or delete anything on Vapi.
- No LLM call without an explicit go: CLI needs `--yes`, UI needs a confirm click,
  and both show the estimated cost first.
- Secrets come from env vars only: `VAPI_API_KEY`, `ANTHROPIC_API_KEY`, `OPENAI_API_KEY`.
  Never write them to disk, logs, or the DB.
- Recordings are never downloaded. Store the URL only.
- "Absent is not zero": if a payload field is missing, store NULL and render "—".
- One step = one PR. Branch `s<n>-<slug>`, squash-merge, delete branch.

## Commands
```
uv sync                      # install
uv run graphify --help
uv run pytest -q             # must be green before any PR
cd ui && pnpm i && pnpm build
```

## Working style
See `~/.claude/skills/mystandard/SKILL.md`. Reports end with **Master Abhishek**.
