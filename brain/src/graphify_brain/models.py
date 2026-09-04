"""Are the models we price still the models the providers have — and is there a newer one?

This is as far as "auto-update" honestly goes. Both providers publish a models list
(`GET /v1/models`), and it answers two questions worth asking:

* has a model we are configured to call been retired, and
* has the provider shipped something newer than what we point at.

Neither list carries a **price**. There is no pricing API at either provider; the rates in
`graphify_brain.cost` were read off a web page by a person and can only be updated the
same way. So this module tells you *when* to go and look, and what changed — it cannot
fetch the number for you, and pretending otherwise would put an invented price under a
spend cap.

The network half and the comparing half are separate on purpose: `compare` is pure, so
the tests exercise the logic without a request, which the spec requires.
"""

from __future__ import annotations

import os
from dataclasses import dataclass, field
from datetime import datetime, timezone

import httpx

from graphify_brain.cost import PRICES, Price

#: Where each provider lists its models, and how each one wants to be told who is asking.
#: A key goes in a header and nowhere else — never a query string, which lands in logs.
ENDPOINTS: dict[str, str] = {
    "anthropic": "https://api.anthropic.com/v1/models?limit=1000",
    "openai": "https://api.openai.com/v1/models",
}

#: The environment variable each provider's key arrives in — the same names the engine
#: exports and `baml_src/clients.baml` reads.
KEY_VARS: dict[str, str] = {
    "anthropic": "ANTHROPIC_API_KEY",
    "openai": "OPENAI_API_KEY",
}


@dataclass(frozen=True)
class Listing:
    """One model as a provider reports it: an id and when it was released."""

    id: str
    released: datetime


@dataclass
class Report:
    """What changed under us."""

    missing: list[Price] = field(default_factory=list)
    """Models we are configured to call that the provider no longer lists. This is the
    one that breaks the brain, so it is the one that sets the exit code."""

    newer: dict[str, list[str]] = field(default_factory=dict)
    """Client name -> models the provider released after the one we point at. Worth a
    look, never automatic: a newer model has a different price, and the price is the part
    no API will tell us."""

    unchecked: list[str] = field(default_factory=list)
    """Providers with no key in the environment. Silence from one of these is not an
    all-clear, so it is reported rather than counted as a pass."""

    @property
    def ok(self) -> bool:
        return not self.missing


def compare(
    priced: dict[str, Price],
    listings: dict[str, list[Listing]],
) -> Report:
    """Match the price table against what each provider says it has.

    A provider absent from `listings` was not checked — which is different from a
    provider that answered and did not mention our model.
    """
    report = Report()
    for client, rate in priced.items():
        available = listings.get(rate.provider)
        if available is None:
            if rate.provider not in report.unchecked:
                report.unchecked.append(rate.provider)
            continue

        ours = next((m for m in available if m.id == rate.model), None)
        if ours is None:
            report.missing.append(rate)
            continue

        # Strictly after, so the model we already use is not reported as newer than
        # itself, and neither is anything released the same day.
        later = sorted(m.id for m in available if m.released > ours.released)
        if later:
            report.newer[client] = later
    return report


def fetch(provider: str, key: str, timeout: float = 20.0) -> list[Listing]:
    """The provider's model list. One request; both endpoints return everything at the
    sizes involved here.

    Raises `httpx.HTTPStatusError` on a bad status. The message is the status code only:
    a provider's error body can echo back the request that caused it, and nothing in this
    process puts a key into a string that something might print.
    """
    headers = (
        {"x-api-key": key, "anthropic-version": "2023-06-01"}
        if provider == "anthropic"
        else {"authorization": f"Bearer {key}"}
    )
    res = httpx.get(ENDPOINTS[provider], headers=headers, timeout=timeout)
    if res.is_error:
        raise httpx.HTTPStatusError(
            f"{provider} model list returned {res.status_code}",
            request=res.request,
            response=res,
        )
    return [_listing(provider, row) for row in res.json()["data"]]


def _listing(provider: str, row: dict) -> Listing:
    """The two providers spell the release date differently: Anthropic sends RFC 3339 in
    `created_at`, OpenAI a unix timestamp in `created`."""
    if provider == "anthropic":
        released = datetime.fromisoformat(row["created_at"])
    else:
        released = datetime.fromtimestamp(row["created"], tz=timezone.utc)
    return Listing(id=row["id"], released=released)


def check(priced: dict[str, Price] | None = None) -> Report:
    """Ask every provider we hold a key for, and compare.

    A provider with no key is skipped and named in `Report.unchecked` rather than
    silently passing.
    """
    rates = PRICES if priced is None else priced
    listings: dict[str, list[Listing]] = {}
    for provider in {rate.provider for rate in rates.values()}:
        key = os.environ.get(KEY_VARS[provider], "").strip()
        if key:
            listings[provider] = fetch(provider, key)
    return compare(rates, listings)
