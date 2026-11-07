"""bin/agent-quick-check: the check llm-toolbox's agent-commit-gate runs for an agent's
small fix commit in place of agent-pre-commit. It runs clippy on what's staged."""

import os
import shutil
import subprocess
from pathlib import Path

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
    for script in ("agent-quick-check", "agent-pre-commit"):
        shutil.copy(REPO / "bin" / script, repo / "bin" / script)
    git(repo, "init", "-q")
    git(repo, "add", "-A")
    return repo


def quick_check(repo):
    return subprocess.run(
        [str(repo / "bin/agent-quick-check")],
        cwd=repo,
        env={k: v for k, v in os.environ.items() if not k.startswith("GIT_")},
        capture_output=True,
        text=True,
        timeout=100,
        check=False,
    )


def test_a_staged_clippy_lint_fails_the_quick_check(tmp_path):
    done = quick_check(scratch_crate(tmp_path, LINT))
    assert done.returncode != 0, done.stdout + done.stderr
    assert "bool_comparison" in done.stdout + done.stderr, done.stdout + done.stderr


def test_a_clean_tree_passes_and_only_what_is_staged_counts(tmp_path):
    repo = scratch_crate(tmp_path, CLEAN)
    # An unstaged lint is no part of the commit.
    (repo / "src/lib.rs").write_text(LINT)
    done = quick_check(repo)
    assert done.returncode == 0, done.stdout + done.stderr
