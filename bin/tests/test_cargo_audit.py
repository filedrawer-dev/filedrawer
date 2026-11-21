"""Tests for bin/cargo-audit, the pinned cargo-audit bin/advisories runs.

As test_lychee.py does for bin/lychee, they serve a fake release archive over
file://, whose `cargo-audit` is a shell script that prints its arguments, and point
the script at it with the two override variables.
"""

import hashlib
import io
import os
import platform
import re
import subprocess
import tarfile
from pathlib import Path

import pytest

SCRIPT = Path(__file__).resolve().parents[1] / "cargo-audit"
SOURCE = SCRIPT.read_text()
VERSION = re.search(r'^VERSION = "(.*)"$', SOURCE, re.M).group(1)
# The release's name for this machine's build; None where the release has none.
TARGET = {
    ("Linux", "x86_64"): "x86_64-unknown-linux-musl",
    ("Linux", "aarch64"): "aarch64-unknown-linux-gnu",
    ("Darwin", "arm64"): "aarch64-apple-darwin",
    ("Darwin", "x86_64"): "x86_64-apple-darwin",
}.get((platform.system(), platform.machine()))

pytestmark = pytest.mark.skipif(
    TARGET is None, reason="cargo-audit has no release for this platform"
)


def fake_archive(path: Path, greeting: str = "fake cargo-audit") -> str:
    """Write a release archive whose cargo-audit prints greeting and its arguments.

    Returns the archive's SHA-256.
    """
    script = f'#!/bin/sh\necho "{greeting}: $*"\nexit "${{FAKE_AUDIT_EXIT:-0}}"\n'
    top = f"cargo-audit-{TARGET}-v{VERSION}"
    with tarfile.open(path, "w:gz") as tar:
        for name, data, mode in [
            (f"{top}/README.md", b"readme\n", 0o644),
            (f"{top}/cargo-audit", script.encode(), 0o755),
        ]:
            info = tarfile.TarInfo(name)
            info.size = len(data)
            info.mode = mode
            tar.addfile(info, io.BytesIO(data))
    return hashlib.sha256(path.read_bytes()).hexdigest()


@pytest.fixture
def archive(tmp_path: Path) -> Path:
    return tmp_path / "release.tgz"


@pytest.fixture
def env(tmp_path: Path, archive: Path) -> dict[str, str]:
    sha = fake_archive(archive)
    return {
        **os.environ,
        "XDG_CACHE_HOME": str(tmp_path / "cache"),
        "FILEDRAWER_CARGO_AUDIT_URL": archive.as_uri(),
        "FILEDRAWER_CARGO_AUDIT_SHA256": sha,
    }


def run(env: dict[str, str], *args: str) -> subprocess.CompletedProcess:
    return subprocess.run(
        [str(SCRIPT), *args], env=env, capture_output=True, text=True, check=False
    )


def assert_refused(result: subprocess.CompletedProcess, env: dict[str, str]) -> None:
    """cargo-audit didn't run, and nothing was cached."""
    assert result.returncode == 1
    assert "fake cargo-audit" not in result.stdout
    assert result.stderr.startswith("bin/cargo-audit: ")
    assert not Path(env["XDG_CACHE_HOME"]).exists()


def test_first_run_downloads_checks_and_runs_cargo_audit_with_its_arguments(env):
    result = run(env, "audit", "--json")
    assert result.returncode == 0, result.stderr
    assert result.stdout == "fake cargo-audit: audit --json\n"


def test_cargo_audits_exit_status_passes_through(env):
    result = run({**env, "FAKE_AUDIT_EXIT": "2"}, "audit")
    assert (result.returncode, result.stdout) == (2, "fake cargo-audit: audit\n")


def test_a_cached_cargo_audit_runs_without_downloading_again(env, archive):
    assert run(env, "first").returncode == 0
    archive.unlink()
    result = run(env, "second")
    assert result.returncode == 0, result.stderr
    assert result.stdout == "fake cargo-audit: second\n"


def test_a_hash_mismatch_refuses_and_caches_nothing(env):
    result = run({**env, "FILEDRAWER_CARGO_AUDIT_SHA256": "0" * 64}, "audit")
    assert_refused(result, env)
    assert "SHA-256" in result.stderr


def test_a_failed_download_refuses(env, tmp_path):
    missing = (tmp_path / "missing.tgz").as_uri()
    assert_refused(run({**env, "FILEDRAWER_CARGO_AUDIT_URL": missing}, "audit"), env)


def test_an_overridden_url_alone_is_still_checked_against_the_pinned_hash(env):
    del env["FILEDRAWER_CARGO_AUDIT_SHA256"]
    result = run(env, "audit")
    assert_refused(result, env)
    assert "SHA-256" in result.stderr


def test_an_archive_without_the_binary_refuses(env, archive):
    with tarfile.open(archive, "w:gz") as tar:
        info = tarfile.TarInfo("elsewhere/cargo-audit")
        tar.addfile(info, io.BytesIO(b""))
    sha = hashlib.sha256(archive.read_bytes()).hexdigest()
    result = run({**env, "FILEDRAWER_CARGO_AUDIT_SHA256": sha}, "audit")
    assert_refused(result, env)
    assert "has no file" in result.stderr


def test_a_new_pin_never_runs_the_binary_cached_under_the_old_one(env, tmp_path):
    assert run(env, "audit").stdout == "fake cargo-audit: audit\n"
    newer = tmp_path / "newer.tgz"
    sha = fake_archive(newer, greeting="newer cargo-audit")
    repinned = {
        **env,
        "FILEDRAWER_CARGO_AUDIT_URL": newer.as_uri(),
        "FILEDRAWER_CARGO_AUDIT_SHA256": sha,
    }
    result = run(repinned, "audit")
    assert result.returncode == 0, result.stderr
    assert result.stdout == "newer cargo-audit: audit\n"


def test_help_says_what_it_runs_and_downloads_nothing(env):
    result = run(env, "-h")
    assert result.returncode == 0
    assert "pinned by SHA-256" in result.stdout
    assert not Path(env["XDG_CACHE_HOME"]).exists()


def test_every_release_target_has_a_pin():
    targets = set(re.findall(r'\("\w+", "\w+"\): "([\w-]+)"', SOURCE))
    pinned = dict(re.findall(r'^    "([\w-]+)": "([0-9a-f]{64})",$', SOURCE, re.M))
    assert targets and targets == set(pinned)
