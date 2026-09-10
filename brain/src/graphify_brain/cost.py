"""What a model call costs, before it is made.

Nothing here talks to a provider. The spec forbids calling a model without showing the
price first and getting an explicit go, and it caps what the daily modes may spend, so
both of those need an answer that exists *before* any request: a token count times a
published rate.

The estimate is deliberately the ceiling. It prices every input token at the base rate,
ignoring the prompt-caching discount, so a real call can come in under the number a
person approved but never over it. A cap built on an under-estimate is not a cap.

Prices are data, and data goes stale. They are written out below with the day they were
read and the pages they were read from; when a provider moves a price, this table is the
one place to change.

`booked` and `spent` at the foot of the file are the two exceptions to "before it is
made", and they are here rather than beside their callers for the same reason as the
table: they are the same arithmetic read backwards, off a provider's own token counts
instead of an estimate of them, and a second copy of it is a second answer to what a call
cost. Neither talks to a provider either.
"""

from __future__ import annotations

from dataclasses import dataclass
from datetime import date
from typing import Any

#: The day the prices below were read from the vendors' own pricing pages:
#: https://platform.claude.com/docs/en/about-claude/pricing and
#: https://developers.openai.com/api/docs/pricing
PRICES_CHECKED = "2026-09-04"

#: How long a reading stays trustworthy. Nothing enforces this — no test fails on a
#: calendar date, because a build that breaks with no change to the code is worse than a
#: stale price. `graphify-brain models` says so out loud instead.
STALE_AFTER_DAYS = 90

#: A million. Rates are published per million tokens, and the arithmetic reads better
#: with the unit named than with 1e6 sitting in the middle of it.
PER = 1_000_000


@dataclass(frozen=True)
class Price:
    """One model's published rate, in USD per million tokens."""

    provider: str
    """`anthropic` or `openai` — the same word `baml_src/clients.baml` uses. It says
    whose model list `graphify_brain.models` should look this id up in."""

    model: str
    """The exact API model id — the string `baml_src/clients.baml` sends."""

    usd_in: float
    """Base input tokens. Cache writes cost more and cache reads much less; neither is
    used here, for the reason in the module docstring."""

    usd_out: float
    """Output tokens."""


#: Keyed by the client name in `baml_src/clients.baml`, because that is what the rest of
#: the brain has in hand: a job records which client it ran on, not which model id that
#: client happened to be pointed at.
PRICES: dict[str, Price] = {
    "opus": Price("anthropic", "claude-opus-5", 5.00, 25.00),
    "sonnet": Price("anthropic", "claude-sonnet-5", 2.00, 10.00),
    # The middle of OpenAI's 5.6 line, and deliberately Sonnet's opposite number: same
    # tier, near enough the same rate ($2/$12 against $2/$10). `gpt-5.6-sol` is the big
    # one at $4/$20 and `gpt-5.6-luna` the small one at $0.20/$1.20.
    "gpt": Price("openai", "gpt-5.6-terra", 2.00, 12.00),
}

#: Client name *and* model id both resolve, so a `patterns.model` row that stored the id
#: rather than the nickname still prices. Built from `PRICES`, so the two spellings can
#: never come to disagree about the rate.
_BY_NAME: dict[str, Price] = {
    **PRICES,
    **{p.model: p for p in PRICES.values()},
}


#: The model nickname a request names, and the BAML client in `baml_src/clients.baml`
#: that nickname selects. Keyed by `PRICES`'s keys, and living beside them for exactly
#: that reason: a model that may be asked for is a model whose spend can be counted, and
#: an unpriced model is refused rather than run against a total that never grows.
CLIENTS = {"opus": "Opus", "sonnet": "Sonnet", "gpt": "GPT"}


def model_name(value: Any, name: str) -> str:
    """The client a request asked for, lowercased — or a refusal that names the command.

    `name` is the command doing the asking, so that `ask`'s refusal does not say `label`.

    It lives here rather than in `label.py`, where it was written, because `plan.py` needs
    it too and cannot import from `label.py`: `label` imports `envelope` from `plan`, so
    that direction closes a cycle. This module imports nothing from the package and
    already holds the list being checked against, which makes it the one place all four
    callers can reach.
    """
    known = ", ".join(sorted(CLIENTS))
    if not isinstance(value, str) or value.strip().lower() not in CLIENTS:
        raise ValueError(f"{name}: model must be one of {known}, not {value!r}")
    return value.strip().lower()


def price(model: str) -> Price:
    """The rate for a client name (`"sonnet"`) or a model id (`"claude-sonnet-5"`).

    Raises `KeyError` for anything else. A model with no published price is not a model
    that costs nothing — refusing is what keeps an unpriced model out of a capped spend
    instead of letting it run against a total that never grows.
    """
    try:
        return _BY_NAME[model.strip().lower()]
    except KeyError:
        known = ", ".join(sorted(PRICES))
        raise KeyError(
            f"no price for model {model!r}; priced clients are {known}"
        ) from None


def estimate(tokens_in: int, tokens_out: int, model: str) -> float:
    """USD for a call of this shape, unrounded.

    Unrounded on purpose: the caller decides how to show it, and a daily cap sums these
    hundreds of times before it compares the total to anything.
    """
    if tokens_in < 0 or tokens_out < 0:
        raise ValueError(f"token counts cannot be negative: {tokens_in=}, {tokens_out=}")
    rate = price(model)
    return (tokens_in * rate.usd_in + tokens_out * rate.usd_out) / PER


def booked(usage: Any, model: str, ceiling: float) -> float:
    """What to book for a call that has already been made.

    The provider's own numbers when it gave them, and the `ceiling` when it did not — the
    figure this call site quoted before it sent, showed the analyst, and checked against
    the cap. Never zero, and that is the whole of this function.

    Zero is not a spare value. It is the right booking for a `daily` run in free mode, for
    a labelling run the cap stopped before it sent anything, and for a pattern recounted by
    rule alone, and `engine/src/db.rs` writes no `spend` row for a job that cost zero. So
    "this was free" and "nobody knows what this cost" would be the same number, and the
    second one is a model call the day's ledger has no record of — which is the second
    Must-never, since `sync.rs` computes the day's remaining budget by subtracting that
    ledger from the cap.

    `Usage.input_tokens` and `Usage.output_tokens` are `Optional[int]` in BAML's own
    signature: whether usage comes back is the provider's to decide, and neither endpoint
    this brain is pinned to withholds it today. What is not the provider's to decide is
    what graphify writes down when it does.

    All or nothing on the two counts. A real input count beside a ceiling output is a third
    number that is neither what was billed nor what was quoted, and a cap that is
    over-booked is wrong in the only direction a cap survives being wrong in.
    """
    tokens_in = usage.input_tokens
    tokens_out = usage.output_tokens
    if tokens_in is None or tokens_out is None:
        return ceiling
    return estimate(tokens_in, tokens_out, model)


def checked_days_ago(today: date | None = None) -> int:
    """How old the price table is, in days. `today` is injectable so a test need not
    depend on the calendar."""
    then = date.fromisoformat(PRICES_CHECKED)
    return ((today or date.today()) - then).days


def is_stale(today: date | None = None) -> bool:
    return checked_days_ago(today) > STALE_AFTER_DAYS


#: The BAML client each price is reached by, inverted from `CLIENTS`. A collected call
#: names the client it went out on — `"Opus"`, not `"opus"` — and that is the only handle
#: on the down path to which rate card it should be priced against. Built by inverting
#: rather than written out, so a rename in `CLIENTS` cannot leave a call unpriceable.
_BY_CLIENT: dict[str, str] = {client: name for name, client in CLIENTS.items()}

#: Every model call the process makes, in one place, for the length of the process.
#:
#: Each call site builds its own `Collector` and reports `usd` from it, and that is
#: untouched: this one is passed *alongside* it and is read in exactly one situation —
#: the process is on its way down and nothing else will ever say what it spent.
#:
#: A collector holds the log of a call that raised, which is the whole reason a job that
#: died after being billed can be booked at all. It also sums across logs, so a labelling
#: run that raises in its fifth batch still knows about the four the provider charged for
#: and that `label.run` was about to discard.
#:
#: Built on first use rather than at import, because `baml_py` is a compiled extension and
#: `graphify-brain version` should not need it loaded to print a string.
_LEDGER: Any = None


def ledger() -> Any:
    """The process-wide collector. Every model call passes it; only `cli` reads it."""
    global _LEDGER
    if _LEDGER is None:
        from baml_py import Collector

        _LEDGER = Collector()
    return _LEDGER


def spent(collector: Any = None) -> float | None:
    """What this process has been billed so far, or `None` if that cannot be worked out.

    `collector` is injectable so a test can hand in one it built, the way
    `checked_days_ago` takes a `today`. Left out, it is the process ledger above.

    Priced one call at a time rather than once over the total, because a `daily` run
    labels several patterns in a single process and each carries its own `patterns.model`.
    Tokens from two rate cards added together are not money at either rate.

    A call whose usage is absent contributes nothing. That is not the `or 0` S-56 removed:
    the calls that come back without usage here are the 5xx responses `retry_policy
    Backoff` retried, and a provider does not bill for those. What is absent is the charge,
    not the knowledge of it.

    A client name that does not resolve to a row in `PRICES` gives up on the whole total
    rather than returning part of one. A partial figure booked as if it were complete is
    the same defect this function exists to fix, one layer further in.
    """
    collector = _LEDGER if collector is None else collector
    if collector is None:
        # Nothing built the ledger, so no model call was made and nothing was billed. Not
        # the same as the `None` this returns below: that one is a total it cannot finish.
        return 0.0
    total = 0.0
    for log in collector.logs:
        for call in log.calls:
            tokens_in = call.usage.input_tokens
            tokens_out = call.usage.output_tokens
            if tokens_in is None or tokens_out is None:
                continue
            name = _BY_CLIENT.get(call.client_name)
            if name is None:
                return None
            total += estimate(tokens_in, tokens_out, name)
    return total
