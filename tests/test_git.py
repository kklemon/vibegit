import subprocess
import pytest

from conftest import test_repositories
from vibegit.git import get_git_status


@pytest.mark.parametrize("repo", list(test_repositories), indirect=True)
def test_git_status(repo):
    status = get_git_status(repo)

    assert len(status.changed_files) > 0

    # Test repo configs have been chosen to always have untracked files
    assert len(status.untracked_files) > 0
