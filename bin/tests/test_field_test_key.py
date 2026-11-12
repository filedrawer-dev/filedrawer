"""Tests for the field-test key: ops/remote/field-test-key.sh, which writes Caddy's
snippet for it as root on the VPS; caddy.sh's gate, which imports that snippet; and
ops/field-test-key, which the owner runs on their host to open and close a key.

The remote scripts run against a temp root with fakes of caddy, systemctl, runuser,
chown and curl first on PATH. The host script runs against conftest's fake ssh. One
test serves the real snippet and gate with a real Caddy, when there is one.
"""

import json
import os
import re
import socket
import stat
import subprocess
import time
from pathlib import Path
from types import SimpleNamespace

import pytest

REPO = Path(__file__).resolve().parents[2]
REMOTE = REPO / "ops/remote/field-test-key.sh"
CADDY_SH = REPO / "ops/remote/caddy.sh"
HOST = REPO / "ops/field-test-key"
KEY = "c0" * 32
OTHER = "d1" * 32
STAGING_KEY = "ab" * 32
STALE_TEXT = (
    "field-test key not valid here: it is expired or replaced, or the request isn't "
    "a GET or HEAD of a page. Ask the owner to run ops/field-test-key open."
)

# A fake curl: notes its stdin config in the log, one line per call, and answers
# the status of the first [substring, status] pair in $FAKE_CURL_RULES whose
# substring the config holds, else 401.
FAKE_CURL = """#!/usr/bin/env python3
import json, os, sys
config = sys.stdin.read()
with open(os.environ["FAKE_LOG"], "a") as log:
    log.write("curl " + " ".join(sys.argv[1:]) + " <<< " + config.replace("\\n", " | ") + "\\n")
for needle, status in json.loads(os.environ.get("FAKE_CURL_RULES", "[]")):
    if needle in config:
        print(status, end="")
        break
else:
    print(401, end="")
"""


def executable(path: Path, text: str) -> Path:
    path.write_text(text)
    path.chmod(path.stat().st_mode | stat.S_IXUSR)
    return path


def caddyfile_with_staging_key() -> str:
    return (
        "(staging_gate) {\n"
        f'\t@agent header X-Staging-Key "{STAGING_KEY}"\n'
        "\timport /etc/caddy/field-test.caddy\n}\n"
    )


@pytest.fixture
def box(tmp_path):
    root = tmp_path / "root"
    (root / "etc/caddy").mkdir(parents=True)
    (root / "etc/caddy/Caddyfile").write_text(caddyfile_with_staging_key())
    fakes = tmp_path / "fakes"
    fakes.mkdir()
    log = tmp_path / "log"
    log.touch()
    # caddy validate fails when $FAKE_CADDY_FAILS is set; systemctl reload when
    # $FAKE_RELOAD_FAILS is.
    executable(
        fakes / "caddy",
        f'#!/bin/sh\necho "caddy $*" >> {log}\n[ -z "$FAKE_CADDY_FAILS" ]\n',
    )
    executable(
        fakes / "systemctl",
        f'#!/bin/sh\necho "systemctl $*" >> {log}\n[ -z "$FAKE_RELOAD_FAILS" ]\n',
    )
    executable(fakes / "chown", f'#!/bin/sh\necho "chown $*" >> {log}\n')
    executable(
        fakes / "runuser",
        f'#!/bin/sh\necho "runuser $*" >> {log}\nshift 3\nexec "$@"\n',
    )
    executable(fakes / "curl", FAKE_CURL)
    env = {
        **os.environ,
        "PATH": f"{fakes}:{os.environ['PATH']}",
        "FILEDRAWER_ROOT": str(root),
        "FAKE_LOG": str(log),
        "FAKE_CURL_RULES": json.dumps(
            [[f"X-Staging-Key: {STAGING_KEY}", 200], [f"key={KEY}", 200]]
        ),
    }
    return SimpleNamespace(
        root=root, log=log, env=env, snippet=root / "etc/caddy/field-test.caddy"
    )


def remote(box, line: str, **env) -> subprocess.CompletedProcess:
    """Runs the remote script as ops/field-test-key does: `line` read into
    $FIELD_TEST by the remote shell."""
    return subprocess.run(
        ["bash", str(REMOTE)],
        env={**box.env, "FIELD_TEST": line, **env},
        capture_output=True,
        text=True,
        check=False,
    )


def expiry_in(snippet: str) -> int:
    return int(re.search(r">= (\d+)", snippet).group(1))


def test_open_writes_a_snippet_that_lets_reads_of_pages_through_with_the_key(box):
    before = int(time.time())
    result = remote(box, f"open {KEY} 8")

    assert result.returncode == 0, result.stderr
    snippet = box.snippet.read_text()
    assert f"query key={KEY}" in snippet
    assert "method GET HEAD" in snippet
    assert "not path /staging* /admin*" in snippet
    assert "uri query -key" in snippet
    assert "reverse_proxy 127.0.0.1:8081" in snippet
    assert before + 8 * 3600 <= expiry_in(snippet) <= int(time.time()) + 8 * 3600
    match = re.fullmatch(r"expires=(\d{4}-\d\d-\d\dT\d\d:\d\d:\d\dZ)\n", result.stdout)
    assert match, result.stdout
    assert KEY not in result.stdout + result.stderr


def test_the_expiry_check_sits_inside_the_keyed_handle_so_only_keyed_requests_run_it(
    box,
):
    remote(box, f"open {KEY} 1")

    snippet = box.snippet.read_text()
    outer = snippet[: snippet.index("handle @field_test {")]
    assert "expression" not in outer
    assert STALE_TEXT in snippet


def test_open_validates_as_the_caddy_user_then_reloads(box):
    remote(box, f"open {KEY} 1")

    log = box.log.read_text()
    assert "runuser -u caddy -- caddy validate" in log
    assert log.index("caddy validate") < log.index("systemctl reload caddy")
    assert "chown root:caddy" in log
    assert stat.S_IMODE(box.snippet.stat().st_mode) == 0o640


def test_it_probes_caddy_locally_with_the_key_on_stdin_never_in_an_argument(box):
    remote(box, f"open {KEY} 1")

    calls = [
        line for line in box.log.read_text().splitlines() if line.startswith("curl ")
    ]
    assert calls, box.log.read_text()
    for call in calls:
        arguments = call.split(" <<< ")[0]
        assert KEY not in arguments and STAGING_KEY not in arguments, call
        assert "--resolve staging.filedrawer.dev:443:127.0.0.1" in arguments
    configs = " ".join(call.split(" <<< ")[1] for call in calls)
    assert f"key={KEY}" in configs
    assert f"X-Staging-Key: {STAGING_KEY}" in configs


def test_open_refuses_while_a_key_is_live_and_replace_takes_its_place(box):
    remote(box, f"open {KEY} 1")

    refused = remote(box, f"open {OTHER} 1")
    assert refused.returncode == 3
    assert "replace" in refused.stderr
    assert f"key={KEY}" in box.snippet.read_text()

    rules = json.dumps(
        [
            [f"X-Staging-Key: {STAGING_KEY}", 200],
            [f"key={OTHER}", 200],
            [f"key={KEY}", 403],
        ]
    )
    replaced = remote(box, f"replace {OTHER} 1", FAKE_CURL_RULES=rules)
    assert replaced.returncode == 0, replaced.stderr
    assert f"key={OTHER}" in box.snippet.read_text()


def test_open_over_an_expired_key_needs_no_replace(box):
    box.snippet.write_text(
        f"# field-test key={OTHER} expires=1000\n"
        f"@field_test {{\n\tquery key={OTHER}\n}}\nhandle @field_test {{\n"
        f"\t@field_test_expired expression int({{time.now.unix}}) >= 1000\n}}\n"
    )

    rules = json.dumps(
        [
            [f"X-Staging-Key: {STAGING_KEY}", 200],
            [f"key={KEY}", 200],
            [f"key={OTHER}", 403],
        ]
    )

    assert remote(box, f"open {KEY} 1", FAKE_CURL_RULES=rules).returncode == 0


def test_close_empties_the_snippet_and_checks_the_old_key_is_refused(box):
    remote(box, f"open {KEY} 1")
    rules = json.dumps([[f"X-Staging-Key: {STAGING_KEY}", 200], [f"key={KEY}", 403]])

    result = remote(box, "close", FAKE_CURL_RULES=rules)

    assert result.returncode == 0, result.stderr
    assert box.snippet.read_text() == ""
    assert result.stdout == "closed\n"
    assert f"key={KEY}" in box.log.read_text().splitlines()[-1]


@pytest.mark.parametrize(
    "line",
    [
        f"open {KEY[:-1]} 1",
        f"open {KEY.upper()} 1",
        f"open {KEY} 0",
        f"open {KEY} 73",
        f"open {KEY} 1.5",
        f"open {KEY}",
        f"open {KEY} 1 extra",
        "close now",
        f"widen {KEY} 1",
        "",
    ],
)
def test_it_refuses_a_line_it_doesnt_expect_and_touches_nothing(box, line):
    result = remote(box, line)

    assert result.returncode == 2, result.stdout
    assert not box.snippet.exists()
    assert "systemctl" not in box.log.read_text()


@pytest.mark.parametrize(
    "failure",
    [
        {"FAKE_CADDY_FAILS": "1"},
        {"FAKE_RELOAD_FAILS": "1"},
        {"FAKE_CURL_RULES": json.dumps([[f"key={KEY}", 500]])},
    ],
    ids=["validate", "reload", "probe"],
)
def test_a_failed_validate_reload_or_probe_puts_the_previous_snippet_back(box, failure):
    box.snippet.write_text("# the previous snippet\n")

    result = remote(box, f"open {KEY} 1", **failure)

    assert result.returncode == 1
    assert box.snippet.read_text() == "# the previous snippet\n"
    assert result.stdout == ""


def test_close_still_closes_when_staging_is_down_and_says_so(box):
    remote(box, f"open {KEY} 1")
    rules = json.dumps([[f"X-Staging-Key: {STAGING_KEY}", "000"], [f"key={KEY}", 403]])

    result = remote(box, "close", FAKE_CURL_RULES=rules)

    assert result.returncode == 0, result.stderr
    assert box.snippet.read_text() == ""
    assert "staging key" in result.stderr


def test_a_close_whose_old_key_still_gets_through_fails_without_reopening_it(box):
    remote(box, f"open {KEY} 1")

    result = remote(box, "close")

    assert result.returncode == 1
    assert box.snippet.read_text() == ""
    assert "still" in result.stderr


def test_open_probes_with_the_key_in_stagings_site_file(box):
    """Once caddy.sh writes a site file per environment, the Caddyfile only imports
    them, and staging's key is in staging's site file."""
    (box.root / "etc/caddy/sites").mkdir()
    (box.root / "etc/caddy/sites/staging.caddy").write_text(
        caddyfile_with_staging_key()
    )
    (box.root / "etc/caddy/Caddyfile").write_text("import /etc/caddy/sites/*.caddy\n")
    result = remote(box, f"open {KEY} 1")
    assert result.returncode == 0, result.stderr
    assert f"X-Staging-Key: {STAGING_KEY}" in box.log.read_text()


def test_open_fails_when_the_caddyfile_holds_no_staging_key_to_probe(box):
    (box.root / "etc/caddy/Caddyfile").write_text("(staging_gate) {\n}\n")

    result = remote(box, f"open {KEY} 1")

    assert result.returncode == 1
    assert not box.snippet.read_text()


def test_a_502_from_a_restarting_staging_still_counts_as_through(box):
    rules = json.dumps([[f"X-Staging-Key: {STAGING_KEY}", 502], [f"key={KEY}", 502]])

    assert remote(box, f"open {KEY} 1", FAKE_CURL_RULES=rules).returncode == 0


# caddy.sh's gate


@pytest.fixture
def caddy_sh(box):
    """Runs caddy.sh against the box, as provision-vps does, with Caddy installed."""

    def run() -> subprocess.CompletedProcess:
        return subprocess.run(
            ["bash", str(CADDY_SH)],
            env={
                **box.env,
                "ENVIRONMENT": "staging",
                "STAGING_USER": "ada",
                "STAGING_SECRETS": f"pw\t{STAGING_KEY}",
            },
            capture_output=True,
            text=True,
            check=False,
        )

    return run


def test_the_gate_takes_the_staging_key_then_the_field_test_key_then_refuses_a_stale_one(
    box, caddy_sh
):
    result = caddy_sh()

    assert result.returncode == 0, result.stderr
    gate = (box.root / "etc/caddy/sites/staging.caddy").read_text()
    order = [
        gate.index("handle /robots.txt"),
        gate.index("handle @agent"),
        gate.index("import /etc/caddy/field-test.caddy"),
        gate.index("@field_test_stale query key=*"),
        gate.index("basic_auth"),
    ]
    assert order == sorted(order), gate
    assert STALE_TEXT in gate


def test_caddy_sh_makes_an_empty_snippet_once_and_keeps_an_open_one(box, caddy_sh):
    caddy_sh()
    assert box.snippet.read_text() == ""
    assert stat.S_IMODE(box.snippet.stat().st_mode) == 0o640

    box.snippet.write_text("# an open key\n")
    caddy_sh()
    assert box.snippet.read_text() == "# an open key\n"


# ops/field-test-key, on the owner's host


@pytest.fixture
def owner(tmp_path, fake_host):
    ops = tmp_path / "ops"
    ops.mkdir()
    (ops / ".vps.env").write_text("VPS_HOST='vps.example'\n")
    key_file = tmp_path / "keep/field-test-key/key"

    def run(*args: str) -> subprocess.CompletedProcess:
        return subprocess.run(
            [str(HOST), *args],
            env={
                **fake_host.env,
                "FILEDRAWER_OPS_DIR": str(ops),
                "FILEDRAWER_FIELD_TEST_KEY_FILE": str(key_file),
            },
            capture_output=True,
            text=True,
            check=False,
        )

    # The script on stdin holds both verbs, so a close is told apart by its line
    # followed by the script's first line.
    fake_host.answer("*", stdout="expires=2026-11-10T06:00:00Z\n")
    fake_host.answer("*", stdout="closed\n", stdin=CLOSE)
    return SimpleNamespace(run=run, host=fake_host, key_file=key_file)


CLOSE = "close\n#!/usr/bin/env bash\n"


def sent_line(host, n: int = 1) -> str:
    return host.sent(n).split("\n", 1)[0]


def test_open_sends_a_new_key_on_stdin_ahead_of_the_script_and_keeps_it_in_the_file(
    owner,
):
    result = owner.run("open")

    assert result.returncode == 0, result.stderr
    verb, key, hours = sent_line(owner.host).split(" ")
    assert (verb, hours) == ("open", "24")
    assert re.fullmatch(r"[0-9a-f]{64}", key)
    assert owner.host.sent(1).split("\n", 1)[1] == REMOTE.read_text()
    assert owner.key_file.read_text() == f"key={key}\nexpires=2026-11-10T06:00:00Z\n"
    assert stat.S_IMODE(owner.key_file.stat().st_mode) == 0o600
    assert stat.S_IMODE(owner.key_file.parent.stat().st_mode) == 0o700
    assert key not in owner.host.log.read_text()
    assert key not in result.stdout + result.stderr
    assert "2026-11-10T06:00:00Z" in result.stdout
    assert owner.host.calls() == ["IFS= read -r FIELD_TEST; export FIELD_TEST; bash -s"]


def test_each_open_makes_a_new_key(owner):
    owner.run("open")
    owner.run("open", "--replace")

    first, second = sent_line(owner.host, 1), sent_line(owner.host, 2)
    assert first.split(" ")[1] != second.split(" ")[1]
    assert second.startswith("replace ")


@pytest.mark.parametrize("hours", ["0", "73", "eight", "1.5"])
def test_open_refuses_hours_outside_1_to_72_and_sends_nothing(owner, hours):
    result = owner.run("open", "--hours", hours)

    assert result.returncode == 2
    assert owner.host.log.read_text() == ""
    assert not owner.key_file.exists()


def test_open_passes_the_hours_on(owner):
    owner.run("open", "--hours", "72")

    assert sent_line(owner.host).endswith(" 72")


def test_a_refused_open_says_why_and_marks_the_file_failed(owner):
    owner.host.answer("*", status=3)

    result = owner.run("open")

    assert result.returncode == 1
    assert re.fullmatch(r"failed=\S+Z\n", owner.key_file.read_text())
    assert "--replace" in result.stderr


@pytest.mark.parametrize("status", [1, 3])
def test_an_open_the_server_turned_down_leaves_the_key_file_as_it_was(owner, status):
    owner.run("open")
    before = owner.key_file.read_text()
    owner.host.answer("*", status=status)

    result = owner.run("open", "--replace")

    assert result.returncode == 1
    assert owner.key_file.read_text() == before


def test_an_open_whose_answer_is_lost_keeps_the_new_key_and_says_to_close(owner):
    owner.host.answer("*", status=255)

    result = owner.run("open")

    assert result.returncode == 1
    key = sent_line(owner.host).split(" ")[1]
    assert owner.key_file.read_text() == f"key={key}\nexpires=unknown\n"
    assert "ops/field-test-key close" in result.stderr


@pytest.mark.parametrize("flag", ["--hours", "--out"])
def test_a_flag_with_no_value_says_so(owner, flag):
    result = owner.run("open", flag)

    assert result.returncode == 2
    assert flag in result.stderr


def test_close_marks_the_file_closed_in_place(owner):
    owner.run("open")
    inode = owner.key_file.stat().st_ino

    result = owner.run("close")

    assert result.returncode == 0, result.stderr
    assert sent_line(owner.host, 2) == "close"
    assert re.fullmatch(r"closed=\S+Z\n", owner.key_file.read_text())
    assert owner.key_file.stat().st_ino == inode


def test_a_failed_close_keeps_the_key_in_the_file(owner):
    owner.run("open")
    before = owner.key_file.read_text()
    owner.host.answer("*", status=1, stdin=CLOSE)

    result = owner.run("close")

    assert result.returncode == 1
    assert owner.key_file.read_text() == before
    assert "still" in result.stderr


def test_help_names_both_commands(owner):
    result = owner.run("-h")

    assert result.returncode == 0
    assert "open" in result.stdout and "close" in result.stdout


# A real Caddy: bin/caddy's, through conftest's real_caddy


def free_port() -> int:
    with socket.socket() as probe:
        probe.bind(("127.0.0.1", 0))
        return probe.getsockname()[1]


def test_a_real_caddy_serves_the_gate_and_snippet_as_meant(
    box, caddy_sh, tmp_path, real_caddy
):
    caddy_sh()
    remote(box, f"open {KEY} 1")
    gate = (box.root / "etc/caddy/sites/staging.caddy").read_text()
    snippet = box.snippet.read_text()
    expired = tmp_path / "expired.caddy"
    expired.write_text(
        re.sub(r">= \d+", f">= {int(time.time()) - 10}", snippet)
        .replace(KEY, OTHER)
        .replace("@field_test", "@field_old")
    )
    front, back = free_port(), free_port()
    gate_body = gate[
        gate.index("(staging_gate) {") : gate.index("\nstaging.filedrawer.dev {")
    ]
    hashed = subprocess.run(
        [real_caddy, "hash-password", "--plaintext", "pw"],
        capture_output=True,
        text=True,
        check=True,
    ).stdout.strip()
    gate_body = re.sub(r"(\n\t\t\tada) \S*", rf"\1 {hashed}", gate_body)
    config = tmp_path / "Caddyfile"
    config.write_text(
        "{\n\tadmin off\n\tauto_https off\n}\n"
        + gate_body.replace(
            "/etc/caddy/field-test.caddy", f"{box.snippet}\n\timport {expired}"
        ).replace("127.0.0.1:8081", f"127.0.0.1:{back}")
        + f'\n:{front} {{\n\timport staging_gate\n}}\n:{back} {{\n\trespond "got {{uri}}"\n}}\n'
    )
    for path in (box.snippet, expired):
        path.write_text(path.read_text().replace("127.0.0.1:8081", f"127.0.0.1:{back}"))
    server = subprocess.Popen(
        [real_caddy, "run", "--config", str(config), "--adapter", "caddyfile"],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    try:

        def fetch(path: str, *extra: str) -> tuple[int, str]:
            done = subprocess.run(
                [
                    "curl",
                    "-s",
                    "-w",
                    "\n%{http_code}",
                    *extra,
                    f"http://127.0.0.1:{front}{path}",
                ],
                capture_output=True,
                text=True,
                check=False,
            )
            body, _, status = done.stdout.rpartition("\n")
            return int(status or 0), body

        while fetch("/robots.txt")[0] != 200:
            assert server.poll() is None, "caddy stopped"
            time.sleep(0.1)

        assert fetch(f"/tags?key={KEY}&q=x") == (200, "got /tags?q=x")
        assert fetch(f"/tags?key={KEY}", "-I")[0] == 200
        assert fetch(f"/tags?key={OTHER}") == (403, STALE_TEXT)
        assert fetch("/tags?key=wrong") == (403, STALE_TEXT)
        assert fetch(f"/entries?key={KEY}", "-X", "POST")[0] == 403
        assert fetch(f"/staging/status?key={KEY}")[0] == 403
        assert fetch(f"/admin?key={KEY}")[0] == 403
        assert fetch(f"/staging?key={KEY}")[0] == 403
        assert fetch("/tags")[0] == 401
        assert fetch("/tags", "-H", f"X-Staging-Key: {STAGING_KEY}") == (
            200,
            "got /tags",
        )
    finally:
        server.terminate()
        server.wait()
