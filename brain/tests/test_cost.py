"""The price table, and the two ways it can be wrong.

It can hold a number that is not the vendor's, which no test can catch — the prices carry
the day they were read so a person can check them. And it can fall out of step with
`baml_src/clients.baml`, which a test can catch, and does.

Nothing here touches the network.
"""

import re
from datetime import date, timedelta
from pathlib import Path

import pytest

from graphify_brain.cost import (
    PRICES,
    PRICES_CHECKED,
    STALE_AFTER_DAYS,
    checked_days_ago,
    estimate,
    is_stale,
    price,
)

CLIENTS = Path(__file__).resolve().parents[1] / "baml_src" / "clients.baml"


def test_estimate_matches_the_table():
    """The acceptance case: 100k in, 2k out on Sonnet, priced from the table itself."""
    rate = PRICES["sonnet"]
    expected = (100_000 * rate.usd_in + 2_000 * rate.usd_out) / 1_000_000

    got = estimate(100_000, 2_000, "sonnet")

    assert got > 0
    assert got == pytest.approx(expected)
    # Spelled out, so a swapped input/output rate would show up here and not only in the
    # arithmetic above, which would agree with itself either way.
    assert got == pytest.approx(0.22)


def test_a_model_id_prices_the_same_as_its_client_name():
    assert price("claude-sonnet-5") is price("sonnet")


def test_case_and_padding_do_not_change_the_price():
    assert price("  Sonnet ") is price("sonnet")


def test_no_tokens_costs_nothing():
    assert estimate(0, 0, "opus") == 0.0


def test_an_unpriced_model_is_refused_rather_than_free():
    with pytest.raises(KeyError, match="no price for model"):
        estimate(10, 10, "gemini")


def test_negative_tokens_are_refused():
    with pytest.raises(ValueError):
        estimate(-1, 0, "opus")


def test_output_costs_more_than_input_everywhere():
    """True of every provider's pricing, and the shape a swapped pair would break."""
    for name, rate in PRICES.items():
        assert rate.usd_out > rate.usd_in, name


def test_every_client_in_clients_baml_is_priced_under_the_right_provider():
    """The drift guard.

    `clients.baml` is the file that says which model actually gets called and whose API
    it is called on. A model that can be called but not priced is a model whose spend the
    daily cap cannot count, which is the one failure this module exists to prevent; a
    model priced under the wrong provider is one `graphify-brain models --check` would go
    looking for in the wrong list; and a client whose name does not match a `PRICES` key
    is one no job could ever price by the name it records.

    Whole `client<llm> ... {}` blocks are matched, not loose `provider` and `model` lines,
    so a comment that happens to contain either word cannot join two blocks together.
    """
    blocks = re.findall(
        r"client<llm>\s+(\w+)\s*\{(.*?)\n\}", CLIENTS.read_text(), re.DOTALL
    )
    assert len(blocks) == len(PRICES), f"found {len(blocks)} client blocks in {CLIENTS}"

    declared = {
        name.lower(): (
            re.search(r"^\s*provider\s+(\S+)", body, re.MULTILINE).group(1),
            re.search(r'^\s*model\s+"([^"]+)"', body, re.MULTILINE).group(1),
        )
        for name, body in blocks
    }

    assert declared == {
        client: (rate.provider, rate.model) for client, rate in PRICES.items()
    }


def test_the_price_table_knows_how_old_it_is():
    assert checked_days_ago(date.fromisoformat(PRICES_CHECKED)) == 0
    assert checked_days_ago(date.fromisoformat(PRICES_CHECKED) + timedelta(days=7)) == 7


def test_staleness_turns_over_at_the_threshold():
    read = date.fromisoformat(PRICES_CHECKED)
    assert not is_stale(read + timedelta(days=STALE_AFTER_DAYS))
    assert is_stale(read + timedelta(days=STALE_AFTER_DAYS + 1))


def test_the_prices_say_when_they_were_read():
    assert re.fullmatch(r"\d{4}-\d{2}-\d{2}", PRICES_CHECKED)
