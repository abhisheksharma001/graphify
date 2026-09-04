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
"""

from __future__ import annotations

from dataclasses import dataclass
from datetime import date

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


def checked_days_ago(today: date | None = None) -> int:
    """How old the price table is, in days. `today` is injectable so a test need not
    depend on the calendar."""
    then = date.fromisoformat(PRICES_CHECKED)
    return ((today or date.today()) - then).days


def is_stale(today: date | None = None) -> bool:
    return checked_days_ago(today) > STALE_AFTER_DAYS
