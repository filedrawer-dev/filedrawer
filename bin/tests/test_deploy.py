"""Tests for the deploy scripts that run on the VPS: ops/remote/filedrawer-receive (the
deploy key's forced command) and ops/remote/filedrawer-install (run as root through
sudo). They run as processes against a temp root, with fake sudo, runuser, systemctl,
systemd-run and curl first on PATH. bin/agent-pre-commit runs them with pytest.
"""

import fcntl
import fnmatch
import hashlib
import os
import re
import shutil
import stat
import subprocess
from pathlib import Path
from types import SimpleNamespace

import pytest

REPO = Path(__file__).resolve().parents[2]
RECEIVE = REPO / "ops/remote/filedrawer-receive"
INSTALL = REPO / "ops/remote/filedrawer-install"


def executable(path: Path, text: str) -> Path:
    path.write_text(text)
    path.chmod(path.stat().st_mode | stat.S_IXUSR)
    return path


@pytest.fixture
def box(tmp_path):
    """A fake server root, fake commands, and a log of what they were asked."""
    root = tmp_path / "root"
    state = root / "var/lib/filedrawer-deploy"
    bin_dir = root / "srv/filedrawer/staging/bin"
    state.mkdir(parents=True)
    bin_dir.mkdir(parents=True)
    (root / "srv/filedrawer/production/bin").mkdir(parents=True)
    fakes = tmp_path / "fakes"
    fakes.mkdir()
    log = tmp_path / "log"
    log.touch()
    # sudo records its arguments and the size of what it was sent on stdin.
    executable(
        fakes / "sudo",
        f'#!/bin/sh\necho "sudo $* <$(wc -c | tr -d " ") bytes" >> {log}\n',
    )
    # runuser records the user, then runs the command as the caller.
    executable(
        fakes / "runuser",
        f'#!/bin/sh\necho "runuser $*" >> {log}\nshift 3\nexec "$@"\n',
    )
    executable(fakes / "systemctl", f'#!/bin/sh\necho "systemctl $*" >> {log}\n')
    # systemd-run records its arguments, then runs the command after `--`.
    executable(
        fakes / "systemd-run",
        f'#!/bin/sh\necho "systemd-run $*" >> {log}\n'
        'while [ "$1" != -- ]; do shift; done\nshift\nexec "$@"\n',
    )
    # curl succeeds unless the file `unhealthy` exists.
    executable(
        fakes / "curl",
        f'#!/bin/sh\necho "curl $*" >> {log}\n[ ! -e {tmp_path}/unhealthy ]\n',
    )
    env = {
        "PATH": f"{fakes}:{os.environ['PATH']}",
        "FILEDRAWER_ROOT": str(root),
        "FILEDRAWER_HEALTH_SECONDS": "2",
    }
    return SimpleNamespace(
        tmp=tmp_path, root=root, state=state, bin=bin_dir, log=log, env=env
    )


def receive(box, command: str, data: bytes) -> subprocess.CompletedProcess:
    env = {**box.env, "SSH_ORIGINAL_COMMAND": command}
    return subprocess.run(
        [RECEIVE], input=data, env=env, capture_output=True, check=False
    )


def install(box, *args: str, data: bytes = b"", **env) -> subprocess.CompletedProcess:
    return subprocess.run(
        [INSTALL, *args],
        input=data,
        env={**box.env, **env},
        capture_output=True,
        check=False,
    )


def binary(version: str, runs: bool = True) -> bytes:
    """A stand-in server binary: a script that answers -h, or fails."""
    return f"#!/bin/sh\n# {version}\nexit {0 if runs else 1}\n".encode()


def sha(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


# filedrawer-receive


def test_receive_pipes_the_upload_to_install_with_its_hash(box):
    data = binary("v1")
    result = receive(box, f"deploy staging {sha(data)}", data)
    assert result.returncode == 0, result.stderr
    assert box.log.read_text() == (
        f"sudo -n /usr/local/sbin/filedrawer-install staging {sha(data)}"
        f" <{len(data)} bytes\n"
    )


@pytest.mark.parametrize(
    "command",
    [
        "",
        "deploy production " + "0" * 64,
        "deploy staging " + "0" * 63,
        "deploy staging " + "G" * 64,
        "deploy staging " + "0" * 64 + "; rm -rf /",
        "sh",
    ],
)
def test_receive_refuses_any_other_command(box, command):
    result = receive(box, command, b"x")
    assert result.returncode != 0
    assert b"refused" in result.stderr
    assert box.log.read_text() == ""


def test_receive_runs_one_deploy_at_a_time(box):
    data = binary("v1")
    with open(box.state / "lock", "w") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        result = receive(box, f"deploy staging {sha(data)}", data)
    assert result.returncode != 0
    assert b"another deploy" in result.stderr
    assert box.log.read_text() == ""


# filedrawer-install


def test_install_swaps_in_the_new_binary_and_keeps_the_old_one(box):
    executable(box.bin / "filedrawer", binary("v1").decode())
    data = binary("v2")
    result = install(box, "staging", sha(data), data=data)
    assert result.returncode == 0, result.stderr
    assert b"v2" in (box.bin / "filedrawer").read_bytes()
    assert b"v1" in (box.bin / "filedrawer.previous").read_bytes()
    log = box.log.read_text()
    assert "-p User=filedrawer " in log
    assert "systemctl restart filedrawer-staging" in log
    assert "http://127.0.0.1:8081/tags" in log


def test_install_to_production_checks_as_productions_user_on_its_port(box):
    prod = box.root / "srv/filedrawer/production/bin"
    executable(prod / "filedrawer", binary("v1").decode())
    data = binary("v2")
    result = install(box, "production", sha(data), data=data)
    assert result.returncode == 0, result.stderr
    assert b"v2" in (prod / "filedrawer").read_bytes()
    assert not (box.bin / "filedrawer").exists(), "staging is left alone"
    log = box.log.read_text()
    assert "-p User=filedrawer-prod " in log
    assert "systemctl restart filedrawer-production" in log
    assert "http://127.0.0.1:8080/tags" in log


@pytest.mark.parametrize("environment", ["staging", "production"])
def test_the_run_check_can_bind_no_socket_and_dies_with_its_unit(box, environment):
    """The check runs the uploaded code as the environment's user, so it runs in a
    transient unit that may bind no socket (it can't take the other environment's
    port) and whose children die with it, and can't see the environment's data."""
    data = binary("v2")
    result = install(box, environment, sha(data), data=data)
    assert result.returncode == 0, result.stderr
    [check] = [
        line for line in box.log.read_text().splitlines() if "systemd-run" in line
    ]
    for setting in (
        f"--unit=filedrawer-check-{environment}",
        "--wait",
        "-p SocketBindDeny=any",
        "-p PrivateNetwork=yes",
        "-p RuntimeMaxSec=10",
        "-p ProtectSystem=strict",
        f"-p InaccessiblePaths=-/srv/filedrawer/{environment}/data",
    ):
        assert setting in check, (setting, check)
    assert "runuser" not in box.log.read_text()


def test_install_refuses_a_hash_mismatch_and_changes_nothing(box):
    executable(box.bin / "filedrawer", binary("v1").decode())
    result = install(box, "staging", "0" * 64, data=binary("v2"))
    assert result.returncode != 0
    assert b"SHA-256" in result.stderr
    assert b"v1" in (box.bin / "filedrawer").read_bytes()
    assert sorted(p.name for p in box.bin.iterdir()) == ["filedrawer"]
    assert box.log.read_text() == ""


def test_install_refuses_an_upload_over_the_cap(box):
    data = b"x" * 101
    result = install(box, "staging", sha(data), data=data, FILEDRAWER_MAX_BYTES="100")
    assert result.returncode != 0
    assert b"over" in result.stderr
    assert box.log.read_text() == ""


def test_install_rolls_back_when_the_new_server_isnt_healthy(box):
    executable(box.bin / "filedrawer", binary("v1").decode())
    (box.tmp / "unhealthy").touch()
    data = binary("v2")
    result = install(box, "staging", sha(data), data=data)
    assert result.returncode != 0
    assert b"rolled back" in result.stderr
    assert b"v1" in (box.bin / "filedrawer").read_bytes()
    assert box.log.read_text().count("systemctl restart filedrawer-staging") == 2


def test_install_never_installs_a_binary_that_doesnt_run(box):
    executable(box.bin / "filedrawer", binary("v1").decode())
    data = binary("v2", runs=False)
    result = install(box, "staging", sha(data), data=data)
    assert result.returncode != 0
    assert b"doesn't run" in result.stderr
    assert b"journalctl -u filedrawer-check-staging" in result.stderr
    assert b"v1" in (box.bin / "filedrawer").read_bytes()
    assert "systemctl" not in box.log.read_text()


@pytest.mark.parametrize(
    "args",
    [
        ["prod", "0" * 64],
        ["../staging", "0" * 64],
        ["staging x", "0" * 64],
        ["staging", "not-a-hash"],
        ["staging"],
        ["staging", "0" * 64, "extra"],
    ],
)
def test_install_refuses_other_arguments(box, args):
    result = install(box, *args, data=binary("v2"))
    assert result.returncode != 0
    assert b"refused" in result.stderr
    assert not (box.bin / "filedrawer").exists()


def sudo_rule() -> str:
    """The command the deploy user's one sudo rule allows, as deploy-user.sh writes it."""
    script = (REPO / "ops/remote/deploy-user.sh").read_text()
    [rule] = re.findall(r"^deploy ALL=\(root\) NOPASSWD: (.+)$", script, re.M)
    return rule


def sudo_allows(command: str) -> bool:
    """Whether the rule allows `command`, matched as sudo matches a rule's arguments
    (sudoers(5), Wildcards): a rule's arguments that start with ^ and end with $ are a
    regular expression over the arguments joined by spaces; otherwise a shell-style
    wildcard, where `*` matches spaces too."""
    allowed_path, allowed_args = sudo_rule().split(" ", 1)
    path, args = command.split(" ", 1)
    if path != allowed_path:
        return False
    if allowed_args.startswith("^") and allowed_args.endswith("$"):
        return re.fullmatch(allowed_args[1:-1], args) is not None
    return fnmatch.fnmatchcase(args, allowed_args)


def test_the_deploy_key_can_reach_only_stagings_install():
    install_staging = f"/usr/local/sbin/filedrawer-install staging {'0' * 64}"
    assert sudo_allows(install_staging)
    for command in (
        f"/usr/local/sbin/filedrawer-install production {'0' * 64}",
        f"/usr/local/sbin/filedrawer-install  production {'0' * 64}",
        f"/usr/local/bin/filedrawer-install production {'0' * 64}",
        f"/usr/local/sbin/filedrawer-install staging {'0' * 64} production",
        f"/usr/local/sbin/filedrawer-install staging {'0' * 63}",
    ):
        assert not sudo_allows(command), command


@pytest.mark.skipif(shutil.which("visudo") is None, reason="no visudo")
def test_visudo_accepts_the_deploy_rule(tmp_path):
    rule = tmp_path / "rule"
    rule.write_text(f"deploy ALL=(root) NOPASSWD: {sudo_rule()}\n")
    result = subprocess.run(
        ["visudo", "-cf", rule], capture_output=True, text=True, check=False
    )
    assert result.returncode == 0, result.stdout + result.stderr


def test_install_refuses_a_third_argument_too(box):
    """The sudo rule refuses `staging <sha> production`; the install script's own
    argument check refuses it as well, for a rule written by an older deploy-user.sh."""
    data = binary("v2")
    result = install(box, "staging", sha(data), "production", data=data)
    assert result.returncode != 0
    assert b"refused" in result.stderr
    assert box.log.read_text() == ""


# The workflow and the wizard


def test_every_secret_and_variable_the_workflow_reads_is_one_the_ops_scripts_set():
    workflow = (REPO / ".github/workflows/ci.yml").read_text()
    # Every ops script, so a new one that sets a secret needs no line here.
    setters = "".join(
        path.read_text()
        for path in sorted((REPO / "ops").rglob("*"))
        if path.is_file() and os.access(path, os.X_OK)
    )
    read = set(re.findall(r"\$\{\{\s*(?:secrets|vars)\.(\w+)\s*\}\}", workflow))
    set_by_scripts = set(
        re.findall(
            r"(?:set_var|set_staging_secret|gh secret set|gh variable set)\s+(\w+)",
            setters,
        )
    )
    # ci_account ROLE SAVED SECRET WHAT stores SECRET.
    set_by_scripts |= set(re.findall(r"^\s*ci_account \w+ \w+ (\w+) ", setters, re.M))
    assert read, "the workflow reads no secrets or variables"
    assert read <= set_by_scripts, read - set_by_scripts


def test_the_settings_the_vps_scripts_write_are_the_ones_the_server_reads():
    written = set()
    for script in ("service.sh", "mail.sh", "screening.sh", "smoke-env.sh", "ping.sh"):
        text = (REPO / "ops/remote" / script).read_text()
        written |= set(re.findall(r"^\s*(?:printf ')?(FILEDRAWER_\w+)=", text, re.M))
    config = (REPO / "crates/server/src/config.rs").read_text()
    read = set(re.findall(r'"(FILEDRAWER_\w+)"', config))
    assert written <= read, written - read
    required = {
        "FILEDRAWER_DATABASE",
        "FILEDRAWER_LISTEN",
        "FILEDRAWER_ENVIRONMENT",
        "FILEDRAWER_SCREENER",
        "FILEDRAWER_PUBLIC_URL",
        "FILEDRAWER_FILES_URL",
        "FILEDRAWER_DAILY_EMAIL_PING",
    }
    assert required <= written, required - written


def workflow_jobs() -> dict[str, str]:
    """Each job in ci.yml, by name, as its text."""
    workflow = (REPO / ".github/workflows/ci.yml").read_text()
    jobs = workflow.split("\njobs:\n", 1)[1]
    parts = re.split(r"^  ([\w-]+):\n", jobs, flags=re.MULTILINE)
    return dict(zip(parts[1::2], parts[2::2]))


def test_a_job_that_reads_secrets_runs_only_githubs_own_actions():
    jobs = workflow_jobs()
    holding = {name: text for name, text in jobs.items() if "secrets." in text}
    assert {"deploy-staging", "smoke-staging", "screen-staging"} <= holding.keys(), (
        jobs.keys()
    )
    for name, text in holding.items():
        actions = re.findall(r"uses:\s*(\S+)", text)
        third_party = [a for a in actions if not a.startswith("actions/")]
        assert not third_party, f"{name} holds secrets and runs {third_party}"


def test_the_smoke_test_runs_after_the_deploy_and_waits_for_it():
    job = workflow_jobs()["smoke-staging"]
    assert re.search(r"^    needs: deploy-staging$", job, re.MULTILINE), job
    assert re.search(r"^    environment: staging$", job, re.MULTILINE), job
    assert re.search(
        r"ops/smoke-staging --wait \d+ https://staging\.filedrawer\.dev", job
    )


def test_the_screen_check_runs_after_the_deploy_as_the_ci_screen_account():
    job = workflow_jobs()["screen-staging"]
    assert re.search(r"^    needs: deploy-staging$", job, re.MULTILINE), job
    assert re.search(r"^    environment: staging$", job, re.MULTILINE), job
    assert "FILEDRAWER_STAGING_SCREEN_API_KEY: ${{ secrets.CI_SCREEN_TOKEN }}" in job, (
        job
    )
    assert re.search(
        r"ops/smoke-staging screen --wait \d+ https://staging\.filedrawer\.dev", job
    ), job


def step_script(job: str, step: str) -> str:
    """The `run: |` block of the step named `step` in a job's text, dedented."""
    after = job.split(f"- name: {step}\n", 1)[1]
    lines = after.split("run: |\n", 1)[1].splitlines()
    indent = len(lines[0]) - len(lines[0].lstrip())
    block = []
    for line in lines:
        if line.strip() and len(line) - len(line.lstrip()) < indent:
            break
        block.append(line[indent:])
    return "\n".join(block) + "\n"


@pytest.mark.parametrize(
    ("token", "status", "exits", "warns", "checked"),
    [
        ("", 0, 0, True, False),
        ("fdt_x", 0, 0, False, True),
        ("fdt_x", 1, 1, False, True),
        ("fdt_x", 3, 0, True, True),
    ],
    ids=["no token", "passed", "failed", "day spent"],
)
def test_the_screen_check_passes_with_a_warning_only_when_nothing_could_be_checked(
    tmp_path, token, status, exits, warns, checked
):
    script = step_script(
        workflow_jobs()["screen-staging"], "Screen with the real model"
    )
    fake = tmp_path / "ops/smoke-staging"
    fake.parent.mkdir()
    fake.write_text(f'#!/bin/sh\necho "checked $*"\nexit {status}\n')
    fake.chmod(0o755)
    env = {
        "PATH": os.environ["PATH"],
        "FILEDRAWER_STAGING_KEY": "k",
        "FILEDRAWER_STAGING_SCREEN_API_KEY": token,
    }
    # GitHub runs a step's script with bash -e.
    result = subprocess.run(
        ["bash", "-e", "-c", script],
        cwd=tmp_path,
        env=env,
        capture_output=True,
        text=True,
        check=False,
    )
    assert result.returncode == exits, result.stdout + result.stderr
    assert ("::warning::" in result.stdout) == warns, result.stdout
    assert ("checked screen" in result.stdout) == checked, result.stdout


def test_the_files_host_has_a_caddy_site_behind_the_staging_gate_and_a_dns_record():
    caddy = (REPO / "ops/remote/caddy.sh").read_text()
    sites = re.findall(r"^([a-z.]+\.filedrawer\.dev) \{\n(.*?)^\}", caddy, re.M | re.S)
    blocks = dict(sites)
    assert "files.staging.filedrawer.dev" in blocks, blocks.keys()
    for name in ("staging.filedrawer.dev", "files.staging.filedrawer.dev"):
        block = blocks[name]
        assert "X-Staging-Key" in block or "import staging_gate" in block, name
        assert "basic_auth" in block or "import staging_gate" in block, name
        assert (
            "reverse_proxy 127.0.0.1:8081" in block or "import staging_gate" in block
        ), name
    # service.sh's FILEDRAWER_FILES_URL for each environment: test_remote_environments.
    wizard = (REPO / "ops/provision-vps").read_text()
    dns = re.search(r"^records=\((.*?)\)$", wizard, re.M).group(1)
    assert '"files.staging.filedrawer.dev"' in dns


# deploy-user.sh

DEPLOY_USER = REPO / "ops/remote/deploy-user.sh"
PUBLIC_KEY = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIabc filedrawer-deploy@github-actions"


@pytest.fixture
def deploy_box(tmp_path):
    root = tmp_path / "root"
    (root / "etc/sudoers.d").mkdir(parents=True)
    (root / "home/deploy").mkdir(parents=True)
    fakes = tmp_path / "fakes"
    fakes.mkdir()
    log = tmp_path / "log"
    log.touch()
    for tool in ("useradd", "passwd", "chown", "visudo"):
        executable(fakes / tool, f'#!/bin/sh\necho "{tool} $*" >> {log}\n')
    executable(fakes / "id", "#!/bin/sh\nexit 0\n")
    env = {"PATH": f"{fakes}:{os.environ['PATH']}", "FILEDRAWER_ROOT": str(root)}

    def run(key: str) -> subprocess.CompletedProcess:
        return subprocess.run(
            ["bash", DEPLOY_USER],
            env={**env, "DEPLOY_PUBLIC_KEY": key},
            capture_output=True,
            text=True,
            check=False,
        )

    return SimpleNamespace(root=root, log=log, run=run)


def test_deploy_user_writes_the_key_and_the_sudo_rule(deploy_box):
    result = deploy_box.run(PUBLIC_KEY)
    assert result.returncode == 0, result.stderr
    keys = (deploy_box.root / "home/deploy/.ssh/authorized_keys").read_text()
    assert (
        keys == f'restrict,command="/usr/local/bin/filedrawer-receive" {PUBLIC_KEY}\n'
    )
    rule = (deploy_box.root / "etc/sudoers.d/filedrawer-deploy").read_text()
    assert rule == f"deploy ALL=(root) NOPASSWD: {sudo_rule()}\n"
    assert "visudo -cf" in deploy_box.log.read_text()


def test_deploy_user_with_no_key_keeps_the_key_and_rewrites_the_sudo_rule(deploy_box):
    """The wizard keeps the deploy key on a rerun, but a newer sudo rule must still
    reach the server."""
    assert deploy_box.run(PUBLIC_KEY).returncode == 0
    rule_file = deploy_box.root / "etc/sudoers.d/filedrawer-deploy"
    rule_file.chmod(0o640)
    rule_file.write_text(
        "deploy ALL=(root) NOPASSWD: /usr/local/sbin/filedrawer-install staging *\n"
    )
    result = deploy_box.run("")
    assert result.returncode == 0, result.stderr
    keys = (deploy_box.root / "home/deploy/.ssh/authorized_keys").read_text()
    assert PUBLIC_KEY in keys
    assert rule_file.read_text() == f"deploy ALL=(root) NOPASSWD: {sudo_rule()}\n"


def test_deploy_user_with_no_key_and_none_on_the_server_fails(deploy_box):
    result = deploy_box.run("")
    assert result.returncode != 0
    assert "no deploy key" in result.stderr
    assert not (deploy_box.root / "etc/sudoers.d/filedrawer-deploy").exists()


@pytest.mark.parametrize(
    "key", ["ssh-rsa AAAA", 'ssh-ed25519 AAAA x"y', "x\nssh-ed25519 AAAA"]
)
def test_deploy_user_refuses_what_isnt_one_ed25519_key(deploy_box, key):
    result = deploy_box.run(key)
    assert result.returncode != 0
    assert not (deploy_box.root / "home/deploy/.ssh/authorized_keys").exists()
