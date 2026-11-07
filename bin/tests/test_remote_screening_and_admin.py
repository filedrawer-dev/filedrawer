"""Tests for ops/remote/screening.sh, ops/remote/github-token.sh and
ops/remote/admin-grant.sh, which ops/provision-vps runs as root on the VPS:
staging's screening settings, the GitHub token the link checks call GitHub's API
with, and making the owner's staging account the admin. Each runs as a process
against a temp root, with fake systemctl and runuser first on PATH.
"""

import os
import stat
import subprocess
from pathlib import Path
from types import SimpleNamespace

import pytest

REPO = Path(__file__).resolve().parents[2]
SCREENING = REPO / "ops/remote/screening.sh"
GITHUB_TOKEN = REPO / "ops/remote/github-token.sh"
ADMIN_GRANT = REPO / "ops/remote/admin-grant.sh"
KEY = "sk-ant-api03-" + "x" * 40


def executable(path: Path, text: str) -> Path:
    path.write_text(text)
    path.chmod(path.stat().st_mode | stat.S_IXUSR)
    return path


@pytest.fixture
def box(tmp_path):
    root = tmp_path / "root"
    (root / "etc/filedrawer").mkdir(parents=True)
    fakes = tmp_path / "fakes"
    fakes.mkdir()
    log = tmp_path / "log"
    log.touch()
    for tool in ("systemctl", "chown"):
        executable(fakes / tool, f'#!/bin/sh\necho "{tool} $*" >> {log}\n')
    executable(
        fakes / "runuser",
        f'#!/bin/sh\necho "runuser $*" >> {log}\nshift 3\nexec "$@"\n',
    )
    env = {
        "PATH": f"{fakes}:{os.environ['PATH']}",
        "FILEDRAWER_ROOT": str(root),
    }
    return SimpleNamespace(root=root, log=log, env=env, tmp=tmp_path)


def run(box, script, stdin="", **env):
    return subprocess.run(
        ["bash", script],
        input=stdin,
        env={**box.env, **env},
        capture_output=True,
        text=True,
        check=False,
    )


def screening(box, key=KEY, **env):
    """Runs screening.sh as the wizard does: the key as stdin's first line, read
    into $API_KEY by the remote shell."""
    return run(box, SCREENING, **{"API_KEY": key, "DAILY_USD": "1", **env})


def settings(box) -> dict[str, str]:
    text = (box.root / "etc/filedrawer/staging-screening.env").read_text()
    pairs = (line.split("=", 1) for line in text.splitlines() if line)
    return {name: value for name, value in pairs}


def test_screening_writes_the_settings_the_server_reads_and_restarts_staging(box):
    result = screening(box, DAILY_USD="0.50")
    assert result.returncode == 0, result.stderr
    assert settings(box) == {
        "FILEDRAWER_SCREENER": "anthropic",
        "FILEDRAWER_ANTHROPIC_API_KEY": f"'{KEY}'",
        "FILEDRAWER_SCREENING_DAILY_USD": "0.50",
    }
    env_file = box.root / "etc/filedrawer/staging-screening.env"
    assert stat.S_IMODE(env_file.stat().st_mode) == 0o640
    log = box.log.read_text()
    assert f"chown root:filedrawer {env_file}" in log
    assert "systemctl try-restart filedrawer-staging.service" in log
    assert KEY not in result.stdout + result.stderr


TOKEN = "github_pat_" + "t" * 70


def test_github_token_writes_the_shared_file_and_restarts_staging(box):
    result = run(box, GITHUB_TOKEN, TOKEN=TOKEN)
    assert result.returncode == 0, result.stderr
    env_file = box.root / "etc/filedrawer/github.env"
    assert env_file.read_text() == f"FILEDRAWER_GITHUB_TOKEN='{TOKEN}'\n"
    assert stat.S_IMODE(env_file.stat().st_mode) == 0o640
    log = box.log.read_text()
    assert f"chown root:filedrawer {env_file}" in log
    assert "systemctl try-restart filedrawer-staging.service" in log
    assert TOKEN not in result.stdout + result.stderr


@pytest.mark.parametrize(
    "token", ["github_pat_it's", "github_pat_a\nb", "sk-ant-api03-xyz", ""]
)
def test_github_token_refuses_what_isnt_a_token_and_writes_nothing(box, token):
    result = run(box, GITHUB_TOKEN, TOKEN=token)
    assert result.returncode != 0
    assert not (box.root / "etc/filedrawer/github.env").exists()


def test_a_rerun_replaces_the_settings(box):
    screening(box)
    other = "sk-ant-api03-" + "y" * 40
    assert screening(box, key=other, DAILY_USD="2").returncode == 0
    assert settings(box)["FILEDRAWER_ANTHROPIC_API_KEY"] == f"'{other}'"
    assert settings(box)["FILEDRAWER_SCREENING_DAILY_USD"] == "2"


@pytest.mark.parametrize(
    "key, daily",
    [
        ("sk-ant-it's", "1"),
        ("sk-ant-a\nb", "1"),
        ("not-a-key", "1"),
        ("", "1"),
        (KEY, "-1"),
        (KEY, "1e3"),
        (KEY, "one"),
        (KEY, ""),
        (KEY, "1000.01"),
    ],
)
def test_screening_refuses_a_bad_key_or_cap_and_writes_nothing(box, key, daily):
    result = screening(box, key=key, DAILY_USD=daily)
    assert result.returncode != 0
    assert not (box.root / "etc/filedrawer/staging-screening.env").exists()
    assert "systemctl" not in box.log.read_text()


def test_the_staging_unit_reads_the_screening_settings_after_its_own():
    unit = (REPO / "ops/remote/staging-service.sh").read_text()
    own = unit.index("EnvironmentFile=/etc/filedrawer/staging.env")
    screening_file = unit.index(
        "EnvironmentFile=-/etc/filedrawer/staging-screening.env"
    )
    assert own < screening_file


# admin-grant.sh

FAKE_FILEDRAWER = """#!/bin/bash
echo "filedrawer $* db=$FILEDRAWER_DATABASE" >> "$LOG"
[[ "$4" == a-known ]] && { echo "$4"; exit 0; }
echo "filedrawer: no account $4" >&2
exit 1
"""


@pytest.fixture
def staged(box):
    bin_dir = box.root / "srv/filedrawer/staging/bin"
    bin_dir.mkdir(parents=True)
    executable(bin_dir / "filedrawer", FAKE_FILEDRAWER)
    box.env["LOG"] = str(box.log)
    return box


def test_admin_grant_runs_grant_as_the_service_user_on_stagings_database(staged):
    result = run(staged, ADMIN_GRANT, ADMIN_ACCOUNT="a-known")
    assert result.returncode == 0, result.stderr
    log = staged.log.read_text()
    assert "runuser -u filedrawer --" in log
    database = staged.root / "srv/filedrawer/staging/data/filedrawer.db"
    assert f"filedrawer admin grant --account a-known db={database}" in log


def test_admin_grant_passes_on_the_servers_refusal(staged):
    result = run(staged, ADMIN_GRANT, ADMIN_ACCOUNT="a-nobody")
    assert result.returncode == 1
    assert "no account a-nobody" in result.stderr


@pytest.mark.parametrize(
    "account", ["", "a-", "x-known", "a-known; rm -rf /", "a-KNOWN"]
)
def test_admin_grant_refuses_an_id_that_isnt_one(staged, account):
    result = run(staged, ADMIN_GRANT, ADMIN_ACCOUNT=account)
    assert result.returncode == 2, result.stderr
    assert "filedrawer" not in staged.log.read_text()


def test_admin_grant_exits_3_before_the_first_deploy(box):
    result = run(box, ADMIN_GRANT, ADMIN_ACCOUNT="a-known")
    assert result.returncode == 3


# smoke-env.sh

SMOKE_ENV = REPO / "ops/remote/smoke-env.sh"


def test_smoke_env_names_the_smoke_account_for_staging_and_restarts_it(box):
    result = run(box, SMOKE_ENV, SMOKE_ACCOUNT="a-smoke1")
    assert result.returncode == 0, result.stderr
    env_file = box.root / "etc/filedrawer/staging-smoke.env"
    assert env_file.read_text() == "FILEDRAWER_SMOKE_ACCOUNT=a-smoke1\n"
    assert stat.S_IMODE(env_file.stat().st_mode) == 0o640
    log = box.log.read_text()
    assert f"chown root:filedrawer {env_file}" in log
    assert "systemctl try-restart filedrawer-staging.service" in log


@pytest.mark.parametrize("account", ["", "a-", "a-x y", "a-X", "b-smoke"])
def test_smoke_env_refuses_an_id_that_isnt_one(box, account):
    result = run(box, SMOKE_ENV, SMOKE_ACCOUNT=account)
    assert result.returncode == 2
    assert not (box.root / "etc/filedrawer/staging-smoke.env").exists()


def test_the_staging_unit_reads_the_smoke_account_setting():
    unit = (REPO / "ops/remote/staging-service.sh").read_text()
    assert "EnvironmentFile=-/etc/filedrawer/staging-smoke.env" in unit


# caddy.sh

CADDY = REPO / "ops/remote/caddy.sh"
STAGING_KEY = "ab" * 32
OLD_HASH = "$2a$14$abcdefghijklmnopqrstuuABCDEFGHIJKLMNOPQRSTUVWXYZ01234"


@pytest.fixture
def caddy_box(box):
    (box.root / "etc/caddy").mkdir(parents=True)
    executable(
        box.tmp / "fakes" / "caddy",
        f'#!/bin/sh\necho "caddy $*" >> {box.log}\n'
        "[ \"$1\" = hash-password ] && echo '$2a$14$NEWNEWNEWNEWNEWNEWNEWNuNEWNEWNEWNEWNEWNEWNEWNEWNEWNEWNE'\n"
        "exit 0\n",
    )
    return box


def caddyfile(box) -> str:
    return (box.root / "etc/caddy/Caddyfile").read_text()


def test_caddy_without_a_password_keeps_the_hash_already_on_the_server(caddy_box):
    (caddy_box.root / "etc/caddy/Caddyfile").write_text(
        f"staging.filedrawer.dev {{\n\tbasic_auth {{\n\t\towner {OLD_HASH}\n\t}}\n}}\n"
    )
    result = run(
        caddy_box,
        CADDY,
        STAGING_USER="owner",
        STAGING_SECRETS=f"\t{STAGING_KEY}",
    )
    assert result.returncode == 0, result.stderr
    assert f"owner {OLD_HASH}" in caddyfile(caddy_box)
    assert "files.staging.filedrawer.dev" in caddyfile(caddy_box)
    assert "hash-password" not in caddy_box.log.read_text()


def test_caddy_keeps_a_hash_only_for_the_same_user(caddy_box):
    (caddy_box.root / "etc/caddy/Caddyfile").write_text(
        f"staging.filedrawer.dev {{\n\tbasic_auth {{\n\t\tsomeone {OLD_HASH}\n\t}}\n}}\n"
    )
    result = run(
        caddy_box,
        CADDY,
        STAGING_USER="owner",
        STAGING_SECRETS=f"\t{STAGING_KEY}",
    )
    assert result.returncode != 0
    assert "someone" in caddyfile(caddy_box), "the old file is left as it was"


def test_caddy_with_a_password_hashes_it_anew(caddy_box):
    result = run(
        caddy_box,
        CADDY,
        STAGING_USER="owner",
        STAGING_SECRETS=f"s3cret\t{STAGING_KEY}",
    )
    assert result.returncode == 0, result.stderr
    assert "owner $2a$14$NEWNEW" in caddyfile(caddy_box)


def test_caddy_without_a_password_or_a_hash_to_keep_fails_naming_the_full_wizard(
    caddy_box,
):
    result = run(
        caddy_box,
        CADDY,
        STAGING_USER="owner",
        STAGING_SECRETS=f"\t{STAGING_KEY}",
    )
    assert result.returncode != 0
    assert "ops/provision-vps" in result.stderr
    assert not (caddy_box.root / "etc/caddy/Caddyfile").exists()


def test_smoke_env_also_names_the_screen_test_account_when_given(box):
    result = run(
        box, SMOKE_ENV, SMOKE_ACCOUNT="a-smoke1", SCREEN_TEST_ACCOUNT="a-screen1"
    )
    assert result.returncode == 0, result.stderr
    env_file = box.root / "etc/filedrawer/staging-smoke.env"
    assert env_file.read_text() == (
        "FILEDRAWER_SMOKE_ACCOUNT=a-smoke1\nFILEDRAWER_SCREEN_TEST_ACCOUNT=a-screen1\n"
    )
    bad = run(box, SMOKE_ENV, SMOKE_ACCOUNT="a-smoke1", SCREEN_TEST_ACCOUNT="b-x")
    assert bad.returncode == 2


def test_smoke_env_also_names_the_ci_screen_account_when_given(box):
    result = run(box, SMOKE_ENV, SMOKE_ACCOUNT="a-smoke1", CI_SCREEN_ACCOUNT="a-ci1")
    assert result.returncode == 0, result.stderr
    env_file = box.root / "etc/filedrawer/staging-smoke.env"
    assert env_file.read_text() == (
        "FILEDRAWER_SMOKE_ACCOUNT=a-smoke1\nFILEDRAWER_CI_SCREEN_ACCOUNT=a-ci1\n"
    )
    bad = run(box, SMOKE_ENV, SMOKE_ACCOUNT="a-smoke1", CI_SCREEN_ACCOUNT="b-x")
    assert bad.returncode == 2
