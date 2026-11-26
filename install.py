#!/usr/bin/env python3
"""Set up a machine to work on File Drawer: point git at the hooks in .githooks,
register the merge driver for skills/versions.txt, and install the cargo tools the
checks and the SQL workflow need. sandbox-setup/setup.py runs it after a sandbox
recreate, after rustup is in place.

    python3 install.py        enable the hooks and the driver; install what's missing
    python3 install.py -h     this text

The hooks: .githooks holds a link to a script in bin/ for each hook git runs
(pre-commit; post-commit, which warns an agent when a fixup!, amend! or squash! commit
touches files its target didn't; post-merge, post-checkout and post-rewrite, which rebuild target/dev.db
when the migrations change; and pre-push, which refuses a skill version origin's tags
already hold with other contents) and each check llm-toolbox's
agent-commit-gate runs from beside pre-commit (agent-pre-commit, agent-quick-check,
agent-commit-check). core.hooksPath=.githooks is relative, so git reads it from the
top of whichever worktree runs the hook: each worktree, and each commit wt-ship checks
out as it lands, runs its own copies of the scripts. The setting goes in the config
every worktree shares, so running this from any of them enables the hooks for all.
It also removes the links an older install.py put in the shared .git/hooks, which git
no longer reads, and names any other hook left there.

The merge driver: .gitattributes names `skills-versions` for skills/versions.txt, and
git runs a driver only when the config defines it too, so this sets
merge.skills-versions.driver to run `bin/skills merge` (bin/skills -h), and
`git merge-file`, for git's ordinary conflict, when that exits non-zero or can't
run. It runs the bin/skills at the top of the tree that is merging, so, like the
hooks, each worktree runs its own copy. A tree whose .gitattributes doesn't name it,
such as one checked out before it, ignores the setting, and a clone without the
setting gets git's ordinary conflict in that file. The main clone's config is the
host's too, so the host's rebases run the driver with the host's python3.

The tools (cargo install --locked, so they build from their locked dependencies):
  cargo-nextest  runs the Rust tests in bin/agent-pre-commit and CI
  sqlx-cli       `sqlx` and `cargo sqlx prepare`, for changing a query (AGENTS.md, SQL)

Safe to rerun: hooks and a driver already set up and a tool already on PATH are left
alone.
"""

import shutil
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent
HOOKS_PATH = ".githooks"
# The git config that defines the merge driver .gitattributes names.
MERGE_DRIVER = {
    "merge.skills-versions.name": "recompute skills/versions.txt (bin/skills merge)",
    # When bin/skills can't merge, or fails to run at all, git merge-file leaves the
    # conflict git's own merge would.
    "merge.skills-versions.driver": '"$(git rev-parse --show-toplevel)/bin/skills"'
    " merge %O %A %B %S %Y || git merge-file -L %X -L %S -L %Y %A %O %B",
}
# (command that shows it is installed, cargo install arguments)
CARGO_TOOLS = [
    ("cargo-nextest", ["cargo-nextest"]),
    ("sqlx", ["sqlx-cli", "--no-default-features", "--features", "sqlite,rustls"]),
]


def git(repo: Path, *args: str, check: bool = True) -> subprocess.CompletedProcess:
    return subprocess.run(
        ["git", "-C", str(repo), *args], capture_output=True, text=True, check=check
    )


def enable_hooks(repo: Path = REPO) -> None:
    """Set core.hooksPath to HOOKS_PATH in the config `repo` shares with its other
    worktrees, then clear out the shared .git/hooks: remove each link the older
    install.py made there, `../` and the tracked link's target, and name each other
    hook left, which git no longer runs."""
    current = git(repo, "config", "--local", "core.hooksPath", check=False)
    if current.stdout.strip() != HOOKS_PATH:
        git(repo, "config", "--local", "core.hooksPath", HOOKS_PATH)
        print(f"set core.hooksPath to {HOOKS_PATH}")
    common_dir = git(repo, "rev-parse", "--path-format=absolute", "--git-common-dir")
    old_hooks = Path(common_dir.stdout.strip()) / "hooks"
    for tracked in sorted((repo / HOOKS_PATH).iterdir()):
        link = old_hooks / tracked.name
        if (
            tracked.is_symlink()
            and link.is_symlink()
            and link.readlink() == Path("..") / tracked.readlink()
        ):
            link.unlink()
            print(f"removed .git/hooks/{tracked.name}, the old link")
    for left in sorted(old_hooks.iterdir()) if old_hooks.is_dir() else []:
        if left.suffix != ".sample":
            print(
                f".git/hooks/{left.name}: git no longer runs it ({HOOKS_PATH} has the hooks)"
            )


def register_merge_driver(repo: Path = REPO) -> None:
    """Define the skills/versions.txt merge driver in the config `repo` shares with
    its other worktrees."""
    for key, value in MERGE_DRIVER.items():
        current = git(repo, "config", "--local", key, check=False)
        if current.stdout.strip() != value:
            git(repo, "config", "--local", key, value)
            print(f"set {key}")


def install_tools() -> bool:
    if shutil.which("cargo") is None:
        print("cargo isn't on PATH: install Rust first (sandbox-setup's rustup step)")
        return False
    ok = True
    for command, args in CARGO_TOOLS:
        if shutil.which(command):
            continue
        print(f"installing {args[0]} with cargo install --locked")
        if (
            subprocess.run(
                ["cargo", "install", "--locked", *args], check=False
            ).returncode
            != 0
        ):
            print(f"FAILED to install {args[0]}")
            ok = False
    return ok


def main() -> int:
    if any(arg in ("-h", "--help") for arg in sys.argv[1:]):
        print(__doc__)
        return 0
    enable_hooks()
    register_merge_driver()
    return 0 if install_tools() else 1


if __name__ == "__main__":
    sys.exit(main())
