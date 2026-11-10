"""Tests for the VPS scripts that keep staging and production apart:
ops/remote/service-user.sh (each environment's user, directories and settings files)
and ops/remote/loopback.sh (staging's user can't reach production's port). They run
against a temp root, with fakes of the tools that need root first on PATH.
"""

import os
import stat
import subprocess
from pathlib import Path
from types import SimpleNamespace

import pytest

REPO = Path(__file__).resolve().parents[2]
SERVICE_USER = REPO / "ops/remote/service-user.sh"
LOOPBACK = REPO / "ops/remote/loopback.sh"


def executable(path: Path, text: str) -> Path:
    path.write_text(text)
    path.chmod(path.stat().st_mode | stat.S_IXUSR)
    return path


@pytest.fixture
def box(tmp_path):
    root = tmp_path / "root"
    root.mkdir()
    fakes = tmp_path / "fakes"
    fakes.mkdir()
    log = tmp_path / "log"
    log.touch()
    users = tmp_path / "users"
    users.write_text("filedrawer\n")
    for tool in ("systemctl", "chown", "useradd", "nft", "apt-get"):
        executable(fakes / tool, f'#!/bin/sh\necho "{tool} $*" >> {log}\n')
    # id knows the users in `users`.
    executable(fakes / "id", f'#!/bin/sh\ngrep -qx "$1" {users}\n')
    env = {"PATH": f"{fakes}:{os.environ['PATH']}", "FILEDRAWER_ROOT": str(root)}
    return SimpleNamespace(root=root, log=log, env=env, users=users)


def run(box, script, **env) -> subprocess.CompletedProcess:
    return subprocess.run(
        ["bash", script],
        env={**box.env, **env},
        capture_output=True,
        text=True,
        check=False,
    )


def mode(path: Path) -> int:
    return stat.S_IMODE(path.stat().st_mode)


def chowns(box) -> list[str]:
    return [
        line for line in box.log.read_text().splitlines() if line.startswith("chown")
    ]


# service-user.sh


@pytest.mark.parametrize(
    ("environment", "user"),
    [("staging", "filedrawer"), ("production", "filedrawer-prod")],
)
def test_each_environment_gets_its_user_and_a_data_folder_only_that_user_reads(
    box, environment, user
):
    box.users.write_text("")
    result = run(box, SERVICE_USER, ENVIRONMENT=environment)
    assert result.returncode == 0, result.stderr
    assert (
        f"useradd --system --home-dir /srv/filedrawer/{environment} "
        f"--shell /usr/sbin/nologin {user}"
    ) in box.log.read_text()
    data = box.root / f"srv/filedrawer/{environment}/data"
    assert mode(data) == 0o700
    assert f"chown {user}:{user} {data}" in chowns(box)
    for path in (
        "srv/filedrawer",
        f"srv/filedrawer/{environment}",
        f"srv/filedrawer/{environment}/bin",
    ):
        assert mode(box.root / path) == 0o755, path
        assert f"chown root:root {box.root / path}" in chowns(box), path


def test_a_user_that_exists_is_kept(box):
    result = run(box, SERVICE_USER, ENVIRONMENT="staging")
    assert result.returncode == 0, result.stderr
    assert "useradd" not in box.log.read_text()


def test_settings_files_are_roots_alone_whatever_ran_before(box):
    """systemd reads a unit's EnvironmentFile as root, so no service user needs to
    read one; staging's files, once group-readable by its user, are made root's."""
    settings = box.root / "etc/filedrawer"
    settings.mkdir(parents=True)
    for name in ("staging.env", "staging-mail.env", "github.env"):
        (settings / name).write_text("X=1\n")
        (settings / name).chmod(0o640)
    result = run(box, SERVICE_USER, ENVIRONMENT="production")
    assert result.returncode == 0, result.stderr
    assert mode(settings) == 0o755
    assert f"chown root:root {settings}" in chowns(box)
    for name in ("staging.env", "staging-mail.env", "github.env"):
        assert mode(settings / name) == 0o600, name
        assert f"chown root:root {settings / name}" in " ".join(chowns(box)), name


@pytest.mark.parametrize("environment", ["", "dev", "../staging"])
def test_service_user_refuses_another_environment(box, environment):
    result = run(box, SERVICE_USER, ENVIRONMENT=environment)
    assert result.returncode == 2
    assert not (box.root / "srv").exists()
    assert box.log.read_text() == ""


# loopback.sh


def test_loopback_drops_stagings_connections_to_productions_port(box):
    result = run(box, LOOPBACK)
    assert result.returncode == 0, result.stderr
    rules_file = box.root / "etc/filedrawer/loopback.nft"
    assert mode(rules_file) == 0o644
    rules = rules_file.read_text()
    assert 'meta skuid "filedrawer" ip daddr 127.0.0.0/8 tcp dport 8080 reject' in rules
    assert 'meta skuid "filedrawer" ip6 daddr ::1 tcp dport 8080 reject' in rules
    # Its own table, replaced whole on a rerun, so ufw's rules stay as they are.
    assert "flush ruleset" not in rules
    assert rules.index("delete table inet filedrawer_loopback") < rules.index(
        "table inet filedrawer_loopback {"
    )
    log = box.log.read_text()
    assert "nft -c -f " in log
    assert "systemctl enable filedrawer-loopback.service" in log
    assert "systemctl restart filedrawer-loopback.service" in log


def test_the_loopback_unit_loads_the_rules_before_production_starts(box):
    result = run(box, LOOPBACK)
    assert result.returncode == 0, result.stderr
    unit = (box.root / "etc/systemd/system/filedrawer-loopback.service").read_text()
    assert "ExecStart=/usr/sbin/nft -f /etc/filedrawer/loopback.nft" in unit
    assert "Before=filedrawer-production.service" in unit
    assert "RemainAfterExit=yes" in unit


def test_loopback_writes_nothing_when_nft_refuses_the_rules(box):
    executable(Path(box.env["PATH"].split(":")[0]) / "nft", "#!/bin/sh\nexit 1\n")
    result = run(box, LOOPBACK)
    assert result.returncode != 0
    assert list((box.root / "etc/filedrawer").iterdir()) == [], "no temp file left"
    assert "systemctl" not in box.log.read_text()
