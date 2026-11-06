"""Tests for conftest's fake_host: the fake ssh, gh, getent, curl and ssh-keygen the
wizard tests run ops/provision-vps against."""

import subprocess


def call(fake_host, *argv, stdin=""):
    return subprocess.run(
        list(argv),
        input=stdin,
        env=fake_host.env,
        capture_output=True,
        text=True,
        check=False,
    )


def test_ssh_logs_each_command_line_and_keeps_each_calls_stdin(fake_host):
    call(fake_host, "ssh", "-o", "BatchMode=yes", "root@h", "bash -s", stdin="one\n")
    call(fake_host, "ssh", "root@h", "test -f /x", stdin="")
    call(fake_host, "ssh", "root@h", "env A=b bash -s", stdin="three\n")
    assert fake_host.calls() == ["bash -s", "test -f /x", "env A=b bash -s"]
    assert [fake_host.sent(n) for n in (1, 2, 3)] == ["one\n", "", "three\n"]
    assert fake_host.log.read_text().splitlines()[0] == "ssh bash -s"


def test_ssh_joins_the_words_after_the_host_as_the_server_would(fake_host):
    call(fake_host, "ssh", "-o", "BatchMode=yes", "-p", "22", "root@h", "cat", "/etc/x")
    assert fake_host.calls() == ["cat /etc/x"]


def test_an_unmatched_call_prints_nothing_and_succeeds(fake_host):
    result = call(fake_host, "ssh", "root@h", "anything")
    assert (result.returncode, result.stdout) == (0, "")


def test_ssh_answers_from_the_latest_rule_that_matches(fake_host):
    fake_host.answer("test -f *", status=1)
    fake_host.answer("test -f /etc/kept", status=0)
    fake_host.answer("*os-release*", stdout="ID=debian\n")
    assert call(fake_host, "ssh", "h", "test -f /etc/other").returncode == 1
    assert call(fake_host, "ssh", "h", "test -f /etc/kept").returncode == 0
    assert call(fake_host, "ssh", "h", "grep x /etc/os-release").stdout == "ID=debian\n"


def test_a_rule_can_match_on_what_the_call_sends(fake_host):
    fake_host.answer("bash -s", stdout="token\n", stdin="create-token")
    assert call(fake_host, "ssh", "h", "bash -s", stdin="x create-token y").stdout == (
        "token\n"
    )
    assert call(fake_host, "ssh", "h", "bash -s", stdin="other").stdout == ""


def test_run_locally_runs_the_command_line_here_on_its_stdin(fake_host):
    fake_host.run_locally()
    result = call(
        fake_host, "ssh", "h", 'read -r v; echo "[$v]"; exit 4', stdin="a b\n"
    )
    assert (result.returncode, result.stdout) == (4, "[a b]\n")
    assert fake_host.sent(1) == "a b\n"


def test_other_tools_answer_from_their_own_rules(fake_host):
    fake_host.answer("ahostsv4 *", stdout="203.0.113.7 STREAM x\n", tool="getent")
    fake_host.answer("*robots.txt*", stdout="200", tool="curl")
    assert call(fake_host, "getent", "ahostsv4", "x").stdout == "203.0.113.7 STREAM x\n"
    assert call(fake_host, "curl", "-s", "https://h/robots.txt").stdout == "200"
    assert call(fake_host, "curl", "-s", "https://h/").stdout == ""
    assert call(fake_host, "ssh", "h", "ahostsv4 x").stdout == ""
    assert fake_host.calls("curl") == ["-s https://h/robots.txt", "-s https://h/"]


def test_gh_keeps_the_staging_secrets_it_is_given(fake_host):
    assert call(fake_host, "gh", "auth", "status").returncode == 0
    call(fake_host, "gh", "secret", "set", "B_KEY", "--env", "staging", stdin="s3cret")
    call(fake_host, "gh", "secret", "set", "A_KEY", "--env", "staging", stdin="other")
    assert (fake_host.secrets / "B_KEY").read_text() == "s3cret"
    assert call(fake_host, "gh", "secret", "list").stdout == "A_KEY\nB_KEY\n"
    assert "s3cret" not in fake_host.log.read_text()
