"""Tests for ops/deploy-production (#42), which the owner runs on their own machine to
promote the binary staging runs to production. It runs as a process against a temp
git repo with an origin, conftest's fake ssh and curl, a fake gh of its own that
serves GitHub's API from a dict and `gh run download` from a file, and a fake
ops/smoke-staging. Answers to its questions go on stdin.
"""

import hashlib
import json
import shutil
import stat
import subprocess
from pathlib import Path
from types import SimpleNamespace

import pytest

REPO = Path(__file__).resolve().parents[2]
SCRIPT = REPO / "ops/deploy-production"
FACTS = REPO / "ops/remote/deploy-facts.sh"
GH_REPO = "repos/filedrawer-dev/filedrawer"


def executable(path: Path, text: str) -> Path:
    path.write_text(text)
    path.chmod(path.stat().st_mode | stat.S_IXUSR)
    return path


def git(cwd: Path, *args: str) -> str:
    return subprocess.run(
        ["git", "-c", "user.name=t", "-c", "user.email=t@t", "-C", str(cwd), *args],
        check=True,
        capture_output=True,
        text=True,
    ).stdout.strip()


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


# The fake gh: `gh api PATH` prints api[PATH] as JSON; `gh run download ID ...
# --dir DIR` copies binaries[ID] to DIR/filedrawer; `gh auth status` succeeds unless
# `logged-out` exists. Every call is a line in `log`.
FAKE_GH = """#!/usr/bin/env python3
import json, shutil, sys
from pathlib import Path
root = Path({root!r})
args = sys.argv[1:]
with (root / "log").open("a") as log:
    log.write(" ".join(args) + "\\n")
if args[:2] == ["auth", "status"]:
    sys.exit(1 if (root / "logged-out").exists() else 0)
if args[0] == "api":
    path = next(a for a in args[1:] if not a.startswith("-"))
    api = json.loads((root / "api.json").read_text())
    if path not in api:
        print(f"gh: Not Found (HTTP 404) {{path}}", file=sys.stderr)
        sys.exit(1)
    print(json.dumps(api[path]))
    sys.exit(0)
if args[:2] == ["run", "download"]:
    run = args[2]
    out = Path(args[args.index("--dir") + 1])
    out.mkdir(parents=True, exist_ok=True)
    shutil.copy(root / f"binary-{{run}}", out / "filedrawer")
    sys.exit(0)
sys.exit(2)
"""


@pytest.fixture
def deploy(tmp_path, fake_host):
    """A checkout at origin/main with two commits, `old` (migrations to 0031) and
    `new` (0032 added); a GitHub whose push run 100 for `new` deployed and passed its
    smoke test; staging running `new`; and production running `old` at schema 31."""
    origin = tmp_path / "origin.git"
    git(tmp_path, "init", "-q", "--bare", "-b", "main", str(origin))
    repo = tmp_path / "repo"
    git(tmp_path, "clone", "-q", str(origin), str(repo))
    git(repo, "checkout", "-q", "-b", "main")
    (repo / "bin").mkdir()
    (repo / "bin/caddy").write_text('VERSION = "2.10.2"\n')
    migrations = repo / "crates/server/migrations"
    migrations.mkdir(parents=True)
    (migrations / "0001_initial.sql").write_text("CREATE TABLE a (x);\n")
    (migrations / "0031_last.sql").write_text("CREATE TABLE b (x);\n")
    (repo / "crates/server/migration-rollback.txt").write_text("# none\n")
    (repo / "ops/remote").mkdir(parents=True)
    for name in ("filedrawer-install", "filedrawer-backup"):
        shutil.copy(REPO / "ops/remote" / name, repo / "ops/remote" / name)
    git(repo, "add", "-A")
    git(repo, "commit", "-q", "-m", "old")
    old = git(repo, "rev-parse", "HEAD")
    (migrations / "0032_next.sql").write_text("ALTER TABLE b ADD COLUMN y TEXT;\n")
    git(repo, "add", "-A")
    git(repo, "commit", "-q", "-m", "new")
    new = git(repo, "rev-parse", "HEAD")
    git(repo, "push", "-q", "-u", "origin", "main")

    ops = tmp_path / "ops"
    ops.mkdir()
    (ops / ".vps.env").write_text("VPS_HOST='vps.example'\nSTAGING_KEY='the-key'\n")

    gh_root = tmp_path / "gh"
    gh_root.mkdir()
    (gh_root / "log").touch()
    fakes = tmp_path / "fakes"
    fakes.mkdir()
    executable(fakes / "gh", FAKE_GH.format(root=str(gh_root)))
    smoke_log = tmp_path / "smoke.log"
    executable(
        fakes / "smoke-staging",
        f'#!/bin/sh\necho "$*" >> {smoke_log}\n'
        f'[ -e {tmp_path}/smoke-fails ] && {{ echo "FAIL something"; exit 1; }}\n'
        'echo "read: 9 passed, 0 failed"\n',
    )

    install = (repo / "ops/remote/filedrawer-install").read_bytes()
    backup = (repo / "ops/remote/filedrawer-backup").read_bytes()
    state = SimpleNamespace(
        tmp=tmp_path,
        repo=repo,
        old=old,
        new=new,
        host=fake_host,
        gh=gh_root,
        smoke_log=smoke_log,
        api={},
        caddy="v2.10.2 h1:abc=",
        schema="31",
        production=old,
        install_sha=sha256(install),
        backup_sha=sha256(backup),
    )

    def run_for(commit: str, run_id: int, **overrides) -> None:
        """A push run of ci.yml on main for `commit`, its jobs and its artifact."""
        run = {
            "id": run_id,
            "head_sha": commit,
            "head_branch": "main",
            "event": "push",
            "path": ".github/workflows/ci.yml",
            "conclusion": "success",
            **overrides.get("run", {}),
        }
        runs = state.api.setdefault(
            f"{GH_REPO}/actions/runs?head_sha={commit}&per_page=100",
            {"workflow_runs": []},
        )
        runs["workflow_runs"].append(run)
        state.api[f"{GH_REPO}/actions/runs/{run_id}/jobs?per_page=100"] = {
            "jobs": overrides.get(
                "jobs",
                [
                    {"name": "check", "conclusion": "success"},
                    {"name": "deploy-staging", "conclusion": "success"},
                    {"name": "smoke-staging", "conclusion": "success"},
                ],
            )
        }
        state.api[f"{GH_REPO}/actions/runs/{run_id}/artifacts"] = {
            "artifacts": [
                {
                    "name": "filedrawer",
                    "expired": False,
                    "expires_at": "2027-02-08T00:00:00Z",
                    **overrides.get("artifact", {}),
                }
            ]
        }
        binary = overrides.get(
            "binary", f"ELF...filedrawer-commit {commit}...".encode()
        )
        (gh_root / f"binary-{run_id}").write_bytes(binary)

    state.run_for = run_for
    run_for(new, 100)
    run_for(old, 90)

    def staging_runs(commit: str) -> None:
        fake_host.answer(
            "*staging.filedrawer.dev/skill.md*",
            stdout=f"HTTP/2 200\r\nx-filedrawer-commit: {commit}\r\n\r\n",
            tool="curl",
        )

    state.staging_runs = staging_runs
    staging_runs(new)

    def facts() -> str:
        binary = f"filedrawer-commit {state.production}" if state.production else "none"
        return (
            f"caddy {state.caddy}\nschema {state.schema}\nbinary {binary}\n"
            f"install {state.install_sha}\nbackup {state.backup_sha}\n"
        )

    def run(*args: str, answers: str = "") -> subprocess.CompletedProcess:
        (gh_root / "api.json").write_text(json.dumps(state.api))
        fake_host.answer("bash -s", stdout=facts())
        env = {
            **fake_host.env,
            "PATH": f"{fakes}:{fake_host.env['PATH']}",
            "FILEDRAWER_REPO": str(repo),
            "FILEDRAWER_OPS_DIR": str(ops),
            "FILEDRAWER_SMOKE_STAGING": str(fakes / "smoke-staging"),
        }
        env.pop("FILEDRAWER_STAGING_KEY", None)
        return subprocess.run(
            [str(SCRIPT), *args],
            input=answers,
            env=env,
            capture_output=True,
            text=True,
            check=False,
        )

    state.run = run
    return state


def installs(deploy) -> list[str]:
    return [c for c in deploy.host.calls() if "filedrawer-install production" in c]


def sent_binary(deploy) -> bytes:
    [n] = [
        i + 1
        for i, call in enumerate(deploy.host.calls())
        if "filedrawer-install production" in call
    ]
    return deploy.host.sent(n).encode()


def test_it_promotes_the_commit_staging_runs(deploy):
    result = deploy.run()
    assert result.returncode == 0, result.stdout + result.stderr
    binary = (deploy.gh / "binary-100").read_bytes()
    assert installs(deploy) == [
        f"/usr/local/sbin/filedrawer-install production {sha256(binary)} --expect-schema 31"
    ]
    assert sent_binary(deploy) == binary
    assert deploy.smoke_log.read_text().split() == [
        "read",
        "--wait",
        "120",
        "--commit",
        deploy.new,
        "https://filedrawer.dev",
    ]
    assert f"deployed {deploy.new}" in result.stdout
    # The staging key went to curl in a config on stdin, never on a command line.
    assert not any("the-key" in call for call in deploy.host.calls("curl"))


def test_the_facts_come_from_one_ssh_round_running_the_facts_script(deploy):
    deploy.run()
    calls = deploy.host.calls()
    assert calls.index("bash -s") < calls.index(installs(deploy)[0])
    assert deploy.host.sent(calls.index("bash -s") + 1) == FACTS.read_text()


def test_a_checkout_behind_origin_is_refused(deploy):
    git(deploy.repo, "reset", "-q", "--hard", deploy.old)
    result = deploy.run()
    assert result.returncode != 0
    assert "pull" in result.stderr
    assert installs(deploy) == []


def test_a_prefix_resolves_to_one_commit_on_main(deploy):
    result = deploy.run(deploy.new[:7])
    assert result.returncode == 0, result.stdout + result.stderr
    assert "the commit staging runs" in result.stdout


@pytest.mark.parametrize("arg", ["abc", "HEAD~1", "main", "0000000", "A" * 7])
def test_anything_but_a_prefix_of_one_commit_on_main_is_refused(deploy, arg):
    result = deploy.run(arg)
    assert result.returncode != 0
    assert "on origin/main" in result.stderr
    assert installs(deploy) == []


def test_another_commit_than_stagings_asks_first(deploy):
    result = deploy.run(deploy.old, answers="n\n")
    assert result.returncode != 0
    assert "not the one staging runs" in result.stdout
    assert installs(deploy) == []


def test_a_rollback_to_an_older_commit_deploys_once_asked(deploy):
    deploy.production = deploy.new
    deploy.schema = "31"
    result = deploy.run(deploy.old, answers="y\n")
    assert result.returncode == 0, result.stdout + result.stderr
    binary = (deploy.gh / "binary-90").read_bytes()
    assert sent_binary(deploy) == binary


@pytest.mark.parametrize(
    "overrides",
    [
        {"run": {"event": "pull_request"}},
        {"run": {"head_branch": "feature"}},
        {"run": {"path": ".github/workflows/mutants.yml"}},
        {"run": {"head_sha": "f" * 40}},
    ],
)
def test_only_a_push_run_of_ci_on_main_for_the_commit_counts(deploy, overrides):
    deploy.api.clear()
    deploy.run_for(deploy.new, 100, **overrides)
    result = deploy.run()
    assert result.returncode != 0
    assert "no push run" in result.stderr
    assert installs(deploy) == []


@pytest.mark.parametrize(
    "jobs",
    [
        [{"name": "deploy-staging", "conclusion": "failure"}],
        [
            {"name": "deploy-staging", "conclusion": "success"},
            {"name": "smoke-staging", "conclusion": "cancelled"},
        ],
        [{"name": "deploy-staging", "conclusion": "success"}],
    ],
)
def test_a_run_whose_deploy_or_smoke_didnt_pass_is_refused(deploy, jobs):
    deploy.api.clear()
    deploy.run_for(deploy.new, 100, jobs=jobs)
    result = deploy.run()
    assert result.returncode != 0
    assert "deploy-staging" in result.stderr or "smoke-staging" in result.stderr
    assert installs(deploy) == []


def test_an_expired_artifact_says_when_and_what_to_do(deploy):
    deploy.api.clear()
    deploy.run_for(
        deploy.new,
        100,
        artifact={"expired": True, "expires_at": "2026-08-01T00:00:00Z"},
    )
    result = deploy.run()
    assert result.returncode != 0
    assert "expired" in result.stderr
    assert "2026-08-01" in result.stderr
    assert "push a new commit" in result.stderr
    assert "run download" not in (deploy.gh / "log").read_text()


def test_a_binary_that_doesnt_name_its_commit_is_refused(deploy):
    deploy.api.clear()
    deploy.run_for(deploy.new, 100, binary=b"ELF... no commit here")
    result = deploy.run()
    assert result.returncode != 0
    assert "doesn't name" in result.stderr
    assert installs(deploy) == []


def test_gh_logged_out_is_refused_first(deploy):
    (deploy.gh / "logged-out").touch()
    result = deploy.run()
    assert result.returncode != 0
    assert "gh auth login" in result.stderr
    assert deploy.host.calls() == []


@pytest.mark.parametrize(
    "header", ["unknown", "f" * 40, None], ids=["unknown", "not-on-main", "none"]
)
def test_staging_that_names_no_commit_on_main_is_refused(deploy, header):
    if header is None:
        deploy.host.answer(
            "*staging.filedrawer.dev/skill.md*",
            stdout="HTTP/2 401\r\n\r\n",
            tool="curl",
        )
    else:
        deploy.staging_runs(header)
    result = deploy.run()
    assert result.returncode != 0
    assert "staging" in result.stderr
    assert "the-key" not in result.stdout + result.stderr
    assert installs(deploy) == []


def test_another_caddy_refuses_a_promotion_and_says_how_to_move(deploy):
    deploy.caddy = "v2.11.0 h1:xyz="
    result = deploy.run()
    assert result.returncode != 0
    assert "2.11.0" in result.stderr and "2.10.2" in result.stderr
    assert "bin/caddy" in result.stderr
    assert "apt-mark hold caddy" in result.stderr
    assert installs(deploy) == []


def test_another_caddy_only_warns_on_a_rollback_and_deploys_once_asked(deploy):
    deploy.caddy = "v2.11.0 h1:xyz="
    deploy.production = deploy.new
    result = deploy.run(deploy.old, answers="y\ny\n")
    assert result.returncode == 0, result.stdout + result.stderr
    assert "2.11.0" in result.stdout
    assert len(installs(deploy)) == 1


def test_a_caddy_version_it_cant_read_is_refused(deploy):
    deploy.caddy = "caddy: command not found"
    result = deploy.run()
    assert result.returncode != 0
    assert "caddy: command not found" in result.stderr


def test_a_rollback_across_additive_migrations_lists_them_and_asks(deploy):
    deploy.production = deploy.new
    deploy.schema = "32"
    result = deploy.run(deploy.old, answers="y\ny\n")
    assert result.returncode == 0, result.stdout + result.stderr
    assert "0032_next.sql" in result.stdout
    assert installs(deploy)[0].endswith("--expect-schema 32")


def test_a_rollback_across_a_migration_that_breaks_it_is_refused(deploy):
    (deploy.repo / "crates/server/migration-rollback.txt").write_text(
        "32 breaks adds a status value\n"
    )
    git(deploy.repo, "commit", "-q", "-am", "mark")
    git(deploy.repo, "push", "-q")
    deploy.production = deploy.new
    deploy.schema = "32"
    result = deploy.run(deploy.old, answers="y\n")
    assert result.returncode != 0
    assert "0032_next.sql" in result.stderr
    assert "adds a status value" in result.stderr
    assert installs(deploy) == []


def test_a_schema_production_couldnt_read_is_refused(deploy):
    deploy.schema = "error"
    result = deploy.run()
    assert result.returncode != 0
    assert "schema" in result.stderr
    assert installs(deploy) == []


def test_an_install_script_on_the_server_older_than_main_is_replaced_first(deploy):
    deploy.install_sha = "0" * 64
    result = deploy.run()
    assert result.returncode == 0, result.stdout + result.stderr
    calls = deploy.host.calls()
    put = [
        i
        for i, call in enumerate(calls)
        if call.startswith("install ")
        and call.endswith("/usr/local/sbin/filedrawer-install")
    ]
    assert len(put) == 1
    assert put[0] < calls.index(installs(deploy)[0])
    assert (
        deploy.host.sent(put[0] + 1)
        == (deploy.repo / "ops/remote/filedrawer-install").read_text()
    )
    assert not any(c.endswith("/usr/local/sbin/filedrawer-backup") for c in calls)


def test_a_failed_install_says_so_and_checks_nothing_after(deploy):
    deploy.host.answer("/usr/local/sbin/filedrawer-install production *", status=1)
    result = deploy.run()
    assert result.returncode != 0
    assert "install" in result.stderr
    assert not deploy.smoke_log.exists()


def test_a_failed_check_after_a_deploy_says_it_deployed(deploy):
    (deploy.tmp / "smoke-fails").touch()
    result = deploy.run()
    assert result.returncode != 0
    assert "deployed" in result.stderr
    assert "post-deploy check failed" in result.stderr
    assert "rolled back" not in result.stderr


def test_a_first_deploy_says_to_set_up_the_account_next(deploy):
    deploy.production = ""
    deploy.schema = "none"
    result = deploy.run()
    assert result.returncode == 0, result.stdout + result.stderr
    assert installs(deploy)[0].endswith("--expect-schema none")
    assert "ops/provision-vps production account" in result.stdout


def test_a_rollback_goes_ahead_while_staging_cant_be_read(deploy):
    deploy.host.answer(
        "*staging.filedrawer.dev/skill.md*", stdout="HTTP/2 502\r\n\r\n", tool="curl"
    )
    deploy.production = deploy.new
    result = deploy.run(deploy.old, answers="y\n")
    assert result.returncode == 0, result.stdout + result.stderr
    assert "502" in result.stdout
    assert len(installs(deploy)) == 1


def test_a_rollback_asks_one_question_that_names_everything(deploy):
    deploy.caddy = "v2.11.0 h1:xyz="
    deploy.production = deploy.new
    deploy.schema = "32"
    result = deploy.run(deploy.old, answers="y\n")
    assert result.returncode == 0, result.stdout + result.stderr
    assert result.stdout.count("[y/N]") == 1
    question = result.stdout
    assert "not the one staging runs" in question
    assert "2.11.0" in question
    assert "0032_next.sql" in question


def test_no_to_the_question_changes_nothing_on_the_server(deploy):
    deploy.install_sha = "0" * 64
    deploy.production = deploy.new
    result = deploy.run(deploy.old, answers="n\n")
    assert result.returncode != 0
    assert deploy.host.calls() == ["bash -s"]
