"""Tests for the npm packages of the `filedrawer` command (#84; design.md,
Distribution): crates/cli/npm/pack.mjs packs a platform package around a binary,
and the launcher, `@filedrawer/cli`, which runs the binary of the platform package
npm installed beside it.

They run the real node and npm, offline: a dead registry and a cache of their own,
so npm never fetches the launcher's other platform packages. Without node or npm
they skip, except in CI.
"""

import json
import os
import platform
import re
import shutil
import subprocess
import tarfile
from pathlib import Path

import pytest

REPO = Path(__file__).resolve().parents[2]
PACK = REPO / "crates/cli/npm/pack.mjs"
CLI_VERSION = re.search(
    r'^version = "(.*)"$', (REPO / "crates/cli/Cargo.toml").read_text(), re.M
).group(1)
REPOSITORY = "git+https://github.com/filedrawer-dev/filedrawer.git"
# Rust target → (npm package, os, cpu, binary's name), as release-cli.yml builds them.
PLATFORMS = {
    "x86_64-unknown-linux-musl": (
        "@filedrawer/cli-linux-x64",
        "linux",
        "x64",
        "filedrawer",
    ),
    "aarch64-unknown-linux-musl": (
        "@filedrawer/cli-linux-arm64",
        "linux",
        "arm64",
        "filedrawer",
    ),
    "aarch64-apple-darwin": (
        "@filedrawer/cli-darwin-arm64",
        "darwin",
        "arm64",
        "filedrawer",
    ),
    "x86_64-apple-darwin": (
        "@filedrawer/cli-darwin-x64",
        "darwin",
        "x64",
        "filedrawer",
    ),
    "x86_64-pc-windows-msvc": (
        "@filedrawer/cli-win32-x64",
        "win32",
        "x64",
        "filedrawer.exe",
    ),
}
HOST_TARGET = {
    ("Linux", "x86_64"): "x86_64-unknown-linux-musl",
    ("Linux", "aarch64"): "aarch64-unknown-linux-musl",
}.get((platform.system(), platform.machine()))

pytestmark = pytest.mark.skipif(
    not (shutil.which("node") and shutil.which("npm")) and not os.environ.get("CI"),
    reason="needs node and npm",
)


def pack(out: Path, *args: str) -> Path:
    done = subprocess.run(
        ["node", str(PACK), *args, str(out)],
        capture_output=True,
        text=True,
        check=False,
    )
    assert done.returncode == 0, done.stdout + done.stderr
    tarball = Path(done.stdout.strip().splitlines()[-1])
    assert tarball.parent == out and tarball.suffix == ".tgz", done.stdout
    return tarball


def stand_in_binary(folder: Path, name: str = "filedrawer") -> Path:
    """A binary that says what it was given and exits 3."""
    binary = folder / name
    binary.write_text('#!/bin/sh\necho "given: $*"\nexit 3\n')
    binary.chmod(0o755)
    return binary


def contents(tarball: Path) -> dict[str, bytes]:
    with tarfile.open(tarball) as tar:
        return {
            m.name.removeprefix("package/"): tar.extractfile(m).read()
            for m in tar.getmembers()
            if m.isfile()
        }


def manifest(tarball: Path) -> dict:
    return json.loads(contents(tarball)["package.json"])


def install(project: Path, *tarballs: Path) -> None:
    project.mkdir()
    (project / "package.json").write_text("{}")
    done = subprocess.run(
        [
            "npm",
            "install",
            "--offline",
            "--no-audit",
            "--no-fund",
            "--registry",
            "http://127.0.0.1:9/",
            "--cache",
            str(project / ".npm-cache"),
            *map(str, tarballs),
        ],
        cwd=project,
        capture_output=True,
        text=True,
        check=False,
    )
    assert done.returncode == 0, done.stdout + done.stderr


def filedrawer(project: Path, *args: str) -> subprocess.CompletedProcess:
    return subprocess.run(
        [str(project / "node_modules/.bin/filedrawer"), *args],
        capture_output=True,
        text=True,
        check=False,
    )


@pytest.mark.skipif(HOST_TARGET is None, reason="no package for this platform")
def test_the_launcher_runs_its_platforms_binary_with_the_arguments_and_status(
    tmp_path,
):
    out = tmp_path / "out"
    out.mkdir()
    binary = stand_in_binary(tmp_path)
    tarballs = [pack(out, "platform", HOST_TARGET, str(binary)), pack(out, "launcher")]
    install(tmp_path / "project", *tarballs)
    done = filedrawer(tmp_path / "project", "check", "a file.md")
    assert (done.returncode, done.stdout) == (3, "given: check a file.md\n")


def test_without_its_platform_package_the_launcher_says_which_there_are(tmp_path):
    out = tmp_path / "out"
    out.mkdir()
    install(tmp_path / "project", pack(out, "launcher"))
    done = filedrawer(tmp_path / "project", "check", "x.md")
    assert done.returncode == 1
    for os_cpu in (
        "linux-x64",
        "linux-arm64",
        "darwin-arm64",
        "darwin-x64",
        "win32-x64",
    ):
        assert os_cpu in done.stderr
    assert "uvx filedrawer" in done.stderr
    assert "cargo install filedrawer --locked" in done.stderr


@pytest.mark.parametrize("target", PLATFORMS)
def test_a_platform_package_holds_its_binary_for_its_os_and_cpu(tmp_path, target):
    name, os_, cpu, exe = PLATFORMS[target]
    out = tmp_path / "out"
    out.mkdir()
    tarball = pack(out, "platform", target, str(stand_in_binary(tmp_path, exe)))
    package = manifest(tarball)
    assert (package["name"], package["os"], package["cpu"]) == (name, [os_], [cpu])
    files = contents(tarball)
    assert files[f"bin/{exe}"] == (tmp_path / exe).read_bytes()
    with tarfile.open(tarball) as tar:
        assert tar.getmember(f"package/bin/{exe}").mode & 0o111
    assert files["LICENSE"] == (REPO / "LICENSE").read_bytes()
    assert_published_metadata(package)


def test_the_launcher_depends_on_every_platform_at_its_own_version(tmp_path):
    out = tmp_path / "out"
    out.mkdir()
    tarball = pack(out, "launcher")
    package = manifest(tarball)
    assert package["name"] == "@filedrawer/cli"
    assert package["optionalDependencies"] == {
        name: CLI_VERSION for name, *_ in PLATFORMS.values()
    }
    assert set(package["bin"]) == {"filedrawer"}
    assert contents(tarball)["LICENSE"] == (REPO / "LICENSE").read_bytes()
    assert "README.md" in contents(tarball)
    assert_published_metadata(package)


def assert_published_metadata(package: dict) -> None:
    assert package["version"] == CLI_VERSION
    assert package["license"] == "AGPL-3.0-only"
    # npm's trusted publishing refuses a repository.url that isn't the repo's own.
    assert package["repository"]["url"] == REPOSITORY
    # A scoped package's first publish is otherwise private, which a free org refuses.
    assert package["publishConfig"]["access"] == "public"
    # No install script: npm's --ignore-scripts would leave a package that runs none.
    assert "scripts" not in package


def test_a_target_with_no_package_is_refused(tmp_path):
    done = subprocess.run(
        [
            "node",
            str(PACK),
            "platform",
            "riscv64gc-unknown-linux-gnu",
            str(stand_in_binary(tmp_path)),
            str(tmp_path),
        ],
        capture_output=True,
        text=True,
        check=False,
    )
    assert done.returncode != 0
    assert "riscv64gc-unknown-linux-gnu" in done.stderr
    assert not list(tmp_path.glob("*.tgz"))
