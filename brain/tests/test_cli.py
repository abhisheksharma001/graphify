from typer.testing import CliRunner

from graphify_brain import __version__
from graphify_brain.cli import app


def test_version_prints_name_and_version():
    result = CliRunner().invoke(app, ["version"])
    assert result.exit_code == 0
    assert result.output.strip() == f"graphify-brain {__version__}"
