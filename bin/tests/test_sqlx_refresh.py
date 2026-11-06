"""Tests for bin/sqlx-refresh, which rebuilds target/dev.db from the migrations and
refreshes .sqlx/. It runs as a process against a scratch repo, with fake `sqlx` and
`cargo` first on PATH that log their arguments and the DATABASE_URL they were given.
"""

import os
import stat
import subprocess
from pathlib import Path

import pytest

SCRIPT = Path(__file__).resolve().parents[1] / "sqlx-refresh"

FAKE = """#!/bin/sh
echo "$(basename "$0") $* url=$DATABASE_URL" >> "$LOG"
case "$(basename "$0") $*" in
  "sqlx database create"*) : > "${DATABASE_URL#sqlite://}" ;;
esac
if [ "$(basename "$0")" = "${FAIL:-}" ]; then echo "boom" >&2; exit 3; fi
"""


@pytest.fixture
def repo(tmp_path):
    root = tmp_path / "repo"
    (root / "target").mkdir(parents=True)
    (root / "crates/server/migrations").mkdir(parents=True)
    fakes = tmp_path / "fakes"
    fakes.mkdir()
    for name in ("sqlx", "cargo"):
        path = fakes / name
        path.write_text(FAKE)
        path.chmod(path.stat().st_mode | stat.S_IXUSR)
    log = tmp_path / "log"
    log.touch()
    env = {**os.environ, "PATH": f"{fakes}:{os.environ['PATH']}", "LOG": str(log)}
    env.pop("DATABASE_URL", None)
    return root, env, log


def run(root, env, *args):
    return subprocess.run(
        [str(SCRIPT), "--repo", str(root), *args],
        env=env,
        capture_output=True,
        text=True,
    )


def test_rebuilds_the_database_then_prepares(repo):
    root, env, log = repo
    old = root / "target/dev.db"
    old.write_text("stale schema with an old checksum")
    (root / "target/dev.db-wal").write_text("wal")
    done = run(root, env)
    assert done.returncode == 0, done.stderr
    url = f"sqlite://{root / 'target/dev.db'}"
    assert log.read_text().splitlines() == [
        f"sqlx database create url={url}",
        f"sqlx migrate run --source crates/server/migrations url={url}",
        f"cargo sqlx prepare --workspace url={url}",
    ]
    assert old.read_text() == "", "the old database was deleted first"
    assert not (root / "target/dev.db-wal").exists()


def test_writes_a_missing_env_file_and_keeps_an_existing_one(repo):
    root, env, _ = repo
    assert run(root, env).returncode == 0
    url = f"sqlite://{root / 'target/dev.db'}"
    assert (root / ".env").read_text() == f"DATABASE_URL={url}\n"
    (root / ".env").write_text("DATABASE_URL=sqlite://elsewhere.db\nOTHER=1\n")
    assert run(root, env).returncode == 0
    assert (
        root / ".env"
    ).read_text() == "DATABASE_URL=sqlite://elsewhere.db\nOTHER=1\n"


def test_a_failing_step_stops_with_one_line(repo):
    root, env, log = repo
    done = run(root, {**env, "FAIL": "sqlx"})
    assert done.returncode == 1
    assert (
        done.stderr.strip()
        .splitlines()[-1]
        .startswith("sqlx-refresh: `sqlx database create` failed (exit 3)")
    )
    assert "cargo" not in log.read_text()


def test_help_says_what_it_does():
    done = subprocess.run([str(SCRIPT), "-h"], capture_output=True, text=True)
    assert done.returncode == 0
    assert "target/dev.db" in done.stdout and ".sqlx" in done.stdout
