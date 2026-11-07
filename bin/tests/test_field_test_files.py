"""Tests for bin/field-test-files, which copies the files the field-test entries'
attachment lists name, matched by SHA-256, into field-test/files/. Each runs the script
as a process over a scratch folder holding a made-up field-test folder and repos."""

import hashlib
import subprocess
from pathlib import Path

import pytest

SCRIPT = Path(__file__).resolve().parents[1] / "field-test-files"

RUN_PY = b"print('made-up analysis script')\n"
NOTES = b"made-up notes the agent wrote\n"


def sha(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def line(origin: str, data: bytes) -> str:
    return f"{origin} {len(data)} {sha(data)}\n"


@pytest.fixture
def box(tmp_path):
    """A field-test folder whose entry lists two files, a repo beside it named by the
    entry's folder, and a run function for the script."""
    entries = tmp_path / "field-test"
    (entries / "round3/demo-repo").mkdir(parents=True)
    (entries / "round3/demo-repo/an-entry.md").write_text("made-up entry\n")
    (entries / "round3/demo-repo/an-entry.attachments").write_text(
        line("repo@1f66d29b25b8:src/run.py", RUN_PY) + line("author:notes.md", NOTES)
    )
    repos = tmp_path / "repos"
    (repos / "demo-repo/moved").mkdir(parents=True)

    def run(*search):
        return subprocess.run(
            [
                str(SCRIPT),
                "--dir",
                str(entries),
                "--repos-root",
                str(repos),
                *map(str, search),
            ],
            capture_output=True,
            text=True,
            check=False,
        )

    return entries, repos, run


def test_copies_each_file_found_by_hash_wherever_it_sits(box, tmp_path):
    entries, repos, run = box
    (repos / "demo-repo/moved/run.py").write_bytes(RUN_PY)
    scratch = tmp_path / "scratch/deep"
    scratch.mkdir(parents=True)
    (scratch / "renamed.md").write_bytes(NOTES)
    done = run(tmp_path / "scratch")
    assert done.returncode == 0, done.stdout + done.stderr
    assert (entries / "files" / sha(RUN_PY)).read_bytes() == RUN_PY
    assert (entries / "files" / sha(NOTES)).read_bytes() == NOTES
    assert "2 new" in done.stdout
    assert "all 2 listed files" in done.stdout


def test_a_file_of_the_same_size_but_other_bytes_isnt_taken(box):
    entries, repos, run = box
    (repos / "demo-repo/run.py").write_bytes(RUN_PY)
    decoy = NOTES.replace(b"notes", b"lies!")
    assert len(decoy) == len(NOTES)
    (repos / "demo-repo/notes.md").write_bytes(decoy)
    done = run()
    assert done.returncode == 1, done.stdout + done.stderr
    assert not (entries / "files" / sha(NOTES)).exists()
    assert (entries / "files" / sha(RUN_PY)).exists()
    assert "1 of 2 listed files not found" in done.stdout
    assert "round3/demo-repo/an-entry.attachments: author:notes.md" in done.stdout
    assert "made-up" not in done.stdout + done.stderr


def test_git_folders_and_symlinks_are_not_searched(box, tmp_path):
    entries, repos, run = box
    (repos / "demo-repo/.git").mkdir()
    (repos / "demo-repo/.git/run.py").write_bytes(RUN_PY)
    outside = tmp_path / "outside.md"
    outside.write_bytes(NOTES)
    (repos / "demo-repo/notes.md").symlink_to(outside)
    done = run()
    assert done.returncode == 1, done.stdout + done.stderr
    assert "2 of 2 listed files not found" in done.stdout
    assert not (entries / "files").exists() or not any((entries / "files").iterdir())


def test_a_rerun_copies_nothing_new(box):
    _, repos, run = box
    (repos / "demo-repo/run.py").write_bytes(RUN_PY)
    (repos / "demo-repo/notes.md").write_bytes(NOTES)
    assert run().returncode == 0
    (repos / "demo-repo/run.py").unlink()
    again = run()
    assert again.returncode == 0, again.stdout + again.stderr
    assert "0 new, 2 already there" in again.stdout


def test_without_the_field_test_folder_it_says_so(tmp_path):
    done = subprocess.run(
        [str(SCRIPT), "--dir", str(tmp_path / "absent")],
        capture_output=True,
        text=True,
        check=False,
    )
    assert done.returncode == 2
    assert "absent" in done.stderr


def test_h_says_what_it_does():
    done = subprocess.run(
        [str(SCRIPT), "-h"], capture_output=True, text=True, check=False
    )
    assert done.returncode == 0
    assert "SHA-256" in done.stdout
    assert "field-test/files" in done.stdout
