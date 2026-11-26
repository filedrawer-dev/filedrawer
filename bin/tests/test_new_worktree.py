"""Tests for bin/new-worktree, which adds a worktree beside the main tree and starts its
warm build, then, while it is cold, one of the commit checks' slot, in the background,
unless --no-build. Each runs a copy of the script
committed to a scratch repo, since it finds its repo from its own path, with a fake
bgrun first on PATH that notes its arguments."""

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
    done = repo.run("series")
    assert done.returncode == 0, done.stderr
    tree = repo.tmp / "proj-wt-series"
    assert git(tree, "rev-parse", "HEAD") == git(repo.main, "rev-parse", "HEAD")
    assert detached(tree)
    assert (tree / "target").is_dir()
    assert repo.log.read_text().splitlines() == [
        f"bgrun -o {tree}/target/warm.log -C {tree} -- heavy -- cargo nextest run --cargo-quiet --no-run"
    ]
    assert str(tree) in done.stdout
    assert "waitone --pid 4242" in done.stdout


def checks_script(cold: bool) -> str:
    """A bin/agent-pre-commit that takes --slot-cold, saying the slot is cold or not."""
    return (
        "#!/bin/sh\n"
        'case "$1" in --slot-cold) echo /the/slot; '
        f"exit {0 if cold else 1} ;; --warm) ;; esac\n"
    )


def test_a_second_job_warms_the_check_slot_only_while_it_is_cold(repo):
    # The commit checks build in a check slot of their own (bin/agent-pre-commit),
    # which a build of the tree alone leaves cold for the first commit and landing.
    # A copy whose --warm predates the slot would warm the tree's own target from its
    # check folder, the leak the slot ends (#146); one with neither would take
    # --warm for the full check.
    check = repo.main / "bin/agent-pre-commit"
    for message, text in (
        ("no warm", "#!/bin/sh\ncase x in --quick) ;; esac\n"),
        ("old warm", "#!/bin/sh\ncase x in --warm) ;; esac\n"),
        ("slot, warm", checks_script(cold=False)),
        ("slot, cold", checks_script(cold=True)),
    ):
        check.write_text(text)
        check.chmod(0o755)
        git(repo.main, "add", "bin")
        git(repo.main, "commit", "-q", "-m", message)
    trees = (
        ("cold", "HEAD"),
        ("warm", "HEAD~1"),
        ("old", "HEAD~2"),
        ("none", "HEAD~3"),
    )
    for topic, at in trees:
        done = repo.run(topic, "--at", at)
        assert done.returncode == 0, done.stderr
        assert "waitone --pid 4242" in done.stdout
        # Only a tree that has the slot and finds it built says it is warm.
        assert ("is warm" in done.stdout) == (topic == "warm"), done.stdout
    cold, warm, old, none = (repo.tmp / f"proj-wt-{t}" for t, _ in trees)
    build = "-- heavy -- cargo nextest run --cargo-quiet --no-run"
    assert repo.log.read_text().splitlines() == [
        f"bgrun -o {cold}/target/warm.log -C {cold} {build}",
        f"bgrun -o {cold}/target/warm-checks.log -C {cold} -- heavy -- bin/agent-pre-commit --warm",
        f"bgrun -o {warm}/target/warm.log -C {warm} {build}",
        f"bgrun -o {old}/target/warm.log -C {old} {build}",
        f"bgrun -o {none}/target/warm.log -C {none} {build}",
    ]


def test_with_the_slot_warm_it_says_so(repo):
    check = repo.main / "bin/agent-pre-commit"
    check.write_text(checks_script(cold=False))
    check.chmod(0o755)
    git(repo.main, "add", "bin")
    git(repo.main, "commit", "-q", "-m", "slot")

    done = repo.run("t")

    assert done.returncode == 0, done.stderr
    assert "/the/slot is warm" in done.stdout


def test_no_build_makes_the_tree_and_starts_no_build(repo):
    done = repo.run("docs", "--no-build")
    assert done.returncode == 0, done.stderr
    tree = repo.tmp / "proj-wt-docs"
    assert git(tree, "rev-parse", "HEAD") == git(repo.main, "rev-parse", "HEAD")
    assert detached(tree)
    assert repo.log.read_text() == ""
    assert not (tree / "target").exists()
    assert str(tree) in done.stdout
    assert "waitone" not in done.stdout


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
