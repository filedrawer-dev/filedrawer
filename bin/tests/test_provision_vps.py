"""Tests for ops/provision-vps's helpers that run scripts on the VPS: remote and
remote_with_secret. A fake ssh runs the remote command line locally, through bash,
as the server's shell would, so the quoting is tested the way the server sees it.
"""

import os
import subprocess
from collections.abc import Callable
from dataclasses import dataclass
from pathlib import Path
from types import SimpleNamespace

import pytest
from conftest import FakeHost, isolated, logged_calls, opened_urls

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


@dataclass(frozen=True)
class Ran:
    """What one wizard run left behind, copied out, so the tests that share a
    module's run can read it but not change it for one another. A wizard run is
    about 200 process starts and seconds of CPU, and a dozen tests asserting
    different things about the same run once ran it a dozen times (#165)."""

    returncode: int
    stdout: str
    stderr: str
    # fake_host's log, and each ssh call's stdin in order.
    log: str
    sent: tuple[str, ...]
    env_file: str
    # The URLs it opened, or None when it opened none.
    opened: str | None

    @classmethod
    def of(
        cls, result: subprocess.CompletedProcess, host: FakeHost, env_file: Path
    ) -> "Ran":
        opened = opened_urls()
        return cls(
            returncode=result.returncode,
            stdout=result.stdout,
            stderr=result.stderr,
            log=host.log.read_text(),
            sent=tuple(host.sent(n + 1) for n in range(len(host.calls()))),
            env_file=env_file.read_text(),
            opened=opened.read_text() if opened.exists() else None,
        )

    def calls(self, tool: str = "ssh") -> list[str]:
        """As FakeHost.calls: the command lines `tool` was called with, in order."""
        return logged_calls(self.log, tool)


def isolated_run(
    tmp_path_factory, name: str, make: Callable[[Path, FakeHost], Ran]
) -> Ran:
    """make(root, host), with a fresh FakeHost under a fresh `root`, for a module's
    fixture, in the environment the autouse fixtures give each test. xdist sets up
    a module's fixtures once in each worker that runs one of its tests, so a shared
    run still runs once per such worker."""
    with isolated(tmp_path_factory):
        root = tmp_path_factory.mktemp(name)
        return make(root, FakeHost(root / "fake-host"))


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

    def run(answers: str = "\n", *flags: str) -> subprocess.CompletedProcess:
        return run_wizard(
            *flags, "smoke", cwd=tmp_path, answers=answers, env=fake_host.env
        )

    return SimpleNamespace(
        run=run,
        tmp=tmp_path,
        host=fake_host,
        log=fake_host.log,
        secrets=fake_host.secrets,
        env_file=env_file,
    )


def stage_calls(smoke) -> list[tuple[int, str]]:
    """The ssh calls of each run before the outbound probe that ends it, each with
    its number for sent(); every run must end with that probe."""
    calls = [c for c in smoke.log.read_text().splitlines() if c.startswith("ssh ")]
    # The closing check of the server's secrets follows each run's probe.
    numbered = [
        (n, call)
        for n, call in enumerate(calls, start=1)
        if smoke.host.sent(n) != script("check-secrets.sh")
    ]
    runs, run = [], []
    for n, call in numbered:
        run.append((n, call))
        if call == f"ssh {PROBE_STAGING}":
            assert run[-2][1] == f"ssh {INSTALL_INSTALL}", run
            runs.extend(run[:-2])
            run = []
    assert not run, f"a run didn't end with the probe: {run}"
    return runs


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
    assert stage_calls(smoke)[-1][1] == (
        "ssh env SMOKE_ACCOUNT=a-new SCREEN_TEST_ACCOUNT=a-screen1"
        " CI_SCREEN_ACCOUNT=a-ci bash -s"
    )
    assert "fdt_secret" not in smoke.log.read_text() + result.stdout + result.stderr
    # The eval's token stays local: CI has an account of its own.
    assert not (smoke.secrets / "SCREEN_TEST_TOKEN").exists()


def test_smoke_names_the_accounts_to_staging_so_their_entries_are_handled(smoke):
    result = smoke.run()
    assert result.returncode == 0, result.stdout + result.stderr
    calls = stage_calls(smoke)
    last, call = calls[-1]
    assert call == "ssh env SMOKE_ACCOUNT=a-new CI_SCREEN_ACCOUNT=a-ci bash -s"
    assert smoke.host.sent(last) == (REPO / "ops/remote/smoke-env.sh").read_text()
    # The unit first, so it reads the file smoke-env.sh writes.
    unit = smoke.host.sent(calls[-2][0])
    assert unit == (REPO / "ops/remote/service.sh").read_text()


def test_a_rerun_passes_the_known_accounts_and_keeps_the_tokens_unless_asked(smoke):
    smoke.run()
    (smoke.secrets / "SMOKE_TEST_TOKEN").write_text("fdt_kept")
    (smoke.secrets / "CI_SCREEN_TOKEN").write_text("fdt_kept2")
    kept = smoke.run("n\nn\n\n", "--ask-all")
    assert kept.returncode == 0, kept.stdout + kept.stderr
    assert (smoke.secrets / "SMOKE_TEST_TOKEN").read_text() == "fdt_kept"
    assert (smoke.secrets / "CI_SCREEN_TOKEN").read_text() == "fdt_kept2"
    # The kept run sends only the setting, not ci-account.sh.
    calls = stage_calls(smoke)
    assert len(calls) == 6, calls
    assert (
        smoke.host.sent(calls[5][0]) == (REPO / "ops/remote/smoke-env.sh").read_text()
    )
    renewed = smoke.run("y\ny\n\n", "--ask-all")
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
    fake_host as a server that holds no settings yet, with the URLs the wizard opens
    in `opened`."""
    ops = tmp_path / "ops"
    ops.mkdir()
    env_file = ops / ".vps.env"
    env_file.write_text("VPS_HOST='h'\n")

    def run(*args: str) -> subprocess.CompletedProcess:
        *names, answers = args
        return run_wizard(*names, cwd=tmp_path, answers=answers, env=fake_host.env)

    def scripts() -> list[str]:
        """What each ssh call sent, in order, leaving out the calls that send
        nothing, such as the checks of what the server holds."""
        sent = (fake_host.sent(n + 1) for n in range(len(fake_host.calls())))
        return [text for text in sent if text]

    return SimpleNamespace(
        run=run,
        host=fake_host,
        log=fake_host.log,
        sent=fake_host.sent,
        scripts=scripts,
        env_file=env_file,
        opened=opened_urls(),
    )


def test_screening_sends_the_key_on_stdin_and_saves_only_the_cap(stage):
    result = stage.run("screening", f"0.25\n{API_KEY}\n{API_KEY}\n\n")
    assert result.returncode == 0, result.stdout + result.stderr
    # The unit first, so it reads the new file, then the settings.
    assert stage.scripts()[:2] == [
        script("service.sh"),
        f"{API_KEY}\n" + script("screening.sh"),
    ]
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
    assert stage.scripts()[:2] == [
        script("service.sh"),
        f"{GITHUB_TOKEN}\n" + script("github-token.sh"),
    ]
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


CLIENT_ID = "Ov23liAbCdEf01234567"
CLIENT_SECRET = "0123456789abcdef" * 2 + "01234567"
PROD_CLIENT_ID = "Ov23liProd0123456789"
PROD_CLIENT_SECRET = "fedcba9876543210" * 2 + "fedcba98"


@pytest.mark.parametrize(
    ("args", "environment", "site", "saved"),
    [
        (["github-oauth"], "staging", "https://staging.filedrawer.dev", "GITHUB"),
        (
            ["production", "github-oauth"],
            "production",
            "https://filedrawer.dev",
            "PROD_GITHUB",
        ),
    ],
)
def test_github_oauth_walks_an_app_with_one_exact_callback_and_sends_its_secret_on_stdin(
    stage, args, environment, site, saved
):
    result = stage.run(*args, f"y\n{CLIENT_ID}\n{CLIENT_SECRET}\n{CLIENT_SECRET}\n\n")
    assert result.returncode == 0, result.stdout + result.stderr
    # The unit first, so it reads the new file, then the settings.
    assert stage.scripts()[:2] == [
        script("service.sh"),
        f"{CLIENT_SECRET}\n" + script("github-oauth.sh"),
    ]
    log = stage.log.read_text()
    assert f"ENVIRONMENT={environment} CLIENT_ID={CLIENT_ID} bash -s" in log
    assert "read -r CLIENT_SECRET" in log
    assert f"{saved}_CLIENT_ID='{CLIENT_ID}'" in stage.env_file.read_text()
    assert (
        stage.opened.read_text()
        == "https://github.com/organizations/filedrawer-dev/settings/applications/new\n"
    )
    out = result.stdout
    assert f"Homepage URL: {site}\n" in out
    assert f"Authorization callback URL: {site}/login/github/callback\n" in out
    assert "wildcard" in out
    for where in (log, stage.env_file.read_text(), result.stdout + result.stderr):
        assert CLIENT_SECRET not in where
    probe = f"/usr/local/sbin/filedrawer-install check-outbound {environment}"
    assert probe in stage.host.calls()


def test_github_oauth_declined_is_saved_so_a_rerun_asks_nothing(stage):
    result = stage.run("github-oauth", "n\n\n")
    assert result.returncode == 0, result.stdout + result.stderr
    assert not any(s.endswith(script("github-oauth.sh")) for s in stage.scripts())
    assert "GITHUB_SIGNIN='off'" in stage.env_file.read_text()
    assert not stage.opened.exists()
    assert "GitHub sign-in (staging): off" in result.stdout.split("Setup complete")[1]
    again = stage.run("github-oauth", "")
    assert again.returncode == 0, again.stdout + again.stderr
    assert "Set up GitHub sign-in" not in again.stdout
    assert "--ask-all github-oauth" in again.stdout
    # --ask-all asks again.
    asked = stage.run("--ask-all", "github-oauth", "n\n\n")
    assert asked.returncode == 0, asked.stdout + asked.stderr
    assert "Set up GitHub sign-in" in asked.stdout


@pytest.mark.parametrize("client_id", ["Ov23 li", "Iv1'x", ""])
def test_github_oauth_refuses_a_client_id_it_cant_use_and_sends_nothing(
    stage, client_id
):
    result = stage.run("github-oauth", f"y\n{client_id}\n")
    assert result.returncode != 0
    assert not any(s.endswith(script("github-oauth.sh")) for s in stage.scripts())
    assert "GITHUB_CLIENT_ID" not in stage.env_file.read_text()


def test_github_oauth_stops_when_the_two_secrets_differ(stage):
    result = stage.run("github-oauth", f"y\n{CLIENT_ID}\n{CLIENT_SECRET}\nother\n\n")
    assert result.returncode == 1
    assert not any(s.endswith(script("github-oauth.sh")) for s in stage.scripts())


def oauth_held(stage, saved=""):
    """`stage`, on a server that holds staging's OAuth App, with `saved` in
    ops/.vps.env."""
    stage.host.answer("test -f *", status=1)
    stage.host.answer("test -f /etc/filedrawer/staging-github-oauth.env")
    stage.env_file.write_text(f"VPS_HOST='h'\n{saved}")


def test_github_oauth_ask_all_rotates_a_held_apps_secret_and_keeps_its_id(stage):
    """revoke-all.md's rotation: yes to a new secret, Enter on the client ID."""
    oauth_held(stage)
    result = stage.run(
        "--ask-all", "github-oauth", f"y\n\n{CLIENT_SECRET}\n{CLIENT_SECRET}\n\n"
    )
    assert result.returncode == 0, result.stdout + result.stderr
    assert f"{CLIENT_SECRET}\n" + script("github-oauth.sh") in stage.scripts()
    assert "ENVIRONMENT=staging CLIENT_ID= bash -s" in stage.log.read_text()
    assert "Set up GitHub sign-in" not in result.stdout
    assert "Generate a new client secret" in result.stdout
    assert "applications/new" not in (
        stage.opened.read_text() if stage.opened.exists() else ""
    )
    assert "GITHUB_SIGNIN" not in stage.env_file.read_text()


def test_github_oauth_plain_rerun_leaves_the_servers_client_id_alone(stage):
    """A checkout whose saved ID differs from the server's mustn't stop the run."""
    oauth_held(stage, f"GITHUB_CLIENT_ID='{PROD_CLIENT_ID}'\n")
    result = stage.run("github-oauth", "")
    assert result.returncode == 0, result.stdout + result.stderr
    assert "\n" + script("github-oauth.sh") in stage.scripts()
    assert "ENVIRONMENT=staging CLIENT_ID= bash -s" in stage.log.read_text()


def test_screening_stops_when_the_two_keys_differ(stage):
    result = stage.run("screening", f"1\n{API_KEY}\nsk-ant-other\n\n")
    assert result.returncode == 1
    assert "API_KEY" not in stage.log.read_text()


def test_admin_refuses_a_mistyped_id_and_saves_nothing(stage):
    stage.host.answer("test -x /srv/filedrawer/staging/bin/filedrawer")
    result = stage.run("admin", "A-abc 123\n\n")
    assert result.returncode == 1
    assert "ADMIN_ACCOUNT" not in stage.env_file.read_text()
    assert "admin-grant" not in stage.log.read_text()


def test_admin_grants_the_account_named(stage):
    stage.host.answer("test -x /srv/filedrawer/staging/bin/filedrawer")
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
    # The deploy check, then the closing check of the server's secrets.
    assert stage.log.read_text().count("ssh ") == 2
    assert stage.scripts() == [script("check-secrets.sh")]


# The whole wizard, rerun on a server it has set up

# base.sh writes it, so a server that has it ran base.sh.
BASE_MARKER = "/etc/ssh/sshd_config.d/10-filedrawer.conf"


@pytest.fixture
def rerun(tmp_path, fake_host):
    """Runs `ops/provision-vps` (a rerun) with empty stdin, so any prompt fails it,
    against fake_host answering as a server it has set up, and an ops/.vps.env
    holding every saved value."""
    present = set_up_rerun(tmp_path, fake_host)

    def run(*args: str, answers: str = "") -> subprocess.CompletedProcess:
        return run_wizard(*args, cwd=tmp_path, answers=answers, env=fake_host.env)

    def sent_scripts() -> list[str]:
        return [fake_host.sent(n + 1) for n in range(len(fake_host.calls()))]

    return SimpleNamespace(
        run=run,
        tmp=tmp_path,
        host=fake_host,
        secrets=fake_host.secrets,
        log=fake_host.log,
        present=present,
        sent=sent_scripts,
        opened=opened_urls(),
    )


@pytest.fixture(scope="module")
def plain_rerun(tmp_path_factory) -> Ran:
    """rerun's `run()`, run once for the tests that only read what it did."""

    def make(root: Path, host: FakeHost) -> Ran:
        set_up_rerun(root, host)
        result = run_wizard(cwd=root, answers="", env=host.env)
        return Ran.of(result, host, root / "ops/.vps.env")

    return isolated_run(tmp_path_factory, "plain-rerun", make)


def set_up_rerun(root: Path, fake_host: FakeHost):
    """Sets fake_host up as a server the wizard has set up, and root/ops/.vps.env
    with every saved value. Returns `present(*paths)`, which puts only those
    settings files on the server."""

    def present(*paths: str) -> None:
        """Only these settings files are on the server."""
        fake_host.answer("test -f *", status=1)
        for path in paths:
            fake_host.answer(f"test -f {path}")

    present(
        "/etc/filedrawer/staging-mail.env",
        "/etc/filedrawer/staging-screening.env",
        "/etc/filedrawer/github.env",
        "/etc/filedrawer/staging-github-oauth.env",
        BASE_MARKER,
        "/etc/caddy/sites/staging.caddy",
    )
    # Staging is deployed.
    fake_host.answer("test -x /srv/filedrawer/staging/bin/filedrawer")
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
    ops = root / "ops"
    ops.mkdir()
    (ops / ".vps.env").write_text(
        "VPS_HOST='203.0.113.7'\nSTAGING_USER='owner'\n"
        f"STAGING_KEY='{STAGING_KEY}'\nSMTP_USER='noreply@filedrawer.dev'\n"
        "MAIL_FROM='File Drawer <noreply@filedrawer.dev>'\nSMOKE_ACCOUNT='a-smoke1'\n"
        "SCREENING_DAILY_USD='1'\nADMIN_ACCOUNT='a-owner1'\n"
    )
    return present


def script(name: str) -> str:
    return (REPO / "ops/remote" / name).read_text()


def test_a_rerun_with_the_old_only_new_flag_makes_the_ci_screen_account_once(rerun):
    first = rerun.run("--only-new")
    assert first.returncode == 0, first.stdout + first.stderr
    assert (rerun.secrets / "CI_SCREEN_TOKEN").read_text() == "fdt_ci7"
    assert "CI_SCREEN_ACCOUNT='a-ci'" in (rerun.tmp / "ops/.vps.env").read_text()
    (rerun.secrets / "CI_SCREEN_TOKEN").write_text("kept")
    again = rerun.run()
    assert again.returncode == 0, again.stdout + again.stderr
    assert (rerun.secrets / "CI_SCREEN_TOKEN").read_text() == "kept"
    assert rerun.log.read_text().count("ROLE=screen") == 1
    assert "CI_SCREEN_ACCOUNT=a-ci bash -s" in rerun.log.read_text()


def test_a_rerun_runs_every_stage_without_asking_for_anything_saved(plain_rerun):
    result = plain_rerun
    assert result.returncode == 0, result.stdout + result.stderr
    sent = result.sent
    assert script("service.sh") in sent
    assert script("base.sh") in sent
    # Caddy keeps the password hash on the server: no password, only the key.
    assert f"\t{STAGING_KEY}\n" + script("caddy.sh") in sent
    # Mail, screening, the GitHub token and the OAuth App are in place already:
    # kept, sent empty.
    for name in ("mail.sh", "screening.sh", "github-token.sh", "github-oauth.sh"):
        assert "\n" + script(name) in sent, name
    # The deploy key stays; its scripts are reinstalled.
    log = result.log
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
    assert result.opened is None, result.opened


def test_staging_loads_the_loopback_rules_once_caddy_is_installed(plain_rerun):
    result = plain_rerun
    assert result.returncode == 0, result.stdout + result.stderr
    sent = result.sent
    # The rules name the user caddy, which Caddy's install makes.
    caddy = next(i for i, text in enumerate(sent) if text.endswith(script("caddy.sh")))
    assert sent.index(script("loopback.sh")) > caddy


def test_a_rerun_runs_a_stage_whose_settings_the_server_lacks(rerun):
    rerun.present(
        "/etc/filedrawer/staging-mail.env",
        BASE_MARKER,
        "/etc/caddy/sites/staging.caddy",
    )
    result = rerun.run()
    # Screening isn't set up yet, so it asks for the API key, and stdin is empty.
    assert result.returncode != 0
    assert "API key" in result.stdout


# Production: ops/provision-vps production, and production account after #42's deploy

PROD_KEY = "sk-ant-api03-" + "p" * 40
DAILY_PING = "https://hc-ping.com/aaaaaaaa-0000-4000-8000-000000000001"
BACKUP_PING = "https://hc-ping.com/aaaaaaaa-0000-4000-8000-000000000002"
CADDY_PING = "https://hc-ping.com/aaaaaaaa-0000-4000-8000-000000000003"
SMTP_PASSWORD = "prod-mail-pw"


@pytest.fixture
def production(tmp_path, fake_host):
    """Runs `ops/provision-vps production [...]` in a folder whose ops/.vps.env holds
    staging's saved values, against fake_host answering as a server staging's wizard
    set up: every name resolves, the Anthropic key works, and production is down."""
    env_file = set_up_production(tmp_path, fake_host)

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


@pytest.fixture(scope="module")
def full_run(tmp_path_factory) -> Ran:
    """production's `run(answers=FULL_RUN)`, run once for the tests that only read
    what it did."""

    def make(root: Path, host: FakeHost) -> Ran:
        env_file = set_up_production(root, host)
        result = run_wizard("production", cwd=root, answers=FULL_RUN, env=host.env)
        return Ran.of(result, host, env_file)

    return isolated_run(tmp_path_factory, "full-run", make)


@pytest.fixture(scope="module")
def production_rerun(tmp_path_factory) -> Ran:
    """`ops/provision-vps production` (a rerun) with empty stdin, on a server that
    holds every setting production needs, and an ops/.vps.env with its values; run
    once for the tests that only read what it did."""

    def make(root: Path, host: FakeHost) -> Ran:
        env_file = set_up_production(root, host)
        for path in (
            "production-mail.env",
            "production-screening.env",
            "production-ping.env",
            "production-backup.env",
            "github.env",
            "production-github-oauth.env",
            "caddy-upgrades.env",
        ):
            host.answer(f"test -f /etc/filedrawer/{path}")
        env_file.write_text(
            env_file.read_text()
            + "PROD_SMTP_USER='prod@filedrawer.dev'\nPROD_SCREENING_DAILY_USD='1'\n"
            + "PROD_MAIL_FROM='File Drawer <noreply@filedrawer.dev>'\n"
            + "PROD_HETZNER_BACKUPS_SHOWN='yes'\nCADDY_ALERT_TO='ops@filedrawer.dev'\n"
        )
        result = run_wizard("production", cwd=root, answers="", env=host.env)
        return Ran.of(result, host, env_file)

    return isolated_run(tmp_path_factory, "production-rerun", make)


def set_up_production(root: Path, fake_host: FakeHost) -> Path:
    """Sets fake_host up as production's fixture describes, and root/ops/.vps.env
    with staging's saved values; returns that file's path."""
    ops = root / "ops"
    ops.mkdir()
    env_file = ops / ".vps.env"
    env_file.write_text(
        "VPS_HOST='203.0.113.7'\nSTAGING_USER='owner'\n"
        f"STAGING_KEY='{STAGING_KEY}'\nSMTP_USER='staging@filedrawer.dev'\n"
        "MAIL_FROM='File Drawer staging <staging@filedrawer.dev>'\n"
        "SCREENING_DAILY_USD='5'\nADMIN_ACCOUNT='a-staging1'\n"
    )
    fake_host.answer("*os-release*", stdout="ID=debian\n")
    fake_host.answer("*ip -6*", stdout="2001:db8::7\n")
    fake_host.answer("*", stdout="203.0.113.7 STREAM x\n", tool="getent")
    fake_host.answer("*", stdout="503\n", tool="curl")
    fake_host.answer("*www.filedrawer.dev*", stdout="301\n", tool="curl")
    fake_host.answer("*api.anthropic.com*", stdout="200\n", tool="curl")
    return env_file


# Answers for a full production run on a server with no production settings, in
# the order the wizard asks; extra blank lines pass any pause. The banner and the
# first four stages ask nothing, so they don't pause.
FULL_RUN = (
    "prod@filedrawer.dev\nFile Drawer <noreply@filedrawer.dev>\n"
    + f"{SMTP_PASSWORD}\n{SMTP_PASSWORD}\n\n"
    + f"\n\n{PROD_KEY}\n{PROD_KEY}\n\n"
    + f"{GITHUB_TOKEN}\n{GITHUB_TOKEN}\n\n"
    # Yes to GitHub sign-in, then the OAuth App's client ID and secret.
    + f"y\n{PROD_CLIENT_ID}\n{PROD_CLIENT_SECRET}\n{PROD_CLIENT_SECRET}\n\n"
    + f"{DAILY_PING}\n{BACKUP_PING}\n"
    # The pings stage's pause, the backup stage's, then the Caddy guard's address
    # (Enter takes ops@filedrawer.dev), its ping URL, and yes, the test arrived.
    + "\n\n"
    + f"\n{CADDY_PING}\ny\n"
    # The guard's pause, then yes, the reboot check's test mail arrived.
    + "\ny\n"
    + "\n" * 10
)


def test_production_sends_each_script_for_production_in_order(full_run):
    result = full_run
    assert result.returncode == 0, result.stdout + result.stderr
    calls = result.calls()
    sent = result.sent

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
        at("github-oauth.sh", "ENVIRONMENT=production"),
        at("ping.sh"),
        at("backup.sh"),
        at("caddy-upgrades.sh"),
        at("reboots.sh"),
    ]
    assert order == sorted(order), order
    # The reboot check mails through the guard's settings, and its units start the
    # program, which is there first; the dry run comes once they're set up.
    assert sent.index(script("filedrawer-reboot-check")) < at("reboots.sh")
    assert calls.index("/usr/local/sbin/filedrawer-reboot-check --dry-run") > at(
        "reboots.sh"
    )
    # The units start the guard, so the program it runs, and the library that
    # program sources, are there first.
    assert sent.index(script("filedrawer-alerts.sh")) < at("caddy-upgrades.sh")
    assert sent.index(script("caddy-upgrade-check")) < at("caddy-upgrades.sh")
    assert calls.index("/usr/local/sbin/caddy-upgrade-check --test-mail") > at(
        "caddy-upgrades.sh"
    )
    # The timer starts with backup.sh, so the program it runs is there first.
    assert sent.index(script("filedrawer-backup")) < at("backup.sh")
    # The install script that knows production goes to the server.
    assert script("filedrawer-install") in sent
    assert "ENVIRONMENT=staging" not in " ".join(
        c for c, s in zip(calls, sent) if not s.endswith(script("service-user.sh"))
    )


def test_production_sends_each_secret_on_stdin_and_shows_or_saves_none(full_run):
    result = full_run
    assert result.returncode == 0, result.stdout + result.stderr
    sent = result.sent
    assert f"{SMTP_PASSWORD}\n" + script("mail.sh") in sent
    assert f"{PROD_KEY}\n" + script("screening.sh") in sent
    assert f"{GITHUB_TOKEN}\n" + script("github-token.sh") in sent
    assert f"{PROD_CLIENT_SECRET}\n" + script("github-oauth.sh") in sent
    assert f"{DAILY_PING}\t{BACKUP_PING}\n" + script("ping.sh") in sent
    assert f"ops@filedrawer.dev\t{CADDY_PING}\n" + script("caddy-upgrades.sh") in sent
    shown = result.log + result.env_file + result.stdout + result.stderr
    for secret in (
        SMTP_PASSWORD,
        PROD_KEY,
        GITHUB_TOKEN,
        PROD_CLIENT_SECRET,
        DAILY_PING,
        BACKUP_PING,
        CADDY_PING,
    ):
        assert secret not in shown, secret


def test_production_saves_its_values_apart_from_stagings(full_run):
    result = full_run
    assert result.returncode == 0, result.stdout + result.stderr
    saved = result.env_file
    assert "PROD_SMTP_USER='prod@filedrawer.dev'" in saved
    assert "PROD_MAIL_FROM='File Drawer <noreply@filedrawer.dev>'" in saved
    assert "PROD_SCREENING_DAILY_USD='1'" in saved
    assert f"PROD_GITHUB_CLIENT_ID='{PROD_CLIENT_ID}'" in saved
    assert "\nGITHUB_CLIENT_ID=" not in saved
    # Staging's stay as they were.
    assert "SMTP_USER='staging@filedrawer.dev'" in saved
    assert "SCREENING_DAILY_USD='5'" in saved
    log = result.log
    assert "SMTP_USER=prod@filedrawer.dev" in log
    assert "DAILY_USD=1 " in log


def test_production_never_offers_stagings_saved_values(full_run):
    """Staging's mail user and cap are in ops/.vps.env, but production's prompts
    offer no saved value to keep."""
    result = full_run
    assert result.returncode == 0, result.stdout + result.stderr
    assert "[Enter keeps current]" not in result.stdout


def test_production_lists_the_three_names_dns_needs(production):
    # Asked to list them all, though they resolve: --ask-all pauses in the banner
    # and the first four stages too.
    result = run_wizard(
        "--ask-all",
        "production",
        cwd=production.env_file.parent.parent,
        answers="\n" * 6 + FULL_RUN,
        env=production.host.env,
    )
    assert result.returncode == 0, result.stdout + result.stderr
    for name in ("filedrawer.dev", "www.filedrawer.dev", "files.filedrawer.dev"):
        assert f"{name}  A     203.0.113.7" in result.stdout, name
        assert f"{name}  AAAA  2001:db8::7" in result.stdout, name
    assert "staging.filedrawer.dev  A" not in result.stdout


def test_production_says_the_spend_limit_and_the_healthchecks_periods(full_run):
    result = full_run
    assert "$40" in result.stdout
    assert "Period: 1 day" in result.stdout
    assert result.opened is not None
    assert "https://console.anthropic.com/settings/keys" in result.opened
    assert "https://healthchecks.io/" in result.opened


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
    # The stage asks nothing, so it doesn't pause either.
    answers = FULL_RUN.replace(f"{GITHUB_TOKEN}\n{GITHUB_TOKEN}\n\n", "")
    result = production.run(answers=answers)
    assert result.returncode == 0, result.stdout + result.stderr
    assert not any(s.endswith(script("github-token.sh")) for s in production.sent())
    assert "shared with staging" in result.stdout


def test_production_ends_listing_the_settings_the_server_still_lacks(full_run):
    result = full_run
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


def test_production_says_to_run_the_account_stage_after_the_first_deploy(full_run):
    result = full_run
    assert "ops/provision-vps production account" in result.stdout


def test_production_refuses_stagings_sending_user(production):
    answers = FULL_RUN.replace("prod@filedrawer.dev\n", "staging@filedrawer.dev\n", 1)
    result = production.run(answers=answers)
    assert result.returncode != 0
    assert "staging's sending user" in result.stdout
    assert not any(s.endswith(script("mail.sh")) for s in production.sent())


def test_production_checks_the_site_from_outside(full_run):
    result = full_run
    assert result.returncode == 0, result.stdout + result.stderr
    assert "503" in result.stdout and "✓" in result.stdout
    curl = result.calls("curl")
    [tags] = [c for c in curl if "https://filedrawer.dev/tags" in c]
    # A 503's Retry-After (120 s) would otherwise stretch curl's retries to minutes.
    assert "--retry-max-time 30" in tags
    assert any("https://www.filedrawer.dev/" in c for c in curl)


def test_a_production_rerun_asks_for_nothing_the_server_has(production_rerun):
    result = production_rerun
    assert result.returncode == 0, result.stdout + result.stderr
    sent = result.sent
    assert any(s.endswith(script("service.sh")) for s in sent)
    assert any(s.endswith(script("caddy.sh")) for s in sent)
    # Each secret is kept: its script goes, with the secret empty.
    for kept in ("mail.sh", "screening.sh", "github-oauth.sh"):
        assert "\n" + script(kept) in sent, kept
    assert "\t\n" + script("ping.sh") in sent
    assert not any(s.endswith(script("github-token.sh")) for s in sent)
    assert "won't start" not in result.stdout
    # The guard's program, apt file and units still go; its settings stay.
    assert script("caddy-upgrade-check") in sent
    assert "ops@filedrawer.dev\t\n" + script("caddy-upgrades.sh") in sent
    assert "--test-mail" not in " ".join(result.calls())


def test_caddy_upgrades_alone_takes_an_address_and_a_ping_url(stage):
    result = stage.run("caddy-upgrades", f"me@example.org\n{CADDY_PING}\ny\n\n")
    assert result.returncode == 0, result.stdout + result.stderr
    assert stage.scripts()[:4] == [
        script("filedrawer-alerts.sh"),
        script("caddy-upgrade-check"),
        script("filedrawer-reboot-check"),
        f"me@example.org\t{CADDY_PING}\n" + script("caddy-upgrades.sh"),
    ]
    assert "https://healthchecks.io/" in stage.opened.read_text()
    for where in (stage.log.read_text(), stage.env_file.read_text(), result.stdout):
        assert CADDY_PING not in where


def test_reboots_alone_installs_the_programs_then_the_units_and_runs_a_dry_run(stage):
    result = stage.run("reboots", "y\n\n")
    assert result.returncode == 0, result.stdout + result.stderr
    assert stage.scripts()[:4] == [
        script("filedrawer-alerts.sh"),
        script("caddy-upgrade-check"),
        script("filedrawer-reboot-check"),
        script("reboots.sh"),
    ]
    assert "/usr/local/sbin/filedrawer-reboot-check --dry-run" in stage.host.calls()
    assert "Reboot check: a test" in result.stdout
    assert "test mail didn't arrive" not in result.stdout


def test_a_reboots_rerun_runs_no_dry_run_and_asks_nothing(stage):
    stage.host.answer("test -f /etc/systemd/system/filedrawer-reboot-check-daily.timer")
    result = stage.run("reboots", "")
    assert result.returncode == 0, result.stdout + result.stderr
    assert script("reboots.sh") in stage.scripts()
    assert not any("--dry-run" in call for call in stage.host.calls())


@pytest.mark.parametrize(
    "answers",
    [
        "me@example.org\nhttps://example.com/x\n",
        f"not an address\n{CADDY_PING}\n",
    ],
)
def test_caddy_upgrades_refuses_what_it_cant_use_and_sends_no_settings(stage, answers):
    result = stage.run("caddy-upgrades", answers + "\n" * 4)
    assert result.returncode != 0
    assert not any(
        stage.sent(n + 1).endswith(script("caddy-upgrades.sh"))
        for n in range(len(stage.host.calls()))
    )


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
        "production",
        "account",
        cwd=production.env_file.parent.parent,
        answers="y\n",
        env=production.host.env,
    )
    assert result.returncode == 0, result.stdout + result.stderr
    assert "ENVIRONMENT=production ADMIN_ACCOUNT=a-prod1" in production.log.read_text()
    assert "https://filedrawer.dev/account" not in production.opened.read_text()


def test_production_opens_the_workspaces_page_before_the_keys(full_run):
    assert full_run.opened is not None
    opened = full_run.opened.splitlines()
    assert opened.index(
        "https://console.anthropic.com/settings/workspaces"
    ) < opened.index("https://console.anthropic.com/settings/keys")


def test_production_asks_the_owner_to_turn_on_hetzners_backups(full_run):
    result = full_run
    assert result.returncode == 0, result.stdout + result.stderr
    assert result.opened is not None
    assert "https://console.hetzner.cloud/" in result.opened
    assert "Enable backups" in result.stdout
    # Shown once: the next rerun leaves it out.
    assert "PROD_HETZNER_BACKUPS_SHOWN='yes'" in result.env_file


def test_a_production_rerun_leaves_hetzners_backups_to_the_first_run(
    production_rerun,
):
    result = production_rerun
    assert result.returncode == 0, result.stdout + result.stderr
    assert "hetzner" not in (result.opened or "")


def test_production_refuses_a_stage_it_doesnt_have(production):
    result = production.run("nonsense", answers="")
    assert result.returncode != 0
    assert "production account" in result.stdout + result.stderr


# Every script in ops/remote/ reaches the server on a rerun, or is exempt below with
# when it does reach it. The owner reruns the wizard to bring the server up to
# date, so a script it skips keeps the old version
# there: a stricter sudo rule in deploy-user.sh once never left this machine. A new
# script fails these tests until it is sent or exempt.

REMOTE = REPO / "ops/remote"


def remote_scripts() -> list[Path]:
    return [path for path in REMOTE.iterdir() if path.is_file()]


# Scripts the wizard installs on the server as programs, never runs, and where.
INSTALLED = {
    "filedrawer-receive": "/usr/local/bin",
    "filedrawer-install": "/usr/local/sbin",
    "filedrawer-backup": "/usr/local/sbin",
    "caddy-upgrade-check": "/usr/local/sbin",
    "filedrawer-alerts.sh": "/usr/local/lib",
    "filedrawer-reboot-check": "/usr/local/sbin",
}

# Each reason says when the script does reach the server.
FIELD_TEST_KEY = (
    "The owner runs it through ops/field-test-key, to open or close a field-test key."
)
CADDY_GUARD = (
    "Production's wizard sends it on every run, and ops/provision-vps caddy-upgrades: "
    "the guard serves the one server's Caddy."
)
AUDIT_FACTS = "ops/audit-vps sends it, when the owner audits the server."
STAGING_RERUN_EXEMPT = {
    "audit-facts.sh": AUDIT_FACTS,
    "deploy-facts.sh": "ops/deploy-production sends it, before each production deploy.",
    "ci-account.sh": "Sent when a CI account's ID or token is missing, or when a "
    "run with --ask-all (ops/provision-vps or ops/provision-vps smoke) is asked for "
    "a new token.",
    "ping.sh": "Production's wizard sends it on every run.",
    "backup.sh": "Production's wizard sends it on every run: only production has "
    "backups.",
    "filedrawer-backup": "Production's wizard installs it on every run, for "
    "backup.sh's timer.",
    "field-test-key.sh": FIELD_TEST_KEY,
    "caddy-upgrades.sh": CADDY_GUARD,
    "caddy-upgrade-check": CADDY_GUARD,
    "filedrawer-alerts.sh": CADDY_GUARD,
    "filedrawer-reboot-check": CADDY_GUARD,
    "reboots.sh": "Production's wizard sends it on every run, and ops/provision-vps "
    "reboots: the reboots serve the one server.",
}
PRODUCTION_RERUN_EXEMPT = {
    "audit-facts.sh": AUDIT_FACTS,
    "base.sh": "Staging's wizard sends it; production shares staging's server.",
    "deploy-facts.sh": "ops/deploy-production sends it, before each production deploy.",
    "deploy-user.sh": "Staging's wizard sends it, for staging's deploy key; "
    "production deploys run ops/deploy-production with the owner's own key.",
    "filedrawer-receive": "Staging's wizard installs it; only staging's deploy key "
    "runs it.",
    "ci-account.sh": "Staging's wizard sends it, as its own exemption says: CI's "
    "accounts are on staging.",
    "smoke-env.sh": "Staging's wizard sends it: the smoke test runs on staging.",
    "admin-grant.sh": "ops/provision-vps production account sends it, after the "
    "first deploy.",
    "github-token.sh": "Sent only when /etc/filedrawer/github.env, which staging "
    "shares, is missing, or by staging's wizard (see its exemption).",
    "field-test-key.sh": FIELD_TEST_KEY,
}


def sent_by_name(calls: list[str], stdins: list[str]) -> dict[str, list[str]]:
    """The scripts in ops/remote/ that the ssh `calls` sent on `stdins`, each with
    the remote command lines that took it. A secret's line may come first."""
    scripts = {path.name: path.read_text() for path in remote_scripts()}
    sent: dict[str, list[str]] = {}
    for call, stdin in zip(calls, stdins, strict=True):
        names = [name for name, text in scripts.items() if stdin.endswith(text)]
        assert len(names) <= 1, names
        for name in names:
            sent.setdefault(name, []).append(call)
    return sent


def assert_every_script_sent_or_exempt(
    commands: list[str], stdins: list[str], exempt: dict[str, str]
) -> None:
    """Of the ssh `commands`, sent on `stdins`, each script in ops/remote/ is sent,
    the way its kind goes to the server, or exempt."""
    scripts = {path.name for path in remote_scripts()}
    assert set(exempt) <= scripts, (
        f"exempt, not in ops/remote/: {set(exempt) - scripts}"
    )
    sent = sent_by_name(commands, stdins)
    unplaced = scripts - set(sent) - set(exempt)
    assert not unplaced, (
        f"a rerun sends none of {sorted(unplaced)}: send each, "
        "or exempt it with when it reaches the server"
    )
    stale = set(sent) & set(exempt)
    assert not stale, f"exempt, yet a rerun sends {sorted(stale)}"
    for name, calls in sent.items():
        if name in INSTALLED:
            install = (
                f"install -o root -g root -m 0755 /dev/stdin {INSTALLED[name]}/{name}"
            )
            assert calls == [install] * len(calls), (name, calls)
        else:
            assert all(call.endswith("bash -s") for call in calls), (name, calls)


def test_a_rerun_sends_every_remote_script_or_exempts_it(rerun):
    # CI's screen account is saved too, so the rerun keeps every account and secret.
    env_file = rerun.tmp / "ops/.vps.env"
    env_file.write_text(env_file.read_text() + "CI_SCREEN_ACCOUNT='a-ci'\n")
    (rerun.secrets / "CI_SCREEN_TOKEN").write_text("kept")
    result = rerun.run()
    assert result.returncode == 0, result.stdout + result.stderr
    assert_every_script_sent_or_exempt(
        rerun.host.calls(), rerun.sent(), STAGING_RERUN_EXEMPT
    )


def test_a_production_rerun_sends_every_remote_script_or_exempts_it(
    production_rerun,
):
    result = production_rerun
    assert result.returncode == 0, result.stdout + result.stderr
    assert_every_script_sent_or_exempt(
        result.calls(), list(result.sent), PRODUCTION_RERUN_EXEMPT
    )


# The outbound probe once the scripts have run (#102)

PROBE_STAGING = "/usr/local/sbin/filedrawer-install check-outbound staging"
PROBE_PRODUCTION = "/usr/local/sbin/filedrawer-install check-outbound production"
INSTALL_INSTALL = (
    "install -o root -g root -m 0755 /dev/stdin /usr/local/sbin/filedrawer-install"
)


def test_a_staging_run_ends_probing_stagings_way_out_with_the_current_install(rerun):
    rerun.host.answer(PROBE_STAGING, stdout="probe: reached api.github.com\n")
    result = rerun.run()
    assert result.returncode == 0, result.stdout + result.stderr
    calls = rerun.host.calls()
    # The probe, then the closing check of the server's secrets.
    assert calls[-2] == PROBE_STAGING
    assert calls[-3] == INSTALL_INSTALL
    assert rerun.sent()[-3] == script("filedrawer-install")
    assert rerun.sent()[-1] == script("check-secrets.sh")
    assert "staging's way out: checked" in result.stdout


def test_a_binary_that_cant_probe_yet_isnt_called_checked(rerun):
    rerun.host.answer(
        PROBE_STAGING, stdout="staging has no binary yet; its first deploy probes\n"
    )
    result = rerun.run()
    assert result.returncode == 0, result.stdout + result.stderr
    assert "its first deploy probes" in result.stdout
    assert "way out: checked" not in result.stdout


def test_a_failed_probe_warns_and_lists_the_hosts_but_finishes(rerun):
    # As filedrawer-install check-outbound answers, its stderr after its stdout.
    rerun.host.answer(
        PROBE_STAGING,
        stdout="probe: reached SMTP\nprobe: can't reach api.github.com: refused\n"
        "staging's server can't reach: api.github.com\n",
        status=3,
    )
    result = rerun.run()
    assert result.returncode == 0, result.stdout + result.stderr
    closing = result.stdout.split("Setup complete")[1]
    assert (
        "staging's outbound probe failed (exit 3; can't reach api.github.com)"
        in closing
    )


@pytest.mark.parametrize(
    ("name", "answers", "probes"),
    [
        ("screening", f"0.25\n{API_KEY}\n{API_KEY}\n\n", True),
        ("github", f"{GITHUB_TOKEN}\n{GITHUB_TOKEN}\n\n", True),
        ("github-oauth", f"y\n{CLIENT_ID}\n{CLIENT_SECRET}\n{CLIENT_SECRET}\n\n", True),
        ("caddy-upgrades", "\n" + "https://hc-ping.com/" + "c" * 36 + "\ny\n", False),
    ],
)
def test_a_stage_alone_probes_staging_when_it_restarts_it(stage, name, answers, probes):
    stage.run(name, answers)
    assert (PROBE_STAGING in stage.host.calls()) == probes


def test_production_probes_its_way_out_with_the_current_install(full_run):
    result = full_run
    assert result.returncode == 0, result.stdout + result.stderr
    calls = result.calls()
    probe = calls.index(PROBE_PRODUCTION)
    assert calls[probe - 1] == INSTALL_INSTALL
    assert PROBE_STAGING not in calls


@pytest.mark.parametrize("args", [["screenng"], ["--only-new", "caddy-upgrade"]])
def test_an_argument_it_doesnt_know_is_refused_before_any_stage(stage, args):
    """A typo once ran the whole staging setup, as a test of a stage not yet written
    found when it hung there."""
    result = run_wizard(
        *args, cwd=stage.env_file.parent.parent, answers="", env=stage.host.env
    )
    assert result.returncode == 2, result.stdout + result.stderr
    assert args[-1] in result.stdout + result.stderr
    assert "caddy-upgrades" in result.stdout + result.stderr
    assert stage.host.calls() == []


# Reruns keep what's set up; --ask-all changes it (#112)

CADDY_ENV = "/etc/filedrawer/caddy-upgrades.env"


@pytest.fixture
def held(stage):
    """`stage`, on a server that holds the guard's settings and staging's screening,
    with the guard's address and the cap saved."""
    stage.host.answer("test -f *", status=1)
    for path in (CADDY_ENV, "/etc/filedrawer/staging-screening.env"):
        stage.host.answer(f"test -f {path}")
    stage.env_file.write_text(
        "VPS_HOST='h'\nCADDY_ALERT_TO='me@example.org'\nSCREENING_DAILY_USD='2'\n"
    )
    return stage


def guard_settings(stage) -> list[str]:
    script_text = script("caddy-upgrades.sh")
    return [
        sent[: -len(script_text)]
        for sent in (stage.sent(n + 1) for n in range(len(stage.host.calls())))
        if sent.endswith(script_text)
    ]


def test_a_plain_rerun_keeps_the_guards_ping_url_and_asks_nothing(held):
    """The owner's case: rerunning the stage they set up keeps the URL given before."""
    result = held.run("caddy-upgrades", "")
    assert result.returncode == 0, result.stdout + result.stderr
    assert guard_settings(held) == ["me@example.org\t\n"]
    assert not held.opened.exists()
    assert "--test-mail" not in " ".join(held.host.calls())
    assert "--ask-all caddy-upgrades" in result.stdout


def test_ask_all_keeps_a_held_secret_on_no_without_its_tab(held):
    result = held.run("--ask-all", "caddy-upgrades", "\n\n\n")
    assert result.returncode == 0, result.stdout + result.stderr
    assert "Change the server's" in result.stdout
    assert guard_settings(held) == ["me@example.org\t\n"]
    assert not held.opened.exists()


def test_ask_all_on_yes_opens_the_tab_and_takes_a_new_url(held):
    result = held.run("--ask-all", "caddy-upgrades", f"\ny\n{CADDY_PING}\ny\n\n")
    assert result.returncode == 0, result.stdout + result.stderr
    assert guard_settings(held) == [f"me@example.org\t{CADDY_PING}\n"]
    assert "https://healthchecks.io/" in held.opened.read_text()


def test_ask_all_on_yes_refuses_an_empty_answer(held):
    """A paste that delivered nothing mustn't keep the old URL silently."""
    result = held.run("--ask-all", "caddy-upgrades", "\ny\n\n\n\n")
    assert result.returncode != 0
    assert guard_settings(held) == []


def test_a_plain_rerun_sends_a_held_key_empty_with_the_saved_cap(held):
    result = held.run("screening", "")
    assert result.returncode == 0, result.stdout + result.stderr
    assert "\n" + script("screening.sh") in held.scripts()
    assert "DAILY_USD=2 bash -s" in held.log.read_text()
    assert not held.opened.exists()


def test_an_ssh_failure_stops_the_wizard_rather_than_ask_again(held):
    held.host.answer(f"test -f {CADDY_ENV}", status=255)
    result = held.run("caddy-upgrades", "")
    assert result.returncode != 0
    assert "ssh" in result.stdout
    assert guard_settings(held) == []


@pytest.mark.parametrize(
    ("args", "script_name", "first_line"),
    [
        (["mail"], "mail.sh", ""),
        (["production", "mail"], "mail.sh", ""),
        (["production", "screening"], "screening.sh", ""),
        (["production", "pings"], "ping.sh", "\t"),
        (["github-oauth"], "github-oauth.sh", ""),
        (["production", "github-oauth"], "github-oauth.sh", ""),
    ],
)
def test_each_secret_has_a_stage_of_its_own(stage, args, script_name, first_line):
    stage.host.answer("test -f *")
    stage.env_file.write_text(
        "VPS_HOST='h'\nSMTP_USER='s@x.dev'\nMAIL_FROM='S <s@x.dev>'\n"
        "PROD_SMTP_USER='p@x.dev'\nPROD_MAIL_FROM='P <p@x.dev>'\n"
        "PROD_SCREENING_DAILY_USD='1'\n"
    )
    result = run_wizard(
        *args, cwd=stage.env_file.parent.parent, answers="", env=stage.host.env
    )
    assert result.returncode == 0, result.stdout + result.stderr
    sent = [stage.sent(n + 1) for n in range(len(stage.host.calls()))]
    assert first_line + "\n" + script(script_name) in sent


def test_a_plain_rerun_sends_each_held_secret_empty(plain_rerun):
    result = plain_rerun
    assert result.returncode == 0, result.stdout + result.stderr
    sent = result.sent
    for name in ("mail.sh", "screening.sh", "github-token.sh", "github-oauth.sh"):
        assert "\n" + script(name) in sent, name
    assert "ssh-keygen" not in result.log


def test_ask_all_with_enter_everywhere_keeps_everything(rerun):
    result = rerun.run("--ask-all", answers="\n" * 80)
    assert result.returncode == 0, result.stdout + result.stderr
    sent = rerun.sent()
    for name in ("mail.sh", "screening.sh", "github-token.sh", "github-oauth.sh"):
        assert "\n" + script(name) in sent, name
    assert "Make a new deploy key" in result.stdout
    log = rerun.log.read_text()
    assert "ssh-keygen" not in log
    assert "gh secret set DEPLOY_SSH_KEY" not in log


def test_a_gh_failure_stops_before_making_a_new_deploy_key(rerun):
    rerun.host.answer("secret list*", status=1, tool="gh")
    result = rerun.run()
    assert result.returncode != 0
    assert "ssh-keygen" not in rerun.log.read_text()


def test_a_first_run_still_asks_before_the_base_system(rerun):
    """Default skips what's done; base.sh turns off password logins, so a server it
    never ran on still gets the question."""
    rerun.present(
        "/etc/filedrawer/staging-mail.env",
        "/etc/filedrawer/staging-screening.env",
        "/etc/filedrawer/github.env",
    )
    result = rerun.run()
    assert "Run it on" in result.stdout
    assert script("base.sh") not in rerun.sent()


def test_an_unknown_argument_names_ask_all(stage):
    result = stage.run("--force", "")
    assert result.returncode == 2
    assert "--ask-all" in result.stdout


def test_a_rerun_lists_what_it_kept_after_the_closing_summary(held):
    """Each stage clears the screen, so what a kept stage said goes at the end."""
    result = held.run("caddy-upgrades", "")
    after = result.stdout.split("Setup complete", 1)[1]
    assert "guard's ping URL" in after
    assert "ops/provision-vps --ask-all caddy-upgrades" in after


def test_a_rerun_sends_empty_what_only_the_server_holds(held):
    """An ops/.vps.env without the cap or the address mustn't put a default over
    the server's."""
    held.env_file.write_text("VPS_HOST='h'\n")
    held.host.answer("test -f /etc/filedrawer/staging-mail.env")
    for name in ("screening", "caddy-upgrades", "mail"):
        result = held.run(name, "")
        assert result.returncode == 0, name + result.stdout + result.stderr
    log = held.log.read_text()
    assert "DAILY_USD= bash -s" in log
    assert "SMTP_USER= MAIL_FROM= bash -s" in log
    assert guard_settings(held) == ["\t\n"]
    assert held.env_file.read_text() == "VPS_HOST='h'\n"


def test_a_first_run_waits_at_the_banner(stage):
    stage.env_file.write_text("")
    first = run_wizard(cwd=stage.env_file.parent.parent, answers="", env=stage.host.env)
    assert "Ready to start?" in first.stdout


def test_a_rerun_doesnt_wait_at_the_banner(plain_rerun):
    assert "Ready to start?" not in plain_rerun.stdout


def test_a_kept_deploy_key_says_how_to_replace_it(plain_rerun):
    result = plain_rerun
    lines = [line for line in result.stdout.splitlines() if "DEPLOY_SSH_KEY" in line]
    assert lines and all("--ask-all" in line for line in lines), lines


def test_pings_ask_only_for_the_url_the_server_lacks(production):
    production.env_file.write_text(production.env_file.read_text())
    production.host.answer("test -f /etc/filedrawer/production-ping.env")
    result = production.run("pings", answers=f"{BACKUP_PING}\n\n")
    assert result.returncode == 0, result.stdout + result.stderr
    assert f"\t{BACKUP_PING}\n" + script("ping.sh") in production.sent()
    assert "daily email's ping URL:" not in result.stdout


# Each run ends checking the secrets the server holds (#112)

CHECK_SECRETS = "# Checks each secret the server holds"


def test_each_run_ends_with_the_servers_secrets_checked(held):
    held.host.answer(
        "bash -s",
        stdout="ok: the Caddy guard's ping URL: healthchecks.io knows it\n",
        stdin=CHECK_SECRETS,
    )
    result = held.run("caddy-upgrades", "")
    assert result.returncode == 0, result.stdout + result.stderr
    assert script("check-secrets.sh") in held.scripts()
    after = result.stdout.split("Setup complete", 1)[1]
    assert "ok: the Caddy guard's ping URL: healthchecks.io knows it" in after


def test_a_check_that_stops_is_left_to_do(held):
    held.host.answer("bash -s", status=255, stdin=CHECK_SECRETS)
    result = held.run("caddy-upgrades", "")
    assert result.returncode == 0, result.stdout + result.stderr
    todo = result.stdout.split("still to do by hand:", 1)[1]
    assert "the server's secrets: their check stopped (exit 255)" in todo


@pytest.mark.parametrize("word", ["failed", "unchecked"])
def test_a_secret_that_fails_its_check_is_left_to_do(held, word):
    failed = f"{word}: the GitHub token: GitHub refused it"
    held.host.answer("bash -s", stdout=failed + "\n", status=1, stdin=CHECK_SECRETS)
    result = held.run("caddy-upgrades", "")
    assert result.returncode == 0, result.stdout + result.stderr
    todo = result.stdout.split("still to do by hand:", 1)[1]
    assert failed in todo
