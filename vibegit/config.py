import collections
import os
from pathlib import Path
from typing import Any, cast, get_args, get_origin

import platformdirs
import toml
from langchain.chat_models import init_chat_model
from langchain_core.language_models.chat_models import BaseChatModel
from pydantic import Field, TypeAdapter, ValidationError, model_validator
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
        """
        Sets a value at the specified path, coercing it to the target type.

        Args:
            path: A dot-separated string representing the path (e.g., "model.temperature").
            value: The value to set. It will be parsed/coerced to the target type.

        Raises:
            AttributeError: If the path is invalid.
            ValueError: If the value cannot be coerced to the target type or path is malformed.
            TypeError: If trying to set a path through an unsupported type.
        """
        parts = path.split(".")
        if not parts:
            raise ValueError("Path cannot be empty")
        self._set_recursive(parts, value, original_path=path)

    def _set_recursive(self, parts: list[str], value: Any, original_path: str):
        current_part = parts[0]
        remaining_parts = parts[1:]

        if not hasattr(self, current_part):
            raise AttributeError(
                f"'{type(self).__name__}' object has no attribute '{current_part}' in path '{original_path}'"
            )

        if not remaining_parts:
            # --- Base Case: Set the final attribute ---
            try:
                field_info = self.model_fields.get(current_part)
                if field_info and field_info.annotation:
                    # Use TypeAdapter for validation and coercion
                    adapter = TypeAdapter(field_info.annotation)
                    coerced_value = adapter.validate_python(value)
                    setattr(self, current_part, coerced_value)
                else:
                    # Fallback if no type annotation found (less safe)
                    # Or raise error if strict typing is required?
                    print(
                        f"Warning: No type annotation found for field '{current_part}'. Setting value directly."
                    )
                    setattr(self, current_part, value)
            except ValidationError as e:
                raise ValueError(
                    f"Invalid value for '{original_path}': {value!r}. Details: {e}"
                ) from e
            return

        # --- Recursive Step: Traverse deeper ---
        target_attribute = getattr(self, current_part)

        if isinstance(target_attribute, BaseSettings):
            # Recurse into nested BaseSettings
            target_attribute._set_recursive(
                remaining_parts, value, original_path=original_path
            )
        elif isinstance(target_attribute, dict):
            if len(remaining_parts) != 1:
                # Currently supporting only one level deep modification in dicts via path
                raise ValueError(
                    f"Path into dict must have exactly one remaining part for key access. "
                    f"Path: '{original_path}', Remaining: {'.'.join(remaining_parts)}"
                )

            dict_key = remaining_parts[0]

            # Determine the expected *value* type for the dictionary
            expected_value_type = Any
            try:
                dict_field_info = self.model_fields.get(current_part)
                if dict_field_info and dict_field_info.annotation:
                    origin = get_origin(dict_field_info.annotation)
                    args = get_args(dict_field_info.annotation)

                    # Check if it's a dict-like type (e.g., dict, Dict) and has type args
                    if (
                        origin
                        and issubclass(origin, collections.abc.Mapping)
                        and len(args) == 2
                    ):
                        expected_value_type = args[1]
                    elif origin and not issubclass(origin, collections.abc.Mapping):
                        raise TypeError(
                            f"Attribute '{current_part}' is not a Mapping type, but {origin}"
                        )

                # Coerce the incoming value using the determined dictionary value type
                adapter = TypeAdapter(expected_value_type)
                coerced_value = adapter.validate_python(value)

                # Set the value in the dictionary
                target_attribute[dict_key] = coerced_value
                # Note: Re-assigning the dict via setattr(self, current_part, target_attribute)
                # might be needed if there are complex validators on the dict field itself,
                # but often isn't necessary for simple dict updates.

            except ValidationError as e:
                raise ValueError(
                    f"Invalid value for '{original_path}' (key: '{dict_key}'): {value!r}. "
                    f"Expected type '{expected_value_type}'. Details: {e}"
                ) from e
            except Exception as e:
                # Catch other potential errors during type introspection/setting
                raise TypeError(
                    f"Error setting dict key '{dict_key}' in '{current_part}' for path '{original_path}': {e}"
                ) from e

        else:
            # Path tries to go through an attribute that is neither BaseSettings nor dict
            raise TypeError(
                f"Cannot set path '{original_path}'. Attribute '{current_part}' is neither a "
                f"BaseSettings instance nor a dict, but {type(target_attribute)}"
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


class ModelConfig(BaseSettings):
    name: str = "google_genai:gemini-2.5-flash-preview-04-17"
    temperature: float | None = None  # Use the default temperature

    def get_chat_model(self) -> BaseChatModel:
        kwargs: dict[str, Any] = dict(
            model=self.name,
        )

        if self.temperature is not None:
            kwargs["temperature"] = self.temperature

        return cast(BaseChatModel, init_chat_model(**kwargs))


class Config(BaseSettings):
    model: ModelConfig = ModelConfig()
    context_formatting: ContextFormattingConfig = ContextFormattingConfig()
    api_keys: dict[str, str] = Field(default_factory=dict)
    allow_excluding_changes: bool = True
    watermark: bool = True

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

    def save_config(self):
        CONFIG_PATH.parent.mkdir(parents=True, exist_ok=True)
        with open(CONFIG_PATH, "w") as f:
            toml.dump(self.model_dump(), f)
