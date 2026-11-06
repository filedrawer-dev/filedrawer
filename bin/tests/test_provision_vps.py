"""Tests for ops/provision-vps's helpers that run scripts on the VPS: remote and
remote_with_secret. A fake ssh runs the remote command line locally, through bash,
as the server's shell would, so the quoting is tested the way the server sees it.
"""

import os
import subprocess
from pathlib import Path
from types import SimpleNamespace

import pytest
from conftest import opened_urls

REPO = Path(__file__).resolve().parents[2]
WIZARD = REPO / "ops/provision-vps"


@pytest.fixture
def run(tmp_path, fake_host):
    fake_host.run_locally()
    remote = tmp_path / "remote"
    remote.mkdir()
    (remote / "show.sh").write_text(
        'echo "user=[${SMTP_USER-unset}] key=[${KEY-unset}] secret=[${SECRET-unset}]"\n'
    )

    def run(call: str) -> subprocess.CompletedProcess:
        script = f'PROVISION_VPS_SOURCE_ONLY=1 source "{WIZARD}"\nVPS_HOST=h\nREMOTE="{remote}"\n{call}\n'
        return subprocess.run(
            ["bash", "-c", script],
            stdin=subprocess.DEVNULL,
            env=fake_host.env,
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
def smoke(tmp_path, fake_host):
    """Runs `ops/provision-vps smoke` in a folder with an ops/.vps.env, against a fake
    ssh that answers as ops/remote/smoke-account.sh would."""
    # Only smoke-account.sh answers with an account and a token.
    fake_host.answer("*", stdout="a-new\tfdt_s3cret\n", stdin="create-token")
    ops = tmp_path / "ops"
    ops.mkdir()
    env_file = ops / ".vps.env"
    env_file.write_text(f"VPS_HOST='h'\nSTAGING_KEY='{STAGING_KEY}'\n")

    def run(answers: str = "\n") -> subprocess.CompletedProcess:
        return subprocess.run(
            ["bash", WIZARD, "smoke"],
            cwd=tmp_path,
            input=answers,
            env=fake_host.env,
            capture_output=True,
            text=True,
            check=False,
        )

    return SimpleNamespace(
        run=run,
        tmp=tmp_path,
        host=fake_host,
        log=fake_host.log,
        secrets=fake_host.secrets,
        env_file=env_file,
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
    sent = smoke.host.sent(1)
    assert sent == (REPO / "ops/remote/smoke-account.sh").read_text()


def test_smoke_also_names_the_screen_test_account_it_finds_in_ops(smoke):
    (smoke.tmp / "ops/.screen-test.env").write_text(
        "SCREEN_TEST_ACCOUNT='a-screen1'\nSCREEN_TEST_TOKEN='fdt_secret'\n"
    )
    result = smoke.run()
    assert result.returncode == 0, result.stdout + result.stderr
    calls = [c for c in smoke.log.read_text().splitlines() if c.startswith("ssh ")]
    assert (
        calls[-1] == "ssh env SMOKE_ACCOUNT=a-new SCREEN_TEST_ACCOUNT=a-screen1 bash -s"
    )
    assert "fdt_secret" not in smoke.log.read_text() + result.stdout + result.stderr


def test_smoke_names_the_account_to_staging_so_its_entries_arent_rejected(smoke):
    result = smoke.run()
    assert result.returncode == 0, result.stdout + result.stderr
    calls = [c for c in smoke.log.read_text().splitlines() if c.startswith("ssh ")]
    assert calls[-1] == "ssh env SMOKE_ACCOUNT=a-new bash -s"
    sent = smoke.host.sent(len(calls))
    assert sent == (REPO / "ops/remote/smoke-env.sh").read_text()
    # The unit first, so it reads the file smoke-env.sh writes.
    unit = smoke.host.sent(len(calls) - 1)
    assert unit == (REPO / "ops/remote/staging-service.sh").read_text()


def test_a_rerun_passes_the_known_account_and_keeps_the_token_unless_asked(smoke):
    smoke.run()
    (smoke.secrets / "SMOKE_TEST_TOKEN").write_text("fdt_kept")
    kept = smoke.run("n\n\n")
    assert kept.returncode == 0, kept.stdout + kept.stderr
    assert (smoke.secrets / "SMOKE_TEST_TOKEN").read_text() == "fdt_kept"
    # The kept run sends only the setting, not smoke-account.sh.
    calls = [c for c in smoke.log.read_text().splitlines() if c.startswith("ssh ")]
    assert len(calls) == 5, calls
    assert smoke.host.sent(5) == (REPO / "ops/remote/smoke-env.sh").read_text()
    renewed = smoke.run("y\n\n")
    assert renewed.returncode == 0, renewed.stdout + renewed.stderr
    assert (smoke.secrets / "SMOKE_TEST_TOKEN").read_text() == "fdt_s3cret"
    assert "SMOKE_ACCOUNT=a-new bash -s" in smoke.log.read_text()


def test_before_the_first_deploy_it_says_to_rerun_and_still_sets_the_key(smoke):
    smoke.host.answer("*", status=3)
    result = smoke.run()
    assert result.returncode == 0, result.stdout + result.stderr
    assert "ops/provision-vps smoke" in result.stdout
    assert not (smoke.secrets / "SMOKE_TEST_TOKEN").exists()
    assert (smoke.secrets / "STAGING_KEY").read_text() == STAGING_KEY


def test_a_failure_on_the_server_stops_the_wizard(smoke):
    smoke.host.answer("*", status=1)
    result = smoke.run()
    assert result.returncode == 1
    assert not (smoke.secrets / "SMOKE_TEST_TOKEN").exists()


# The screening and admin stages, run alone: ops/provision-vps screening | admin

API_KEY = "sk-ant-api03-" + "k" * 40


@pytest.fixture
def stage(tmp_path, fake_host):
    """Runs `ops/provision-vps <name>` in a folder with an ops/.vps.env, against
    fake_host, with the URLs the wizard opens in `opened`."""
    ops = tmp_path / "ops"
    ops.mkdir()
    env_file = ops / ".vps.env"
    env_file.write_text("VPS_HOST='h'\n")

    def run(name: str, answers: str) -> subprocess.CompletedProcess:
        return subprocess.run(
            ["bash", WIZARD, name],
            cwd=tmp_path,
            input=answers,
            env=fake_host.env,
            capture_output=True,
            text=True,
            check=False,
        )

    return SimpleNamespace(
        run=run,
        host=fake_host,
        log=fake_host.log,
        sent=fake_host.sent,
        env_file=env_file,
        opened=opened_urls(),
    )


def test_screening_sends_the_key_on_stdin_and_saves_only_the_cap(stage):
    result = stage.run("screening", f"0.25\n{API_KEY}\n{API_KEY}\n\n")
    assert result.returncode == 0, result.stdout + result.stderr
    # The unit first, so it reads the new file, then the settings.
    assert stage.sent(1) == (REPO / "ops/remote/staging-service.sh").read_text()
    assert (
        stage.sent(2) == f"{API_KEY}\n" + (REPO / "ops/remote/screening.sh").read_text()
    )
    log = stage.log.read_text()
    assert "read -r API_KEY" in log
    assert "DAILY_USD=0.25" in log
    assert "SCREENING_DAILY_USD='0.25'" in stage.env_file.read_text()
    assert stage.opened.read_text() == "https://console.anthropic.com/settings/keys\n"
    for where in (log, stage.env_file.read_text(), result.stdout + result.stderr):
        assert API_KEY not in where


def test_screening_stops_when_the_two_keys_differ(stage):
    result = stage.run("screening", f"1\n{API_KEY}\nsk-ant-other\n\n")
    assert result.returncode == 1
    assert "API_KEY" not in stage.log.read_text()


def test_admin_refuses_a_mistyped_id_and_saves_nothing(stage):
    result = stage.run("admin", "A-abc 123\n\n")
    assert result.returncode == 1
    assert "ADMIN_ACCOUNT" not in stage.env_file.read_text()
    assert "admin-grant" not in stage.log.read_text()


def test_admin_grants_the_account_named(stage):
    result = stage.run("admin", "a-abc123\n\n")
    assert result.returncode == 0, result.stdout + result.stderr
    log = stage.log.read_text().splitlines()
    assert log[0] == "ssh test -x /srv/filedrawer/staging/bin/filedrawer"
    assert log[1] == "ssh env ADMIN_ACCOUNT=a-abc123 bash -s"
    assert stage.sent(2) == (REPO / "ops/remote/admin-grant.sh").read_text()


def test_admin_before_the_first_deploy_asks_nothing_and_says_to_rerun(stage):
    stage.host.answer("*", status=1)
    result = stage.run("admin", "\n")
    assert result.returncode == 0, result.stdout + result.stderr
    assert "ops/provision-vps admin" in result.stdout
    assert "account ID" not in result.stdout
    assert stage.log.read_text().count("ssh ") == 1


# The whole wizard, rerun with --only-new on a server it has set up


@pytest.fixture
def rerun(tmp_path, fake_host):
    """Runs `ops/provision-vps --only-new` with empty stdin, so any prompt fails it,
    against fake_host answering as a server it has set up, and an ops/.vps.env
    holding every saved value."""

    def present(*paths: str) -> None:
        """Only these settings files are on the server."""
        fake_host.answer("test -f *", status=1)
        for path in paths:
            fake_host.answer(f"test -f {path}")

    present("/etc/filedrawer/staging-mail.env", "/etc/filedrawer/staging-screening.env")
    fake_host.answer("*os-release*", stdout="ID=debian\n")
    fake_host.answer(
        "*ssh_host_ed25519_key.pub*", stdout="ssh-ed25519 AAAAhost root@vps\n"
    )
    for secret in ("DEPLOY_SSH_KEY", "SMOKE_TEST_TOKEN", "STAGING_KEY"):
        (fake_host.secrets / secret).write_text("kept")
    fake_host.answer("*", stdout="203.0.113.7 STREAM x\n", tool="getent")
    fake_host.answer("*", stdout="401\n", tool="curl")
    fake_host.answer("*robots.txt*", stdout="200\n", tool="curl")
    fake_host.answer("*X-Staging-Key*", stdout="200\n", tool="curl")
    ops = tmp_path / "ops"
    ops.mkdir()
    env_file = ops / ".vps.env"
    env_file.write_text(
        "VPS_HOST='203.0.113.7'\nSTAGING_USER='owner'\n"
        f"STAGING_KEY='{STAGING_KEY}'\nSMTP_USER='noreply@filedrawer.dev'\n"
        "MAIL_FROM='File Drawer <noreply@filedrawer.dev>'\nSMOKE_ACCOUNT='a-smoke1'\n"
        "SCREENING_DAILY_USD='1'\nADMIN_ACCOUNT='a-owner1'\n"
    )

    def run() -> subprocess.CompletedProcess:
        return subprocess.run(
            ["bash", WIZARD, "--only-new"],
            cwd=tmp_path,
            input="",
            env=fake_host.env,
            capture_output=True,
            text=True,
            check=False,
            timeout=60,
        )

    def sent_scripts() -> list[str]:
        return [fake_host.sent(n + 1) for n in range(len(fake_host.calls()))]

    return SimpleNamespace(
        run=run,
        log=fake_host.log,
        present=present,
        sent=sent_scripts,
        opened=opened_urls(),
    )


def script(name: str) -> str:
    return (REPO / "ops/remote" / name).read_text()


def test_only_new_reruns_every_stage_without_asking_for_anything_saved(rerun):
    result = rerun.run()
    assert result.returncode == 0, result.stdout + result.stderr
    sent = rerun.sent()
    assert script("staging-service.sh") in sent
    assert script("base.sh") in sent
    # Caddy keeps the password hash on the server: no password, only the key.
    assert f"\t{STAGING_KEY}\n" + script("caddy.sh") in sent
    # Mail and screening are in place already.
    assert script("mail.sh") not in sent
    assert not any(s.endswith(script("screening.sh")) for s in sent)
    # The deploy key stays; its scripts are reinstalled.
    log = rerun.log.read_text()
    assert "ssh-keygen" not in log
    assert "gh secret set DEPLOY_SSH_KEY" not in log
    assert script("filedrawer-receive") in sent
    # The admin account is granted again, and the smoke token kept.
    assert "ADMIN_ACCOUNT=a-owner1 bash -s" in log
    assert "gh secret set SMOKE_TEST_TOKEN" not in log
    assert "SMOKE_ACCOUNT=a-smoke1 bash -s" in log
    assert script("smoke-env.sh") in sent
    for question in ("Press Enter", "Make a new token anyway", "Run it on"):
        assert question not in result.stdout, question
    # Everything resolves, so nothing opens a browser.
    assert not rerun.opened.exists(), rerun.opened.read_text()


def test_only_new_runs_a_stage_whose_settings_the_server_lacks(rerun):
    rerun.present("/etc/filedrawer/staging-mail.env")
    result = rerun.run()
    # Screening isn't set up yet, so it asks for the API key, and stdin is empty.
    assert result.returncode != 0
    assert "API key" in result.stdout
