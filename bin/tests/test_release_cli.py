"""Tests for ops/release-cli, the owner's command that starts a release of the
`filedrawer` command: .github/workflows/release-cli.yml on main. It runs against
conftest's FakeHost, whose gh notes each call."""

import subprocess
from pathlib import Path

import pytest

from conftest import FakeHost

SCRIPT = Path(__file__).resolve().parents[2] / "ops/release-cli"
DISPATCH = "workflow run release-cli.yml --repo filedrawer-dev/filedrawer --ref main"


@pytest.fixture
def host(tmp_path):
    return FakeHost(tmp_path)


def run(host: FakeHost, *args: str):
    return subprocess.run(
        [str(SCRIPT), *args], env=host.env, capture_output=True, text=True, check=False
    )


def test_it_starts_the_workflow_on_main_with_the_version_given(host):
    done = run(host, "0.1.2")
    assert done.returncode == 0, done.stderr
    assert host.calls("gh") == [f"{DISPATCH} -f version=0.1.2"]
    assert "actions/workflows/release-cli.yml" in done.stdout


def test_without_a_version_it_releases_mains_as_it_is(host):
    done = run(host)
    assert done.returncode == 0, done.stderr
    assert host.calls("gh") == [DISPATCH]


@pytest.mark.parametrize("args", [["v0.1.2"], ["0.1"], ["0.1.2", "extra"]])
def test_other_arguments_are_refused_before_gh_runs(host, args):
    done = run(host, *args)
    assert done.returncode == 2
    assert "usage" in done.stderr
    assert host.calls("gh") == []


def test_a_dispatch_gh_refuses_fails(host):
    host.answer("workflow run *", status=1, tool="gh")
    done = run(host, "0.1.2")
    assert done.returncode == 1


def test_help_says_what_it_starts_and_what_the_run_waits_for(host):
    done = run(host, "-h")
    assert done.returncode == 0
    assert "release-cli.yml" in done.stdout and "check" in done.stdout
    assert host.calls("gh") == []
