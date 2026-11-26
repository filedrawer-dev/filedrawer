"""Tests for bin/ruff, which runs ruff at the version .pre-commit-config.yaml's hooks
pin. Each runs the script with a fake uvx first on PATH that notes its arguments, so
no test reaches the network; the ones about the config run a copy of the script
beside a scratch config, since it finds the config from its own path."""

import os
import shutil
import subprocess
from pathlib import Path

import pytest

REPO = Path(__file__).resolve().parents[2]
SCRIPT = REPO / "bin/ruff"

FAKE_UVX = """#!/bin/sh
printf '%s\\n' "$@" > "$LOG"
exit "${FAKE_STATUS:-0}"
"""

HOOK = """      - id: {id}
        name: {id}
        entry: uvx ruff@{version} {command}
        language: system
"""


def config(*versions: str) -> str:
    hooks = "".join(
        HOOK.format(id=f"ruff-{n}", version=version, command="check")
        for n, version in enumerate(versions)
    )
    return f"repos:\n  - repo: local\n    hooks:\n{hooks}"


@pytest.fixture
def fake_uvx(tmp_path):
    fakes = tmp_path / "fakes"
    fakes.mkdir()
    (fakes / "uvx").write_text(FAKE_UVX)
    (fakes / "uvx").chmod(0o755)
    log = tmp_path / "uvx.log"
    env = {**os.environ, "PATH": f"{fakes}:{os.environ['PATH']}", "LOG": str(log)}
    return env, log


def run(script: Path, env: dict, *args: str) -> subprocess.CompletedProcess:
    return subprocess.run(
        [str(script), *args], env=env, capture_output=True, text=True, check=False
    )


def copy_beside(tmp_path: Path, config_text: str) -> Path:
    """A copy of the script in a scratch repo's bin/, beside `config_text`."""
    (tmp_path / "repo/bin").mkdir(parents=True)
    shutil.copy(SCRIPT, tmp_path / "repo/bin/ruff")
    (tmp_path / "repo/.pre-commit-config.yaml").write_text(config_text)
    return tmp_path / "repo/bin/ruff"


def test_runs_the_version_the_config_pins_with_every_argument(tmp_path, fake_uvx):
    env, log = fake_uvx
    script = copy_beside(tmp_path, config("9.8.7", "9.8.7"))
    done = run(script, env, "check", "--fix", "a file.py", "-h")
    assert done.returncode == 0, done.stderr
    assert log.read_text().splitlines() == [
        "--quiet",
        "ruff@9.8.7",
        "check",
        "--fix",
        "a file.py",
        "-h",
    ]


@pytest.mark.parametrize("flag", ["-h", "--help"])
def test_help_first_prints_its_own_and_runs_nothing(fake_uvx, flag):
    env, log = fake_uvx
    done = run(SCRIPT, env, flag)
    assert done.returncode == 0, done.stderr
    assert "bin/ruff ARGS..." in done.stdout
    assert not log.exists()


def test_a_missing_config_is_one_line_and_runs_nothing(tmp_path, fake_uvx):
    env, log = fake_uvx
    script = copy_beside(tmp_path, "")
    (tmp_path / "repo/.pre-commit-config.yaml").unlink()
    done = run(script, env, "check")
    assert done.returncode == 2
    assert ".pre-commit-config.yaml" in done.stderr
    assert len(done.stderr.strip().splitlines()) == 1
    assert not log.exists()


def test_exits_with_ruffs_status(tmp_path, fake_uvx):
    env, _ = fake_uvx
    script = copy_beside(tmp_path, config("9.8.7"))
    done = run(script, {**env, "FAKE_STATUS": "1"}, "check")
    assert done.returncode == 1


@pytest.mark.parametrize(
    ("versions", "named"), [((), "none"), (("9.8.8", "9.8.7"), "9.8.7, 9.8.8")]
)
def test_no_pin_or_two_pins_is_refused_and_runs_nothing(
    tmp_path, fake_uvx, versions, named
):
    env, log = fake_uvx
    script = copy_beside(tmp_path, config(*versions))
    done = run(script, env, "check")
    assert done.returncode == 2
    assert ".pre-commit-config.yaml" in done.stderr
    assert f"names {named};" in done.stderr
    assert len(done.stderr.strip().splitlines()) == 1
    assert not log.exists()


def test_an_entry_with_no_version_is_refused_beside_a_pinned_one(tmp_path, fake_uvx):
    env, log = fake_uvx
    unpinned = HOOK.format(id="ruff-bare", version="", command="format").replace(
        "ruff@ ", "ruff "
    )
    script = copy_beside(tmp_path, config("9.8.7") + unpinned)
    done = run(script, env, "check")
    assert done.returncode == 2
    assert "uvx ruff format" in done.stderr
    assert len(done.stderr.strip().splitlines()) == 1
    assert not log.exists()


def test_the_real_script_runs_the_version_every_ruff_hook_pins(fake_uvx):
    env, log = fake_uvx
    done = run(SCRIPT, env, "format", "--check")
    assert done.returncode == 0, done.stderr
    _, ran, *args = log.read_text().splitlines()
    assert args == ["format", "--check"]
    entries = [
        line.strip()
        for line in (REPO / ".pre-commit-config.yaml").read_text().splitlines()
        if line.strip().startswith("entry: uvx ruff@")
    ]
    assert entries
    for entry in entries:
        assert entry.startswith(f"entry: uvx {ran} "), entry
