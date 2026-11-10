"""Tests for bin/mutants-shards, which sizes the shard matrix of
.github/workflows/mutants.yml from the setup job's list of mutants."""

import json
import subprocess
from pathlib import Path

import pytest

SCRIPT = Path(__file__).resolve().parents[1] / "mutants-shards"


def shards(tmp_path: Path, mutants: int, *args: str) -> list[int]:
    listing = tmp_path / "all-mutants.txt"
    listing.write_text(
        "".join(f"crates/a/src/lib.rs:{n}:1: replace f\n" for n in range(mutants))
    )
    done = subprocess.run(
        [SCRIPT, str(listing), *args], capture_output=True, text=True, check=False
    )
    assert done.returncode == 0, done.stderr
    return json.loads(done.stdout)


@pytest.mark.parametrize(
    ("mutants", "expected"),
    [(1, [0]), (40, [0]), (41, [0, 1]), (120, [0, 1, 2]), (121, [0, 1, 2, 3])],
)
def test_a_daily_run_gets_a_shard_per_40_mutants(tmp_path, mutants, expected):
    assert shards(tmp_path, mutants, "--daily") == expected


def test_a_daily_run_never_takes_more_than_4_shards(tmp_path):
    assert shards(tmp_path, 2300, "--daily") == [0, 1, 2, 3]


def test_a_daily_run_with_no_mutants_gets_no_shards(tmp_path):
    assert shards(tmp_path, 0, "--daily") == []


@pytest.mark.parametrize("mutants", [5, 2300])
def test_a_full_run_always_takes_32_shards(tmp_path, mutants):
    assert shards(tmp_path, mutants) == list(range(32))


def test_an_unreadable_list_fails_in_one_line(tmp_path):
    done = subprocess.run(
        [SCRIPT, str(tmp_path / "missing.txt")],
        capture_output=True,
        text=True,
        check=False,
    )
    assert done.returncode == 3
    assert done.stderr.startswith("mutants-shards: can't read ")
    assert done.stderr.count("\n") == 1
