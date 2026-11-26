"""Tests for the server's reboot check: ops/remote/filedrawer-reboot-check, run
against a temp root with fake_caddy_world.py standing in for systemctl, loginctl,
caddy and curl, so nothing reboots; and ops/remote/reboots.sh, which installs it,
against a temp root.
"""

import fcntl
import json
import shutil
import stat
import subprocess
from datetime import UTC, datetime
from pathlib import Path
from types import SimpleNamespace

import pytest

REPO = Path(__file__).resolve().parents[2]
CHECK = REPO / "ops/remote/filedrawer-reboot-check"
INSTALL = REPO / "ops/remote/reboots.sh"
ALERTS = REPO / "ops/remote/filedrawer-alerts.sh"
FAKE = Path(__file__).with_name("fake_caddy_world.py")
TOOLS = ["caddy", "systemctl", "systemd-run", "loginctl", "curl"]
KEY = "ab" * 32
SMTP_PASSWORD = 'pa"ss\\word'
PING = "https://hc-ping.com/aaaaaaaa-0000-4000-8000-000000000003"
HOUR = 3600
DAY = 86400
RUNNING = "6.12.48+deb13-amd64"
KERNEL = "linux-image-6.12.57+deb13-amd64"
SERVICES = [
    "caddy.service",
    "filedrawer-production.service",
    "filedrawer-staging.service",
    "filedrawer-loopback.service",
]


def at(text: str) -> int:
    """A UTC time, such as '2026-11-02 06:31', in seconds."""
    return int(datetime.fromisoformat(text).replace(tzinfo=UTC).timestamp())


# A kernel unattended-upgrades installed on a Monday morning, and the first 05:00
# UTC at least 4 days after it was noticed: Saturday's.
INSTALLED = at("2026-11-02 06:31")
DUE = at("2026-11-07 05:00")


def executable(path: Path, text: str) -> Path:
    path.write_text(text)
    path.chmod(path.stat().st_mode | stat.S_IXUSR)
    return path


@pytest.fixture
def server(tmp_path):
    """A server with both sites answering, mail and the guard's ping set up, every
    service active, nobody logged in, and no reboot pending."""
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
    (root / "usr/local/lib").mkdir(parents=True)
    shutil.copy(ALERTS, root / "usr/local/lib/filedrawer-alerts.sh")
    (root / "run").mkdir()
    boot = root / "proc/sys/kernel/random/boot_id"
    boot.parent.mkdir(parents=True)
    boot.write_text("boot-1\n")
    (root / "proc/sys/kernel/osrelease").write_text(RUNNING + "\n")
    (root / "var/lib/dpkg").mkdir(parents=True)
    (root / "var/lib/dpkg/lock-frontend").touch()
    deploy_lock = root / "var/lib/filedrawer-deploy/lock"
    deploy_lock.parent.mkdir(parents=True)
    deploy_lock.touch()
    (root / "srv/filedrawer/production/data/backups").mkdir(parents=True)
    state_dir = root / "var/lib/filedrawer/reboot"

    world_dir = tmp_path / "world"
    world_dir.mkdir()
    fakes = tmp_path / "fakes"
    fakes.mkdir()
    for tool in TOOLS:
        executable(
            fakes / tool,
            f'#!/bin/sh\nexec python3 -I -S {FAKE} {world_dir} {tool} "$@"\n',
        )
    state = SimpleNamespace(
        root=root,
        state=state_dir,
        world={
            "installed": "2.11.9",
            "config_ok": ["2.11.9"],
            "sites_ok": ["2.11.9"],
            "backends": {"production": True, "staging": True},
            "key": KEY,
            "units": dict.fromkeys(SERVICES, "active"),
            "sessions": [],
            "reboots": 0,
        },
    )

    def run(*args: str, now: int) -> subprocess.CompletedProcess:
        (world_dir / "world.json").write_text(json.dumps(state.world))
        (world_dir / "clock").write_text(f"{now}\n")
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
                "FILEDRAWER_PUTOFF_TRIES": "1",
                "FILEDRAWER_PUTOFF_PAUSE": "0",
                "FILEDRAWER_REBOOT_WAIT": "0",
                # Read at each look at the clock, after FILEDRAWER_NOW at the start.
                "FILEDRAWER_CLOCK": str(world_dir / "clock"),
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
        """Each ping's URL suffix after PING, and its body. The reboot check shares
        the Caddy guard's."""
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

    def kernel(*packages: str) -> None:
        """unattended-upgrades' kernel hook, as Debian's runs it."""
        (root / "run/reboot-required").touch()
        with (root / "run/reboot-required.pkgs").open("a") as pkgs:
            for package in packages:
                pkgs.write(package + "\n")

    def boot(release: str = KERNEL.removeprefix("linux-image-")) -> None:
        """A reboot into RELEASE: /run is a tmpfs, and the boot id changes."""
        for name in ("reboot-required", "reboot-required.pkgs"):
            (root / "run" / name).unlink(missing_ok=True)
        boot_file = root / "proc/sys/kernel/random/boot_id"
        boot_file.write_text(boot_file.read_text().strip() + "x\n")
        (root / "proc/sys/kernel/osrelease").write_text(release + "\n")
        state.world["reboots"] = 0

    state.run = run
    state.pings = pings
    state.kernel = kernel
    state.boot = boot
    state.deploy_lock = deploy_lock
    return state


def subject(mail: str) -> str:
    [line] = [h for h in mail.splitlines() if h.startswith("Subject: ")]
    return line.removeprefix("Subject: ")


def noticed(server) -> None:
    """The kernel arrives, and the run after its dpkg run mails the notice."""
    server.kernel(KERNEL)
    result = server.run(now=INSTALLED)
    assert result.returncode == 0, result.stderr
    assert len(server.mails) == 1


def no_secret_leaks(server, result) -> None:
    secrets = [KEY, SMTP_PASSWORD, PING]
    for call in server.calls:
        for secret in secrets:
            assert all(secret not in arg for arg in call["args"]), call
    for text in [*server.mails, result.stdout, result.stderr]:
        for secret in secrets:
            assert secret not in text


# Noticing a pending reboot


def test_nothing_pending_mails_and_pings_nothing(server):
    result = server.run("--daily", now=at("2026-11-02 05:00"))
    assert result.returncode == 0, result.stderr
    assert server.mails == [] and server.pings() == []
    assert server.world["reboots"] == 0


def test_the_first_sighting_mails_the_packages_and_the_reboots_time(server):
    server.kernel(KERNEL)
    result = server.run(now=INSTALLED)
    assert result.returncode == 0, result.stderr
    [mail] = server.mails
    assert "2026-11-07 05:00 UTC" in subject(mail)
    assert KERNEL in mail
    assert "systemctl reboot" in mail
    assert "libc" in mail
    assert (server.state / "announced").read_text().strip() == str(INSTALLED)
    assert server.world["reboots"] == 0
    no_secret_leaks(server, result)


def test_a_second_run_the_same_day_mails_nothing(server):
    noticed(server)
    result = server.run("--daily", now=INSTALLED + 3 * HOUR)
    assert result.returncode == 0, result.stderr
    assert server.mails == []


def test_a_sighting_by_the_0500_run_counts_from_0500(server):
    server.kernel(KERNEL)
    server.run("--daily", now=at("2026-11-03 05:00") + 4)
    [mail] = server.mails
    assert "2026-11-07 05:00 UTC" in subject(mail)


def test_a_second_kernel_mails_once_and_keeps_the_date(server):
    noticed(server)
    server.kernel("linux-image-6.12.58+deb13-amd64")
    server.run(now=INSTALLED + 3 * DAY)
    [mail] = server.mails
    assert "linux-image-6.12.58+deb13-amd64" in subject(mail)
    assert "2026-11-07 05:00 UTC" in mail
    server.run(now=INSTALLED + 3 * DAY + HOUR)
    assert server.mails == []
    server.run("--daily", now=DUE)
    assert server.world["reboots"] == 1


def test_a_pending_file_naming_no_package_still_gets_a_notice(server):
    (server.root / "run/reboot-required").touch()
    server.run(now=INSTALLED)
    [mail] = server.mails
    assert "no package named" in mail


# When it reboots


def test_three_days_and_23_hours_in_at_0500_it_doesnt_reboot(server):
    server.kernel(KERNEL)
    server.run(now=at("2026-11-02 06:00"))
    result = server.run("--daily", now=at("2026-11-06 05:00"))
    assert result.returncode == 0, result.stderr
    assert server.world["reboots"] == 0


def test_the_day_before_its_due_at_0500_it_doesnt_reboot(server):
    noticed(server)
    server.run("--daily", now=DUE - DAY)
    assert server.world["reboots"] == 0


@pytest.mark.parametrize("hour", ["04:59", "05:30", "08:00", "23:00"])
def test_once_due_it_reboots_only_between_0500_and_0530(server, hour):
    noticed(server)
    server.run("--daily", now=at(f"2026-11-08 {hour}"))
    assert server.world["reboots"] == 0


def test_once_due_at_0500_it_reboots_once_and_leaves_a_marker(server):
    noticed(server)
    result = server.run("--daily", now=DUE + 20)
    # The shutdown stopped it.
    assert result.returncode == -15, result.stderr
    assert server.world["reboots"] == 1
    assert (server.state / "rebooted").read_text().split()[1] == "boot-1"
    # The reboot it made goes unmailed: the notice said when.
    assert server.mails == []
    # A second start in the window, before the reboot takes, doesn't reboot again.
    server.world["reboots"] = 0
    server.run("--daily", now=DUE + 60)
    assert server.world["reboots"] == 0


def test_a_run_whose_checks_end_past_0530_doesnt_reboot(server):
    """The window holds at the reboot, not only when the run started: a run that
    waited on the state lock, or on the checks, past 05:30 reboots nothing."""
    noticed(server)
    server.world["clock_after_sessions"] = DUE + 1800
    result = server.run("--daily", now=DUE + 1500)
    assert server.world["reboots"] == 0
    assert result.returncode == 1
    assert [suffix for suffix, _ in server.pings()] == ["/fail"]


def test_a_reboot_that_fails_leaves_no_marker_and_pings_fail(server):
    noticed(server)
    server.world["reboot_fails"] = True
    result = server.run("--daily", now=DUE)
    assert result.returncode == 1
    assert not (server.state / "rebooted").exists()
    assert "systemctl reboot failed" in server.pings()[0][1]


def test_a_reboot_that_never_comes_leaves_no_marker_and_pings_fail(server):
    noticed(server)
    server.world["reboot_ignored"] = True
    result = server.run("--daily", now=DUE)
    assert result.returncode == 1
    assert not (server.state / "rebooted").exists()
    assert "hadn't rebooted" in server.pings()[0][1]


def test_the_runs_at_boot_and_after_dpkg_never_reboot(server):
    noticed(server)
    server.run(now=DUE)
    assert server.world["reboots"] == 0


# After the reboot


def test_the_run_after_boot_mails_that_the_server_is_back_and_clears_the_state(
    server,
):
    noticed(server)
    server.run("--daily", now=DUE)
    server.boot()
    result = server.run(now=DUE + 120)
    assert result.returncode == 0, result.stderr
    [mail] = server.mails
    assert "back" in subject(mail)
    for unit in SERVICES:
        assert f"{unit}: active" in mail
    assert "Checked and passing: production staging." in mail
    assert sorted(p.name for p in server.state.iterdir()) == ["lock"]
    # The next run mails nothing.
    server.run("--daily", now=DUE + DAY)
    assert server.mails == [] and server.world["reboots"] == 0


def test_the_back_mail_names_a_service_that_didnt_come_back(server):
    noticed(server)
    server.run("--daily", now=DUE)
    server.boot()
    server.world["units"]["filedrawer-production.service"] = "failed"
    server.world["backends"]["production"] = False
    result = server.run(now=DUE + 120)
    assert result.returncode == 1
    [mail] = server.mails
    assert "filedrawer-production.service: failed" in mail
    assert [suffix for suffix, _ in server.pings()] == ["/fail"]


def test_a_boot_into_an_older_kernel_is_mailed_and_starts_no_second_reboot(server):
    noticed(server)
    server.run("--daily", now=DUE)
    # GRUB's default was pinned, so the old kernel came back.
    server.boot(RUNNING)
    result = server.run(now=DUE + 120)
    assert result.returncode == 1
    [mail] = server.mails
    assert RUNNING in mail and KERNEL.removeprefix("linux-image-") in mail
    assert [suffix for suffix, _ in server.pings()] == ["/fail"]
    server.run("--daily", now=DUE + DAY)
    assert server.world["reboots"] == 0


def test_a_back_mail_that_fails_is_tried_again_soon(server):
    noticed(server)
    server.run("--daily", now=DUE)
    server.boot()
    server.world["mail_fails"] = True
    server.run(now=DUE + 120)
    assert any(call["tool"] == "systemd-run" for call in server.calls)
    assert (server.state / "rebooted").exists()
    server.world["mail_fails"] = False
    server.run(now=DUE + 720)
    [mail] = server.mails
    assert "back" in subject(mail)
    assert not (server.state / "rebooted").exists()


def test_a_reboot_still_pending_after_its_reboot_starts_no_second_one(server):
    noticed(server)
    server.run("--daily", now=DUE)
    server.boot()
    # Something wrote the file again at boot.
    server.kernel(KERNEL)
    result = server.run(now=DUE + 120)
    assert result.returncode == 1
    [mail] = server.mails
    assert "still pending" in subject(mail)
    assert "/var/lib/filedrawer/reboot/stuck" in mail
    for days in (1, 5, 9):
        result = server.run("--daily", now=DUE + days * DAY)
        assert server.world["reboots"] == 0
        assert server.mails == []
        assert [suffix for suffix, _ in server.pings()] == ["/fail"]


def test_clearing_stuck_mails_a_fresh_notice_before_any_reboot(server):
    noticed(server)
    server.run("--daily", now=DUE)
    server.boot()
    server.kernel(KERNEL)
    server.run(now=DUE + 120)
    (server.state / "stuck").unlink()
    server.run("--daily", now=DUE + DAY)
    assert server.world["reboots"] == 0
    [mail] = server.mails
    assert "reboot is pending" in subject(mail)
    assert "2026-11-12 05:00 UTC" in subject(mail)


def test_a_new_package_after_a_stuck_reboot_starts_over_with_a_notice(server):
    noticed(server)
    server.run("--daily", now=DUE)
    server.boot()
    server.kernel(KERNEL)
    server.run(now=DUE + 120)
    server.kernel("dbus")
    server.run(now=DUE + DAY + 2 * HOUR)
    [mail] = server.mails
    assert "reboot is pending" in subject(mail) and "dbus" in mail
    server.world["reboots"] = 0
    server.run("--daily", now=DUE + 5 * DAY)
    assert server.world["reboots"] == 0
    server.run("--daily", now=DUE + 6 * DAY)
    assert server.world["reboots"] == 1


def test_a_reboot_by_hand_clears_the_state_and_mails_nothing(server):
    noticed(server)
    server.boot()
    server.run("--daily", now=INSTALLED + DAY)
    assert server.mails == []
    assert sorted(p.name for p in server.state.iterdir()) == ["lock"]


# What puts it off


def hold(path: Path):
    """A process that holds an flock on PATH, as filedrawer-receive, the install, the
    nightly copy or the guard does, until released."""
    holder = subprocess.Popen(
        ["flock", str(path), "sh", "-c", "echo held; exec cat"],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        text=True,
    )
    assert holder.stdout.readline() == "held\n"
    return holder


def release(holder) -> None:
    holder.stdin.close()
    holder.wait()


def test_a_held_deploy_lock_puts_the_reboot_off_and_says_so(server):
    noticed(server)
    holder = hold(server.deploy_lock)
    try:
        result = server.run("--daily", now=DUE)
    finally:
        release(holder)
    assert result.returncode == 1
    assert server.world["reboots"] == 0
    [mail] = server.mails
    assert "put off to Sun 2026-11-08 05:00 UTC" in subject(mail)
    assert "filedrawer-deploy/lock" in mail
    assert server.pings()[0][0] == "/fail"
    server.run("--daily", now=DUE + DAY)
    assert server.world["reboots"] == 1


def test_productions_deploy_or_nightly_copy_puts_it_off(server):
    noticed(server)
    holder = hold(server.root / "srv/filedrawer/production/data/backups")
    try:
        server.run("--daily", now=DUE)
    finally:
        release(holder)
    assert server.world["reboots"] == 0
    assert "backups" in server.mails[0]


def test_the_caddy_guards_run_puts_it_off(server):
    guard = server.root / "var/lib/filedrawer/caddy-guard/lock"
    guard.parent.mkdir(parents=True)
    guard.touch()
    noticed(server)
    holder = hold(guard)
    try:
        server.run("--daily", now=DUE)
    finally:
        release(holder)
    assert server.world["reboots"] == 0


def test_a_rollback_the_guard_left_waiting_puts_it_off(server):
    guard = server.root / "var/lib/filedrawer/caddy-guard"
    guard.mkdir(parents=True)
    (guard / "rollback").write_text("2.11.8 2.11.9\n")
    noticed(server)
    server.run("--daily", now=DUE)
    assert server.world["reboots"] == 0
    assert "rollback" in server.mails[0]


def test_a_dpkg_run_puts_it_off(server, tmp_path):
    noticed(server)
    with (server.root / "var/lib/dpkg/lock-frontend").open("w") as lock:
        fcntl.lockf(lock, fcntl.LOCK_EX)
        server.run("--daily", now=DUE)
    assert server.world["reboots"] == 0
    assert "dpkg" in server.mails[0]


def test_apts_daily_upgrade_running_puts_it_off(server):
    noticed(server)
    server.world["units"]["apt-daily-upgrade.service"] = "activating"
    server.run("--daily", now=DUE)
    assert server.world["reboots"] == 0
    assert "apt-daily-upgrade.service" in server.mails[0]


@pytest.mark.parametrize("state", ["active", "online", "opening"])
def test_someone_logged_in_puts_it_off(server, state):
    noticed(server)
    server.world["sessions"] = [
        {
            "id": "4",
            "class": "user",
            "state": state,
            "name": "root",
            "remotehost": "203.0.113.9",
        }
    ]
    server.run("--daily", now=DUE)
    assert server.world["reboots"] == 0
    assert "root is logged in from 203.0.113.9" in server.mails[0]


@pytest.mark.parametrize(
    ("cls", "state"),
    [("user", "closing"), ("manager", "active"), ("background", "active")],
)
def test_a_closing_session_or_a_user_manager_doesnt_put_it_off(server, cls, state):
    noticed(server)
    server.world["sessions"] = [
        {"id": "4", "class": cls, "state": state, "name": "root", "remotehost": ""}
    ]
    server.run("--daily", now=DUE)
    assert server.world["reboots"] == 1


def test_logind_not_answering_puts_it_off(server):
    noticed(server)
    server.world["logind_down"] = True
    server.run("--daily", now=DUE)
    assert server.world["reboots"] == 0
    assert "loginctl" in server.mails[0]


# Without mail


def test_without_mail_settings_it_never_reboots_and_pings_fail_daily(server):
    (server.root / "etc/filedrawer/production-mail.env").unlink()
    server.kernel(KERNEL)
    result = server.run(now=INSTALLED)
    assert result.returncode == 1
    for days in range(1, 8):
        result = server.run("--daily", now=at("2026-11-03 05:00") + days * DAY)
        assert server.world["reboots"] == 0
        [(suffix, body)] = server.pings()
        assert suffix == "/fail" and "won't reboot itself" in body


def test_a_notice_mailed_late_starts_the_clock_then(server):
    server.world["mail_fails"] = True
    server.kernel(KERNEL)
    server.run(now=INSTALLED)
    server.world["mail_fails"] = False
    server.run("--daily", now=at("2026-11-04 05:00"))
    [mail] = server.mails
    assert "2026-11-08 05:00 UTC" in subject(mail)
    server.run("--daily", now=DUE)
    assert server.world["reboots"] == 0


# The dry run


def test_the_dry_run_mails_a_test_and_changes_nothing(server):
    noticed(server)
    before = sorted((p.name, p.read_text()) for p in server.state.iterdir())
    result = server.run("--dry-run", now=DUE)
    assert result.returncode == 0, result.stderr
    assert server.world["reboots"] == 0
    [mail] = server.mails
    assert "a test" in subject(mail)
    assert "2026-11-07 05:00 UTC" in result.stdout
    assert "inside the reboot window" in result.stdout
    assert "Nothing would put a reboot off now." in result.stdout
    assert sorted((p.name, p.read_text()) for p in server.state.iterdir()) == before
    assert server.pings() == []


def test_the_dry_run_says_what_would_put_it_off(server):
    holder = hold(server.deploy_lock)
    try:
        result = server.run("--dry-run", now=INSTALLED)
    finally:
        release(holder)
    assert "filedrawer-deploy/lock" in result.stdout
    assert "No reboot is pending." in result.stdout


def test_the_dry_run_without_mail_fails(server):
    server.world["mail_fails"] = True
    result = server.run("--dry-run", now=INSTALLED)
    assert result.returncode == 1


# The installer


@pytest.fixture
def install(tmp_path):
    root = tmp_path / "root"
    (root / "etc/filedrawer").mkdir(parents=True)
    (root / "etc/filedrawer/caddy-upgrades.env").write_text(
        f"ALERT_TO=ops@filedrawer.dev\nPING={PING}\n"
    )
    fakes = tmp_path / "fakes"
    fakes.mkdir()
    log = tmp_path / "log"
    pam = tmp_path / "pam-installed"
    for tool in ("systemctl", "apt-get"):
        executable(fakes / tool, f'#!/bin/sh\necho "{tool} $*" >> {log}\n')
    # libpam-systemd's status, as dpkg-query prints it.
    executable(
        fakes / "dpkg-query",
        f'#!/bin/sh\n[ -e {pam} ] && printf "install ok installed"\nexit 0\n',
    )

    def run() -> subprocess.CompletedProcess:
        return subprocess.run(
            ["bash", str(INSTALL)],
            env={"PATH": f"{fakes}:/usr/bin:/bin", "FILEDRAWER_ROOT": str(root)},
            stdin=subprocess.DEVNULL,
            capture_output=True,
            text=True,
            check=False,
        )

    def files() -> dict[str, str]:
        return {
            str(p.relative_to(root)): p.read_text()
            for p in sorted(root.rglob("*"))
            if p.is_file()
        }

    return SimpleNamespace(
        root=root,
        run=run,
        files=files,
        pam=pam,
        log=lambda: log.read_text() if log.exists() else "",
    )


def test_the_installer_writes_the_units_the_timer_and_the_apt_hook(install):
    result = install.run()
    assert result.returncode == 0, result.stderr
    units = install.root / "etc/systemd/system"
    notice = (units / "filedrawer-reboot-check.service").read_text()
    daily = (units / "filedrawer-reboot-check-daily.service").read_text()
    timer = (units / "filedrawer-reboot-check-daily.timer").read_text()
    assert "ExecStart=/usr/local/sbin/filedrawer-reboot-check\n" in notice
    assert "WantedBy=multi-user.target" in notice
    assert "ExecStart=/usr/local/sbin/filedrawer-reboot-check --daily\n" in daily
    assert "OnCalendar=*-*-* 05:00:00 UTC\n" in timer
    assert "Persistent=true" not in timer
    apt = (install.root / "etc/apt/apt.conf.d/53filedrawer-reboot").read_text()
    assert "systemctl start --no-block filedrawer-reboot-check.service" in apt
    assert "|| true" in apt
    log = install.log()
    assert "systemctl enable filedrawer-reboot-check.service" in log
    assert "systemctl enable --now filedrawer-reboot-check-daily.timer" in log
    assert "systemctl start filedrawer-reboot-check.service" in log


def test_the_installer_installs_libpam_systemd_only_when_missing(install):
    """Without it sshd registers no session, and the logged-in check passes."""
    install.run()
    assert "apt-get -y -q install libpam-systemd" in install.log()
    install.pam.touch()
    before = install.log().count("apt-get")
    install.run()
    assert install.log().count("apt-get") == before


def test_a_rerun_of_the_installer_changes_nothing(install):
    install.run()
    before = install.files()
    result = install.run()
    assert result.returncode == 0, result.stderr
    assert install.files() == before


def test_the_installer_refuses_without_the_guards_settings(install):
    (install.root / "etc/filedrawer/caddy-upgrades.env").unlink()
    result = install.run()
    assert result.returncode == 1
    assert "ops/provision-vps caddy-upgrades" in result.stderr
    assert not (install.root / "etc/systemd/system").exists()
