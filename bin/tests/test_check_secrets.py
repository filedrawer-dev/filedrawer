"""Tests for ops/remote/check-secrets.sh (#112), which ops/provision-vps runs on the
server at the end of each run: it checks each secret the server holds with the
service that takes it, unattended, and never shows one. Runs against a temp root
with fake_curl.py as curl."""

import json
import stat
import sys
import subprocess
from pathlib import Path
from types import SimpleNamespace

import pytest

REPO = Path(__file__).resolve().parents[2]
CHECK = REPO / "ops/remote/check-secrets.sh"
FAKE_CURL = Path(__file__).with_name("fake_curl.py")
KEY = "sk-ant-api03-" + "k" * 40
TOKEN = "github_pat_" + "t" * 40
DAILY = "https://hc-ping.com/aaaaaaaa-0000-4000-8000-000000000001"
GUARD = "https://hc-ping.com/aaaaaaaa-0000-4000-8000-000000000003"
NOW = 1_800_000_000
DAY = 86400


@pytest.fixture
def box(tmp_path):
    root = tmp_path / "root"
    (root / "etc/filedrawer").mkdir(parents=True)
    fakes = tmp_path / "fakes"
    fakes.mkdir()
    curl = fakes / "curl"
    curl.write_text(f'#!/bin/sh\nexec "{sys.executable}" -I "{FAKE_CURL}" "$@"\n')
    curl.chmod(curl.stat().st_mode | stat.S_IXUSR)
    log = tmp_path / "curl.log"
    rules = tmp_path / "rules.json"
    rules.write_text("[]")

    def answer(*rule):
        rules.write_text(json.dumps([list(rule), *json.loads(rules.read_text())]))

    def put(name, text):
        (root / "etc/filedrawer" / name).write_text(text)

    def run():
        return subprocess.run(
            ["bash", CHECK],
            env={
                "PATH": f"{fakes}:/usr/bin:/bin",
                "FILEDRAWER_ROOT": str(root),
                "FILEDRAWER_NOW": str(NOW),
                "FAKE_CURL_LOG": str(log),
                "FAKE_CURL_RULES": str(rules),
            },
            stdin=subprocess.DEVNULL,
            capture_output=True,
            text=True,
            check=False,
        )

    def calls():
        if not log.exists():
            return []
        return [json.loads(line) for line in log.read_text().splitlines()]

    return SimpleNamespace(answer=answer, put=put, run=run, calls=calls)


def secrets_shown(box, result):
    shown = result.stdout + result.stderr
    argv = " ".join(" ".join(call["argv"]) for call in box.calls())
    return [s for s in (KEY, TOKEN, DAILY, GUARD) if s in shown or s in argv]


def test_a_server_holding_nothing_checks_nothing(box):
    result = box.run()
    assert result.returncode == 0, result.stderr
    assert result.stdout == ""
    assert box.calls() == []


@pytest.mark.parametrize(
    ("status", "line", "code"),
    [
        # A workspace out of credit answers 200 too, so it's "accepted", not "ok".
        (200, "accepted: staging's Anthropic API key", 0),
        (401, "failed: staging's Anthropic API key: Anthropic refused it", 1),
        (529, "unchecked: staging's Anthropic API key: Anthropic answered 529", 0),
    ],
)
def test_the_anthropic_key_goes_in_a_header_on_stdin(box, status, line, code):
    box.put("staging-screening.env", f"FILEDRAWER_ANTHROPIC_API_KEY='{KEY}'\n")
    box.answer("api.anthropic.com/v1/models", status, "{}", "")
    result = box.run()
    assert result.returncode == code
    assert result.stdout.startswith(line), result.stdout
    if code:
        assert "ops/provision-vps --ask-all screening" in result.stdout
    [call] = box.calls()
    assert f'header = "x-api-key: {KEY}"' in call["config"]
    assert secrets_shown(box, result) == []


def test_productions_key_names_its_own_stage(box):
    box.put("production-screening.env", f"FILEDRAWER_ANTHROPIC_API_KEY='{KEY}'\n")
    box.answer("api.anthropic.com", 401, "", "")
    result = box.run()
    assert "production's Anthropic API key" in result.stdout
    assert "--ask-all production screening" in result.stdout


def expiry(days: int) -> str:
    when = subprocess.run(
        ["date", "-u", "-d", f"@{NOW + days * DAY}", "+%Y-%m-%d %H:%M:%S UTC"],
        capture_output=True,
        text=True,
        check=True,
    ).stdout.strip()
    return f"github-authentication-token-expiration: {when}\r\n"


@pytest.mark.parametrize(
    ("status", "headers", "start", "code"),
    [
        (200, "x-ratelimit-limit: 5000\r\n" + expiry(90), "ok: the GitHub token", 0),
        (
            200,
            "x-ratelimit-limit: 5000\r\n" + expiry(5),
            "expiring: the GitHub token",
            0,
        ),
        # Without a token GitHub answers 200 too, with 60 calls an hour: a token
        # that never reached it, so nothing to say of the token itself.
        (200, "x-ratelimit-limit: 60\r\n", "unchecked: the GitHub token", 0),
        (200, "", "unchecked: the GitHub token", 0),
        (401, "", "failed: the GitHub token", 1),
        (403, "", "unchecked: the GitHub token", 0),
    ],
)
def test_the_github_token(box, status, headers, start, code):
    box.put("github.env", f"FILEDRAWER_GITHUB_TOKEN='{TOKEN}'\n")
    box.answer("api.github.com/rate_limit", status, "{}", headers)
    result = box.run()
    assert result.returncode == code
    assert result.stdout.startswith(start), result.stdout
    if start.startswith(("failed", "expiring")):
        assert "ops/provision-vps --ask-all github" in result.stdout
    [call] = box.calls()
    assert f'header = "Authorization: Bearer {TOKEN}"' in call["config"]
    assert secrets_shown(box, result) == []


@pytest.mark.parametrize(
    ("body", "status", "start", "code"),
    [
        ("OK", 200, "ok: the daily email's ping URL", 0),
        # healthchecks.io answers an unknown UUID so, not with a 404.
        ("OK (not found)", 200, "failed: the daily email's ping URL", 1),
        ("not found", 404, "failed: the daily email's ping URL", 1),
        ("OK (rate limited)", 200, "unchecked: the daily email's ping URL", 0),
    ],
)
def test_a_ping_url_is_checked_through_its_log_endpoint(box, body, status, start, code):
    box.put("production-ping.env", f"FILEDRAWER_DAILY_EMAIL_PING={DAILY}\n")
    box.answer("hc-ping.com", status, body, "")
    result = box.run()
    assert result.returncode == code
    assert result.stdout.startswith(start), result.stdout
    [call] = box.calls()
    # /log records an event and leaves the check's state alone.
    assert f'url = "{DAILY}/log"' in call["config"]
    assert secrets_shown(box, result) == []


def test_a_ping_url_with_a_suffix_isnt_sent(box):
    box.put("caddy-upgrades.env", f"ALERT_TO=a@b.dev\nPING={GUARD}/fail\n")
    result = box.run()
    assert result.returncode == 0
    assert result.stdout.startswith("unchecked: the Caddy guard's ping URL")
    assert "--ask-all caddy-upgrades" in result.stdout
    assert box.calls() == []


def test_every_secret_held_gets_a_line(box):
    box.put("staging-screening.env", f"FILEDRAWER_ANTHROPIC_API_KEY='{KEY}'\n")
    box.put("production-screening.env", f"FILEDRAWER_ANTHROPIC_API_KEY='{KEY}'\n")
    box.put("github.env", f"FILEDRAWER_GITHUB_TOKEN='{TOKEN}'\n")
    box.put("production-ping.env", f"FILEDRAWER_DAILY_EMAIL_PING={DAILY}\n")
    box.put("production-backup.env", f"BACKUP_PING={DAILY}\n")
    box.put("caddy-upgrades.env", f"ALERT_TO=a@b.dev\nPING={GUARD}\n")
    box.answer("api.anthropic.com", 200, "{}", "")
    box.answer("api.github.com", 200, "{}", "x-ratelimit-limit: 5000\r\n")
    box.answer("hc-ping.com", 200, "OK", "")
    result = box.run()
    assert result.returncode == 0, result.stdout + result.stderr
    assert len(result.stdout.splitlines()) == 6
    assert all(
        line.startswith(("ok: ", "accepted: ")) for line in result.stdout.splitlines()
    )


def test_a_server_that_cant_get_out_says_nothing_answered(box):
    box.put("github.env", f"FILEDRAWER_GITHUB_TOKEN='{TOKEN}'\n")
    box.put("caddy-upgrades.env", f"ALERT_TO=a@b.dev\nPING={GUARD}\n")
    result = box.run()
    assert result.returncode == 0
    assert result.stdout.splitlines() == [
        "unchecked: the GitHub token: GitHub didn't answer",
        "unchecked: the Caddy guard's ping URL: healthchecks.io didn't answer",
    ]
