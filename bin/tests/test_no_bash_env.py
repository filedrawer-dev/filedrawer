"""bin/tests/conftest.py's guard: no bash the tests start reads a startup file first.
The sandbox sets BASH_ENV=/etc/sandbox-persistent.sh, which every non-interactive
bash sources, about 10 ms of CPU each, for more than 1,500 bashes a run (#168). CI
and the owner's host set no BASH_ENV, so dropping it runs the tests as they run there.
The guard drops it when conftest is imported, so a module's or the session's fixtures
start their bashes without it too, not only the tests."""

import subprocess

import pytest

UNSET = "unset"


def bash_env_seen() -> str:
    seen = subprocess.run(
        ["bash", "-c", f'printf %s "${{BASH_ENV-{UNSET}}}"'],
        capture_output=True,
        text=True,
        check=True,
    )
    return seen.stdout


@pytest.fixture(scope="module")
def seen_by_a_module_fixture():
    return bash_env_seen()


def test_a_shell_a_test_starts_has_no_bash_env():
    assert bash_env_seen() == UNSET


def test_a_shell_a_module_fixture_starts_has_no_bash_env(seen_by_a_module_fixture):
    assert seen_by_a_module_fixture == UNSET
