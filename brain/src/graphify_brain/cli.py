"""Command-line entry point. `graphify-brain --help` lists commands."""

import typer

from graphify_brain import __version__, cost

app = typer.Typer(help="graphify-brain — plan, clarify, label, synthesize, ask.", no_args_is_help=True)


@app.callback()
def main() -> None:
    """graphify-brain — the LLM side of graphify."""


@app.command()
def version() -> None:
    """Print the installed graphify-brain version."""
    typer.echo(f"graphify-brain {__version__}")


@app.command()
def models(
    check: bool = typer.Option(
        False,
        "--check",
        help="Ask each provider whether these models still exist, and what is newer. "
        "Needs ANTHROPIC_API_KEY / OPENAI_API_KEY; makes no model call and costs nothing.",
    ),
) -> None:
    """Show the price table, how old it is, and optionally what the providers now have."""
    age = cost.checked_days_ago()
    typer.echo(f"prices read {cost.PRICES_CHECKED} ({age} days ago)")
    for client, rate in cost.PRICES.items():
        typer.echo(
            f"  {client:<7} {rate.model:<20} {rate.provider:<10} "
            f"${rate.usd_in:g} in / ${rate.usd_out:g} out per MTok"
        )
    if cost.is_stale():
        # A warning, not a failure. Prices are read by a person off a web page, and a
        # command that starts exiting non-zero on a calendar date with no change to the
        # code teaches everyone to pass --no-verify.
        typer.echo(
            f"\nstale: older than {cost.STALE_AFTER_DAYS} days. Re-read the pricing "
            "pages named in graphify_brain/cost.py and update PRICES_CHECKED.",
            err=True,
        )

    if not check:
        return

    # Imported here so `graphify-brain models` with no flag stays a pure local print and
    # never needs httpx to be importable.
    from graphify_brain import models as m

    report = m.check()
    typer.echo("")
    for provider in report.unchecked:
        typer.echo(f"not checked: no {m.KEY_VARS[provider]} in the environment", err=True)
    for rate in report.missing:
        typer.echo(f"GONE: {rate.provider} no longer lists {rate.model}", err=True)
    for client, later in report.newer.items():
        typer.echo(f"newer than {client}: {', '.join(later)}")
    checked = [c for c, r in cost.PRICES.items() if r.provider not in report.unchecked]
    if checked and report.ok and not report.newer:
        # Only about the clients actually checked: with no key at all, nothing was
        # verified and saying otherwise would be the wrong kind of green.
        typer.echo(f"current: {', '.join(checked)}.")
    if not report.ok:
        # The only failing case. A newer model is news; a retired one is broken.
        raise typer.Exit(1)
