"""Command-line entry point. `graphify-brain --help` lists commands."""

import sys
from contextlib import closing
from pathlib import Path
from typing import Any, Callable, Optional

import typer

from graphify_brain import __version__, cost
from graphify_brain import label as labelling
from graphify_brain import plan as planning
from graphify_brain import synth as synthesis

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


#: The engine spawns every brain function the same way — `graphify-brain <fn> --db PATH`
#: — so both commands below take `--db` even though neither reads a row. It is opened and
#: closed, not ignored: a wrong path is then an error at the first step of the wizard,
#: instead of surviving `plan` and `clarify` and failing at `label`, after the spend.
DB = typer.Option(None, "--db", help="The engine's SQLite file. Checked, not read.")


@app.command()
def plan(db: Optional[Path] = DB) -> None:
    """Turn one plain-English criterion into a plan. Reads JSON on stdin, writes JSON out.

    Input `{criterion, system_prompt?}`; output the whole `Plan`, printed as it came back.
    """
    _pipe(planning.plan, db)


@app.command()
def clarify(db: Optional[Path] = DB) -> None:
    """Revise a plan with the analyst's answers. Reads JSON on stdin, writes JSON out.

    Input `{criterion, plan, answers: [{question, answer}]}`; output the whole `Plan`.
    """
    _pipe(planning.clarify, db)


#: `label` is the first command that reads a row, so its `--db` is required rather than
#: merely checked: the transcripts it shows a model come out of the engine's `calls` table
#: and there is no other place to get them.
LABEL_DB = typer.Option(..., "--db", help="The engine's SQLite file. Read for transcripts.")


@app.command()
def label(
    db: Path = LABEL_DB,
    yes: bool = typer.Option(False, "--yes", help="Spend without waiting for GO on stdin."),
) -> None:
    """Read calls and label them against a plan. Costs money; prints the price first.

    Input, on one line of stdin: `{criterion, plan, call_ids, model, max_usd,
    batch_size?, pattern_id?}`. Then `ESTIMATE {usd}` on stdout, and — unless `--yes` —
    `GO` on stdin before a single call is read. Output: the labels, what was not labelled
    and why, and what it actually cost.
    """
    from graphify_brain import db as database

    try:
        with closing(database.read_write(db)) as conn:
            labelling.run(sys.stdin, sys.stdout, sys.stderr, conn, yes)
    except (ValueError, FileNotFoundError) as e:
        typer.echo(str(e), err=True)
        raise typer.Exit(1) from e


@app.command()
def synthesize(db: Path = LABEL_DB) -> None:
    """Turn labels into a rule the engine can run, and store the pattern.

    Input on stdin: `{criterion, plan, labels, model, max_usd, org_id, name,
    assistant_ids?}`. Then `ESTIMATE {usd}` on stdout and the result: the rule, the chart
    to draw it with, how much of the sample the rule agrees with, and what it cost. No
    `GO` — see `synth.run`.
    """
    from graphify_brain import db as database

    try:
        with closing(database.read_write(db)) as conn:
            synthesis.run(sys.stdin, sys.stdout, sys.stderr, conn)
    except (ValueError, FileNotFoundError) as e:
        typer.echo(str(e), err=True)
        raise typer.Exit(1) from e


@app.command()
def daily(db: Path = LABEL_DB) -> None:
    """Read the day's new calls for every hybrid and full pattern, inside two caps.

    Input on stdin: `{org, max_usd}`, where `max_usd` is what is left of the org's day —
    the engine works that out from the global cap and the `spend` table. Output: what each
    pattern read, what it matched, and what the whole run cost.

    No `--yes` and no `GO`. D-8 replaces the click with a cap for the daily modes, so the
    flag that would let one run without a cap does not exist.
    """
    from graphify_brain import daily as dailies
    from graphify_brain import db as database

    try:
        with closing(database.read_write(db)) as conn:
            dailies.run(sys.stdin, sys.stdout, sys.stderr, conn)
    except (ValueError, FileNotFoundError) as e:
        typer.echo(str(e), err=True)
        raise typer.Exit(1) from e


def _pipe(fn: Callable[[dict[str, Any]], dict[str, Any]], db: Optional[Path]) -> None:
    """The engine ↔ brain contract: JSON in, JSON out, exit 0 or 1, complaints on stderr.

    Two exceptions are caught, and they are the two the caller can fix: a bad input
    (`ValueError`) and a `--db` that is not there. Anything else — no key, a provider
    that is down, a model that answered with something unparseable — is this program
    failing, and its traceback on stderr is worth more to whoever reads `jobs.log` than
    one tidy line would be.
    """
    try:
        if db is not None:
            from graphify_brain import db as database

            database.read_only(db).close()
        typer.echo(planning.run(fn, sys.stdin.read()))
    except (ValueError, FileNotFoundError) as e:
        typer.echo(str(e), err=True)
        raise typer.Exit(1) from e
