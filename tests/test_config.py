import pytest

from vibegit.config import ModelConfig
from vibegit.llm import _build_openai_model, resolve_model


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


def test_default_model_is_current_recommended_gemini():
    assert ModelConfig().name == "google:gemini-3.7-flash"


@pytest.mark.parametrize("legacy_provider", ["google-gla", "google_genai"])
def test_legacy_google_provider_is_normalized(legacy_provider):
    model, model_settings = resolve_model(
        ModelConfig(name=f"{legacy_provider}:gemini-3.7-flash")
    )

    assert model == "google:gemini-3.7-flash"
    assert model_settings is None


def test_openai_compatible_model_can_be_constructed():
    model = _build_openai_model(
        "custom-model", "https://api.example.com/v1", "secret-key"
    )

    assert model.model_name == "custom-model"
    assert model.base_url == "https://api.example.com/v1/"
