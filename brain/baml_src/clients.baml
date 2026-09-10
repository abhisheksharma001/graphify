// The three models the brain is allowed to call, and where their keys come from.
//
// A key is never written here. `env.ANTHROPIC_API_KEY` / `env.OPENAI_API_KEY` are the
// names the engine exports when it spawns `graphify-brain` (engine/src/secrets.rs maps
// the stored secrets `anthropic` and `openai` to exactly these), so the key travels in
// the child's environment and appears in no file, no argument list and no log line.
//
// Every model named below must also have a price in `graphify_brain.cost`. That is not a
// convention — `tests/test_cost.py` reads this file and fails if a model here is unpriced,
// because an unpriced model is a model whose spend the daily cap cannot count.

// `max_tokens` is written on every client rather than left to the provider's default, and
// that is the whole of this file's part in the second Must-never. Every price the brain
// shows is input tokens counted at an over-counting rate plus output tokens priced at
// `MAX_OUTPUT_TOKENS`, and `plan.py`, `label.py` and `ask.py` each say in as many words
// that the output half is a *bound* because this is what BAML sends. It was a bound on
// two of the three: the Messages API requires `max_tokens`, so BAML filled one in for the
// anthropic clients; the openai request went out with `messages` and `model` and nothing
// else, which that API reads as "up to the model's own limit". A default belongs to a
// provider and can move with a version bump. The number the quote is built from belongs
// here. `tests/test_wire.py` renders every function on every client and reads it back.

client<llm> Opus {
  provider anthropic
  retry_policy Backoff
  options {
    model "claude-opus-5"
    api_key env.ANTHROPIC_API_KEY
    max_tokens 4096
  }
}

client<llm> Sonnet {
  provider anthropic
  retry_policy Backoff
  options {
    model "claude-sonnet-5"
    api_key env.ANTHROPIC_API_KEY
    max_tokens 4096
  }
}

client<llm> GPT {
  provider openai
  retry_policy Backoff
  options {
    model "gpt-5.6-terra"
    api_key env.OPENAI_API_KEY
    max_tokens 4096
  }
}

// Labelling runs in batches of twenty transcripts, so one overloaded-provider response
// mid-run would otherwise waste every batch already paid for. Retries are few and short:
// the caller is a person watching a progress bar, not a queue.
retry_policy Backoff {
  max_retries 3
  strategy {
    type exponential_backoff
    delay_ms 500
    multiplier 2
    max_delay_ms 8000
  }
}
