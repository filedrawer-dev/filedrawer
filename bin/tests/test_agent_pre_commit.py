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
    for script in ("agent-pre-commit", "agent-quick-check", "checks-needed"):
        shutil.copy(REPO / "bin" / script, root / "bin" / script)
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
) -> tuple[set[str], subprocess.CompletedProcess]:
    """Run the hook with fake cargo and uvx, and CHECK_BASE set to `base` as
    `git rev-parse` gives it (None: unset), and check it exits with `code`; return the
    checks it ran (nextest, doctest, clippy, pytest) and the hook's run."""
    fakes = repo.parent / "fakes"
    fakes.mkdir(exist_ok=True)
    for tool in ("cargo", "uvx"):
        (fakes / tool).write_text(FAKE)
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
        [str(repo / "bin/agent-pre-commit"), *args],
        cwd=repo,
        env=hook_env,
        capture_output=True,
        text=True,
        timeout=60,
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
        timeout=10,
        check=False,
    )
    assert done.returncode == 0, done.stdout + done.stderr
    return {line.split(" ", 1)[0] for line in done.stdout.splitlines()}


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
    ],
)
def test_the_checks_a_change_to_each_reference_needs(template, path, checks):
    assert needed(template, path) == checks


def test_names_a_python_script_builds_from_each_other_are_read_in_time(repo):
    (repo / "bin/tests/test_cycle.py").write_text(
        "from pathlib import Path\n"
        "a = Path(__file__).parent\n"
        'a = b / "x"\n'
        'b = a / "y"\n'
    )
    assert needed(repo, "docs/notes.md") == set()


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
    "script", ["bin/checks-needed", "bin/agent-pre-commit", "bin/agent-quick-check"]
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


@pytest.mark.parametrize("script", ["agent-pre-commit", "agent-quick-check"])
def test_both_checks_opt_in_to_a_first_run_outside_the_queue(script):
    assert "CHECK_QUEUE_EXIT" in (REPO / "bin" / script).read_text()
