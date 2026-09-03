"""Command-line entry point. `graphify-brain --help` lists commands."""

import typer

from graphify_brain import __version__

app = typer.Typer(help="graphify-brain — plan, clarify, label, synthesize, ask.", no_args_is_help=True)


@app.callback()
def main() -> None:
    """graphify-brain — the LLM side of graphify."""


@app.command()
def version() -> None:
    """Print the installed graphify-brain version."""
    typer.echo(f"graphify-brain {__version__}")
