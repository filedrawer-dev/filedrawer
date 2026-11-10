"""Tests for bin/mutants-base, which .github/workflows/mutants.yml's daily run asks for
the commit to test the changes since. Each test runs the script as a process in a
scratch git repo, with a fake gh first on PATH that answers from the runs and
artifacts the test sets up."""

import json
import os
import shutil
import subprocess
import sys
from datetime import UTC, datetime, timedelta
from pathlib import Path

import pytest

SCRIPT = Path(__file__).resolve().parents[1] / "mutants-base"
REPO = "filedrawer-dev/filedrawer"

# Answers by the API path in the call, from files in the fake's folder: the runs' list
# and each run's artifacts, whatever name the call asks for. `fail` in the folder makes
# every call fail as gh does on an API error.
FAKE_GH = r"""
import json
import re
import sys
from pathlib import Path

root = Path(sys.argv[1])
args = sys.argv[2:]
with (root / "log").open("a") as log:
    log.write(json.dumps(args) + "\n")
if (root / "fail").exists():
    sys.stderr.write("HTTP 502: Bad Gateway\n")
    sys.exit(1)
path = next(arg for arg in args if arg.startswith("repos/"))
if path.endswith("/actions/workflows/mutants.yml/runs"):
    for run in json.loads((root / "runs.json").read_text()):
        print(json.dumps(run))
elif found := re.search(r"/actions/runs/(\d+)/artifacts", path):
    for artifact in json.loads((root / "artifacts.json").read_text()).get(found[1], []):
        print(json.dumps(artifact))
else:
    sys.stderr.write(f"fake gh: no answer for {args}\n")
    sys.exit(1)
"""


def git(repo: Path, *args: str) -> str:
    done = subprocess.run(
        ["git", "-C", str(repo), *args], capture_output=True, text=True, check=True
    )
    return done.stdout.strip()


@pytest.fixture(scope="module")
def template(tmp_path_factory) -> tuple[Path, dict[str, str]]:
    """A repo whose history is: a first commit, the one that adds the workflow, and a
    code change on top; plus a commit off the second that HEAD doesn't hold."""
    repo = tmp_path_factory.mktemp("template") / "repo"
    repo.mkdir()
    git(repo, "init", "-q", "-b", "main")
    git(repo, "config", "user.email", "t@example.com")
    git(repo, "config", "user.name", "t")
    shas = {}

    def commit(name: str, path: str, text: str) -> None:
        target = repo / path
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(text)
        git(repo, "add", path)
        git(repo, "commit", "-q", "-m", name)
        shas[name] = git(repo, "rev-parse", "HEAD")

    commit("first", "README.md", "x\n")
    commit("workflow", ".github/workflows/mutants.yml", "name: Mutants\n")
    git(repo, "checkout", "-q", "-b", "side")
    commit("side", "side.rs", "fn side() {}\n")
    git(repo, "checkout", "-q", "main")
    commit("code", "lib.rs", "fn f() {}\n")
    commit("workflow-change", ".github/workflows/mutants.yml", "name: M\n")
    return repo, shas


class Fake:
    """The scratch repo and what its fake gh answers."""

    def __init__(self, root: Path, repo: Path, shas: dict[str, str]):
        self.root = root
        self.repo = repo
        self.shas = shas
        self.runs: list[dict] = []
        self.artifacts: dict[str, list[dict]] = {}
        bin_dir = root / "bin"
        bin_dir.mkdir()
        (root / "fake_gh.py").write_text(FAKE_GH)
        gh = bin_dir / "gh"
        gh.write_text(
            f'#!/bin/sh\nexec "{sys.executable}" -I "{root}/fake_gh.py" "{root}" "$@"\n'
        )
        gh.chmod(0o755)
        self.env = {**os.environ, "PATH": f"{bin_dir}{os.pathsep}{os.environ['PATH']}"}

    def run(
        self,
        commit: str,
        *,
        covered: bool = True,
        mutants: bool = True,
        expired: bool = False,
        event: str = "schedule",
        branch: str = "main",
        conclusion: str = "success",
        sha: str | None = None,
    ) -> int:
        """Add a run at `commit` (a name in shas), newer than those added before it,
        and return its id. A run that tested mutants has the `mutants` artifact; one
        that tested every mutant in its range, or found none, `mutants-covered`."""
        run_id = 100 + len(self.runs)
        self.runs.insert(
            0,
            {
                "id": run_id,
                "head_sha": sha or self.shas[commit],
                "head_branch": branch,
                "event": event,
                "conclusion": conclusion,
                "html_url": f"https://github.com/{REPO}/actions/runs/{run_id}",
            },
        )
        names = [
            name
            for name, has in (("mutants", mutants), ("mutants-covered", covered))
            if has
        ]
        self.artifacts[str(run_id)] = [
            {"id": 9000 + run_id * 2 + i, "name": name, "expired": expired}
            for i, name in enumerate(names)
        ]
        return run_id

    def base(self, *args: str) -> subprocess.CompletedProcess:
        (self.root / "runs.json").write_text(json.dumps(self.runs))
        (self.root / "artifacts.json").write_text(json.dumps(self.artifacts))
        return subprocess.run(
            [SCRIPT, *(args or (REPO,))],
            cwd=self.repo,
            env=self.env,
            capture_output=True,
            text=True,
            check=False,
        )

    def calls(self) -> list[list[str]]:
        log = self.root / "log"
        if not log.exists():
            return []
        return [json.loads(line) for line in log.read_text().splitlines()]


@pytest.fixture
def fake(tmp_path, template) -> Fake:
    repo, shas = template
    shutil.copytree(repo, tmp_path / "repo")
    return Fake(tmp_path, tmp_path / "repo", shas)


def test_the_base_is_the_head_of_the_newest_covered_run(fake):
    fake.run("workflow")
    fake.run("code")
    done = fake.base()
    assert done.returncode == 0, done.stderr
    assert done.stdout == fake.shas["code"] + "\n"
    assert "actions/runs/101" in done.stderr


def test_a_run_that_left_mutants_untested_is_passed_over(fake):
    fake.run("workflow")
    fake.run("code", covered=False)
    done = fake.base()
    assert done.returncode == 0, done.stderr
    assert done.stdout == fake.shas["workflow"] + "\n"


@pytest.mark.parametrize(
    "older",
    [
        {"covered": False, "mutants": False},
        {"expired": True},
        {"event": "push"},
        {"branch": "topic"},
        {"conclusion": "failure"},
    ],
)
def test_a_run_that_cant_show_it_covered_main_is_passed_over(fake, older):
    fake.run("workflow")
    fake.run("code", **older)
    done = fake.base()
    assert done.returncode == 0, done.stderr
    assert done.stdout == fake.shas["workflow"] + "\n"


def test_a_run_that_found_no_mutants_in_its_lines_counts(fake):
    fake.run("workflow")
    fake.run("code", mutants=False)
    done = fake.base()
    assert done.returncode == 0, done.stderr
    assert done.stdout == fake.shas["code"] + "\n"


def test_a_dispatched_run_counts(fake):
    fake.run("code", event="workflow_dispatch")
    done = fake.base()
    assert done.stdout == fake.shas["code"] + "\n"


@pytest.mark.parametrize("sha", [None, "f" * 40])
def test_a_run_at_a_commit_this_branch_doesnt_hold_is_passed_over(fake, sha):
    fake.run("workflow")
    fake.run("side", sha=sha)
    done = fake.base()
    assert done.returncode == 0, done.stderr
    assert done.stdout == fake.shas["workflow"] + "\n"


def test_with_no_covered_run_the_base_is_the_commit_that_added_the_workflow(fake):
    fake.run("code", covered=False)
    done = fake.base()
    assert done.returncode == 0, done.stderr
    assert done.stdout == fake.shas["workflow"] + "\n"
    assert "added the workflow" in done.stderr


def test_it_asks_only_for_the_repos_successful_runs_on_main(fake):
    fake.run("code")
    fake.base()
    runs_call = fake.calls()[0]
    assert f"repos/{REPO}/actions/workflows/mutants.yml/runs" in runs_call
    assert "branch=main" in runs_call
    assert "status=success" in runs_call
    # The artifacts last 30 days, so older runs can't show they covered anything.
    [created] = [arg for arg in runs_call if arg.startswith("created=>=")]
    since = datetime.strptime(created, "created=>=%Y-%m-%d").replace(tzinfo=UTC)
    assert timedelta(days=30) < datetime.now(UTC) - since < timedelta(days=32)


def test_an_api_error_fails_with_no_base(fake):
    fake.run("code")
    (fake.root / "fail").touch()
    done = fake.base()
    assert done.returncode == 1
    assert done.stdout == ""
    assert "HTTP 502" in done.stderr


def test_without_a_repo_it_prints_usage(fake):
    done = subprocess.run(
        [SCRIPT], cwd=fake.repo, capture_output=True, text=True, check=False
    )
    assert done.returncode == 2
    assert "usage" in done.stderr
    assert fake.calls() == []
