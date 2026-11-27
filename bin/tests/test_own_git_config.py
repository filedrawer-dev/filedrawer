"""bin/tests/conftest.py's guard: each test's git reads a config of the tests' own, not
the machine's. The sandbox's global config rewrites `git@github.com:` to HTTPS and
names an identity, CI's runner and the owner's host hold others, and a hook or a
rebase that runs the suite exports GIT_DIR and the like, which would send a test's
git to the real repository (#182). The guard is session-wide, so a module's fixtures
get it too, not only the tests."""

import os
import re
import subprocess
from pathlib import Path

import pytest


def git_config(*args: str, cwd=None) -> subprocess.CompletedProcess:
    return subprocess.run(
        ["git", "config", *args], capture_output=True, text=True, cwd=cwd
    )


def global_config() -> str:
    return git_config("--global", "--list").stdout


@pytest.fixture(scope="module")
def seen_by_a_module_fixture():
    return global_config()


def test_a_tests_git_reads_only_the_tests_global_config():
    assert global_config().splitlines() == [
        "user.name=t",
        "user.email=t@example.com",
        "init.defaultbranch=main",
        "core.editor=false",
    ]


def test_a_module_fixtures_git_reads_the_same_config(seen_by_a_module_fixture):
    assert seen_by_a_module_fixture == global_config()


def test_git_reads_no_config_but_the_tests(tmp_path):
    """No system config, nor any other file: every value git lists comes from the
    tests' global config."""
    listed = git_config("--list", "--show-origin", cwd=tmp_path).stdout.splitlines()
    origins = {line.split("\t", 1)[0] for line in listed}
    assert origins == {f"file:{os.environ['GIT_CONFIG_GLOBAL']}"}


def test_no_url_rewrite_reaches_a_test(tmp_path):
    subprocess.run(["git", "init", "-q", str(tmp_path)], check=True)
    url = "git@github.com:o/r.git"
    git_config("remote.origin.url", url, cwd=tmp_path)
    got = subprocess.run(
        ["git", "remote", "get-url", "origin"],
        capture_output=True,
        text=True,
        cwd=tmp_path,
        check=True,
    )
    assert got.stdout.strip() == url


def test_a_test_cant_write_the_global_config(tmp_path_factory):
    """A test that sets a global value would leak it to the tests beside it. The write
    is tried only once the global config is known to be the tests' own, so a broken
    guard fails here rather than writing the machine's ~/.gitconfig."""
    config = os.environ.get("GIT_CONFIG_GLOBAL")
    basetemp = tmp_path_factory.getbasetemp().resolve()
    assert config and Path(config).resolve().is_relative_to(basetemp)
    assert git_config("--global", "x.y", "z").returncode != 0


def test_no_variable_sends_git_to_another_repository():
    leaked = [name for name in os.environ if name.startswith("GIT_")]
    assert sorted(leaked) == ["GIT_CONFIG_GLOBAL", "GIT_CONFIG_NOSYSTEM"]


# An env built from os.environ minus every GIT_* variable drops the guard's two, and
# its git reads the machine's config again.
DROPS_THE_GUARD = re.compile(r"""not \w+\.startswith\(\(?["']GIT_""")


def test_no_test_drops_the_guards_variables_from_a_processs_env():
    here = Path(__file__).parent
    found = [
        f"{path.name}:{n}"
        for path in sorted(here.glob("*.py"))
        for n, line in enumerate(path.read_text().splitlines(), 1)
        if DROPS_THE_GUARD.search(line)
    ]
    assert found == []
