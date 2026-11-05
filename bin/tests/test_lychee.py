"""Tests for bin/lychee, the pinned lychee the commit hook runs.

The CLI tests serve a fake release archive over file://, whose `lychee` is a shell
script that prints its arguments, and point the script at it with the two override
variables. Run them with `uvx pytest bin/tests`.
"""

import hashlib
import importlib.machinery
import importlib.util
import io
import os
import platform
import subprocess
import tarfile
from pathlib import Path

import pytest

SCRIPT = Path(__file__).resolve().parents[1] / "lychee"


def load_script():
    loader = importlib.machinery.SourceFileLoader("lychee_hook", str(SCRIPT))
    spec = importlib.util.spec_from_loader("lychee_hook", loader)
    module = importlib.util.module_from_spec(spec)
    loader.exec_module(module)
    return module


HOOK = load_script()
FAKE_LYCHEE = b'#!/bin/sh\necho "fake lychee: $*"\nexit "${FAKE_LYCHEE_EXIT:-0}"\n'


def fake_archive(path: Path) -> str:
    """Write a release archive holding the fake lychee; return its SHA-256."""
    top = f"lychee-{HOOK.target(platform.system(), platform.machine())}"
    with tarfile.open(path, "w:gz") as tar:
        for name, data, mode in [
            (f"{top}/README.md", b"readme\n", 0o644),
            (f"{top}/lychee", FAKE_LYCHEE, 0o755),
        ]:
            info = tarfile.TarInfo(name)
            info.size = len(data)
            info.mode = mode
            tar.addfile(info, io.BytesIO(data))
    return hashlib.sha256(path.read_bytes()).hexdigest()


@pytest.fixture
def env(tmp_path: Path) -> dict[str, str]:
    archive = tmp_path / "release.tar.gz"
    sha = fake_archive(archive)
    return {
        **os.environ,
        "XDG_CACHE_HOME": str(tmp_path / "cache"),
        "FILEDRAWER_LYCHEE_URL": archive.as_uri(),
        "FILEDRAWER_LYCHEE_SHA256": sha,
    }


def run(env: dict[str, str], *args: str) -> subprocess.CompletedProcess:
    return subprocess.run(
        [str(SCRIPT), *args], env=env, capture_output=True, text=True, check=False
    )


def cached_files(env: dict[str, str]) -> list[Path]:
    cache = Path(env["XDG_CACHE_HOME"])
    return sorted(p for p in cache.rglob("*") if p.is_file()) if cache.exists() else []


def test_first_run_downloads_checks_and_runs_lychee_with_its_arguments(env):
    result = run(env, "--offline", "docs/a b.md")
    assert result.returncode == 0, result.stderr
    assert result.stdout == "fake lychee: --offline docs/a b.md\n"


def test_lychee_exit_status_passes_through(env):
    result = run({**env, "FAKE_LYCHEE_EXIT": "2"}, "x.md")
    assert result.returncode == 2
    assert result.stdout == "fake lychee: x.md\n"


def test_cached_lychee_runs_without_downloading_again(env):
    assert run(env, "first").returncode == 0
    Path(env["FILEDRAWER_LYCHEE_URL"].removeprefix("file://")).unlink()
    result = run(env, "second")
    assert result.returncode == 0, result.stderr
    assert result.stdout == "fake lychee: second\n"


def test_hash_mismatch_refuses_and_caches_nothing(env):
    result = run({**env, "FILEDRAWER_LYCHEE_SHA256": "0" * 64}, "x.md")
    assert result.returncode != 0
    assert "fake lychee" not in result.stdout
    assert "SHA-256" in result.stderr
    assert "SKIP=lychee" in result.stderr
    assert cached_files(env) == []


def test_failed_download_refuses_with_the_skip_hint(env, tmp_path):
    missing = (tmp_path / "missing.tar.gz").as_uri()
    result = run({**env, "FILEDRAWER_LYCHEE_URL": missing}, "x.md")
    assert result.returncode != 0
    assert "fake lychee" not in result.stdout
    assert "SKIP=lychee" in result.stderr
    assert cached_files(env) == []


def test_overridden_url_alone_is_still_checked_against_the_pinned_hash(env):
    del env["FILEDRAWER_LYCHEE_SHA256"]
    result = run(env, "x.md")
    assert result.returncode != 0
    assert "fake lychee" not in result.stdout
    assert "SHA-256" in result.stderr
    assert cached_files(env) == []


@pytest.mark.parametrize(
    "system, machine, expected",
    [
        ("Linux", "x86_64", "x86_64-unknown-linux-musl"),
        ("Linux", "amd64", "x86_64-unknown-linux-musl"),
        ("Linux", "aarch64", "aarch64-unknown-linux-musl"),
        ("Linux", "arm64", "aarch64-unknown-linux-musl"),
        ("Darwin", "arm64", "aarch64-apple-darwin"),
        ("Darwin", "x86_64", "x86_64-apple-darwin"),
        ("Windows", "AMD64", None),
        ("Linux", "riscv64", None),
    ],
)
def test_platform_maps_to_its_release_target(system, machine, expected):
    assert HOOK.target(system, machine) == expected
