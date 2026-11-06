#!/usr/bin/env python3
"""Set up a machine to work on File Drawer: link the git hooks and install the cargo
tools the checks and the SQL workflow need. sandbox-setup/setup.py runs it after a
sandbox recreate, after rustup is in place.

    python3 install.py        link the hooks; install what's missing
    python3 install.py -h     this text

The tools (cargo install --locked, so they build from their locked dependencies):
  cargo-nextest  runs the Rust tests in bin/agent-pre-commit and CI
  sqlx-cli       `sqlx` and `cargo sqlx prepare`, for changing a query (AGENTS.md, SQL)

Safe to rerun: a hook already linked and a tool already on PATH are left alone.
"""

import shutil
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent
HOOKS = {"pre-commit": "bin/git-pre-commit", "agent-pre-commit": "bin/agent-pre-commit"}
# (command that shows it is installed, cargo install arguments)
CARGO_TOOLS = [
    ("cargo-nextest", ["cargo-nextest"]),
    ("sqlx", ["sqlx-cli", "--no-default-features", "--features", "sqlite,rustls"]),
]


def link_hooks() -> None:
    hooks = REPO / ".git" / "hooks"
    for name, target in HOOKS.items():
        link = hooks / name
        wanted = Path("../..") / target
        if link.is_symlink() and Path(link.readlink()) == wanted:
            continue
        if link.exists() or link.is_symlink():
            link.unlink()
        link.symlink_to(wanted)
        print(f"linked .git/hooks/{name} -> {target}")


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
    link_hooks()
    return 0 if install_tools() else 1


if __name__ == "__main__":
    sys.exit(main())
