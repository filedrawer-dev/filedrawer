"""ops/usability-run's run: what it hands drive-agent, and what it refuses before
any account or agent exists.

A fake drive-agent first on PATH records its arguments and the variables the run
named for it, and a fake server answers /staging/status and /tags. Scenarios here
use token "none", which makes no account, so no signup or mail is needed; the
account's own steps are tested against a real server in
crates/server/tests/it/usability.rs.
"""

import datetime
import http.server
import json
import os
import subprocess
import sys
import threading
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "ops" / "usability-run"

FAKE_DRIVE_AGENT = """#!/usr/bin/env python3
import json, os, sys
names = [sys.argv[i + 1] for i, a in enumerate(sys.argv) if a in ("--var", "--setup-env", "--agent-env")]
json.dump({"argv": sys.argv[1:], "env": {n: os.environ.get(n) for n in names}},
          open(os.environ["FAKE_DRIVE_LOG"], "w"))
"""


class Handler(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        ok = self.path == "/staging/status" or self.path == "/tags?key=open-key"
        self.send_response(200 if ok else 403)
        self.end_headers()

    def log_message(self, *args):
        pass


@pytest.fixture
def server():
    httpd = http.server.HTTPServer(("127.0.0.1", 0), Handler)
    thread = threading.Thread(target=httpd.serve_forever, daemon=True)
    thread.start()
    yield f"http://127.0.0.1:{httpd.server_port}"
    httpd.shutdown()
    httpd.server_close()


@pytest.fixture
def setting(tmp_path):
    bin_dir = tmp_path / "bin"
    bin_dir.mkdir()
    (bin_dir / "drive-agent").write_text(FAKE_DRIVE_AGENT)
    (bin_dir / "drive-agent").chmod(0o755)
    folder = tmp_path / "usability"
    (folder / "scenarios").mkdir(parents=True)
    (folder / "persona.md").write_text("You are Jason.")
    expires = datetime.datetime.now(datetime.UTC) + datetime.timedelta(hours=8)
    key = tmp_path / "key"
    key.write_text(f"key=open-key\nexpires={expires:%Y-%m-%dT%H:%M:%SZ}\n")
    env = {k: v for k, v in os.environ.items() if not k.startswith("FILEDRAWER_")}
    env.update(
        PATH=f"{bin_dir}:{os.environ['PATH']}",
        FILEDRAWER_USABILITY_DIR=str(folder),
        FILEDRAWER_FIELD_TEST_KEY_FILE=str(key),
        FAKE_DRIVE_LOG=str(tmp_path / "drive.json"),
    )
    return {"env": env, "folder": folder, "key": key, "log": tmp_path / "drive.json"}


def scenario(setting, name="s1.json", **keys):
    path = setting["folder"] / "scenarios" / name
    path.write_text(json.dumps({"gated": ["posting"], "token": "none", **keys}))
    return name


def usability_run(setting, *args):
    return subprocess.run(
        [str(SCRIPT), *args],
        env=setting["env"],
        capture_output=True,
        text=True,
        check=False,
    )


def test_a_run_hands_drive_agent_the_setup_persona_and_key(setting, server):
    name = scenario(setting, turns=["Read {BASE}/skill.md?key={KEY}"])
    done = usability_run(setting, name, "codex", "jason7", "--base", server)
    assert done.returncode == 0, done.stderr
    seen = json.loads(setting["log"].read_text())
    argv = seen["argv"]
    assert argv[:2] == ["run", str(setting["folder"] / "scenarios" / name)]
    assert argv[argv.index("--harness") + 1] == "codex"
    assert argv[argv.index("--name") + 1] == "jason7"
    assert argv[argv.index("--setup") + 1] == str(
        ROOT / "ops" / "usability" / "setup.py"
    )
    assert argv[argv.index("--persona") + 1] == str(setting["folder"] / "persona.md")
    out = Path(argv[argv.index("--out") + 1])
    assert (
        out.parent.parent == setting["folder"] / "runs" and out.name == "jason7-codex"
    )
    assert seen["env"]["KEY"] == "open-key"
    assert seen["env"]["FILEDRAWER_USABILITY_BASE"] == server
    # The key reaches drive-agent by name only, never on a command line.
    assert "open-key" not in " ".join(argv)
    assert "FILEDRAWER_TOKEN" not in seen["env"]
    assert "ops/field-test-key close" in done.stdout


@pytest.mark.parametrize(
    "key_file, why",
    [
        ("closed=2026-11-10T00:00:00Z\n", "no open key"),
        ("key=open-key\nexpires=unknown\n", "expires=unknown"),
        ("key=open-key\nexpires=2026-01-01T00:00:00Z\n", "expires within 2 hours"),
        ("key=shut-key\nexpires=2999-01-01T00:00:00Z\n", "isn't open"),
    ],
)
def test_a_key_that_wont_last_is_refused_before_the_run(setting, server, key_file, why):
    setting["key"].write_text(key_file)
    name = scenario(setting, turns=["Read skill.md?key={KEY}"])
    done = usability_run(setting, name, "claude", "jason8", "--base", server)
    assert done.returncode == 2 and why in done.stderr, done.stderr
    assert not setting["log"].exists()


def test_a_scenario_without_gated_actions_is_refused(setting, server):
    path = setting["folder"] / "scenarios" / "bare.json"
    path.write_text(json.dumps({"turns": ["hi"], "token": "none"}))
    done = usability_run(setting, "bare.json", "claude", "jason9", "--base", server)
    assert done.returncode == 2 and "gated" in done.stderr
    assert not setting["log"].exists()


def test_production_is_refused(setting):
    name = scenario(setting, turns=["hi"])
    done = usability_run(
        setting, name, "claude", "jason10", "--base", "https://filedrawer.dev"
    )
    assert done.returncode == 2 and "--base must be" in done.stderr


def test_an_unknown_token_mode_is_refused(setting, server):
    name = scenario(setting, turns=["hi"], token="headr")
    done = usability_run(setting, name, "claude", "jason11", "--base", server)
    assert done.returncode == 2 and "header, env or none" in done.stderr
    assert not setting["log"].exists()


# Each run of a batch: says it started, waits for the run named in FAKE_WAIT_<NAME>
# to start (so runs one after another would never end), then saves run.json in its
# --out as drive-agent does; a NAME ending in 9 times out.
FAKE_BATCH_AGENT = """#!/usr/bin/env python3
import json, os, sys, time
a = sys.argv
name, out = a[a.index("--name") + 1], a[a.index("--out") + 1]
d = os.environ["FAKE_DIR"]
open(os.path.join(d, "started-" + name), "w").close()
other = os.environ.get("FAKE_WAIT_" + name)
while other and not os.path.exists(os.path.join(d, "started-" + other)):
    time.sleep(0.05)
os.makedirs(out)
outcome = "timeout" if name.endswith("9") else "done"
open(os.path.join(out, "run.json"), "w").write(json.dumps({"outcome": outcome}))
print(name + ": " + outcome + " key " + os.environ.get("KEY", "-"))
sys.exit(outcome != "done")
"""


@pytest.fixture
def batch_setting(setting, tmp_path):
    (tmp_path / "bin" / "drive-agent").write_text(FAKE_BATCH_AGENT)
    setting["env"].update(FAKE_DIR=str(tmp_path), FAKE_WAIT_jason1="jason9")
    return setting


def test_a_batch_runs_in_parallel_and_prints_one_line_a_run(batch_setting, server):
    scenario(batch_setting, "s1.json", turns=["Read skill.md?key={KEY}"])
    scenario(batch_setting, "s2.json", turns=["hi"])
    done = usability_run(
        batch_setting,
        "batch",
        "s1:claude:jason1",
        "s2.json:codex:jason9",
        "--base",
        server,
    )
    assert done.returncode == 1, done.stderr
    lines = done.stdout.splitlines()
    runs = batch_setting["folder"] / "runs"
    day = next(runs.iterdir())
    assert lines[0] == f"jason1-claude: exit 0, done, {day / 'jason1-claude'}"
    assert lines[1] == f"jason9-codex: exit 1, timeout, {day / 'jason9-codex'}"
    logs = batch_setting["folder"] / "logs"
    assert "jason1: done" in (logs / "s1-claude-jason1.log").read_text()
    assert "jason9: timeout" in (logs / "s2-codex-jason9.log").read_text()
    # The key reaches each run by name only, and nothing prints it.
    assert "open-key" not in done.stdout + done.stderr
    assert done.stdout.count("ops/field-test-key close") == 1


def test_a_batch_refuses_one_name_twice_before_any_run(batch_setting, server):
    scenario(batch_setting, "s1.json", turns=["hi"])
    done = usability_run(
        batch_setting, "batch", "s1:claude:jason3", "s1:codex:jason3", "--base", server
    )
    assert done.returncode == 2 and "jason3" in done.stderr
    assert not (batch_setting["folder"] / "logs").exists()


def test_a_batch_refuses_a_run_whose_folder_exists(batch_setting, server):
    scenario(batch_setting, "s1.json", turns=["hi"])
    day = batch_setting["folder"] / "runs" / datetime.date.today().isoformat()
    (day / "jason6-claude").mkdir(parents=True)
    done = usability_run(batch_setting, "batch", "s1:claude:jason6", "--base", server)
    assert done.returncode == 2 and "jason6-claude" in done.stderr
    assert not (batch_setting["folder"] / "logs").exists()


@pytest.mark.parametrize(
    "spec", ["s1:claude", "s1:gemini:jason4", "s1:claude:Jason4", "s0:claude:jason4"]
)
def test_a_batch_refuses_a_malformed_run(batch_setting, server, spec):
    scenario(batch_setting, "s1.json", turns=["hi"])
    done = usability_run(batch_setting, "batch", spec, "--base", server)
    assert done.returncode == 2 and spec in done.stderr
    assert not (batch_setting["folder"] / "logs").exists()


SETUP = ROOT / "ops" / "usability" / "setup.py"


def run_setup(tmp_path, keys, **env):
    home = tmp_path / "home"
    home.mkdir(exist_ok=True)
    path = tmp_path / "scenario.json"
    path.write_text(json.dumps(keys))
    base_env = {
        "PATH": os.environ["PATH"],
        "HOME": str(home),
        "FILEDRAWER_USABILITY_BASE": "https://staging.filedrawer.dev",
    }
    done = subprocess.run(
        [sys.executable, str(SETUP), str(path)],
        env={**base_env, **env},
        capture_output=True,
        text=True,
        check=False,
    )
    return done, home


def test_setup_writes_the_header_file_with_the_key_and_the_token(tmp_path):
    done, home = run_setup(
        tmp_path,
        {"token": "header"},
        FILEDRAWER_STAGING_KEY="k-1",
        FILEDRAWER_TOKEN="fdt_t",
        FILEDRAWER_USABILITY_HARNESS="codex",
    )
    assert done.returncode == 0, done.stderr
    header = home / ".config" / "filedrawer" / "staging.filedrawer.dev.header"
    assert header.read_text() == "X-Staging-Key: k-1\nAuthorization: Bearer fdt_t\n"
    assert header.stat().st_mode & 0o777 == 0o600
    assert not (home / ".claude").exists()


def test_setup_leaves_the_token_out_unless_the_scenario_puts_it_there(tmp_path):
    done, home = run_setup(
        tmp_path,
        {"token": "env"},
        FILEDRAWER_STAGING_KEY="k-1",
        FILEDRAWER_TOKEN="fdt_t",
    )
    assert done.returncode == 0, done.stderr
    header = home / ".config" / "filedrawer" / "staging.filedrawer.dev.header"
    assert header.read_text() == "X-Staging-Key: k-1\n"


def test_setup_makes_claude_codes_folder_and_blocks_agents_when_asked(tmp_path):
    done, home = run_setup(
        tmp_path,
        {"agents_blocked": True},
        FILEDRAWER_STAGING_KEY="k",
        FILEDRAWER_USABILITY_HARNESS="claude",
    )
    assert done.returncode == 0, done.stderr
    assert (home / ".claude").is_dir()
    assert (home / ".agents").is_file()


def test_setup_without_the_token_it_needs_says_so(tmp_path):
    done, _ = run_setup(tmp_path, {"token": "header"}, FILEDRAWER_STAGING_KEY="k")
    assert done.returncode == 1 and "FILEDRAWER_TOKEN is unset" in done.stderr
    assert "Traceback" not in done.stderr


# A stand-in for /install.sh that records its argument and the header file it found.
FAKE_INSTALLER = b"""printf '%s\\n' "$*" > "$HOME/installed-with"
cp "$HOME/.config/filedrawer/127.0.0.1.header" "$HOME/header-seen"
"""


class InstallerSite(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        keyed = self.headers.get("X-Staging-Key") == "k"
        ok = self.path == "/install.sh" and keyed
        self.send_response(200 if ok else 403)
        self.end_headers()
        if ok:
            self.wfile.write(FAKE_INSTALLER)

    def log_message(self, *args):
        pass


@pytest.fixture
def installer_site():
    httpd = http.server.HTTPServer(("127.0.0.1", 0), InstallerSite)
    thread = threading.Thread(target=httpd.serve_forever, daemon=True)
    thread.start()
    yield f"http://127.0.0.1:{httpd.server_port}"
    httpd.shutdown()
    httpd.server_close()


def test_setup_installs_with_the_sites_installer_in_the_scenarios_mode(
    tmp_path, installer_site
):
    done, home = run_setup(
        tmp_path,
        {"install": "installed", "mode": "ask"},
        FILEDRAWER_STAGING_KEY="k",
        FILEDRAWER_USABILITY_BASE=installer_site,
    )
    assert done.returncode == 0, done.stderr
    assert (home / "installed-with").read_text() == "ask\n"
    # The installer ran after the header file was written, so its own requests carry
    # the key.
    assert (home / "header-seen").read_text() == "X-Staging-Key: k\n"
    assert (home / ".config" / "filedrawer" / "install.sh").exists()
