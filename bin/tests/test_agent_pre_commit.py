"""bin/agent-pre-commit runs the cargo checks only when a path changed since
$CHECK_BASE is a build input, and bin/tests only when what those tests cover changed;
bin/checks-needed decides. llm-toolbox's agent-commit-gate sets CHECK_BASE to the
nearest commit that passed the hook; these tests set it themselves, to HEAD unless a
test says otherwise, as at an agent's commit. They run the real hook and
bin/checks-needed in a scratch workspace, the hook with fakes of cargo and uvx first on
PATH that note their arguments, so no real check runs. Which references count is asked
of bin/checks-needed directly, which is faster."""

import fcntl
import hashlib
import os
import re
import shutil
import subprocess
from pathlib import Path

import pytest

REPO = Path(__file__).resolve().parents[2]

# A workspace with one reference of each kind bin/checks-needed follows.
LAYOUT = {
    "Cargo.toml": '[workspace]\nresolver = "3"\nmembers = ["crates/*"]\n',
    "crates/app/Cargo.toml": '[package]\nname = "app"\nversion = "0.1.0"\n',
    # A compile-time embed from outside crates/.
    "crates/app/src/lib.rs": (
        'pub const FORMAT: &str = include_str!("../../../docs/format.md");\n'
    ),
    # A test that runs a script and reads a file outside crates/ at run time.
    "crates/app/tests/run.rs": (
        "use std::path::{Path, PathBuf};\n"
        'const SCRIPT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../ops/run");\n'
        "fn fixture() -> PathBuf {\n"
        '    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ops/fixture.md")\n'
        "}\n"
    ),
    # It builds from the repo root, as ops/smoke-staging does, which mustn't make every
    # path an input, and reaches files outside crates/ each way a shell script can.
    "ops/run": (
        "#!/usr/bin/env bash\n"
        'repo="$(dirname "$0")/.."\n'
        'cd "$repo" && cargo build\n'
        '"$(dirname "$0")/../bin/helper"\n'
        '"$(dirname "$0")/../bin/pyhelper"\n'
        'cat "$repo/conf/settings.toml"\n'
        'cat "${0%/*}/../conf/source.toml"\n'
        '. "$(dirname "${BASH_SOURCE[0]}")/../conf/bash-source.toml"\n'
    ),
    "ops/fixture.md": "an entry\n",
    "bin/helper": "#!/bin/sh\n",
    # A Python script it runs, reaching files each way a Python script can.
    "bin/pyhelper": (
        "#!/usr/bin/env python3\n"
        "from pathlib import Path\n"
        'SIBLING = Path(__file__).with_name("sibling")\n'
        'CONF = Path(__file__).resolve().parents[1].joinpath("conf", "joined.toml")\n'
        "def data(name):\n"
        '    return Path(__file__).parents[1].joinpath("data", name)\n'
    ),
    "bin/other": "#!/bin/sh\n",
    "conf/settings.toml": "",
    # A bin test reads a root file; its prose and its scratch paths name others.
    "bin/tests/test_tool.py": (
        '"""Mentions docs/notes.md, which bin/tests doesn\'t read."""\n'
        "from pathlib import Path\n"
        "REPO = Path(__file__).resolve().parents[2]\n"
        'CONFIG = REPO / "tool.toml"\n'
        "def test_tool(tmp_path):\n"
        '    (tmp_path / "docs").mkdir()\n'
    ),
    "tool.toml": "",
    # A bin test that runs a script, which reads a root file: the full check selects
    # that test for a change to the file.
    "bin/tests/test_reader.py": (
        "from pathlib import Path\n"
        "REPO = Path(__file__).resolve().parents[2]\n"
        'READER = REPO / "bin/reader"\n'
    ),
    "bin/reader": '#!/bin/sh\ncat "$(dirname "$0")/../lib/read.toml"\n',
    "lib/read.toml": "",
    # A bin test that reaches all of bin/, which doesn't make it a test of each script.
    "bin/tests/test_everything.py": (
        'from pathlib import Path\nBIN = Path(__file__).resolve().parents[2] / "bin"\n'
    ),
    # A helper beside the tests, which any of them may import, reads a root file.
    "bin/tests/helpers.py": (
        "from pathlib import Path\n"
        'SHARED = Path(__file__).resolve().parents[2] / "shared.toml"\n'
    ),
    "shared.toml": "",
    # A bin test that joins both a script and the file the script reads, and one that
    # names two scripts reading the same file.
    "bin/tests/test_both.py": (
        "from pathlib import Path\n"
        "REPO = Path(__file__).resolve().parents[2]\n"
        'SCRIPT = REPO / "bin/both"\n'
        'READ = REPO / "lib/both.toml"\n'
    ),
    "bin/both": '#!/bin/sh\ncat "$(dirname "$0")/../lib/both.toml"\n',
    "lib/both.toml": "",
    "bin/tests/test_pair.py": (
        "from pathlib import Path\n"
        "REPO = Path(__file__).resolve().parents[2]\n"
        'ONE = REPO / "bin/one"\n'
        'TWO = REPO / "ops/two"\n'
    ),
    "bin/one": '#!/bin/sh\ncat "$(dirname "$0")/../lib/pair.toml"\n',
    "ops/two": '#!/bin/sh\ncat "$(dirname "$0")/../lib/pair.toml"\n',
    "lib/pair.toml": "",
    # The crate's integration tests, one binary: a file and a folder with mod.rs,
    # each with its `mod` line, and a folder of data that is no module.
    "crates/app/tests/it/main.rs": "mod common;\nmod one;\n",
    "crates/app/tests/it/one.rs": "",
    "crates/app/tests/it/common/mod.rs": "",
    "crates/app/tests/it/fixtures/entry.md": "",
    "docs/format.md": "# Format\n",
    "docs/notes.md": "# Notes\n",
    # As in the repo: the check's folder lives in target/.
    ".gitignore": "target/\n",
}
FAKE = (
    "#!/bin/sh\n"
    'echo "$(basename "$0") $*" >> "$FAKE_LOG"\n'
    'if [ -n "${CHECK_QUEUE_EXIT+set}" ]; then echo saw-CHECK_QUEUE_EXIT >> "$FAKE_LOG"; fi\n'
)


def git(repo: Path, *args: str, env: dict | None = None) -> str:
    done = subprocess.run(
        ["git", "-C", str(repo), *args],
        capture_output=True,
        text=True,
        check=True,
        env=env,
    )
    return done.stdout.strip()


def commit(repo: Path, message: str = "change") -> None:
    git(repo, "add", "-A")
    git(repo, "commit", "-qm", message)


@pytest.fixture(scope="module")
def template(tmp_path_factory) -> Path:
    root = tmp_path_factory.mktemp("template") / "repo"
    for name, text in LAYOUT.items():
        path = root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text)
        if text.startswith("#!"):
            path.chmod(0o755)
    for script in (
        "agent-pre-commit",
        "agent-quick-check",
        "agent-commit-check",
        "checks-needed",
        "pytest",
        "test-modules",
    ):
        shutil.copy(REPO / "bin" / script, root / "bin" / script)
    # The rule test the commit check runs for a bin/tests change.
    shutil.copy(
        REPO / "bin/tests/test_no_wall_clock_limits.py",
        root / "bin/tests/test_no_wall_clock_limits.py",
    )
    # And the ones it runs beside it, here only names the fake uvx is handed.
    for name in (
        "checks_needed",
        "no_early_exit_readers",
        "workflow_run_blocks",
        "workflow_supply_chain",
    ):
        (root / f"bin/tests/test_{name}.py").write_text("")
    git(root, "init", "-q")
    # Its own identity, since CI's runner has no global one.
    git(root, "config", "user.name", "t")
    git(root, "config", "user.email", "t@t")
    commit(root, "init")
    return root


@pytest.fixture
def repo(template, tmp_path) -> Path:
    copy = tmp_path / "repo"
    shutil.copytree(template, copy, symlinks=True)
    return copy


def hook(
    repo: Path,
    *args: str,
    env: dict | None = None,
    base: str | None = "HEAD",
    code: int = 0,
    script: str = "agent-pre-commit",
) -> tuple[set[str], subprocess.CompletedProcess]:
    """Run the hook with fake cargo and uvx, and CHECK_BASE set to `base` as
    `git rev-parse` gives it (None: unset), and check it exits with `code`; return the
    checks it ran (nextest, doctest, clippy, pytest) and the hook's run."""
    done = subprocess.run(
        [str(repo / "bin" / script), *args],
        cwd=repo,
        env=hook_env(repo, env, base),
        capture_output=True,
        text=True,
        check=False,
    )
    assert done.returncode == code, done.stdout + done.stderr
    calls = (repo.parent / "calls").read_text().splitlines()
    named = {
        "nextest": "cargo nextest run",
        "doctest": "cargo test --doc",
        "clippy": "cargo clippy",
        "pytest": "uvx",
    }
    ran = {
        check
        for check, start in named.items()
        if any(call.startswith(start) for call in calls)
    }
    return ran, done


def hook_env(repo: Path, env: dict | None, base: str | None) -> dict:
    """The hook's environment, as hook() takes `env` and `base`: fakes of cargo and uvx
    first on PATH, which note their calls in the log `calls` beside the repo, emptied
    here, and TMPDIR the folder `tmp` beside it, where the hook keeps its check
    folder."""
    (repo.parent / "tmp").mkdir(exist_ok=True)
    fakes = repo.parent / "fakes"
    fakes.mkdir(exist_ok=True)
    for tool in ("cargo", "uvx"):
        # FAKE_UVX in `env` replaces the fake uvx's text.
        (fakes / tool).write_text((env or {}).get(f"FAKE_{tool.upper()}", FAKE))
        (fakes / tool).chmod(0o755)
    log = repo.parent / "calls"
    log.write_text("")
    out = {
        k: v
        for k, v in os.environ.items()
        if not k.startswith("GIT_") and k not in ("CHECK_BASE", "CHECK_QUEUE_EXIT")
    }
    out |= {
        "PATH": f"{fakes}{os.pathsep}{os.environ['PATH']}",
        "TMPDIR": str(repo.parent / "tmp"),
    }
    if base is not None:
        out["CHECK_BASE"] = git(repo, "rev-parse", base)
    return out | {"FAKE_LOG": str(log), **(env or {})}


def run(repo: Path, *args: str, **options) -> set[str]:
    """The checks the hook ran; `options` as hook() takes them."""
    return hook(repo, *args, **options)[0]


def needed(tree: Path, *paths: str) -> set[str]:
    """The checks bin/checks-needed says a change to `paths` in `tree` needs."""
    done = subprocess.run(
        [str(tree / "bin/checks-needed"), str(tree)],
        input="\0".join(paths),
        capture_output=True,
        text=True,
        check=False,
    )
    assert done.returncode == 0, done.stdout + done.stderr
    return {line.split(" ", 1)[0] for line in done.stdout.splitlines()}


def why(tree: Path, path: str) -> str:
    """What bin/checks-needed --why says made `path` in `tree` an input of each check."""
    done = subprocess.run(
        [str(tree / "bin/checks-needed"), "--why", path, str(tree)],
        capture_output=True,
        text=True,
        check=False,
    )
    assert done.returncode == 0, done.stdout + done.stderr
    return done.stdout


def selected(tree: Path, *paths: str, env: dict | None = None) -> list[str]:
    """The pytest arguments bin/checks-needed --bin-tests picks for a change to
    `paths` in `tree`: the bin test files to run, or bin/tests for all of them."""
    out = tree.parent / "bin-tests"
    done = subprocess.run(
        [str(tree / "bin/checks-needed"), "--bin-tests", str(out), str(tree)],
        input="\0".join(paths),
        capture_output=True,
        text=True,
        check=False,
        env=None if env is None else os.environ | env,
    )
    assert done.returncode == 0, done.stdout + done.stderr
    return out.read_text().splitlines()


RUST = {"nextest", "doctest", "clippy"}
EVERY = RUST | {"pytest"}


def change(repo: Path, name: str, text: str = "changed\n") -> None:
    (repo / name).parent.mkdir(parents=True, exist_ok=True)
    (repo / name).write_text(text)
    git(repo, "add", name)


def test_a_docs_only_change_runs_no_check(repo):
    change(repo, "docs/notes.md")
    assert run(repo) == set()


def test_a_change_under_crates_runs_the_cargo_checks_and_bin_tests(repo):
    # bin/tests/test_checks_needed.py reads the crates' references.
    change(repo, "crates/app/src/lib.rs", "pub fn f() {}\n")
    assert run(repo) == EVERY


def test_a_change_to_an_embedded_file_runs_the_cargo_checks(repo):
    change(repo, "docs/format.md")
    assert run(repo) == RUST


def test_deleting_an_embedded_file_runs_the_cargo_checks(repo):
    git(repo, "rm", "-q", "docs/format.md")
    assert run(repo) == RUST


def test_renaming_an_embedded_file_runs_the_cargo_checks(repo):
    git(repo, "mv", "docs/format.md", "docs/renamed.md")
    assert run(repo) == RUST


@pytest.mark.parametrize(
    "path, checks",
    [
        ("docs/format.md", {"rust"}),  # include_str!
        (
            "ops/fixture.md",
            {"rust", "bin-tests", "sh-rules"},
        ),  # a crate's test reads it
        ("bin/helper", {"rust", "bin-tests", "sh-rules"}),  # that test's script runs it
        # ops/run reaches these, so they're bin/tests' inputs too: they run it.
        ("conf/settings.toml", {"rust", "bin-tests"}),  # through the script's $repo
        ("conf/source.toml", {"rust", "bin-tests"}),  # through ${0%/*}
        ("conf/bash-source.toml", {"rust", "bin-tests"}),  # through BASH_SOURCE
        (
            "bin/sibling",
            {"rust", "bin-tests", "sh-rules"},
        ),  # a Python script's with_name
        ("conf/joined.toml", {"rust", "bin-tests"}),  # its joinpath
        ("data/any.json", {"rust", "bin-tests"}),  # the folder before a built name
        ("Cargo.lock", {"rust"}),
        (".sqlx/query-1.json", {"rust"}),
        (".gitattributes", {"rust", "bin-tests"}),
        ("crates/app/src/lib.rs", {"rust", "bin-tests"}),
        ("tool.toml", {"bin-tests"}),  # a bin test joins it onto the repo
        ("bin/other", {"bin-tests", "sh-rules"}),
        ("docs/notes.md", set()),  # in prose and a scratch path only
        # The skills' record follows their folders and the format they copy.
        ("skills/demo/SKILL.md", {"skills", "routes"}),
        ("skills/versions.txt", {"skills", "routes"}),
        ("docs/entry-format.md", {"skills"}),
        ("bin/skills", {"skills", "bin-tests", "sh-rules"}),
        # The rule tests read every file in bin/tests, the scripts in bin/ and
        # ops/, and the workflows.
        ("bin/tests/test_tool.py", {"bin-tests", "py-rules", "sh-rules"}),
        (".github/workflows/x.yml", {"sh-rules"}),
        # The module check reads every crate's tests/it, and its own script.
        ("crates/app/tests/it/one.rs", {"rust", "bin-tests", "test-mods"}),
        ("crates/app/tests/it/main.rs", {"rust", "bin-tests", "test-mods"}),
        ("crates/app/tests/it/fixtures/entry.md", {"rust", "bin-tests", "test-mods"}),
        ("bin/test-modules", {"bin-tests", "sh-rules", "test-mods"}),
        # The real-Caddy tests, their scripts and the files they live in.
        ("ops/remote/caddy.sh", {"bin-tests", "sh-rules", "caddy"}),
        ("ops/remote/field-test-key.sh", {"bin-tests", "sh-rules", "caddy"}),
        ("bin/caddy", {"bin-tests", "sh-rules", "caddy"}),
        (
            "bin/tests/test_remote_environments.py",
            {"bin-tests", "py-rules", "sh-rules", "caddy"},
        ),
        (
            "bin/tests/test_field_test_key.py",
            {"bin-tests", "py-rules", "sh-rules", "caddy"},
        ),
        # It holds the real_caddy fixture and the hook that marks its tests.
        ("bin/tests/conftest.py", {"bin-tests", "py-rules", "sh-rules", "caddy"}),
        ("ops/remote/other.sh", {"bin-tests", "sh-rules"}),
    ],
)
def test_the_checks_a_change_to_each_reference_needs(template, path, checks):
    assert needed(template, path) == checks


RULE_TEST = "bin/tests/test_no_wall_clock_limits.py"
NEEDED_TEST = "bin/tests/test_checks_needed.py"
SH_RULE_TESTS = (
    "bin/tests/test_no_early_exit_readers.py bin/tests/test_workflow_run_blocks.py "
    "bin/tests/test_workflow_supply_chain.py"
)


def uvx_calls(repo: Path) -> list[str]:
    return [
        call
        for call in (repo.parent / "calls").read_text().splitlines()
        if call.startswith("uvx ")
    ]


def test_the_commit_check_runs_the_source_rules_for_a_bin_tests_change(repo):
    change(repo, "bin/tests/test_tool.py", "def test_tool():\n    pass\n")

    hook(repo, env=QUEUE_EXIT, script="agent-commit-check")

    calls = uvx_calls(repo)
    assert len(calls) == 1, calls
    # Each reads sources and takes seconds: the wall-clock rule, the table of which
    # checks a path needs, which a new test's reads can change, and, since bin/tests
    # lies in bin/, the shell and workflow rules.
    assert calls[0].endswith(f"-n0 {RULE_TEST} {NEEDED_TEST} {SH_RULE_TESTS}"), calls
    assert "pytest@9.2.0" in calls[0]


@pytest.mark.parametrize(
    "path", ["bin/other", "ops/remote/other.sh", ".github/workflows/x.yml"]
)
def test_a_script_or_workflow_change_runs_only_the_shell_and_workflow_rules(repo, path):
    """A pipe into `grep -q` and parsing in a workflow's YAML once passed their
    commits, and an action named by a tag would pass one; the table of checks, which
    takes 9 s, stays with bin/tests changes."""
    change(repo, path, "x\n")

    hook(repo, env=QUEUE_EXIT, script="agent-commit-check")

    (call,) = uvx_calls(repo)
    assert call.endswith(f"-n0 {SH_RULE_TESTS}"), call


def test_a_tree_without_the_checks_needed_test_runs_the_rest(repo):
    """As a tree checked out at a commit older than that test is."""
    git(repo, "rm", "-q", NEEDED_TEST)
    change(repo, "bin/tests/test_tool.py", "def test_tool():\n    pass\n")

    hook(repo, env=QUEUE_EXIT, script="agent-commit-check")

    (call,) = uvx_calls(repo)
    assert call.endswith(f"-n0 {RULE_TEST} {SH_RULE_TESTS}"), call


def test_the_rules_run_only_with_rules_and_only_for_a_change_they_read(repo):
    change(repo, "bin/tests/test_tool.py", "def test_tool():\n    pass\n")
    hook(repo, "--quick", "--routes")
    assert uvx_calls(repo) == []
    git(repo, "reset", "-q")
    change(repo, "docs/notes.md")
    hook(repo, "--quick", "--routes", "--rules")
    assert uvx_calls(repo) == []


def test_a_failing_rule_refuses_the_commit_with_its_output(repo):
    change(repo, "bin/tests/test_tool.py", "def test_tool():\n    pass\n")
    failing_uvx = FAKE + 'echo "a wall-clock limit at test_tool.py:2"\nexit 1\n'

    _, done = hook(
        repo,
        "--quick",
        "--routes",
        "--rules",
        env={"FAKE_UVX": failing_uvx},
        code=1,
    )

    assert "a wall-clock limit at test_tool.py:2" in done.stdout + done.stderr


def commit_check(repo: Path, code: int = 0) -> str:
    """The commit check's output, run outside the queue as the gate first runs it,
    which must exit with `code` there."""
    _, done = hook(repo, env=QUEUE_EXIT, script="agent-commit-check", code=code)
    return done.stdout + done.stderr


def test_the_commit_check_refuses_a_test_file_main_rs_has_no_mod_line_for(repo):
    change(repo, "crates/app/tests/it/two.rs", "#[test]\nfn two() {}\n")
    change(repo, "crates/app/tests/it/helpers/mod.rs", "")

    output = commit_check(repo, code=1)

    # Before the queue: no cargo or uvx call at all.
    assert (repo.parent / "calls").read_text() == ""
    for path, line in (
        ("crates/app/tests/it/two.rs", "mod two;"),
        ("crates/app/tests/it/helpers/mod.rs", "mod helpers;"),
    ):
        assert path in output, output
        assert f"`{line}`" in output, output
    assert "crates/app/tests/it/main.rs" in output, output
    assert "one.rs" not in output and "common" not in output, output


def test_the_commit_check_passes_a_test_file_with_its_mod_line(repo):
    change(repo, "crates/app/tests/it/two.rs", "")
    change(repo, "crates/app/tests/it/main.rs", "mod common;\nmod one;\nmod two;\n")
    commit_check(repo, code=75)  # on to the queue for clippy


def test_the_commit_check_reads_the_staged_main_rs_not_the_working_one(repo):
    change(repo, "crates/app/tests/it/two.rs", "")
    (repo / "crates/app/tests/it/main.rs").write_text(
        "mod common;\nmod one;\nmod two;\n"
    )
    assert "`mod two;`" in commit_check(repo, code=1)


def test_the_commit_check_ignores_an_unstaged_test_file(repo):
    (repo / "crates/app/tests/it/two.rs").write_text("")
    change(repo, "crates/app/tests/it/one.rs", "#[test]\nfn one() {}\n")
    commit_check(repo, code=75)


def test_the_module_check_runs_only_for_a_change_under_tests_it(repo):
    # A tree that already holds a file with no `mod` line: the Rust test's to find.
    change(repo, "crates/app/tests/it/two.rs", "")
    commit(repo)
    change(repo, "docs/notes.md")
    commit_check(repo)


def test_the_module_check_runs_only_at_a_commit(repo):
    change(repo, "crates/app/tests/it/two.rs", "")
    hook(repo, "--quick", env=QUEUE_EXIT, code=75)
    hook(repo, "--quick", env=QUEUE_EXIT, script="agent-quick-check", code=75)


CADDY_RUN = "-n0 -m real_caddy bin/tests"


def test_the_commit_check_runs_the_real_caddy_tests_for_a_caddy_change(repo):
    change(repo, "ops/remote/caddy.sh")

    hook(repo, script="agent-commit-check")

    # The source rules first, as for any script; then the real-Caddy tests.
    calls = uvx_calls(repo)
    assert len(calls) == 2, calls
    assert calls[1].endswith(CADDY_RUN), calls
    assert "pytest@9.2.0" in calls[1]


def test_outside_the_queue_the_real_caddy_tests_ask_for_it_first(repo):
    change(repo, "bin/caddy")

    _, done = hook(repo, env=QUEUE_EXIT, script="agent-commit-check", code=75)

    # Only the source rules, which need no queue, ran before it asked.
    assert not any(call.endswith(CADDY_RUN) for call in uvx_calls(repo))
    assert "agent-pre-commit: needs heavy's queue for caddy bin/caddy" in done.stderr


@pytest.mark.parametrize(
    "path, script",
    [
        ("ops/remote/other.sh", "agent-commit-check"),
        ("ops/remote/caddy.sh", "agent-quick-check"),
    ],
    ids=["another ops script", "the quick check"],
)
def test_the_real_caddy_tests_run_only_with_caddy_and_for_a_caddy_change(
    repo, path, script
):
    change(repo, path)
    hook(repo, script=script)
    assert not any(call.endswith(CADDY_RUN) for call in uvx_calls(repo))


def test_a_failing_real_caddy_test_refuses_the_commit_with_its_output(repo):
    change(repo, "ops/remote/field-test-key.sh")
    failing_uvx = FAKE + 'echo "FAILED test_a_real_caddy_serves"\nexit 1\n'

    _, done = hook(
        repo, script="agent-commit-check", env={"FAKE_UVX": failing_uvx}, code=1
    )

    assert "FAILED test_a_real_caddy_serves" in done.stdout + done.stderr


def test_the_real_caddy_marker_selects_every_test_using_the_fixture():
    done = subprocess.run(
        [str(REPO / "bin/pytest"), "-n0", "--collect-only", "-m", "real_caddy"],
        cwd=REPO / "bin/tests",
        capture_output=True,
        text=True,
        check=False,
    )
    assert done.returncode == 0, done.stdout + done.stderr
    selected = {line for line in done.stdout.splitlines() if "::" in line}
    assert selected == {
        "test_field_test_key.py::"
        "test_a_real_caddy_serves_the_gate_and_snippet_as_meant",
        "test_remote_environments.py::"
        "test_a_real_caddy_takes_both_environments_site_files_together",
        "test_remote_environments.py::"
        "test_a_real_caddy_listens_for_its_admin_api_on_the_socket_alone_and_reloads",
        "test_remote_environments.py::"
        "test_a_real_caddy_answers_503_while_production_is_down_and_redirects_www",
        "test_remote_environments.py::"
        "test_a_real_caddy_stops_reading_a_request_body_that_takes_too_long",
    }


def test_bin_pytest_runs_the_pinned_pytest_with_its_plugins_on_its_arguments(repo):
    fakes = repo.parent / "fakes"
    fakes.mkdir(exist_ok=True)
    (fakes / "uvx").write_text(FAKE)
    (fakes / "uvx").chmod(0o755)
    log = repo.parent / "calls"
    done = subprocess.run(
        [str(repo / "bin/pytest"), "-n0", "bin/tests/test_tool.py", "-k", "tool"],
        env={
            **os.environ,
            "PATH": f"{fakes}{os.pathsep}{os.environ['PATH']}",
            "FAKE_LOG": str(log),
        },
        capture_output=True,
        text=True,
        check=False,
    )

    assert done.returncode == 0, done.stderr
    assert log.read_text() == (
        "uvx --quiet --with pytest-timeout==2.4.0 --with pytest-xdist==3.8.0 --with hypothesis==6.170.15 pytest@9.2.0"
        " -q -p no:cacheprovider -n0 bin/tests/test_tool.py -k tool\n"
    )


def test_the_full_check_runs_bin_tests_through_bin_pytest(repo):
    (repo / "bin/pytest").write_text('#!/bin/sh\necho "bin/pytest $*" >> "$FAKE_LOG"\n')
    commit(repo)
    change(repo, "bin/other")

    run(repo)

    assert "bin/pytest bin/tests" in (repo.parent / "calls").read_text().splitlines()


def test_names_a_python_script_builds_from_each_other_are_read_in_time(repo):
    (repo / "bin/tests/test_cycle.py").write_text(
        "from pathlib import Path\n"
        "a = Path(__file__).parent\n"
        'a = b / "x"\n'
        'b = a / "y"\n'
    )
    assert needed(repo, "docs/notes.md") == set()


# The chains --why prints, file:line by file:line through LAYOUT.


def test_why_follows_a_crates_embed_to_the_file(template):
    assert why(template, "docs/format.md") == (
        "rust\n"
        "  crates/app: a crate (it holds a Cargo.toml)\n"
        "  crates/app/src/lib.rs:1: ../../../docs/format.md -> docs/format.md\n"
        "  docs/format.md\n"
    )


def test_why_follows_a_crates_test_through_the_scripts_it_runs(template):
    assert why(template, "data/any.json") == (
        "rust\n"
        "  crates/app: a crate (it holds a Cargo.toml)\n"
        "  crates/app/tests/run.rs:2: /../../ops/run -> ops/run\n"
        '  ops/run:5: $(dirname "$0")/../bin/pyhelper -> bin/pyhelper\n'
        '  bin/pyhelper:6: Path(__file__).parents[1].joinpath("data", name) -> data\n'
        "  data/any.json\n"
        "bin-tests\n"
        "  bin: bin/tests covers it\n"
        '  bin/pyhelper:6: Path(__file__).parents[1].joinpath("data", name) -> data\n'
        "  data/any.json\n"
    )


def test_why_follows_a_name_a_shell_script_assigns_its_folder(template):
    assert why(template, "conf/settings.toml").split("bin-tests\n")[1] == (
        "  ops: bin/tests covers it\n"
        "  ops/run:6: $repo/conf/settings.toml -> conf/settings.toml\n"
        "  conf/settings.toml\n"
    )


def test_why_names_the_rule_for_a_path_no_file_names(template):
    assert why(template, "Cargo.lock") == (
        "rust\n  any Cargo.toml, Cargo.lock or build.rs: a build input\n  Cargo.lock\n"
    )
    assert why(template, ".sqlx/query-1.json") == (
        "rust\n  .sqlx: a build input\n  .sqlx/query-1.json\n"
    )


def test_why_blames_a_join_onto_the_root_with_a_name_for_every_path(repo):
    (repo / "bin/tests/test_join.py").write_text(
        "from pathlib import Path\n"
        "REPO = Path(__file__).resolve().parents[2]\n"
        "def read(script):\n"
        "    return (REPO / script).read_text()\n"
    )
    assert why(repo, "docs/notes.md") == (
        "bin-tests\n"
        "  bin: bin/tests covers it\n"
        "  bin/tests/test_join.py:4: REPO / script -> .\n"
        "  docs/notes.md\n"
    )


def test_why_starts_a_crate_outside_crates_at_its_own_rule(repo):
    # Another crate's path dependency names it too, a step further from a rule.
    (repo / "tools/xtask").mkdir(parents=True)
    (repo / "tools/xtask/Cargo.toml").write_text('[package]\nname = "xtask"\n')
    with (repo / "crates/app/Cargo.toml").open("a") as manifest:
        manifest.write('[dependencies]\nxtask = { path = "../../tools/xtask" }\n')
    assert why(repo, "tools/xtask/src/main.rs").split("bin-tests\n")[0] == (
        "rust\n"
        "  tools/xtask: a crate (it holds a Cargo.toml)\n"
        "  tools/xtask/src/main.rs\n"
    )


def test_a_crate_naming_its_own_folder_still_needs_its_checks(repo):
    (repo / "crates/app/src/up.rs").write_text('pub const UP: &str = "../";\n')
    assert needed(repo, "crates/app/src/lib.rs") == {"rust", "bin-tests"}
    assert why(repo, "crates/app/src/lib.rs") == (
        "rust\n"
        "  crates: a build input\n"
        "  crates/app/src/lib.rs\n"
        "bin-tests\n"
        "  crates/app: a crate (it holds a Cargo.toml)\n"
        "  crates/app/src/lib.rs\n"
    )


def test_why_reads_no_stdin(template, tmp_path):
    # A write-only stdin fails a read at once, where an open pipe would wait forever.
    with (tmp_path / "write-only").open("w") as write_only:
        done = subprocess.run(
            [str(template / "bin/checks-needed"), "--why", "docs/format.md"],
            cwd=template,
            stdin=write_only,
            capture_output=True,
            text=True,
            check=False,
        )
    assert done.returncode == 0, done.stderr
    assert done.stdout.startswith("rust\n"), done.stdout


def test_why_cites_a_python_files_first_join_to_a_path(repo):
    # The later line's join is the one the walk meets first.
    (repo / "bin/tests/test_two.py").write_text(
        "from pathlib import Path\n"
        "REPO = Path(__file__).resolve().parents[2]\n"
        "def test_two():\n"
        '    return REPO / "tool.toml"\n'
        'CONFIG = REPO / "tool.toml"\n'
    )
    (repo / "bin/tests/test_tool.py").unlink()
    assert why(repo, "tool.toml") == (
        "bin-tests\n"
        "  bin: bin/tests covers it\n"
        '  bin/tests/test_two.py:4: REPO / "tool.toml" -> tool.toml\n'
        "  tool.toml\n"
    )


@pytest.mark.parametrize("path", ["/etc/hosts", "../outside", "."])
def test_why_refuses_a_path_outside_the_tree_in_one_line(template, path):
    done = subprocess.run(
        [str(template / "bin/checks-needed"), "--why", path, str(template)],
        capture_output=True,
        text=True,
        check=False,
    )
    assert done.returncode == 2
    assert done.stdout == ""
    assert len(done.stderr.splitlines()) == 1, done.stderr


def test_why_says_when_no_check_needs_the_path(template):
    assert why(template, "docs/notes.md") == "no check needs docs/notes.md\n"


def test_a_change_under_bin_runs_bin_tests_and_no_cargo(repo):
    change(repo, "bin/other")
    assert run(repo) == {"pytest"}


def test_a_root_file_a_bin_test_reads_runs_bin_tests(repo):
    change(repo, "tool.toml")
    assert run(repo) == {"pytest"}


@pytest.mark.parametrize(
    "paths, args",
    [
        # The test that joins the file.
        (["tool.toml"], ["bin/tests/test_tool.py"]),
        # The test that names the script that reads it, not the one that joins all
        # of bin/.
        (["lib/read.toml"], ["bin/tests/test_reader.py"]),
        (
            ["tool.toml", "lib/read.toml"],
            ["bin/tests/test_reader.py", "bin/tests/test_tool.py"],
        ),
        # A crate's sources hold the references test_checks_needed.py's table follows.
        (["crates/app/src/lib.rs"], ["bin/tests/test_checks_needed.py"]),
        (
            ["crates/app/src/lib.rs", "tool.toml"],
            ["bin/tests/test_checks_needed.py", "bin/tests/test_tool.py"],
        ),
        # A helper beside the tests reads it, and any test may import that.
        (["shared.toml"], ["bin/tests"]),
        # Only ops/run reads it, and no test names ops/run.
        (["conf/settings.toml"], ["bin/tests"]),
        # bin/ and ops/ are bin/tests' own.
        (["bin/other"], ["bin/tests"]),
        (["tool.toml", "bin/other"], ["bin/tests"]),
        # Nothing changed narrows nothing.
        ([], ["bin/tests"]),
        # No bin test reads them.
        (["docs/notes.md"], []),
        (["docs/format.md"], []),
    ],
)
def test_bin_tests_selects_the_tests_that_reach_the_change(repo, paths, args):
    assert selected(repo, *paths) == args


@pytest.mark.parametrize(
    "path, args",
    [
        ("lib/both.toml", ["bin/tests/test_both.py"]),
        ("lib/pair.toml", ["bin/tests/test_pair.py"]),
    ],
)
def test_a_script_reached_down_another_chain_still_counts_as_tested(repo, path, args):
    # The holders are sets, so their order follows the hash seed: each seed must give
    # the same selection.
    for seed in range(8):
        assert selected(repo, path, env={"PYTHONHASHSEED": str(seed)}) == args, seed


def pytest_args(repo: Path) -> list[str]:
    """What each pytest run through bin/pytest was handed, after bin/pytest's own
    options."""
    return [call.split("no:cacheprovider ", 1)[1] for call in uvx_calls(repo)]


def test_the_full_check_runs_only_the_bin_tests_a_change_reaches(repo):
    change(repo, "tool.toml")
    change(repo, "lib/read.toml")
    run(repo)
    assert pytest_args(repo) == ["bin/tests/test_reader.py bin/tests/test_tool.py"]


def test_the_full_check_runs_every_bin_test_for_a_change_in_bin(repo):
    change(repo, "bin/other")
    run(repo)
    assert pytest_args(repo) == ["bin/tests"]


def test_without_check_base_the_full_check_runs_every_bin_test(repo):
    change(repo, "tool.toml")
    run(repo, base=None)
    assert pytest_args(repo) == ["bin/tests"]


def test_without_check_base_as_in_ci_every_check_runs(repo):
    change(repo, "docs/notes.md")
    assert run(repo, base=None) == EVERY


def test_nothing_changed_since_check_base_runs_every_check(repo):
    assert run(repo) == EVERY


def test_a_recheck_runs_only_what_the_commits_since_check_base_need(repo):
    change(repo, "docs/notes.md")
    commit(repo)
    assert run(repo, base="HEAD~1") == set()
    change(repo, "docs/format.md")
    commit(repo)
    assert run(repo, base="HEAD~2") == RUST


def test_a_check_base_git_cant_diff_against_runs_every_check(repo):
    change(repo, "docs/notes.md")
    ran, done = hook(repo, base="0" * 40)  # no such commit
    assert ran == EVERY
    assert "can't diff against CHECK_BASE" in done.stderr


def test_quick_runs_the_routes_test_when_the_router_changes(repo):
    change(repo, "crates/server/Cargo.toml", '[package]\nname = "filedrawer-server"\n')
    git(repo, "commit", "-qm", "the server crate")
    change(repo, "crates/server/src/lib.rs", "pub fn f() {}\n")
    ran, done = hook(repo, "--quick", "--routes")
    assert ran == {"clippy", "nextest"}, done.stdout + done.stderr
    calls = (repo.parent / "calls").read_text()
    assert "skill::the_documents_name_every_route_meant_for_agents" in calls, calls
    # The small-fix check runs no test.
    assert run(repo, "--quick") == {"clippy"}


def test_quick_runs_no_routes_test_for_another_crates_change_or_without_the_server(
    repo,
):
    change(repo, "crates/server/Cargo.toml", '[package]\nname = "filedrawer-server"\n')
    git(repo, "commit", "-qm", "the server crate")
    change(repo, "crates/app/src/lib.rs", "pub fn f() {}\n")
    assert run(repo, "--quick", "--routes") == {"clippy"}
    git(repo, "reset", "-q")
    git(repo, "rm", "-q", "crates/server/Cargo.toml")
    git(repo, "commit", "-qm", "no server crate")
    change(repo, "crates/server/src/lib.rs", "pub fn f() {}\n")
    assert run(repo, "--quick", "--routes") == {"clippy"}


def test_quick_runs_clippy_for_a_crates_change_and_nothing_for_docs(repo):
    change(repo, "crates/app/src/lib.rs", "pub fn f() {}\n")
    assert run(repo, "--quick") == {"clippy"}
    git(repo, "reset", "-q")
    change(repo, "docs/notes.md")
    assert run(repo, "--quick") == set()


def test_a_tree_without_checks_needed_runs_every_check(repo):
    git(repo, "rm", "-q", "bin/checks-needed")
    commit(repo)
    change(repo, "docs/notes.md")
    assert run(repo) == EVERY


def test_a_failing_checks_needed_runs_every_check_and_says_so(repo):
    (repo / "bin/checks-needed").write_text("#!/bin/sh\necho broken >&2\nexit 3\n")
    commit(repo)
    change(repo, "docs/notes.md")
    ran, done = hook(repo)
    assert ran == EVERY
    assert "broken" in done.stderr
    assert "bin/checks-needed failed, so every check runs" in done.stderr
    assert pytest_args(repo) == ["bin/tests"]


def test_a_checks_needed_that_writes_a_selection_and_fails_runs_every_bin_test(repo):
    (repo / "bin/checks-needed").write_text(
        '#!/bin/sh\nwhile [ "$1" != --bin-tests ]; do shift; done\n'
        'echo bin/tests/test_tool.py > "$2"\nexit 3\n'
    )
    commit(repo)
    change(repo, "tool.toml")
    run(repo)
    assert pytest_args(repo) == ["bin/tests"]


@pytest.mark.parametrize("committed", [False, True], ids=["staged", "committed"])
@pytest.mark.parametrize(
    "script",
    [
        "bin/checks-needed",
        "bin/agent-pre-commit",
        "bin/agent-quick-check",
        "bin/agent-commit-check",
    ],
)
def test_a_change_to_the_rules_since_check_base_runs_every_check(
    repo, script, committed
):
    """Staged, as at a commit, or committed since CHECK_BASE, as on a recheck."""
    with (repo / script).open("a") as text:
        text.write("# a comment\n")
    git(repo, "add", script)
    if committed:
        commit(repo)
        change(repo, "docs/notes.md")
    assert run(repo, base="HEAD~1" if committed else "HEAD") == EVERY


def test_the_staged_checks_needed_decides_not_the_working_copy(repo):
    change(repo, "docs/format.md")
    (repo / "bin/checks-needed").write_text("#!/bin/sh\ncat >/dev/null\n")
    assert run(repo) == RUST


def test_the_index_git_commit_paths_builds_decides(repo):
    change(repo, "docs/notes.md")
    index = repo / ".git/paths-index"
    env = {**os.environ, "GIT_INDEX_FILE": str(index)}
    git(repo, "read-tree", "HEAD", env=env)
    (repo / "crates/app/src/lib.rs").write_text("pub fn f() {}\n")
    git(repo, "add", "crates/app/src/lib.rs", env=env)
    assert run(repo, env={"GIT_INDEX_FILE": str(index)}) == EVERY


def test_a_crate_reading_relative_to_the_root_makes_every_path_an_input(repo):
    change(repo, "crates/app/src/root.rs", 'pub const ROOT: &str = "../..";\n')
    commit(repo)
    change(repo, "docs/notes.md")
    assert run(repo) == RUST


def test_a_workspace_member_outside_crates_is_a_build_input(repo):
    change(repo, "tools/xtask/Cargo.toml", '[package]\nname = "xtask"\n')
    commit(repo)
    change(repo, "tools/xtask/src/main.rs", "fn main() {}\n")
    assert run(repo) == EVERY


# llm-toolbox's agent-commit-gate runs a check first outside heavy's queue when its
# file names CHECK_QUEUE_EXIT, with it set to this; the check exits with it to queue.
QUEUE_EXIT = {"CHECK_QUEUE_EXIT": "75"}


@pytest.mark.parametrize(
    "path, args, base, reason",
    [
        ("crates/app/src/lib.rs", (), "HEAD", "bin-tests crates/app/src/lib.rs"),
        ("docs/format.md", (), "HEAD", "rust docs/format.md"),
        ("bin/other", (), "HEAD", "bin-tests bin/other"),
        ("docs/notes.md", (), None, "bin-tests (no CHECK_BASE"),
        ("crates/app/src/lib.rs", ("--quick",), "HEAD", "rust crates/app/src/lib.rs"),
    ],
    ids=["a crate", "an embedded file", "a bin script", "no CHECK_BASE", "--quick"],
)
def test_outside_the_queue_a_change_that_needs_a_check_asks_for_it_first(
    repo, path, args, base, reason
):
    change(repo, path)
    ran, done = hook(repo, *args, env=QUEUE_EXIT, base=base, code=75)
    assert (repo.parent / "calls").read_text() == ""  # no cargo or uvx call at all
    assert f"agent-pre-commit: needs heavy's queue for {reason}" in done.stderr


def test_outside_the_queue_a_docs_only_change_passes_there(repo):
    change(repo, "docs/notes.md")
    assert run(repo, env=QUEUE_EXIT) == set()


@pytest.mark.parametrize("value", ["", "0", "075", "256", "seventy-five"])
def test_a_check_queue_exit_that_is_no_exit_status_from_1_to_255_is_ignored(
    repo, value
):
    change(repo, "crates/app/src/lib.rs", "pub fn f() {}\n")
    assert run(repo, env={"CHECK_QUEUE_EXIT": value}) == EVERY
    # The checks never see it.
    assert "saw-CHECK_QUEUE_EXIT" not in (repo.parent / "calls").read_text()


@pytest.mark.parametrize(
    "script", ["agent-pre-commit", "agent-quick-check", "agent-commit-check"]
)
def test_each_check_opts_in_to_a_first_run_outside_the_queue(script):
    assert "CHECK_QUEUE_EXIT" in (REPO / "bin" / script).read_text()


SKILL_MD = """---
name: demo
description: A demo skill.
metadata:
  version: "1"
---

# Demo
"""


@pytest.fixture
def skills_repo(repo) -> Path:
    """`repo` with a skill on record, as `bin/skills sync` leaves it, and no origin:
    the check's fetch fails, so it compares the tree with itself."""
    (repo / "docs/entry-format.md").write_text("# Format\n")
    (repo / "skills/demo").mkdir(parents=True)
    (repo / "skills/demo/SKILL.md").write_text(SKILL_MD)
    shutil.copy(REPO / "bin/skills", repo / "bin/skills")
    done = subprocess.run(
        [str(repo / "bin/skills"), "sync"],
        cwd=repo,
        capture_output=True,
        text=True,
        check=False,
    )
    assert done.returncode == 0, done.stdout + done.stderr
    commit(repo, "a skill")
    return repo


def edit_skill(repo: Path, text: str) -> None:
    change(repo, "skills/demo/SKILL.md", SKILL_MD.replace("# Demo", text))


def test_a_skill_change_synced_passes_with_no_build_or_test(skills_repo):
    edit_skill(skills_repo, "# Demo, edited")
    subprocess.run(
        [str(skills_repo / "bin/skills"), "sync"],
        cwd=skills_repo,
        check=True,
        capture_output=True,
    )
    git(skills_repo, "add", "-A")
    assert run(skills_repo) == set()


@pytest.mark.parametrize("args", [(), ("--quick",)], ids=["full", "quick"])
def test_a_skill_changed_without_sync_is_refused_in_both_modes(skills_repo, args):
    edit_skill(skills_repo, "# Demo, edited")
    _, done = hook(skills_repo, *args, code=1)
    assert "skills/versions.txt" in done.stdout + done.stderr
    assert "bin/skills sync" in done.stdout + done.stderr


def test_outside_the_queue_a_stale_skill_fails_at_once_not_after_the_queue(skills_repo):
    edit_skill(skills_repo, "# Demo, edited")
    change(skills_repo, "crates/app/src/lib.rs", "pub fn f() {}\n")
    _, done = hook(skills_repo, env=QUEUE_EXIT, code=1)
    assert (skills_repo.parent / "calls").read_text() == ""
    assert "skills/versions.txt" in done.stdout + done.stderr


def test_an_unrelated_change_runs_no_skills_check(skills_repo):
    # A stale skill already committed: a skills check that ran would refuse.
    edit_skill(skills_repo, "# Demo, stale")
    commit(skills_repo, "a stale skill")
    change(skills_repo, "docs/notes.md")
    assert run(skills_repo) == set()


def test_a_skills_script_that_cant_check_warns_and_passes(skills_repo):
    # A worktree forked before `sync --check` existed, say: argparse exits 2.
    old = (
        "#!/bin/sh\necho 'skills: error: unrecognized arguments: --check' >&2\nexit 2\n"
    )
    change(skills_repo, "bin/skills", old)
    (skills_repo / "bin/skills").chmod(0o755)
    git(skills_repo, "add", "bin/skills")
    edit_skill(skills_repo, "# Demo, edited")
    ran, done = hook(skills_repo)
    assert "pytest" in ran  # bin/skills changed, so the bin tests run as before
    assert "couldn't check the skills" in done.stderr


def test_a_stale_query_cache_refusal_names_sqlx_refresh(repo):
    change(repo, "crates/server/src/lib.rs", "fn changed() {}\n")
    stale_cargo = FAKE + (
        'case "$1" in clippy)\n'
        '  echo "error: set \\`DATABASE_URL\\` to use query macros online,'
        ' or run \\`cargo sqlx prepare\\` to update the query cache"\n'
        "  exit 101;;\nesac\n"
    )

    _, done = hook(repo, "--quick", env={"FAKE_CARGO": stale_cargo}, code=1)

    assert "run bin/sqlx-refresh, stage .sqlx/" in done.stdout + done.stderr


def test_another_clippy_failure_says_nothing_of_sqlx(repo):
    change(repo, "crates/server/src/lib.rs", "fn changed() {}\n")
    failing_cargo = (
        FAKE + 'case "$1" in clippy) echo "error: unused"; exit 101;; esac\n'
    )

    _, done = hook(repo, "--quick", env={"FAKE_CARGO": failing_cargo}, code=1)

    assert "sqlx-refresh" not in done.stdout + done.stderr


# The parts' timings the hook writes for agent-commit-gate, which names the file.


def timings(repo: Path, *args: str, **options) -> list[str]:
    """The names the hook wrote to $CHECK_TIMINGS, in order, each line checked to be
    a name and its seconds to the millisecond."""
    out = repo.parent / "timings"
    out.write_text("")
    hook(
        repo,
        *args,
        env={"CHECK_TIMINGS": str(out), **options.pop("env", {})},
        **options,
    )
    lines = out.read_text().splitlines()
    for line in lines:
        assert re.fullmatch(r"[a-z-]+ \d+\.\d{3}", line), line
    return [line.split(" ")[0] for line in lines]


def test_the_checks_the_hook_runs_never_see_check_timings(repo):
    # Its own tests run hooks that would write their fakes' timings there.
    fake = FAKE + 'echo "uvx saw ${CHECK_TIMINGS-nothing}" >> "$FAKE_LOG"\n'
    change(repo, "bin/other")
    assert timings(repo, env={"FAKE_UVX": fake}) == ["bin-tests"]
    assert "uvx saw nothing" in (repo.parent / "calls").read_text()


def test_a_failed_test_build_shows_its_output_and_runs_no_tests(repo):
    failing = (
        FAKE
        + 'case "$*" in *--no-run*) echo "error[E0425]: broken"; exit 101 ;; esac\n'
    )
    change(repo, "crates/app/src/lib.rs", "pub fn f() {}\n")
    _, done = hook(repo, env={"FAKE_CARGO": failing}, code=1)
    assert "error[E0425]: broken" in done.stdout + done.stderr
    calls = (repo.parent / "calls").read_text().splitlines()
    assert "cargo nextest run --cargo-quiet" not in calls, calls


def test_the_hook_writes_each_check_it_runs_with_its_seconds(repo):
    change(repo, "crates/app/src/lib.rs", "pub fn f() {}\n")
    assert timings(repo) == [
        "bin-tests",
        "nextest-build",
        "nextest",
        "doctest",
        "clippy",
    ]


def test_a_bin_only_change_times_only_bin_tests(repo):
    change(repo, "bin/other")
    assert timings(repo) == ["bin-tests"]


def test_the_commit_checks_rules_are_timed_too(repo):
    change(repo, "bin/other")
    assert timings(repo, "--quick", "--rules") == ["rules"]


def test_a_docs_change_times_nothing(repo):
    change(repo, "docs/notes.md")
    assert timings(repo) == []


def test_nextest_builds_before_it_runs(repo):
    change(repo, "crates/app/src/lib.rs", "pub fn f() {}\n")
    run(repo)
    nextest = [
        call
        for call in (repo.parent / "calls").read_text().splitlines()
        if call.startswith("cargo nextest run")
    ]
    assert nextest == [
        "cargo nextest run --cargo-quiet --no-run",
        "cargo nextest run --cargo-quiet",
    ]
# The checks run in one folder per worktree, which each run brings to what's staged, so
# cargo finds the files it built from where they were (#136). It lies in $TMPDIR, out
# of the repo, so no tool that searches parent folders for its settings (sqlx's .env,
# clippy.toml) finds the worktree's.


def check_state(repo: Path) -> Path:
    """The folder of the worktree's check: the copy, its index, the lock."""
    digest = hashlib.sha256(str(repo.resolve()).encode()).hexdigest()[:16]
    return repo.parent / "tmp" / f"filedrawer-check-{digest}"


def check_tree(repo: Path) -> Path:
    return check_state(repo) / "tree"


def logging_cargo(line: str) -> dict:
    """An env for hook() whose fake cargo appends the shell line `line`'s output to
    the log."""
    return {"FAKE_CARGO": f'#!/bin/sh\n{{ {line}; }} >> "$FAKE_LOG" 2>&1\n'}


def test_the_checks_run_in_the_worktrees_check_folder(repo):
    change(repo, "crates/app/src/extra.rs")

    hook(repo, "--quick", env=logging_cargo("pwd"))

    calls = (repo.parent / "calls").read_text().splitlines()
    assert calls == [str(check_tree(repo).resolve())]


def test_a_second_check_rewrites_only_the_files_that_changed(repo):
    change(repo, "crates/app/src/extra.rs", "one\n")
    hook(repo, "--quick")
    unchanged = check_tree(repo) / "docs/notes.md"
    assert unchanged.is_file()
    before = unchanged.stat().st_mtime_ns

    change(repo, "crates/app/src/extra.rs", "two\n")
    hook(repo, "--quick")

    assert unchanged.stat().st_mtime_ns == before
    assert (check_tree(repo) / "crates/app/src/extra.rs").read_text() == "two\n"


def test_the_check_folder_holds_exactly_what_is_staged(repo):
    change(repo, "crates/app/src/extra.rs", "one\n")
    hook(repo, "--quick")
    tree = check_tree(repo)
    assert tree.is_dir()
    (tree / "stray.rs").write_text("left behind\n")
    (tree / "stray").mkdir()
    (tree / "stray/inner.rs").write_text("left behind\n")
    (tree / "docs/notes.md").write_text("tampered\n")
    (tree / "bin/other").chmod(0o644)
    git(repo, "rm", "-q", "tool.toml")
    (repo / "conf/settings.toml").write_text("unstaged\n")

    change(repo, "crates/app/src/extra.rs", "two\n")
    hook(repo, "--quick")

    assert not (tree / "stray.rs").exists()
    assert not (tree / "stray").exists()
    assert (tree / "docs/notes.md").read_text() == "# Notes\n"
    assert os.access(tree / "bin/other", os.X_OK)
    assert not (tree / "tool.toml").exists()
    assert (tree / "conf/settings.toml").read_text() == ""


def test_git_in_the_check_folder_finds_no_repo(repo):
    # Else a check that runs git there would read the worktree, unstaged edits and all.
    change(repo, "crates/app/src/extra.rs")

    hook(
        repo,
        "--quick",
        env=logging_cargo("git rev-parse --show-toplevel || echo no-repo"),
    )

    assert (repo.parent / "calls").read_text().splitlines()[-1] == "no-repo"


def test_no_file_of_the_worktree_lies_above_the_check_folder(repo):
    # sqlx's macros read the nearest .env up from each crate, which sqlx-refresh
    # writes in a worktree: found, they'd build against its database, not .sqlx/.
    (repo / ".env").write_text("DATABASE_URL=sqlite://unstaged.db\n")
    change(repo, "crates/app/src/extra.rs")
    above = (
        'd=$(dirname "$PWD"); while [ "$d" != / ]; do'
        ' [ -e "$d/.env" ] && echo "found $d/.env"; d=$(dirname "$d"); done; echo looked'
    )

    hook(repo, "--quick", env=logging_cargo(above))

    assert (repo.parent / "calls").read_text().splitlines() == ["looked"]


def test_a_lock_file_a_killed_sync_left_doesnt_stop_the_next_check(repo):
    change(repo, "crates/app/src/extra.rs", "one\n")
    hook(repo, "--quick")
    (check_state(repo) / "index.lock").write_text("")

    change(repo, "crates/app/src/extra.rs", "two\n")
    hook(repo, "--quick")

    assert (check_tree(repo) / "crates/app/src/extra.rs").read_text() == "two\n"


def test_a_check_removes_the_folders_of_worktrees_that_are_gone(repo, tmp_path):
    gone = repo.parent / "tmp/filedrawer-check-0000000000000000"
    (gone / "tree").mkdir(parents=True)
    (gone / "worktree").write_text(str(tmp_path / "removed-worktree"))
    kept = repo.parent / "tmp/filedrawer-check-1111111111111111"
    (kept / "tree").mkdir(parents=True)
    (kept / "worktree").write_text(str(tmp_path))
    change(repo, "docs/notes.md")

    hook(repo)

    assert not gone.exists()
    assert (kept / "tree").is_dir()


def test_a_check_keeps_a_gone_worktrees_folder_while_its_check_runs(repo, tmp_path):
    busy = repo.parent / "tmp/filedrawer-check-2222222222222222"
    (busy / "tree").mkdir(parents=True)
    (busy / "worktree").write_text(str(tmp_path / "removed-worktree"))
    change(repo, "docs/notes.md")

    with (busy / "lock").open("w") as held:
        fcntl.flock(held, fcntl.LOCK_EX)
        hook(repo)

    assert (busy / "tree").is_dir()


def test_the_checks_processes_dont_hold_the_lock(repo):
    # A process a check leaves running, such as the sccache server cargo starts, would
    # otherwise hold the lock and stall every later check of the worktree.
    change(repo, "crates/app/src/extra.rs")

    hook(repo, "--quick", env=logging_cargo("ls -l /proc/$$/fd"))

    calls = (repo.parent / "calls").read_text()
    assert "/dev/null" in calls or "pipe:" in calls, calls
    assert "check-tree.lock" not in calls


def blocked_in_flock(pid: int) -> bool:
    """Whether a child of process `pid` is a flock that waits."""
    children = Path(f"/proc/{pid}/task/{pid}/children").read_text().split()
    for child in children:
        try:
            comm = Path(f"/proc/{child}/comm").read_text().strip()
            state = Path(f"/proc/{child}/stat").read_text().rsplit(")", 1)[1].split()[0]
        except FileNotFoundError:
            continue
        if comm == "flock" and state == "S":
            return True
    return False


def test_a_check_waits_for_another_check_of_the_worktree(repo):
    change(repo, "crates/app/src/extra.rs")
    calls = repo.parent / "calls"
    env = hook_env(repo, None, "HEAD")
    check_state(repo).mkdir(parents=True)
    with (check_state(repo) / "lock").open("w") as held:
        fcntl.flock(held, fcntl.LOCK_EX)
        check = subprocess.Popen(
            [str(repo / "bin/agent-pre-commit"), "--quick"],
            cwd=repo,
            env=env,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )
        try:
            assert check.stderr is not None
            said = check.stderr.readline()
            # Until it blocks on the lock, or gets past it, which fails below.
            waited = False
            while check.poll() is None and not calls.read_text():
                if blocked_in_flock(check.pid):
                    waited = True
                    break
            calls_while_held = calls.read_text()
        except BaseException:
            check.kill()
            check.wait()
            raise
    out, err = check.communicate()

    assert "waits for another check of this worktree" in said
    assert waited and calls_while_held == ""
    assert check.returncode == 0, out + said + err
    assert "cargo clippy" in calls.read_text()
