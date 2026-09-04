# graphify-brain

The BAML-powered brain for [graphify](../README.md): plan, clarify, label, synthesize, ask.
Talks to the Rust engine over JSON on stdin/stdout.

## Models and prices

`baml_src/clients.baml` declares the three clients the brain may call. `src/graphify_brain/cost.py`
holds what each one costs, so a call can be priced before it is made.

```
uv run graphify-brain models           # the table, and how old it is
uv run graphify-brain models --check   # ...and what the providers now list
```

`--check` reads each provider's `GET /v1/models` with the key already in the environment.
It makes no model call and costs nothing. It reports a configured model the provider has
retired (exit 1) and any model released after the one a client points at (exit 0 — news,
not a failure).

**Prices cannot be fetched.** Neither provider publishes rates through an API, so the
numbers in `cost.py` are read off the pricing pages named in that file by a person, and
`PRICES_CHECKED` records the day. `models` warns once the reading is older than
`STALE_AFTER_DAYS`; nothing fails on a calendar date.
