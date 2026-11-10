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


def run_wizard(
    *args: str, cwd: Path, answers: str, env: dict
) -> subprocess.CompletedProcess:
    """Runs ops/provision-vps with `args` in `cwd`, `answers` on stdin."""
    return subprocess.run(
        ["bash", WIZARD, *args],
        cwd=cwd,
        input=answers,
        env=env,
        capture_output=True,
        text=True,
        check=False,
    )


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
            "ENVIRONMENT": "staging",
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
    ssh that answers as ops/remote/ci-account.sh would."""
    # Only ci-account.sh answers with an account and a token, a-new for the smoke
    # test and a-ci for the screen check.
    fake_host.answer("*", stdout="a-new\tfdt_s3cret\n", stdin="create-token")
    fake_host.answer("*ROLE=screen*", stdout="a-ci\tfdt_ci7\n", stdin="create-token")
    ops = tmp_path / "ops"
    ops.mkdir()
    env_file = ops / ".vps.env"
    env_file.write_text(f"VPS_HOST='h'\nSTAGING_KEY='{STAGING_KEY}'\n")

    def run(answers: str = "\n") -> subprocess.CompletedProcess:
        return run_wizard("smoke", cwd=tmp_path, answers=answers, env=fake_host.env)

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
    assert sent == (REPO / "ops/remote/ci-account.sh").read_text()


def test_smoke_makes_the_ci_screen_account_and_stores_its_token_unshown(smoke):
    result = smoke.run()
    assert result.returncode == 0, result.stdout + result.stderr
    assert (smoke.secrets / "CI_SCREEN_TOKEN").read_text() == "fdt_ci7"
    assert "CI_SCREEN_ACCOUNT='a-ci'" in smoke.env_file.read_text()
    assert "fdt_ci7" not in smoke.log.read_text() + result.stdout + result.stderr
    calls = [c for c in smoke.log.read_text().splitlines() if c.startswith("ssh ")]
    assert "ssh env ROLE=screen ACCOUNT= bash -s" in calls, calls
    assert (
        smoke.host.sent(calls.index("ssh env ROLE=screen ACCOUNT= bash -s") + 1)
        == (REPO / "ops/remote/ci-account.sh").read_text()
    )


def test_smoke_also_names_the_screen_test_account_it_finds_in_ops(smoke):
    (smoke.tmp / "ops/.screen-test.env").write_text(
        "SCREEN_TEST_ACCOUNT='a-screen1'\nSCREEN_TEST_TOKEN='fdt_secret'\n"
    )
    result = smoke.run()
    assert result.returncode == 0, result.stdout + result.stderr
    calls = [c for c in smoke.log.read_text().splitlines() if c.startswith("ssh ")]
    assert calls[-1] == (
        "ssh env SMOKE_ACCOUNT=a-new SCREEN_TEST_ACCOUNT=a-screen1"
        " CI_SCREEN_ACCOUNT=a-ci bash -s"
    )
    assert "fdt_secret" not in smoke.log.read_text() + result.stdout + result.stderr
    # The eval's token stays local: CI has an account of its own.
    assert not (smoke.secrets / "SCREEN_TEST_TOKEN").exists()


def test_smoke_names_the_accounts_to_staging_so_their_entries_are_handled(smoke):
    result = smoke.run()
    assert result.returncode == 0, result.stdout + result.stderr
    calls = [c for c in smoke.log.read_text().splitlines() if c.startswith("ssh ")]
    assert calls[-1] == "ssh env SMOKE_ACCOUNT=a-new CI_SCREEN_ACCOUNT=a-ci bash -s"
    sent = smoke.host.sent(len(calls))
    assert sent == (REPO / "ops/remote/smoke-env.sh").read_text()
    # The unit first, so it reads the file smoke-env.sh writes.
    unit = smoke.host.sent(len(calls) - 1)
    assert unit == (REPO / "ops/remote/service.sh").read_text()


def test_a_rerun_passes_the_known_accounts_and_keeps_the_tokens_unless_asked(smoke):
    smoke.run()
    (smoke.secrets / "SMOKE_TEST_TOKEN").write_text("fdt_kept")
    (smoke.secrets / "CI_SCREEN_TOKEN").write_text("fdt_kept2")
    kept = smoke.run("n\nn\n\n")
    assert kept.returncode == 0, kept.stdout + kept.stderr
    assert (smoke.secrets / "SMOKE_TEST_TOKEN").read_text() == "fdt_kept"
    assert (smoke.secrets / "CI_SCREEN_TOKEN").read_text() == "fdt_kept2"
    # The kept run sends only the setting, not ci-account.sh.
    calls = [c for c in smoke.log.read_text().splitlines() if c.startswith("ssh ")]
    assert len(calls) == 6, calls
    assert smoke.host.sent(6) == (REPO / "ops/remote/smoke-env.sh").read_text()
    renewed = smoke.run("y\ny\n\n")
    assert renewed.returncode == 0, renewed.stdout + renewed.stderr
    assert (smoke.secrets / "SMOKE_TEST_TOKEN").read_text() == "fdt_s3cret"
    assert (smoke.secrets / "CI_SCREEN_TOKEN").read_text() == "fdt_ci7"
    log = smoke.log.read_text()
    assert "ssh env ROLE=smoke ACCOUNT=a-new bash -s" in log
    assert "ssh env ROLE=screen ACCOUNT=a-ci bash -s" in log


def test_before_the_first_deploy_it_says_to_rerun_and_still_sets_the_key(smoke):
    smoke.host.answer("*", status=3)
    result = smoke.run()
    assert result.returncode == 0, result.stdout + result.stderr
    assert "ops/provision-vps smoke" in result.stdout
    assert not (smoke.secrets / "SMOKE_TEST_TOKEN").exists()
    assert not (smoke.secrets / "CI_SCREEN_TOKEN").exists()
    assert (smoke.secrets / "STAGING_KEY").read_text() == STAGING_KEY


def test_a_failure_on_the_second_account_still_names_the_first_to_staging(smoke):
    smoke.host.answer("*ROLE=screen*", status=1)
    result = smoke.run()
    assert result.returncode == 1, result.stdout + result.stderr
    assert (smoke.secrets / "SMOKE_TEST_TOKEN").read_text() == "fdt_s3cret"
    assert not (smoke.secrets / "CI_SCREEN_TOKEN").exists()
    calls = [c for c in smoke.log.read_text().splitlines() if c.startswith("ssh ")]
    assert calls[-1] == "ssh env SMOKE_ACCOUNT=a-new bash -s", calls


def test_smoke_says_to_delete_the_eval_token_ci_no_longer_reads(smoke):
    first = smoke.run()
    assert "SCREEN_TEST_TOKEN" not in first.stdout
    (smoke.secrets / "SCREEN_TEST_TOKEN").write_text("old")
    result = smoke.run("n\nn\n\n")
    assert result.returncode == 0, result.stdout + result.stderr
    assert "gh secret delete SCREEN_TEST_TOKEN --env staging" in result.stdout


def test_a_failure_on_the_server_stops_the_wizard(smoke):
    smoke.host.answer("*", status=1)
    result = smoke.run()
    assert result.returncode == 1
    assert not (smoke.secrets / "SMOKE_TEST_TOKEN").exists()


@pytest.mark.parametrize(
    "reply",
    ["a-new\n", "a-new fdt_s3cret\n", "a-new\tfdt_\n", "a-new\tfdt_s3cret extra\n"],
)
def test_a_reply_that_isnt_an_account_tab_token_stores_nothing(smoke, reply):
    smoke.host.answer("*", stdout=reply, stdin="create-token")
    result = smoke.run()
    assert result.returncode == 1, result.stdout + result.stderr
    assert "Making the smoke-test account failed" in result.stdout
    assert not (smoke.secrets / "SMOKE_TEST_TOKEN").exists()
    assert "SMOKE_ACCOUNT" not in smoke.env_file.read_text()


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
        return run_wizard(name, cwd=tmp_path, answers=answers, env=fake_host.env)

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
    assert stage.sent(1) == (REPO / "ops/remote/service.sh").read_text()
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


GITHUB_TOKEN = "github_pat_" + "g" * 70


def test_github_sends_the_token_on_stdin_and_saves_nothing(stage):
    result = stage.run("github", f"{GITHUB_TOKEN}\n{GITHUB_TOKEN}\n\n")
    assert result.returncode == 0, result.stdout + result.stderr
    # The unit first, so it reads the new file, then the token.
    assert stage.sent(1) == (REPO / "ops/remote/service.sh").read_text()
    assert (
        stage.sent(2)
        == f"{GITHUB_TOKEN}\n" + (REPO / "ops/remote/github-token.sh").read_text()
    )
    assert "read -r TOKEN" in stage.log.read_text()
    assert (
        stage.opened.read_text()
        == "https://github.com/settings/personal-access-tokens/new\n"
    )
    for where in (
        stage.log.read_text(),
        stage.env_file.read_text(),
        result.stdout + result.stderr,
    ):
        assert GITHUB_TOKEN not in where


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
    assert log[1] == "ssh env ENVIRONMENT=staging ADMIN_ACCOUNT=a-abc123 bash -s"
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

    present(
        "/etc/filedrawer/staging-mail.env",
        "/etc/filedrawer/staging-screening.env",
        "/etc/filedrawer/github.env",
    )
    fake_host.answer("*os-release*", stdout="ID=debian\n")
    fake_host.answer(
        "*ssh_host_ed25519_key.pub*", stdout="ssh-ed25519 AAAAhost root@vps\n"
    )
    for secret in ("DEPLOY_SSH_KEY", "SMOKE_TEST_TOKEN", "STAGING_KEY"):
        (fake_host.secrets / secret).write_text("kept")
    # The CI screen account is new to this server: ci-account.sh makes it.
    fake_host.answer("*ROLE=screen*", stdout="a-ci\tfdt_ci7\n", stdin="create-token")
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
        return run_wizard("--only-new", cwd=tmp_path, answers="", env=fake_host.env)

    def sent_scripts() -> list[str]:
        return [fake_host.sent(n + 1) for n in range(len(fake_host.calls()))]

    return SimpleNamespace(
        run=run,
        tmp=tmp_path,
        secrets=fake_host.secrets,
        log=fake_host.log,
        present=present,
        sent=sent_scripts,
        opened=opened_urls(),
    )


def script(name: str) -> str:
    return (REPO / "ops/remote" / name).read_text()


def test_only_new_makes_the_ci_screen_account_once_then_keeps_it(rerun):
    first = rerun.run()
    assert first.returncode == 0, first.stdout + first.stderr
    assert (rerun.secrets / "CI_SCREEN_TOKEN").read_text() == "fdt_ci7"
    assert "CI_SCREEN_ACCOUNT='a-ci'" in (rerun.tmp / "ops/.vps.env").read_text()
    (rerun.secrets / "CI_SCREEN_TOKEN").write_text("kept")
    again = rerun.run()
    assert again.returncode == 0, again.stdout + again.stderr
    assert (rerun.secrets / "CI_SCREEN_TOKEN").read_text() == "kept"
    assert rerun.log.read_text().count("ROLE=screen") == 1
    assert "CI_SCREEN_ACCOUNT=a-ci bash -s" in rerun.log.read_text()


def test_only_new_reruns_every_stage_without_asking_for_anything_saved(rerun):
    result = rerun.run()
    assert result.returncode == 0, result.stdout + result.stderr
    sent = rerun.sent()
    assert script("service.sh") in sent
    assert script("base.sh") in sent
    # Caddy keeps the password hash on the server: no password, only the key.
    assert f"\t{STAGING_KEY}\n" + script("caddy.sh") in sent
    # Mail, screening and the GitHub token are in place already.
    assert script("mail.sh") not in sent
    assert not any(s.endswith(script("screening.sh")) for s in sent)
    assert not any(s.endswith(script("github-token.sh")) for s in sent)
    # The deploy key stays; its scripts are reinstalled.
    log = rerun.log.read_text()
    assert "ssh-keygen" not in log
    assert "gh secret set DEPLOY_SSH_KEY" not in log
    assert script("filedrawer-receive") in sent
    # deploy-user.sh still runs, with no key, so a newer sudo rule reaches the server.
    assert "ssh env DEPLOY_PUBLIC_KEY= bash -s" in log
    assert script("deploy-user.sh") in sent
    # The admin account is granted again, and the smoke token kept.
    assert "ADMIN_ACCOUNT=a-owner1 bash -s" in log
    assert "gh secret set SMOKE_TEST_TOKEN" not in log
    assert "SMOKE_ACCOUNT=a-smoke1 CI_SCREEN_ACCOUNT=a-ci bash -s" in log
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


# Production: ops/provision-vps production, and production account after #42's deploy

PROD_KEY = "sk-ant-api03-" + "p" * 40
DAILY_PING = "https://hc-ping.com/aaaaaaaa-0000-4000-8000-000000000001"
BACKUP_PING = "https://hc-ping.com/aaaaaaaa-0000-4000-8000-000000000002"
SMTP_PASSWORD = "prod-mail-pw"


@pytest.fixture
def production(tmp_path, fake_host):
    """Runs `ops/provision-vps production [...]` in a folder whose ops/.vps.env holds
    staging's saved values, against fake_host answering as a server staging's wizard
    set up: every name resolves, the Anthropic key works, and production is down."""
    ops = tmp_path / "ops"
    ops.mkdir()
    env_file = ops / ".vps.env"
    env_file.write_text(
        "VPS_HOST='203.0.113.7'\nSTAGING_USER='owner'\n"
        f"STAGING_KEY='{STAGING_KEY}'\nSMTP_USER='staging@filedrawer.dev'\n"
        "MAIL_FROM='File Drawer staging <staging@filedrawer.dev>'\n"
        "SCREENING_DAILY_USD='5'\nADMIN_ACCOUNT='a-staging1'\n"
    )
    fake_host.answer("test -f *", status=1)
    fake_host.answer("test -x *", status=1)
    fake_host.answer("*os-release*", stdout="ID=debian\n")
    fake_host.answer("*ip -6*", stdout="2001:db8::7\n")
    fake_host.answer("*", stdout="203.0.113.7 STREAM x\n", tool="getent")
    fake_host.answer("*", stdout="503\n", tool="curl")
    fake_host.answer("*www.filedrawer.dev*", stdout="301\n", tool="curl")
    fake_host.answer("*api.anthropic.com*", stdout="200\n", tool="curl")

    def run(*args: str, answers: str = "") -> subprocess.CompletedProcess:
        return run_wizard(
            "production", *args, cwd=tmp_path, answers=answers, env=fake_host.env
        )

    def sent_scripts() -> list[str]:
        return [fake_host.sent(n + 1) for n in range(len(fake_host.calls()))]

    return SimpleNamespace(
        run=run,
        host=fake_host,
        log=fake_host.log,
        env_file=env_file,
        sent=sent_scripts,
        opened=opened_urls(),
    )


# Answers for a full production run on a server with no production settings, in
# the order the wizard asks; extra blank lines pass any pause.
FULL_RUN = (
    "\n" * 6
    + "prod@filedrawer.dev\nFile Drawer <noreply@filedrawer.dev>\n"
    + f"{SMTP_PASSWORD}\n{SMTP_PASSWORD}\n\n"
    + f"\n\n{PROD_KEY}\n{PROD_KEY}\n\n"
    + f"{GITHUB_TOKEN}\n{GITHUB_TOKEN}\n\n"
    + f"{DAILY_PING}\n{BACKUP_PING}\n"
    + "\n" * 10
)


def ssh_calls(production) -> list[str]:
    return production.host.calls()


def test_production_sends_each_script_for_production_in_order(production):
    result = production.run(answers=FULL_RUN)
    assert result.returncode == 0, result.stdout + result.stderr
    calls = ssh_calls(production)
    sent = production.sent()

    def at(name: str, setting: str = "") -> int:
        for i, (call, text) in enumerate(zip(calls, sent)):
            if text.endswith(script(name)) and setting in call:
                return i
        raise AssertionError(f"{name} {setting} not sent: {calls}")

    order = [
        at("service-user.sh", "ENVIRONMENT=production"),
        at("loopback.sh"),
        at("service.sh", "ENVIRONMENT=production"),
        at("caddy.sh", "ENVIRONMENT=production"),
        at("mail.sh", "ENVIRONMENT=production"),
        at("screening.sh", "ENVIRONMENT=production"),
        at("github-token.sh"),
        at("ping.sh"),
    ]
    assert order == sorted(order), order
    # loopback.sh names staging's user, so it must exist first.
    assert at("service-user.sh", "ENVIRONMENT=staging") < at("loopback.sh")
    # The install script that knows production goes to the server.
    assert script("filedrawer-install") in sent
    assert "ENVIRONMENT=staging" not in " ".join(
        c for c, s in zip(calls, sent) if not s.endswith(script("service-user.sh"))
    )


def test_production_sends_each_secret_on_stdin_and_shows_or_saves_none(production):
    result = production.run(answers=FULL_RUN)
    assert result.returncode == 0, result.stdout + result.stderr
    sent = production.sent()
    assert f"{SMTP_PASSWORD}\n" + script("mail.sh") in sent
    assert f"{PROD_KEY}\n" + script("screening.sh") in sent
    assert f"{GITHUB_TOKEN}\n" + script("github-token.sh") in sent
    assert f"{DAILY_PING}\t{BACKUP_PING}\n" + script("ping.sh") in sent
    shown = (
        production.log.read_text()
        + production.env_file.read_text()
        + result.stdout
        + result.stderr
    )
    for secret in (SMTP_PASSWORD, PROD_KEY, GITHUB_TOKEN, DAILY_PING, BACKUP_PING):
        assert secret not in shown, secret


def test_production_saves_its_values_apart_from_stagings(production):
    result = production.run(answers=FULL_RUN)
    assert result.returncode == 0, result.stdout + result.stderr
    saved = production.env_file.read_text()
    assert "PROD_SMTP_USER='prod@filedrawer.dev'" in saved
    assert "PROD_MAIL_FROM='File Drawer <noreply@filedrawer.dev>'" in saved
    assert "PROD_SCREENING_DAILY_USD='1'" in saved
    # Staging's stay as they were.
    assert "SMTP_USER='staging@filedrawer.dev'" in saved
    assert "SCREENING_DAILY_USD='5'" in saved
    log = production.log.read_text()
    assert "SMTP_USER=prod@filedrawer.dev" in log
    assert "DAILY_USD=1 " in log


def test_production_never_offers_stagings_saved_values(production):
    """Staging's mail user and cap are in ops/.vps.env, but production's prompts
    offer no saved value to keep."""
    result = production.run(answers=FULL_RUN)
    assert result.returncode == 0, result.stdout + result.stderr
    assert "[Enter keeps current]" not in result.stdout


def test_production_lists_the_three_names_dns_needs(production):
    result = production.run(answers=FULL_RUN)
    assert result.returncode == 0, result.stdout + result.stderr
    for name in ("filedrawer.dev", "www.filedrawer.dev", "files.filedrawer.dev"):
        assert f"{name}  A     203.0.113.7" in result.stdout, name
        assert f"{name}  AAAA  2001:db8::7" in result.stdout, name
    assert "staging.filedrawer.dev  A" not in result.stdout


def test_production_says_the_spend_limit_and_the_healthchecks_periods(production):
    result = production.run(answers=FULL_RUN)
    assert "$40" in result.stdout
    assert "Period: 1 day" in result.stdout
    assert (
        "https://console.anthropic.com/settings/keys" in production.opened.read_text()
    )
    assert "https://healthchecks.io/" in production.opened.read_text()


def test_a_key_anthropic_refuses_stops_before_it_reaches_the_server(production):
    production.host.answer("*api.anthropic.com*", stdout="401\n", tool="curl")
    result = production.run(answers=FULL_RUN)
    assert result.returncode != 0
    assert "refused the key" in result.stdout
    assert not any(s.endswith(script("screening.sh")) for s in production.sent())
    assert PROD_KEY not in production.log.read_text()


def test_a_ping_url_that_isnt_one_stops_before_it_reaches_the_server(production):
    answers = FULL_RUN.replace(BACKUP_PING, "https://example.com/x")
    result = production.run(answers=answers)
    assert result.returncode != 0
    assert "isn't a healthchecks.io ping URL" in result.stdout
    assert not any(s.endswith(script("ping.sh")) for s in production.sent())


def test_production_keeps_the_shared_github_token_the_server_has(production):
    production.host.answer("test -f /etc/filedrawer/github.env")
    answers = FULL_RUN.replace(f"{GITHUB_TOKEN}\n{GITHUB_TOKEN}\n", "")
    result = production.run(answers=answers)
    assert result.returncode == 0, result.stdout + result.stderr
    assert not any(s.endswith(script("github-token.sh")) for s in production.sent())
    assert "shared with staging" in result.stdout


def test_production_ends_listing_the_settings_the_server_still_lacks(production):
    result = production.run(answers=FULL_RUN)
    assert result.returncode == 0, result.stdout + result.stderr
    # fake_host says no file exists, so all of them are listed.
    for path in (
        "/etc/filedrawer/production-mail.env",
        "/etc/filedrawer/production-screening.env",
        "/etc/filedrawer/production-ping.env",
        "/etc/filedrawer/github.env",
    ):
        assert path in result.stdout, path
    assert "won't start" in result.stdout


def test_production_says_to_run_the_account_stage_after_the_first_deploy(production):
    result = production.run(answers=FULL_RUN)
    assert "ops/provision-vps production account" in result.stdout


def test_production_refuses_stagings_sending_user(production):
    answers = FULL_RUN.replace("prod@filedrawer.dev\n", "staging@filedrawer.dev\n", 1)
    result = production.run(answers=answers)
    assert result.returncode != 0
    assert "staging's sending user" in result.stdout
    assert not any(s.endswith(script("mail.sh")) for s in production.sent())


def test_production_checks_the_site_from_outside(production):
    result = production.run(answers=FULL_RUN)
    assert result.returncode == 0, result.stdout + result.stderr
    assert "503" in result.stdout and "✓" in result.stdout
    curl = production.host.calls("curl")
    [tags] = [c for c in curl if "https://filedrawer.dev/tags" in c]
    # A 503's Retry-After (120 s) would otherwise stretch curl's retries to minutes.
    assert "--retry-max-time 30" in tags
    assert any("https://www.filedrawer.dev/" in c for c in curl)


def test_production_only_new_asks_for_nothing_the_server_has(production):
    for path in (
        "production-mail.env",
        "production-screening.env",
        "production-ping.env",
        "github.env",
    ):
        production.host.answer(f"test -f /etc/filedrawer/{path}")
    production.env_file.write_text(
        production.env_file.read_text()
        + "PROD_SMTP_USER='prod@filedrawer.dev'\nPROD_SCREENING_DAILY_USD='1'\n"
    )
    result = run_wizard(
        "--only-new",
        "production",
        cwd=production.env_file.parent.parent,
        answers="",
        env=production.host.env,
    )
    assert result.returncode == 0, result.stdout + result.stderr
    sent = production.sent()
    assert any(s.endswith(script("service.sh")) for s in sent)
    assert any(s.endswith(script("caddy.sh")) for s in sent)
    for kept in ("mail.sh", "screening.sh", "github-token.sh", "ping.sh"):
        assert not any(s.endswith(script(kept)) for s in sent), kept
    assert "won't start" not in result.stdout


# production account


ACCOUNT_RUN = "a-prod1\n" + "\n" * 6


def test_account_before_the_first_deploy_asks_nothing_and_says_to_rerun(production):
    result = production.run("account", answers="")
    assert result.returncode == 0, result.stdout + result.stderr
    assert "ops/provision-vps production account" in result.stdout
    assert "admin-grant" not in production.log.read_text()


def test_account_grants_the_production_account_once_the_mail_arrived(production):
    production.host.answer("test -x /srv/filedrawer/production/bin/filedrawer")
    result = production.run("account", answers="y\n" + ACCOUNT_RUN)
    assert result.returncode == 0, result.stdout + result.stderr
    assert "ssh env ENVIRONMENT=production ADMIN_ACCOUNT=a-prod1 bash -s" in (
        production.log.read_text()
    )
    assert "PROD_ADMIN_ACCOUNT='a-prod1'" in production.env_file.read_text()
    assert "ADMIN_ACCOUNT='a-staging1'" in production.env_file.read_text()
    opened = production.opened.read_text()
    assert "https://filedrawer.dev/signup" in opened
    assert "verification link" in result.stdout
    assert "https://uptimerobot.com/" in opened
    assert "https://filedrawer.dev/tags" in result.stdout


def test_account_walks_spf_dkim_and_dmarc_when_the_mail_went_to_spam(production):
    production.host.answer("test -x /srv/filedrawer/production/bin/filedrawer")
    result = production.run("account", answers="n\n" + "\n" * 10)
    assert result.returncode != 0
    for record in ("SPF", "DKIM", "DMARC"):
        assert record in result.stdout, record
    assert "the DMARC record Purelymail lists" in result.stdout
    assert "admin-grant" not in production.log.read_text()
    assert "ADMIN_ACCOUNT=" not in production.log.read_text()


def test_account_under_only_new_takes_the_saved_id_and_opens_no_account_page(
    production,
):
    production.host.answer("test -x /srv/filedrawer/production/bin/filedrawer")
    production.env_file.write_text(
        production.env_file.read_text() + "PROD_ADMIN_ACCOUNT='a-prod1'\n"
    )
    result = run_wizard(
        "--only-new",
        "production",
        "account",
        cwd=production.env_file.parent.parent,
        answers="y\n",
        env=production.host.env,
    )
    assert result.returncode == 0, result.stdout + result.stderr
    assert "ENVIRONMENT=production ADMIN_ACCOUNT=a-prod1" in production.log.read_text()
    assert "https://filedrawer.dev/account" not in production.opened.read_text()


def test_production_opens_the_workspaces_page_before_the_keys(production):
    production.run(answers=FULL_RUN)
    opened = production.opened.read_text().splitlines()
    assert opened.index(
        "https://console.anthropic.com/settings/workspaces"
    ) < opened.index("https://console.anthropic.com/settings/keys")


def test_production_refuses_a_stage_it_doesnt_have(production):
    result = production.run("nonsense", answers="")
    assert result.returncode != 0
    assert "production account" in result.stdout + result.stderr
