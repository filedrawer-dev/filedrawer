"""Tests for the guard on the server's Caddy (#100): ops/remote/caddy-upgrade-check,
run against a temp root with fake_caddy_world.py standing in for caddy, apt, dpkg,
systemctl and curl; and ops/remote/caddy-upgrades.sh, which installs it, against a
temp root.
"""

import json
import os
import stat
import subprocess
from pathlib import Path
from types import SimpleNamespace

import pytest

REPO = Path(__file__).resolve().parents[2]
CHECK = REPO / "ops/remote/caddy-upgrade-check"
INSTALL = REPO / "ops/remote/caddy-upgrades.sh"
FAKE = Path(__file__).with_name("fake_caddy_world.py")
TOOLS = [
    "caddy",
    "dpkg-query",
    "apt-get",
    "dpkg",
    "apt-mark",
    "apt-cache",
    "systemctl",
    "systemd-run",
    "curl",
]
KEY = "ab" * 32
SMTP_PASSWORD = 'pa"ss\\word'
PING = "https://hc-ping.com/aaaaaaaa-0000-4000-8000-000000000003"
NOW = 1_800_000_000
DAY = 86400
OLD, NEW = "2.11.9", "2.12.0"
STAMP = "var/lib/apt/periodic/update-stamp"


def executable(path: Path, text: str) -> Path:
    path.write_text(text)
    path.chmod(path.stat().st_mode | stat.S_IXUSR)
    return path


@pytest.fixture
def server(tmp_path):
    """A server whose Caddy OLD passed the guard's last check, both sites set up and
    answering, mail and pings set up, and apt's lists fresh."""
    root = tmp_path / "root"
    (root / "etc/caddy/sites").mkdir(parents=True)
    (root / "etc/caddy/Caddyfile").write_text("import /etc/caddy/sites/*.caddy\n")
    (root / "etc/caddy/sites/production.caddy").write_text("filedrawer.dev {}\n")
    (root / "etc/caddy/sites/staging.caddy").write_text(
        f'\t@agent header X-Staging-Key "{KEY}"\n'
    )
    (root / "etc/filedrawer").mkdir()
    (root / "etc/filedrawer/caddy-upgrades.env").write_text(
        f"ALERT_TO=ops@filedrawer.dev\nPING={PING}\n"
    )
    (root / "etc/filedrawer/production-mail.env").write_text(
        "FILEDRAWER_SMTP_HOST=smtp.purelymail.com\nFILEDRAWER_SMTP_PORT=465\n"
        "FILEDRAWER_SMTP_USER='prod@filedrawer.dev'\n"
        f"FILEDRAWER_SMTP_PASSWORD='{SMTP_PASSWORD}'\n"
        "FILEDRAWER_MAIL_FROM='File Drawer <noreply@filedrawer.dev>'\n"
    )
    guard = root / "var/lib/filedrawer/caddy-guard"
    guard.mkdir(parents=True)
    (guard / "good").write_text(OLD + "\n")
    stamp = root / STAMP
    stamp.parent.mkdir(parents=True)
    stamp.touch()
    os.utime(stamp, (NOW - 3600, NOW - 3600))
    (root / "var/cache/apt/archives").mkdir(parents=True)

    world_dir = tmp_path / "world"
    world_dir.mkdir()
    fakes = tmp_path / "fakes"
    fakes.mkdir()
    for tool in TOOLS:
        executable(
            fakes / tool, f'#!/bin/sh\nexec python3 {FAKE} {world_dir} {tool} "$@"\n'
        )
    state = SimpleNamespace(
        root=root,
        guard=guard,
        world={
            "installed": OLD,
            "held": False,
            "available": [OLD, NEW],
            "config_ok": [OLD, NEW],
            "sites_ok": [OLD, NEW],
            "backends": {"production": True, "staging": True},
            "key": KEY,
        },
    )

    def run(*args: str, now: int = NOW) -> subprocess.CompletedProcess:
        (world_dir / "world.json").write_text(json.dumps(state.world))
        for name in ("log.jsonl", "mails.jsonl"):
            (world_dir / name).write_text("")
        result = subprocess.run(
            [str(CHECK), *args],
            env={
                "PATH": f"{fakes}:/usr/bin:/bin",
                "FILEDRAWER_ROOT": str(root),
                "FILEDRAWER_NOW": str(now),
                "FILEDRAWER_PROBE_TRIES": "1",
                "FILEDRAWER_PROBE_PAUSE": "0",
            },
            stdin=subprocess.DEVNULL,
            capture_output=True,
            text=True,
            check=False,
        )
        state.world = json.loads((world_dir / "world.json").read_text())
        state.calls = [
            json.loads(line)
            for line in (world_dir / "log.jsonl").read_text().splitlines()
        ]
        state.mails = [
            json.loads(line)
            for line in (world_dir / "mails.jsonl").read_text().splitlines()
        ]
        return result

    def pings() -> list[tuple[str, str]]:
        """Each ping's URL suffix after PING, and its body."""
        out = []
        for call in state.calls:
            if call["tool"] == "curl" and PING in call["stdin"]:
                url = call["stdin"].split('url = "', 1)[1].split('"', 1)[0]
                args = call["args"]
                body = (
                    args[args.index("--data-raw") + 1] if "--data-raw" in args else ""
                )
                out.append((url.removeprefix(PING), body))
        return out

    def ran(tool: str, *words: str) -> bool:
        return any(
            call["tool"] == tool and all(w in call["args"] for w in words)
            for call in state.calls
        )

    state.run = run
    state.pings = pings
    state.ran = ran
    state.good = lambda: (guard / "good").read_text().strip()
    return state


def upgraded(server, version: str = NEW) -> None:
    server.world["installed"] = version


def no_secret_leaks(server, result) -> None:
    """No secret on a command line, in a mail, or in what the run printed."""
    secrets = [KEY, SMTP_PASSWORD, PING]
    for call in server.calls:
        for secret in secrets:
            assert all(secret not in arg for arg in call["args"]), call
    for text in [*server.mails, result.stdout, result.stderr]:
        for secret in secrets:
            assert secret not in text


# The hook's run: only a change of version does anything.


def test_an_unchanged_caddy_does_nothing(server):
    result = server.run()
    assert result.returncode == 0, result.stderr
    assert server.ran("dpkg-query")
    assert server.mails == []
    assert not any(call["tool"] == "curl" for call in server.calls)


def test_an_upgrade_that_passes_is_recorded_and_mailed(server):
    upgraded(server)
    result = server.run()
    assert result.returncode == 0, result.stderr
    assert server.good() == NEW
    assert server.ran("systemctl", "restart", "caddy")
    [mail] = server.mails
    assert f"{OLD} -> {NEW}" in mail and "passed" in mail
    assert "Message-ID:" in mail and "Date:" in mail
    assert "bin/caddy" in mail
    # The hook's run never reports success to healthchecks.io: only the daily one does.
    assert server.pings() == []
    no_secret_leaks(server, result)


def test_the_staging_key_reaches_curl_on_stdin(server):
    upgraded(server)
    server.run()
    keyed = [c for c in server.calls if c["tool"] == "curl" and KEY in c["stdin"]]
    assert keyed, "no request through Caddy sent the staging key"


def test_a_config_the_new_caddy_refuses_rolls_back_and_holds(server):
    server.world["config_ok"] = [OLD]
    upgraded(server)
    result = server.run()
    assert result.returncode == 1
    assert server.world["installed"] == OLD
    assert server.world["held"]
    assert (server.guard / "hold").exists()
    assert server.good() == OLD
    [mail] = server.mails
    assert "rolled back" in mail and "held" in mail
    assert "unrecognized directive" in mail
    assert "apt-mark unhold caddy" in mail
    assert [suffix for suffix, _ in server.pings()] == ["/fail"]
    no_secret_leaks(server, result)


def test_sites_that_fail_only_on_the_new_caddy_roll_back(server):
    server.world["sites_ok"] = [OLD]
    upgraded(server)
    result = server.run()
    assert result.returncode == 1
    assert server.world["installed"] == OLD and server.world["held"]
    [mail] = server.mails
    assert "502" in mail


def test_the_rollback_installs_the_old_version_with_conffile_options(server):
    server.world["config_ok"] = [OLD]
    upgraded(server)
    server.run()
    [install] = [c["args"] for c in server.calls if c["tool"] == "apt-get"]
    assert f"caddy={OLD}" in install
    assert "Dpkg::Options::=--force-confold" in install
    assert "--allow-downgrades" in install and "--allow-change-held-packages" in install


def test_without_the_old_version_in_apt_the_rollback_takes_apts_cached_deb(server):
    server.world["config_ok"] = [OLD]
    server.world["available"] = [NEW]
    (server.root / f"var/cache/apt/archives/caddy_{OLD}_amd64.deb").write_text("deb")
    upgraded(server)
    server.run()
    assert server.world["installed"] == OLD
    assert server.ran("dpkg", "--force-confold")


def test_a_rollback_that_cant_install_says_the_site_may_be_down(server):
    server.world["config_ok"] = [OLD]
    server.world["available"] = [NEW]
    upgraded(server)
    result = server.run()
    assert result.returncode == 1
    assert server.world["installed"] == NEW
    [mail] = server.mails
    assert "may be down" in mail.split("\r\n\r\n")[0]
    assert server.pings()[-1][0] == "/fail"


def test_a_busy_dpkg_lock_retries_the_rollback_later(server):
    server.world["config_ok"] = [OLD]
    server.world["lock_busy"] = True
    upgraded(server)
    server.run()
    assert server.world["installed"] == NEW
    assert server.ran("systemd-run")
    assert (server.guard / "rollback").exists()
    assert server.mails == []
    # The next run, with the lock free, finishes it.
    server.world["lock_busy"] = False
    server.run()
    assert server.world["installed"] == OLD and server.world["held"]
    assert not (server.guard / "rollback").exists()
    [mail] = server.mails
    assert "rolled back" in mail


def test_a_busy_dpkg_lock_isnt_reported_as_all_well(server):
    server.world["config_ok"] = [OLD]
    server.world["lock_busy"] = True
    upgraded(server)
    result = server.run("--daily")
    assert result.returncode == 1
    assert [suffix for suffix, _ in server.pings()] == ["/fail"]


def test_a_new_version_that_cant_go_back_on_isnt_said_to_be_back(server):
    server.world["sites_ok"] = []
    upgraded(server)
    # apt can install OLD for the rollback, but no longer NEW.
    server.world["available"] = [OLD]
    result = server.run()
    assert result.returncode == 1
    assert server.world["installed"] == OLD
    assert not server.world["held"]
    assert not (server.guard / "failing").exists()
    [mail] = server.mails
    assert "is back" not in mail and "couldn't" in mail


def test_a_cleared_hold_leaves_no_guard_marker(server):
    server.world["config_ok"] = [OLD]
    upgraded(server)
    server.run()
    assert (server.guard / "hold").exists()
    # The owner fixes the config and unholds; the new version installs and passes.
    server.world["config_ok"] = [OLD, NEW]
    server.world["held"] = False
    upgraded(server)
    server.run()
    assert server.good() == NEW
    assert not (server.guard / "hold").exists()


def test_a_downgrade_is_probed_after_a_restart(server):
    server.world["installed"] = "2.11.8"
    server.world["config_ok"].append("2.11.8")
    server.run()
    assert server.ran("systemctl", "restart", "caddy")


def test_a_failing_version_is_mailed_once_not_on_every_dpkg_run(server):
    server.world["sites_ok"] = []
    upgraded(server)
    server.run()
    assert len(server.mails) == 1
    result = server.run()
    assert result.returncode == 1
    assert server.mails == []


def test_a_daily_check_that_fails_mails_once_then_when_it_passes_again(server):
    server.world["sites_ok"] = []
    server.run("--daily")
    assert len(server.mails) == 1
    server.run("--daily", now=NOW + DAY)
    assert server.mails == []
    assert server.pings()[-1][0] == "/fail"
    server.world["sites_ok"] = [OLD]
    server.run("--daily", now=NOW + 2 * DAY)
    [mail] = server.mails
    assert "passes" in mail
    assert server.pings()[-1][0] == ""


def test_a_daily_run_that_rolls_back_sends_one_mail(server):
    server.world["config_ok"] = [OLD]
    upgraded(server)
    server.run("--daily")
    assert len(server.mails) == 1


def test_when_both_versions_fail_the_new_one_goes_back_unheld(server):
    server.world["sites_ok"] = []
    upgraded(server)
    result = server.run()
    assert result.returncode == 1
    assert server.world["installed"] == NEW
    assert not server.world["held"]
    assert not (server.guard / "hold").exists()
    assert (server.guard / "failing").read_text().strip() == NEW
    assert server.good() == OLD
    [mail] = server.mails
    assert "both" in mail


def test_a_failing_version_isnt_rolled_back_again_and_recovers_when_checks_pass(server):
    server.world["sites_ok"] = []
    upgraded(server)
    server.run()
    result = server.run("--daily")
    assert result.returncode == 1
    assert not server.ran("apt-get")
    assert server.pings()[-1][0] == "/fail"
    server.world["sites_ok"] = [NEW]
    result = server.run("--daily")
    assert result.returncode == 0, result.stderr
    assert server.good() == NEW
    assert not (server.guard / "failing").exists()
    assert server.pings()[-1][0] == ""


def test_a_backend_that_is_down_cant_be_judged_and_never_rolls_back(server):
    server.world["backends"]["production"] = False
    upgraded(server)
    result = server.run()
    assert result.returncode == 0, result.stderr
    assert server.world["installed"] == NEW and not server.world["held"]
    assert server.good() == NEW
    [mail] = server.mails
    assert "production" in mail and "can't be judged" in mail


def test_a_staging_key_it_cant_find_leaves_staging_unjudged(server):
    (server.root / "etc/caddy/sites/staging.caddy").write_text(
        "staging.filedrawer.dev {}\n"
    )
    server.world["sites_ok"] = [OLD]
    server.world["backends"]["production"] = False
    upgraded(server)
    server.run()
    assert server.world["installed"] == NEW
    [mail] = server.mails
    assert "staging" in mail and "can't be judged" in mail


def test_with_no_known_good_version_a_passing_check_records_one(server):
    (server.guard / "good").unlink()
    result = server.run()
    assert result.returncode == 0, result.stderr
    assert server.good() == OLD
    [mail] = server.mails
    assert OLD in mail


def test_with_no_known_good_version_a_failing_check_records_none_and_rolls_back_nothing(
    server,
):
    (server.guard / "good").unlink()
    server.world["sites_ok"] = []
    result = server.run()
    assert result.returncode == 1
    assert not (server.guard / "good").exists()
    assert not server.ran("apt-get")


def test_a_downgrade_is_checked_never_fought(server):
    server.world["installed"] = "2.11.8"
    server.world["sites_ok"] = []
    result = server.run()
    assert result.returncode == 1
    assert server.world["installed"] == "2.11.8"
    assert not server.ran("apt-get")
    [mail] = server.mails
    assert "2.11.8" in mail


def test_mail_that_cant_be_sent_pings_fail(server):
    server.world["mail_fails"] = True
    upgraded(server)
    result = server.run()
    assert result.returncode == 1
    assert server.pings()[-1][0] == "/fail"


def test_the_smtp_password_reaches_curl_escaped_on_stdin(server):
    upgraded(server)
    server.run()
    [smtp] = [
        c for c in server.calls if c["tool"] == "curl" and "--mail-rcpt" in c["args"]
    ]
    assert 'user = "prod@filedrawer.dev:pa\\"ss\\\\word"' in smtp["stdin"]


def test_staging_mail_settings_serve_when_production_has_none(server):
    env = server.root / "etc/filedrawer"
    (env / "production-mail.env").rename(env / "staging-mail.env")
    upgraded(server)
    server.run()
    assert len(server.mails) == 1


# The daily run.


def test_a_daily_run_with_all_well_pings_success(server):
    result = server.run("--daily")
    assert result.returncode == 0, result.stderr
    assert server.pings() == [("", f"Caddy {OLD}")]
    assert server.mails == []


def test_a_daily_run_whose_check_fails_pings_fail_and_mails(server):
    server.world["sites_ok"] = []
    result = server.run("--daily")
    assert result.returncode == 1
    [(suffix, body)] = server.pings()
    assert suffix == "/fail" and "502" in body
    assert len(server.mails) == 1
    no_secret_leaks(server, result)


def test_a_daily_run_doesnt_restart_caddy_when_nothing_changed(server):
    server.run("--daily")
    assert server.ran("caddy", "validate")
    assert not server.ran("systemctl", "restart", "caddy")


@pytest.mark.parametrize(
    ("marker", "says"), [(True, "rolled back"), (False, "by hand")]
)
def test_a_hold_pings_fail_daily_and_mails_weekly(server, marker, says):
    server.world["held"] = True
    if marker:
        (server.guard / "hold").write_text(f"{NEW}\n")
    result = server.run("--daily")
    assert result.returncode == 1
    assert server.pings()[-1][0] == "/fail"
    [mail] = server.mails
    assert "held" in mail and says in mail
    server.run("--daily", now=NOW + DAY)
    assert server.pings()[-1][0] == "/fail"
    assert server.mails == []
    server.run("--daily", now=NOW + 8 * DAY)
    assert len(server.mails) == 1


def test_a_newer_candidate_waiting_three_days_is_stuck(server):
    server.world["candidate"] = NEW
    server.run("--daily")
    assert server.pings()[-1][0] == ""
    server.run("--daily", now=NOW + 2 * DAY)
    assert server.pings()[-1][0] == ""
    result = server.run("--daily", now=NOW + 3 * DAY + 60)
    assert result.returncode == 1
    suffix, body = server.pings()[-1]
    assert suffix == "/fail" and NEW in body


def test_apt_lists_not_updated_for_three_days_are_stuck(server):
    os.utime(server.root / STAMP, (NOW - 4 * DAY, NOW - 4 * DAY))
    result = server.run("--daily")
    assert result.returncode == 1
    assert server.pings()[-1][0] == "/fail"


def test_apt_lists_never_updated_are_stuck(server):
    (server.root / STAMP).unlink()
    result = server.run("--daily")
    assert result.returncode == 1
    suffix, body = server.pings()[-1]
    assert suffix == "/fail" and "never succeeded" in body


def test_no_ping_url_mails_weekly(server):
    (server.root / "etc/filedrawer/caddy-upgrades.env").write_text(
        "ALERT_TO=ops@filedrawer.dev\n"
    )
    result = server.run("--daily")
    assert result.returncode == 1
    assert server.pings() == []
    [mail] = server.mails
    assert "ping" in mail


def test_test_mail_sends_one_mail_and_one_ping(server):
    result = server.run("--test-mail")
    assert result.returncode == 0, result.stderr
    assert len(server.mails) == 1
    assert len(server.pings()) == 1


# The installer.


@pytest.fixture
def install(tmp_path):
    root = tmp_path / "root"
    root.mkdir()
    fakes = tmp_path / "fakes"
    fakes.mkdir()
    log = tmp_path / "log"
    for tool in ("systemctl", "apt-mark", "chown"):
        executable(fakes / tool, f'#!/bin/sh\necho "{tool} $*" >> {log}\n')

    def run(settings: str | None) -> subprocess.CompletedProcess:
        env = {"PATH": f"{fakes}:/usr/bin:/bin", "FILEDRAWER_ROOT": str(root)}
        if settings is not None:
            env["SETTINGS"] = settings
        return subprocess.run(
            ["bash", str(INSTALL)],
            env=env,
            stdin=subprocess.DEVNULL,
            capture_output=True,
            text=True,
            check=False,
        )

    return SimpleNamespace(
        root=root,
        run=run,
        log=lambda: log.read_text() if log.exists() else "",
    )


def test_the_installer_has_apt_follow_caddy_and_start_the_guard_after_dpkg(install):
    result = install.run(f"ops@filedrawer.dev\t{PING}")
    assert result.returncode == 0, result.stderr
    apt = (install.root / "etc/apt/apt.conf.d/52filedrawer-caddy").read_text()
    assert '"origin=cloudsmith/caddy/stable";' in apt
    assert '"--force-confold";' in apt and '"--force-confdef";' in apt
    assert "systemctl start --no-block caddy-upgrade-check.service" in apt
    assert "|| true" in apt
    assert 'APT::Periodic::Unattended-Upgrade "1";' in apt


def test_the_installer_writes_the_units_and_enables_the_timer(install):
    install.run(f"ops@filedrawer.dev\t{PING}")
    units = install.root / "etc/systemd/system"
    hook = (units / "caddy-upgrade-check.service").read_text()
    daily = (units / "caddy-upgrade-daily.service").read_text()
    assert "ExecStart=/usr/local/sbin/caddy-upgrade-check\n" in hook
    assert "ExecStart=/usr/local/sbin/caddy-upgrade-check --daily\n" in daily
    for unit in (hook, daily):
        assert "TimeoutStartSec=" in unit and "After=" in unit
    assert "Persistent=true" in (units / "caddy-upgrade-daily.timer").read_text()
    assert "systemctl enable --now caddy-upgrade-daily.timer" in install.log()


def test_the_installer_keeps_the_settings_private(install):
    install.run(f"ops@filedrawer.dev\t{PING}")
    env = install.root / "etc/filedrawer/caddy-upgrades.env"
    assert env.read_text() == f"ALERT_TO=ops@filedrawer.dev\nPING={PING}\n"
    assert stat.S_IMODE(env.stat().st_mode) == 0o600


def test_the_installer_without_settings_keeps_the_ones_there(install):
    install.run(f"ops@filedrawer.dev\t{PING}")
    result = install.run("")
    assert result.returncode == 0, result.stderr
    env = install.root / "etc/filedrawer/caddy-upgrades.env"
    assert PING in env.read_text()


@pytest.mark.parametrize(
    "settings",
    [
        f"ops@filedrawer.dev\thttps://example.com/{PING}",
        f"not an address\t{PING}",
        f"a@b@c\t{PING}",
        "ops@filedrawer.dev",
    ],
)
def test_the_installer_refuses_settings_it_cant_use(install, settings):
    result = install.run(settings)
    assert result.returncode != 0
    assert not (install.root / "etc/filedrawer/caddy-upgrades.env").exists()
    assert PING not in result.stdout + result.stderr


def test_the_installer_leaves_a_hold_alone(install):
    install.run(f"ops@filedrawer.dev\t{PING}")
    assert "systemctl" in install.log()
    assert "apt-mark" not in install.log()
