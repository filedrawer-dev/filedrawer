"""install.py points git at the hooks tracked in .githooks, from the main tree or from a
worktree, whose `.git` is a file naming the shared git directory, so each worktree's
commits run that worktree's own hook scripts."""

import importlib.util
import os
import subprocess
from pathlib import Path

import pytest

REPO = Path(__file__).resolve().parents[2]
# What git runs, and what llm-toolbox's agent-commit-gate looks for beside pre-commit.
HOOK_NAMES = (
    "pre-commit",
    "post-merge",
    "post-checkout",
    "post-rewrite",
    "pre-push",
    "agent-pre-commit",
    "agent-quick-check",
    "agent-commit-check",
)
# The hook a test repo keeps: it notes which tree's copy ran.
NOTE_TREE = '#!/bin/sh\necho {tree} >> "$HOOK_RAN"\n'


def install_module():
    spec = importlib.util.spec_from_file_location("install", REPO / "install.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def git(cwd, *args, env=None):
    return subprocess.run(
        # CI's runners have no git identity of their own.
        ["git", "-c", "user.name=t", "-c", "user.email=t@t", "-C", str(cwd), *args],
        check=True,
        capture_output=True,
        text=True,
        env=env,
    ).stdout.strip()


@pytest.fixture
def trees(tmp_path):
    """A main tree whose pre-commit hook is linked the way the old install.py linked
    it, from .git/hooks into bin/, and a worktree beside it."""
    main = tmp_path / "repo"
    (main / "bin").mkdir(parents=True)
    script = main / "bin" / "git-pre-commit"
    script.write_text(NOTE_TREE.format(tree="main"))
    script.chmod(0o755)
    (main / ".githooks").mkdir()
    (main / ".githooks" / "pre-commit").symlink_to("../bin/git-pre-commit")
    git(main, "init", "-q")
    git(main, "add", "-A")
    git(main, "commit", "-q", "-m", "start")
    (main / ".git" / "hooks" / "pre-commit").symlink_to("../../bin/git-pre-commit")
    worktree = tmp_path / "repo-wt-topic"
    git(main, "worktree", "add", "-q", "--detach", str(worktree))
    return main, worktree


def test_a_commit_in_a_worktree_runs_that_worktrees_own_hook(trees, tmp_path):
    main, worktree = trees
    (worktree / "bin" / "git-pre-commit").write_text(NOTE_TREE.format(tree="worktree"))
    git(worktree, "add", "-A")
    ran = tmp_path / "ran"

    install_module().enable_hooks(worktree)
    git(worktree, "commit", "-q", "-m", "edit", env=os.environ | {"HOOK_RAN": str(ran)})

    assert ran.read_text() == "worktree\n"
    assert git(main, "config", "core.hooksPath") == ".githooks"


def test_the_old_links_go_and_other_hooks_stay(trees, capsys):
    main, worktree = trees
    hooks = main / ".git" / "hooks"
    own = hooks / "post-commit"
    own.write_text("#!/bin/sh\n")
    own.chmod(0o755)
    # Not every git install writes its sample hooks.
    (hooks / "pre-commit.sample").write_text("#!/bin/sh\n")

    install_module().enable_hooks(worktree)

    assert not (hooks / "pre-commit").is_symlink()
    assert own.is_file()
    assert (hooks / "pre-commit.sample").is_file()
    said = capsys.readouterr().out
    assert ".git/hooks/post-commit" in said
    assert "pre-commit.sample" not in said


@pytest.mark.parametrize("made", ["a file", "a link elsewhere"])
def test_a_hook_install_py_never_made_stays_though_named_as_a_tracked_one(
    trees, made, capsys
):
    main, worktree = trees
    hook = main / ".git" / "hooks" / "pre-commit"
    hook.unlink()
    if made == "a file":
        hook.write_text("#!/bin/sh\n")
    else:
        hook.symlink_to("../../bin/other-pre-commit")

    install_module().enable_hooks(worktree)

    assert hook.is_symlink() or hook.is_file()
    assert ".git/hooks/pre-commit" in capsys.readouterr().out


def test_every_hook_the_tools_look_for_links_to_a_script_in_bin():
    hooks = REPO / ".githooks"
    for name in HOOK_NAMES:
        link = hooks / name
        assert link.is_symlink(), name
        assert link.readlink().parts[:2] == ("..", "bin"), name
        script = link.resolve()
        assert script.parent == REPO / "bin", name
        assert script.is_file() and os.access(script, os.X_OK), name
