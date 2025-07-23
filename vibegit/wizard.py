import os
import sys

import inquirer
from rich.console import Console

from vibegit.config import CONFIG_PATH, Config, ModelConfig

console = Console()


class ConfigWizard:
    """Interactive configuration wizard for VibeGit.

    Shows up when users start VibeGit for the first time,
    asking for essential configuration values like the LLM model and API keys.
    """

    # Model presets with friendly names and their LangChain init_chat_model format
    MODEL_PRESETS = {
        "Gemini 2.5 Flash (Recommended)": "google_genai:gemini-2.5-flash",
        "Gemini 2.5 Pro": "google_genai:gemini-2.5-pro",
        "GPT-4o": "openai:gpt-4o",
        "GPT-4.1": "openai:gpt-4.1",
        "o4-mini": "openai:o4-mini",
        "o3-mini": "openai:o3-mini",
        "Custom model (LangChain format)": "custom",
    }

    # Map model name prefixes to their API key environment variables
    MODEL_TO_API_KEY_ENV = {
        "google_genai": "GOOGLE_API_KEY",
        "openai": "OPENAI_API_KEY",
    }

    def __init__(self):
        self.config = Config()

    def run(self):
        """Run the interactive configuration wizard."""
        console.print("[bold blue]VibeGit Configuration Wizard[/bold blue]")
        console.print("Let's set up VibeGit for first use.\n")

        self._configure_model()
        self._configure_api_keys()
        self._save_config()

        console.print(
            "\n[bold green]Configuration complete! VibeGit is ready to use.[/bold green]"
        )

    def _configure_model(self):
        """Configure the LLM model to use."""
        console.print("[bold]LLM Model Configuration[/bold]")
        console.print("Choose which AI model to use for generating commit proposals:")

        # Create choices for the model selection
        model_choices = list(self.MODEL_PRESETS.keys())

        questions = [
            inquirer.List(
                "model_choice",
                message="Select an LLM model:",
                choices=model_choices,
                default=model_choices[0],  # Default to Gemini 2.5 Flash
            ),
        ]

        answers = inquirer.prompt(questions)
        model_choice = answers.get("model_choice")

        if model_choice == "Custom model":
            custom_model = self._get_custom_model()
            self.config.model = ModelConfig(name=custom_model)
        else:
            self.config.model = ModelConfig(name=self.MODEL_PRESETS[model_choice])

        console.print(f"[green]Model set to: {self.config.model.name}[/green]")

    def _get_custom_model(self):
        """Prompt for a custom model name."""
        questions = [
            inquirer.Text(
                "custom_model",
                message="Enter the model name in LangChain's init_chat_model format:",
                validate=lambda _, x: len(x) > 0,
            ),
        ]

        answers = inquirer.prompt(questions)
        return answers.get("custom_model")

    def _configure_api_keys(self):
        """Configure API keys based on the selected model."""
        console.print("\n[bold]API Key Configuration[/bold]")

        model_name = self.config.model.name

        # Determine which API key we need based on the model prefix
        api_key_env = None

        for prefix, env_var in self.MODEL_TO_API_KEY_ENV.items():
            if model_name.startswith(prefix):
                api_key_env = env_var
                break

        if not api_key_env:
            console.print(
                "[yellow]No API key configuration needed for this model.[/yellow]"
            )
            return

        # Check if the API key is already in the environment
        if api_key_env in os.environ and os.environ[api_key_env]:
            console.print(
                f"[green]Found {api_key_env} in environment variables.[/green]"
            )

            # Ask if user wants to save to config
            questions = [
                inquirer.Confirm(
                    "save_api_key",
                    message=f"Do you want to save this {api_key_env} to the VibeGit config?",
                    default=True,
                ),
            ]

            answers = inquirer.prompt(questions)
            if answers and answers["save_api_key"]:
                self.config.api_keys[api_key_env] = os.environ[api_key_env]
                console.print(f"[green]{api_key_env} saved to config.[/green]")
            else:
                console.print(
                    f"[yellow]{api_key_env} will be used from environment variables.[/yellow]"
                )

            return

        # API key not found in environment, prompt for it
        console.print(
            f"[yellow]No {api_key_env} found in environment variables.[/yellow]"
        )

        questions = [
            inquirer.Password(
                "api_key",
                message=f"Enter your {api_key_env}:",
                validate=lambda _, x: len(x) > 0,
            ),
        ]

        answers = inquirer.prompt(questions)
        api_key = answers.get("api_key")

        if api_key:
            # Set in both environment and config
            os.environ[api_key_env] = api_key
            self.config.api_keys[api_key_env] = api_key
            console.print(f"[green]{api_key_env} configured successfully.[/green]")
        else:
            console.print(
                f"[red]No {api_key_env} provided. You'll need to set it later.[/red]"
            )

    def _save_config(self):
        """Save the configuration to disk."""
        try:
            self.config.save_config()
            console.print(f"[green]Configuration saved to {CONFIG_PATH}[/green]")
        except Exception as e:
            console.print(f"[bold red]Error saving configuration: {e}[/bold red]")
            sys.exit(1)


def should_run_wizard():
    """Check if the config wizard should run (first time use)."""
    return not CONFIG_PATH.exists()


def run_wizard_if_needed():
    """Run the config wizard if no configuration file exists."""
    if should_run_wizard():
        wizard = ConfigWizard()
        wizard.run()
        return True
    return False
