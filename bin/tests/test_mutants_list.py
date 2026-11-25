"""Tests for bin/mutants-list, the setup job's list step in
.github/workflows/mutants.yml. Each test runs the script as a process in a copy of a
scratch git repo, with a fake cargo first on PATH that records its arguments and
prints as many mutants as FAKE_MUTANTS says."""

import json
import os
import shutil
import subprocess
from pathlib import Path

import pytest

SCRIPT = Path(__file__).resolve().parents[1] / "mutants-list"

FAKE_CARGO = """\
#!/bin/sh
printf '%s\\n' "$@" > "$FAKE_CARGO_ARGS"
echo "cargo says: nothing" >&2
i=0
while [ "$i" -lt "${FAKE_MUTANTS:-0}" ]; do
  echo "crates/a/src/lib.rs:$i:1: replace f with ()"
  i=$((i + 1))
done
exit "${FAKE_CARGO_EXIT:-0}"
"""


def git(repo: Path, *args: str) -> str:
    return subprocess.run(
        ["git", "-C", str(repo), *args],
        capture_output=True,
        text=True,
        check=True,
    ).stdout.strip()


@pytest.fixture(scope="module")
def template(tmp_path_factory) -> tuple[Path, str]:
    """A repo whose second commit moves one file and changes another; returns it and
    the first commit."""
    repo = tmp_path_factory.mktemp("template") / "repo"
    repo.mkdir()
    git(repo, "init", "-q")
    git(repo, "config", "user.email", "test@example.com")
    git(repo, "config", "user.name", "Test")
    (repo / "moved.rs").write_text("fn moved() {}\n" * 5)
    (repo / "changed.rs").write_text("fn a() {}\n")
    git(repo, "add", ".")
    git(repo, "commit", "-q", "-m", "base")
    base = git(repo, "rev-parse", "HEAD")
    git(repo, "mv", "moved.rs", "elsewhere.rs")
    (repo / "changed.rs").write_text("fn a() {}\nfn b() {}\n")
    git(repo, "add", ".")
    git(repo, "commit", "-q", "-m", "head")
    return repo, base


@pytest.fixture
def repo(template, tmp_path, monkeypatch) -> tuple[Path, str]:
    source, base = template
    copy = tmp_path / "repo"
    shutil.copytree(source, copy)
    bin_dir = tmp_path / "fake-bin"
    bin_dir.mkdir()
    cargo = bin_dir / "cargo"
    cargo.write_text(FAKE_CARGO)
    cargo.chmod(0o755)
    monkeypatch.setenv("PATH", f"{bin_dir}:{os.environ['PATH']}")
    monkeypatch.setenv("FAKE_CARGO_ARGS", str(tmp_path / "cargo-args"))
    return copy, base


def run(repo: Path, *args: str, mutants: int = 0, cargo_exit: int = 0):
    env_args = {"FAKE_MUTANTS": str(mutants), "FAKE_CARGO_EXIT": str(cargo_exit)}
    return subprocess.run(
        [SCRIPT, "list", *args],
        cwd=repo,
        capture_output=True,
        text=True,
        check=False,
        env={**os.environ, **env_args},
    )


def outputs(stdout: str) -> dict[str, str]:
    return dict(line.split("=", 1) for line in stdout.splitlines())


def cargo_args(repo: Path) -> list[str]:
    return (repo.parent / "cargo-args").read_text().splitlines()


def test_a_full_run_lists_every_mutant_over_32_shards(repo, tmp_path):
    path, _ = repo
    summary = tmp_path / "summary.md"
    summary.write_text("before\n")
    done = run(
        path, "--daily", "false", "--base", "", "--summary", str(summary), mutants=3
    )
    assert done.returncode == 0, done.stderr
    assert outputs(done.stdout) == {
        "mutants": "3",
        "shards": json.dumps(list(range(32))),
    }
    assert cargo_args(path) == ["mutants", "--list"]
    assert len((path / "list/all-mutants.txt").read_text().splitlines()) == 3
    assert not (path / "list/in-diff.patch").exists()
    # A full run's summary is the summary job's alone.
    assert summary.read_text() == "before\n"


def test_a_daily_run_lists_the_mutants_in_the_lines_changed_since_base(repo):
    path, base = repo
    done = run(path, "--daily", "true", "--base", base, mutants=41)
    assert done.returncode == 0, done.stderr
    assert outputs(done.stdout) == {"mutants": "41", "shards": "[0, 1]"}
    assert cargo_args(path) == ["mutants", "--list", "--in-diff", "list/in-diff.patch"]
    patch = (path / "list/in-diff.patch").read_text()
    assert "+fn b() {}" in patch


def test_a_daily_run_counts_a_moved_file_as_all_changed(repo):
    # cargo-mutants reads only a diff's added lines, so a rename would hide them.
    path, base = repo
    run(path, "--daily", "true", "--base", base)
    patch = (path / "list/in-diff.patch").read_text()
    assert "rename from" not in patch
    assert "new file mode" in patch
    assert patch.count("+fn moved() {}") == 5


def test_a_daily_run_adds_its_count_to_the_summary(repo, tmp_path):
    path, base = repo
    summary = tmp_path / "summary.md"
    summary.write_text("before\n")
    done = run(
        path, "--daily", "true", "--base", base, "--summary", str(summary), mutants=7
    )
    assert done.returncode == 0, done.stderr
    assert (
        summary.read_text() == f"before\n7 mutants in the lines changed since {base}.\n"
    )


def test_a_daily_run_with_no_mutants_takes_no_shards(repo):
    path, base = repo
    done = run(path, "--daily", "true", "--base", base)
    assert done.returncode == 0, done.stderr
    assert outputs(done.stdout) == {"mutants": "0", "shards": "[]"}


def test_the_log_shows_cargo_and_then_says_what_it_found(repo):
    path, base = repo
    done = run(path, "--daily", "true", "--base", base, mutants=41)
    assert done.stderr == (
        "cargo says: nothing\n"
        f"41 mutants in the lines changed since {base}, in 2 shards.\n"
    )


def test_a_failed_listing_fails_and_writes_no_output(repo):
    path, _ = repo
    done = run(path, "--daily", "false", mutants=2, cargo_exit=4)
    assert done.returncode == 1
    assert done.stdout == ""
    assert done.stderr == (
        "cargo says: nothing\nmutants-list: cargo mutants --list exited 4.\n"
    )


def test_a_base_git_cant_diff_fails(repo):
    path, _ = repo
    done = run(path, "--daily", "true", "--base", "0" * 40)
    assert done.returncode == 1
    assert done.stdout == ""
    assert done.stderr.endswith(
        f"\nmutants-list: git diff against {'0' * 40} exited 128.\n"
    )


@pytest.mark.parametrize(
    "args",
    [
        ["--daily", "true"],
        ["--daily", "true", "--base", ""],
        ["--daily", "false", "--base", "abc123"],
        ["--daily", "yes"],
        [],
    ],
)
def test_a_base_must_come_with_a_daily_run_and_only_then(repo, args):
    path, _ = repo
    done = run(path, *args)
    assert done.returncode == 2
    assert done.stdout == ""
    assert not (path / "list").exists()


def test_an_existing_out_folder_fails(repo):
    path, _ = repo
    (path / "list").mkdir()
    done = run(path, "--daily", "false")
    assert done.returncode == 1
    assert done.stderr.startswith("mutants-list: ")
