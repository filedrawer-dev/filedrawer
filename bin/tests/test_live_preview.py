"""Tests for bin/live-preview, run as a process: against a fake dev server (--upstream),
it renders the home page from a copy of the templates on each request, serves the
stylesheet and script from disk, and forwards everything else; with a fake
`filedrawer` binary in a scratch checkout, it seeds a new database once, starts the
server in an environment of its own, and stops it. The build isn't tested: it is
cargo's.
"""

import http.server
import json
import os
import re
import shutil
import socket
import subprocess
import threading
import time
import urllib.error
import urllib.request
from pathlib import Path

import pytest

REPO = Path(__file__).resolve().parents[2]
SCRIPT = REPO / "bin" / "live-preview"


@pytest.fixture(autouse=True)
def outside_heavy(monkeypatch):
    """These tests may run inside a heavy job, as the commit checks do, which the
    preview refuses; each runs it as from a shell outside one."""
    monkeypatch.delenv("HEAVY_SLOT", raising=False)


# What the dev server's own home page holds below the README card: the drawer, which
# the preview keeps, between markup it replaces.
DRAWER = (
    '<nav class="guides" aria-label="Filter by outcome"><a href="/search">ALL</a></nav>\n'
    '<div class="drawer"><article class="card">a seeded entry</article></div>\n'
)
SERVER_HOME = (
    '<!DOCTYPE html><html><body><div class="col">'
    f"<section>the binary's README</section>\n{DRAWER}"
    '<p class="foot">foot</p></div></body></html>'
)


class NoRedirects(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *args, **kwargs):
        return None


# No proxy, even where the environment names one: every request here is to loopback.
# No redirect followed, so a test sees the preview's own answer.
OPENER = urllib.request.build_opener(urllib.request.ProxyHandler({}), NoRedirects)


class FakeServer(http.server.ThreadingHTTPServer):
    """A dev server's stand-in that answers a browser's / with SERVER_HOME and anything
    else with what was asked, and notes each request."""

    def __init__(self) -> None:
        super().__init__(("127.0.0.1", 0), FakeHandler)
        self.requests: list[tuple[str, str, bytes]] = []


class FakeHandler(http.server.BaseHTTPRequestHandler):
    server: FakeServer

    def answer(self) -> None:
        length = int(self.headers.get("Content-Length") or 0)
        body = self.rfile.read(length) if length else b""
        self.server.requests.append((self.command, self.path, body))
        browser = self.headers.get("Sec-Fetch-Dest") == "document"
        extra = []
        if self.path == "/" and browser:
            status, kind, text = 200, "text/html", SERVER_HOME
        elif self.path == "/missing":
            status, kind, text = 404, "text/plain", "no such page"
        elif self.path == "/login":
            status, kind, text = 303, "text/plain", "signed in"
            extra = [("Location", "/account"), ("Set-Cookie", "session=s1; HttpOnly")]
        else:
            status, kind, text = 200, "text/plain", f"server {self.command} {self.path}"
        reply = text.encode()
        self.send_response(status)
        for key, value in extra:
            self.send_header(key, value)
        self.send_header("Content-Type", kind)
        self.send_header("Content-Length", str(len(reply)))
        self.end_headers()
        self.wfile.write(reply)

    do_GET = do_POST = answer

    def log_message(self, format, *args) -> None:
        pass


class Preview:
    def __init__(self, root: Path, url: str, server: FakeServer) -> None:
        self.root = root
        self.url = url
        self.server = server

    def template(self, name: str) -> Path:
        return self.root / "crates/server/templates" / name

    def static(self, name: str) -> Path:
        return self.root / "crates/server/static" / name

    def get(self, path: str, *, browser: bool = True, method: str = "GET", data=None):
        """The status, headers and text of the preview's answer to `path`."""
        request = urllib.request.Request(self.url + path, method=method, data=data)
        if browser:
            request.add_header("Sec-Fetch-Dest", "document")
        try:
            with OPENER.open(request) as reply:
                return reply.status, reply.headers, reply.read().decode()
        except urllib.error.HTTPError as error:
            return error.code, error.headers, error.read().decode()


def copy_pages(root: Path) -> None:
    """This repo's templates and static files, copied under `root`."""
    for part in ("templates", "static"):
        shutil.copytree(REPO / "crates/server" / part, root / "crates/server" / part)


def start(*args, env=None) -> tuple[subprocess.Popen, str, list[str]]:
    """The script started with `args`, the URL it serves on, and what it printed
    before that. It keeps the machine's caches, though pytest's folders may be on the
    sandbox's host mount."""
    env = dict(env or os.environ, LIVE_PREVIEW_KEEP_CACHES="1")
    process = subprocess.Popen(
        [SCRIPT, "--port", "0", *args], stdout=subprocess.PIPE, text=True, env=env
    )
    printed = []
    for line in process.stdout:
        printed.append(line)
        match = re.search(r"http://127\.0\.0\.1:\d+", line)
        if match:
            return process, match.group(0), printed
    process.wait()
    raise AssertionError(f"it exited {process.returncode} before serving: {printed}")


def stop(process: subprocess.Popen) -> None:
    process.terminate()
    process.wait()


@pytest.fixture
def preview(tmp_path):
    """The script previewing a copy of this repo's templates and static files, against
    a fake dev server."""
    copy_pages(tmp_path)
    fake = FakeServer()
    threading.Thread(target=fake.serve_forever, daemon=True).start()
    upstream = f"http://127.0.0.1:{fake.server_port}"
    process, url, _ = start("--root", tmp_path, "--upstream", upstream)
    try:
        yield Preview(tmp_path, url, fake)
    finally:
        stop(process)
        fake.shutdown()
        fake.server_close()


def test_the_home_page_renders_from_the_checkout_with_the_servers_drawer(preview):
    status, headers, page = preview.get("/")
    assert status == 200, page
    assert headers["Content-Type"].startswith("text/html")
    assert "Paste this to your agent:" in page, page
    assert "a seeded entry" in page, page
    assert "the binary's README" not in page, page
    # It reloads itself when a file changes.
    assert "/__live/version" in page, page


def test_an_edit_shows_on_the_next_request(preview):
    home = preview.template("home.html")
    home.write_text(
        home.read_text().replace("Paste this to your agent:", "Hand this over:")
    )
    _, _, page = preview.get("/")
    assert "Hand this over:" in page, page
    assert "Paste this to your agent:" not in page, page


def href(page: str, attribute: str, path: str) -> str:
    match = re.search(rf'{attribute}="({re.escape(path)}\?[^"]+)"', page)
    assert match, f"no {attribute} for {path} in {page}"
    return match.group(1)


def test_the_stylesheet_and_script_come_from_disk_at_urls_that_change_with_them(
    preview,
):
    _, _, page = preview.get("/")
    css = href(page, "href", "/static/site.css")
    script = href(page, "src", "/static/copy.js")
    status, headers, text = preview.get(css, browser=False)
    assert status == 200
    assert headers["Content-Type"].startswith("text/css")
    assert text == preview.static("site.css").read_text()
    status, headers, text = preview.get(script, browser=False)
    assert headers["Content-Type"].startswith("text/javascript")
    assert text == preview.static("copy.js").read_text()

    for name in ("site.css", "copy.js"):
        with preview.static(name).open("a") as file:
            file.write("\n/* an edit */\n")
    _, _, page = preview.get("/")
    assert href(page, "href", "/static/site.css") != css
    assert href(page, "src", "/static/copy.js") != script
    _, _, text = preview.get(href(page, "href", "/static/site.css"), browser=False)
    assert text.endswith("/* an edit */\n")
    _, _, text = preview.get(href(page, "src", "/static/copy.js"), browser=False)
    assert text.endswith("/* an edit */\n")


def test_everything_else_goes_to_the_dev_server(preview):
    assert preview.get("/search?q=steering")[2] == "server GET /search?q=steering"
    # An agent's /, which the server answers in plain text.
    assert preview.get("/", browser=False)[2] == "server GET /"
    status, _, text = preview.get("/entries", method="POST", data=b"an entry")
    assert (status, text) == (200, "server POST /entries")
    assert ("POST", "/entries", b"an entry") in preview.server.requests
    assert preview.get("/missing")[:1] == (404,)


def test_a_redirect_goes_back_to_the_browser_with_its_cookie(preview):
    status, headers, _ = preview.get("/login", method="POST", data=b"email=a")
    assert status == 303
    assert headers["Location"] == "/account"
    assert headers["Set-Cookie"] == "session=s1; HttpOnly"
    assert [path for _, path, _ in preview.server.requests] == ["/login"]


def test_the_version_changes_when_a_watched_file_does(preview):
    for file in (
        preview.template("base.html"),
        preview.template("home.html"),
        preview.static("site.css"),
        preview.static("copy.js"),
    ):
        _, _, before = preview.get("/__live/version", browser=False)
        file.write_text(file.read_text() + "\n")
        _, _, after = preview.get("/__live/version", browser=False)
        assert before and after != before, file


def free_port() -> int:
    with socket.socket() as probe:
        probe.bind(("127.0.0.1", 0))
        return probe.getsockname()[1]


def test_a_dev_server_that_doesnt_answer_gets_a_502_saying_so(tmp_path):
    copy_pages(tmp_path)
    upstream = f"http://127.0.0.1:{free_port()}"
    process, url, _ = start("--root", tmp_path, "--upstream", upstream)
    try:
        for browser in (True, False):
            status, _, text = Preview(tmp_path, url, None).get("/", browser=browser)
            assert status == 502, text
            assert f"the dev server at {upstream} didn't answer" in text, text
    finally:
        stop(process)


def fails_in_one_line(*args, env=None) -> str:
    """What the script printed when it refused `args`, checked to be one line."""
    done = subprocess.run([SCRIPT, *args], capture_output=True, text=True, env=env)
    assert done.returncode != 0, done
    assert "Traceback" not in done.stderr, done.stderr
    assert len(done.stderr.strip().splitlines()) == 1, done.stderr
    return done.stderr


def test_a_port_in_use_is_refused_in_one_line(tmp_path):
    with socket.socket() as taken:
        taken.bind(("0.0.0.0", 0))
        taken.listen()
        port = taken.getsockname()[1]
        message = fails_in_one_line("--root", tmp_path, "--port", str(port))
    assert f"port {port}" in message and "--port" in message, message


def test_inside_a_heavy_job_it_refuses_in_one_line(tmp_path):
    # A preview runs until stopped, so inside heavy it would hold the lane that every
    # session's commit checks wait for; it queues its own build instead.
    copy_pages(tmp_path)
    env = {**os.environ, "HEAVY_SLOT": "a slot"}
    message = fails_in_one_line(
        "--root", tmp_path, "--port", "0", "--no-build", env=env
    )
    assert "without heavy" in message, message


def test_no_build_without_a_binary_is_refused_in_one_line(tmp_path):
    copy_pages(tmp_path)
    message = fails_in_one_line("--root", tmp_path, "--port", "0", "--no-build")
    assert "--no-build" in message, message


# A stand-in for target/debug/filedrawer: its admin commands make the database file,
# as the real ones do, and print an ID and a token,
# and `serve` answers on FILEDRAWER_LISTEN, accepting each entry unless its text holds
# a word in ./refuse and exiting at the first publish when ./die-on-publish exists.
# It notes each command in ./calls, and serve's environment and PID beside it.
FAKE_FILEDRAWER = r"""#!/usr/bin/env python3
import http.server, json, os, sys
from pathlib import Path

here = Path(__file__).parent
with (here / "calls").open("a") as calls:
    calls.write(" ".join(sys.argv[1:]) + "\n")
if sys.argv[1] == "admin":
    Path(os.environ["FILEDRAWER_DATABASE"]).touch()
if sys.argv[1:3] == ["admin", "create-account"]:
    print("a-fake")
elif sys.argv[1:3] == ["admin", "create-token"]:
    print("fdt_fake")
elif sys.argv[1:] == ["serve"]:
    (here / "serve-env.json").write_text(json.dumps(dict(os.environ)))
    (here / "serve-pid").write_text(str(os.getpid()))
    refused = (here / "refuse").read_text().split() if (here / "refuse").exists() else []
    entries = []

    class Handler(http.server.BaseHTTPRequestHandler):
        def send(self, status, text):
            body = text.encode()
            self.send_response(status)
            self.send_header("Content-Type", "text/plain")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def do_GET(self):
            self.send(200, "ok")

        def do_POST(self):
            body = self.rfile.read(int(self.headers.get("Content-Length") or 0)).decode()
            if self.path == "/entries":
                if any(word in body for word in refused):
                    return self.send(422, "refused")
                entries.append(body)
                return self.send(200, f"id: e{len(entries)}\nstatus: pending\n")
            if (here / "die-on-publish").exists():
                os._exit(1)
            with (here / "calls").open("a") as calls:
                calls.write(f"publish {self.path}\n")
            self.send(200, "published")

        def log_message(self, *args):
            pass

    host, port = os.environ["FILEDRAWER_LISTEN"].rsplit(":", 1)
    http.server.HTTPServer((host, int(port)), Handler).serve_forever()
"""

FIXTURES = (
    "crates/analyzer/tests/it/fixtures/example.md",
    "ops/smoke-fixture.md",
    "ops/screen-eval/public/one.md",
    "ops/screen-eval/public/two.md",
)


class Checkout:
    """A scratch checkout with the pages, the seed fixtures in git's index, and a fake
    server binary."""

    def __init__(self, root: Path) -> None:
        self.root = root
        copy_pages(root)
        for name in FIXTURES:
            (root / name).parent.mkdir(parents=True, exist_ok=True)
            (root / name).write_text(f"an entry from {name}\n")
        subprocess.run(["git", "init", "-q", root], check=True)
        subprocess.run(["git", "-C", root, "add", *FIXTURES], check=True)
        self.bin = root / "target/debug"
        self.bin.mkdir(parents=True)
        (self.bin / "filedrawer").write_text(FAKE_FILEDRAWER)
        (self.bin / "filedrawer").chmod(0o755)

    def calls(self) -> list[str]:
        path = self.bin / "calls"
        return path.read_text().splitlines() if path.exists() else []

    def start(self, env=None):
        return start(
            "--root",
            self.root,
            "--no-build",
            "--server-port",
            str(free_port()),
            env=env,
        )


def test_a_new_database_is_seeded_once_with_each_fixture_published(tmp_path):
    checkout = Checkout(tmp_path)
    (checkout.bin / "refuse").write_text("smoke-fixture")
    process, _, printed = checkout.start()
    stop(process)
    assert any("live-preview: seeded 3 entries" in line for line in printed), printed
    assert any("smoke-fixture.md" in line for line in printed), printed
    calls = checkout.calls()
    assert sum(call.startswith("admin create-account") for call in calls) == 1, calls
    assert [call for call in calls if call.startswith("publish")] == [
        "publish /e/e1/publish",
        "publish /e/e2/publish",
        "publish /e/e3/publish",
    ]

    process, _, printed = checkout.start()
    stop(process)
    assert not any(re.search(r"seeded \d+ entries", line) for line in printed), printed
    assert (
        sum(call.startswith("admin create-account") for call in checkout.calls()) == 1
    )


def test_a_seed_cut_short_seeds_again_next_time(tmp_path):
    checkout = Checkout(tmp_path)
    (checkout.bin / "die-on-publish").touch()
    done = subprocess.run(
        [
            SCRIPT,
            "--root",
            tmp_path,
            "--no-build",
            "--port",
            "0",
            "--server-port",
            str(free_port()),
        ],
        capture_output=True,
        text=True,
    )
    assert done.returncode != 0, done
    assert "Traceback" not in done.stderr, done.stderr
    (checkout.bin / "die-on-publish").unlink()
    process, _, printed = checkout.start()
    stop(process)
    assert any("seeded 4 entries" in line for line in printed), printed
    assert (
        sum(call.startswith("admin create-account") for call in checkout.calls()) == 2
    )


def test_the_dev_server_gets_no_setting_of_this_shell(tmp_path):
    checkout = Checkout(tmp_path)
    env = dict(os.environ, FILEDRAWER_SMTP_HOST="smtp.example.org")
    process, _, _ = checkout.start(env=env)
    stop(process)
    served = json.loads((checkout.bin / "serve-env.json").read_text())
    # Names only: the values can be secrets.
    assert "FILEDRAWER_SMTP_HOST" not in served, sorted(served)
    assert served["FILEDRAWER_ENVIRONMENT"] == "dev"
    assert served["FILEDRAWER_LISTEN"].startswith("127.0.0.1:")


def test_stopping_the_preview_stops_the_dev_server(tmp_path):
    checkout = Checkout(tmp_path)
    process, _, _ = checkout.start()
    pid = int((checkout.bin / "serve-pid").read_text())
    stop(process)
    while True:
        try:
            os.kill(pid, 0)
        except ProcessLookupError:
            break
        time.sleep(0.05)


def test_a_template_error_shows_as_a_page_that_reloads(preview):
    home = preview.template("home.html")
    good = home.read_text()
    home.write_text(good.replace("{% block page %}", "{% block page %}{% if %}"))
    status, headers, page = preview.get("/")
    assert headers["Content-Type"].startswith("text/html")
    assert "TemplateSyntaxError" in page, page
    assert "/__live/version" in page, page
    home.write_text(good)
    assert "Paste this to your agent:" in preview.get("/")[2]


def test_help_names_the_options():
    help_text = subprocess.run(
        [SCRIPT, "-h"], capture_output=True, text=True, check=True
    ).stdout
    for option in ("--root", "--port", "--server-port", "--upstream", "--no-build"):
        assert option in help_text, help_text
    assert "sbx ports" in help_text, help_text
