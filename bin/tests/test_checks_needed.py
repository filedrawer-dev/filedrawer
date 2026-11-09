"""bin/checks-needed on this repo's own tree: which checks a change to each path needs,
from the references the crates and bin/tests make today. A change that adds or drops
such a reference updates this list. bin/tests/test_agent_pre_commit.py covers each
kind of reference in a scratch workspace."""

import subprocess
from pathlib import Path

import pytest

REPO = Path(__file__).resolve().parents[2]
SCRIPT = REPO / "bin/checks-needed"


def checks_needed(*paths: str) -> subprocess.CompletedProcess:
    done = subprocess.run(
        [str(SCRIPT), str(REPO)],
        input="\0".join(paths),
        capture_output=True,
        text=True,
        check=False,
    )
    assert done.returncode == 0, done.stdout + done.stderr
    return done


def needed(*paths: str) -> set[str]:
    return {line.split(" ", 1)[0] for line in checks_needed(*paths).stdout.splitlines()}


def test_the_other_docs_need_no_check():
    assert (
        needed("docs/design.md", "README.md", "AGENTS.md", "docs/field-test.md")
        == set()
    )


@pytest.mark.parametrize(
    "path, checks",
    [
        # include_str! from crates/server/src/routes/docs.rs
        ("docs/entry-format.md", {"rust"}),
        # crates/server/tests/smoke.rs runs it and reads these at run time
        ("ops/smoke-staging", {"rust", "bin-tests"}),
        ("ops/smoke-fixture.md", {"rust", "bin-tests"}),
        ("ops/screen-eval/public/refusal-categories.md", {"rust", "bin-tests"}),
        # which ops/smoke-staging runs
        ("bin/field-test-batches", {"rust", "bin-tests"}),
        ("bin/lychee", {"bin-tests"}),
        # read by bin/tests/test_deploy.py
        (".github/workflows/ci.yml", {"bin-tests"}),
        ("crates/server/src/config.rs", {"rust", "bin-tests"}),
    ],
)
def test_the_checks_a_change_to_each_path_needs(path, checks):
    assert needed(path) == checks


def test_nothing_staged_needs_every_check():
    assert needed() == {"rust", "bin-tests"}


def test_each_check_names_the_first_path_that_needs_it():
    done = checks_needed("docs/design.md", "bin/lychee", "crates/server/src/main.rs")
    assert done.stdout.splitlines() == [
        "rust crates/server/src/main.rs",
        "bin-tests bin/lychee",
    ]


def test_a_tree_that_isnt_a_folder_fails_in_one_line():
    done = subprocess.run(
        [str(SCRIPT), str(REPO / "no-such-folder")],
        input="docs/design.md",
        capture_output=True,
        text=True,
        check=False,
    )
    assert done.returncode != 0
    assert done.stdout == ""
    assert len(done.stderr.splitlines()) == 1, done.stderr


def test_help_says_what_it_does():
    done = subprocess.run(
        [str(SCRIPT), "-h"], capture_output=True, text=True, check=False
    )
    assert done.returncode == 0
    assert "agent-pre-commit" in done.stdout
