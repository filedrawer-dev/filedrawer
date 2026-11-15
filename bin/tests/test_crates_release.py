"""Tests for bin/crates-release, which release-cli.yml's crates job runs (#84): it
trades the job's GitHub token for a short-lived crates.io token through crates.io's
trusted publishing, publishes filedrawer-analyzer and then filedrawer, each only
when asked, and revokes the token, whatever happened.

They run it against fakes of curl, answering GitHub's token endpoint and crates.io's
trusted publishing endpoint, and of cargo, which records each publish and the
token it was given.
"""

import json
import os
import subprocess
from pathlib import Path

import pytest

REPO = Path(__file__).resolve().parents[2]
SCRIPT = REPO / "bin/crates-release"
JWT = "github-jwt-for-crates"
TOKEN = "cio-short-lived-s3cret"
TOKEN_URL = "https://pipelines.example/token?api-version=2.0"

FAKE = r'''#!/usr/bin/env python3
"""curl or cargo, by the name it was called with, against $FAKE_STATE."""
import json, os, sys
from pathlib import Path

state = Path(os.environ["FAKE_STATE"])
tool, args = Path(sys.argv[0]).name, sys.argv[1:]


def log(entry):
    with (state / "log").open("a") as f:
        f.write(json.dumps(entry) + "\n")


def header(name):
    """The value of header NAME, given as -H 'NAME: value' or in a file as -H @FILE."""
    for i, arg in enumerate(args):
        if arg == "-H":
            value = args[i + 1]
            lines = Path(value[1:]).read_text().splitlines() if value.startswith("@") else [value]
            for line in lines:
                key, _, rest = line.partition(":")
                if key.strip().lower() == name.lower():
                    return rest.strip()
    return None


if tool == "cargo":
    log({
        "tool": "cargo",
        "args": args,
        "token": os.environ.get("CARGO_REGISTRY_TOKEN"),
        "github_token_env": sorted(k for k in os.environ if k.startswith("ACTIONS_ID_TOKEN_REQUEST")),
    })
    name = args[args.index("-p") + 1]
    sys.exit(1 if name == os.environ.get("FAKE_PUBLISH_FAILS") else 0)

url = args[-1]
method = args[args.index("-X") + 1] if "-X" in args else ("POST" if "--data-binary" in args else "GET")
data = sys.stdin.read() if "--data-binary" in args and args[args.index("--data-binary") + 1] == "@-" else None
log({"tool": "curl", "method": method, "url": url, "argv": args, "authorization": header("Authorization"), "data": data})
if url.startswith(os.environ["ACTIONS_ID_TOKEN_REQUEST_URL"].split("?")[0]):
    assert header("Authorization") == "Bearer " + os.environ["ACTIONS_ID_TOKEN_REQUEST_TOKEN"]
    print(json.dumps({"value": os.environ["FAKE_JWT"]}))
    sys.exit(0)
if url == "https://crates.io/api/v1/trusted_publishing/tokens":
    if method == "POST":
        if os.environ.get("FAKE_EXCHANGE_REFUSED"):
            print('{"errors":[{"detail":"no matching trusted publisher"}]}')
            sys.exit(22)
        assert json.loads(data) == {"jwt": os.environ["FAKE_JWT"]}
        print(json.dumps({"token": os.environ["FAKE_TOKEN"]}))
        sys.exit(0)
    if method == "DELETE":
        sys.exit(22 if os.environ.get("FAKE_REVOKE_FAILS") else 0)
sys.exit(6)
'''


@pytest.fixture
def box(tmp_path):
    state = tmp_path / "state"
    state.mkdir()
    (state / "log").touch()
    fakes = tmp_path / "fakes"
    fakes.mkdir()
    for tool in ("curl", "cargo"):
        (fakes / tool).write_text(FAKE)
        (fakes / tool).chmod(0o755)
    env = {
        **os.environ,
        "PATH": f"{fakes}{os.pathsep}{os.environ['PATH']}",
        "FAKE_STATE": str(state),
        "FAKE_JWT": JWT,
        "FAKE_TOKEN": TOKEN,
        "ACTIONS_ID_TOKEN_REQUEST_URL": TOKEN_URL,
        "ACTIONS_ID_TOKEN_REQUEST_TOKEN": "runner-request-token",
    }

    class Box:
        def run(self, *args: str, **extra: str):
            return subprocess.run(
                [str(SCRIPT), *args],
                cwd=tmp_path,
                env={**env, **extra},
                capture_output=True,
                text=True,
                check=False,
            )

        def log(self) -> list[dict]:
            return [
                json.loads(line) for line in (state / "log").read_text().splitlines()
            ]

        def published(self) -> list[str]:
            return [
                e["args"][e["args"].index("-p") + 1]
                for e in self.log()
                if e["tool"] == "cargo"
            ]

        def revoked(self) -> bool:
            return any(
                e["tool"] == "curl"
                and e["method"] == "DELETE"
                and e["authorization"] == f"Bearer {TOKEN}"
                for e in self.log()
            )

    return Box()


def test_it_publishes_the_analyzer_then_the_cli_with_the_token_then_revokes_it(box):
    done = box.run("--analyzer", "--cli")
    assert done.returncode == 0, done.stdout + done.stderr
    assert box.published() == ["filedrawer-analyzer", "filedrawer"]
    cargo = [e for e in box.log() if e["tool"] == "cargo"]
    assert all(
        e["args"][:3] == ["publish", "--locked", "--no-verify"] and e["token"] == TOKEN
        for e in cargo
    )
    # The token is asked for with audience crates.io.
    first = box.log()[0]
    assert first["url"] == TOKEN_URL + "&audience=crates.io"
    assert box.revoked()


def test_cargo_builds_nothing_and_cant_mint_another_token(box):
    """A dependency's build script would run beside the token: CI's check built
    this commit already, so cargo packs without building, and the job's GitHub
    token variables are gone before it starts."""
    done = box.run("--analyzer", "--cli")
    assert done.returncode == 0, done.stdout + done.stderr
    for entry in (e for e in box.log() if e["tool"] == "cargo"):
        assert "--no-verify" in entry["args"]
        assert entry["github_token_env"] == []


def test_it_publishes_only_the_crates_asked_for(box):
    done = box.run("--cli")
    assert done.returncode == 0, done.stdout + done.stderr
    assert box.published() == ["filedrawer"]


def test_a_failed_publish_still_revokes_the_token_and_fails(box):
    done = box.run("--analyzer", "--cli", FAKE_PUBLISH_FAILS="filedrawer-analyzer")
    assert done.returncode != 0
    assert box.published() == ["filedrawer-analyzer"]
    assert box.revoked()


def test_a_refused_exchange_publishes_nothing_and_says_why(box):
    done = box.run("--analyzer", "--cli", FAKE_EXCHANGE_REFUSED="1")
    assert done.returncode != 0
    assert box.published() == []
    assert "trusted publisher" in done.stdout + done.stderr


def test_a_failed_revoke_warns_but_passes(box):
    """The token lapses on its own within the hour."""
    done = box.run("--cli", FAKE_REVOKE_FAILS="1")
    assert done.returncode == 0, done.stdout + done.stderr
    assert "::warning::" in done.stdout


def test_neither_secret_is_printed_or_put_on_a_command_line(box):
    done = box.run("--analyzer", "--cli")
    assert done.returncode == 0, done.stdout + done.stderr
    for secret in (TOKEN, JWT):
        assert secret not in done.stdout + done.stderr
        for entry in box.log():
            if entry["tool"] == "curl":
                assert not any(secret in arg for arg in entry["argv"]), entry["argv"]


def test_without_the_jobs_id_token_it_says_what_the_job_needs(box):
    done = box.run("--cli", ACTIONS_ID_TOKEN_REQUEST_URL="")
    assert done.returncode != 0
    assert "id-token: write" in done.stdout
    assert box.published() == []


def test_with_nothing_asked_it_does_nothing(box):
    done = box.run()
    assert done.returncode == 2
    assert box.log() == []


@pytest.mark.parametrize("flag", ["-h", "--help"])
def test_help_says_what_it_does(flag):
    done = subprocess.run(
        [str(SCRIPT), flag], capture_output=True, text=True, check=False
    )
    assert done.returncode == 0
    assert done.stdout.startswith("usage: bin/crates-release")
