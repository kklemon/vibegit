from functools import partial
import git
import subprocess
import tempfile
from contextlib import contextmanager
from dataclasses import dataclass
from typing import Iterator
from urllib.parse import urlparse
from langchain.chat_models import init_chat_model

import pytest


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

            call = partial(subprocess.call, cwd=repo_path, stdout=subprocess.DEVNULL)

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
        num_reset_commits=8,
    ),
    "remix-jokes": TestRepoConfig(
        repo_url="https://github.com/remix-run/remix-jokes",
        clone_to_commit="03eace68a9ed7d5cd1f5081512ecd87ddecdcd14",
        num_reset_commits=8,
    ),
    "pypa/sampleproject": TestRepoConfig(
        repo_url="https://github.com/pypa/sampleproject",
        clone_to_commit="9b109b4522789762ad9dcd10540872133b6126e1",
        num_reset_commits=8,
    ),
}


test_models = [
    "google_genai:gemini-2.5-flash-preview-04-17",
    "google_genai:gemini-2.5-pro-preview-03-25",
    "openai:gpt-4o",
    "openai:gpt-4.1",
    "openai:o4-mini",
    "openai:o3-mini",
]


@pytest.fixture
def repo(request):
    repo_config = test_repositories[request.param]
    with repo_config.clone() as repo_path:
        yield git.Repo(repo_path)


@pytest.fixture
def chat_model(request):
    yield init_chat_model(request.param)
