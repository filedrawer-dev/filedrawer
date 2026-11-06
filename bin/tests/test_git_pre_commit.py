"""Tests for bin/git-pre-commit's formatting of an agent's staged Rust. Each runs the
hook in a scratch repo holding a one-file crate, with bin/pre-commit-run stubbed to
pass, so only the formatting step acts. It needs cargo and rustfmt, which every
machine that builds this repo has.
"""

import os
import shutil
import subprocess
from pathlib import Path

import pytest

HOOK = Path(__file__).resolve().parents[1] / "git-pre-commit"
UGLY = "pub fn a( ){}\n"
TIDY = "pub fn a() {}\n"


def git(repo: Path, *args: str) -> str:
    done = subprocess.run(
        ["git", "-C", str(repo), *args], capture_output=True, text=True, check=True
    )
    return done.stdout


@pytest.fixture
def repo(tmp_path):
    if shutil.which("cargo") is None:
        pytest.skip("needs cargo")
    root = tmp_path / "repo"
    (root / "src").mkdir(parents=True)
    (root / "bin").mkdir()
    (root / "Cargo.toml").write_text(
        '[package]\nname = "x"\nversion = "0.1.0"\nedition = "2024"\n'
    )
    (root / "src/lib.rs").write_text(TIDY)
    shutil.copy(HOOK, root / "bin/git-pre-commit")
    stub = root / "bin/pre-commit-run"
    stub.write_text("#!/bin/sh\nexit 0\n")
    stub.chmod(0o755)
    git(root, "init", "-q")
    git(root, "-c", "user.name=t", "-c", "user.email=t@t", "add", ".")
    git(root, "-c", "user.name=t", "-c", "user.email=t@t", "commit", "-qm", "init")
    return root


def hook(repo: Path, agent: bool) -> subprocess.CompletedProcess:
    env = {k: v for k, v in os.environ.items() if not k.startswith("GIT_")}
    env.pop("AI_AGENT", None)
    env["AGENT_PRE_COMMIT"] = "1"
    if agent:
        env["AI_AGENT"] = "test"
    return subprocess.run(
        [str(repo / "bin/git-pre-commit")],
        cwd=repo,
        env=env,
        capture_output=True,
        text=True,
    )


def test_an_agents_staged_rust_is_formatted_and_staged(repo):
    (repo / "src/lib.rs").write_text(UGLY)
    git(repo, "add", "src/lib.rs")
    done = hook(repo, agent=True)
    assert done.returncode == 0, done.stderr
    assert (repo / "src/lib.rs").read_text() == TIDY
    assert git(repo, "show", ":src/lib.rs") == TIDY, "the formatted file is staged"


def test_a_file_with_unstaged_changes_is_left_alone(repo):
    (repo / "src/lib.rs").write_text(UGLY)
    git(repo, "add", "src/lib.rs")
    (repo / "src/lib.rs").write_text(UGLY + "// not for this commit\n")
    hook(repo, agent=True)
    assert (repo / "src/lib.rs").read_text() == UGLY + "// not for this commit\n"
    assert git(repo, "show", ":src/lib.rs") == UGLY


def test_a_persons_commit_is_only_checked(repo):
    (repo / "src/lib.rs").write_text(UGLY)
    git(repo, "add", "src/lib.rs")
    hook(repo, agent=False)
    assert git(repo, "show", ":src/lib.rs") == UGLY


def test_nothing_is_formatted_when_an_unstaged_file_needs_it(repo):
    (repo / "src/lib.rs").write_text(UGLY + "mod other;\n")
    (repo / "src/other.rs").write_text(UGLY)
    git(repo, "add", "src/lib.rs")
    hook(repo, agent=True)
    assert (repo / "src/other.rs").read_text() == UGLY
    assert git(repo, "show", ":src/lib.rs") == UGLY + "mod other;\n"
