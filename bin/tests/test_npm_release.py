"""Tests for bin/npm-release, which the release run's npm job runs (#84): it checks
the tarballs are the six packages at the version given, then publishes those npm
lacks, platform packages first and the launcher last, and publishes nothing until
ops/claim-names has made the packages.

They run it against fakes of curl, answering registry.npmjs.org from a made-up
state, and of npx, which records each publish.
"""

import io
import json
import os
import subprocess
import tarfile
from pathlib import Path

import pytest

REPO = Path(__file__).resolve().parents[2]
SCRIPT = REPO / "bin/npm-release"
PLATFORMS = [
    "cli-linux-x64",
    "cli-linux-arm64",
    "cli-darwin-arm64",
    "cli-darwin-x64",
    "cli-win32-x64",
]
VERSION = "0.4.0"

FAKE = r'''#!/usr/bin/env python3
"""curl or npx, by the name it was called with, against the state in $FAKE_STATE."""
import json, os, re, sys
from pathlib import Path

state = Path(os.environ["FAKE_STATE"])
tool, args = Path(sys.argv[0]).name, sys.argv[1:]
with (state / "log").open("a") as log:
    log.write(" ".join([tool, *args]) + "\n")

if tool == "npx":
    if os.environ.get("FAKE_PUBLISH_FAILS"):
        sys.exit(1)
    sys.exit(0)

url = args[-1]
if os.environ.get("FAKE_NPM_DOWN"):
    sys.stdout.write("503")
    sys.exit(0)
m = re.fullmatch(r"https://registry\.npmjs\.org/@filedrawer/([^/]+)(?:/([^/]+))?", url)
if not m:
    sys.exit(6)
versions = json.loads((state / "npm.json").read_text()).get(m.group(1))
found = versions is not None and (m.group(2) is None or m.group(2) in versions)
sys.stdout.write("200" if found else "404")
'''


def tarball(folder: Path, name: str, version: str, **manifest) -> Path:
    data = json.dumps(
        {"name": f"@filedrawer/{name}", "version": version, **manifest}
    ).encode()
    path = folder / f"filedrawer-{name}-{version}.tgz"
    with tarfile.open(path, "w:gz") as tar:
        info = tarfile.TarInfo("package/package.json")
        info.size = len(data)
        tar.addfile(info, io.BytesIO(data))
    return path


class Box:
    def __init__(self, root: Path, npm: dict[str, list[str]]):
        self.dist = root / "dist"
        self.dist.mkdir()
        self.state = root / "state"
        self.state.mkdir()
        (self.state / "log").touch()
        (self.state / "npm.json").write_text(json.dumps(npm))
        fakes = root / "fakes"
        fakes.mkdir()
        for tool in ("curl", "npx"):
            (fakes / tool).write_text(FAKE)
            (fakes / tool).chmod(0o755)
        self.env = {
            **os.environ,
            "PATH": f"{fakes}{os.pathsep}{os.environ['PATH']}",
            "FAKE_STATE": str(self.state),
        }
        for name in PLATFORMS:
            tarball(self.dist, name, VERSION)
        tarball(
            self.dist,
            "cli",
            VERSION,
            optionalDependencies={f"@filedrawer/{p}": VERSION for p in PLATFORMS},
        )

    def run(self, version: str = VERSION, dist: str | None = None, **env: str):
        return subprocess.run(
            [str(SCRIPT), dist or str(self.dist), version],
            cwd=self.dist.parent,
            env={**self.env, **env},
            capture_output=True,
            text=True,
            check=False,
        )

    def published(self) -> list[str]:
        return [
            Path(line.split()[-1]).name
            for line in (self.state / "log").read_text().splitlines()
            if line.startswith("npx") and " publish " in line
        ]


ALL_PLACEHOLDERS = {name: ["0.0.0"] for name in [*PLATFORMS, "cli"]}


def test_it_publishes_the_platform_packages_then_the_launcher(tmp_path):
    box = Box(tmp_path, ALL_PLACEHOLDERS)
    done = box.run()
    assert done.returncode == 0, done.stdout + done.stderr
    assert box.published() == [
        *(f"filedrawer-{p}-{VERSION}.tgz" for p in PLATFORMS),
        f"filedrawer-cli-{VERSION}.tgz",
    ]


def test_a_relative_dir_reaches_npm_as_a_path_not_a_github_repository(tmp_path):
    """npm reads `dist/x.tgz` as the GitHub repository dist/x.tgz, as release
    run 43669677567 found; a path that starts with / is a file."""
    box = Box(tmp_path, ALL_PLACEHOLDERS)
    done = box.run(dist="dist")
    assert done.returncode == 0, done.stdout + done.stderr
    tarballs = [
        line.split()[-1]
        for line in (box.state / "log").read_text().splitlines()
        if line.startswith("npx") and " publish " in line
    ]
    assert len(tarballs) == 6
    for tarball in tarballs:
        assert Path(tarball).is_absolute() and Path(tarball).is_file(), tarball


def test_a_rerun_publishes_only_what_npm_lacks(tmp_path):
    npm = {**ALL_PLACEHOLDERS, "cli-linux-x64": ["0.0.0", VERSION]}
    box = Box(tmp_path, npm)
    done = box.run()
    assert done.returncode == 0, done.stdout + done.stderr
    assert f"filedrawer-cli-linux-x64-{VERSION}.tgz" not in box.published()
    assert len(box.published()) == 5


def test_before_claim_names_made_the_packages_it_publishes_nothing_and_passes(tmp_path):
    box = Box(tmp_path, {})
    done = box.run()
    assert done.returncode == 0, done.stdout + done.stderr
    assert box.published() == []
    assert "::notice::" in done.stdout and "ops/claim-names" in done.stdout


def test_when_npm_has_only_some_packages_it_publishes_nothing_and_fails(tmp_path):
    """A launcher must never name a platform package nobody published."""
    npm = {k: v for k, v in ALL_PLACEHOLDERS.items() if k != "cli-win32-x64"}
    box = Box(tmp_path, npm)
    done = box.run()
    assert done.returncode == 1
    assert box.published() == []
    assert "cli-win32-x64" in done.stdout


def test_when_npm_does_not_answer_it_publishes_nothing_and_fails(tmp_path):
    box = Box(tmp_path, ALL_PLACEHOLDERS)
    done = box.run(FAKE_NPM_DOWN="1")
    assert done.returncode == 1
    assert box.published() == []
    assert "::error::" in done.stdout


def test_tarballs_of_another_version_publish_nothing(tmp_path):
    box = Box(tmp_path, ALL_PLACEHOLDERS)
    done = box.run("0.4.1")
    assert done.returncode == 1
    assert box.published() == []
    assert "0.4.1" in done.stdout


def test_a_missing_platform_tarball_publishes_nothing(tmp_path):
    box = Box(tmp_path, ALL_PLACEHOLDERS)
    (box.dist / f"filedrawer-cli-darwin-x64-{VERSION}.tgz").unlink()
    done = box.run()
    assert done.returncode == 1
    assert box.published() == []
    assert "cli-darwin-x64" in done.stdout


def test_an_extra_file_publishes_nothing(tmp_path):
    box = Box(tmp_path, ALL_PLACEHOLDERS)
    tarball(box.dist, "cli-linux-riscv64", VERSION)
    done = box.run()
    assert done.returncode == 1
    assert box.published() == []
    assert "cli-linux-riscv64" in done.stdout


def test_a_failed_publish_stops_before_the_launcher(tmp_path):
    box = Box(tmp_path, ALL_PLACEHOLDERS)
    done = box.run(FAKE_PUBLISH_FAILS="1")
    assert done.returncode != 0
    assert box.published() == [f"filedrawer-{PLATFORMS[0]}-{VERSION}.tgz"]


@pytest.mark.parametrize("flag", ["-h", "--help"])
def test_help_says_what_it_does(flag):
    done = subprocess.run(
        [str(SCRIPT), flag], capture_output=True, text=True, check=False
    )
    assert done.returncode == 0
    assert done.stdout.startswith("usage: bin/npm-release DIR VERSION")
