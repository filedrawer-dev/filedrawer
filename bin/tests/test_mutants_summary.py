"""Tests for bin/mutants-summary, which .github/workflows/mutants.yml runs to merge the
shards' lists of a cargo-mutants run and write the summary. Each test lays out a
scratch download the way actions/download-artifact does and runs the script as a
process."""

import subprocess
from pathlib import Path

SCRIPT = Path(__file__).resolve().parents[1] / "mutants-summary"

# cargo-mutants --list prints one mutant a line, in source order.
ALL = """\
crates/a/src/lib.rs:9:5: replace f -> bool with true
crates/a/src/lib.rs:10:5: replace f -> bool with false
crates/a/src/lib.rs:100:9: replace == with != in g
crates/b/src/x.rs:2:1: replace h with ()
crates/b/src/x.rs:2:14: replace k with ()
crates/b/src/x.rs:7:3: replace + with - in k
"""


def run(tmp_path: Path, shards: dict[str, dict[str, str]] | None = None):
    """Lay out each shard in a folder of its own under shards/, then run the script."""
    for name, lists in (shards or {}).items():
        folder = tmp_path / "shards" / name
        (folder / "log").mkdir(parents=True)
        for list_name, text in lists.items():
            (folder / f"{list_name}.txt").write_text(text)
    (tmp_path / "all.txt").write_text(ALL)
    return subprocess.run(
        [SCRIPT, "shards", "out", "all.txt"],
        cwd=tmp_path,
        capture_output=True,
        text=True,
    )


def two_shards() -> dict[str, dict[str, str]]:
    """Every mutant, split over two shards as round-robin sharding splits them."""
    return {
        "mutants-shard-0": {
            "caught": "crates/b/src/x.rs:2:1: replace h with ()\n"
            "crates/a/src/lib.rs:9:5: replace f -> bool with true\n",
            "missed": "crates/a/src/lib.rs:100:9: replace == with != in g\n",
            "timeout": "",
            "unviable": "",
        },
        "mutants-shard-1": {
            "caught": "crates/b/src/x.rs:2:14: replace k with ()\n"
            "crates/a/src/lib.rs:10:5: replace f -> bool with false\n",
            "missed": "",
            "timeout": "crates/b/src/x.rs:7:3: replace + with - in k\n",
            "unviable": "",
        },
    }


SUMMARY_COMPLETE = """\
## Mutants

Tested 6 of 6 mutants.

| Outcome | Mutants |
| --- | ---: |
| missed | 1 |
| caught | 4 |
| timeout | 1 |
| unviable | 0 |

### Missed

```text
crates/a/src/lib.rs:100:9: replace == with != in g
```
"""


def test_every_mutant_tested_merges_the_lists_and_passes(tmp_path):
    result = run(tmp_path, two_shards())
    assert result.returncode == 0, result.stderr
    assert result.stdout == SUMMARY_COMPLETE
    out = tmp_path / "out"
    assert (out / "summary.md").read_text() == SUMMARY_COMPLETE
    # Sorted by file, then line, then column, as numbers: a sort by text would put
    # line 10 before line 9, and column 14 before column 1.
    assert (out / "caught.txt").read_text() == (
        "crates/a/src/lib.rs:9:5: replace f -> bool with true\n"
        "crates/a/src/lib.rs:10:5: replace f -> bool with false\n"
        "crates/b/src/x.rs:2:1: replace h with ()\n"
        "crates/b/src/x.rs:2:14: replace k with ()\n"
    )
    assert (out / "missed.txt").read_text() == (
        "crates/a/src/lib.rs:100:9: replace == with != in g\n"
    )
    assert (out / "timeout.txt").read_text() == (
        "crates/b/src/x.rs:7:3: replace + with - in k\n"
    )
    assert (out / "unviable.txt").read_text() == ""


def test_a_missing_shard_fails_but_still_writes_the_summary(tmp_path):
    shards = two_shards()
    del shards["mutants-shard-1"]
    result = run(tmp_path, shards)
    assert result.returncode == 1
    assert result.stderr == (
        "mutants-summary: only 3 of 6 mutants were tested; a shard failed, was "
        "cancelled or ran out of time.\n"
    )
    assert "Tested 3 of 6 mutants." in result.stdout
    assert (tmp_path / "out/summary.md").read_text() == result.stdout
    assert (tmp_path / "out/missed.txt").read_text() == (
        "crates/a/src/lib.rs:100:9: replace == with != in g\n"
    )


def test_a_single_artifact_unpacked_flat_into_shards(tmp_path):
    # download-artifact puts a lone match straight into its path, not a subfolder.
    shards = tmp_path / "shards"
    (shards / "log").mkdir(parents=True)
    for lists in two_shards().values():
        for name, text in lists.items():
            with open(shards / f"{name}.txt", "a") as merged:
                merged.write(text)
    result = run(tmp_path)
    assert result.returncode == 0, result.stderr
    assert result.stdout == SUMMARY_COMPLETE


def test_no_shards_folder_counts_nothing_tested(tmp_path):
    # download-artifact makes no folder when no shard uploaded anything.
    result = run(tmp_path)
    assert result.returncode == 1
    assert "Tested 0 of 6 mutants." in result.stdout
    assert (tmp_path / "out/missed.txt").read_text() == ""


def test_an_unreadable_mutant_list_fails_in_one_line(tmp_path):
    result = subprocess.run(
        [SCRIPT, "shards", "out", "missing.txt"],
        cwd=tmp_path,
        capture_output=True,
        text=True,
    )
    assert result.returncode == 3
    assert result.stderr.startswith("mutants-summary: can't read missing.txt: ")
    assert result.stderr.count("\n") == 1
