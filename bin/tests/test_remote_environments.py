"""Tests for the VPS scripts that keep staging and production apart:
ops/remote/service-user.sh (each environment's user, directories and settings files)
and ops/remote/loopback.sh (staging's user can't reach production's port). They run
against a temp root, with fakes of the tools that need root first on PATH.
"""

import os
import re
import socket
import time
import stat
import subprocess
from pathlib import Path
from types import SimpleNamespace

import pytest
from conftest import unix_listener

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


# The environments' names, users and ports agree across the scripts


ENVIRONMENTS = {
    "staging": ("filedrawer", 8081),
    "production": ("filedrawer-prod", 8080),
}


def test_every_script_maps_each_environment_to_the_same_user_and_port():
    """Each script goes to the server alone, so each keeps its own copy of the map."""
    for name in ("filedrawer-install", "service-user.sh", "service.sh"):
        text = (REPO / "ops/remote" / name).read_text()
        for environment, (user, port) in ENVIRONMENTS.items():
            [line] = re.findall(rf"^\s*{environment}\) (.*);;$", text, re.M)
            assert f"user={user} " in line + " ", (name, line)
            if "port=" in line:
                assert f"port={port} " in line + " ", (name, line)
    loopback = LOOPBACK.read_text()
    staging_user, _ = ENVIRONMENTS["staging"]
    _, production_port = ENVIRONMENTS["production"]
    assert f'meta skuid "{staging_user}"' in loopback
    assert f"dport {production_port} " in loopback


# service.sh

SERVICE = REPO / "ops/remote/service.sh"


def unit(box, environment) -> str:
    return (
        box.root / f"etc/systemd/system/filedrawer-{environment}.service"
    ).read_text()


def settings(box, name) -> dict[str, str]:
    text = (box.root / "etc/filedrawer" / name).read_text()
    return dict(line.split("=", 1) for line in text.splitlines() if line)


def test_production_gets_its_own_settings_and_unit(box):
    result = run(box, SERVICE, ENVIRONMENT="production")
    assert result.returncode == 0, result.stderr
    assert settings(box, "production.env") == {
        "FILEDRAWER_DATABASE": "/srv/filedrawer/production/data/filedrawer.db",
        "FILEDRAWER_LISTEN": "127.0.0.1:8080",
        "FILEDRAWER_ENVIRONMENT": "production",
        "FILEDRAWER_PUBLIC_URL": "https://filedrawer.dev",
        "FILEDRAWER_FILES_URL": "https://files.filedrawer.dev",
        "RUST_LOG": "info",
    }
    text = unit(box, "production")
    for line in (
        "User=filedrawer-prod",
        "EnvironmentFile=/etc/filedrawer/production.env",
        "EnvironmentFile=-/etc/filedrawer/production-mail.env",
        "EnvironmentFile=-/etc/filedrawer/production-screening.env",
        "EnvironmentFile=-/etc/filedrawer/production-ping.env",
        "EnvironmentFile=-/etc/filedrawer/github.env",
        "ExecStart=/srv/filedrawer/production/bin/filedrawer serve",
        "ConditionPathIsExecutable=/srv/filedrawer/production/bin/filedrawer",
        "ReadWritePaths=/srv/filedrawer/production/data",
        "SocketBindAllow=tcp:8080",
        "SocketBindAllow=udp",
        "SocketBindDeny=any",
        "Requires=filedrawer-loopback.service",
    ):
        assert f"\n{line}\n" in text, line
    assert "smoke" not in text
    assert "systemctl enable filedrawer-production.service" in box.log.read_text()


def test_staging_keeps_its_settings_and_unit_and_may_bind_only_its_tcp_port(box):
    result = run(box, SERVICE, ENVIRONMENT="staging")
    assert result.returncode == 0, result.stderr
    assert settings(box, "staging.env") == {
        "FILEDRAWER_DATABASE": "/srv/filedrawer/staging/data/filedrawer.db",
        "FILEDRAWER_LISTEN": "127.0.0.1:8081",
        "FILEDRAWER_ENVIRONMENT": "staging",
        "FILEDRAWER_SCREENER": "pass-all",
        "FILEDRAWER_PUBLIC_URL": "https://staging.filedrawer.dev",
        "FILEDRAWER_FILES_URL": "https://files.staging.filedrawer.dev",
        "RUST_LOG": "info",
    }
    text = unit(box, "staging")
    for line in (
        "User=filedrawer",
        "EnvironmentFile=-/etc/filedrawer/staging-smoke.env",
        "SocketBindAllow=tcp:8081",
        "SocketBindAllow=udp",
        "SocketBindDeny=any",
        "InaccessiblePaths=-/srv/filedrawer/production",
    ):
        assert f"\n{line}\n" in text, line
    assert "loopback" not in text
    # Screening's settings are read after the unit's own, so they replace pass-all.
    assert text.index("staging.env\n") < text.index("staging-screening.env\n")


@pytest.mark.parametrize("environment", ["staging", "production"])
def test_service_writes_settings_only_root_reads(box, environment):
    result = run(box, SERVICE, ENVIRONMENT=environment)
    assert result.returncode == 0, result.stderr
    path = box.root / f"etc/filedrawer/{environment}.env"
    assert mode(path) == 0o600
    assert f"chown root:root {path}" in chowns(box)


def test_service_for_one_environment_leaves_the_others_files_alone(box):
    assert run(box, SERVICE, ENVIRONMENT="production").returncode == 0
    before = {path: path.read_bytes() for path in box.root.rglob("*") if path.is_file()}
    assert run(box, SERVICE, ENVIRONMENT="staging").returncode == 0
    for path, content in before.items():
        assert path.read_bytes() == content, path


def restarts(box, environment) -> list[str]:
    return [
        line
        for line in box.log.read_text().splitlines()
        if line == f"systemctl try-restart filedrawer-{environment}.service"
    ]


@pytest.mark.parametrize("environment", ["staging", "production"])
def test_a_changed_unit_restarts_its_service_and_an_unchanged_one_doesnt(
    box, environment
):
    # The first run writes the unit; a second, unchanged, restarts nothing.
    assert run(box, SERVICE, ENVIRONMENT=environment).returncode == 0
    assert run(box, SERVICE, ENVIRONMENT=environment).returncode == 0
    assert restarts(box, environment) == []
    # As an older service.sh wrote it: on a rerun nothing else may restart it.
    path = box.root / f"etc/systemd/system/filedrawer-{environment}.service"
    path.write_text(path.read_text().replace("SocketBindAllow=udp\n", ""))
    assert run(box, SERVICE, ENVIRONMENT=environment).returncode == 0
    assert restarts(box, environment) == [
        f"systemctl try-restart filedrawer-{environment}.service"
    ]
    log = box.log.read_text()
    assert log.rindex("daemon-reload") < log.rindex("try-restart")


def test_changed_base_settings_restart_the_service_too(box):
    assert run(box, SERVICE, ENVIRONMENT="staging").returncode == 0
    path = box.root / "etc/filedrawer/staging.env"
    path.write_text(path.read_text().replace("RUST_LOG=info", "RUST_LOG=debug"))
    assert run(box, SERVICE, ENVIRONMENT="staging").returncode == 0
    assert restarts(box, "staging") == [
        "systemctl try-restart filedrawer-staging.service"
    ]


@pytest.mark.parametrize("environment", ["", "dev"])
def test_service_refuses_another_environment(box, environment):
    result = run(box, SERVICE, ENVIRONMENT=environment)
    assert result.returncode == 2
    assert not (box.root / "etc").exists()


# mail.sh and screening.sh by environment

MAIL = REPO / "ops/remote/mail.sh"
SCREENING = REPO / "ops/remote/screening.sh"
API_KEY = "sk-ant-api03-" + "x" * 40


@pytest.mark.parametrize("environment", ["staging", "production"])
def test_mail_writes_the_environments_own_file_and_restarts_only_it(box, environment):
    result = run(
        box,
        MAIL,
        ENVIRONMENT=environment,
        SMTP_USER=f"{environment}@filedrawer.dev",
        MAIL_FROM="File Drawer <noreply@filedrawer.dev>",
        SMTP_PASSWORD="pw",
    )
    assert result.returncode == 0, result.stderr
    path = box.root / f"etc/filedrawer/{environment}-mail.env"
    assert (
        settings(box, path.name)["FILEDRAWER_SMTP_USER"]
        == f"'{environment}@filedrawer.dev'"
    )
    assert mode(path) == 0o600
    assert f"chown root:root {path}" in chowns(box)
    restarted = [line for line in box.log.read_text().splitlines() if "restart" in line]
    assert restarted == [f"systemctl try-restart filedrawer-{environment}.service"]


@pytest.mark.parametrize("environment", ["staging", "production"])
def test_screening_writes_the_environments_own_file(box, environment):
    result = run(
        box, SCREENING, ENVIRONMENT=environment, API_KEY=API_KEY, DAILY_USD="1"
    )
    assert result.returncode == 0, result.stderr
    path = box.root / f"etc/filedrawer/{environment}-screening.env"
    assert settings(box, path.name)["FILEDRAWER_SCREENER"] == "anthropic"
    assert mode(path) == 0o600
    assert (
        f"systemctl try-restart filedrawer-{environment}.service" in box.log.read_text()
    )


@pytest.mark.parametrize("script", [MAIL, SCREENING])
def test_mail_and_screening_refuse_another_environment(box, script):
    result = run(
        box,
        script,
        ENVIRONMENT="dev",
        SMTP_USER="u",
        MAIL_FROM="f",
        SMTP_PASSWORD="pw",
        API_KEY=API_KEY,
        DAILY_USD="1",
    )
    assert result.returncode == 2
    assert not list((box.root).rglob("*.env"))


# github-token.sh

GITHUB = REPO / "ops/remote/github-token.sh"


def test_the_github_token_restarts_every_unit_set_up_and_only_those(box):
    units = box.root / "etc/systemd/system"
    units.mkdir(parents=True)
    (units / "filedrawer-staging.service").write_text("")
    result = run(box, GITHUB, TOKEN="github_pat_x")
    assert result.returncode == 0, result.stderr
    log = box.log.read_text()
    assert "try-restart filedrawer-staging.service" in log
    assert "filedrawer-production" not in log
    (units / "filedrawer-production.service").write_text("")
    box.log.write_text("")
    assert run(box, GITHUB, TOKEN="github_pat_y").returncode == 0
    log = box.log.read_text()
    assert "try-restart filedrawer-staging.service" in log
    assert "try-restart filedrawer-production.service" in log
    path = box.root / "etc/filedrawer/github.env"
    assert mode(path) == 0o600
    assert f"chown root:root {path}" in chowns(box)


# ping.sh: production's healthchecks.io URLs

PING = REPO / "ops/remote/ping.sh"
DAILY = "https://hc-ping.com/1f6a0b2c-0000-4000-8000-000000000001"
BACKUP = "https://hc-ping.com/1f6a0b2c-0000-4000-8000-000000000002"


def test_ping_writes_the_daily_emails_url_for_the_server_and_the_backups_for_its_timer(
    box,
):
    result = run(box, PING, PINGS=f"{DAILY}\t{BACKUP}")
    assert result.returncode == 0, result.stderr
    assert settings(box, "production-ping.env") == {
        "FILEDRAWER_DAILY_EMAIL_PING": DAILY
    }
    assert settings(box, "production-backup.env") == {"BACKUP_PING": BACKUP}
    for name in ("production-ping.env", "production-backup.env"):
        assert mode(box.root / "etc/filedrawer" / name) == 0o600
    assert "systemctl try-restart filedrawer-production.service" in box.log.read_text()


@pytest.mark.parametrize(
    "pings",
    [
        f"{DAILY}",
        f"http://hc-ping.com/x\t{BACKUP}",
        f"{DAILY}\thttps://example.com/x",
        f"{DAILY}\thttps://hc-ping.com/",
        f"{DAILY}\thttps://hc-ping.com.evil.dev/x",
        f"{DAILY}\t{BACKUP}'",
    ],
)
def test_ping_refuses_what_isnt_a_healthchecks_ping_url_and_writes_nothing(box, pings):
    result = run(box, PING, PINGS=pings)
    assert result.returncode == 1
    # The message's example, https://hc-ping.com/<uuid>, holds the bare host.
    for url in pings.split("\t"):
        if url not in ("", "https://hc-ping.com/"):
            assert url not in result.stdout + result.stderr
    assert not (box.root / "etc/filedrawer").exists() or not list(
        (box.root / "etc/filedrawer").iterdir()
    )


# caddy.sh: one site file per environment

CADDY = REPO / "ops/remote/caddy.sh"
STAGING_KEY = "ab" * 32
NEW_HASH = "$2a$14$NEWNEWNEWNEWNEWNEWNEWNuNEWNEWNEWNEWNEWNEWNEWNEWNEWNEWNE"
OLD_HASH = "$2a$14$abcdefghijklmnopqrstuuABCDEFGHIJKLMNOPQRSTUVWXYZ01234"


@pytest.fixture
def caddy(box):
    """caddy.sh against the box, with a fake caddy whose validate fails while the
    file `invalid` exists, and which notes the site files it validated with. Asked
    to hash a password it reads from stdin, it keeps what it read in `hashed`."""
    (box.root / "etc/caddy").mkdir(parents=True)
    fakes = Path(box.env["PATH"].split(":")[0])
    invalid = box.root.parent / "invalid"
    hashed = box.root.parent / "hashed"
    executable(
        fakes / "caddy",
        f'#!/bin/sh\necho "caddy $*" >> {box.log}\n'
        f'[ "$*" = hash-password ] && cat > {hashed}\n'
        f"[ \"$1\" = hash-password ] && echo '{NEW_HASH}'\n"
        f'[ "$1" = validate ] && [ -e {invalid} ] && exit 1\n'
        "exit 0\n",
    )

    def run_caddy(
        environment: str, password: str = "pw"
    ) -> subprocess.CompletedProcess:
        env = {"ENVIRONMENT": environment}
        if environment == "staging":
            env |= {
                "STAGING_USER": "owner",
                "STAGING_SECRETS": f"{password}\t{STAGING_KEY}",
            }
        return run(box, CADDY, **env)

    return SimpleNamespace(
        run=run_caddy, invalid=invalid, hashed=hashed, dir=box.root / "etc/caddy"
    )


ADMIN_SOCKET = "/var/lib/caddy/admin.sock"


def site_blocks(text: str) -> dict[str, str]:
    return dict(re.findall(r"^([a-z.]+\.dev) \{\n(.*?)^\}", text, re.M | re.S))


def test_the_caddyfile_imports_each_environments_site_file(box, caddy):
    assert caddy.run("staging").returncode == 0
    caddyfile = (caddy.dir / "Caddyfile").read_text()
    assert "import /etc/caddy/sites/*.caddy" in caddyfile
    assert "filedrawer.dev {" not in caddyfile


@pytest.mark.parametrize("environment", ["staging", "production"])
def test_the_admin_api_is_a_socket_only_caddy_and_root_can_open(
    box, caddy, environment
):
    """Caddy's admin API reads and replaces the whole config, every site's, so no
    other local user may reach it: not on loopback, Caddy's default, where every
    user connects, but on a socket in Caddy's home that its owner alone may write."""
    assert caddy.run(environment).returncode == 0
    lines = [
        line
        for line in (caddy.dir / "Caddyfile").read_text().splitlines()
        if not line.startswith("#")
    ]
    assert lines == [
        "{",
        f"\tadmin unix/{ADMIN_SOCKET}|0200",
        "}",
        "import /etc/caddy/sites/*.caddy",
    ]


def test_caddy_restarts_while_its_admin_socket_isnt_there_to_reload_through(box, caddy):
    """A reload goes through the admin API the new config names, which a Caddy
    started with another config, or none, isn't listening on."""
    assert caddy.run("production").returncode == 0
    calls = [
        line
        for line in box.log.read_text().splitlines()
        if line.startswith("systemctl") and line.endswith(" caddy")
    ]
    assert calls == ["systemctl enable caddy", "systemctl restart caddy"]


def test_caddy_reloads_through_its_admin_socket_once_it_listens_there(box, caddy):
    with unix_listener(box.root / ADMIN_SOCKET.lstrip("/")):
        assert caddy.run("production").returncode == 0
    calls = [
        line
        for line in box.log.read_text().splitlines()
        if line.startswith("systemctl") and line.endswith(" caddy")
    ]
    assert calls == ["systemctl enable caddy", "systemctl reload-or-restart caddy"]


def test_a_reload_that_fails_says_how_to_load_the_config_caddy_took(box, caddy):
    """The files stay, since Caddy validated them; a Caddy started with another
    config, whose socket file is left over, can't be reloaded into this one."""
    fakes = Path(box.env["PATH"].split(":")[0])
    executable(
        fakes / "systemctl",
        f'#!/bin/sh\necho "systemctl $*" >> {box.log}\n[ "$1" != reload-or-restart ]\n',
    )
    with unix_listener(box.root / ADMIN_SOCKET.lstrip("/")):
        result = caddy.run("production")
    assert result.returncode == 1
    assert "systemctl restart caddy" in result.stderr
    assert (caddy.dir / "sites/production.caddy").exists()


def test_the_password_reaches_caddy_on_stdin_and_never_in_a_command_line(box, caddy):
    """Any local user can read another process's arguments while it runs."""
    assert caddy.run("staging", password="correct horse").returncode == 0
    log = box.log.read_text()
    assert "caddy hash-password\n" in log
    assert "correct horse" not in log
    assert caddy.hashed.read_text() == "correct horse"


@pytest.mark.parametrize("environment", ["staging", "production"])
def test_a_site_file_is_caddys_group_alone_to_read(box, caddy, environment):
    """Staging's holds the staging key and the basic-auth hash, which no other local
    user, such as the one CI's binaries run as, may read (#101)."""
    result = caddy.run(environment)
    assert result.returncode == 0, result.stderr
    site = caddy.dir / f"sites/{environment}.caddy"
    assert mode(site) == 0o640
    chowned = [
        line for line in box.log.read_text().splitlines() if line.startswith("chown")
    ]
    # Owner and mode are set before the rename, on the temp file beside it.
    assert any(
        line.startswith("chown root:caddy ") and "/sites/.caddy." in line
        for line in chowned
    ), chowned
    assert mode(caddy.dir / "Caddyfile") == 0o644


def test_production_serves_the_apex_and_files_and_redirects_www(box, caddy):
    result = caddy.run("production")
    assert result.returncode == 0, result.stderr
    text = (caddy.dir / "sites/production.caddy").read_text()
    blocks = site_blocks(text)
    assert set(blocks) == {
        "filedrawer.dev",
        "files.filedrawer.dev",
        "www.filedrawer.dev",
    }
    for name in ("filedrawer.dev", "files.filedrawer.dev"):
        assert "reverse_proxy 127.0.0.1:8080" in blocks[name], name
        assert "import production_down" in blocks[name], name
    assert "redir https://filedrawer.dev{uri} permanent" in blocks["www.filedrawer.dev"]
    for gate in ("basic_auth", "robots.txt", "X-Staging-Key", "field-test"):
        assert gate not in text, gate


def test_production_answers_a_503_that_says_to_retry_while_the_server_is_down(
    box, caddy
):
    assert caddy.run("production").returncode == 0
    text = (caddy.dir / "sites/production.caddy").read_text()
    down = text[text.index("(production_down) {") :]
    assert "handle_errors" in down
    assert "header Retry-After" in down
    assert re.search(r'respond "File Drawer [^"]*" 503', down), down


def test_staging_keeps_its_gate_in_its_own_site_file(box, caddy):
    assert caddy.run("staging").returncode == 0
    text = (caddy.dir / "sites/staging.caddy").read_text()
    assert "(staging_gate) {" in text
    assert f'@agent header X-Staging-Key "{STAGING_KEY}"' in text
    assert f"owner {NEW_HASH}" in text
    assert set(site_blocks(text)) == {
        "staging.filedrawer.dev",
        "files.staging.filedrawer.dev",
    }


def test_provisioning_one_environment_leaves_the_others_site_file_alone(box, caddy):
    assert caddy.run("production").returncode == 0
    production = (caddy.dir / "sites/production.caddy").read_bytes()
    assert caddy.run("staging").returncode == 0
    assert (caddy.dir / "sites/production.caddy").read_bytes() == production
    staging = (caddy.dir / "sites/staging.caddy").read_bytes()
    assert caddy.run("production").returncode == 0
    assert (caddy.dir / "sites/staging.caddy").read_bytes() == staging


def test_a_staging_rerun_finds_the_hash_in_the_old_single_caddyfile(box, caddy):
    """The first rerun after the split: the hash is only in the Caddyfile caddy.sh
    wrote before, and a rerun sends no password."""
    (caddy.dir / "Caddyfile").write_text(
        f"staging.filedrawer.dev {{\n\tbasic_auth {{\n\t\towner {OLD_HASH}\n\t}}\n}}\n"
    )
    result = caddy.run("staging", password="")
    assert result.returncode == 0, result.stderr
    assert f"owner {OLD_HASH}" in (caddy.dir / "sites/staging.caddy").read_text()
    assert "import /etc/caddy/sites/*.caddy" in (caddy.dir / "Caddyfile").read_text()


def test_a_staging_rerun_keeps_the_hash_in_its_site_file(box, caddy):
    assert caddy.run("staging").returncode == 0
    result = caddy.run("staging", password="")
    assert result.returncode == 0, result.stderr
    assert f"owner {NEW_HASH}" in (caddy.dir / "sites/staging.caddy").read_text()


def test_a_config_caddy_refuses_puts_every_file_back_and_reloads_nothing(box, caddy):
    old = (
        f"staging.filedrawer.dev {{\n\tbasic_auth {{\n\t\towner {OLD_HASH}\n\t}}\n}}\n"
    )
    (caddy.dir / "Caddyfile").write_text(old)
    caddy.invalid.touch()
    result = caddy.run("production")
    assert result.returncode == 1
    assert (caddy.dir / "Caddyfile").read_text() == old
    assert not (caddy.dir / "sites/production.caddy").exists()
    assert [p.name for p in caddy.dir.rglob("*") if p.is_file()] == ["Caddyfile"]
    assert "reload" not in box.log.read_text()


def test_a_refused_site_file_puts_back_the_one_before(box, caddy):
    assert caddy.run("production").returncode == 0
    before = (caddy.dir / "sites/production.caddy").read_bytes()
    (caddy.dir / "sites/production.caddy").write_bytes(before + b"# hand edit\n")
    edited = (caddy.dir / "sites/production.caddy").read_bytes()
    caddy.invalid.touch()
    assert caddy.run("production").returncode == 1
    assert (caddy.dir / "sites/production.caddy").read_bytes() == edited


def test_production_refuses_to_replace_a_single_caddyfile_that_still_holds_staging(
    box, caddy
):
    """On a server set up before the site files, staging's sites are only in the
    Caddyfile; replacing it with the import line would drop them."""
    old = (
        f"staging.filedrawer.dev {{\n\tbasic_auth {{\n\t\towner {OLD_HASH}\n\t}}\n}}\n"
    )
    (caddy.dir / "Caddyfile").write_text(old)
    result = caddy.run("production")
    assert result.returncode == 1
    assert "a rerun of ops/provision-vps" in result.stderr
    assert (caddy.dir / "Caddyfile").read_text() == old
    assert not (caddy.dir / "sites/production.caddy").exists()


def test_production_after_stagings_migration_goes_ahead(box, caddy):
    (caddy.dir / "Caddyfile").write_text(
        f"staging.filedrawer.dev {{\n\tbasic_auth {{\n\t\towner {OLD_HASH}\n\t}}\n}}\n"
    )
    assert caddy.run("staging", password="").returncode == 0
    result = caddy.run("production")
    assert result.returncode == 0, result.stderr
    assert f"owner {OLD_HASH}" in (caddy.dir / "sites/staging.caddy").read_text()


def test_a_run_stopped_after_the_swap_puts_the_files_back(box, caddy):
    """Should the ssh session die between the swap and Caddy's verdict, the files
    before come back."""
    old = "import /etc/caddy/sites/*.caddy\n"
    (caddy.dir / "Caddyfile").write_text(old)
    (caddy.dir / "sites").mkdir()
    (caddy.dir / "sites/staging.caddy").write_text("# staging\n")
    fakes = Path(box.env["PATH"].split(":")[0])
    executable(
        fakes / "caddy", '#!/bin/sh\n[ "$1" = validate ] && kill -TERM $PPID\nexit 0\n'
    )
    result = caddy.run("production")
    assert result.returncode != 0
    assert (caddy.dir / "Caddyfile").read_text() == old
    assert not (caddy.dir / "sites/production.caddy").exists()


@pytest.mark.parametrize("environment", ["", "dev"])
def test_caddy_refuses_another_environment(box, caddy, environment):
    assert caddy.run(environment).returncode == 2
    assert list(caddy.dir.iterdir()) == []


def free_port() -> int:
    with socket.socket() as probe:
        probe.bind(("127.0.0.1", 0))
        return probe.getsockname()[1]


def test_a_real_caddy_takes_both_environments_site_files_together(
    box, caddy, tmp_path, real_caddy
):
    assert caddy.run("production").returncode == 0
    assert caddy.run("staging").returncode == 0
    here = str(caddy.dir)
    for site in (caddy.dir / "sites").iterdir():
        site.write_text(site.read_text().replace("/etc/caddy", here))
    (caddy.dir / "field-test.caddy").write_text("")
    config = tmp_path / "Caddyfile"
    config.write_text((caddy.dir / "Caddyfile").read_text().replace("/etc/caddy", here))
    result = subprocess.run(
        [real_caddy, "validate", "--config", str(config), "--adapter", "caddyfile"],
        capture_output=True,
        text=True,
        check=False,
    )
    assert result.returncode == 0, result.stderr[-2000:]


def test_a_real_caddy_answers_503_while_production_is_down_and_redirects_www(
    box, caddy, tmp_path, real_caddy
):
    assert caddy.run("production").returncode == 0
    text = (caddy.dir / "sites/production.caddy").read_text()
    apex, www, down = free_port(), free_port(), free_port()
    config = tmp_path / "Caddyfile"
    config.write_text(
        "{\n\tadmin off\n\tauto_https off\n}\n"
        + text.replace("127.0.0.1:8080", f"127.0.0.1:{down}")
        .replace("\nfiledrawer.dev {", f"\n:{apex} {{")
        .replace("www.filedrawer.dev {", f":{www} {{")
        .replace("files.filedrawer.dev {", "(files_unused) {")
    )
    server = subprocess.Popen(
        [real_caddy, "run", "--config", str(config), "--adapter", "caddyfile"],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    try:

        def fetch(port: int, path: str) -> tuple[int, str, str]:
            done = subprocess.run(
                ["curl", "-s", "-D", "-", f"http://127.0.0.1:{port}{path}"],
                capture_output=True,
                text=True,
                check=False,
            )
            # text=True reads curl's \r\n line ends as \n.
            head, _, body = done.stdout.partition("\n\n")
            status = int(head.split()[1]) if head else 0
            return status, head, body

        while fetch(apex, "/tags")[0] == 0:
            assert server.poll() is None, "caddy stopped"
            time.sleep(0.1)
        status, head, body = fetch(apex, "/tags")
        assert status == 503
        assert "Retry-After: 120" in head
        assert body.startswith("File Drawer isn't answering right now")
        status, head, _ = fetch(www, "/e/x?y=1")
        assert status == 301
        assert "Location: https://filedrawer.dev/e/x?y=1" in head
    finally:
        server.terminate()
        server.wait()


def listening_ports(pid: int) -> set[int]:
    """The TCP ports the process listens on, on any address."""
    done = subprocess.run(
        ["ss", "-H", "-ltnp"], capture_output=True, text=True, check=True
    )
    return {
        int(line.split()[3].rsplit(":", 1)[1])
        for line in done.stdout.splitlines()
        if f"pid={pid}," in line
    }


def test_a_real_caddy_listens_for_its_admin_api_on_the_socket_alone_and_reloads(
    box, caddy, tmp_path, real_caddy
):
    """The Caddyfile as caddy.sh writes it, with only its paths moved: /etc/caddy
    to the box, and the socket to the folder Caddy runs in, since a temp path runs
    past the 107 bytes a socket's address holds."""
    assert caddy.run("production").returncode == 0
    apex, www, down = free_port(), free_port(), free_port()
    site = caddy.dir / "sites/production.caddy"
    site.write_text(
        site.read_text()
        .replace("127.0.0.1:8080", f"127.0.0.1:{down}")
        .replace("\nfiledrawer.dev {", f"\n:{apex} {{")
        .replace("www.filedrawer.dev {", f":{www} {{")
        .replace("files.filedrawer.dev {", "(files_unused) {")
    )
    home = tmp_path / "home"
    home.mkdir()
    config = tmp_path / "Caddyfile"
    config.write_text(
        (caddy.dir / "Caddyfile")
        .read_text()
        .replace("/etc/caddy", str(caddy.dir))
        .replace(f"unix/{ADMIN_SOCKET}", "unix/admin.sock")
    )
    assert "\tadmin unix/admin.sock|0200\n" in config.read_text()
    server = subprocess.Popen(
        [real_caddy, "run", "--config", str(config), "--adapter", "caddyfile"],
        cwd=home,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    try:

        def retry_after() -> str:
            done = subprocess.run(
                ["curl", "-s", "-D", "-", f"http://127.0.0.1:{apex}/tags"],
                capture_output=True,
                text=True,
                check=False,
            )
            found = re.search(r"^Retry-After: (\d+)$", done.stdout, re.M)
            return found.group(1) if found else ""

        while not retry_after():
            assert server.poll() is None, "caddy stopped"
            time.sleep(0.1)
        assert listening_ports(server.pid) == {apex, www}
        admin = (home / "admin.sock").stat()
        assert stat.S_ISSOCK(admin.st_mode)
        # A connection needs write access, which only the owner has.
        assert stat.S_IMODE(admin.st_mode) == 0o200

        # As the packaged unit's ExecReload runs it: the address comes from the config.
        site.write_text(site.read_text().replace("Retry-After 120", "Retry-After 121"))
        reloaded = subprocess.run(
            [real_caddy, "reload", "--config", str(config), "--force"],
            cwd=home,
            capture_output=True,
            text=True,
            check=False,
        )
        assert reloaded.returncode == 0, reloaded.stderr[-2000:]
        assert retry_after() == "121"
        assert listening_ports(server.pid) == {apex, www}
    finally:
        server.terminate()
        server.wait()


# An empty secret keeps the one stored (#112): the wizard sends one when the owner
# keeps the server's copy, and still runs the script so its new logic reaches the
# server.


def stored(box, name) -> str:
    return (box.root / "etc/filedrawer" / name).read_text()


def mail(box, password, user="noreply@filedrawer.dev", environment="staging"):
    return run(
        box,
        MAIL,
        ENVIRONMENT=environment,
        SMTP_USER=user,
        MAIL_FROM="File Drawer <noreply@filedrawer.dev>",
        SMTP_PASSWORD=password,
    )


def test_screening_keeps_the_stored_key_and_takes_the_new_cap(box):
    run(box, SCREENING, ENVIRONMENT="staging", API_KEY=API_KEY, DAILY_USD="1")
    result = run(box, SCREENING, ENVIRONMENT="staging", API_KEY="", DAILY_USD="2")
    assert result.returncode == 0, result.stderr
    kept = settings(box, "staging-screening.env")
    assert kept["FILEDRAWER_ANTHROPIC_API_KEY"] == f"'{API_KEY}'"
    assert kept["FILEDRAWER_SCREENING_DAILY_USD"] == "2"
    assert API_KEY not in result.stdout + result.stderr


@pytest.mark.parametrize("before", [None, "FILEDRAWER_ANTHROPIC_API_KEY='bogus'\n"])
@pytest.mark.parametrize(
    ("environment", "stage"),
    [
        ("staging", "--ask-all screening"),
        ("production", "--ask-all production screening"),
    ],
)
def test_screening_refuses_an_empty_key_with_none_it_can_keep(
    box, before, environment, stage
):
    path = box.root / f"etc/filedrawer/{environment}-screening.env"
    if before is not None:
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(before)
    result = run(box, SCREENING, ENVIRONMENT=environment, API_KEY="", DAILY_USD="1")
    assert result.returncode == 1
    assert stage in result.stderr
    assert "bogus" not in result.stdout + result.stderr
    assert before is None or path.read_text() == before


def test_mail_keeps_the_stored_password_for_the_same_user(box):
    mail(box, 'pa"ss\\word')
    result = mail(box, "")
    assert result.returncode == 0, result.stderr
    assert (
        settings(box, "staging-mail.env")["FILEDRAWER_SMTP_PASSWORD"]
        == "'pa\"ss\\word'"
    )
    assert 'pa"ss' not in result.stdout + result.stderr


def test_mail_wont_keep_one_users_password_for_another(box):
    mail(box, "pw")
    before = stored(box, "staging-mail.env")
    result = mail(box, "", user="other@filedrawer.dev")
    assert result.returncode == 1
    assert "--ask-all mail" in result.stderr
    assert stored(box, "staging-mail.env") == before


def test_mail_names_productions_stage_when_it_has_nothing_to_keep(box):
    result = mail(box, "", environment="production")
    assert result.returncode == 1
    assert "--ask-all production mail" in result.stderr
    assert not (box.root / "etc/filedrawer/production-mail.env").exists()


def test_the_github_token_keeps_the_stored_one(box):
    run(box, GITHUB, TOKEN="github_pat_kept")
    result = run(box, GITHUB, TOKEN="")
    assert result.returncode == 0, result.stderr
    assert stored(box, "github.env") == "FILEDRAWER_GITHUB_TOKEN='github_pat_kept'\n"
    assert "github_pat_kept" not in result.stdout + result.stderr


def test_the_github_token_refuses_an_empty_one_with_none_stored(box):
    result = run(box, GITHUB, TOKEN="")
    assert result.returncode == 1
    assert "--ask-all github" in result.stderr
    assert not (box.root / "etc/filedrawer/github.env").exists()


NEW_BACKUP = "https://hc-ping.com/1f6a0b2c-0000-4000-8000-000000000009"


@pytest.mark.parametrize(
    ("pings", "daily", "backup"),
    [
        ("\t", DAILY, BACKUP),
        (f"\t{NEW_BACKUP}", DAILY, NEW_BACKUP),
        ("", DAILY, BACKUP),
    ],
)
def test_ping_keeps_each_stored_url_sent_empty(box, pings, daily, backup):
    run(box, PING, PINGS=f"{DAILY}\t{BACKUP}")
    result = run(box, PING, PINGS=pings)
    assert result.returncode == 0, result.stderr
    assert settings(box, "production-ping.env") == {
        "FILEDRAWER_DAILY_EMAIL_PING": daily
    }
    assert settings(box, "production-backup.env") == {"BACKUP_PING": backup}
    assert DAILY not in result.stdout + result.stderr


@pytest.mark.parametrize("pings", [f"{DAILY}\t", f"\t{BACKUP}", "\t", ""])
def test_ping_refuses_to_keep_a_url_it_doesnt_have(box, pings):
    result = run(box, PING, PINGS=pings)
    assert result.returncode == 1
    assert "--ask-all production pings" in result.stderr
    assert not (box.root / "etc/filedrawer/production-ping.env").exists()


@pytest.mark.parametrize(
    ("script", "env"),
    [
        (SCREENING, {"ENVIRONMENT": "production", "API_KEY": "", "DAILY_USD": "1"}),
        (
            MAIL,
            {
                "ENVIRONMENT": "production",
                "SMTP_USER": "u@filedrawer.dev",
                "MAIL_FROM": "f",
                "SMTP_PASSWORD": "",
            },
        ),
        (GITHUB, {"TOKEN": ""}),
        (PING, {"PINGS": "\t"}),
    ],
)
def test_a_rerun_that_changes_nothing_restarts_nothing(box, script, env):
    """A default wizard rerun sends each kept secret empty; production mustn't
    restart for a file that came out the same."""
    units = box.root / "etc/systemd/system"
    units.mkdir(parents=True)
    (units / "filedrawer-production.service").write_text("")
    first = {
        **env,
        "API_KEY": API_KEY,
        "SMTP_PASSWORD": "pw",
        "TOKEN": "github_pat_x",
        "PINGS": f"{DAILY}\t{BACKUP}",
    }
    assert run(box, script, **{k: first[k] for k in env}).returncode == 0
    box.log.write_text("")
    result = run(box, script, **env)
    assert result.returncode == 0, result.stderr
    assert "restart" not in box.log.read_text()
    assert "unchanged" in result.stdout


def test_screening_keeps_the_stored_cap_sent_empty(box):
    """A rerun whose ops/.vps.env lacks the cap mustn't reset the server's."""
    run(box, SCREENING, ENVIRONMENT="staging", API_KEY=API_KEY, DAILY_USD="5")
    result = run(box, SCREENING, ENVIRONMENT="staging", API_KEY="", DAILY_USD="")
    assert result.returncode == 0, result.stderr
    assert (
        settings(box, "staging-screening.env")["FILEDRAWER_SCREENING_DAILY_USD"] == "5"
    )


def test_mail_keeps_the_stored_user_and_from_line_sent_empty(box):
    mail(box, "pw")
    before = stored(box, "staging-mail.env")
    result = run(
        box, MAIL, ENVIRONMENT="staging", SMTP_USER="", MAIL_FROM="", SMTP_PASSWORD=""
    )
    assert result.returncode == 0, result.stderr
    assert stored(box, "staging-mail.env") == before
