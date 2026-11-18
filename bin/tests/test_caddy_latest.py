"""Tests for bin/caddy-latest, which .github/workflows/caddy-latest.yml runs (#100),
with fakes of gh and curl first on PATH: gh answers from a JSON world and notes the
issues it opens and the comments it adds; curl serves files from a folder.
"""

import hashlib
import io
import json
import stat
import subprocess
import tarfile
from pathlib import Path
from types import SimpleNamespace

import pytest

REPO = Path(__file__).resolve().parents[2]
SCRIPT = REPO / "bin/caddy-latest"
RUN = "https://github.com/filedrawer-dev/filedrawer/actions/runs/7"
TITLE = "Caddy 2.12.0 is out; bin/caddy pins 2.11.9"

FAKE_GH = """#!/usr/bin/env python3
import json, sys
from pathlib import Path
world_path = Path({world!r})
world = json.loads(world_path.read_text())
args = sys.argv[1:]
with open({log!r}, "a") as log:
    log.write(json.dumps(args) + "\\n")
def answer(value):
    print(json.dumps(value))
    sys.exit(0)
if args[:1] == ["api"]:
    path = args[1]
    if path in world["api"]:
        answer(world["api"][path])
    print("gh: Not Found (HTTP 404)", file=sys.stderr)
    sys.exit(1)
if args[:2] == ["issue", "list"]:
    answer([{{k: i[k] for k in ("number", "title", "state")}} for i in world["issues"]])
if args[:2] == ["issue", "view"]:
    [issue] = [i for i in world["issues"] if str(i["number"]) == args[2]]
    answer({{"body": issue["body"], "comments": [{{"body": c}} for c in issue["comments"]]}})
if args[:2] == ["issue", "create"]:
    title = args[args.index("--title") + 1]
    body = args[args.index("--body") + 1]
    world["issues"].append({{"number": len(world["issues"]) + 1, "title": title,
        "state": "OPEN", "body": body, "comments": []}})
elif args[:2] == ["issue", "comment"]:
    [issue] = [i for i in world["issues"] if str(i["number"]) == args[2]]
    issue["comments"].append(args[args.index("--body") + 1])
else:
    sys.exit(2)
world_path.write_text(json.dumps(world))
"""

FAKE_CURL = """#!/bin/sh
# Serves URL's last path part from the files folder into -o's path.
out=""; url=""
while [ $# -gt 0 ]; do
  case "$1" in
    -o) out=$2; shift ;;
    http*) url=$1 ;;
  esac
  shift
done
[ -f {files}/"${{url##*/}}" ] || exit 22
cp {files}/"${{url##*/}}" "$out"
"""


def executable(path: Path, text: str) -> None:
    path.write_text(text)
    path.chmod(path.stat().st_mode | stat.S_IXUSR)


def archive(binary: bytes = b"#!/bin/sh\necho v2.12.0\n") -> bytes:
    data = io.BytesIO()
    with tarfile.open(fileobj=data, mode="w:gz") as tar:
        info = tarfile.TarInfo("caddy")
        info.size = len(binary)
        info.mode = 0o755
        tar.addfile(info, io.BytesIO(binary))
    return data.getvalue()


@pytest.fixture
def ci(tmp_path):
    """A checkout whose bin/caddy pins 2.11.9, and a GitHub where Caddy's latest
    release is 2.12.0, its linux_amd64 archive matching its checksums and digest."""
    checkout = tmp_path / "checkout"
    (checkout / "bin").mkdir(parents=True)
    (checkout / "bin/caddy").write_text('VERSION = "2.11.9"\n')
    files = tmp_path / "files"
    files.mkdir()
    asset = "caddy_2.12.0_linux_amd64.tar.gz"
    data = archive()
    (files / asset).write_bytes(data)
    (files / "caddy_2.12.0_checksums.txt").write_text(
        f"{hashlib.sha512(b'other').hexdigest()}  caddy_2.12.0_mac_arm64.tar.gz\n"
        f"{hashlib.sha512(data).hexdigest()}  {asset}\n"
    )
    world = {
        "api": {
            "repos/caddyserver/caddy/releases/latest": {"tag_name": "v2.12.0"},
            "repos/caddyserver/caddy/releases/tags/v2.12.0": {
                "assets": [
                    {
                        "name": asset,
                        "digest": f"sha256:{hashlib.sha256(data).hexdigest()}",
                    }
                ]
            },
        },
        "issues": [],
    }
    fakes = tmp_path / "fakes"
    fakes.mkdir()
    world_path = tmp_path / "world.json"
    log = tmp_path / "gh.log"
    executable(fakes / "gh", FAKE_GH.format(world=str(world_path), log=str(log)))
    executable(fakes / "curl", FAKE_CURL.format(files=files))
    state = SimpleNamespace(world=world, files=files, data=data, asset=asset)

    def run(*args: str) -> subprocess.CompletedProcess:
        world_path.write_text(json.dumps(state.world))
        result = subprocess.run(
            [str(SCRIPT), *args],
            cwd=checkout,
            env={"PATH": f"{fakes}:/usr/bin:/bin"},
            stdin=subprocess.DEVNULL,
            capture_output=True,
            text=True,
            check=False,
        )
        state.world = json.loads(world_path.read_text())
        return result

    state.run = run
    state.out = tmp_path / "out"
    return state


def outputs(result) -> dict[str, str]:
    return dict(line.split("=", 1) for line in result.stdout.splitlines())


# versions


def test_versions_says_a_newer_release_is_needed(ci):
    result = ci.run("versions")
    assert result.returncode == 0, result.stderr
    assert outputs(result) == {"latest": "2.12.0", "pinned": "2.11.9", "needed": "true"}


@pytest.mark.parametrize("tag", ["v2.11.9", "v2.11.8", "v2.9.10"])
def test_versions_needs_nothing_for_a_release_at_or_behind_the_pin(ci, tag):
    ci.world["api"]["repos/caddyserver/caddy/releases/latest"] = {"tag_name": tag}
    result = ci.run("versions")
    assert result.returncode == 0, result.stderr
    assert outputs(result)["needed"] == "false"


@pytest.mark.parametrize("tag", ["2.12.0", "v2.12.0-beta.1", "", "v2.12"])
def test_versions_fails_on_a_tag_it_doesnt_expect(ci, tag):
    ci.world["api"]["repos/caddyserver/caddy/releases/latest"] = {"tag_name": tag}
    result = ci.run("versions")
    assert result.returncode != 0
    assert "tag" in result.stderr


# fetch


def test_fetch_extracts_a_caddy_whose_hashes_match(ci):
    result = ci.run("fetch", "2.12.0", str(ci.out))
    assert result.returncode == 0, result.stderr
    caddy = ci.out / "caddy"
    assert result.stdout.strip() == str(caddy)
    assert caddy.read_bytes() == b"#!/bin/sh\necho v2.12.0\n"


def test_fetch_refuses_an_archive_the_checksums_file_doesnt_list(ci):
    (ci.files / "caddy_2.12.0_checksums.txt").write_text(f"{'0' * 128}  {ci.asset}\n")
    result = ci.run("fetch", "2.12.0", str(ci.out))
    assert result.returncode != 0
    assert "SHA-512" in result.stderr
    assert not (ci.out / "caddy").exists()


@pytest.mark.parametrize("digest", ["sha256:" + "0" * 64, None])
def test_fetch_refuses_a_missing_or_wrong_digest(ci, digest):
    [asset] = ci.world["api"]["repos/caddyserver/caddy/releases/tags/v2.12.0"]["assets"]
    if digest is None:
        del asset["digest"]
    else:
        asset["digest"] = digest
    result = ci.run("fetch", "2.12.0", str(ci.out))
    assert result.returncode != 0
    assert "SHA-256" in result.stderr
    assert not (ci.out / "caddy").exists()


def test_fetch_fails_on_a_renamed_asset(ci):
    (ci.files / ci.asset).rename(ci.files / "caddy_2.12.0_linux_x86_64.tar.gz")
    result = ci.run("fetch", "2.12.0", str(ci.out))
    assert result.returncode != 0


# report


def test_report_opens_an_issue_naming_the_verdict_and_the_run(ci):
    result = ci.run("report", "2.12.0", "2.11.9", "passed", RUN)
    assert result.returncode == 0, result.stderr
    [issue] = ci.world["issues"]
    assert issue["title"] == TITLE
    assert "The real-Caddy tests passed against Caddy 2.12.0" in issue["body"]
    assert RUN in issue["body"]
    assert "apt" in issue["body"]


def test_report_without_a_verdict_says_the_tests_didnt_finish(ci):
    ci.run("report", "2.12.0", "2.11.9", "", RUN)
    [issue] = ci.world["issues"]
    assert "didn't finish" in issue["body"]


def test_report_with_the_same_verdict_adds_nothing(ci):
    ci.run("report", "2.12.0", "2.11.9", "failed", RUN)
    ci.run("report", "2.12.0", "2.11.9", "failed", RUN.replace("7", "8"))
    [issue] = ci.world["issues"]
    assert issue["comments"] == []


def test_report_comments_when_the_verdict_changes(ci):
    ci.run("report", "2.12.0", "2.11.9", "failed", RUN)
    ci.run("report", "2.12.0", "2.11.9", "passed", RUN)
    ci.run("report", "2.12.0", "2.11.9", "passed", RUN)
    [issue] = ci.world["issues"]
    assert len(issue["comments"]) == 1
    assert "passed" in issue["comments"][0]


def test_report_leaves_a_closed_issue_for_the_release_closed(ci):
    ci.world["issues"].append(
        {"number": 1, "title": TITLE, "state": "CLOSED", "body": "x", "comments": []}
    )
    result = ci.run("report", "2.12.0", "2.11.9", "failed", RUN)
    assert result.returncode == 0, result.stderr
    assert "#1 is closed" in result.stdout
    assert len(ci.world["issues"]) == 1
    assert ci.world["issues"][0]["comments"] == []


def test_report_matches_the_title_exactly(ci):
    ci.world["issues"].append(
        {
            "number": 1,
            "title": "Caddy 2.12.0 is out; bin/caddy pins 2.11.8",
            "state": "OPEN",
            "body": "x",
            "comments": [],
        }
    )
    ci.run("report", "2.12.0", "2.11.9", "passed", RUN)
    assert [i["title"] for i in ci.world["issues"]][-1] == TITLE
    assert len(ci.world["issues"]) == 2
