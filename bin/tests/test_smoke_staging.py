"""ops/smoke-staging screen against a fake server, for the replies a real one can't be
made to give in order: a check that fails, then a day's cap spent; the screen
rejecting the series sent to learn why a case wasn't decided. The real server's
replies are covered by crates/server/tests/it/smoke.rs."""

import hashlib
import json
import os
import subprocess
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

import pytest

REPO = Path(__file__).resolve().parents[2]
SCRIPT = REPO / "ops/smoke-staging"
TOKEN = "fdt_screen9"
JOIN_KEY = "sk-join-77"
# The 429 the server's POST /series gives when the day's screening cap is spent.
SPENT = (
    429,
    "refused:\n- the day's screening budget is spent; send it again tomorrow (UTC)\n",
)


def fake_server(
    statuses: list[str],
    series: tuple[int, str],
    attachments: tuple[int, str] = (404, "not found\n"),
) -> ThreadingHTTPServer:
    """A server whose Nth entry, sent alone or in a batch, reports status statuses[N],
    and whose POST /series and POST /attachments answer series and attachments (code,
    body). Its `posts` lists the path of every POST it took."""
    submitted: list[str] = []
    posts: list[str] = []

    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *_args) -> None:
            pass

        def answer(self, code: int, body: str) -> None:
            data = body.encode()
            self.send_response(code)
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)

        def do_POST(self) -> None:
            raw = self.rfile.read(int(self.headers.get("Content-Length", 0)))
            posts.append(self.path)
            if self.path == "/entries":
                submitted.append(f"e{len(submitted)}")
                self.answer(201, f"id: {submitted[-1]}\nstatus: pending\n")
            elif self.path == "/entries/batch":
                lines = ["id: b1", "status: pending"]
                for _ in json.loads(raw)["entries"]:
                    submitted.append(f"e{len(submitted)}")
                    lines.append(
                        f"entry: {submitted[-1]} http://fake/e/{submitted[-1]}"
                    )
                self.answer(201, "\n".join(lines) + "\n")
            elif self.path.endswith("/publish"):
                self.answer(200, "status: screening\n")
            elif self.path == "/series":
                self.answer(*series)
            elif self.path == "/attachments":
                self.answer(*attachments)
            else:
                self.answer(404, "not found\n")

        def do_GET(self) -> None:
            entry = self.path.removeprefix("/e/")
            status = statuses[submitted.index(entry)]
            reasons = (
                "reason: it isn't AI safety research\n" if status == "rejected" else ""
            )
            self.answer(200, f"id: {entry}\nstatus: {status}\n{reasons}")

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    server.posts = posts
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return server


@pytest.fixture
def screen(tmp_path):
    servers = []

    def run(
        statuses: list[str], series: tuple[int, str]
    ) -> subprocess.CompletedProcess:
        server = fake_server(statuses, series)
        servers.append(server)
        env = {
            "PATH": os.environ["PATH"],
            "FILEDRAWER_STAGING_KEY": "k",
            "FILEDRAWER_STAGING_SCREEN_API_KEY": TOKEN,
            "FILEDRAWER_OPS_DIR": str(tmp_path),
            "FILEDRAWER_SCREEN_WAIT_SECONDS": "0",
        }
        url = f"http://127.0.0.1:{server.server_address[1]}"
        return subprocess.run(
            ["bash", SCRIPT, "screen", url],
            env=env,
            capture_output=True,
            text=True,
            check=False,
        )

    yield run
    for server in servers:
        server.shutdown()


def test_a_failed_check_fails_even_when_the_day_is_spent_after_it(screen):
    result = screen(["rejected", "screening"], SPENT)
    assert result.returncode == 1, result.stdout
    assert "FAIL on-topic entry: public" in result.stdout
    assert "the day's screening cap is spent" in result.stdout


def test_only_the_cap_s_429_counts_as_a_spent_day(screen):
    assert screen(["screening"], SPENT).returncode == 3
    allowance = "refused:\n- the account's series have taken today's allowance; send it again tomorrow (UTC)\n"
    result = screen(["screening"], (429, allowance))
    assert result.returncode == 1, result.stdout
    assert (
        "POST /series answered 429: the account's series have taken today's allowance"
        in result.stdout
    )


def test_the_screen_rejecting_the_series_says_so(screen):
    body = "error: the screen failed it\n---\nreason: it reads as spam\n"
    result = screen(["screening"], (422, body))
    assert result.returncode == 1, result.stdout
    assert "POST /series answered 422: the screen failed it" in result.stdout
    # The model's words below the delimiter stay out of the log.
    assert "reads as spam" not in result.stdout


def test_a_series_made_says_the_case_was_slow_and_prints_no_key(screen):
    body = f"id: s-1\nstatus: hidden\nkey: {JOIN_KEY}\n"
    result = screen(["screening"], (201, body))
    assert result.returncode == 1, result.stdout
    assert "the case was only slow" in result.stdout
    assert JOIN_KEY not in result.stdout + result.stderr
    assert TOKEN not in result.stdout + result.stderr


# The labelled eval cases' expected statuses, in the order eval sends them.
LABELLED = [
    expected
    for expected in ("public", "rejected", "review")
    for _ in sorted((REPO / "ops/screen-eval" / expected).glob("*.md"))
]
# The 429 the server's POST /attachments gives once the account's day of uploads is spent.
UPLOADS_SPENT = (
    429,
    "refused:\n- this account has uploaded its 10000000 bytes of attachments for today (UTC); send it tomorrow, or publish the code on GitHub or Zenodo\n",
)


@pytest.fixture
def eval_run(tmp_path):
    """Runs eval with one made-up field-test entry that attaches a gathered file,
    against a fake server, and a fake filedrawer binary whose unpack finds nothing."""
    field_test = tmp_path / "field-test"
    (field_test / "round3/demo").mkdir(parents=True)
    (field_test / "files").mkdir()
    (field_test / "round3/demo/one.md").write_text(
        "---\ntitle: made-up\n---\nmade up\n"
    )
    text = "print('made-up')\n"
    digest = hashlib.sha256(text.encode()).hexdigest()
    (field_test / "files" / digest).write_text(text)
    (field_test / "round3/demo/one.attachments").write_text(
        f"author:run.py {len(text)} {digest}\n"
    )
    fake_bin = tmp_path / "filedrawer"
    fake_bin.write_text('#!/bin/sh\nfor f; do echo "$f: nothing hidden"; done\n')
    fake_bin.chmod(0o755)
    servers = []

    def run(statuses, series, attachments, args=("--field-test", "1"), server=None):
        """One run of eval; pass a server again to resume against it."""
        if server is None:
            server = fake_server(statuses, series, attachments)
            servers.append(server)
        env = {
            "PATH": os.environ["PATH"],
            "FILEDRAWER_STAGING_KEY": "k",
            "FILEDRAWER_STAGING_SCREEN_API_KEY": TOKEN,
            "FILEDRAWER_OPS_DIR": str(tmp_path),
            "FILEDRAWER_SCREEN_WAIT_SECONDS": "0",
            "FILEDRAWER_FIELD_TEST_DIR": str(field_test),
            "FILEDRAWER_BIN": str(fake_bin),
        }
        url = f"http://127.0.0.1:{server.server_address[1]}"
        return subprocess.run(
            ["bash", SCRIPT, "eval", *args, url],
            env=env,
            capture_output=True,
            text=True,
            check=False,
        )

    run.servers = servers
    yield run
    for server in servers:
        server.shutdown()


def test_a_refused_attachment_set_is_a_failure_reported_again_on_resume(eval_run):
    refused = (422, "refused:\n- file made-up.py isn't UTF-8 text\n")
    result = eval_run(LABELLED, SPENT, refused)
    failure = "FAIL a field-test entry's attachment set was refused (422)"
    assert failure in result.stdout, result.stdout
    assert "1 refused at submission" in result.stdout
    run_file = next(
        line.removeprefix("run file: ")
        for line in result.stdout.splitlines()
        if line.startswith("run file: ")
    )
    again = eval_run(
        LABELLED, SPENT, refused, ("--resume", run_file), eval_run.servers[-1]
    )
    assert failure in again.stdout, again.stdout
    assert "made-up.py" not in result.stdout + again.stdout


def test_uploads_spent_for_the_day_stop_the_run_for_a_resume(eval_run):
    result = eval_run(LABELLED, SPENT, UPLOADS_SPENT)
    assert result.returncode == 1, result.stdout
    assert (
        "POST /attachments answered 429: this account has uploaded its 10000000 bytes"
        in result.stdout
    )
    assert "Stopped: an attachment set couldn't be uploaded now." in result.stdout
    run_file = next(
        line.removeprefix("run file: ")
        for line in result.stdout.splitlines()
        if line.startswith("run file: ")
    )
    assert f"Resume with: ops/smoke-staging eval --resume {run_file}" in result.stdout
    # The entry wasn't sent, so a resume sends it with its set.
    assert "one.md\t" not in Path(run_file).read_text()
    # The run isn't over, so the review cases aren't the owner's to reject yet: a
    # resume would read them as rejected.
    assert "Reject the review cases" not in result.stdout


def test_an_upload_the_server_fails_stops_the_run_rather_than_skip_the_entry(
    eval_run,
):
    result = eval_run(LABELLED, SPENT, (503, "the server is restarting\n"))
    assert result.returncode == 1, result.stdout
    assert "POST /attachments answered 503" in result.stdout
    assert "passed over" not in result.stdout
    run_file = next(
        line.removeprefix("run file: ")
        for line in result.stdout.splitlines()
        if line.startswith("run file: ")
    )
    assert f"Resume with: ops/smoke-staging eval --resume {run_file}" in result.stdout
    assert "\nrefused\t" not in Path(run_file).read_text()


def test_an_undecided_entry_with_attachments_is_most_likely_slow(eval_run):
    series_made = (201, f"id: s-1\nstatus: hidden\nkey: {JOIN_KEY}\n")
    uploaded = (201, "id: as-1\nbytes: 17\n")
    result = eval_run([*LABELLED, "screening"], series_made, uploaded)
    assert "WAIT public field-test-1" in result.stdout
    # Attached code is screened in parts, which takes minutes; the cap's half for
    # attachments is the rarer cause, and only /staging/status can tell.
    assert "attached code is screened in parts" in result.stdout
    assert "FILEDRAWER_SCREEN_WAIT_SECONDS" in result.stdout
    assert "/staging/status" in result.stdout
    assert JOIN_KEY not in result.stdout + result.stderr


@pytest.fixture
def families_run(tmp_path):
    """Runs eval --families, or --resume, on one made-up family of two entries that
    cite a made-up series, against a fake server that a test may pass again to resume
    against it. Returns the result and the server."""
    family = tmp_path / "field-test/round3/demo"
    family.mkdir(parents=True)
    for name in ("a", "b"):
        (family / f"{name}.md").write_text(
            f"---\ntitle: made-up {name}\nslug: {name}\nseries: ser_madeUp1@1\n---\nmade up\n"
        )
    (family / "series-demo.md").write_text(
        "---\nid: ser_madeUp1\ntitle: made-up\n---\nmade-up setup\n"
    )
    servers = []

    def run(args, statuses, series, server=None):
        if server is None:
            server = fake_server(statuses, series)
            servers.append(server)
        env = {
            "PATH": os.environ["PATH"],
            "FILEDRAWER_STAGING_KEY": "k",
            "FILEDRAWER_STAGING_SCREEN_API_KEY": TOKEN,
            "FILEDRAWER_OPS_DIR": str(tmp_path),
            "FILEDRAWER_SCREEN_WAIT_SECONDS": "0",
            "FILEDRAWER_FIELD_TEST_DIR": str(tmp_path / "field-test"),
        }
        url = f"http://127.0.0.1:{server.server_address[1]}"
        result = subprocess.run(
            ["bash", SCRIPT, "eval", *args, url],
            env=env,
            capture_output=True,
            text=True,
            check=False,
        )
        return result, server

    yield run
    for server in servers:
        server.shutdown()


SERIES_MADE = (201, f"id: s-1\nversion: 1\nkey: {JOIN_KEY}\n")


def test_a_member_in_review_holds_its_batch_without_stopping_the_run(families_run):
    result, _ = families_run(
        ["--families"], [*LABELLED, "review", "screening"], SERIES_MADE
    )
    assert "FAIL public field-test-1" in result.stdout, result.stdout
    assert "got review" in result.stdout
    assert (
        "1 field-test entries wait in a batch for the owner's decision on a member in review"
        in result.stdout
    )
    assert "WAIT" not in result.stdout and "Stopped" not in result.stdout
    assert JOIN_KEY not in result.stdout + result.stderr


def test_a_series_the_screen_held_back_isnt_sent_again_on_resume(families_run):
    held = (
        422,
        "refused:\n- the screen held this series back for the archive's owner; don't send it again reworded\n",
    )
    result, server = families_run(["--families"], [*LABELLED, "public", "public"], held)
    assert "FAIL series series-1: 422" in result.stdout, result.stdout
    assert "PASS public field-test-2" in result.stdout
    run_file = next(
        line.removeprefix("run file: ")
        for line in result.stdout.splitlines()
        if line.startswith("run file: ")
    )
    again, _ = families_run(
        ["--resume", run_file], [*LABELLED, "public", "public"], held, server
    )
    assert "PASS public field-test-2" in again.stdout, again.stdout
    assert "FAIL series" not in again.stdout
    assert server.posts.count("/series") == 1, server.posts


@pytest.mark.parametrize("poll", ["soon", "0", "0.0"])
def test_a_poll_interval_that_isnt_a_positive_number_is_refused_first(poll):
    # 0 would ask staging as fast as it answers, into its per-IP rate limit.
    env = {
        "PATH": os.environ["PATH"],
        "FILEDRAWER_STAGING_KEY": "k",
        "FILEDRAWER_STAGING_SCREEN_API_KEY": TOKEN,
        "FILEDRAWER_SCREEN_POLL_SECONDS": poll,
    }
    result = subprocess.run(
        ["bash", SCRIPT, "screen", "http://127.0.0.1:9"],
        env=env,
        capture_output=True,
        text=True,
        check=False,
    )
    assert result.returncode == 2, result.stdout + result.stderr
    assert "FILEDRAWER_SCREEN_POLL_SECONDS" in result.stderr


def test_a_sample_takes_entries_with_a_bom_or_windows_line_ends_and_nothing_else(
    tmp_path,
):
    family = tmp_path / "field-test/round3/demo"
    family.mkdir(parents=True)
    (family / "bom.md").write_bytes(
        "\ufeff---\ntitle: made-up\n---\nmade up\n".encode()
    )
    (family / "crlf.md").write_bytes(b"---\r\ntitle: made-up\r\n---\r\nmade up\r\n")
    (family / "FINDINGS.md").write_text("made-up notes\n")
    (family / "series-demo.md").write_text("---\ntitle: made-up\n---\nmade up\n")
    server = fake_server([*LABELLED, "public", "public"], SPENT)
    try:
        env = {
            "PATH": os.environ["PATH"],
            "FILEDRAWER_STAGING_KEY": "k",
            "FILEDRAWER_STAGING_SCREEN_API_KEY": TOKEN,
            "FILEDRAWER_OPS_DIR": str(tmp_path),
            "FILEDRAWER_SCREEN_WAIT_SECONDS": "0",
            "FILEDRAWER_FIELD_TEST_DIR": str(tmp_path / "field-test"),
        }
        url = f"http://127.0.0.1:{server.server_address[1]}"
        result = subprocess.run(
            ["bash", SCRIPT, "eval", "--field-test", "5", url],
            env=env,
            capture_output=True,
            text=True,
            check=False,
        )
    finally:
        server.shutdown()
    assert "PASS public field-test-2" in result.stdout, result.stdout
    assert "field-test-3" not in result.stdout
    assert server.posts.count("/entries") == len(LABELLED) + 2, server.posts


def test_an_entry_whose_status_comes_back_empty_doesnt_break_the_decoding_tally(
    eval_run, tmp_path
):
    # A GET that fails under load leaves no status; the tally still counts the entry,
    # whose set (says this fake filedrawer) hides text.
    (tmp_path / "filedrawer").write_text(
        '#!/bin/sh\nfor f; do echo "$f: 1 part, 10 bytes hidden"; done\n'
    )
    uploaded = (201, "id: as-1\nbytes: 17\n")
    result = eval_run([*LABELLED, ""], SPENT, uploaded)
    assert "bad array subscript" not in result.stderr, result.stderr
    assert (
        "citing a set that hides text or passes a limit: 1 (public 0, rejected 0, review 0, undecided 1)"
        in result.stdout
    ), result.stdout


def test_a_family_whose_entries_cant_be_read_is_a_failure_not_a_silent_gap(
    tmp_path,
):
    family = tmp_path / "field-test/round3/demo"
    family.mkdir(parents=True)
    (family / "good.md").write_text("---\ntitle: made-up\n---\nmade up\n")
    (family / "bad.md").write_bytes(b"---\ntitle: made-up \xff\n---\nmade up\n")
    server = fake_server([*LABELLED, "public"], SPENT)
    try:
        env = {
            "PATH": os.environ["PATH"],
            "FILEDRAWER_STAGING_KEY": "k",
            "FILEDRAWER_STAGING_SCREEN_API_KEY": TOKEN,
            "FILEDRAWER_OPS_DIR": str(tmp_path),
            "FILEDRAWER_SCREEN_WAIT_SECONDS": "0",
            "FILEDRAWER_FIELD_TEST_DIR": str(tmp_path / "field-test"),
        }
        url = f"http://127.0.0.1:{server.server_address[1]}"
        result = subprocess.run(
            ["bash", SCRIPT, "eval", "--field-test", "5", url],
            env=env,
            capture_output=True,
            text=True,
            check=False,
        )
    finally:
        server.shutdown()
    assert result.returncode == 1, result.stdout
    assert (
        "FAIL a family's entries: bin/field-test-batches couldn't read" in result.stdout
    )
