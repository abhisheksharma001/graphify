from typer.testing import CliRunner

from graphify import __version__
from graphify.cli import app


def test_version_prints_name_and_version():
    result = CliRunner().invoke(app, ["version"])
    assert result.exit_code == 0
    assert result.output.strip() == f"graphify {__version__}"
