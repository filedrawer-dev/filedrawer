"""Tests for bin/new-worktree, which adds a worktree beside the main tree and starts
its warm build in the background. Each runs a copy of the script committed to a
scratch repo, since it finds its repo from its own path, with a fake bgrun first on
PATH that notes its arguments."""

import os
import shutil
import subprocess
from pathlib import Path
from types import SimpleNamespace

import pytest

SCRIPT = Path(__file__).resolve().parents[1] / "new-worktree"

FAKE_BGRUN = """#!/bin/sh
echo "bgrun $*" >> "$LOG"
echo 4242
"""


def git(tree: Path, *args: str) -> str:
    return subprocess.run(
        ["git", "-C", str(tree), "-c", "user.name=t", "-c", "user.email=t@t", *args],
        check=True,
        capture_output=True,
        text=True,
    ).stdout.strip()


@pytest.fixture
def repo(tmp_path):
    main = tmp_path / "proj"
    main.mkdir()
    git(main, "init", "-q", "-b", "main")
    (main / "bin").mkdir()
    shutil.copy(SCRIPT, main / "bin/new-worktree")
    git(main, "add", "bin")
    git(main, "commit", "-q", "-m", "one")
    git(main, "commit", "-q", "--allow-empty", "-m", "two")
    fakes = tmp_path / "fakes"
    fakes.mkdir()
    (fakes / "bgrun").write_text(FAKE_BGRUN)
    (fakes / "bgrun").chmod(0o755)
    log = tmp_path / "log"
    log.touch()
    env = {**os.environ, "PATH": f"{fakes}:{os.environ['PATH']}", "LOG": str(log)}

    def run(
        *args: str, tree: Path = main, cwd: Path | None = None
    ) -> subprocess.CompletedProcess:
        """Runs `tree`'s copy of the script, from `cwd` (default: `tree`)."""
        return subprocess.run(
            [str(tree / "bin/new-worktree"), *args],
            cwd=cwd or tree,
            env=env,
            capture_output=True,
            text=True,
            check=False,
        )

    return SimpleNamespace(main=main, tmp=tmp_path, log=log, run=run)


def detached(tree: Path) -> bool:
    return (
        subprocess.run(
            ["git", "-C", str(tree), "symbolic-ref", "-q", "HEAD"], capture_output=True
        ).returncode
        == 1
    )


def test_adds_a_worktree_beside_the_main_tree_and_starts_its_warm_build(repo):
    done = repo.run("embargo")
    assert done.returncode == 0, done.stderr
    tree = repo.tmp / "proj-wt-embargo"
    assert git(tree, "rev-parse", "HEAD") == git(repo.main, "rev-parse", "HEAD")
    assert detached(tree)
    assert (tree / "target").is_dir()
    assert repo.log.read_text().splitlines() == [
        f"bgrun -o {tree}/target/warm.log -C {tree} -- heavy -- cargo nextest run --cargo-quiet --no-run"
    ]
    assert str(tree) in done.stdout
    assert "waitone --pid 4242" in done.stdout


def test_run_from_another_worktree_it_still_starts_beside_the_main_tree(repo):
    assert repo.run("first").returncode == 0
    git(repo.main, "commit", "-q", "--allow-empty", "-m", "three")
    done = repo.run("second", tree=repo.tmp / "proj-wt-first")
    assert done.returncode == 0, done.stderr
    tree = repo.tmp / "proj-wt-second"
    assert git(tree, "rev-parse", "HEAD") == git(repo.main, "rev-parse", "HEAD")


def test_run_from_inside_another_repo_it_still_works_on_its_own(repo):
    other = repo.tmp / "other"
    other.mkdir()
    git(other, "init", "-q", "-b", "main")
    git(other, "commit", "-q", "--allow-empty", "-m", "elsewhere")
    done = repo.run("mine", cwd=other)
    assert done.returncode == 0, done.stderr
    assert (repo.tmp / "proj-wt-mine").is_dir()
    assert not (repo.tmp / "other-wt-mine").exists()


def test_at_detaches_the_new_tree_where_it_says(repo):
    done = repo.run("older", "--at", "HEAD~1")
    assert done.returncode == 0, done.stderr
    tree = repo.tmp / "proj-wt-older"
    assert git(tree, "rev-parse", "HEAD") == git(repo.main, "rev-parse", "HEAD~1")


@pytest.mark.parametrize("topic", ["taken", "a/b", ""])
def test_a_taken_path_or_an_odd_topic_is_refused_and_nothing_made(repo, topic):
    (repo.tmp / "proj-wt-taken").mkdir()
    done = repo.run(topic)
    assert done.returncode == 2, done.stdout
    assert repo.log.read_text() == ""
    assert git(repo.main, "worktree", "list").count("\n") == 0
    if topic == "taken":
        assert str(repo.tmp / "proj-wt-taken") in done.stderr
