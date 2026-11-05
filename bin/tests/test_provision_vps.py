"""Tests for ops/provision-vps's helpers that run scripts on the VPS: remote and
remote_with_secret. A fake ssh runs the remote command line locally, through bash,
as the server's shell would, so the quoting is tested the way the server sees it.
"""

import os
import stat
import subprocess
from pathlib import Path

import pytest

REPO = Path(__file__).resolve().parents[2]
WIZARD = REPO / "ops/provision-vps"


@pytest.fixture
def run(tmp_path):
    fakes = tmp_path / "fakes"
    fakes.mkdir()
    ssh = fakes / "ssh"
    # The last argument is the remote command line; ssh hands it to the login shell.
    ssh.write_text('#!/bin/bash\nfor last in "$@"; do :; done\nexec bash -c "$last"\n')
    ssh.chmod(ssh.stat().st_mode | stat.S_IXUSR)
    remote = tmp_path / "remote"
    remote.mkdir()
    (remote / "show.sh").write_text(
        'echo "user=[${SMTP_USER-unset}] key=[${KEY-unset}] secret=[${SECRET-unset}]"\n'
    )

    def run(call: str) -> subprocess.CompletedProcess:
        script = f'PROVISION_VPS_SOURCE_ONLY=1 source "{WIZARD}"\nVPS_HOST=h\nREMOTE="{remote}"\n{call}\n'
        return subprocess.run(
            ["bash", "-c", script],
            env={**os.environ, "PATH": f"{fakes}:{os.environ['PATH']}"},
            capture_output=True,
            text=True,
            check=False,
        )

    return run


def test_remote_runs_a_script_with_no_settings(run):
    result = run("remote show.sh")
    assert result.returncode == 0, result.stderr
    assert result.stdout == "user=[unset] key=[unset] secret=[unset]\n"


def test_remote_passes_settings_with_spaces_and_quotes_whole(run):
    key = "ssh-ed25519 AAAAC3Nz it's a comment"
    result = run(f'remote show.sh "SMTP_USER=a b" "KEY={key}"')
    assert result.returncode == 0, result.stderr
    assert result.stdout == f"user=[a b] key=[{key}] secret=[unset]\n"


def test_remote_with_secret_reads_the_secret_from_stdin(run):
    result = run(
        "remote_with_secret show.sh SECRET 'pa ss$word\"%s%%' 'SMTP_USER=noreply@x.dev'"
    )
    assert result.returncode == 0, result.stderr
    assert (
        result.stdout == 'user=[noreply@x.dev] key=[unset] secret=[pa ss$word"%s%%]\n'
    )


def test_remote_with_secret_and_no_settings(run):
    result = run("remote_with_secret show.sh SECRET hunter2")
    assert result.returncode == 0, result.stderr
    assert result.stdout == "user=[unset] key=[unset] secret=[hunter2]\n"


def test_caddy_sh_splits_the_password_from_the_staging_key(tmp_path):
    """caddy.sh takes "<password><TAB><key>" on one line; a % or $ stays as written."""
    script = (REPO / "ops/remote/caddy.sh").read_text()
    split = script[: script.index("if ! command -v caddy")]
    probe = split + 'printf "%s|%s\\n" "$STAGING_PASSWORD" "$STAGING_KEY"\n'
    key = "ab" * 32
    result = subprocess.run(
        ["bash", "-c", probe],
        env={
            **os.environ,
            "STAGING_USER": "ada",
            "STAGING_SECRETS": f"50% off $ale\t{key}",
        },
        capture_output=True,
        text=True,
        check=False,
    )
    assert result.returncode == 0, result.stderr
    assert result.stdout == f"50% off $ale|{key}\n"
