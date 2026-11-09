"""bin/git-pre-push, the pre-push hook: a push to origin's main is refused when a
skill in the commit pushed has a version origin's tag holds with other contents, which
CI's release-skills would otherwise fail on after the push. Each test pushes for real,
from a scratch repo whose .githooks/pre-push links to the script, to a bare origin."""

import shutil
import subprocess
from pathlib import Path

import pytest

REPO = Path(__file__).resolve().parents[2]

SKILL = """---
name: demo
description: A demo skill.
metadata:
  version: "1"
---

# Demo
"""


def git(cwd: Path, *args: str) -> str:
    return subprocess.run(
        ["git", *args], cwd=cwd, capture_output=True, text=True, check=True
    ).stdout


def skills(cwd: Path, *args: str) -> subprocess.CompletedProcess:
    return subprocess.run(
        [str(cwd / "bin/skills"), *args],
        cwd=cwd,
        capture_output=True,
        text=True,
        check=False,
    )


@pytest.fixture
def repo(tmp_path) -> Path:
    """A repo whose version 1 of the demo skill is pushed to origin and tagged, with
    the pre-push hook enabled as install.py enables it."""
    origin = tmp_path / "origin.git"
    subprocess.run(
        ["git", "init", "-q", "--bare", "-b", "main", str(origin)], check=True
    )
    root = tmp_path / "repo"
    (root / "bin").mkdir(parents=True)
    for script in ("git-pre-push", "skills"):
        shutil.copy(REPO / "bin" / script, root / "bin" / script)
    (root / ".githooks").mkdir()
    (root / ".githooks/pre-push").symlink_to("../bin/git-pre-push")
    (root / "docs").mkdir()
    (root / "docs/entry-format.md").write_text("# Format\n")
    (root / "skills/demo").mkdir(parents=True)
    (root / "skills/demo/SKILL.md").write_text(SKILL)
    git(root, "init", "-q", "-b", "main")
    git(root, "config", "user.email", "t@example.com")
    git(root, "config", "user.name", "t")
    git(root, "remote", "add", "origin", str(origin))
    assert skills(root, "sync").returncode == 0
    git(root, "add", "-A")
    git(root, "commit", "-qm", "version 1")
    git(root, "config", "core.hooksPath", ".githooks")
    git(root, "push", "-q", "origin", "main")
    assert skills(root, "release", "--push-url", str(origin)).returncode == 0
    return root


def push(repo: Path, *refspec: str) -> subprocess.CompletedProcess:
    return subprocess.run(
        ["git", "push", "origin", *(refspec or ("main",))],
        cwd=repo,
        capture_output=True,
        text=True,
        check=False,
    )


def change_skill(repo: Path, text: str) -> None:
    skill = repo / "skills/demo/SKILL.md"
    skill.write_text(skill.read_text().replace("# Demo", text))


def test_a_skill_changed_under_a_tagged_version_is_refused(repo):
    # Committed without bin/skills sync, so still version 1, which is tagged.
    change_skill(repo, "# Demo, changed")
    git(repo, "commit", "-qam", "changed, version 1")
    done = push(repo)
    assert done.returncode != 0
    assert "demo: version 1" in done.stderr
    # The hook's last line; git's own "failed to push" follows it.
    assert any(line.startswith("PUSH REFUSED:") for line in done.stderr.splitlines()), (
        done.stderr
    )
    assert "bin/skills sync" in done.stderr
    assert git(repo, "rev-parse", "origin/main") != git(repo, "rev-parse", "HEAD")


def test_a_skill_bumped_by_sync_is_pushed(repo):
    change_skill(repo, "# Demo, changed")
    assert skills(repo, "sync").returncode == 0
    git(repo, "commit", "-qam", "changed, version 2")
    done = push(repo)
    assert done.returncode == 0, done.stderr


def test_another_branch_passes(repo):
    change_skill(repo, "# Demo, changed")
    git(repo, "commit", "-qam", "changed, version 1, stale")
    done = push(repo, "HEAD:refs/heads/topic")
    assert done.returncode == 0, done.stderr


def test_deleting_main_is_not_checked(repo):
    done = push(repo, ":main")
    assert "PUSH REFUSED" not in done.stderr
    assert "skills" not in done.stderr, done.stderr


def test_another_remote_passes(repo, tmp_path):
    fork = tmp_path / "fork.git"
    subprocess.run(["git", "init", "-q", "--bare", "-b", "main", str(fork)], check=True)
    git(repo, "remote", "add", "fork", str(fork))
    change_skill(repo, "# Demo, changed")
    git(repo, "commit", "-qam", "changed, version 1, stale")
    done = subprocess.run(
        ["git", "push", "fork", "main"],
        cwd=repo,
        capture_output=True,
        text=True,
        check=False,
    )
    assert done.returncode == 0, done.stderr


def test_a_check_that_cant_run_warns_and_lets_the_push_go(repo):
    (repo / "bin/skills").write_text("#!/bin/sh\necho 'skills: broken' >&2\nexit 2\n")
    change_skill(repo, "# Demo, changed")
    git(repo, "commit", "-qam", "changed, version 1, stale")
    done = push(repo)
    assert done.returncode == 0, done.stderr
    assert "couldn't check the skills" in done.stderr


def test_a_fetch_that_fails_is_said_when_the_push_goes_ahead(repo, tmp_path):
    # Fetches go to a URL that's gone; pushes still reach origin.
    origin = git(repo, "remote", "get-url", "origin").strip()
    git(repo, "remote", "set-url", "origin", str(tmp_path / "gone.git"))
    git(repo, "remote", "set-url", "--push", "origin", origin)
    change_skill(repo, "# Demo, changed")
    assert skills(repo, "sync").returncode == 0
    git(repo, "commit", "-qam", "changed, version 2")
    done = push(repo)
    assert done.returncode == 0, done.stderr
    assert "couldn't fetch origin" in done.stderr
