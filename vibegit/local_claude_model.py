"""Local Claude CLI chat model implementation for LangChain integration.

This module provides a custom LangChain chat model that integrates with the
Claude CLI command-line tool. It enables using a locally-installed Claude CLI
for AI-powered commit message generation.

Usage:
    Configure VibeGit to use Local Claude via the wizard:
        vibegit config wizard

    Or manually set the model:
        vibegit config set model.name local:claude

    The Claude CLI must be installed and available in your PATH.
    Install from: https://github.com/anthropics/claude-code

Requirements:
    - claude CLI must be installed and in PATH
    - claude must support --output-format json and --json-schema flags
"""

import json
import subprocess
from typing import Any, Iterator, Literal, cast

from langchain_core.callbacks import CallbackManagerForLLMRun
from langchain_core.language_models.chat_models import BaseChatModel
from langchain_core.messages import AIMessage, BaseMessage
from langchain_core.outputs import ChatGeneration, ChatResult
from langchain_core.runnables import Runnable, RunnableConfig
from pydantic import BaseModel


class StructuredOutputRunnable(Runnable):
    """Wrapper that parses JSON output into Pydantic models."""

    def __init__(
        self,
        model: "LocalClaudeChatModel",
        schema: type[BaseModel],
    ):
        """Initialize the structured output wrapper.

        Args:
            model: The base LocalClaudeChatModel instance
            schema: Pydantic model class to parse into
        """
        self.model = model
        self.schema = schema

    def invoke(
        self,
        input: Any,
        config: RunnableConfig | None = None,
        **kwargs: Any,
    ) -> BaseModel:
        """Invoke the model and parse the output into a Pydantic object.

        Args:
            input: Input messages
            config: Optional runnable config
            **kwargs: Additional arguments

        Returns:
            Parsed Pydantic model instance
        """
        # Call the underlying model
        result = self.model.invoke(input, config=config, **kwargs)

        # Parse the JSON content from the AIMessage
        if isinstance(result, AIMessage):
            content = result.content
        else:
            content = str(result)

        # Parse JSON and create Pydantic object
        try:
            parsed_data = json.loads(content)
            return self.schema.model_validate(parsed_data)
        except json.JSONDecodeError as e:
            raise RuntimeError(
                f"Failed to parse JSON from model output: {e}\n"
                f"Content: {content}"
            ) from e
        except Exception as e:
            raise RuntimeError(
                f"Failed to validate output against schema: {e}\n"
                f"Content: {content}"
            ) from e


class LocalClaudeChatModel(BaseChatModel):
    """Chat model that uses local Claude CLI for inference.

    This model wraps the 'claude' CLI command and enables structured output
    via JSON schema validation.
    """

    model_name: str = "local:claude"
    temperature: float | None = None
    json_schema: dict[str, Any] | None = None
    schema_class: type[BaseModel] | None = None
    claude_command: str = "claude"

    @property
    def _llm_type(self) -> str:
        """Return identifier for the LLM type."""
        return "local-claude"

    def _format_messages_to_prompt(self, messages: list[BaseMessage]) -> str:
        """Convert LangChain messages to a single prompt string.

        Args:
            messages: List of LangChain messages (system, user, assistant, etc.)

        Returns:
            Formatted prompt string combining all messages
        """
        prompt_parts = []

        for message in messages:
            role = message.type
            content = message.content

            if role == "system":
                prompt_parts.append(f"System: {content}\n")
            elif role == "user" or role == "human":
                prompt_parts.append(f"User: {content}\n")
            elif role == "assistant" or role == "ai":
                prompt_parts.append(f"Assistant: {content}\n")
            else:
                # Generic fallback
                prompt_parts.append(f"{role.title()}: {content}\n")

        return "\n".join(prompt_parts)

    def _generate(
        self,
        messages: list[BaseMessage],
        stop: list[str] | None = None,
        run_manager: CallbackManagerForLLMRun | None = None,
        **kwargs: Any,
    ) -> ChatResult:
        """Generate a response using the Claude CLI.

        Args:
            messages: List of messages to send to Claude
            stop: Stop sequences (not supported by CLI)
            run_manager: Callback manager for streaming
            **kwargs: Additional arguments

        Returns:
            ChatResult containing the generated response

        Raises:
            RuntimeError: If Claude CLI execution fails
        """
        # Format messages into a prompt
        prompt = self._format_messages_to_prompt(messages)

        # Build CLI command
        cmd = [self.claude_command]

        # Add output format flags
        cmd.extend(["--output-format", "json", "--model", "haiku", '--tools', '""'])

        # Add JSON schema if structured output is configured
        if self.json_schema:
            cmd.extend(["--json-schema", json.dumps(self.json_schema)])

        # Add temperature if specified
        if self.temperature is not None:
            cmd.extend(["--temperature", str(self.temperature)])

        # Add print flag to get direct output
        cmd.append("--print")

        print(cmd)

        # Execute Claude CLI
        try:
            result = subprocess.run(
                cmd,
                input=prompt,
                capture_output=True,
                text=True,
                check=True,
            )

            output = result.stdout.strip()

            # Parse JSON response if we're using structured output
            if self.json_schema:
                try:
                    print(output)
                    parsed_output = json.loads(output)['structured_output']
                    # Return as JSON string for LangChain to parse
                    content = json.dumps(parsed_output)
                except json.JSONDecodeError as e:
                    raise RuntimeError(
                        f"Failed to parse JSON response from Claude CLI: {e}\n"
                        f"Output: {output}"
                    ) from e
            else:
                content = output

            # Create AI message with the response
            message = AIMessage(content=content)
            generation = ChatGeneration(message=message)

            return ChatResult(generations=[generation])

        except subprocess.CalledProcessError as e:
            error_msg = (
                f"Claude CLI command failed with exit code {e.returncode}\n"
                f"Command: {' '.join(cmd)}\n"
                f"Stderr: {e.stderr}"
            )
            raise RuntimeError(error_msg) from e
        except FileNotFoundError as e:
            raise RuntimeError(
                f"Claude CLI command '{self.claude_command}' not found. "
                "Please ensure the 'claude' CLI is installed and in your PATH."
            ) from e

    def with_structured_output(
        self,
        schema: type[BaseModel] | dict[str, Any],
        **kwargs: Any,
    ) -> Runnable:
        """Enable structured output using JSON schema validation.

        Args:
            schema: Pydantic model or JSON schema dict
            **kwargs: Additional configuration

        Returns:
            Runnable that returns parsed Pydantic objects
        """
        # Convert Pydantic model to JSON schema if needed
        if isinstance(schema, type) and issubclass(schema, BaseModel):
            json_schema = schema.model_json_schema()
            schema_class = schema
        else:
            json_schema = schema
            schema_class = None

        # Create new instance with schema configuration
        configured_model = self.__class__(
            model_name=self.model_name,
            temperature=self.temperature,
            claude_command=self.claude_command,
            json_schema=json_schema,
            schema_class=schema_class,
        )

        # If we have a Pydantic class, wrap in StructuredOutputRunnable
        if schema_class is not None:
            return StructuredOutputRunnable(configured_model, schema_class)
        else:
            # For dict schemas, return the model directly
            return configured_model

    def _stream(
        self,
        messages: list[BaseMessage],
        stop: list[str] | None = None,
        run_manager: CallbackManagerForLLMRun | None = None,
        **kwargs: Any,
    ) -> Iterator[ChatGeneration]:
        """Stream responses from Claude CLI.

        Note: Streaming is not currently implemented for Local Claude.
        This falls back to non-streaming generation.
        """
        # For now, fall back to non-streaming
        result = self._generate(messages, stop=stop, run_manager=run_manager, **kwargs)
        yield result.generations[0]
