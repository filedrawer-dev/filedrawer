"""Tests for the deploy scripts that run on the VPS: ops/remote/filedrawer-receive (the
deploy key's forced command) and ops/remote/filedrawer-install (run as root through
sudo). They run as processes against a temp root, with fake sudo, runuser, systemctl
and curl first on PATH. bin/agent-pre-commit runs them with pytest.
"""

import fcntl
import hashlib
import os
import re
import stat
import subprocess
from pathlib import Path
from types import SimpleNamespace

import pytest

REPO = Path(__file__).resolve().parents[2]
RECEIVE = REPO / "ops/remote/filedrawer-receive"
INSTALL = REPO / "ops/remote/filedrawer-install"


def executable(path: Path, text: str) -> Path:
    path.write_text(text)
    path.chmod(path.stat().st_mode | stat.S_IXUSR)
    return path


@pytest.fixture
def box(tmp_path):
    """A fake server root, fake commands, and a log of what they were asked."""
    root = tmp_path / "root"
    state = root / "var/lib/filedrawer-deploy"
    bin_dir = root / "srv/filedrawer/staging/bin"
    state.mkdir(parents=True)
    bin_dir.mkdir(parents=True)
    fakes = tmp_path / "fakes"
    fakes.mkdir()
    log = tmp_path / "log"
    log.touch()
    # sudo records its arguments and the size of what it was sent on stdin.
    executable(
        fakes / "sudo",
        f'#!/bin/sh\necho "sudo $* <$(wc -c | tr -d " ") bytes" >> {log}\n',
    )
    # runuser records the user, then runs the command as the caller.
    executable(
        fakes / "runuser",
        f'#!/bin/sh\necho "runuser $*" >> {log}\nshift 3\nexec "$@"\n',
    )
    executable(fakes / "systemctl", f'#!/bin/sh\necho "systemctl $*" >> {log}\n')
    # curl succeeds unless the file `unhealthy` exists.
    executable(
        fakes / "curl",
        f'#!/bin/sh\necho "curl $*" >> {log}\n[ ! -e {tmp_path}/unhealthy ]\n',
    )
    env = {
        "PATH": f"{fakes}:{os.environ['PATH']}",
        "FILEDRAWER_ROOT": str(root),
        "FILEDRAWER_HEALTH_SECONDS": "2",
    }
    return SimpleNamespace(
        tmp=tmp_path, root=root, state=state, bin=bin_dir, log=log, env=env
    )


def receive(box, command: str, data: bytes) -> subprocess.CompletedProcess:
    env = {**box.env, "SSH_ORIGINAL_COMMAND": command}
    return subprocess.run(
        [RECEIVE], input=data, env=env, capture_output=True, check=False
    )


def install(box, *args: str, data: bytes = b"", **env) -> subprocess.CompletedProcess:
    return subprocess.run(
        [INSTALL, *args],
        input=data,
        env={**box.env, **env},
        capture_output=True,
        check=False,
    )


def binary(version: str, runs: bool = True) -> bytes:
    """A stand-in server binary: a script that answers -h, or fails."""
    return f"#!/bin/sh\n# {version}\nexit {0 if runs else 1}\n".encode()


def sha(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


# filedrawer-receive


def test_receive_pipes_the_upload_to_install_with_its_hash(box):
    data = binary("v1")
    result = receive(box, f"deploy staging {sha(data)}", data)
    assert result.returncode == 0, result.stderr
    assert box.log.read_text() == (
        f"sudo -n /usr/local/sbin/filedrawer-install staging {sha(data)}"
        f" <{len(data)} bytes\n"
    )


@pytest.mark.parametrize(
    "command",
    [
        "",
        "deploy production " + "0" * 64,
        "deploy staging " + "0" * 63,
        "deploy staging " + "G" * 64,
        "deploy staging " + "0" * 64 + "; rm -rf /",
        "sh",
    ],
)
def test_receive_refuses_any_other_command(box, command):
    result = receive(box, command, b"x")
    assert result.returncode != 0
    assert b"refused" in result.stderr
    assert box.log.read_text() == ""


def test_receive_runs_one_deploy_at_a_time(box):
    data = binary("v1")
    with open(box.state / "lock", "w") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        result = receive(box, f"deploy staging {sha(data)}", data)
    assert result.returncode != 0
    assert b"another deploy" in result.stderr
    assert box.log.read_text() == ""


# filedrawer-install


def test_install_swaps_in_the_new_binary_and_keeps_the_old_one(box):
    executable(box.bin / "filedrawer", binary("v1").decode())
    data = binary("v2")
    result = install(box, "staging", sha(data), data=data)
    assert result.returncode == 0, result.stderr
    assert b"v2" in (box.bin / "filedrawer").read_bytes()
    assert b"v1" in (box.bin / "filedrawer.previous").read_bytes()
    log = box.log.read_text()
    assert "runuser -u filedrawer -- timeout 10 " in log
    assert "systemctl restart filedrawer-staging" in log
    assert "http://127.0.0.1:8081/tags" in log


def test_install_refuses_a_hash_mismatch_and_changes_nothing(box):
    executable(box.bin / "filedrawer", binary("v1").decode())
    result = install(box, "staging", "0" * 64, data=binary("v2"))
    assert result.returncode != 0
    assert b"SHA-256" in result.stderr
    assert b"v1" in (box.bin / "filedrawer").read_bytes()
    assert sorted(p.name for p in box.bin.iterdir()) == ["filedrawer"]
    assert box.log.read_text() == ""


def test_install_refuses_an_upload_over_the_cap(box):
    data = b"x" * 101
    result = install(box, "staging", sha(data), data=data, FILEDRAWER_MAX_BYTES="100")
    assert result.returncode != 0
    assert b"over" in result.stderr
    assert box.log.read_text() == ""


def test_install_rolls_back_when_the_new_server_isnt_healthy(box):
    executable(box.bin / "filedrawer", binary("v1").decode())
    (box.tmp / "unhealthy").touch()
    data = binary("v2")
    result = install(box, "staging", sha(data), data=data)
    assert result.returncode != 0
    assert b"rolled back" in result.stderr
    assert b"v1" in (box.bin / "filedrawer").read_bytes()
    assert box.log.read_text().count("systemctl restart filedrawer-staging") == 2


def test_install_never_installs_a_binary_that_doesnt_run(box):
    executable(box.bin / "filedrawer", binary("v1").decode())
    data = binary("v2", runs=False)
    result = install(box, "staging", sha(data), data=data)
    assert result.returncode != 0
    assert b"doesn't run" in result.stderr
    assert b"v1" in (box.bin / "filedrawer").read_bytes()
    assert "systemctl" not in box.log.read_text()


@pytest.mark.parametrize(
    "args",
    [
        ["production", "0" * 64],
        ["../staging", "0" * 64],
        ["staging", "not-a-hash"],
        ["staging"],
        ["staging", "0" * 64, "extra"],
    ],
)
def test_install_refuses_other_arguments(box, args):
    result = install(box, *args, data=binary("v2"))
    assert result.returncode != 0
    assert b"refused" in result.stderr
    assert not (box.bin / "filedrawer").exists()


# The workflow and the wizard


def test_every_secret_and_variable_the_workflow_reads_is_one_the_wizard_sets():
    workflow = (REPO / ".github/workflows/ci.yml").read_text()
    wizard = (REPO / "ops/provision-vps").read_text()
    read = set(re.findall(r"\$\{\{\s*(?:secrets|vars)\.(\w+)\s*\}\}", workflow))
    set_by_wizard = set(
        re.findall(r"(?:set_var|gh secret set|gh variable set)\s+(\w+)", wizard)
    )
    assert read, "the workflow reads no secrets or variables"
    assert read <= set_by_wizard, read - set_by_wizard


def test_the_settings_the_vps_scripts_write_are_the_ones_the_server_reads():
    written = set()
    for script in ("staging-service.sh", "mail.sh"):
        text = (REPO / "ops/remote" / script).read_text()
        written |= set(re.findall(r"^(FILEDRAWER_\w+)=", text, re.MULTILINE))
    config = (REPO / "crates/server/src/config.rs").read_text()
    read = set(re.findall(r'"(FILEDRAWER_\w+)"', config))
    assert written <= read, written - read
    required = {
        "FILEDRAWER_DATABASE",
        "FILEDRAWER_LISTEN",
        "FILEDRAWER_ENVIRONMENT",
        "FILEDRAWER_SCREENER",
        "FILEDRAWER_PUBLIC_URL",
    }
    assert required <= written, required - written
