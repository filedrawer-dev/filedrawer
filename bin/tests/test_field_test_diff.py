"""Tests for bin/field-test-diff, which compares entry-check's outcomes on the
field-test entries with those of an entry-check built at an earlier commit, the fork
point by default. Each runs the script as a process in a scratch repo, with a fake
cargo first on PATH whose "build" copies the tree's own fake-entry-check into the target
folder, so each commit carries the entry-check it builds."""

import os
import subprocess
import time
from pathlib import Path

import pytest

SCRIPT = Path(__file__).resolve().parents[1] / "field-test-diff"

# Answers from each file's first word.
ACCEPTING = """#!/bin/bash
for f in "$@"; do
  case "$(head -c 6 "$f")" in
    ACCEPT) echo "$f: accepted, 0 warnings"; echo "card:"; echo "  SECRET CARD TEXT" ;;
    REFUSE) echo "$f: refused, 1 warning"; echo "refused: it holds a long run of base64 (starting \\`SECRETSECRET\\`)" ;;
  esac
done
"""
# Refuses every file, naming a field and quoting an author's value.
STRICT = """#!/bin/bash
for f in "$@"; do
  echo "$f: refused, 0 warnings"
  echo "refused: the required field \\`title\\` is missing"
  echo "refused: \\`embargo_until: SECRET\\` is not a date"
done
"""
FAKE_CARGO = """#!/bin/sh
echo "$PWD $*" >> "$CARGO_LOG"
[ -e fail-build ] && { echo "error: could not compile" >&2; exit 101; }
target=${CARGO_TARGET_DIR:-target}
mkdir -p "$target/debug"
cp fake-entry-check "$target/debug/entry-check"
chmod +x "$target/debug/entry-check"
"""


def git(cwd: Path, *args: str, env: dict | None = None) -> str:
    return subprocess.run(
        ["git", "-c", "user.name=t", "-c", "user.email=t@t", *args],
        cwd=cwd,
        capture_output=True,
        text=True,
        check=True,
        env=env,
    ).stdout.strip()


class Box:
    def __init__(self, root: Path):
        self.repo = root / "proj"
        (self.repo / "bin").mkdir(parents=True)
        (self.repo / "bin/field-test-diff").write_text(SCRIPT.read_text())
        (self.repo / "bin/field-test-diff").chmod(0o755)
        self.write_fake(ACCEPTING)
        git(self.repo, "init", "-q", "-b", "main")
        # field-test/ is git-excluded in the real repo.
        (self.repo / ".git/info/exclude").write_text("field-test/\ntarget/\n")
        self.commit("one")
        entries = self.repo / "field-test/round1"
        entries.mkdir(parents=True)
        (entries / "a.md").write_text("ACCEPT secret entry text\n")
        (entries / "b.md").write_text("ACCEPT other secret\n")
        fakes = root / "fakes"
        fakes.mkdir()
        (fakes / "cargo").write_text(FAKE_CARGO)
        (fakes / "cargo").chmod(0o755)
        self.log = root / "cargo-calls"
        self.env = os.environ | {
            "PATH": f"{fakes}{os.pathsep}{os.environ['PATH']}",
            "CARGO_LOG": str(self.log),
        }
        self.env.pop("CARGO_TARGET_DIR", None)

    def write_fake(self, text: str, tree: Path | None = None) -> None:
        fake = (tree or self.repo) / "fake-entry-check"
        fake.write_text(text)
        fake.chmod(0o755)

    def commit(self, message: str, tree: Path | None = None, dated: str = "") -> str:
        git(tree or self.repo, "add", "-A")
        env = os.environ | ({"GIT_COMMITTER_DATE": dated} if dated else {})
        git(tree or self.repo, "commit", "-qm", message, env=env)
        return git(tree or self.repo, "rev-parse", "HEAD")

    def run(self, *args: str, tree: Path | None = None) -> subprocess.CompletedProcess:
        return subprocess.run(
            [str((tree or self.repo) / "bin/field-test-diff"), *args],
            capture_output=True,
            text=True,
            check=False,
            env=self.env,
        )

    def base_builds(self) -> list[str]:
        calls = self.log.read_text().splitlines() if self.log.exists() else []
        return [call for call in calls if "field-test-base" in call]


@pytest.fixture
def box(tmp_path) -> Box:
    return Box(tmp_path)


def test_unchanged_since_the_fork_point_it_says_so_and_exits_0(box):
    done = box.run()
    assert done.returncode == 0, done.stdout + done.stderr
    assert "unchanged: 2 files" in done.stdout
    head = git(box.repo, "rev-parse", "--short", "HEAD")
    assert head in done.stdout  # it names the commit it compares with
    assert "Traceback" not in done.stderr


def test_a_changed_outcome_names_the_file_and_never_its_contents(box):
    box.write_fake(STRICT)  # the working tree's entry-check, against HEAD's
    done = box.run()
    assert done.returncode == 1, done.stdout + done.stderr
    assert "round1/a.md" in done.stdout and "round1/b.md" in done.stdout
    assert "accepted" in done.stdout and "refused" in done.stdout
    for secret in ("SECRET", "secret"):
        assert secret not in done.stdout + done.stderr


def test_a_reason_keeps_the_field_it_names(box):
    box.write_fake(STRICT)
    done = box.run()
    assert "the required field `title` is missing" in done.stdout
    assert "`embargo_until" in done.stdout


def test_against_compares_with_the_commit_it_names(box):
    first = git(box.repo, "rev-parse", "HEAD")
    box.write_fake(STRICT)
    box.commit("strict")
    assert box.run().returncode == 0  # on main, HEAD is the fork point
    done = box.run("--against", first)
    assert done.returncode == 1, done.stdout + done.stderr
    assert first[:7] in done.stdout


def test_an_entry_check_built_at_a_tree_is_kept(box):
    assert box.run().returncode == 0
    assert box.run().returncode == 0
    assert len(box.base_builds()) == 1


def test_the_base_holds_only_the_commits_files_each_freshly_written(box):
    (box.repo / "extra.txt").write_text("only in the first commit\n")
    first = box.commit("with extra")
    assert box.run("--against", first).returncode == 0
    (box.repo / "extra.txt").unlink()
    # Dated long ago, as an archive of the commit would date its files.
    second = box.commit("without extra", dated="2020-01-01T00:00:00")
    before = time.time()
    assert box.run("--against", second).returncode == 0
    source = box.repo / "target/field-test-base/src"
    assert sorted(p.name for p in source.iterdir()) == ["bin", "fake-entry-check"]
    # A commit's own time would make cargo take an older build for current.
    for path in source.rglob("*"):
        assert path.stat().st_mtime >= before - 1, path


def test_a_build_that_fails_says_so_in_one_line(box):
    (box.repo / "fail-build").write_text("")
    done = box.run()
    assert done.returncode == 2
    assert "Traceback" not in done.stderr
    assert len(done.stderr.strip().splitlines()) == 1, done.stderr


def test_an_unknown_commit_is_a_usage_error(box):
    done = box.run("--against", "no-such-commit")
    assert done.returncode == 2
    assert "no-such-commit" in done.stderr


def test_from_a_worktree_it_reads_the_main_trees_field_test_and_its_fork_point(box):
    fork = git(box.repo, "rev-parse", "HEAD")
    worktree = box.repo.parent / "wt"
    git(box.repo, "worktree", "add", "-q", "--detach", str(worktree))
    box.write_fake(STRICT, worktree)
    box.commit("strict, in the worktree", worktree)
    # Main moves on after the fork; the comparison stays with the fork point.
    box.write_fake(STRICT)
    box.commit("strict on main too")
    done = box.run(tree=worktree)
    assert done.returncode == 1, done.stdout + done.stderr
    assert fork[:7] in done.stdout
    assert "round1/a.md" in done.stdout
