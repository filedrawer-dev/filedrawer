"""Tests for ops/claim-names, the wizard the owner runs to publish entry-check as
filedrawer on crates.io and PyPI and to claim the npm scope.

It runs in a clone of a made-up repo whose origin is a local bare repo, against
fakes of cargo, gh and curl that share one made-up registry state: `cargo publish`
puts a crate on the fake crates.io, a `gh workflow run` of release-cli.yml puts the
version on the fake PyPI, and curl answers the registries' APIs from that state.
Every prompt gets the same answer line, which also serves as the crates.io token.
"""

import os
import shutil
import subprocess
from pathlib import Path

import pytest
from conftest import opened_urls

REPO = Path(__file__).resolve().parents[2]
WIZARD = REPO / "ops/claim-names"
REPO_URL = "https://github.com/filedrawer-dev/filedrawer"
# Starts with y, so every confirm says yes.
TOKEN = "yes-cio_s3cret-token"

FAKE = r'''#!/usr/bin/env python3
"""cargo, gh or curl, by the name it was called with, against the state in $FAKE_STATE."""
import json, os, re, sys
from pathlib import Path

state = Path(os.environ["FAKE_STATE"])
tool, args = Path(sys.argv[0]).name, sys.argv[1:]
with (state / "log").open("a") as log:
    log.write(" ".join([tool, *args]) + "\n")


def version(path):
    return re.search(r'^version = "(.*)"$', Path(path).read_text(), re.M).group(1)


def put(registry, name, ver, repo):
    folder = state / registry / name
    (folder / "versions").mkdir(parents=True, exist_ok=True)
    (folder / "repository").write_text(repo)
    (folder / "versions" / ver).touch()


def answer(body, status):
    sys.stdout.write(body + "\n" + str(status))
    sys.exit(0)


if tool == "cargo":
    if args[0] == "login":
        (state / "cargo-token").write_text(sys.stdin.read())
    elif args[0] == "publish":
        name = args[args.index("-p") + 1]
        if name == "filedrawer" and not (state / "crates/filedrawer-analyzer").exists():
            print("error: no matching package named `filedrawer-analyzer` found", file=sys.stderr)
            sys.exit(101)
        folder = {"filedrawer": "crates/cli", "filedrawer-analyzer": "crates/analyzer"}[name]
        put("crates", name, version(folder + "/Cargo.toml"), os.environ["FAKE_REPO_URL"])
    sys.exit(0)

if tool == "gh":
    joined = " ".join(args)
    runs = state / "runs"
    if args[:2] == ["auth", "status"]:
        sys.exit(0)
    if args[0] == "api" and "deployment-branch-policies" in joined:
        (state / "pypi-env-policy").write_text(joined)
        sys.exit(0)
    if args[0] == "api" and joined.endswith("environments/pypi"):
        sys.exit(0 if (state / "pypi-env").exists() else 1)
    if args[0] == "api" and "-X PUT" in joined and "environments/pypi" in joined:
        (state / "pypi-env").write_text(sys.stdin.read())
        sys.exit(0)
    if args[:2] == ["run", "list"]:
        # Only the release's own runs: a pull request's run of the workflow isn't one.
        if "--event workflow_dispatch" not in joined or "--branch main" not in joined:
            print(99)
            sys.exit(0)
        print(len(runs.read_text().split()) if runs.exists() and runs.read_text() else "")
        sys.exit(0)
    if args[:2] == ["workflow", "run"]:
        with runs.open("a") as f:
            f.write("run ")
        ver = joined.split("version=")[1].split()[0]
        if not os.environ.get("FAKE_RUN_FAILS"):
            put("pypi", "filedrawer", ver, os.environ["FAKE_REPO_URL"])
        sys.exit(0)
    if args[:2] == ["run", "watch"]:
        sys.exit(1 if os.environ.get("FAKE_RUN_FAILS") else 0)
    sys.exit(0)

if tool == "curl":
    url = args[-1]
    # crates.io refuses API calls with no User-Agent of the client's own.
    if "crates.io" in url and "filedrawer-claim-names" not in " ".join(args):
        answer("{}", 403)
    if m := re.fullmatch(r"https://crates\.io/api/v1/crates/([^/]+)(?:/([^/]+))?", url):
        registry, name, ver = "crates", m.group(1), m.group(2)
        body = lambda repo: json.dumps({"crate": {"repository": repo}})
    elif m := re.fullmatch(r"https://pypi\.org/pypi/([^/]+)(?:/([^/]+))?/json", url):
        registry, name, ver = "pypi", m.group(1), m.group(2)
        body = lambda repo: json.dumps({"info": {"project_urls": {"Repository": repo}}})
    elif url == "https://registry.npmjs.org/-/org/filedrawer/package":
        if (state / "npm-scope").exists():
            answer("{}", 200)
        answer('{"error":"Scope not found"}', 404)
    else:
        sys.exit(6)
    folder = state / registry / name
    if not folder.exists() or (ver and not (folder / "versions" / ver).exists()):
        answer("{}", 404)
    answer(body((folder / "repository").read_text()), 200)
'''


def git(cwd: Path, *args: str) -> None:
    subprocess.run(
        ["git", "-c", "user.name=t", "-c", "user.email=t@t", *args],
        cwd=cwd,
        capture_output=True,
        check=True,
    )


class Box:
    """A clone, its origin, the fakes and their registry state."""

    def __init__(self, root: Path, *, workflow_on_main: bool = True):
        self.clone = root / "clone"
        self.state = root / "state"
        self.state.mkdir()
        (self.state / "log").touch()
        fakes = root / "fakes"
        fakes.mkdir()
        for tool in ("cargo", "gh", "curl"):
            (fakes / tool).write_text(FAKE)
            (fakes / tool).chmod(0o755)
        self.env = {
            **os.environ,
            "PATH": f"{fakes}{os.pathsep}{os.environ['PATH']}",
            "FAKE_STATE": str(self.state),
            "FAKE_REPO_URL": REPO_URL,
            "CLAIM_NAMES_POLL_SECONDS": "0",
        }
        origin = root / "origin.git"
        git(root, "init", "-q", "--bare", "-b", "main", str(origin))
        self.clone.mkdir()
        git(self.clone, "init", "-q", "-b", "main")
        for crate, ver in (("analyzer", "0.3.0"), ("cli", "0.2.0")):
            (self.clone / f"crates/{crate}").mkdir(parents=True)
            (self.clone / f"crates/{crate}/Cargo.toml").write_text(
                f'[package]\nname = "x"\nversion = "{ver}"\n'
            )
        if workflow_on_main:
            (self.clone / ".github/workflows").mkdir(parents=True)
            (self.clone / ".github/workflows/release-cli.yml").write_text("name: x\n")
        git(self.clone, "add", ".")
        git(self.clone, "commit", "-q", "-m", "c")
        git(self.clone, "remote", "add", "origin", str(origin))
        git(self.clone, "push", "-q", "origin", "main")

    def run(self, answer: str = TOKEN, *args: str, env: dict | None = None):
        return subprocess.run(
            ["bash", str(WIZARD), *args],
            cwd=self.clone,
            input=f"{answer}\n" * 60,
            env={**self.env, **(env or {})},
            capture_output=True,
            text=True,
            check=False,
        )

    def calls(self, tool: str) -> list[str]:
        prefix = f"{tool} "
        return [
            line[len(prefix) :]
            for line in (self.state / "log").read_text().splitlines()
            if line.startswith(prefix)
        ]

    def seed(self, registry: str, name: str, version: str, repo: str = REPO_URL):
        folder = self.state / registry / name
        (folder / "versions").mkdir(parents=True, exist_ok=True)
        (folder / "repository").write_text(repo)
        (folder / "versions" / version).touch()

    def has(self, registry: str, name: str, version: str) -> bool:
        return (self.state / registry / name / "versions" / version).exists()


@pytest.fixture
def box(tmp_path):
    return Box(tmp_path)


def test_help_says_what_it_claims():
    done = subprocess.run(
        ["bash", str(WIZARD), "-h"], capture_output=True, text=True, check=False
    )
    assert done.returncode == 0, done.stderr
    assert done.stdout.startswith("usage: ops/claim-names")
    for registry in ("crates.io", "PyPI", "npm", "Homebrew"):
        assert registry in done.stdout


def test_a_first_run_publishes_the_library_then_the_cli_then_starts_the_release(box):
    done = box.run()
    assert done.returncode == 0, done.stdout + done.stderr
    publishes = [c for c in box.calls("cargo") if c.startswith("publish")]
    assert publishes == [
        "publish --locked -p filedrawer-analyzer",
        "publish --locked -p filedrawer",
    ]
    assert box.has("crates", "filedrawer-analyzer", "0.3.0")
    assert box.has("crates", "filedrawer", "0.2.0")
    # The token reaches cargo on stdin, and nothing prints it.
    assert (box.state / "cargo-token").read_text() == TOKEN + "\n"
    assert TOKEN not in done.stdout + done.stderr
    assert not any(TOKEN in call for call in box.calls("cargo"))
    # The pypi environment, open to main only, then the release of main's version.
    assert '"custom_branch_policies": true' in (box.state / "pypi-env").read_text()
    assert "-f name=main" in (box.state / "pypi-env-policy").read_text()
    assert (
        box.calls("gh").count(
            "workflow run release-cli.yml --repo filedrawer-dev/filedrawer --ref main -f version=0.2.0"
        )
        == 1
    )
    assert box.has("pypi", "filedrawer", "0.2.0")
    # The trusted publisher's form, as PyPI names its fields.
    for line in (
        "PyPI Project Name: filedrawer",
        "Owner: filedrawer-dev",
        "Repository name: filedrawer",
        "Workflow name: release-cli.yml",
        "Environment name: pypi",
    ):
        assert line in done.stdout
    opened = opened_urls().read_text().split()
    assert "https://pypi.org/manage/account/publishing/" in opened
    assert "https://www.npmjs.com/org/create" in opened
    # An empty org looks free from outside, so the summary says how to check it.
    assert "settings/filedrawer/members" in done.stdout


def test_a_rerun_once_everything_is_claimed_publishes_nothing(box):
    box.seed("crates", "filedrawer-analyzer", "0.3.0")
    box.seed("crates", "filedrawer", "0.2.0")
    box.seed("pypi", "filedrawer", "0.2.0")
    (box.state / "pypi-env").touch()
    (box.state / "npm-scope").touch()
    done = box.run()
    assert done.returncode == 0, done.stdout + done.stderr
    assert box.calls("cargo") == []
    assert not any(c.startswith(("workflow run", "api -X")) for c in box.calls("gh")), (
        box.calls("gh")
    )
    assert "still to do by hand" not in done.stdout
    assert not opened_urls().exists() or "pypi.org" not in opened_urls().read_text()


def test_a_new_version_publishes_only_what_is_missing(box):
    """After a release bumps the CLI alone, the library isn't published again."""
    box.seed("crates", "filedrawer-analyzer", "0.3.0")
    box.seed("crates", "filedrawer", "0.1.0")
    box.seed("pypi", "filedrawer", "0.1.0")
    (box.state / "pypi-env").touch()
    done = box.run()
    assert done.returncode == 0, done.stdout + done.stderr
    assert [c for c in box.calls("cargo") if c.startswith("publish")] == [
        "publish --locked -p filedrawer"
    ]
    assert box.has("pypi", "filedrawer", "0.2.0")
    assert "https://pypi.org/manage/account/publishing/" not in (
        opened_urls().read_text() if opened_urls().exists() else ""
    )


def test_without_the_workflow_on_origin_main_it_says_to_push_and_starts_nothing(
    tmp_path,
):
    box = Box(tmp_path, workflow_on_main=False)
    done = box.run()
    assert done.returncode == 0, done.stdout + done.stderr
    assert not any(c.startswith("workflow run") for c in box.calls("gh"))
    assert "Push main, then rerun ops/claim-names." in done.stdout


def test_a_clone_that_isnt_origin_main_publishes_no_crate(box):
    """crates.io gets only what GitHub's main holds, as PyPI does."""
    (box.clone / "local.txt").write_text("unpushed\n")
    git(box.clone, "add", ".")
    git(box.clone, "commit", "-q", "-m", "local")
    done = box.run()
    assert not any(c.startswith("publish") for c in box.calls("cargo"))
    assert "isn't origin/main" in done.stdout


def test_uncommitted_changes_publish_no_crate(box):
    (box.clone / "crates/cli/Cargo.toml").write_text(
        '[package]\nname = "x"\nversion = "0.2.0"\n# edited\n'
    )
    done = box.run()
    assert not any(c.startswith("publish") for c in box.calls("cargo"))
    assert "isn't origin/main" in done.stdout


def test_a_crate_name_another_repository_holds_is_never_published(box):
    box.seed("crates", "filedrawer-analyzer", "9.9.9", repo="https://example.com/x")
    done = box.run()
    assert not any(c.startswith("publish") for c in box.calls("cargo"))
    assert "belongs to another repository" in done.stdout


def test_a_pypi_name_another_project_holds_starts_no_release(box):
    box.seed("pypi", "filedrawer", "1.0.0", repo="https://example.com/x")
    done = box.run()
    assert not any(c.startswith("workflow run") for c in box.calls("gh"))
    assert "belongs to another project" in done.stdout


def test_declining_publishes_nothing(box):
    done = box.run("n")
    assert done.returncode == 0, done.stdout + done.stderr
    assert not any(c.startswith("publish") for c in box.calls("cargo"))
    assert not any(c.startswith("workflow run") for c in box.calls("gh"))
    assert "still to do by hand" in done.stdout


def test_a_failed_release_run_says_how_to_read_its_log(box):
    done = box.run(env={"FAKE_RUN_FAILS": "1"})
    assert "--log-failed" in done.stdout
    assert not box.has("pypi", "filedrawer", "0.2.0")


def test_without_cargo_it_says_how_to_get_it(box):
    path = [
        d
        for d in box.env["PATH"].split(os.pathsep)
        if not (Path(d) / "cargo").exists() or d.endswith("fakes")
    ]
    fakes = next(d for d in path if d.endswith("fakes"))
    (Path(fakes) / "cargo").unlink()
    assert shutil.which("cargo", path=os.pathsep.join(path)) is None
    # BASH_ENV would source the sandbox's profile, which puts cargo back on PATH.
    done = box.run(env={"PATH": os.pathsep.join(path), "BASH_ENV": ""})
    assert done.returncode == 1
    assert "https://rustup.rs" in done.stdout
    assert box.calls("gh") == ["auth status"]


def test_it_refuses_a_clone_without_crates_cli(box):
    shutil.rmtree(box.clone / "crates/cli")
    done = box.run()
    assert done.returncode == 1
    assert "git pull first" in done.stderr
