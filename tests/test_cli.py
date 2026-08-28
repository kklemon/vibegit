from pathlib import Path

from click.testing import CliRunner

import vibegit.cli as cli_module


def test_init_creates_repository_rules_file(monkeypatch, tmp_path):
    runner = CliRunner()
    monkeypatch.chdir(tmp_path)
    monkeypatch.setattr(
        cli_module,
        "get_config",
        lambda: (_ for _ in ()).throw(AssertionError("config should not be loaded")),
    )

    result = runner.invoke(cli_module.cli, ["init"])

    assert result.exit_code == 0
    assert Path(".vibegitrules").read_text(encoding="utf-8") == (
        cli_module.get_default_rules()
    )


def test_init_preserves_existing_rules_unless_forced(monkeypatch, tmp_path):
    runner = CliRunner()
    monkeypatch.chdir(tmp_path)
    rules_path = Path(".vibegitrules")
    rules_path.write_text("custom rules\n", encoding="utf-8")

    result = runner.invoke(cli_module.cli, ["init"])

    assert result.exit_code == 1
    assert "already exists" in result.output
    assert rules_path.read_text(encoding="utf-8") == "custom rules\n"

    result = runner.invoke(cli_module.cli, ["init", "--force"])

    assert result.exit_code == 0
    assert rules_path.read_text(encoding="utf-8") == (cli_module.get_default_rules())


def test_packaged_rules_match_repository_rules():
    repository_rules = Path(__file__).parents[1] / ".vibegitrules"

    assert cli_module.get_default_rules() == repository_rules.read_text(
        encoding="utf-8"
    )
