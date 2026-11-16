"""Tests for bin/pypi-release, which the release run's publish job runs (#87): it
checks DIR holds the sdist and the wheels of one version, asks PyPI which of them
it has, and uploads the rest, the sdist first.

They run it against fakes of curl, answering pypi.org from a made-up state, and of
pipx, which records each upload.
"""

import json
import os
import subprocess
from pathlib import Path

import pytest

REPO = Path(__file__).resolve().parents[2]
SCRIPT = REPO / "bin/pypi-release"
VERSION = "0.4.0"
SDIST = f"filedrawer-{VERSION}.tar.gz"
WHEELS = [
    f"filedrawer-{VERSION}-py3-none-{tag}.whl"
    for tag in (
        "macosx_10_12_x86_64",
        "macosx_11_0_arm64",
        "manylinux_2_17_aarch64.manylinux2014_aarch64.musllinux_1_2_aarch64",
        "manylinux_2_17_x86_64.manylinux2014_x86_64.musllinux_1_2_x86_64",
        "win_amd64",
    )
]

FAKE = r'''#!/usr/bin/env python3
"""curl or pipx, by the name it was called with, against the state in $FAKE_STATE."""
import os, sys
from pathlib import Path

state = Path(os.environ["FAKE_STATE"])
tool, args = Path(sys.argv[0]).name, sys.argv[1:]
with (state / "log").open("a") as log:
    log.write(" ".join([tool, *args]) + "\n")

if tool == "pipx":
    sys.exit(1 if os.environ.get("FAKE_UPLOAD_FAILS") else 0)

url = args[-1]
out = Path(args[args.index("-o") + 1])
if os.environ.get("FAKE_PYPI_DOWN"):
    sys.stdout.write("503")
    sys.exit(0)
if not url.startswith("https://pypi.org/pypi/filedrawer/") or not url.endswith("/json"):
    sys.exit(6)
answer = state / "pypi.json"
if answer.exists():
    out.write_bytes(answer.read_bytes())
    sys.stdout.write("200")
else:
    sys.stdout.write("404")
'''


class Box:
    """A dist folder with a whole release's files, and a PyPI that has `has` of them
    (None: it doesn't have the version)."""

    def __init__(self, root: Path, has: list[str] | None):
        self.dist = root / "dist"
        self.dist.mkdir()
        for name in [SDIST, *WHEELS]:
            (self.dist / name).write_text(name)
        self.state = root / "state"
        self.state.mkdir()
        (self.state / "log").touch()
        if has is not None:
            self.answer(json.dumps({"urls": [{"filename": name} for name in has]}))
        fakes = root / "fakes"
        fakes.mkdir()
        for tool in ("curl", "pipx"):
            (fakes / tool).write_text(FAKE)
            (fakes / tool).chmod(0o755)
        self.env = {
            **os.environ,
            "PATH": f"{fakes}{os.pathsep}{os.environ['PATH']}",
            "FAKE_STATE": str(self.state),
        }

    def answer(self, body: str) -> None:
        (self.state / "pypi.json").write_text(body)

    def run(self, version: str = VERSION, count: str = "6", **env: str):
        return subprocess.run(
            [str(SCRIPT), str(self.dist), version, count],
            cwd=self.dist.parent,
            env={**self.env, **env},
            capture_output=True,
            text=True,
            check=False,
        )

    def uploads(self) -> list[list[str]]:
        """The file names of each upload, one list a call."""
        return [
            [Path(word).name for word in line.split() if "filedrawer-" in word]
            for line in (self.state / "log").read_text().splitlines()
            if line.startswith("pipx")
        ]

    def asked(self) -> list[str]:
        return [
            line.split()[-1]
            for line in (self.state / "log").read_text().splitlines()
            if line.startswith("curl")
        ]


def test_a_version_pypi_lacks_gets_every_file_the_sdist_first(tmp_path):
    box = Box(tmp_path, None)
    done = box.run()
    assert done.returncode == 0, done.stdout + done.stderr
    assert box.asked() == [f"https://pypi.org/pypi/filedrawer/{VERSION}/json"]
    # uv sorts the files of one call by name, which puts the sdist last, so it
    # gets a call of its own.
    assert box.uploads() == [[SDIST], sorted(WHEELS)]
    log = (box.state / "log").read_text()
    assert "uv==0.13.5 publish --trusted-publishing always" in log


def test_a_rerun_after_a_partial_upload_uploads_only_the_files_pypi_lacks(tmp_path):
    box = Box(tmp_path, WHEELS[:2])
    done = box.run()
    assert done.returncode == 0, done.stdout + done.stderr
    assert box.uploads() == [[SDIST], sorted(WHEELS[2:])]


def test_a_version_pypi_has_whole_uploads_nothing_and_passes(tmp_path):
    box = Box(tmp_path, [SDIST, *WHEELS])
    done = box.run()
    assert done.returncode == 0, done.stdout + done.stderr
    assert box.uploads() == []
    assert "::notice::" in done.stdout and VERSION in done.stdout


def test_a_file_only_pypi_has_is_a_warning_and_the_rest_still_uploads(tmp_path):
    stray = f"filedrawer-{VERSION}-py3-none-linux_riscv64.whl"
    box = Box(tmp_path, [SDIST, stray])
    done = box.run()
    assert done.returncode == 0, done.stdout + done.stderr
    assert "::warning::" in done.stdout and stray in done.stdout
    assert box.uploads() == [sorted(WHEELS)]


@pytest.mark.parametrize(
    "change",
    [
        lambda dist: (dist / WHEELS[0]).unlink(),
        lambda dist: (dist / SDIST).unlink(),
        lambda dist: (dist / "filedrawer-0.3.9-py3-none-win_amd64.whl").write_text(""),
        lambda dist: (dist / "notes.txt").write_text(""),
        lambda dist: (dist / WHEELS[0]).rename(dist / f"filedrawer-{VERSION}.zip"),
    ],
    ids=["a wheel missing", "no sdist", "another version", "a stray file", "a zip"],
)
def test_a_dist_that_isnt_the_whole_release_uploads_nothing(tmp_path, change):
    box = Box(tmp_path, None)
    change(box.dist)
    done = box.run()
    assert done.returncode == 1, done.stdout + done.stderr
    assert "::error::" in done.stdout
    assert box.uploads() == [] and box.asked() == []


def test_only_pypis_404_counts_as_absent(tmp_path):
    box = Box(tmp_path, None)
    done = box.run(FAKE_PYPI_DOWN="1")
    assert done.returncode == 1, done.stdout + done.stderr
    assert "::error::" in done.stdout and "503" in done.stdout
    assert box.uploads() == []


@pytest.mark.parametrize(
    "body",
    [
        "<html>",
        "[]",
        '{"urls": {}}',
        '{"urls": [{"name": "x"}]}',
        '{"urls": [3]}',
        '{"urls": [{"filename": 3}]}',
    ],
)
def test_an_answer_in_another_shape_is_refused_in_a_line(tmp_path, body):
    box = Box(tmp_path, [])
    box.answer(body)
    done = box.run()
    assert done.returncode == 1, done.stdout + done.stderr
    assert "::error::" in done.stdout and "Traceback" not in done.stderr
    assert box.uploads() == []


def test_a_failed_upload_fails_and_names_pypis_cache(tmp_path):
    box = Box(tmp_path, WHEELS[:1])
    done = box.run(FAKE_UPLOAD_FAILS="1")
    assert done.returncode == 1, done.stdout + done.stderr
    assert "::error::" in done.stdout and "15 minutes" in done.stdout
    # No wheel goes up once the sdist's upload failed.
    assert box.uploads() == [[SDIST]]


@pytest.mark.parametrize(
    "args", [[], ["dist"], ["dist", VERSION], ["dist", VERSION, "x"]]
)
def test_it_takes_a_folder_a_version_and_a_count(tmp_path, args):
    done = subprocess.run(
        [str(SCRIPT), *args], cwd=tmp_path, capture_output=True, text=True, check=False
    )
    assert done.returncode == 2, done.stdout + done.stderr
    assert "usage" in done.stderr.lower()


def test_help_says_what_it_does():
    done = subprocess.run(
        [str(SCRIPT), "-h"], capture_output=True, text=True, check=False
    )
    assert done.returncode == 0
    assert "PyPI lacks" in done.stdout
