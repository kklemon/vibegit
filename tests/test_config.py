from vibegit.config import ModelConfig


def test_model_config_openai_compatible(monkeypatch):
    captured_config = {}

    def fake_resolve_model(config):
        captured_config["config"] = config
        return "model_instance", "model_settings"

    monkeypatch.setattr("vibegit.config.resolve_model", fake_resolve_model)

    config = ModelConfig(
        name="my-openai-model",
        base_url="https://api.example.com/v1",
        api_key="secret-key",
        model_provider="openai",
        temperature=0.25,
    )

    result = config.get_model()

    assert result == ("model_instance", "model_settings")
    assert captured_config["config"] is config


def test_model_config_default_provider(monkeypatch):
    captured_config = {}

    def fake_resolve_model(config):
        captured_config["config"] = config
        return "model_instance", None

    monkeypatch.setattr("vibegit.config.resolve_model", fake_resolve_model)

    config = ModelConfig(name="google_genai:gemini-2.5-flash")

    result = config.get_model()

    assert result == ("model_instance", None)
    assert captured_config["config"] is config
