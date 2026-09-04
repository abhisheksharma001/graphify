from typer.testing import CliRunner

from graphify_brain import __version__
from graphify_brain.cli import app
from graphify_brain.cost import PRICES, PRICES_CHECKED


def test_version_prints_name_and_version():
    result = CliRunner().invoke(app, ["version"])
    assert result.exit_code == 0
    assert result.output.strip() == f"graphify-brain {__version__}"


def test_models_prints_every_priced_client_without_touching_the_network():
    result = CliRunner().invoke(app, ["models"])

    assert result.exit_code == 0
    for client, rate in PRICES.items():
        assert client in result.output
        assert rate.model in result.output
    assert PRICES_CHECKED in result.output


def test_models_check_with_no_keys_names_the_variables_and_claims_nothing(monkeypatch):
    """No key is not a pass. The command must not print a client as current when it
    never asked anyone about it."""
    monkeypatch.delenv("ANTHROPIC_API_KEY", raising=False)
    monkeypatch.delenv("OPENAI_API_KEY", raising=False)

    result = CliRunner().invoke(app, ["models", "--check"])

    assert result.exit_code == 0
    assert "no ANTHROPIC_API_KEY" in result.output
    assert "no OPENAI_API_KEY" in result.output
    assert "current:" not in result.output
