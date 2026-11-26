"""Fixtures every test in bin/tests gets, and fake_host for the wizard's tests.

No test opens a browser: the wizard's open_url calls the first of wslview,
explorer.exe, xdg-open or open on PATH, and on 2026-11-06 wizard tests opened tabs on
the owner's machine. Every test runs with no-op fakes of all four first on PATH, so
a test that builds its subprocess's environment from os.environ, as they do, can't
reach a real one. Each fake notes the URL in NO_BROWSER_DIR/opened. A test may still
put its own recording fakes ahead of these.
"""

import contextlib
import os
import re
import shutil
import socket
import socketserver
import subprocess
import threading
from collections.abc import Iterable, Iterator
from pathlib import Path

import pytest
from hypothesis import HealthCheck, settings

# Autouse fixtures, registered by import: each test, and each module's fixtures, fail
# when they leave a process running. leftover_processes.py is a copy of llm-toolbox's,
# which the host has none of to import: change it there and copy it back.
from leftover_processes import (  # noqa: F401
    _leftover_processes_of_module,
    _leftover_processes_of_test,
)

# The property tests' example counts, by HYPOTHESIS_PROFILE: `dev` while writing one,
# `deep` for a longer search before trusting a change, the default otherwise. No
# deadline and no too_slow check: a slow machine may let a bug pass, but must never
# fail a test (CODING_STANDARDS.md, Python in bin/).
_UNTIMED = {"deadline": None, "suppress_health_check": [HealthCheck.too_slow]}
settings.register_profile("default", max_examples=100, **_UNTIMED)
settings.register_profile("dev", max_examples=20, **_UNTIMED)
settings.register_profile("deep", max_examples=3000, **_UNTIMED)
settings.load_profile(os.environ.get("HYPOTHESIS_PROFILE", "default"))

# No bash the tests start sources a startup file first: the sandbox's BASH_ENV names
# /etc/sandbox-persistent.sh, which cost each of the run's 1,500 or more bashes 10 ms
# of CPU (#168), and puts back on PATH what a test took off it. CI and the owner's
# host set none. Dropped here, not by monkeypatch, so the session's and each module's
# fixtures start their bashes without it too; this process holds everything the file
# exports already.
os.environ.pop("BASH_ENV", None)

OPENERS = ("wslview", "explorer.exe", "xdg-open", "open")
# Tools that reach other machines. A test that needs one uses fake_host.
NETWORK_TOOLS = ("gh", "ssh", "scp", "sftp", "curl")
# Looked up before any fixture changes PATH: the guard's curl passes calls on that
# reach only this machine.
REAL_CURL = shutil.which("curl")
REFUSE = 'echo "blocked: a test ran the real $(basename "$0"); use the fake_host fixture" >&2; exit 97'
# curl with only file:// and loopback URLs runs the real curl. Refused: any other URL;
# one with a user name, whose host is after the @; and a bare host with a common
# top-level domain, which curl fetches over http (an option's value, such as
# `-o out.json`, must still pass).
LOCAL_CURL = """for arg in "$@"; do
  case "$arg" in
    file://*) ;;
    *://*@*) {refuse} ;;
    http://127.0.0.1|http://127.0.0.1[:/]*|http://localhost|http://localhost[:/]*|http://\\[::1\\]*) ;;
    *://*) {refuse} ;;
    -*) ;;
    *)
      if printf '%s' "$arg" | grep -Eq '^[A-Za-z0-9-]+(\\.[A-Za-z0-9-]+)*\\.(com|org|net|dev|io|ai|co|app|gov|edu|uk|de)([:/]|$)'; then
        {refuse}
      fi ;;
  esac
done
exec "{curl}" "$@"
"""


@pytest.fixture(autouse=True)
def no_browser(tmp_path_factory, monkeypatch):
    _no_browser(tmp_path_factory, monkeypatch)


def _no_browser(tmp_path_factory, monkeypatch):
    fakes = tmp_path_factory.mktemp("no-browser")
    for opener in OPENERS:
        fake = fakes / opener
        fake.write_text(f'#!/bin/sh\necho "$1" >> "{fakes}/opened"\n')
        fake.chmod(0o755)
    monkeypatch.setenv("PATH", f"{fakes}{os.pathsep}{os.environ['PATH']}")
    monkeypatch.setenv("NO_BROWSER_DIR", str(fakes))


@pytest.fixture(autouse=True)
def no_network_tools(tmp_path_factory, monkeypatch):
    """No test reaches a real gh, ssh, scp, sftp or curl: on 2026-11-06 a stubbed
    fixture's red run called the real `gh secret set` from inside the repo. Each is a
    refusal first on PATH, exit 97, except that curl of only `file://` and loopback
    URLs runs the real curl, as tests that serve a fake site on 127.0.0.1 need.
    fake_host's fakes go ahead of these. The one exception is the session-scoped
    real_caddy, set up before these, whose bin/caddy fetches the pinned Caddy."""
    _no_network_tools(tmp_path_factory, monkeypatch)


def _no_network_tools(tmp_path_factory, monkeypatch):
    fakes = tmp_path_factory.mktemp("no-network")
    for tool in NETWORK_TOOLS:
        fake = fakes / tool
        body = REFUSE + "\n"
        if tool == "curl" and REAL_CURL:
            body = LOCAL_CURL.format(refuse=REFUSE, curl=REAL_CURL)
        fake.write_text("#!/bin/sh\n" + body)
        fake.chmod(0o755)
    monkeypatch.setenv("PATH", f"{fakes}{os.pathsep}{os.environ['PATH']}")


@pytest.fixture(autouse=True)
def no_shared_sccache(monkeypatch):
    """No test's build goes through sccache: sandbox-setup sets RUSTC_WRAPPER=sccache,
    and when the server has gone idle a build starts it again, a server every session
    on the machine shares, which the test would leave running. Empty, not unset: a
    login or interactive bash sources /etc/sandbox-persistent.sh, which sets an unset
    one again; cargo takes an empty one as none."""
    _no_shared_sccache(monkeypatch)


def _no_shared_sccache(monkeypatch):
    monkeypatch.setenv("RUSTC_WRAPPER", "")


@pytest.fixture(autouse=True)
def own_tmpdir(tmp_path_factory, monkeypatch):
    """Each test's processes get an empty TMPDIR of its own, under pytest's base temp,
    so none writes to the real /tmp: agent-pre-commit keeps its check folder there, and
    the hook tests left one in /tmp per test repo. Python's own tempfile caches its
    folder before this runs, so this reaches only the processes a test starts."""
    _own_tmpdir(tmp_path_factory, monkeypatch)


def _own_tmpdir(tmp_path_factory, monkeypatch):
    monkeypatch.setenv("TMPDIR", str(tmp_path_factory.mktemp("tmpdir")))


@pytest.fixture(autouse=True)
def own_check_slot(tmp_path_factory, monkeypatch):
    """Each test's processes get a check slot of their own (bin/agent-pre-commit), so a
    hook a test runs never brings the real one, in the workspace's cache/, to its own
    scratch tree."""
    _own_check_slot(tmp_path_factory, monkeypatch)


def _own_check_slot(tmp_path_factory, monkeypatch):
    slot = tmp_path_factory.mktemp("slot") / "check-slot"
    monkeypatch.setenv("FILEDRAWER_CHECK_SLOT", str(slot))


@contextlib.contextmanager
def isolated(tmp_path_factory) -> Iterator[None]:
    """The autouse fixtures' environment, for a fixture wider than one test: pytest
    sets up a module's fixtures before the test's autouse ones, so what a module
    fixture starts would otherwise reach a real browser, ssh or curl."""
    with pytest.MonkeyPatch.context() as monkeypatch:
        _no_browser(tmp_path_factory, monkeypatch)
        _no_network_tools(tmp_path_factory, monkeypatch)
        _no_shared_sccache(monkeypatch)
        _own_tmpdir(tmp_path_factory, monkeypatch)
        _own_check_slot(tmp_path_factory, monkeypatch)
        yield


CADDY_SCRIPT = Path(__file__).resolve().parents[1] / "caddy"
# bin/caddy's exit status when Caddy can't be had on this machine (no network).
CADDY_UNAVAILABLE = 69


def installed_caddy(script: Path = CADDY_SCRIPT) -> str:
    """The Caddy the real-Caddy tests run: $FILEDRAWER_TEST_CADDY, else bin/caddy,
    which `version` here downloads and caches on its first run on a machine.

    Without a network the test skips, saying why, except in CI, which always has one,
    so there the tests can't quietly stop running. A wrong pin fails it."""
    override = os.environ.get("FILEDRAWER_TEST_CADDY")
    if override:
        return override
    done = subprocess.run(
        [str(script), "version"], capture_output=True, text=True, check=False
    )
    if done.returncode == CADDY_UNAVAILABLE and not os.environ.get("CI"):
        pytest.skip(f"no Caddy on this machine: {done.stderr.strip()}")
    assert done.returncode == 0, done.stderr
    return str(script)


@pytest.fixture(scope="session")
def real_caddy() -> str:
    """installed_caddy's Caddy. Session-scoped, so it runs before no_network_tools
    puts its curl first on PATH: this is the one place a test run may reach the
    network, to fetch the pinned Caddy from GitHub once per machine."""
    return installed_caddy()


def pytest_configure(config):
    config.addinivalue_line(
        "markers",
        "real_caddy: uses the real_caddy fixture; bin/agent-commit-check runs these"
        " when a Caddy file changed",
    )


def pytest_collection_modifyitems(items):
    """Mark each test that uses real_caddy, so `-m real_caddy` selects them all."""
    for item in items:
        if "real_caddy" in getattr(item, "fixturenames", ()):
            item.add_marker("real_caddy")


def unix_listener(path: Path) -> socket.socket:
    """A unix socket listening at PATH, bound from its folder, since a temp path
    runs past the 107 bytes a socket's address holds."""
    path.parent.mkdir(parents=True, exist_ok=True)
    listener = socket.socket(socket.AF_UNIX)
    before = os.getcwd()
    os.chdir(path.parent)
    try:
        listener.bind(path.name)
    finally:
        os.chdir(before)
    listener.listen()
    return listener


def serve_in_background(server: socketserver.BaseServer) -> threading.Thread:
    """Runs SERVER's serve_forever in a daemon thread, checking for shutdown() every
    10 ms: at the default 0.5 s, each test that stopped a server waited out most of
    a poll, which summed to about 70 s of the suite (#166)."""
    thread = threading.Thread(target=server.serve_forever, args=(0.01,), daemon=True)
    thread.start()
    return thread


def opened_urls() -> Path:
    """Where the no_browser fakes note the URLs a test's subprocess opened."""
    return Path(os.environ["NO_BROWSER_DIR"]) / "opened"


def uv_cached(*argv: str) -> bool:
    """Whether uvx runs `argv` offline, from uv's cache alone. The tests that run
    pre-commit or ruff skip without it, as on a machine that never fetched them."""
    if shutil.which("uvx") is None:
        return False
    done = subprocess.run(
        ["uvx", "--offline", "-q", *argv], capture_output=True, check=False
    )
    return done.returncode == 0


def hook_blocks(config: str) -> dict[str, str]:
    """The hooks in `config`, laid out as this repo's .pre-commit-config.yaml is: each
    hook's lines, by its id."""
    return {
        found.group(1): found.group(0)
        for found in re.finditer(r"^      - id: (\S+)\n(?:        .*\n)+", config, re.M)
    }


def local_config(blocks: Iterable[str]) -> str:
    """A pre-commit config of its own that holds `blocks`, as hook_blocks gives them."""
    return "repos:\n  - repo: local\n    hooks:\n" + "".join(blocks)


@pytest.fixture
def fake_host(tmp_path):
    """Fakes of ssh, gh, getent, curl and ssh-keygen, first on PATH in `env`; see
    FakeHost."""
    return FakeHost(tmp_path / "fake-host")


FAKE_TOOL = Path(__file__).with_name("fake_tool.sh")
FAKED = ("ssh", "gh", "getent", "curl", "ssh-keygen")


class FakeHost:
    """What the wizard's ssh, gh, getent, curl and ssh-keygen calls reach in a test.

    Every call is a line in `log`: "ssh <remote command line>" (the words after the
    host), or "<tool> <args>" for the others. Each ssh call's stdin is kept, read
    back with sent(n). A call answers from the latest rule that matches it, set with
    answer() or run_locally(), or with no output and status 0; but an unmatched ssh
    `test -f` or `test -x` answers 1, as a fresh server would, so a fixture that means
    a file is there says so with a rule.
    gh keeps what `gh secret set NAME --env staging` is given in `secrets/NAME` and
    lists them for `gh secret list`, unless a rule answers first.

    The fakes are links to fake_tool.sh, and each tool's rules a shell file of
    `case` statements, newest first, so a pattern takes `*` and `?` but not `[`.
    """

    def __init__(self, root: Path):
        self.root = root
        self.bin = root / "bin"
        self.bin.mkdir(parents=True)
        self.log = root / "log"
        self.log.touch()
        self.secrets = root / "secrets"
        self.secrets.mkdir()
        (root / "sent").mkdir()
        (root / "rules").mkdir()
        self._rules: dict[str, list[str]] = {tool: [] for tool in FAKED}
        for tool in FAKED:
            self._save_rules(tool)
            (self.bin / tool).symlink_to(FAKE_TOOL)

    @property
    def env(self) -> dict[str, str]:
        """os.environ with the fakes first on PATH."""
        return {**os.environ, "PATH": f"{self.bin}{os.pathsep}{os.environ['PATH']}"}

    def answer(
        self,
        pattern: str,
        stdout: str = "",
        status: int = 0,
        *,
        tool: str = "ssh",
        stdin: str | None = None,
    ) -> None:
        """Answer `tool` calls whose command line matches the glob `pattern` (and whose
        stdin contains `stdin`, when given) with `stdout` and `status`."""
        self._add_rule(tool, pattern, stdin, stdout=stdout, status=status)

    def run_locally(self, pattern: str = "*") -> None:
        """Answer ssh calls whose remote command line matches the glob `pattern` by
        running it through bash here, on the call's stdin, as the server would."""
        self._add_rule("ssh", pattern, None, run_locally=True)

    def _add_rule(
        self, tool, pattern, stdin, *, stdout="", status=0, run_locally=False
    ):
        if run_locally:
            act = 'exec bash -c "$line" <"$sent"'
        else:
            act = f"printf %s {sh_word(stdout)}; exit {int(status)}"
        if stdin is not None:
            act = f"case $stdin in *{sh_word(stdin)}*) {act};; esac"
        self._rules[tool].append(f"case $line in {case_pattern(pattern)}) {act};; esac")
        self._save_rules(tool)

    def calls(self, tool: str = "ssh") -> list[str]:
        """The command lines `tool` was called with, in order."""
        return logged_calls(self.log.read_text(), tool)

    def sent(self, n: int) -> str:
        """The stdin of the nth ssh call, counting from 1."""
        return (self.root / "sent" / str(n)).read_text()

    def _save_rules(self, tool: str) -> None:
        lines = reversed(self._rules[tool])
        (self.root / "rules" / f"{tool}.sh").write_text(
            "".join(f"{x}\n" for x in lines)
        )


def logged_calls(log: str, tool: str) -> list[str]:
    """The command lines `tool` was called with, in order, in a FakeHost's log."""
    prefix = f"{tool} "
    return [line[len(prefix) :] for line in log.splitlines() if line.startswith(prefix)]


def sh_word(text: str) -> str:
    """`text` as one single-quoted shell word."""
    return "'" + text.replace("'", "'\\''") + "'"


def case_pattern(glob: str) -> str:
    """The glob `glob`, as fnmatch reads it, as a `case` pattern: `*` and `?` bare,
    the rest quoted. A `[`, which fnmatch reads as a set, is refused rather than
    matched another way."""
    if "[" in glob:
        raise ValueError(f"FakeHost patterns take * and ? only: {glob!r}")
    parts = re.split(r"([*?])", glob)
    return "".join(p if p in ("*", "?") else sh_word(p) for p in parts if p) or "''"
