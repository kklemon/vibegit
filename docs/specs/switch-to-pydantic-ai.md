# Switch from LangChain to Pydantic AI

## Goals
- Replace LangChain with Pydantic AI while preserving current CLI behavior and config shape.
- Keep breaking changes minimal for existing users and configs.
- Maintain support for the same default model presets (Gemini, OpenAI, xAI, custom OpenAI-compatible).
- Preserve structured-output parsing into the existing Pydantic schemas.

## Non-goals
- Redesigning the prompts or commit proposal logic beyond what is required for compatibility.
- Adding new providers or features unrelated to the migration.

## Current State (Code Assessment)
- `vibegit/config.py`
  - `ModelConfig.get_chat_model()` uses LangChain `init_chat_model` and returns `BaseChatModel`.
  - Model config is stored as `model.name` (LangChain `provider:model` string), `temperature`, `base_url`, `api_key`, `model_provider`.
  - `Config.inject_api_keys()` writes `api_keys` entries to environment variables.
- `vibegit/ai.py`
  - `CommitProposalAI` uses `BaseChatModel.with_structured_output(schema)` and `.invoke(...)`.
  - Schema is chosen based on `allow_excluding_changes`.
- `vibegit/wizard.py`
  - Uses LangChain wording and expects `provider:model` (via `init_chat_model` format).
  - Supports custom OpenAI-compatible config with base URL and API key.
- Tests
  - `tests/conftest.py` uses `init_chat_model` directly in fixture.
  - `tests/test_config.py` monkeypatches `init_chat_model`.
  - `tests/test_pipeline.py` performs a live end-to-end run with the model.
- Docs
  - `README.md` references LangChain and `init_chat_model` format.
- Prompt/schema mismatch
  - `vibegit/prompts.py` uses `reasoning` in the "complete" example, while
    `CommitProposalSchema` expects `explanation`. LangChain structured output may have
    been forgiving; Pydantic AI will be strict unless we address this.

## Pydantic AI Facts That Affect This Migration
- Pydantic AI supports model strings in the form `<provider>:<model>` and auto-selects
  the appropriate model class/provider when an `Agent` is created with that string.
- `Agent` accepts `model`, `system_prompt`, `output_type`, and `model_settings`.
- Structured output is driven by `output_type` and `run_sync()` returns a result with
  `result.output`.
- Temperature and similar parameters are set via `ModelSettings`.
- Provider strings and env vars used by Pydantic AI differ from LangChain:
  - OpenAI: `openai:<model>` with `OPENAI_API_KEY`.
  - Google Gemini (GLA): `google-gla:<model>` with `GOOGLE_API_KEY`.
  - xAI Grok: `grok:<model>` with `GROK_API_KEY`.
  - Custom OpenAI-compatible endpoints require `OpenAIProvider(base_url=..., api_key=...)`.

## Proposed Approach
Prefer native Pydantic AI string models wherever possible:
1. Use `Agent(model_string, output_type=..., system_prompt=..., model_settings=...)`
   for standard providers.
2. Only instantiate model classes/providers directly when a custom `base_url` or
   non-default provider settings are required.
3. Keep a thin adapter layer so the rest of the code (CLI, Git logic, schemas) stays stable.

The public-facing config should remain the same, but we should map legacy LangChain
prefixes to Pydantic AI equivalents to avoid breaking existing configs.

## Model Mapping Strategy
Keep the current `model.name` format but normalize to Pydantic AI model strings:
- **OpenAI**: `openai:<model>`
- **Gemini (Google GLA)**: `google-gla:<model>`
  - Map legacy `google_genai:<model>` → `google-gla:<model>`.
- **xAI/Grok**: `grok:<model>`
  - Map legacy `xai:<model>` → `grok:<model>`.
- **Custom OpenAI-compatible**:
  - Use `OpenAIChatModel(..., provider=OpenAIProvider(base_url=..., api_key=...))` when
    `base_url` is provided.

API keys:
- Keep reading/writing current config `api_keys`.
- If legacy `XAI_API_KEY` is set, mirror it to `GROK_API_KEY` at runtime to maintain
  compatibility.

## Detailed Plan

### 1) Add a Pydantic AI model resolver
Create a small module (e.g. `vibegit/llm.py`) that:
- Accepts `ModelConfig` and returns `(model, model_settings)` where:
  - `model` is either a Pydantic AI model string or a model instance.
  - `model_settings` is a `ModelSettings` object when temperature is set.
- Normalizes legacy prefixes:
  - `google_genai:` → `google-gla:`
  - `xai:` → `grok:`
- Builds a custom OpenAI-compatible model if `base_url` is provided.
- Optionally mirrors `XAI_API_KEY` → `GROK_API_KEY` for backward compatibility.

Acceptance criteria:
- The resolver supports the current wizard presets (but mapped to Pydantic AI names).
- The resolver handles custom OpenAI-compatible endpoints.

### 2) Update `ModelConfig` to return Pydantic AI model(s)
In `vibegit/config.py`:
- Replace `get_chat_model()` with `get_model()` or `get_llm()` that calls the resolver.
- Update type hints to accept either a model string or a model instance.
- Keep existing fields (`name`, `temperature`, `base_url`, `api_key`, `model_provider`).

Acceptance criteria:
- `Config` load/save remains unchanged.
- Existing configs work without changes for default presets.

### 3) Replace LangChain usage in `CommitProposalAI`
In `vibegit/ai.py`:
- Replace `BaseChatModel` usage with a Pydantic AI `Agent`.
- Initialize the agent with:
  - `model` from the resolver (string or model instance)
  - `output_type` set to either `CommitProposalsResultSchema` or
    `IncompleteCommitProposalsResultSchema`
  - `system_prompt` from `build_system_prompt(...)`
  - `model_settings` from the resolver
- In `propose_commits`, call `agent.run_sync(context)` and return `result.output`.
- Add error handling for validation failures with a clear CLI message.

Acceptance criteria:
- Behavior remains synchronous and `CommitProposalAI` interface is unchanged for the CLI.
- `result` remains a Pydantic model instance (same schema objects as today).

### 4) Fix prompt/schema field mismatch
To minimize parsing failures with stricter Pydantic AI validation:
- Update the prompt examples in `vibegit/prompts.py` to use `explanation` consistently.
- Alternatively (or additionally), add `alias="reasoning"` to `CommitProposalSchema.explanation`
  so both keys are accepted.

Acceptance criteria:
- Structured outputs validate even if the model uses the older `reasoning` key.

### 5) Update the wizard and docs to remove LangChain wording
In `vibegit/wizard.py`:
- Replace "LangChain format" with "Pydantic AI format (`<provider>:<model>`)".
- Update presets to Pydantic AI names:
  - `google-gla:gemini-2.5-flash` (default)
  - `google-gla:gemini-2.5-pro`
  - `openai:gpt-4o`, `openai:gpt-4.1`, `openai:o4-mini`, `openai:o3-mini`
  - `grok:<model>` for xAI
- Consider keeping legacy labels but mapping behind the scenes for compatibility.

In `README.md`:
- Replace LangChain references with Pydantic AI.
- Update model naming examples to use `<provider>:<model>`.
- Document provider/env-var differences (OpenAI, Google, Grok).

Acceptance criteria:
- Users are no longer instructed to use LangChain-specific formats.
- Existing configs continue to work.

### 6) Update tests
In `tests/conftest.py`:
- Replace `init_chat_model` usage with the new resolver and Pydantic AI `Agent`.
- Consider using Pydantic AI test models (`TestModel`/`FunctionModel`) to allow
  offline/deterministic runs for unit tests.

In `tests/test_config.py`:
- Update monkeypatch to the resolver or the new `ModelConfig` method.

In `tests/test_pipeline.py`:
- Keep the same flow but update initialization to the Pydantic AI client.
- Consider skipping tests if required API keys are missing.

Acceptance criteria:
- Tests pass with the same environment/API keys used today.

### 7) Update dependencies and lockfile
In `pyproject.toml`:
- Remove LangChain packages.
- Add `pydantic-ai` (or `pydantic-ai-slim`) with required extras:
  - `openai` for OpenAI and OpenAI-compatible providers
  - `google` for Gemini
- Ensure `pydantic` version remains compatible (Pydantic AI requires v2).

Update `uv.lock` accordingly.

Acceptance criteria:
- `uv sync` installs Pydantic AI and provider dependencies.
- LangChain packages are removed.

## Compatibility and Breaking Changes
- Keep `model.name` format `provider:model` to avoid config migration.
- Map legacy prefixes transparently:
  - `google_genai:` → `google-gla:`
  - `xai:` → `grok:`
- Handle env var differences:
  - xAI uses `GROK_API_KEY` (not `XAI_API_KEY`)
  - Google uses `GOOGLE_API_KEY`
  - OpenAI uses `OPENAI_API_KEY`
- Keep support for custom OpenAI-compatible endpoints via `OpenAIProvider(base_url=...)`.
- If a provider is unsupported by Pydantic AI, surface a clear error with guidance.

## Testing and Validation
- Unit tests:
  - Model resolver produces expected provider/model mapping.
  - Config still loads and saves unchanged.
- Integration tests:
  - Run `tests/test_pipeline.py` with at least one model per provider.
- Manual smoke test:
  - `vibegit config wizard` to verify model selection and key prompts.
  - `vibegit commit` on a sample repo to confirm end-to-end flow.

## Open Questions
- Do we want to deprecate legacy prefixes (`google_genai:`, `xai:`) with warnings,
  or keep silent mapping indefinitely?
- Should `model_provider` be repurposed as a provider override (e.g. `openrouter:`)
  when `model.name` is just a bare model name?

## Rollout Notes
- Make changes in a single release to avoid mixed LangChain/Pydantic AI states.
- Add release notes that call out the internal switch and any provider-specific changes.
