"""The sandbox ops/remote/admin-grant.sh and ops/remote/ci-account.sh run the binary in
(#117). A binary sent with staging's deploy key may be an attacker's, so each admin
call runs in a transient unit under the service's own sandbox, as ops/remote/service.sh
writes it, with no network of its own: nothing it starts outlives the call, and it
can't bind a port Caddy sends traffic to. Each script runs as a process against a temp
root, with fake systemd-run, systemctl and chown first on PATH.
"""

import os
import stat
import subprocess
from pathlib import Path
from types import SimpleNamespace

import fake_systemd_run
import pytest

REPO = Path(__file__).resolve().parents[2]
REMOTE = REPO / "ops/remote"

# The fake binary notes what its stdin is and answers each admin command.
FAKE_FILEDRAWER = """#!/bin/bash
echo "filedrawer $* stdin=$(readlink /proc/self/fd/0) db=$FILEDRAWER_DATABASE" >> "$LOG"
case "$2" in
  create-account) echo a-1 ;;
  create-token) echo fdt_1 ;;
  grant) echo "$4" ;;
esac
"""

# The [Service] settings of the unit that aren't its sandbox: how it starts and
# restarts, its settings files, and the port it may bind, which the admin call may not.
NOT_SANDBOX = {
    "ExecStart",
    "Restart",
    "RestartSec",
    "EnvironmentFile",
    "SocketBindAllow",
}

# Each script, the environment it acts on, and its settings.
CASES = [
    pytest.param(
        "admin-grant.sh", "staging", {"ADMIN_ACCOUNT": "a-1"}, id="grant-staging"
    ),
    pytest.param(
        "admin-grant.sh", "production", {"ADMIN_ACCOUNT": "a-1"}, id="grant-production"
    ),
    pytest.param("ci-account.sh", "staging", {"ROLE": "smoke"}, id="ci-account"),
]


def executable(path: Path, text: str) -> Path:
    path.write_text(text)
    path.chmod(path.stat().st_mode | stat.S_IXUSR)
    return path


@pytest.fixture
def box(tmp_path):
    root = tmp_path / "root"
    fakes = tmp_path / "fakes"
    fakes.mkdir()
    log = tmp_path / "log"
    log.touch()
    for tool in ("systemctl", "chown"):
        executable(fakes / tool, "#!/bin/sh\n")
    # A runuser that runs anything would hide a call outside the unit.
    executable(fakes / "runuser", f'#!/bin/sh\necho "runuser $*" >> {log}\nexit 1\n')
    fake_systemd_run.install(fakes, log)
    env = {
        "PATH": f"{fakes}:{os.environ['PATH']}",
        "FILEDRAWER_ROOT": str(root),
        "LOG": str(log),
    }
    for environment in ("staging", "production"):
        bin_dir = root / f"srv/filedrawer/{environment}/bin"
        bin_dir.mkdir(parents=True)
        executable(bin_dir / "filedrawer", FAKE_FILEDRAWER)
        subprocess.run(
            ["bash", REMOTE / "service.sh"],
            env={**env, "ENVIRONMENT": environment},
            capture_output=True,
            check=True,
        )
    return SimpleNamespace(root=root, log=log, env=env)


def run(box, script, environment, settings) -> subprocess.CompletedProcess:
    """SCRIPT as ops/provision-vps runs it: read by `bash -s` from stdin."""
    return subprocess.run(
        ["bash", "-s"],
        stdin=(REMOTE / script).open(),
        env={**box.env, "ENVIRONMENT": environment, **settings},
        capture_output=True,
        text=True,
        check=False,
    )


def service_sandbox(box, environment) -> list[str]:
    """The sandbox lines of the [Service] section service.sh wrote for ENVIRONMENT."""
    unit = box.root / f"etc/systemd/system/filedrawer-{environment}.service"
    lines, section = [], ""
    for line in unit.read_text().splitlines():
        if line.startswith("["):
            section = line
        elif (
            section == "[Service]"
            and line
            and not line.startswith("#")
            and line.split("=", 1)[0] not in NOT_SANDBOX
        ):
            lines.append(line)
    return lines


@pytest.mark.parametrize(("script", "environment", "settings"), CASES)
def test_each_admin_call_runs_in_a_unit_under_the_services_sandbox(
    box, script, environment, settings
):
    result = run(box, script, environment, settings)
    assert result.returncode == 0, result.stderr
    calls = fake_systemd_run.calls(box.log)
    assert calls
    sandbox = service_sandbox(box, environment)
    assert any(line.startswith("User=") for line in sandbox)
    for call in calls:
        name = fake_systemd_run.command(call)[2]
        assert f"--unit=filedrawer-admin-{environment}-{name}" in call
        assert {"--wait", "--collect", "--pipe", "--quiet"} <= set(call)
        properties = fake_systemd_run.properties(call)
        assert [line for line in sandbox if line not in properties] == []
        bin_path = box.root / f"srv/filedrawer/{environment}/bin/filedrawer"
        assert fake_systemd_run.command(call)[:2] == [str(bin_path), "admin"]


@pytest.mark.parametrize(("script", "environment", "settings"), CASES)
def test_an_admin_call_has_no_network_binds_nothing_and_ends(
    box, script, environment, settings
):
    run(box, script, environment, settings)
    calls = fake_systemd_run.calls(box.log)
    assert calls
    for call in calls:
        properties = fake_systemd_run.properties(call)
        assert "PrivateNetwork=yes" in properties
        assert "SocketBindDeny=any" in properties
        assert not [p for p in properties if p.startswith("SocketBindAllow")]
        assert [p for p in properties if p.startswith("RuntimeMaxSec=")]


@pytest.mark.parametrize(("script", "environment", "settings"), CASES)
def test_an_admin_call_gets_only_the_database_and_never_the_scripts_stdin(
    box, script, environment, settings
):
    """The script itself arrives on bash's stdin, so a call that read it would eat the
    rest of the script."""
    result = run(box, script, environment, settings)
    assert result.returncode == 0, result.stderr
    database = box.root / f"srv/filedrawer/{environment}/data/filedrawer.db"
    log = box.log.read_text()
    assert f"stdin=/dev/null db={database}" in log
    assert "runuser" not in log
    for call in fake_systemd_run.calls(box.log):
        assert f"Environment=FILEDRAWER_DATABASE={database}" in (
            fake_systemd_run.properties(call)
        )


@pytest.mark.parametrize(("script", "environment", "settings"), CASES)
def test_a_unit_that_fails_names_its_journal(box, script, environment, settings):
    """--quiet keeps systemd's own failures, such as a timeout (255) or a sandbox the
    binary can't start in (226), out of stderr, where the wizard says to look."""
    result = run(box, script, environment, {**settings, "SYSTEMD_RUN_EXIT": "226"})
    assert result.returncode != 0
    assert result.stdout == ""
    [call] = fake_systemd_run.calls(box.log)
    name = fake_systemd_run.command(call)[2]
    assert (
        f"filedrawer admin {name} failed (exit 226; "
        f"journalctl -u filedrawer-admin-{environment}-{name} says why)"
    ) in result.stderr


def admin_function(script: str) -> str:
    """SCRIPT's admin function, from its comment to its closing brace."""
    text = (REMOTE / script).read_text()
    start = text.index("# admin ARGS...")
    return text[start : text.index("\n}\n", start) + 2]


def test_both_scripts_hold_the_same_admin_function():
    """Each script goes to the server alone, so neither can source the other's; what
    differs is only the comment's pointer to the other script."""
    grant = admin_function("admin-grant.sh").replace("ci-account.sh", "OTHER")
    ci = admin_function("ci-account.sh").replace("admin-grant.sh", "OTHER")
    assert grant == ci


def test_ci_account_makes_the_account_and_its_token_in_two_calls(box):
    result = run(box, "ci-account.sh", "staging", {"ROLE": "smoke"})
    assert result.stdout == "a-1\tfdt_1\n"
    commands = [
        fake_systemd_run.command(c)[1:3] for c in fake_systemd_run.calls(box.log)
    ]
    assert commands == [["admin", "create-account"], ["admin", "create-token"]]
