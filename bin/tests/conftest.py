"""Fixtures every test in bin/tests gets, and fake_host for the wizard's tests.

No test opens a browser: the wizard's open_url calls the first of wslview,
explorer.exe, xdg-open or open on PATH, and on 2026-11-06 wizard tests opened tabs on
the owner's machine. Every test runs with no-op fakes of all four first on PATH, so
a test that builds its subprocess's environment from os.environ, as they do, can't
reach a real one. Each fake notes the URL in NO_BROWSER_DIR/opened. A test may still
put its own recording fakes ahead of these.
"""

import json
import os
import shutil
import sys
from pathlib import Path

import pytest

# Autouse fixtures, registered by import: each test, and each module's fixtures, fail
# when they leave a process running. leftover_processes.py is a copy of llm-toolbox's,
# which the host has none of to import: change it there and copy it back.
from leftover_processes import (  # noqa: F401
    _leftover_processes_of_module,
    _leftover_processes_of_test,
)

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
    fake_host's fakes go ahead of these."""
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
    on the machine shares, which the test would leave running. Empty, not unset: a bash
    sources /etc/sandbox-persistent.sh, which sets an unset one again; cargo takes an
    empty one as none."""
    monkeypatch.setenv("RUSTC_WRAPPER", "")


def opened_urls() -> Path:
    """Where the no_browser fakes note the URLs a test's subprocess opened."""
    return Path(os.environ["NO_BROWSER_DIR"]) / "opened"


@pytest.fixture
def fake_host(tmp_path):
    """Fakes of ssh, gh, getent, curl and ssh-keygen, first on PATH in `env`; see
    FakeHost."""
    return FakeHost(tmp_path / "fake-host")


FAKE_TOOL = Path(__file__).with_name("fake_tool.py")
FAKED = ("ssh", "gh", "getent", "curl", "ssh-keygen")


class FakeHost:
    """What the wizard's ssh, gh, getent, curl and ssh-keygen calls reach in a test.

    Every call is a line in `log`: "ssh <remote command line>" (the words after the
    host), or "<tool> <args>" for the others. Each ssh call's stdin is kept, read
    back with sent(n). A call answers from the latest rule that matches it, set with
    answer() or run_locally(), or with no output and status 0.
    gh keeps what `gh secret set NAME --env staging` is given in `secrets/NAME` and
    lists them for `gh secret list`, unless a rule answers first.
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
        self._rules: list[dict] = []
        self._save_rules()
        for tool in FAKED:
            fake = self.bin / tool
            fake.write_text(
                f'#!/bin/sh\nexec "{sys.executable}" -I "{FAKE_TOOL}" "{root}" {tool} "$@"\n'
            )
            fake.chmod(0o755)

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
        self._rules.append(
            {
                "tool": tool,
                "pattern": pattern,
                "stdin": stdin,
                "stdout": stdout,
                "status": status,
                "run_locally": run_locally,
            }
        )
        self._save_rules()

    def calls(self, tool: str = "ssh") -> list[str]:
        """The command lines `tool` was called with, in order."""
        prefix = f"{tool} "
        return [
            line[len(prefix) :]
            for line in self.log.read_text().splitlines()
            if line.startswith(prefix)
        ]

    def sent(self, n: int) -> str:
        """The stdin of the nth ssh call, counting from 1."""
        return (self.root / "sent" / str(n)).read_text()

    def _save_rules(self) -> None:
        (self.root / "rules.json").write_text(json.dumps(self._rules))
