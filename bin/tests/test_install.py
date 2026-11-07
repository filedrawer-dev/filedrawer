"""install.py links the git hooks, from the main tree or from a worktree, whose `.git`
is a file naming the shared git directory."""

import importlib.util
import subprocess
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]


def install_module():
    spec = importlib.util.spec_from_file_location("install", REPO / "install.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def git(cwd, *args):
    subprocess.run(["git", "-C", str(cwd), *args], check=True, capture_output=True)


def test_hooks_link_into_the_shared_git_dir_from_a_worktree(tmp_path):
    main = tmp_path / "repo"
    main.mkdir()
    git(main, "init", "-q")
    # CI's runners have no git identity of their own.
    git(
        main,
        "-c",
        "user.name=t",
        "-c",
        "user.email=t@t",
        "commit",
        "-q",
        "--allow-empty",
        "-m",
        "start",
    )
    worktree = tmp_path / "repo-wt-topic"
    git(main, "worktree", "add", "-q", "--detach", str(worktree))
    install = install_module()

    install.link_hooks(worktree)

    for name, target in install.HOOKS.items():
        link = main / ".git" / "hooks" / name
        assert link.is_symlink(), name
        assert link.readlink() == Path("../..") / target, name
