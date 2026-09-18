import os
import subprocess
import tempfile
from collections.abc import Iterator
from contextlib import contextmanager
from dataclasses import dataclass
from functools import partial
from urllib.parse import urlparse

import git
import pytest
from dotenv import load_dotenv

load_dotenv()


@dataclass
class TestRepoConfig:
    repo_url: str
    clone_to_commit: str
    num_reset_commits: int

    @contextmanager
    def clone(self) -> Iterator[str]:
        repo_url = urlparse(self.repo_url)
        repo_name = repo_url.path.split("/")[-1]

        with tempfile.TemporaryDirectory(prefix=repo_name) as temp_dir:
            repo_path = temp_dir

            call = partial(
                subprocess.run,
                cwd=repo_path,
                stdout=subprocess.DEVNULL,
                check=True,
            )

            call(["git", "init"])
            call(["git", "remote", "add", "origin", self.repo_url])
            call(
                [
                    "git",
                    "fetch",
                    "origin",
                    f"--depth={self.num_reset_commits + 1}",
                    self.clone_to_commit,
                ],
            )
            call(["git", "reset", "--hard", "FETCH_HEAD"])
            call(["git", "reset", "--soft", f"HEAD~{self.num_reset_commits}"])
            call(["git", "reset"])

            yield repo_path


test_repositories: dict[str, TestRepoConfig] = {
    "gdquest-demos/godot-3-beginner-2d-platformer": TestRepoConfig(
        repo_url="https://github.com/gdquest-demos/godot-3-beginner-2d-platformer",
        clone_to_commit="569cfa322d9588ebacf6159784f1cabd9802f07b",
        num_reset_commits=5,
    ),
    "remix-jokes": TestRepoConfig(
        repo_url="https://github.com/remix-run/remix-jokes",
        clone_to_commit="03eace68a9ed7d5cd1f5081512ecd87ddecdcd14",
        num_reset_commits=5,
    ),
    "pypa/sampleproject": TestRepoConfig(
        repo_url="https://github.com/pypa/sampleproject",
        clone_to_commit="9b109b4522789762ad9dcd10540872133b6126e1",
        num_reset_commits=5,
    ),
}


test_models = [
    "google:gemini-3.8-flash",
    "google:gemini-3.5-flash-lite",
    "google:gemini-3.1-pro-preview",
    "openai:gpt-5.6-terra",
    "openai:gpt-5.6-sol",
    "openai:gpt-5.6-luna",
]


@pytest.fixture
def repo(request):
    repo_config = test_repositories[request.param]
    with repo_config.clone() as repo_path:
        yield git.Repo(repo_path)


@pytest.fixture
def chat_model(request):
    model_name = request.param
    if model_name.startswith("openai:") and not os.environ.get("OPENAI_API_KEY"):
        pytest.skip("OPENAI_API_KEY is not set")
    if model_name.startswith("google:") and not os.environ.get("GOOGLE_API_KEY"):
        pytest.skip("GOOGLE_API_KEY is not set")
    if model_name.startswith("grok:") and not os.environ.get("GROK_API_KEY"):
        pytest.skip("GROK_API_KEY is not set")

    yield model_name
