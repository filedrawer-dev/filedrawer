"""Tests for production's nightly copy of its database (#41): ops/remote/filedrawer-backup,
which the timer runs, against a real SQLite database with fakes of curl, date and df
first on PATH; and ops/remote/backup.sh, which installs its unit and timer, against
a temp root.
"""

import fcntl
import os
import sqlite3
import stat
import subprocess
import time
from pathlib import Path
from types import SimpleNamespace

import pytest

REPO = Path(__file__).resolve().parents[2]
BACKUP = REPO / "ops/remote/filedrawer-backup"
INSTALL = REPO / "ops/remote/backup.sh"
PING = "https://hc-ping.com/aaaaaaaa-0000-4000-8000-000000000002"
TODAY = "2026-11-10"


def executable(path: Path, text: str) -> Path:
    path.write_text(text)
    path.chmod(path.stat().st_mode | stat.S_IXUSR)
    return path


def make_database(path: Path, entries: int = 3) -> None:
    with sqlite3.connect(path) as db:
        db.execute("PRAGMA journal_mode=WAL")
        db.execute("CREATE TABLE entries (id TEXT PRIMARY KEY, body TEXT)")
        db.executemany(
            "INSERT INTO entries VALUES (?, ?)",
            [(f"e{n}", f"entry {n}") for n in range(entries)],
        )
    db.close()


@pytest.fixture
def job(tmp_path):
    data = tmp_path / "data"
    data.mkdir()
    database = data / "filedrawer.db"
    make_database(database)
    fakes = tmp_path / "fakes"
    fakes.mkdir()
    pings = tmp_path / "pings"
    pings.mkdir()
    # curl notes its arguments and its stdin, one pair of files a call, and exits
    # with the status in curl-status, if any.
    executable(
        fakes / "curl",
        f"""#!/bin/sh
n=$(ls {pings} | wc -l)
printf '%s\\n' "$@" > {pings}/$n.args
cat > {pings}/$n.stdin
[ -f {tmp_path}/curl-status ] && exit "$(cat {tmp_path}/curl-status)"
exit 0
""",
    )
    executable(fakes / "date", f'#!/bin/sh\necho "$(cat {tmp_path}/today)"\n')
    (tmp_path / "today").write_text(TODAY + "\n")
    env = {
        "PATH": f"{fakes}:{os.environ['PATH']}",
        "FILEDRAWER_DATABASE": str(database),
        "BACKUP_PING": PING,
    }

    def run(**extra) -> subprocess.CompletedProcess:
        return subprocess.run(
            [str(BACKUP)],
            env={**env, **extra},
            capture_output=True,
            text=True,
            check=False,
        )

    def calls() -> list[SimpleNamespace]:
        out = []
        for n in range(len(list(pings.glob("*.args")))):
            out.append(
                SimpleNamespace(
                    args=(pings / f"{n}.args").read_text().splitlines(),
                    stdin=(pings / f"{n}.stdin").read_text(),
                )
            )
        return out

    def pinged() -> list[str]:
        """The URL of each ping, from the config curl read on stdin."""
        urls = []
        for call in calls():
            for line in call.stdin.splitlines():
                if line.startswith("url"):
                    urls.append(line.split('"')[1])
        return urls

    return SimpleNamespace(
        tmp=tmp_path,
        fakes=fakes,
        database=database,
        backups=data / "backups",
        run=run,
        calls=calls,
        pinged=pinged,
        curl_fails=lambda: (tmp_path / "curl-status").write_text("22\n"),
        set_today=lambda day: (tmp_path / "today").write_text(day + "\n"),
    )


def copies(job) -> list[str]:
    return sorted(p.name for p in job.backups.iterdir() if not p.name.startswith("."))


def rows(path: Path) -> int:
    with sqlite3.connect(f"file:{path}?mode=ro", uri=True) as db:
        count = db.execute("SELECT count(*) FROM entries").fetchone()[0]
    db.close()
    return count


def test_a_copy_lands_under_todays_utc_date_with_every_row(job):
    result = job.run()
    assert result.returncode == 0, result.stderr
    assert copies(job) == [f"filedrawer-{TODAY}.db"]
    assert rows(job.backups / f"filedrawer-{TODAY}.db") == 3
    assert job.pinged() == [PING]


def test_the_ping_url_never_reaches_a_command_line_or_the_output(job):
    result = job.run()
    assert result.returncode == 0, result.stderr
    (call,) = job.calls()
    assert not any(PING in arg for arg in call.args), call.args
    assert PING not in result.stdout + result.stderr
    # The success ping carries the copy's size, so healthchecks.io's log shows it.
    size = (job.backups / f"filedrawer-{TODAY}.db").stat().st_size
    assert any(str(size) in arg for arg in call.args), call.args


def test_it_keeps_the_newest_two_copies_and_nothing_else_it_didnt_make(job):
    job.backups.mkdir()
    for day in ("2026-11-07", "2026-11-08", "2026-11-09"):
        make_database(job.backups / f"filedrawer-{day}.db")
    (job.backups / "notes.txt").write_text("the owner's\n")
    result = job.run()
    assert result.returncode == 0, result.stderr
    assert copies(job) == [
        "filedrawer-2026-11-09.db",
        f"filedrawer-{TODAY}.db",
        "notes.txt",
    ]


def test_a_second_run_on_one_day_replaces_that_days_copy(job):
    assert job.run().returncode == 0
    with sqlite3.connect(job.database) as db:
        db.execute("INSERT INTO entries VALUES ('e9', 'later')")
    db.close()
    result = job.run()
    assert result.returncode == 0, result.stderr
    assert copies(job) == [f"filedrawer-{TODAY}.db"]
    assert rows(job.backups / f"filedrawer-{TODAY}.db") == 4


@pytest.mark.parametrize("how", ["missing", "no entries table", "not a database"])
def test_a_failed_copy_pings_fail_and_leaves_nothing_under_its_name(job, how):
    job.database.unlink()
    for side in ("-wal", "-shm"):
        Path(f"{job.database}{side}").unlink(missing_ok=True)
    if how == "no entries table":
        with sqlite3.connect(job.database) as db:
            db.execute("CREATE TABLE other (x)")
        db.close()
    elif how == "not a database":
        job.database.write_bytes(b"this is not sqlite\n" * 300)
    result = job.run()
    assert result.returncode != 0
    assert job.pinged() == [PING + "/fail"]
    assert not job.backups.exists() or copies(job) == []
    # A missing database stays missing: the copy never makes an empty one.
    if how == "missing":
        assert not job.database.exists()


def test_a_failed_ping_after_the_copy_keeps_the_copy(job):
    job.curl_fails()
    result = job.run()
    assert result.returncode != 0
    assert copies(job) == [f"filedrawer-{TODAY}.db"]
    assert rows(job.backups / f"filedrawer-{TODAY}.db") == 3


def test_a_clock_behind_the_newest_copy_fails_and_touches_no_copy(job):
    job.backups.mkdir()
    for day in ("2026-11-10", "2026-11-11"):
        make_database(job.backups / f"filedrawer-{day}.db")
    job.set_today("2026-11-09")
    result = job.run()
    assert result.returncode != 0
    assert "clock" in result.stderr
    assert copies(job) == ["filedrawer-2026-11-10.db", "filedrawer-2026-11-11.db"]
    assert job.pinged() == [PING + "/fail"]


def test_a_run_while_another_holds_the_lock_fails_without_copying(job):
    job.backups.mkdir()
    with open(job.backups / ".lock", "w") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        result = job.run()
    assert result.returncode != 0
    assert "another" in result.stderr
    assert copies(job) == []
    assert job.pinged() == [PING + "/fail"]


def test_too_little_free_space_fails_before_copying(job):
    executable(
        job.fakes / "df",
        "#!/bin/sh\nprintf '     Avail\\n4096\\n'\n",
    )
    result = job.run()
    assert result.returncode != 0
    assert "space" in result.stderr
    assert copies(job) == []
    assert job.pinged() == [PING + "/fail"]


def test_a_leftover_partial_copy_goes_once_it_is_a_day_old(job):
    job.backups.mkdir()
    old = job.backups / "filedrawer-2026-11-08.abc123.tmp"
    recent = job.backups / "filedrawer-2026-11-10.def456.tmp"
    for path in (old, recent):
        path.write_bytes(b"partial")
    two_days_ago = time.time() - 2 * 86400
    os.utime(old, (two_days_ago, two_days_ago))
    result = job.run()
    assert result.returncode == 0, result.stderr
    assert not old.exists()
    assert recent.exists()


def test_an_empty_ping_url_is_refused(job):
    result = job.run(BACKUP_PING="")
    assert result.returncode != 0
    assert "BACKUP_PING" in result.stderr
    assert job.calls() == []


# backup.sh


@pytest.fixture
def box(tmp_path):
    root = tmp_path / "root"
    root.mkdir()
    fakes = tmp_path / "fakes"
    fakes.mkdir()
    log = tmp_path / "log"
    log.touch()
    for tool in ("systemctl", "chown"):
        executable(fakes / tool, f'#!/bin/sh\necho "{tool} $*" >> {log}\n')
    env = {"PATH": f"{fakes}:{os.environ['PATH']}", "FILEDRAWER_ROOT": str(root)}
    return SimpleNamespace(root=root, log=log, env=env)


def install(box) -> subprocess.CompletedProcess:
    return subprocess.run(
        ["bash", str(INSTALL)], env=box.env, capture_output=True, text=True, check=False
    )


def unit_lines(path: Path) -> list[str]:
    return [line.strip() for line in path.read_text().splitlines()]


def test_the_timer_runs_the_copy_at_2330_utc_and_catches_up_after_downtime(box):
    result = install(box)
    assert result.returncode == 0, result.stderr
    timer = unit_lines(box.root / "etc/systemd/system/filedrawer-backup.timer")
    assert "OnCalendar=*-*-* 23:30:00 UTC" in timer
    assert "Persistent=true" in timer
    assert "systemctl enable --now filedrawer-backup.timer" in box.log.read_text()


def test_the_copy_runs_as_productions_user_with_its_settings(box):
    result = install(box)
    assert result.returncode == 0, result.stderr
    unit = unit_lines(box.root / "etc/systemd/system/filedrawer-backup.service")
    for line in (
        "Type=oneshot",
        "User=filedrawer-prod",
        # The ping file is required: without it the job could never report.
        "EnvironmentFile=/etc/filedrawer/production.env",
        "EnvironmentFile=/etc/filedrawer/production-backup.env",
        "ExecStart=/usr/local/sbin/filedrawer-backup",
        # Until #42's first deploy there's no database, and the timer skips quietly.
        "ConditionPathExists=/srv/filedrawer/production/data/filedrawer.db",
        # A WAL reader writes -shm beside the database.
        "ReadWritePaths=/srv/filedrawer/production/data",
        "TimeoutStartSec=1h",
    ):
        assert line in unit, line


def test_the_backups_folder_is_productions_users_alone(box):
    result = install(box)
    assert result.returncode == 0, result.stderr
    backups = box.root / "srv/filedrawer/production/data/backups"
    assert stat.S_IMODE(backups.stat().st_mode) == 0o700
    assert f"chown filedrawer-prod:filedrawer-prod {backups}" in box.log.read_text()


def test_a_rerun_writes_the_same_units(box):
    assert install(box).returncode == 0
    units = box.root / "etc/systemd/system"
    first = {p.name: p.read_text() for p in units.iterdir()}
    result = install(box)
    assert result.returncode == 0, result.stderr
    assert {p.name: p.read_text() for p in units.iterdir()} == first
