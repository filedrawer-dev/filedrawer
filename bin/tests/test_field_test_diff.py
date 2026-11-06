"""Tests for bin/field-test-diff, which compares entry-check's outcomes on the
field-test entries with a saved baseline. Each runs the script as a process with a
fake entry-check that answers from the file's first word, over a scratch folder."""

import subprocess
from pathlib import Path

import pytest

SCRIPT = Path(__file__).resolve().parents[1] / "field-test-diff"

FAKE = """#!/bin/bash
for f in "$@"; do
  case "$(head -c 6 "$f")" in
    ACCEPT) echo "$f: accepted, 0 warnings"; echo "card:"; echo "  SECRET CARD TEXT" ;;
    REFUSE) echo "$f: refused, 1 warning"; echo "refused: it holds a long run of base64 (starting \\`SECRETSECRET\\`)" ;;
  esac
done
"""


@pytest.fixture
def box(tmp_path):
    entries = tmp_path / "field-test"
    (entries / "round1").mkdir(parents=True)
    (entries / "round1/a.md").write_text("ACCEPT secret entry text\n")
    (entries / "round1/b.md").write_text("ACCEPT other secret\n")
    fake = tmp_path / "entry-check"
    fake.write_text(FAKE)
    fake.chmod(0o755)
    baseline = tmp_path / "baseline.json"

    def run(*args):
        return subprocess.run(
            [
                str(SCRIPT),
                *args,
                "--dir",
                str(entries),
                "--entry-check",
                str(fake),
                "--baseline",
                str(baseline),
            ],
            capture_output=True,
            text=True,
            check=False,
        )

    return entries, baseline, run


def test_no_change_since_the_baseline_prints_nothing_and_exits_0(box):
    entries, baseline, run = box
    assert run("save").returncode == 0
    assert baseline.exists()
    done = run()
    assert done.returncode == 0, done.stdout + done.stderr
    assert "unchanged" in done.stdout


def test_a_changed_outcome_names_the_file_and_never_its_contents(box):
    entries, baseline, run = box
    run("save")
    (entries / "round1/b.md").write_text("REFUSE now\n")
    done = run()
    assert done.returncode == 1
    assert "round1/b.md" in done.stdout
    assert "accepted" in done.stdout and "refused" in done.stdout
    assert "round1/a.md" not in done.stdout
    printed = done.stdout + done.stderr + baseline.read_text()
    for secret in ("SECRET", "secret"):
        assert secret not in printed, printed


def test_a_reason_keeps_the_field_it_names(box):
    entries, baseline, run = box
    fake = entries.parent / "entry-check"
    fake.write_text(
        '#!/bin/bash\nfor f in "$@"; do echo "$f: refused, 0 warnings";'
        ' echo "refused: the required field \\`title\\` is missing";'
        ' echo "refused: \\`embargo_until: SECRET\\` is not a date"; done\n'
    )
    run("save")
    saved = baseline.read_text()
    assert "the required field `title` is missing" in saved
    assert "`embargo_until" in saved and "SECRET" not in saved


def test_a_build_that_fails_says_so_in_one_line(box):
    entries, baseline, run = box
    done = subprocess.run(
        [str(SCRIPT), "save", "--dir", str(entries), "--entry-check", "/nonexistent"],
        capture_output=True,
        text=True,
        check=False,
    )
    assert done.returncode == 2
    assert "Traceback" not in done.stderr
    assert len(done.stderr.strip().splitlines()) == 1, done.stderr


def test_without_a_baseline_it_says_to_save_one(box):
    _, _, run = box
    done = run()
    assert done.returncode == 2
    assert "save" in done.stderr
