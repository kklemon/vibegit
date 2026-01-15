from __future__ import annotations

import os
from typing import Any, TYPE_CHECKING

if TYPE_CHECKING:
    from vibegit.config import ModelConfig


LEGACY_PROVIDER_MAP: dict[str, str] = {
    "google_genai": "google-gla",
    "xai": "grok",
}


def _split_model_name(name: str, model_provider: str | None) -> tuple[str | None, str]:
    if ":" in name:
        provider, model = name.split(":", 1)
        return provider, model
    if model_provider:
        return model_provider, name
    return None, name


def _normalize_provider(provider: str | None) -> str | None:
    if provider is None:
        return None
    return LEGACY_PROVIDER_MAP.get(provider, provider)


def _mirror_xai_key() -> None:
    if "GROK_API_KEY" not in os.environ and os.environ.get("XAI_API_KEY"):
        os.environ["GROK_API_KEY"] = os.environ["XAI_API_KEY"]


def _build_openai_model(
    model_name: str,
    base_url: str | None,
    api_key: str | None,
) -> Any:
    from pydantic_ai.models.openai import OpenAIChatModel, OpenAIProvider

    provider = OpenAIProvider(base_url=base_url, api_key=api_key)
    return OpenAIChatModel(model_name, provider=provider)


def resolve_model(config: "ModelConfig") -> tuple[Any, Any | None]:
    provider, model_name = _split_model_name(config.name, config.model_provider)
    provider = _normalize_provider(provider)

    _mirror_xai_key()

    model_settings = None
    if config.temperature is not None:
        from pydantic_ai.models import ModelSettings

        model_settings = ModelSettings(temperature=config.temperature)

    if config.base_url:
        return (
            _build_openai_model(model_name, config.base_url, config.api_key),
            model_settings,
        )

    if provider:
        return f"{provider}:{model_name}", model_settings

    return config.name, model_settings
