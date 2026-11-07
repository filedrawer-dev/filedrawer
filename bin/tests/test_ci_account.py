"""Tests for ops/remote/ci-account.sh, which ops/provision-vps runs as root on the VPS
to make a CI account and token: the smoke test's, or the screen check's. It runs as a
process against a temp root, with a fake runuser first on PATH and a fake filedrawer
binary that keeps its accounts in a file. bin/agent-pre-commit runs them with pytest.
"""

import os
import stat
import subprocess
from pathlib import Path
from types import SimpleNamespace

import pytest

REPO = Path(__file__).resolve().parents[2]
SCRIPT = REPO / "ops/remote/ci-account.sh"

# The fake binary: create-account prints a-<n>; create-token prints fdt_<n> for a known
# account and fails as the real one does for an unknown one. FAIL_TOKEN makes
# create-token fail with another error.
FAKE_FILEDRAWER = """#!/bin/bash
state="$(dirname "$0")/state"
echo "filedrawer $* db=$FILEDRAWER_DATABASE" >> "$LOG"
case "$2" in
  create-account)
    n=$(( $(wc -l < "$state/accounts") + 1 ))
    echo "a-$n" >> "$state/accounts"
    echo "a-$n" ;;
  create-token)
    if [[ -n "${FAIL_TOKEN:-}" ]]; then echo "filedrawer: database is locked" >&2; exit 1; fi
    if ! grep -qx -- "$4" "$state/accounts"; then echo "filedrawer: no account $4" >&2; exit 1; fi
    n=$(( $(wc -l < "$state/tokens") + 1 ))
    echo "t$n" >> "$state/tokens"
    echo "fdt_$n" ;;
esac
"""


def executable(path: Path, text: str) -> Path:
    path.write_text(text)
    path.chmod(path.stat().st_mode | stat.S_IXUSR)
    return path


@pytest.fixture
def box(tmp_path):
    root = tmp_path / "root"
    bin_dir = root / "srv/filedrawer/staging/bin"
    bin_dir.mkdir(parents=True)
    state = bin_dir / "state"
    state.mkdir()
    (state / "accounts").write_text("")
    (state / "tokens").write_text("")
    executable(bin_dir / "filedrawer", FAKE_FILEDRAWER)
    fakes = tmp_path / "fakes"
    fakes.mkdir()
    log = tmp_path / "log"
    log.touch()
    # runuser records the user, then runs the command as the caller.
    executable(
        fakes / "runuser",
        f'#!/bin/sh\necho "runuser $*" >> {log}\nshift 3\nexec "$@"\n',
    )
    env = {
        "PATH": f"{fakes}:{os.environ['PATH']}",
        "FILEDRAWER_ROOT": str(root),
        "LOG": str(log),
    }
    return SimpleNamespace(root=root, bin=bin_dir, state=state, log=log, env=env)


def run(box, role="smoke", **env) -> subprocess.CompletedProcess:
    return subprocess.run(
        ["bash", SCRIPT],
        env={**box.env, "ROLE": role, **env},
        capture_output=True,
        text=True,
        check=False,
    )


def accounts(box) -> list[str]:
    return (box.state / "accounts").read_text().split()


def test_a_first_run_makes_the_account_and_a_token_as_the_service_user(box):
    result = run(box)
    assert result.returncode == 0, result.stderr
    assert result.stdout == "a-1\tfdt_1\n"
    log = box.log.read_text()
    assert "runuser -u filedrawer -- env" in log
    database = box.root / "srv/filedrawer/staging/data/filedrawer.db"
    assert f"db={database}" in log


@pytest.mark.parametrize("role", ["smoke", "screen"])
def test_a_rerun_keeps_the_account_and_adds_a_token(box, role):
    run(box, role=role)
    result = run(box, role=role, ACCOUNT="a-1")
    assert result.returncode == 0, result.stderr
    assert result.stdout == "a-1\tfdt_2\n"
    assert accounts(box) == ["a-1"]


def test_each_role_names_its_account_and_token(box):
    smoke = run(box)
    screen = run(box, role="screen")
    assert smoke.returncode == screen.returncode == 0, smoke.stderr + screen.stderr
    assert screen.stdout == "a-2\tfdt_2\n"
    log = box.log.read_text()
    assert "create-account --display-name Smoke test (CI)" in log
    assert "create-token --account a-1 --name smoke test (CI)" in log
    assert "create-account --display-name Screen check (CI)" in log
    assert "create-token --account a-2 --name screen check (CI)" in log


@pytest.mark.parametrize("role", ["", "admin"])
def test_another_role_is_refused_before_anything_is_made(box, role):
    result = run(box, role=role)
    assert result.returncode == 2, result.stderr
    assert result.stdout == ""
    assert "ROLE" in result.stderr
    assert accounts(box) == []


@pytest.mark.parametrize("role", ["smoke", "screen"])
def test_an_account_staging_lost_is_made_again(box, role):
    result = run(box, role=role, ACCOUNT="a-gone")
    assert result.returncode == 0, result.stderr
    assert result.stdout == "a-1\tfdt_1\n"
    assert "no account a-gone" in result.stderr


def test_another_token_error_stops_without_a_new_account(box):
    run(box)
    result = run(box, ACCOUNT="a-1", FAIL_TOKEN="1")
    assert result.returncode == 1
    assert result.stdout == ""
    assert "database is locked" in result.stderr
    assert accounts(box) == ["a-1"]


def test_with_no_binary_yet_it_exits_3(box):
    (box.bin / "filedrawer").unlink()
    result = run(box)
    assert result.returncode == 3
    assert result.stdout == ""
