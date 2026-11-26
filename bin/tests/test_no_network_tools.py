"""Tests for conftest's no_network_tools guard: a test that shells out to gh, ssh,
scp, sftp or curl gets a refusal, not the real tool, unless it uses fake_host."""

import shutil
import subprocess
from pathlib import Path

import pytest
from conftest import NETWORK_TOOLS, OPENERS, isolated


@pytest.fixture(scope="module")
def module_lookups(tmp_path_factory) -> dict[str, Path]:
    """Where each network tool and opener resolves for a module's fixture under
    isolated(). pytest sets up a module's fixtures before each test's autouse ones,
    so without it these would be the real tools."""
    with isolated(tmp_path_factory):
        found = {tool: shutil.which(tool) for tool in (*NETWORK_TOOLS, *OPENERS)}
    return {tool: Path(path) for tool, path in found.items() if path}


@pytest.mark.parametrize("tool", [*NETWORK_TOOLS, *OPENERS])
def test_a_module_fixture_under_isolated_gets_the_fakes(module_lookups, tool):
    assert module_lookups[tool].parent.name.startswith(("no-network", "no-browser"))


@pytest.mark.parametrize(
    "argv",
    [
        ["gh", "secret", "set", "X", "--env", "staging"],
        ["ssh", "root@example.com", "true"],
        ["scp", "a", "root@example.com:b"],
        ["sftp", "root@example.com"],
        ["curl", "-s", "https://example.com/"],
        # With no scheme, curl takes a host and fetches it over http.
        ["curl", "-s", "example.com"],
        ["curl", "-s", "staging.filedrawer.dev/tags"],
        # A loopback address before an @ is a user name: the host is evil.com.
        ["curl", "-s", "http://127.0.0.1:1@evil.com/"],
    ],
)
def test_a_network_tool_is_refused(argv):
    done = subprocess.run(argv, capture_output=True, text=True, check=False, input="")
    assert done.returncode == 97, done.stdout + done.stderr
    assert "fake_host" in done.stderr


def test_curl_of_a_local_file_still_works(tmp_path):
    target = tmp_path / "x.txt"
    target.write_text("local\n")
    out = tmp_path / "out.json"
    done = subprocess.run(
        ["curl", "-s", "-o", str(out), f"file://{target}"],
        capture_output=True,
        text=True,
        check=False,
    )
    assert done.returncode == 0, done.stderr
    assert out.read_text() == "local\n"


def test_fake_host_still_answers(fake_host):
    done = subprocess.run(
        ["gh", "auth", "status"], env=fake_host.env, capture_output=True, check=False
    )
    assert done.returncode == 0
