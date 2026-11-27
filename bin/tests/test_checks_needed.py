"""bin/checks-needed on this repo's own tree: which checks a change to each path needs,
from the references the crates and bin/tests make today. A change that adds or drops
such a reference updates this list. bin/tests/test_agent_pre_commit.py covers each
kind of reference in a scratch workspace."""

import re
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


def why(*paths: str) -> str:
    """bin/checks-needed --why for each path: for each check it needs, the chain of
    references that made it an input, which a failure message shows to blame one."""
    chains = []
    for path in paths:
        done = subprocess.run(
            [str(SCRIPT), "--why", path, str(REPO)],
            capture_output=True,
            text=True,
            check=False,
        )
        assert done.returncode == 0, done.stdout + done.stderr
        chains.append(done.stdout)
    return "".join(chains)


def test_the_other_docs_need_no_check():
    docs = ("docs/design.md", "README.md", "AGENTS.md", "docs/field-test.md")
    assert needed(*docs) == set(), why(*docs)


@pytest.mark.parametrize(
    "path, checks",
    [
        # include_str! from crates/server/src/routes/docs.rs; each skill copies it;
        # bin/tests/test_analyzer_docs.py reads its headings
        ("docs/entry-format.md", {"rust", "skills", "bin-tests"}),
        # include_str! from crates/server (the skills it serves), and their record;
        # crates/server/tests/it/skill.rs checks the routes the skills name
        ("skills/filedrawer-submit/SKILL.md", {"rust", "skills", "routes"}),
        # the router, /skill.md and the test's own NOT_FOR_AGENTS list
        ("crates/server/src/lib.rs", {"rust", "bin-tests", "routes"}),
        ("crates/server/install.md", {"rust", "bin-tests", "routes"}),
        (
            "crates/server/tests/it/skill.rs",
            {"rust", "bin-tests", "routes", "test-mods"},
        ),
        # bin/test-modules reads each crate's tests/it
        ("crates/analyzer/tests/it/main.rs", {"rust", "bin-tests", "test-mods"}),
        ("bin/test-modules", {"bin-tests", "sh-rules", "test-mods"}),
        # crates/server/tests/it/smoke.rs runs it and reads these at run time
        ("ops/smoke-staging", {"rust", "bin-tests", "sh-rules"}),
        ("ops/smoke-fixture.md", {"rust", "bin-tests", "sh-rules"}),
        (
            "ops/screen-eval/public/refusal-categories.md",
            {"rust", "bin-tests", "sh-rules"},
        ),
        # which ops/smoke-staging runs
        ("bin/field-test-batches", {"rust", "bin-tests", "sh-rules"}),
        ("bin/lychee", {"bin-tests", "sh-rules"}),
        # read by bin/tests/test_deploy.py
        (".github/workflows/ci.yml", {"bin-tests", "sh-rules"}),
        ("crates/server/src/config.rs", {"rust", "bin-tests"}),
        # crates/analyzer/src/file_mentions.rs names its own crate's folder with `../`
        ("crates/analyzer/src/lib.rs", {"rust", "bin-tests"}),
    ],
)
def test_the_checks_a_change_to_each_path_needs(path, checks):
    assert needed(path) == checks, why(path)


def selected(tmp_path: Path, *paths: str) -> list[str]:
    """The bin test files bin/checks-needed --bin-tests picks for a change to `paths`,
    or bin/tests for all of them."""
    out = tmp_path / "bin-tests"
    done = subprocess.run(
        [str(SCRIPT), "--bin-tests", str(out), str(REPO)],
        input="\0".join(paths),
        capture_output=True,
        text=True,
        check=False,
    )
    assert done.returncode == 0, done.stdout + done.stderr
    return out.read_text().splitlines()


@pytest.mark.parametrize(
    "path, tests",
    [
        # test_analyzer_docs.py reads the analyzer's sources, test_license_copies.py
        # each crate's LICENSE, test_no_doctests.py every crate's sources, and this
        # file's table the crates' references.
        (
            "crates/analyzer/src/front_matter.rs",
            [
                "bin/tests/test_analyzer_docs.py",
                "bin/tests/test_checks_needed.py",
                "bin/tests/test_license_copies.py",
                "bin/tests/test_no_doctests.py",
            ],
        ),
        # test_live_preview.py copies the server's templates and static files, not
        # its sources.
        (
            "crates/server/src/routes/read.rs",
            [
                "bin/tests/test_checks_needed.py",
                "bin/tests/test_license_copies.py",
                "bin/tests/test_no_doctests.py",
            ],
        ),
        (
            "crates/server/templates/home.html",
            [
                "bin/tests/test_checks_needed.py",
                "bin/tests/test_license_copies.py",
                "bin/tests/test_live_preview.py",
                "bin/tests/test_no_doctests.py",
            ],
        ),
        # test_npm_package.py runs pack.mjs, which packs the launcher beside it.
        (
            "crates/cli/npm/filedrawer.js",
            [
                "bin/tests/test_checks_needed.py",
                "bin/tests/test_license_copies.py",
                "bin/tests/test_no_doctests.py",
                "bin/tests/test_npm_package.py",
            ],
        ),
        ("bin/lychee", ["bin/tests"]),
        ("docs/design.md", []),
    ],
)
def test_a_change_runs_only_the_bin_tests_that_reach_it(tmp_path, path, tests):
    assert selected(tmp_path, path) == tests


def test_why_names_the_crate_source_that_embeds_a_doc():
    output = why("docs/entry-format.md").splitlines()
    # The rust check's chain, up to the next check's name (skills follows it too).
    end = next(
        (i for i, line in enumerate(output[1:], 1) if not line.startswith(" ")),
        len(output),
    )
    lines = output[:end]
    assert lines[0] == "rust", output
    assert lines[-1] == "  docs/entry-format.md", lines
    assert re.fullmatch(
        r"  crates/server/\S+\.rs:\d+: \S*/docs/entry-format\.md"
        r" -> docs/entry-format\.md",
        lines[-2],
    ), lines


def test_nothing_staged_needs_every_check():
    assert needed() == {
        "rust",
        "bin-tests",
        "skills",
        "routes",
        "py-rules",
        "sh-rules",
        "test-mods",
        "caddy",
    }


def test_each_check_names_the_first_path_that_needs_it():
    done = checks_needed("docs/design.md", "bin/lychee", "crates/server/src/main.rs")
    assert done.stdout.splitlines() == [
        "rust crates/server/src/main.rs",
        "bin-tests bin/lychee",
        "sh-rules bin/lychee",
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
    assert "bin/checks-needed --why PATH" in done.stdout
