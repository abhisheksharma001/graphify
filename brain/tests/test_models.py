"""Comparing the price table against what a provider says it has.

`compare` is pure, and every case below hands it a listing literal. Nothing here makes a
request: `fetch` is the only function that touches the network and it is not exercised
here, because the spec forbids a test that does.
"""

from datetime import datetime, timezone

from graphify_brain.cost import Price
from graphify_brain.models import Listing, compare


def at(day: str) -> datetime:
    return datetime.fromisoformat(f"{day}T00:00:00+00:00")


PRICED = {
    "sonnet": Price("anthropic", "claude-sonnet-5", 2.00, 10.00),
    "gpt": Price("openai", "gpt-5.6-terra", 2.00, 12.00),
}

ANTHROPIC = [Listing("claude-sonnet-5", at("2026-07-24"))]
OPENAI = [Listing("gpt-5.6-terra", at("2026-08-01"))]


def test_current_models_report_nothing():
    report = compare(PRICED, {"anthropic": ANTHROPIC, "openai": OPENAI})

    assert report.ok
    assert report.missing == []
    assert report.newer == {}
    assert report.unchecked == []


def test_a_retired_model_is_the_failing_case():
    report = compare(PRICED, {"anthropic": [], "openai": OPENAI})

    assert not report.ok
    assert [rate.model for rate in report.missing] == ["claude-sonnet-5"]


def test_a_later_release_is_news_and_not_a_failure():
    """A newer model is worth a look and never automatic: its price is the part no
    provider's API will tell us."""
    listings = {
        "anthropic": [*ANTHROPIC, Listing("claude-sonnet-6", at("2026-11-01"))],
        "openai": OPENAI,
    }

    report = compare(PRICED, listings)

    assert report.ok
    assert report.newer == {"sonnet": ["claude-sonnet-6"]}


def test_an_older_model_is_not_newer():
    listings = {
        "anthropic": [*ANTHROPIC, Listing("claude-sonnet-4-6", at("2026-01-01"))],
        "openai": OPENAI,
    }

    assert compare(PRICED, listings).newer == {}


def test_a_model_released_the_same_day_is_not_newer_than_itself():
    listings = {
        "anthropic": [*ANTHROPIC, Listing("claude-opus-5", at("2026-07-24"))],
        "openai": OPENAI,
    }

    assert compare(PRICED, listings).newer == {}


def test_a_provider_with_no_key_is_named_rather_than_passed():
    """Silence from an unchecked provider is not an all-clear."""
    report = compare(PRICED, {"anthropic": ANTHROPIC})

    assert report.unchecked == ["openai"]
    # Not a failure — nothing is known to be broken, only unverified.
    assert report.ok
    assert report.missing == []
