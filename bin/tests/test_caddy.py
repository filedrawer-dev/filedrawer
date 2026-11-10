"""Tests for bin/caddy, the pinned Caddy the real-Caddy tests run, and for conftest's
installed_caddy, which those tests get it through.

The CLI tests serve a fake release archive over file:// or 127.0.0.1, whose `caddy`
is a shell script that prints its arguments, and point the script at it with the two
override variables. No test here reaches the network.
"""

import hashlib
import importlib.machinery
import importlib.util
import io
import os
import subprocess
import sys
import tarfile
from pathlib import Path

import pytest

from conftest import CADDY_UNAVAILABLE, installed_caddy

SCRIPT = Path(__file__).resolve().parents[1] / "caddy"


def load_script():
    loader = importlib.machinery.SourceFileLoader("caddy_script", str(SCRIPT))
    spec = importlib.util.spec_from_loader("caddy_script", loader)
    module = importlib.util.module_from_spec(spec)
    loader.exec_module(module)
    return module


CADDY = load_script()


def fake_archive(path: Path, greeting: str = "fake caddy", member="caddy") -> str:
    """Write a release archive, laid out as Caddy's are, whose caddy prints greeting
    and its arguments. Returns the archive's SHA-512."""
    script = f'#!/bin/sh\necho "{greeting}: $*"\nexit "${{FAKE_CADDY_EXIT:-0}}"\n'
    with tarfile.open(path, "w:gz") as tar:
        for name, data, mode in [
            ("LICENSE", b"licence\n", 0o644),
            ("README.md", b"readme\n", 0o644),
            (member, script.encode(), 0o755),
        ]:
            info = tarfile.TarInfo(name)
            info.size = len(data)
            info.mode = mode
            tar.addfile(info, io.BytesIO(data))
    return hashlib.sha512(path.read_bytes()).hexdigest()


@pytest.fixture
def archive(tmp_path: Path) -> Path:
    return tmp_path / "release.tar.gz"


@pytest.fixture
def env(tmp_path: Path, archive: Path) -> dict[str, str]:
    sha = fake_archive(archive)
    return {
        **os.environ,
        "XDG_CACHE_HOME": str(tmp_path / "cache"),
        "FILEDRAWER_CADDY_URL": archive.as_uri(),
        "FILEDRAWER_CADDY_SHA512": sha,
    }


def run(env: dict[str, str], *args: str) -> subprocess.CompletedProcess:
    return subprocess.run(
        [str(SCRIPT), *args], env=env, capture_output=True, text=True, check=False
    )


def assert_refused(
    result: subprocess.CompletedProcess, env: dict[str, str], status: int
) -> None:
    """caddy didn't run, bin/caddy said why, and nothing was cached."""
    assert result.returncode == status, result.stderr
    assert "fake caddy" not in result.stdout
    assert result.stderr.startswith("bin/caddy: ")
    assert not Path(env["XDG_CACHE_HOME"]).exists()


def test_first_run_downloads_checks_and_runs_caddy_with_its_arguments(env):
    result = run(env, "run", "--config", "a b/Caddyfile")
    assert result.returncode == 0, result.stderr
    assert result.stdout == "fake caddy: run --config a b/Caddyfile\n"


def test_caddy_exit_status_passes_through(env):
    result = run({**env, "FAKE_CADDY_EXIT": "2"}, "validate")
    assert result.returncode == 2
    assert result.stdout == "fake caddy: validate\n"


def test_cached_caddy_runs_without_downloading_again(env, archive):
    assert run(env, "first").returncode == 0
    archive.unlink()
    result = run(env, "second")
    assert result.returncode == 0, result.stderr
    assert result.stdout == "fake caddy: second\n"


def test_hash_mismatch_fails_and_caches_nothing(env):
    result = run({**env, "FILEDRAWER_CADDY_SHA512": "0" * 128}, "version")
    assert_refused(result, env, 1)
    assert "SHA-512" in result.stderr


def test_an_archive_without_caddy_fails_and_caches_nothing(env, archive):
    sha = fake_archive(archive, member="caddy-renamed")
    result = run({**env, "FILEDRAWER_CADDY_SHA512": sha}, "version")
    assert_refused(result, env, 1)


def test_a_download_that_cant_connect_says_caddy_is_unavailable(env):
    # Port 9 on loopback (discard) has nothing listening, so curl can't connect.
    url = "http://127.0.0.1:9/caddy.tar.gz"
    result = run({**env, "FILEDRAWER_CADDY_URL": url}, "version")
    assert_refused(result, env, CADDY.UNAVAILABLE)


def test_a_missing_file_says_caddy_is_unavailable(env, tmp_path):
    missing = (tmp_path / "missing.tar.gz").as_uri()
    result = run({**env, "FILEDRAWER_CADDY_URL": missing}, "version")
    assert_refused(result, env, CADDY.UNAVAILABLE)


# A server on port 0 that answers every GET with the status in argv[1], and prints its
# port once it listens.
STATUS_SERVER = """
import http.server, sys
status = int(sys.argv[1])
class Handler(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        self.send_error(status)
    def log_message(self, *args):
        pass
server = http.server.HTTPServer(("127.0.0.1", 0), Handler)
print(server.server_address[1], flush=True)
server.serve_forever()
"""


def download_answered(env: dict[str, str], status: int) -> subprocess.CompletedProcess:
    """bin/caddy's run when the release URL answers with HTTP status."""
    server = subprocess.Popen(
        [sys.executable, "-I", "-c", STATUS_SERVER, str(status)],
        stdout=subprocess.PIPE,
        text=True,
    )
    try:
        port = server.stdout.readline().strip()
        url = f"http://127.0.0.1:{port}/caddy.tar.gz"
        return run({**env, "FILEDRAWER_CADDY_URL": url}, "version")
    finally:
        server.terminate()
        server.wait()


@pytest.mark.parametrize("status", [404, 410])
def test_a_url_that_isnt_there_fails_rather_than_passing_for_no_network(env, status):
    """A 404 means a wrong pin or a renamed asset, which no skip should hide."""
    result = download_answered(env, status)
    assert_refused(result, env, 1)
    assert str(status) in result.stderr


@pytest.mark.parametrize("status", [403, 429, 500, 503])
def test_a_refused_or_failing_download_says_caddy_is_unavailable(env, status):
    """GitHub down or rate-limiting, or a firewall refusing, isn't a wrong pin."""
    result = download_answered(env, status)
    assert_refused(result, env, CADDY.UNAVAILABLE)
    assert str(status) in result.stderr


def test_curl_missing_says_caddy_is_unavailable(env, tmp_path):
    bare = tmp_path / "bare-bin"
    bare.mkdir()
    (bare / "python3").symlink_to(sys.executable)
    result = run({**env, "PATH": str(bare)}, "version")
    assert_refused(result, env, CADDY.UNAVAILABLE)
    assert "curl" in result.stderr


def test_overridden_url_alone_is_still_checked_against_the_pinned_hash(env):
    del env["FILEDRAWER_CADDY_SHA512"]
    result = run(env, "version")
    assert_refused(result, env, 1)
    assert "SHA-512" in result.stderr


def test_a_new_pin_never_runs_the_binary_cached_under_the_old_one(env, tmp_path):
    assert run(env, "x").stdout == "fake caddy: x\n"
    newer = tmp_path / "newer.tar.gz"
    sha = fake_archive(newer, greeting="newer caddy")
    repinned = {
        **env,
        "FILEDRAWER_CADDY_URL": newer.as_uri(),
        "FILEDRAWER_CADDY_SHA512": sha,
    }
    result = run(repinned, "x")
    assert result.returncode == 0, result.stderr
    assert result.stdout == "newer caddy: x\n"


@pytest.mark.parametrize(
    "system, machine, expected",
    [
        ("Linux", "x86_64", "linux_amd64"),
        ("Linux", "aarch64", "linux_arm64"),
        ("Darwin", "arm64", "mac_arm64"),
        ("Darwin", "x86_64", "mac_amd64"),
        ("Windows", "AMD64", None),
        ("Linux", "riscv64", None),
    ],
)
def test_platform_maps_to_its_release_target(system, machine, expected):
    assert CADDY.target(system, machine) == expected


def test_every_target_has_a_pinned_sha512():
    assert set(CADDY.PINS) == set(CADDY.TARGETS.values())
    for pin in CADDY.PINS.values():
        assert len(pin) == 128
        int(pin, 16)


# installed_caddy, which the real_caddy fixture calls


def test_conftest_and_bin_caddy_agree_on_the_unavailable_status():
    assert CADDY_UNAVAILABLE == CADDY.UNAVAILABLE


def fake_bin_caddy(tmp_path: Path, status: int) -> Path:
    script = tmp_path / "caddy"
    script.write_text(
        f'#!/bin/sh\necho "$*" >> "{tmp_path}/calls"\n'
        f'echo "bin/caddy: why {status}" >&2\nexit {status}\n'
    )
    script.chmod(0o755)
    return script


@pytest.fixture
def no_override(monkeypatch):
    monkeypatch.delenv("FILEDRAWER_TEST_CADDY", raising=False)
    monkeypatch.delenv("CI", raising=False)


def test_installed_caddy_installs_through_bin_caddy_and_returns_it(
    tmp_path, no_override
):
    script = fake_bin_caddy(tmp_path, 0)
    assert installed_caddy(script) == str(script)
    assert (tmp_path / "calls").read_text() == "version\n"


def test_installed_caddy_skips_when_caddy_is_unavailable_and_says_why(
    tmp_path, no_override
):
    script = fake_bin_caddy(tmp_path, CADDY.UNAVAILABLE)
    with pytest.raises(pytest.skip.Exception, match=f"why {CADDY.UNAVAILABLE}"):
        installed_caddy(script)


def test_installed_caddy_fails_when_unavailable_in_ci(
    tmp_path, no_override, monkeypatch
):
    """CI always has the network, so there a missing Caddy fails rather than letting
    the real-Caddy tests quietly skip."""
    monkeypatch.setenv("CI", "true")
    script = fake_bin_caddy(tmp_path, CADDY.UNAVAILABLE)
    with pytest.raises(AssertionError, match=f"why {CADDY.UNAVAILABLE}"):
        installed_caddy(script)


def test_installed_caddy_fails_on_a_wrong_pin(tmp_path, no_override):
    script = fake_bin_caddy(tmp_path, 1)
    with pytest.raises(AssertionError, match="why 1"):
        installed_caddy(script)


def test_filedrawer_test_caddy_overrides_without_running_bin_caddy(
    tmp_path, no_override, monkeypatch
):
    monkeypatch.setenv("FILEDRAWER_TEST_CADDY", "/opt/caddy")
    script = fake_bin_caddy(tmp_path, 0)
    assert installed_caddy(script) == "/opt/caddy"
    assert not (tmp_path / "calls").exists()
