"""Tests for conftest's no_network_tools guard: a test that shells out to gh, ssh,
scp, sftp or curl gets a refusal, not the real tool, unless it uses fake_host."""

import subprocess

import pytest


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
