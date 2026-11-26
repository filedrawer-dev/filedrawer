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


# The fake gh: `gh api PATH` prints api[PATH] as JSON, or, when that is
# {"sequence": [...]}, the next of those on each call, then the last again; `gh run download ID ...
# --dir DIR` copies binary-ID to DIR, under the name in file-ID, as GitHub unpacks an
# artifact of one file under that file's name; `gh auth status` succeeds unless
# `logged-out` exists. Every call is a line in `log`.
FAKE_GH = """#!/usr/bin/env -S python3 -IS
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
    value = api[path]
    if isinstance(value, dict) and "sequence" in value:
        seq = value["sequence"]
        counter = root / ("count-" + path.replace("/", "_"))
        n = int(counter.read_text()) if counter.exists() else 0
        counter.write_text(str(n + 1))
        value = seq[min(n, len(seq) - 1)]
    print(json.dumps(value))
    sys.exit(0)
if args[:2] == ["run", "download"]:
    run = args[2]
    out = Path(args[args.index("--dir") + 1])
    out.mkdir(parents=True, exist_ok=True)
    name = (root / f"file-{{run}}").read_text()
    shutil.copy(root / f"binary-{{run}}", out / name)
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
        caddy_hold="no",
        schema="31",
        production=old,
        install_sha=sha256(install),
        backup_sha=sha256(backup),
    )

    def run_for(commit: str, run_id: int, **overrides) -> None:
        """A push run of ci.yml on main for `commit`, its jobs and its artifact, whose
        one file is named `file`: the server's binary, `filedrawer-server` since #77."""
        run = {
            "id": run_id,
            "head_sha": commit,
            "head_branch": "main",
            "event": "push",
            "path": ".github/workflows/ci.yml",
            "status": "completed",
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
        (gh_root / f"file-{run_id}").write_text(
            overrides.get("file", "filedrawer-server")
        )

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
            f"caddy {state.caddy}\ncaddy-hold {state.caddy_hold}\n"
            f"schema {state.schema}\nbinary {binary}\n"
            f"install {state.install_sha}\nbackup {state.backup_sha}\n"
        )

    def run(
        *args: str, answers: str = "", env: dict[str, str] | None = None
    ) -> subprocess.CompletedProcess:
        (gh_root / "api.json").write_text(json.dumps(state.api))
        fake_host.answer("bash -s", stdout=facts())
        env = {
            **fake_host.env,
            "PATH": f"{fakes}:{fake_host.env['PATH']}",
            "FILEDRAWER_REPO": str(repo),
            "FILEDRAWER_OPS_DIR": str(ops),
            "FILEDRAWER_SMOKE_STAGING": str(fakes / "smoke-staging"),
            "FILEDRAWER_CI_POLL": "0",
            "FILEDRAWER_CI_APPEAR_WAIT": "0",
            **(env or {}),
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


PASSED = ["check", "deploy-staging", "smoke-staging", "screen-staging"]


def ci_progresses(deploy, commit: str, run_id: int, polls: list) -> None:
    """GitHub's answers for `commit`'s run `run_id`, one poll at a time: each poll is
    (run status, run conclusion, {job: state}), a job's state its status until it
    completes, then its conclusion. A poll of None is a time before the run exists."""
    deploy.api.clear()
    deploy.run_for(commit, run_id)
    runs_path = f"{GH_REPO}/actions/runs?head_sha={commit}&per_page=100"
    jobs_path = f"{GH_REPO}/actions/runs/{run_id}/jobs?per_page=100"
    run = deploy.api[runs_path]["workflow_runs"][0]
    run["html_url"] = f"https://github.com/x/actions/runs/{run_id}"
    runs, jobs = [], []
    for poll in polls:
        if poll is None:
            runs.append({"workflow_runs": []})
            continue
        status, conclusion, states = poll
        runs.append(
            {"workflow_runs": [{**run, "status": status, "conclusion": conclusion}]}
        )
        jobs.append(
            {
                "jobs": [
                    {
                        "name": name,
                        "status": state
                        if state in ("queued", "in_progress")
                        else "completed",
                        "conclusion": None
                        if state in ("queued", "in_progress")
                        else state,
                    }
                    for name, state in states.items()
                ]
            }
        )
    deploy.api[runs_path] = {"sequence": runs}
    deploy.api[jobs_path] = {"sequence": jobs}


def finished_run(**failed: str) -> tuple:
    states = {name: "success" for name in PASSED} | {
        k.replace("_", "-"): v for k, v in failed.items()
    }
    conclusion = "success" if not failed else "failure"
    return ("completed", conclusion, states)


RUNNING = ("in_progress", None, {"check": "in_progress"})


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


def test_a_rollback_takes_a_binary_built_before_the_rename(deploy):
    # Before #77 the binary, and so the artifact's one file, was named `filedrawer`.
    deploy.production = deploy.new
    deploy.api.clear()
    deploy.run_for(deploy.old, 90, file="filedrawer")
    result = deploy.run(deploy.old, answers="y\n")
    assert result.returncode == 0, result.stdout + result.stderr
    assert sent_binary(deploy) == (deploy.gh / "binary-90").read_bytes()


def test_an_artifact_without_the_servers_binary_is_refused(deploy):
    deploy.api.clear()
    deploy.run_for(deploy.new, 100, file="entry-check")
    result = deploy.run()
    assert result.returncode != 0
    assert "holds no filedrawer-server" in result.stderr
    assert installs(deploy) == []


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


def test_a_newer_caddy_than_the_pin_deploys_without_asking_and_says_to_bump(deploy):
    deploy.caddy = "v2.11.0 h1:xyz="
    result = deploy.run()
    assert result.returncode == 0, result.stdout + result.stderr
    assert "[y/N]" not in result.stdout
    assert "2.11.0" in result.stdout and "2.10.2" in result.stdout
    assert "bin/caddy" in result.stdout
    assert len(installs(deploy)) == 1


def test_an_older_caddy_than_the_pin_asks_and_a_no_changes_nothing(deploy):
    deploy.caddy = "v2.9.1 h1:xyz="
    result = deploy.run(answers="n\n")
    assert result.returncode != 0
    assert result.stdout.count("[y/N]") == 1
    assert "2.9.1" in result.stdout and "2.10.2" in result.stdout
    assert "not held" in result.stdout
    assert "apt-mark hold" not in result.stdout + result.stderr
    assert installs(deploy) == []


def test_the_question_on_an_older_caddy_says_it_is_held(deploy):
    deploy.caddy = "v2.9.1 h1:xyz="
    deploy.caddy_hold = "guard"
    result = deploy.run(answers="n\n")
    question = result.stdout.split("[y/N]")[0].splitlines()
    assert any(
        "2.9.1" in line and "held" in line and "not held" not in line
        for line in question
    )


def test_a_pin_it_cant_read_is_refused(deploy):
    (deploy.repo / "bin/caddy").write_text("# no version here\n")
    git(deploy.repo, "commit", "-q", "-am", "unreadable pin")
    git(deploy.repo, "push", "-q")
    head = git(deploy.repo, "rev-parse", "HEAD")
    deploy.run_for(head, 101)
    deploy.staging_runs(head)
    result = deploy.run(answers="y\n")
    assert result.returncode != 0
    assert "bin/caddy" in result.stderr
    assert installs(deploy) == []


def test_an_older_caddy_than_the_pin_deploys_once_asked(deploy):
    deploy.caddy = "v2.9.1 h1:xyz="
    result = deploy.run(answers="y\n")
    assert result.returncode == 0, result.stdout + result.stderr
    assert len(installs(deploy)) == 1


def test_a_version_compare_is_numeric_not_textual(deploy):
    # 2.10.2 sorts before 2.9.1 as text; as versions the server is newer.
    deploy.caddy = "v2.10.10 h1:xyz="
    result = deploy.run()
    assert result.returncode == 0, result.stdout + result.stderr
    assert "[y/N]" not in result.stdout
    assert len(installs(deploy)) == 1


@pytest.mark.parametrize(
    ("hold", "says"),
    [("guard", "rolled"), ("manual", "by hand")],
)
def test_a_held_caddy_is_named_with_who_held_it(deploy, hold, says):
    deploy.caddy_hold = hold
    result = deploy.run()
    assert result.returncode == 0, result.stdout + result.stderr
    assert "held" in result.stdout and says in result.stdout


def test_an_older_caddy_on_a_rollback_goes_in_the_one_question(deploy):
    deploy.caddy = "v2.9.1 h1:xyz="
    deploy.production = deploy.new
    result = deploy.run(deploy.old, answers="y\n")
    assert result.returncode == 0, result.stdout + result.stderr
    assert result.stdout.count("[y/N]") == 1
    assert "2.9.1" in result.stdout
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


def test_an_install_that_kept_the_binary_with_a_warning_checks_then_exits_3(deploy):
    # Exit 3: the new binary serves, but the outbound probe failed for both (#102).
    deploy.host.answer("/usr/local/sbin/filedrawer-install production *", status=3)
    result = deploy.run()
    assert result.returncode == 3
    assert deploy.smoke_log.exists(), "the read check still runs"
    assert "deployed" in result.stderr
    assert "nor could" not in result.stderr, "exit 3 has more causes than one"
    assert "warning" in result.stderr


def test_a_first_deploy_kept_with_a_warning_still_says_to_set_up_the_account(deploy):
    deploy.production = ""
    deploy.schema = "none"
    deploy.host.answer("/usr/local/sbin/filedrawer-install production *", status=3)
    result = deploy.run()
    assert result.returncode == 3
    assert "ops/provision-vps production account" in result.stdout


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


@pytest.mark.parametrize(
    ("held", "guard_marker", "expected"),
    [(False, False, "no"), (True, False, "manual"), (True, True, "guard")],
)
def test_the_facts_say_who_holds_caddy(tmp_path, held, guard_marker, expected):
    """deploy-facts.sh on its own, with fakes of caddy and apt-mark first on PATH and
    FILEDRAWER_ROOT for the guard's marker."""
    fakes = tmp_path / "fakes"
    fakes.mkdir()
    executable(fakes / "caddy", "#!/bin/sh\necho 'v2.11.9 h1:abc='\n")
    executable(fakes / "runuser", "#!/bin/sh\nexit 1\n")
    showhold = "echo caddy" if held else "true"
    executable(
        fakes / "apt-mark",
        f'#!/bin/sh\n[ "$1 $2" = "showhold caddy" ] || exit 2\n{showhold}\n',
    )
    if guard_marker:
        marker = tmp_path / "var/lib/filedrawer/caddy-guard/hold"
        marker.parent.mkdir(parents=True)
        marker.write_text("2.11.8\n")
    result = subprocess.run(
        ["bash", str(FACTS)],
        env={"PATH": f"{fakes}:/usr/bin:/bin", "FILEDRAWER_ROOT": str(tmp_path)},
        capture_output=True,
        text=True,
        check=False,
    )
    assert f"caddy-hold {expected}\n" in result.stdout, result.stdout + result.stderr


def test_a_finished_run_needs_no_wait(deploy):
    result = deploy.run()
    assert result.returncode == 0, result.stdout + result.stderr
    assert "waiting" not in result.stdout.lower()


def test_it_waits_for_ci_running_on_the_tip_then_promotes_it(deploy):
    # Just pushed: staging still runs `old`, and CI is building `new`.
    deploy.staging_runs(deploy.new)
    ci_progresses(
        deploy,
        deploy.new,
        100,
        [
            ("queued", None, {}),
            RUNNING,
            (
                "in_progress",
                None,
                {"check": "success", "deploy-staging": "in_progress"},
            ),
            finished_run(),
        ],
    )
    result = deploy.run()
    assert result.returncode == 0, result.stdout + result.stderr
    out = result.stdout
    assert "CI is still running on " + deploy.new in out
    assert "https://github.com/x/actions/runs/100" in out
    assert "check: in_progress" in out
    assert "check: success" in out
    assert "deploy-staging: in_progress" in out
    assert "screen-staging: success" in out
    # A state is shown once, when it changes.
    assert out.count("check: success") == 1
    assert out.index("check: in_progress") < out.index("check: success")
    assert f"deployed {deploy.new}" in out
    assert len(installs(deploy)) == 1


def test_the_wait_comes_before_staging_or_the_server_is_read(deploy):
    # While deploy-staging restarts staging it can't name its commit.
    ci_progresses(deploy, deploy.new, 100, [RUNNING, finished_run()])
    deploy.host.answer(
        "*staging.filedrawer.dev/skill.md*", stdout="HTTP/2 502\r\n\r\n", tool="curl"
    )
    result = deploy.run()
    assert "CI is still running" in result.stdout
    assert "502" in result.stderr, "staging was read after the wait"
    assert deploy.host.calls() == []


@pytest.mark.parametrize(
    ("failed", "rerun_promotes"),
    [
        ({"check": "failure", "deploy_staging": "skipped"}, False),
        ({"smoke_staging": "cancelled"}, False),
        ({"screen_staging": "failure"}, True),
    ],
)
def test_a_run_it_waited_for_that_fails_stops_and_says_why(
    deploy, failed, rerun_promotes
):
    ci_progresses(deploy, deploy.new, 100, [RUNNING, finished_run(**failed)])
    result = deploy.run()
    assert result.returncode != 0
    for name, conclusion in failed.items():
        if conclusion != "skipped":
            assert f"{name.replace('_', '-')} {conclusion}" in result.stderr
    assert "https://github.com/x/actions/runs/100" in result.stderr
    assert "nothing changed" in result.stderr
    assert ("again promotes it" in result.stderr) == rerun_promotes
    assert deploy.host.calls() == []


def test_a_run_that_failed_before_any_job_says_how_it_ended(deploy):
    # GitHub creates a run of an invalid workflow already completed.
    deploy.staging_runs(deploy.old)
    ci_progresses(deploy, deploy.new, 100, [("completed", "startup_failure", {})])
    result = deploy.run()
    assert result.returncode != 0
    assert "startup_failure" in result.stderr
    assert installs(deploy) == []


def test_a_tip_whose_ci_already_failed_stops_and_names_stagings_commit(deploy):
    deploy.staging_runs(deploy.old)
    ci_progresses(
        deploy,
        deploy.new,
        100,
        [finished_run(check="failure", deploy_staging="skipped")],
    )
    result = deploy.run()
    assert result.returncode != 0
    assert f"CI failed on {deploy.new}: check failure" in result.stderr
    assert deploy.old in result.stderr, "it says how to promote staging's instead"
    assert installs(deploy) == []


def test_a_tip_whose_only_failure_is_another_job_deploys_without_waiting(deploy):
    ci_progresses(deploy, deploy.new, 100, [finished_run(screen_staging="failure")])
    result = deploy.run()
    assert result.returncode == 0, result.stdout + result.stderr
    assert f"deployed {deploy.new}" in result.stdout


def test_a_cancelled_job_says_a_newer_push_may_have_cancelled_it(deploy):
    ci_progresses(
        deploy, deploy.new, 100, [RUNNING, finished_run(deploy_staging="cancelled")]
    )
    result = deploy.run()
    assert result.returncode != 0
    assert "newer push" in result.stderr


def test_right_after_a_push_it_waits_for_the_run_to_appear(deploy):
    ci_progresses(deploy, deploy.new, 100, [None, None, RUNNING, finished_run()])
    result = deploy.run(env={"FILEDRAWER_CI_APPEAR_WAIT": "30"})
    assert result.returncode == 0, result.stdout + result.stderr
    assert "Waiting for GitHub to start CI on " + deploy.new in result.stdout
    assert "CI is still running" in result.stdout
    assert len(installs(deploy)) == 1


def test_a_tip_no_run_appears_for_goes_on_with_stagings_commit(deploy):
    # The tip, `new`, gets no run; staging still runs `old`, whose run passed.
    deploy.staging_runs(deploy.old)
    ci_progresses(deploy, deploy.new, 100, [None])
    deploy.run_for(deploy.old, 90)
    result = deploy.run(
        env={"FILEDRAWER_CI_POLL": "0.2", "FILEDRAWER_CI_APPEAR_WAIT": "1"}
    )
    assert result.returncode == 0, result.stdout + result.stderr
    assert "No CI run started for " + deploy.new in result.stdout
    assert sent_binary(deploy) == (deploy.gh / "binary-90").read_bytes()


def test_a_named_commit_whose_ci_runs_is_waited_for(deploy):
    ci_progresses(deploy, deploy.new, 100, [RUNNING, finished_run()])
    result = deploy.run(deploy.new[:8])
    assert result.returncode == 0, result.stdout + result.stderr
    assert "CI is still running on " + deploy.new in result.stdout


def test_a_named_commit_with_no_run_doesnt_wait_for_one(deploy):
    ci_progresses(deploy, deploy.new, 100, [None])
    result = deploy.run(
        deploy.new, env={"FILEDRAWER_CI_POLL": "5", "FILEDRAWER_CI_APPEAR_WAIT": "60"}
    )
    assert "Waiting for GitHub" not in result.stdout
    assert "no push run" in result.stderr


def test_a_run_still_going_past_the_limit_stops(deploy):
    ci_progresses(deploy, deploy.new, 100, [RUNNING])
    result = deploy.run(env={"FILEDRAWER_CI_TIMEOUT": "0"})
    assert result.returncode != 0
    assert "still running" in result.stderr
    assert "nothing changed" in result.stderr
    assert deploy.host.calls("curl") == []
