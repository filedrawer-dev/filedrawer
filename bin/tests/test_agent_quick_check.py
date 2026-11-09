"""bin/agent-quick-check and bin/agent-commit-check: the checks llm-toolbox's
agent-commit-gate runs in place of agent-pre-commit, the first for an agent's small fix
commit, the second for any other agent commit. Each runs clippy on what's staged."""

import os
import shutil
import subprocess
from pathlib import Path

import pytest

REPO = Path(__file__).resolve().parents[2]

CLEAN = "pub fn on(flag: bool) -> bool {\n    flag\n}\n"
# clippy's bool_comparison, an error under -D warnings.
LINT = "pub fn on(flag: bool) -> bool {\n    flag == true\n}\n"


def git(repo, *args):
    subprocess.run(["git", "-C", str(repo), *args], check=True, capture_output=True)


def scratch_crate(tmp_path, lib):
    """A git repo holding a crate whose staged lib.rs is `lib`, and the quick check."""
    repo = tmp_path / "repo"
    (repo / "src").mkdir(parents=True)
    (repo / "bin").mkdir()
    (repo / "Cargo.toml").write_text(
        '[package]\nname = "quick"\nversion = "0.1.0"\nedition = "2024"\n'
    )
    (repo / "src/lib.rs").write_text(lib)
    for script in ("agent-quick-check", "agent-commit-check", "agent-pre-commit"):
        shutil.copy(REPO / "bin" / script, repo / "bin" / script)
    git(repo, "init", "-q")
    git(repo, "add", "-A")
    return repo


CHECKS = pytest.mark.parametrize("name", ["agent-quick-check", "agent-commit-check"])


def run_check(repo, name):
    return subprocess.run(
        [str(repo / "bin" / name)],
        cwd=repo,
        env={k: v for k, v in os.environ.items() if not k.startswith("GIT_")},
        capture_output=True,
        text=True,
        check=False,
    )


@CHECKS
def test_a_staged_clippy_lint_fails_the_check(tmp_path, name):
    done = run_check(scratch_crate(tmp_path, LINT), name)
    assert done.returncode != 0, done.stdout + done.stderr
    assert "bool_comparison" in done.stdout + done.stderr, done.stdout + done.stderr


@CHECKS
def test_a_clean_tree_passes_and_only_what_is_staged_counts(tmp_path, name):
    repo = scratch_crate(tmp_path, CLEAN)
    # An unstaged lint is no part of the commit.
    (repo / "src/lib.rs").write_text(LINT)
    done = run_check(repo, name)
    assert done.returncode == 0, done.stdout + done.stderr
