"""Command-line entry point. `graphify --help` lists commands."""

import typer

from graphify import __version__

app = typer.Typer(help="Open-source Vapi call analytics.", no_args_is_help=True)


@app.callback()
def main() -> None:
    """graphify — see how your Vapi agent is really doing."""


@app.command()
def version() -> None:
    """Print the installed graphify version."""
    typer.echo(f"graphify {__version__}")
