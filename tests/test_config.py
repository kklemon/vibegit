from vibegit.config import ModelConfig


def test_model_config_openai_compatible(monkeypatch):
    captured_kwargs = {}

    def fake_init_chat_model(**kwargs):
        captured_kwargs.update(kwargs)
        return "chat_model_instance"

    monkeypatch.setattr("vibegit.config.init_chat_model", fake_init_chat_model)

    config = ModelConfig(
        name="my-openai-model",
        base_url="https://api.example.com/v1",
        api_key="secret-key",
        model_provider="openai",
        temperature=0.25,
    )

    result = config.get_chat_model()

    assert result == "chat_model_instance"
    assert captured_kwargs["model"] == "my-openai-model"
    assert captured_kwargs["model_provider"] == "openai"
    assert captured_kwargs["base_url"] == "https://api.example.com/v1"
    assert captured_kwargs["api_key"] == "secret-key"
    assert captured_kwargs["temperature"] == 0.25


def test_model_config_default_provider(monkeypatch):
    captured_kwargs = {}

    def fake_init_chat_model(**kwargs):
        captured_kwargs.update(kwargs)
        return "chat_model_instance"

    monkeypatch.setattr("vibegit.config.init_chat_model", fake_init_chat_model)

    config = ModelConfig(name="google_genai:gemini-2.5-flash")

    config.get_chat_model()

    assert captured_kwargs["model"] == "google_genai:gemini-2.5-flash"
    assert "model_provider" not in captured_kwargs
    assert "base_url" not in captured_kwargs
    assert "api_key" not in captured_kwargs
