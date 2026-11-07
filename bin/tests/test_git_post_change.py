"""Tests for bin/git-post-change, the post-merge, post-checkout and post-rewrite hook
that runs `sqlx-refresh --if-stale`. It runs in a scratch repo whose hooks point at
it, with a stand-in bin/sqlx-refresh that logs how it was called."""

import os
import shutil
import subprocess
from pathlib import Path

import pytest

HOOK = Path(__file__).resolve().parents[1] / "git-post-change"


def git(repo, *args, env=None):
    return subprocess.run(
        ["git", *args], cwd=repo, capture_output=True, text=True, check=False, env=env
    )


@pytest.fixture
def repo(tmp_path):
    root = tmp_path / "repo"
    (root / "bin").mkdir(parents=True)
    hooks = tmp_path / "hooks"
    hooks.mkdir()
    (hooks / "post-checkout").symlink_to(HOOK)
    git(root, "init", "-q", "-b", "main")
    git(root, "config", "user.name", "t")
    git(root, "config", "user.email", "t@example.com")
    git(root, "config", "core.hooksPath", str(hooks))
    (root / "README").write_text("x\n")
    git(root, "add", "README")
    git(root, "commit", "-qm", "start")
    log = tmp_path / "called"
    return root, log


def stand_in(root, log, status=0):
    script = root / "bin/sqlx-refresh"
    script.write_text(
        f"import sys\nopen({str(log)!r}, 'a').write(' '.join(sys.argv[1:]) + '\\n')\n"
        f"print('stand-in ran', file=sys.stderr)\nsys.exit({status})\n"
    )


def test_a_checkout_runs_sqlx_refresh_if_stale_for_the_repo(repo):
    root, log = repo
    stand_in(root, log)
    done = git(root, "checkout", "-q", "-b", "other")
    assert done.returncode == 0, done.stderr
    assert log.read_text() == f"--repo {root} --if-stale\n"


def test_a_refresh_that_fails_never_fails_the_git_command(repo):
    root, log = repo
    stand_in(root, log, status=3)
    done = git(root, "checkout", "-q", "-b", "other")
    assert done.returncode == 0, done.stderr
    assert log.exists()


def test_without_python3_it_does_nothing(repo, tmp_path):
    root, log = repo
    stand_in(root, log)
    only_git = tmp_path / "only-git"
    only_git.mkdir()
    (only_git / "git").symlink_to(shutil.which("git"))
    env = {**os.environ, "PATH": str(only_git)}
    done = subprocess.run(
        ["/bin/sh", str(HOOK)],
        cwd=root,
        env=env,
        capture_output=True,
        text=True,
        check=False,
    )
    assert (done.returncode, done.stdout, done.stderr) == (0, "", "")
    assert not log.exists()
