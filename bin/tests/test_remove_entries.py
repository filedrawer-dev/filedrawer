"""Tests for ops/remove-entries' memory of the admin email (#112): offered on Enter,
saved in ops/.vps.env per site once typed, never the password."""

import http.server
import importlib.machinery
import importlib.util
import stat
import threading
from pathlib import Path

import pytest

REPO = Path(__file__).resolve().parents[2]
STAGING = "https://staging.filedrawer.dev"
PRODUCTION = "https://filedrawer.dev"


@pytest.fixture
def remove_entries(tmp_path, monkeypatch):
    env = tmp_path / ".vps.env"
    monkeypatch.setenv("FILEDRAWER_VPS_ENV", str(env))
    loader = importlib.machinery.SourceFileLoader(
        "remove_entries", str(REPO / "ops/remove-entries")
    )
    spec = importlib.util.spec_from_loader("remove_entries", loader)
    module = importlib.util.module_from_spec(spec)
    loader.exec_module(module)
    module.env = env
    return module


def answers(*lines):
    it = iter(lines)
    asked = []

    def read(prompt):
        asked.append(prompt)
        return next(it)

    read.asked = asked
    return read


def test_a_typed_email_is_saved_per_site_and_offered_next_time(remove_entries):
    remove_entries.env.write_text("VPS_HOST='h'\n")
    email, typed = remove_entries.admin_email(STAGING, answers("o'ne@x.dev"))
    assert (email, typed) == ("o'ne@x.dev", True)
    remove_entries.remember_email(STAGING, email)
    assert remove_entries.env.read_text() == (
        "VPS_HOST='h'\nSTAGING_ADMIN_EMAIL='o'\\''ne@x.dev'\n"
    )
    read = answers("")
    assert remove_entries.admin_email(STAGING, read) == ("o'ne@x.dev", False)
    assert "o'ne@x.dev" in read.asked[0]
    # Production's account is a sign-up of its own.
    assert remove_entries.admin_email(PRODUCTION, answers("p@gmail.com"))[0] == (
        "p@gmail.com"
    )


def test_a_new_email_replaces_the_saved_one(remove_entries):
    remove_entries.remember_email(PRODUCTION, "a@x.dev")
    remove_entries.remember_email(PRODUCTION, "b@x.dev")
    assert remove_entries.env.read_text() == "PROD_ADMIN_EMAIL='b@x.dev'\n"
    assert stat.S_IMODE(remove_entries.env.stat().st_mode) == 0o600


def test_another_site_is_neither_offered_nor_saved(remove_entries):
    remove_entries.remember_email(STAGING, "s@x.dev")
    read = answers("d@x.dev")
    assert remove_entries.admin_email("http://localhost:8081", read) == (
        "d@x.dev",
        True,
    )
    assert "s@x.dev" not in read.asked[0]
    remove_entries.remember_email("http://localhost:8081", "d@x.dev")
    assert "d@x.dev" not in remove_entries.env.read_text()


@pytest.mark.parametrize(("status", "saved"), [(303, True), (401, False)])
def test_the_email_is_saved_only_once_a_login_with_it_works(
    remove_entries, tmp_path, monkeypatch, status, saved
):
    ids = tmp_path / "ids.txt"
    ids.write_text("")
    monkeypatch.setattr("builtins.input", lambda prompt: "typed@x.dev")
    monkeypatch.setattr(remove_entries.getpass, "getpass", lambda prompt: "pw")

    def post(site, path, form):
        site.cookie = "session" if status == 303 else ""
        return status, ""

    monkeypatch.setattr(remove_entries.Site, "post", post)
    monkeypatch.setattr("sys.argv", ["remove-entries", str(ids), "--base", STAGING])
    remove_entries.main()
    text = remove_entries.env.read_text() if remove_entries.env.exists() else ""
    assert ("STAGING_ADMIN_EMAIL='typed@x.dev'" in text) == saved
    assert "pw" not in text


class SessionSite(http.server.BaseHTTPRequestHandler):
    """Logs in on /login with the server's session cookie, then echoes the Cookie
    header each later POST sends."""

    def do_POST(self):
        self.rfile.read(int(self.headers.get("Content-Length", 0)))
        self.send_response(200)
        if self.path == "/login":
            self.send_header(
                "Set-Cookie", "__Host-fd_session=s3cret; Path=/; HttpOnly; Secure"
            )
        self.end_headers()
        self.wfile.write((self.headers.get("Cookie") or "").encode())

    def log_message(self, *args):
        pass


def test_a_post_after_the_login_sends_the_session_cookie_back(
    remove_entries, monkeypatch
):
    monkeypatch.delenv("FILEDRAWER_STAGING_KEY", raising=False)
    httpd = http.server.HTTPServer(("127.0.0.1", 0), SessionSite)
    threading.Thread(target=httpd.serve_forever, daemon=True).start()
    try:
        site = remove_entries.Site(f"http://127.0.0.1:{httpd.server_port}")
        site.post("/login", {"email": "a@x.dev", "password": "pw"})
        _, sent = site.post("/admin/e/x/remove", {})
    finally:
        httpd.shutdown()
        httpd.server_close()
    assert sent == "__Host-fd_session=s3cret"
