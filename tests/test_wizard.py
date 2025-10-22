from vibegit.config import Config, ContextFormattingConfig, ModelConfig
from vibegit.wizard import ConfigWizard


def build_config(model: ModelConfig) -> Config:
    return Config.model_construct(
        model=model,
        context_formatting=ContextFormattingConfig(),
        api_keys={},
        allow_excluding_changes=True,
        watermark=True,
    )


def build_wizard(config: Config) -> ConfigWizard:
    wizard = ConfigWizard.__new__(ConfigWizard)
    wizard.config = config
    return wizard


def test_openai_wizard_prefills_existing_and_keeps_api_key(monkeypatch):
    config = build_config(
        ModelConfig.model_construct(
            name="existing-model",
            base_url="https://api.example.com/v1",
            api_key="secret-key",
            model_provider="openai",
        )
    )

    wizard = build_wizard(config)

    responses = iter(
        [
            {},  # base_url and model_name prompt (user keeps defaults)
            {"keep_api_key": True},  # confirm reuse existing key
        ]
    )

    def fake_prompt(_questions):
        return next(responses, {})

    monkeypatch.setattr("vibegit.wizard.inquirer.prompt", fake_prompt)

    result = wizard._get_openai_compatible_model()

    assert result["base_url"] == "https://api.example.com/v1"
    assert result["model_name"] == "existing-model"
    assert result["api_key"] == "secret-key"


def test_openai_wizard_updates_api_key(monkeypatch):
    config = build_config(
        ModelConfig.model_construct(
            name="existing-model",
            base_url="https://api.example.com/v1",
            api_key="secret-key",
            model_provider="openai",
        )
    )

    wizard = build_wizard(config)

    responses = iter(
        [
            {
                "base_url": "https://new.example.com/v1",
                "model_name": "new-model",
            },
            {"keep_api_key": False},
            {"api_key": "new-secret"},
        ]
    )

    def fake_prompt(_questions):
        return next(responses, {})

    monkeypatch.setattr("vibegit.wizard.inquirer.prompt", fake_prompt)

    result = wizard._get_openai_compatible_model()

    assert result["base_url"] == "https://new.example.com/v1"
    assert result["model_name"] == "new-model"
    assert result["api_key"] == "new-secret"
