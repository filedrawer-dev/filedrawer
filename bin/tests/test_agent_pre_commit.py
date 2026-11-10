"""bin/agent-pre-commit runs the cargo checks only when a path changed since
$CHECK_BASE is a build input, and bin/tests only when what those tests cover changed;
bin/checks-needed decides. llm-toolbox's agent-commit-gate sets CHECK_BASE to the
nearest commit that passed the hook; these tests set it themselves, to HEAD unless a
test says otherwise, as at an agent's commit. They run the real hook and
bin/checks-needed in a scratch workspace, the hook with fakes of cargo and uvx first on
PATH that note their arguments, so no real check runs. Which references count is asked
of bin/checks-needed directly, which is faster."""

import os
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
    # The crate's integration tests, one binary: a file and a folder with mod.rs,
    # each with its `mod` line, and a folder of data that is no module.
    "crates/app/tests/it/main.rs": "mod common;\nmod one;\n",
    "crates/app/tests/it/one.rs": "",
    "crates/app/tests/it/common/mod.rs": "",
    "crates/app/tests/it/fixtures/entry.md": "",
    "docs/format.md": "# Format\n",
    "docs/notes.md": "# Notes\n",
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
    git(repo, "-c", "user.name=t", "-c", "user.email=t@t", "commit", "-qm", message)


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
    git(root, "init", "-q")
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
    fakes = repo.parent / "fakes"
    fakes.mkdir(exist_ok=True)
    for tool in ("cargo", "uvx"):
        # FAKE_UVX in `env` replaces the fake uvx's text.
        (fakes / tool).write_text((env or {}).get(f"FAKE_{tool.upper()}", FAKE))
        (fakes / tool).chmod(0o755)
    log = repo.parent / "calls"
    log.write_text("")
    hook_env = {
        k: v
        for k, v in os.environ.items()
        if not k.startswith("GIT_") and k not in ("CHECK_BASE", "CHECK_QUEUE_EXIT")
    }
    hook_env |= {"PATH": f"{fakes}{os.pathsep}{os.environ['PATH']}"}
    if base is not None:
        hook_env["CHECK_BASE"] = git(repo, "rev-parse", base)
    hook_env |= {"FAKE_LOG": str(log), **(env or {})}
    done = subprocess.run(
        [str(repo / "bin" / script), *args],
        cwd=repo,
        env=hook_env,
        capture_output=True,
        text=True,
        check=False,
    )
    assert done.returncode == code, done.stdout + done.stderr
    calls = log.read_text().splitlines()
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
        ("ops/fixture.md", {"rust", "bin-tests"}),  # a crate's test reads it
        ("bin/helper", {"rust", "bin-tests"}),  # the script that test runs runs it
        # ops/run reaches these, so they're bin/tests' inputs too: they run it.
        ("conf/settings.toml", {"rust", "bin-tests"}),  # through the script's $repo
        ("conf/source.toml", {"rust", "bin-tests"}),  # through ${0%/*}
        ("conf/bash-source.toml", {"rust", "bin-tests"}),  # through BASH_SOURCE
        ("bin/sibling", {"rust", "bin-tests"}),  # a Python script's with_name
        ("conf/joined.toml", {"rust", "bin-tests"}),  # its joinpath
        ("data/any.json", {"rust", "bin-tests"}),  # the folder before a built name
        ("Cargo.lock", {"rust"}),
        (".sqlx/query-1.json", {"rust"}),
        (".gitattributes", {"rust", "bin-tests"}),
        ("crates/app/src/lib.rs", {"rust", "bin-tests"}),
        ("tool.toml", {"bin-tests"}),  # a bin test joins it onto the repo
        ("bin/other", {"bin-tests"}),
        ("docs/notes.md", set()),  # in prose and a scratch path only
        # The skills' record follows their folders and the format they copy.
        ("skills/demo/SKILL.md", {"skills", "routes"}),
        ("skills/versions.txt", {"skills", "routes"}),
        ("docs/entry-format.md", {"skills"}),
        ("bin/skills", {"skills", "bin-tests"}),
        # The rule tests read every file in bin/tests.
        ("bin/tests/test_tool.py", {"bin-tests", "py-rules"}),
        ("bin/tests/conftest.py", {"bin-tests", "py-rules"}),
        # The module check reads every crate's tests/it, and its own script.
        ("crates/app/tests/it/one.rs", {"rust", "bin-tests", "test-mods"}),
        ("crates/app/tests/it/main.rs", {"rust", "bin-tests", "test-mods"}),
        ("crates/app/tests/it/fixtures/entry.md", {"rust", "bin-tests", "test-mods"}),
        ("bin/test-modules", {"bin-tests", "test-mods"}),
        # The real-Caddy tests, their scripts and the files they live in.
        ("ops/remote/caddy.sh", {"bin-tests", "caddy"}),
        ("ops/remote/field-test-key.sh", {"bin-tests", "caddy"}),
        ("bin/caddy", {"bin-tests", "caddy"}),
        ("bin/tests/test_remote_environments.py", {"bin-tests", "py-rules", "caddy"}),
        ("bin/tests/test_field_test_key.py", {"bin-tests", "py-rules", "caddy"}),
        ("ops/remote/other.sh", {"bin-tests"}),
    ],
)
def test_the_checks_a_change_to_each_reference_needs(template, path, checks):
    assert needed(template, path) == checks


RULE_TEST = "bin/tests/test_no_wall_clock_limits.py"


def uvx_calls(repo: Path) -> list[str]:
    return [
        call
        for call in (repo.parent / "calls").read_text().splitlines()
        if call.startswith("uvx ")
    ]


def test_the_commit_check_runs_the_wall_clock_rule_for_a_bin_tests_change(repo):
    change(repo, "bin/tests/test_tool.py", "def test_tool():\n    pass\n")

    hook(repo, env=QUEUE_EXIT, script="agent-commit-check")

    calls = uvx_calls(repo)
    assert len(calls) == 1, calls
    assert calls[0].endswith(f"-n0 {RULE_TEST}"), calls
    assert "pytest@9.2.0" in calls[0]


def test_the_rule_runs_only_with_rules_and_only_for_a_bin_tests_change(repo):
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

    calls = uvx_calls(repo)
    assert len(calls) == 1, calls
    assert calls[0].endswith(CADDY_RUN), calls
    assert "pytest@9.2.0" in calls[0]


def test_outside_the_queue_the_real_caddy_tests_ask_for_it_first(repo):
    change(repo, "bin/caddy")

    _, done = hook(repo, env=QUEUE_EXIT, script="agent-commit-check", code=75)

    assert uvx_calls(repo) == []
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
        "test_a_real_caddy_answers_503_while_production_is_down_and_redirects_www",
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
        "uvx --quiet --with pytest-timeout==2.4.0 --with pytest-xdist==3.8.0 pytest@9.2.0"
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
