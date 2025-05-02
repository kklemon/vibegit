import os
from pathlib import Path
from typing import Any, Self

import platformdirs
import toml
from langchain.chat_models import init_chat_model
from langchain_core.language_models.chat_models import BaseChatModel
from pydantic import Field, model_validator
from pydantic_settings import BaseSettings as _BaseSettings
from pydantic_settings import (
    PydanticBaseSettingsSource,
    SettingsConfigDict,
    TomlConfigSettingsSource,
)

from vibegit.git import GitContextFormatter

CONFIG_PATH = Path(platformdirs.user_config_dir("vibegit")) / "config.toml"


class BaseSettings(_BaseSettings):
    def get_by_path(self, path: str):
        current_part, *remaining_parts = path.split(".", maxsplit=1)

        current_value = getattr(self, current_part)

        if not remaining_parts:
            return current_value

        if isinstance(current_value, dict):
            if len(remaining_parts) != 1:
                raise ValueError(
                    f"Expected exactly one remaining part, got {len(remaining_parts)}"
                )
            return current_value.get(remaining_parts[0])

        if isinstance(current_value, BaseSettings):
            return current_value.get_by_path(remaining_parts[0])

        raise ValueError(f"Expected a BaseSettings or dict, got {type(current_value)}")

    def set_by_path(self, path: str, value: Any):
        current_part, *remaining_parts = path.split(".", maxsplit=1)

        if not remaining_parts:
            # This is the final part, set the attribute
            if hasattr(self, current_part):
                # TODO: Consider type validation/coercion here if needed
                setattr(self, current_part, value)
            else:
                raise AttributeError(
                    f"'{type(self).__name__}' object has no attribute '{current_part}'"
                )
            return

        # Need to delve deeper
        try:
            current_attribute = getattr(self, current_part)
        except AttributeError:
            raise AttributeError(
                f"'{type(self).__name__}' object has no attribute '{current_part}'"
            )

        if isinstance(current_attribute, BaseSettings):
            current_attribute.set_by_path(remaining_parts[0], value)
        elif isinstance(current_attribute, dict):
            # Assuming the dictionary path has only one level left
            dict_key = remaining_parts[0]
            current_attribute[dict_key] = value
            # Re-assign to potentially trigger Pydantic updates if needed
            setattr(self, current_part, current_attribute)
        else:
            raise TypeError(
                f"Cannot set path '{path}'. Attribute '{current_part}' is neither a "
                f"BaseSettings instance nor a dict, but {type(current_attribute)}"
            )


class ContextFormattingConfig(BaseSettings):
    include_active_branch: bool = True
    truncate_lines: int | None = 240
    include_latest_commits: int | None = 5

    def get_context_formatter(
        self, user_instructions: str | None = None
    ) -> GitContextFormatter:
        return GitContextFormatter(
            include_active_branch=self.include_active_branch,
            truncate_lines=self.truncate_lines,
            include_latest_commits=self.include_latest_commits,
            user_instructions=user_instructions,
        )

    def save_config(self):
        CONFIG_PATH.parent.mkdir(parents=True, exist_ok=True)
        with open(CONFIG_PATH, "w") as f:
            toml.dump(self.model_dump(), f)


class Config(BaseSettings):
    model_name: str = "google_genai:gemini-2.5-flash-preview-04-17"
    context_formatting: ContextFormattingConfig = ContextFormattingConfig()
    api_keys: dict[str, str] = Field(default_factory=dict)

    model_config = SettingsConfigDict(toml_file=CONFIG_PATH)

    @model_validator(mode="after")
    def inject_api_keys(self):
        for key, value in self.api_keys.items():
            os.environ[key.upper()] = value
        return self

    @classmethod
    def settings_customise_sources(
        cls,
        settings_cls: type[_BaseSettings],
        init_settings: PydanticBaseSettingsSource,
        env_settings: PydanticBaseSettingsSource,
        dotenv_settings: PydanticBaseSettingsSource,
        file_secret_settings: PydanticBaseSettingsSource,
    ) -> tuple[PydanticBaseSettingsSource, ...]:
        return (TomlConfigSettingsSource(settings_cls),)

    def get_chat_model(self) -> BaseChatModel:
        return init_chat_model(self.model_name)

    def save_config(self):
        CONFIG_PATH.parent.mkdir(parents=True, exist_ok=True)
        with open(CONFIG_PATH, "w") as f:
            toml.dump(self.model_dump(), f)


config = Config()
