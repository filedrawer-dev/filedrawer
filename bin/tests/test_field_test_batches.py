"""Tests for bin/field-test-batches, which turns a field-test family into the bodies of
POST /entries/batch. Each runs the script as a process on a made-up family in a scratch
folder."""

import json
import subprocess
from pathlib import Path

import pytest

SCRIPT = Path(__file__).resolve().parents[1] / "field-test-batches"

ALPHA = """---
title: Made-up alpha
slug: alpha
uses_runs:
  - run-a
series: ser_madeUp1@2#setup
tags: [made-up]
---
Made-up body that cites slug:beta.
"""

BETA = """---
title: Made-up beta
slug: beta
uses_runs: [run-a, run-b]
replicates: slug:alpha
series: ser_madeUp1@1
tags: [made-up]
---
Made-up beta body.
"""

SERIES = """---
title: Made-up series
# its made-up id: ser_madeUp1
---
Made-up shared setup.

## Setup

Made-up setup.
"""

RUNS = """runs:
  - id: run-a
    description: Made-up run A.
    n: 10
  - id: run-b
    description: |
      Made-up run B,
      over two lines.
    n: 20
"""


@pytest.fixture
def family(tmp_path):
    """A made-up family: two entries, a series, a runs file, notes that aren't an
    entry, and an attachment list beside the second entry."""
    folder = tmp_path / "field-test/round3/demo-repo"
    folder.mkdir(parents=True)
    (folder / "a.md").write_text(ALPHA)
    (folder / "b.md").write_text(BETA)
    (folder / "b.attachments").write_text("author:run.py 3 " + "0" * 64 + "\n")
    (folder / "series-demo.md").write_text(SERIES)
    (folder / "runs.yaml").write_text(RUNS)
    (folder / "FINDINGS.md").write_text("Made-up notes, not an entry.\n")
    return folder


def run(*args):
    return subprocess.run(
        [str(SCRIPT), *map(str, args)],
        capture_output=True,
        text=True,
        check=False,
        timeout=60,
    )


def batch(out: Path, k: int) -> tuple[dict, list[str]]:
    body = json.loads((out / f"batch-{k}.json").read_text())
    members = (out / f"batch-{k}.members").read_text().splitlines()
    return body, members


def test_needs_lists_the_series_and_attachment_lists_a_family_cites(family):
    done = run("needs", family)
    assert done.returncode == 0, done.stderr
    assert done.stdout.splitlines() == [
        f"series\tser_madeUp1\t{family / 'series-demo.md'}",
        f"list\t{family / 'b.attachments'}",
    ]


def test_series_body_keeps_only_the_title_and_the_description(family):
    done = run("series-body", family / "series-demo.md")
    assert done.returncode == 0, done.stderr
    assert done.stdout == (
        "---\ntitle: Made-up series\n---\n"
        "Made-up shared setup.\n\n## Setup\n\nMade-up setup.\n"
    )


def test_build_rewrites_slugs_runs_series_tags_and_attachments(family, tmp_path):
    out = tmp_path / "out"
    done = run(
        "build",
        family,
        "--tag",
        "T",
        "--out",
        out,
        "--series",
        "ser_madeUp1=real-1",
        "--set",
        f"{family / 'b.attachments'}=as-xyz",
    )
    assert done.returncode == 0, done.stderr
    body, members = batch(out, 1)
    assert not (out / "batch-2.json").exists()
    assert members == [str(family / "a.md"), str(family / "b.md")]
    assert body["entries"] == [
        """---
title: Made-up alpha
slug: T-alpha
uses_runs:
  - T-run-a
series: real-1@1#setup
tags: [screen-test, T]
---
Made-up body that cites slug:T-beta.
""",
        """---
attachments: as-xyz
title: Made-up beta
slug: T-beta
uses_runs: [T-run-a, T-run-b]
replicates: slug:T-alpha
series: real-1@1
tags: [screen-test, T]
---
Made-up beta body.
""",
    ]
    assert body["runs"] == [
        {"id": "T-run-a", "description": "Made-up run A.", "n": 10},
        {"id": "T-run-b", "description": "Made-up run B,\nover two lines.\n", "n": 20},
    ]
    assert "Made-up" not in done.stdout + done.stderr
    assert "1 batch" in done.stdout


def test_a_series_eval_didnt_make_is_left_out(family, tmp_path):
    out = tmp_path / "out"
    assert run("build", family, "--tag", "T", "--out", out).returncode == 0
    body, _ = batch(out, 1)
    for entry in body["entries"]:
        assert "series:" not in entry


def test_a_family_past_the_size_splits_with_its_runs_declared_first(family, tmp_path):
    out = tmp_path / "out"
    # Room for the runs and one entry (about 300 bytes), not two (about 450).
    done = run("build", family, "--tag", "T", "--out", out, "--max-bytes", 380)
    assert done.returncode == 0, done.stderr
    first, first_members = batch(out, 1)
    second, second_members = batch(out, 2)
    assert first_members == [str(family / "a.md")]
    assert second_members == [str(family / "b.md")]
    assert [run["id"] for run in first["runs"]] == ["T-run-a", "T-run-b"]
    assert second["runs"] == []
    assert "2 batches" in done.stdout


def test_a_batch_holds_at_most_100_entries(tmp_path):
    folder = tmp_path / "field-test/round2/big"
    folder.mkdir(parents=True)
    for n in range(101):
        (folder / f"e{n:03}.md").write_text(f"---\ntitle: Made-up {n}\n---\nBody.\n")
    out = tmp_path / "out"
    assert run("build", folder, "--tag", "T", "--out", out).returncode == 0
    assert len(batch(out, 1)[1]) == 100
    assert batch(out, 2)[1] == [str(folder / "e100.md")]


def test_h_says_what_it_does():
    done = run("-h")
    assert done.returncode == 0
    assert "POST /entries/batch" in done.stdout
    assert "needs" in done.stdout and "series-body" in done.stdout


def test_a_family_with_no_entries_makes_no_batch(tmp_path):
    folder = tmp_path / "field-test/round2/empty"
    folder.mkdir(parents=True)
    (folder / "FINDINGS.md").write_text("Made-up notes.\n")
    out = tmp_path / "out"
    done = run("build", folder, "--tag", "T", "--out", out)
    assert done.returncode == 0, done.stderr
    assert not (out / "batch-1.json").exists()


def test_tags_and_runs_written_as_block_lists_are_rewritten_whole(tmp_path):
    folder = tmp_path / "field-test/round2/blocks"
    folder.mkdir(parents=True)
    (folder / "e.md").write_text(
        '---\ntitle: Made-up\ntags:\n  - made-up\n  - other\nuses_runs:\n  - "run-a"\n'
        "slug: e\n---\nBody.\n"
    )
    out = tmp_path / "out"
    assert run("build", folder, "--tag", "T", "--out", out).returncode == 0
    assert batch(out, 1)[0]["entries"] == [
        "---\ntitle: Made-up\ntags: [screen-test, T]\nuses_runs:\n  - T-run-a\n"
        "slug: T-e\n---\nBody.\n"
    ]


def test_an_entry_with_windows_line_ends_or_a_bom_is_still_an_entry(tmp_path):
    folder = tmp_path / "field-test/round2/crlf"
    folder.mkdir(parents=True)
    (folder / "crlf.md").write_bytes(
        b"---\r\ntitle: Made-up\r\nslug: c\r\n---\r\nBody.\r\n"
    )
    (folder / "bom.md").write_bytes(
        "﻿---\ntitle: Made-up\nslug: b\n---\nBody.\n".encode()
    )
    out = tmp_path / "out"
    assert run("build", folder, "--tag", "T", "--out", out).returncode == 0
    body, members = batch(out, 1)
    assert members == [str(folder / "bom.md"), str(folder / "crlf.md")]
    assert body["entries"] == [
        "---\ntitle: Made-up\nslug: T-b\n---\nBody.\n",
        "---\ntitle: Made-up\nslug: T-c\n---\nBody.\n",
    ]


@pytest.mark.parametrize(
    "runs",
    [
        "runs:\n  - id: [made-up\n",
        "runs:\n  - description: SECRET made-up run with no id\n    n: 1\n",
        "",
        "- id: run-a\n",
    ],
)
def test_a_runs_file_it_cant_read_fails_in_one_line_naming_it(family, tmp_path, runs):
    (family / "runs.yaml").write_text(runs)
    done = run("build", family, "--tag", "T", "--out", tmp_path / "out")
    assert done.returncode == 2, done.stderr
    assert done.stderr.count("\n") == 1, done.stderr
    assert "runs.yaml" in done.stderr
    assert "made-up" not in done.stderr and "SECRET" not in done.stderr


def test_needs_picks_the_series_whose_id_field_names_the_id(family):
    # Another series mentions the made-up ID in passing; the one whose `id` is it wins.
    (family / "series-aaa.md").write_text(
        "---\ntitle: Made-up other\n---\nMade-up text that names ser_madeUp1.\n"
    )
    (family / "series-zzz.md").write_text(
        "---\nid: ser_madeUp1\nversion: 2\ntitle: Made-up own\n---\nMade-up text.\n"
    )
    (family / "series-demo.md").unlink()
    done = run("needs", family)
    assert (
        f"series\tser_madeUp1\t{family / 'series-zzz.md'}" in done.stdout.splitlines()
    )
