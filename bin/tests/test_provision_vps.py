"""Tests for ops/provision-vps's helpers that run scripts on the VPS: remote and
remote_with_secret. A fake ssh runs the remote command line locally, through bash,
as the server's shell would, so the quoting is tested the way the server sees it.
"""

import os
import stat
import subprocess
from pathlib import Path
from types import SimpleNamespace

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


# The smoke stage, run alone: ops/provision-vps smoke


STAGING_KEY = "ab" * 32


@pytest.fixture
def smoke(tmp_path):
    """Runs `ops/provision-vps smoke` in a folder with an ops/.vps.env, against a fake
    ssh that answers as ops/remote/smoke-account.sh would and a fake gh that keeps the
    secrets it is given."""
    fakes = tmp_path / "fakes"
    fakes.mkdir()
    log = tmp_path / "log"
    log.touch()
    secrets = tmp_path / "secrets"
    secrets.mkdir()
    (fakes / "ssh").write_text(
        "#!/bin/bash\n"
        'for last in "$@"; do :; done\n'
        f'echo "ssh $last" >> {log}\n'
        f"cat > {tmp_path}/sent-script\n"
        f"exit_status=$(cat {tmp_path}/ssh-exit 2>/dev/null || echo 0)\n"
        '[[ $exit_status == 0 ]] && printf "a-new\\tfdt_s3cret\\n"\n'
        "exit $exit_status\n"
    )
    (fakes / "gh").write_text(
        "#!/bin/bash\n"
        f'echo "gh $*" >> {log}\n'
        'case "$1 $2" in\n'
        '  "auth status") exit 0 ;;\n'
        f'  "secret list") ls {secrets} ;;\n'
        f'  "secret set") [[ "$4 $5" == "--env staging" ]] && cat > {secrets}/"$3" ;;\n'
        "esac\n"
    )
    for fake in fakes.iterdir():
        fake.chmod(fake.stat().st_mode | stat.S_IXUSR)
    ops = tmp_path / "ops"
    ops.mkdir()
    env_file = ops / ".vps.env"
    env_file.write_text(f"VPS_HOST='h'\nSTAGING_KEY='{STAGING_KEY}'\n")

    def run(answers: str = "\n") -> subprocess.CompletedProcess:
        return subprocess.run(
            ["bash", WIZARD, "smoke"],
            cwd=tmp_path,
            input=answers,
            env={**os.environ, "PATH": f"{fakes}:{os.environ['PATH']}"},
            capture_output=True,
            text=True,
            check=False,
        )

    return SimpleNamespace(
        run=run, tmp=tmp_path, log=log, secrets=secrets, env_file=env_file
    )


def test_smoke_stores_the_token_and_key_as_staging_secrets_and_shows_neither(smoke):
    result = smoke.run()
    assert result.returncode == 0, result.stdout + result.stderr
    assert (smoke.secrets / "SMOKE_TEST_TOKEN").read_text() == "fdt_s3cret"
    assert (smoke.secrets / "STAGING_KEY").read_text() == STAGING_KEY
    assert "SMOKE_ACCOUNT='a-new'" in smoke.env_file.read_text()
    shown = result.stdout + result.stderr
    assert "fdt_s3cret" not in shown
    assert STAGING_KEY not in shown
    assert "fdt_s3cret" not in smoke.log.read_text()
    sent = (smoke.tmp / "sent-script").read_text()
    assert sent == (REPO / "ops/remote/smoke-account.sh").read_text()


def test_a_rerun_passes_the_known_account_and_keeps_the_token_unless_asked(smoke):
    smoke.run()
    (smoke.secrets / "SMOKE_TEST_TOKEN").write_text("fdt_kept")
    kept = smoke.run("n\n\n")
    assert kept.returncode == 0, kept.stdout + kept.stderr
    assert (smoke.secrets / "SMOKE_TEST_TOKEN").read_text() == "fdt_kept"
    assert smoke.log.read_text().count("ssh ") == 1
    renewed = smoke.run("y\n\n")
    assert renewed.returncode == 0, renewed.stdout + renewed.stderr
    assert (smoke.secrets / "SMOKE_TEST_TOKEN").read_text() == "fdt_s3cret"
    assert "SMOKE_ACCOUNT=a-new bash -s" in smoke.log.read_text()


def test_before_the_first_deploy_it_says_to_rerun_and_still_sets_the_key(smoke):
    (smoke.tmp / "ssh-exit").write_text("3")
    result = smoke.run()
    assert result.returncode == 0, result.stdout + result.stderr
    assert "ops/provision-vps smoke" in result.stdout
    assert not (smoke.secrets / "SMOKE_TEST_TOKEN").exists()
    assert (smoke.secrets / "STAGING_KEY").read_text() == STAGING_KEY


def test_a_failure_on_the_server_stops_the_wizard(smoke):
    (smoke.tmp / "ssh-exit").write_text("1")
    result = smoke.run()
    assert result.returncode == 1
    assert not (smoke.secrets / "SMOKE_TEST_TOKEN").exists()
