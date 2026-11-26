"""Tests for ops/claim-names, the wizard the owner runs to publish entry-check as
filedrawer on crates.io and PyPI, and @filedrawer/cli on npm.

It runs in a clone of a made-up repo whose origin is a local bare repo, against
fakes of cargo, gh, curl and npx that share one made-up registry state: `cargo
publish` puts a crate on the fake crates.io, npx's `npm publish` a placeholder on
the fake npm, a `gh workflow run` of release-cli.yml puts the version on the fake
PyPI, and on npm when every package there has a trusted publisher, and curl answers
the registries' APIs from that state. The clone holds the real
crates/cli/npm/pack.mjs, which the real node runs. Every prompt gets the same
answer line, which also serves as the crates.io token.
"""

import json
import os
import shutil
import subprocess
from pathlib import Path

import pytest
from conftest import opened_urls

REPO = Path(__file__).resolve().parents[2]
WIZARD = REPO / "ops/claim-names"
NPM_PACKAGES = [
    "cli-linux-x64",
    "cli-linux-arm64",
    "cli-darwin-arm64",
    "cli-darwin-x64",
    "cli-win32-x64",
    "cli",
]
NPM = "--yes npm@11.21.2 "
REPO_URL = "https://github.com/filedrawer-dev/filedrawer"
# Starts with y, so every confirm says yes.
TOKEN = "yes-cio_s3cret-token"

FAKE = r'''#!/usr/bin/env python3
"""cargo, gh, curl or npx, by the name it was called with, against the state in $FAKE_STATE."""
import json, os, re, sys
from pathlib import Path

NPM_PACKAGES = ["cli-linux-x64", "cli-linux-arm64", "cli-darwin-arm64", "cli-darwin-x64", "cli-win32-x64", "cli"]
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
        if os.environ.get("FAKE_CARGO_FAILS"):
            sys.exit(101)
        name = args[args.index("-p") + 1]
        if name == "filedrawer" and not (state / "crates/filedrawer-analyzer").exists():
            print("error: no matching package named `filedrawer-analyzer` found", file=sys.stderr)
            sys.exit(101)
        folder = {"filedrawer": "crates/cli", "filedrawer-analyzer": "crates/analyzer"}[name]
        put("crates", name, version(folder + "/Cargo.toml"), os.environ["FAKE_REPO_URL"])
    sys.exit(0)

if tool == "npx":
    assert args[:2] == ["--yes", "npm@11.21.2"], args
    cmd = args[2:]
    trust = state / "npm-trust"
    trust.mkdir(exist_ok=True)
    if cmd == ["whoami"]:
        sys.exit(0 if (state / "npm-login").exists() else 1)
    if cmd == ["login"]:
        (state / "npm-login").touch()
        sys.exit(0)
    if cmd[0] == "publish":
        package = json.loads((Path(cmd[1]) / "package.json").read_text())
        if package["name"] == os.environ.get("FAKE_NPM_PUBLISH_REFUSES"):
            sys.exit(1)
        put("npm", package["name"].removeprefix("@filedrawer/"), package["version"], os.environ["FAKE_REPO_URL"])
        sys.exit(0)
    if cmd[:2] == ["trust", "list"]:
        found = trust / cmd[2].removeprefix("@filedrawer/")
        print(found.read_text() if found.exists() else "")
        sys.exit(0)
    if cmd[:2] == ["trust", "revoke"]:
        found = trust / cmd[2].removeprefix("@filedrawer/")
        assert f"--id={json.loads(found.read_text())['id']}" in cmd, cmd
        found.unlink()
        sys.exit(0)
    if cmd[:2] == ["trust", "github"]:
        found = trust / cmd[2].removeprefix("@filedrawer/")
        if found.exists():
            print("only one trust configuration per package", file=sys.stderr)
            sys.exit(1)
        found.write_text(json.dumps({"id": "new-" + found.name, "type": "github", "args": cmd[3:]}))
        sys.exit(0)
    sys.exit(2)

if tool == "gh":
    joined = " ".join(args)
    runs = state / "runs"
    if args[:2] == ["auth", "status"]:
        sys.exit(0)
    if args[0] == "api" and "deployment-branch-policies" in joined:
        env = re.search(r"environments/(\w+)/", joined).group(1)
        (state / f"{env}-env-policy").write_text(joined)
        sys.exit(0)
    if args[0] == "api" and (m := re.search(r"environments/(\w+)$", joined)) and "-X" not in args:
        sys.exit(0 if (state / f"{m.group(1)}-env").exists() else 1)
    if args[0] == "api" and "-X PUT" in joined and (m := re.search(r"environments/(\w+) ", joined)):
        (state / f"{m.group(1)}-env").write_text(sys.stdin.read())
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
        # As release-cli.yml's npm job, once npm has every package, each with a
        # trusted publisher: one set up here, or one a release has used.
        def trusted(p):
            versions = state / "npm" / p / "versions"
            released = versions.exists() and {v.name for v in versions.iterdir()} - {"0.0.0"}
            return (state / "npm-trust" / p).exists() or bool(released)
        failed = os.environ.get("FAKE_RUN_FAILS") or os.environ.get("FAKE_NPM_FAILS")
        if all(trusted(p) for p in NPM_PACKAGES) and not failed:
            for p in NPM_PACKAGES:
                put("npm", p, ver, os.environ["FAKE_REPO_URL"])
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
    elif m := re.fullmatch(r"https://registry\.npmjs\.org/@filedrawer/([^/]+)(?:/([^/]+))?", url):
        if os.environ.get("FAKE_NPM_DOWN"):
            answer("", 503)
        registry, name, ver = "npm", m.group(1), m.group(2)
        folder = state / "npm" / name
        body = lambda repo: json.dumps({"versions": {v.name: {} for v in (folder / "versions").iterdir()}})
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
        for tool in ("cargo", "gh", "curl", "npx"):
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
        (self.clone / "crates/cli/npm").mkdir()
        shutil.copy(REPO / "crates/cli/npm/pack.mjs", self.clone / "crates/cli/npm")
        shutil.copy(REPO / "crates/cli/LICENSE", self.clone / "crates/cli")
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

    def seed_npm(self, *versions: str, packages=NPM_PACKAGES):
        for package in packages:
            for version in versions:
                self.seed("npm", package, version)

    def npm_calls(self, command: str) -> list[str]:
        """npx's npm commands that start with COMMAND, without the npx pin."""
        return [
            c.removeprefix(NPM)
            for c in self.calls("npx")
            if c.removeprefix(NPM).startswith(command)
        ]

    def environments_made(self) -> set[str]:
        return {p.name.removesuffix("-env") for p in self.state.glob("*-env")}


needs_node = pytest.mark.skipif(
    not shutil.which("node") and not os.environ.get("CI"), reason="needs node"
)


def trust_github(package: str) -> str:
    return (
        f"trust github @filedrawer/{package} --file release-cli.yml"
        " --repo filedrawer-dev/filedrawer --env npm --allow-publish --yes"
    )


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


@needs_node
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
    # The pypi and npm environments, open to main only, then the release of main's
    # version.
    for env in ("pypi", "npm"):
        assert (
            '"custom_branch_policies": true' in (box.state / f"{env}-env").read_text()
        )
        assert "-f name=main" in (box.state / f"{env}-env-policy").read_text()
    assert (
        box.calls("gh").count(
            "workflow run release-cli.yml --repo filedrawer-dev/filedrawer --ref main -f version=0.2.0"
        )
        == 1
    )
    assert box.has("pypi", "filedrawer", "0.2.0")
    # npm: a login, a placeholder of each package, platforms first, then a trusted
    # publisher for each, all before the release, which publishes the real version.
    assert box.npm_calls("login") == ["login"]
    published = [Path(c.split()[1]).name for c in box.npm_calls("publish")]
    assert published == NPM_PACKAGES
    for package in NPM_PACKAGES:
        assert box.has("npm", package, "0.0.0")
        assert box.has("npm", package, "0.2.0")
    assert box.npm_calls("trust github") == [trust_github(p) for p in NPM_PACKAGES]
    log = (box.state / "log").read_text().splitlines()
    last_trust = max(i for i, line in enumerate(log) if " trust github " in line)
    release = next(
        i for i, line in enumerate(log) if line.startswith("gh workflow run")
    )
    assert last_trust < release
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


def test_a_rerun_once_everything_is_claimed_publishes_nothing(box):
    box.seed("crates", "filedrawer-analyzer", "0.3.0")
    box.seed("crates", "filedrawer", "0.2.0")
    box.seed("pypi", "filedrawer", "0.2.0")
    box.seed_npm("0.0.0", "0.2.0")
    (box.state / "pypi-env").touch()
    (box.state / "npm-env").touch()
    (box.state / "npm-scope").touch()
    done = box.run()
    assert done.returncode == 0, done.stdout + done.stderr
    assert box.calls("cargo") == []
    assert box.calls("npx") == []
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
    box.seed_npm("0.0.0", "0.1.0")
    (box.state / "pypi-env").touch()
    (box.state / "npm-env").touch()
    (box.state / "npm-scope").touch()
    done = box.run()
    assert done.returncode == 0, done.stdout + done.stderr
    assert [c for c in box.calls("cargo") if c.startswith("publish")] == [
        "publish --locked -p filedrawer"
    ]
    assert box.has("pypi", "filedrawer", "0.2.0")
    # A released package's trusted publisher has been used, so it stays as it is.
    assert box.calls("npx") == []
    assert box.has("npm", "cli", "0.2.0")
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
    done = box.run(env={"PATH": os.pathsep.join(path)})
    assert done.returncode == 1
    assert "https://rustup.rs" in done.stdout
    assert box.calls("gh") == ["auth status"]


@needs_node
def test_a_rerun_publishes_only_the_placeholders_npm_lacks(box):
    box.seed_npm("0.0.0", packages=NPM_PACKAGES[:3])
    (box.state / "npm-login").touch()
    done = box.run()
    assert done.returncode == 0, done.stdout + done.stderr
    published = [Path(c.split()[1]).name for c in box.npm_calls("publish")]
    assert published == NPM_PACKAGES[3:]
    # Each package still holds only a placeholder, so each gets its trusted publisher.
    assert box.npm_calls("trust github") == [trust_github(p) for p in NPM_PACKAGES]
    assert box.npm_calls("login") == []


@needs_node
def test_a_placeholders_old_trusted_publisher_is_made_again_before_the_release(box):
    """npm drops a trusted publisher that publishes nothing within 2 days, and
    `npm trust list` doesn't say whether it has, so a package never released gets
    its trusted publisher afresh."""
    box.seed_npm("0.0.0")
    (box.state / "npm-login").touch()
    (box.state / "npm-trust").mkdir()
    (box.state / "npm-trust/cli-darwin-x64").write_text(json.dumps({"id": "old-1"}))
    done = box.run()
    assert done.returncode == 0, done.stdout + done.stderr
    assert box.npm_calls("publish") == []
    assert box.npm_calls("trust revoke") == [
        "trust revoke @filedrawer/cli-darwin-x64 --id=old-1"
    ]
    assert box.npm_calls("trust github") == [trust_github(p) for p in NPM_PACKAGES]
    assert box.has("npm", "cli", "0.2.0")


def test_without_crates_io_at_mains_version_the_release_waits(box):
    """The three registries stay at one version."""
    done = box.run(env={"FAKE_CARGO_FAILS": "1"})
    assert not any(c.startswith("workflow run") for c in box.calls("gh"))
    assert "crates.io has no filedrawer 0.2.0" in done.stdout


def test_with_an_old_node_npm_is_left_for_later_and_pypi_still_releases(box):
    fakes = Path(box.env["PATH"].split(os.pathsep)[0])
    (fakes / "node").write_text("#!/bin/sh\necho 18.19.1\n")
    (fakes / "node").chmod(0o755)
    done = box.run()
    assert done.returncode == 0, done.stdout + done.stderr
    assert box.calls("npx") == []
    assert "https://nodejs.org" in done.stdout
    assert box.has("pypi", "filedrawer", "0.2.0")


@needs_node
def test_when_npm_does_not_answer_nothing_is_published_there(box):
    done = box.run(env={"FAKE_NPM_DOWN": "1"})
    assert box.npm_calls("publish") == []
    assert box.npm_calls("trust") == []
    assert "npm didn't answer" in done.stdout


@needs_node
def test_after_npm_refused_a_placeholder_the_release_waits_for_a_rerun(box):
    """npm then has some packages and not others, so the release's npm job would
    fail: the wizard starts no run until npm has them all."""
    done = box.run(env={"FAKE_NPM_PUBLISH_REFUSES": "@filedrawer/cli-darwin-arm64"})
    assert box.has("npm", "cli-linux-x64", "0.0.0")
    assert not any(c.startswith("workflow run") for c in box.calls("gh"))
    assert "rerun ops/claim-names" in done.stdout


@needs_node
def test_a_release_npm_refused_says_how_to_rerun_it(box):
    done = box.run(env={"FAKE_NPM_FAILS": "1"})
    assert box.has("pypi", "filedrawer", "0.2.0")
    assert not box.has("npm", "cli", "0.2.0")
    assert "npm doesn't list @filedrawer/cli 0.2.0" in done.stdout
    assert "gh run rerun" in done.stdout


def test_it_refuses_a_clone_without_crates_cli(box):
    shutil.rmtree(box.clone / "crates/cli")
    done = box.run()
    assert done.returncode == 1
    assert "git pull first" in done.stderr
