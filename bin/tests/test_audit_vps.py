"""ops/audit-vps and the facts script it sends, ops/remote/audit-facts.sh: one ssh
round, the drift of the server's root-owned scripts from origin/main, and no secret
in the report."""

import hashlib
import os
import shutil
import subprocess
from pathlib import Path
from types import SimpleNamespace

import pytest

REPO = Path(__file__).resolve().parents[2]
AUDIT = REPO / "ops/audit-vps"
FACTS = REPO / "ops/remote/audit-facts.sh"
INSTALL = "/usr/local/sbin/filedrawer-install"


def git(cwd: Path, *args: str) -> str:
    return subprocess.run(
        ["git", "-c", "user.name=t", "-c", "user.email=t@example.com", *args],
        cwd=cwd,
        check=True,
        capture_output=True,
        text=True,
    ).stdout.strip()


def sha256(text: str) -> str:
    return hashlib.sha256(text.encode()).hexdigest()


@pytest.fixture
def audit(tmp_path, fake_host):
    """A checkout at origin/main whose ops/remote/filedrawer-install has two
    revisions, `old` then `new`, and a server that answers the facts round with
    whatever `state.facts` holds."""
    origin = tmp_path / "origin.git"
    git(tmp_path, "init", "-q", "--bare", "-b", "main", str(origin))
    repo = tmp_path / "repo"
    git(tmp_path, "clone", "-q", str(origin), str(repo))
    git(repo, "checkout", "-q", "-b", "main")
    (repo / "bin").mkdir()
    (repo / "bin/caddy").write_text('VERSION = "2.10.2"\n')
    script = repo / "ops/remote/filedrawer-install"
    script.parent.mkdir(parents=True)
    script.write_text("old\n")
    git(repo, "add", "-A")
    git(repo, "commit", "-q", "-m", "old")
    old = git(repo, "rev-parse", "--short", "HEAD")
    script.write_text("new\n")
    git(repo, "commit", "-q", "-am", "new")
    git(repo, "push", "-q", "-u", "origin", "main")

    ops = tmp_path / "ops"
    ops.mkdir()
    (ops / ".vps.env").write_text("VPS_HOST='vps.example'\nSTAGING_KEY='the-key'\n")
    state = SimpleNamespace(host=fake_host, ops=ops, old=old, facts="")

    def run() -> subprocess.CompletedProcess:
        fake_host.answer("bash -s", stdout=state.facts)
        return subprocess.run(
            ["bash", str(AUDIT)],
            env={
                **fake_host.env,
                "FILEDRAWER_OPS_DIR": str(ops),
                "FILEDRAWER_REPO": str(repo),
            },
            capture_output=True,
            text=True,
            check=False,
            stdin=subprocess.DEVNULL,
        )

    state.run = run
    return state


def test_the_facts_come_from_one_ssh_round_running_the_facts_script(audit):
    audit.facts = "\n== system ==\n12.9\n"
    result = audit.run()
    assert result.returncode == 0, result.stderr
    calls = [
        line
        for line in audit.host.log.read_text().splitlines()
        if line.startswith("ssh ")
    ]
    assert calls == ["ssh bash -s"]
    assert audit.host.sent(1) == FACTS.read_text()
    assert "== system ==\n12.9\n" in result.stdout


@pytest.mark.parametrize(
    ("served", "verdict"),
    [
        ("new\n", "current"),
        ("old\n", "behind: origin/main's as of {old}"),
        ("edited by hand\n", "MATCHES NO REVISION on origin/main"),
    ],
)
def test_a_servers_script_is_current_behind_or_no_revision_of_mains(
    audit, served, verdict
):
    audit.facts = f"sha256 {INSTALL} {sha256(served)}\n"
    result = audit.run()
    assert result.returncode == 0, result.stderr
    drift = result.stdout.split("== drift from origin/main ==")[1]
    assert f"{INSTALL}: {verdict.format(old=audit.old)}" in drift


def test_a_script_the_server_lacks_is_called_absent(audit):
    audit.facts = f"sha256 {INSTALL} none\n"
    assert f"{INSTALL}: absent from the server" in audit.run().stdout


def test_the_report_names_the_pinned_caddy_and_the_env_files_keys_not_values(audit):
    result = audit.run()
    assert "Caddy pinned by bin/caddy on origin/main: 2.10.2" in result.stdout
    assert "VPS_HOST STAGING_KEY" in result.stdout
    assert "the-key" not in result.stdout


def test_no_vps_host_is_refused_before_any_ssh(audit):
    (audit.ops / ".vps.env").write_text("")
    result = audit.run()
    assert result.returncode == 1
    assert "no VPS_HOST" in result.stderr
    assert "ssh " not in audit.host.log.read_text()


def test_a_failed_ssh_fails_and_says_so(audit):
    audit.host.answer("bash -s", status=255)
    result = subprocess.run(
        ["bash", str(AUDIT)],
        env={
            **audit.host.env,
            "FILEDRAWER_OPS_DIR": str(audit.ops),
            "FILEDRAWER_REPO": str(audit.ops.parent / "repo"),
        },
        capture_output=True,
        text=True,
        check=False,
        stdin=subprocess.DEVNULL,
    )
    assert result.returncode == 1
    assert "ssh root@vps.example failed" in result.stderr
    assert "audit-vps report" not in result.stdout


# What the facts script reads files with, and what a command run as another user is.
PLAIN_TOOLS = (
    "bash cat cut find grep ls paste sed sh sha256sum stat tail test timeout true uname"
).split()
SECRET = "hunter2-do-not-print"
KEY_BLOB = "AAAAC3NzaC1lZDI1NTE5AAAAIPretendKeyMaterial0123456789abcdefghijk"
PASSWORD_HASH = "$2a$14$PretendBcryptHashOfStagingsPassword0123456789abcdefghi"


@pytest.fixture
def server(tmp_path):
    """A fake root holding the files the facts script reads. PATH holds only the
    fakes the tests need and the plain tools the script reads files with, so no
    test asks this machine's own ss, systemctl, sudo or apt for anything."""
    root = tmp_path / "root"
    env_dir = root / "etc/filedrawer"
    env_dir.mkdir(parents=True)
    (env_dir / "staging.env").write_text(
        f"FILEDRAWER_ANTHROPIC_API_KEY={SECRET}\nexport FILEDRAWER_LISTEN=0.0.0.0:9999\n"
    )
    sites = root / "etc/caddy/sites"
    sites.mkdir(parents=True)
    (sites / "staging.caddy").write_text(f"header X-Staging-Key {SECRET}\n")
    # The older layout kept staging's site, with its password's hash, in this file.
    (root / "etc/caddy/Caddyfile").write_text(
        "{\n\tadmin off\n}\nimport sites/*.caddy\n"
        f"staging.example {{\n\tbasic_auth {{\n\t\tadmin {PASSWORD_HASH}\n\t}}\n}}\n"
    )
    ssh_dir = root / "home/deploy/.ssh"
    ssh_dir.mkdir(parents=True)
    (ssh_dir / "authorized_keys").write_text(
        f'restrict,command="/usr/local/bin/filedrawer-receive" ssh-ed25519 {KEY_BLOB} ci\n'
        f"ssh-ed25519\t{KEY_BLOB}\ttabbed\n"
    )
    units = root / "etc/systemd/system"
    units.mkdir(parents=True)
    (units / "filedrawer-staging.service").write_text("[Service]\nUser=filedrawer\n")
    database = root / "srv/filedrawer/production/data/filedrawer.db"
    database.parent.mkdir(parents=True)
    database.write_text("rows\n")
    sbin = root / "usr/local/sbin"
    sbin.mkdir(parents=True)
    (sbin / "filedrawer-install").write_text("the install script\n")
    (root / "etc/passwd").write_text(
        "root:x:0:0:root:/root:/bin/bash\nwww:x:33:33::/var/www:/usr/sbin/nologin\n"
    )
    fakes = tmp_path / "fakes"
    fakes.mkdir()
    for tool in PLAIN_TOOLS:
        (fakes / tool).symlink_to(shutil.which(tool))

    def fake(name: str, body: str) -> None:
        path = fakes / name
        path.write_text(f"#!/bin/sh\n{body}\n")
        path.chmod(0o755)

    # A ps asked for arguments shows the secret one of them holds.
    fake(
        "ps",
        'case "$*" in *args*|*cmd*|*-f*|*aux*) '
        f"echo 'root sshd --password={SECRET}' ;; *) echo 'root sshd' ;; esac",
    )
    fake("caddy", "echo 'v2.11.9 h1:abc='")
    fake("visudo", "echo 'broken rule' >&2; exit 1")
    fake("runuser", "exit 1")

    def run(**fakes_for_this_run: str) -> subprocess.CompletedProcess:
        for name, body in fakes_for_this_run.items():
            fake(name, body)
        return subprocess.run(
            ["bash", str(FACTS)],
            env={"PATH": str(fakes), "FILEDRAWER_ROOT": str(root)},
            capture_output=True,
            text=True,
            check=False,
        )

    return SimpleNamespace(root=root, run=run)


def test_an_env_file_shows_its_key_names_and_no_value(server):
    out = server.run().stdout
    assert "keys: FILEDRAWER_ANTHROPIC_API_KEY FILEDRAWER_LISTEN" in out
    assert "lines: 2" in out
    assert "0.0.0.0:9999" not in out


def test_a_caddy_site_file_shows_only_its_owner_and_mode(server):
    out = server.run().stdout
    assert " /etc/caddy/sites/staging.caddy\n" in out
    assert "X-Staging-Key" not in out
    assert "2:\tadmin off\n" in out, "the Caddyfile's global options show"
    assert "4:import sites/*.caddy\n" in out


def test_a_password_hash_in_the_caddyfile_isnt_printed_whatever_the_user_is_called(
    server,
):
    assert PASSWORD_HASH not in server.run().stdout


def test_an_authorized_keys_line_shows_its_options_without_the_key(server):
    out = server.run().stdout
    assert (
        'restrict,command="/usr/local/bin/filedrawer-receive" ssh-ed25519 <key> ci'
        in out
    )
    assert "ssh-ed25519 <key>\ttabbed" in out
    assert KEY_BLOB not in out


def test_each_installed_script_is_hashed_or_none(server):
    out = server.run().stdout
    assert f"sha256 {INSTALL} {sha256('the install script' + chr(10))}\n" in out
    assert "sha256 /usr/local/bin/filedrawer-receive none\n" in out


def test_each_generated_unit_and_config_is_hashed_with_its_mode(server):
    out = server.run().stdout
    unit = "/etc/systemd/system/filedrawer-staging.service"
    assert (
        f"sha256 {unit} {sha256('[Service]' + chr(10) + 'User=filedrawer' + chr(10))}\n"
        in out
    )
    assert f" {unit}\n" in out
    assert "sha256 /etc/filedrawer/loopback.nft none\n" in out


def test_a_probe_that_cant_run_as_the_user_says_so_and_claims_nothing(server):
    out = server.run().stdout
    assert "filedrawer -> 127.0.0.1:2019: (can't run as filedrawer)\n" in out
    database = "/srv/filedrawer/production/data/filedrawer.db"
    assert f"filedrawer reading {database}: (can't run as filedrawer)\n" in out
    assert "can't read" not in out
    assert "no connection" not in out


def test_a_service_user_that_reads_the_other_environment_is_named(server):
    # A runuser that runs the command as the test's own user, who can read the file.
    out = server.run(runuser='shift 3; exec "$@"').stdout
    assert "filedrawer CAN read /srv/filedrawer/production/data/filedrawer.db\n" in out
    assert (
        "filedrawer-prod can't read /srv/filedrawer/staging/data/filedrawer.db\n" in out
    )


def test_only_accounts_with_a_login_shell_are_listed(server):
    users = server.run().stdout.split("== users ==")[1].split("== sudo ==")[0]
    assert "root:0:0:/root:/bin/bash" in users
    assert "www" not in users


def test_a_missing_or_failing_command_is_reported_and_the_rest_still_prints(server):
    result = server.run()
    assert result.returncode == 0, result.stderr
    out = result.stdout
    assert "$ ufw status verbose\n(not found)\n" in out
    assert "$ visudo -c\nbroken rule\n(exit 1)\n" in out
    assert "$ caddy version\nv2.11.9 h1:abc=\n" in out
    assert out.rstrip().splitlines()[-1] != "", "the last section printed"
    assert "== journal ==" in out


def test_nothing_the_server_holds_leaves_as_a_secret_or_changes(server):
    def snapshot() -> dict[str, tuple[bytes, int]]:
        return {
            str(path.relative_to(server.root)): (path.read_bytes(), path.stat().st_mode)
            for path in sorted(server.root.rglob("*"))
            if path.is_file()
        }

    before = snapshot()
    result = server.run()
    assert SECRET not in result.stdout + result.stderr
    assert snapshot() == before
    assert sorted(os.listdir(server.root)) == ["etc", "home", "srv", "usr"]


def test_every_part_the_help_promises_has_a_heading(server):
    out = server.run().stdout
    for heading in (
        "system",
        "listening sockets",
        "firewall",
        "loopback reach, by user",
        "sshd",
        "authorized_keys",
        "users",
        "sudo",
        "installed scripts",
        "installed units and configs: hashes, owners and modes",
        "secrets: owners, modes and key names, never values",
        "caddy",
        "data and binaries",
        "units",
        "updates",
        "processes: user and name",
        "journal",
    ):
        assert f"\n== {heading} ==\n" in out, heading
