"""Tests for bin/caddy-latest, which .github/workflows/caddy-latest.yml runs (#100),
with fakes of gh and curl first on PATH: gh answers from a JSON world and notes the
issues it opens and the comments it adds; curl serves files from a folder.

The fake gh, fake_gh_issues.py, lists every issue and comment whatever `creator` the
call names, so these tests show what the script itself counts as its own.
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
BOT = "github-actions[bot]"
TITLE = "Caddy 2.12.0 is out; bin/caddy pins 2.11.9"
FAKE_GH = Path(__file__).with_name("fake_gh_issues.py")


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
    executable(
        fakes / "gh", f'#!/bin/sh\nexec python3 {FAKE_GH} {world_path} {log} "$@"\n'
    )
    executable(fakes / "curl", FAKE_CURL.format(files=files))
    state = SimpleNamespace(world=world, files=files, data=data, asset=asset)

    def run(*args: str) -> subprocess.CompletedProcess:
        world_path.write_text(json.dumps(state.world))
        result = subprocess.run(
            [str(SCRIPT), *args],
            cwd=checkout,
            env={
                "PATH": f"{fakes}:/usr/bin:/bin",
                "GH_REPO": "filedrawer-dev/filedrawer",
            },
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
    # The run's log shows the versions; $GITHUB_OUTPUT gets only stdout.
    assert "Caddy's latest release is 2.12.0; bin/caddy pins 2.11.9" in result.stderr
    assert "tests run against it" in result.stderr


@pytest.mark.parametrize("tag", ["v2.11.9", "v2.11.8", "v2.9.10"])
def test_versions_needs_nothing_for_a_release_at_or_behind_the_pin(ci, tag):
    ci.world["api"]["repos/caddyserver/caddy/releases/latest"] = {"tag_name": tag}
    result = ci.run("versions")
    assert result.returncode == 0, result.stderr
    assert outputs(result)["needed"] == "false"
    assert f"latest release is {tag[1:]}" in result.stderr
    assert "nothing to test" in result.stderr


@pytest.mark.parametrize("tag", ["2.12.0", "v2.12.0-beta.1", "", "v2.12", "v2.12.0\n"])
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


def test_fetch_takes_only_a_version(ci):
    result = ci.run("fetch", "2.12.0\n", str(ci.out))
    assert result.returncode == 2
    assert not ci.out.exists()


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
    [comment] = issue["comments"]
    assert "passed" in comment["body"]


def an_issue(number: int, title: str = TITLE, **fields) -> dict:
    """An issue as the fake gh keeps it, the workflow's own unless `author` says."""
    return {
        "number": number,
        "title": title,
        "state": "open",
        "body": "x",
        "author": BOT,
        "comments": [],
    } | fields


def test_report_leaves_a_closed_issue_for_the_release_closed(ci):
    ci.world["issues"].append(an_issue(1, state="closed"))
    result = ci.run("report", "2.12.0", "2.11.9", "failed", RUN)
    assert result.returncode == 0, result.stderr
    assert "#1 is closed" in result.stdout
    assert len(ci.world["issues"]) == 1
    assert ci.world["issues"][0]["comments"] == []


def test_report_matches_the_title_exactly(ci):
    ci.world["issues"].append(an_issue(1, "Caddy 2.12.0 is out; bin/caddy pins 2.11.8"))
    ci.run("report", "2.12.0", "2.11.9", "passed", RUN)
    assert [i["title"] for i in ci.world["issues"]][-1] == TITLE
    assert len(ci.world["issues"]) == 2


@pytest.mark.parametrize(
    "theirs",
    [
        {"state": "closed"},
        {"body": f"The real-Caddy tests failed against Caddy 2.12.0 ({RUN})."},
        # A user can't take the workflow's name, but another app could have a like one.
        {"author": "github-actions-x[bot]", "state": "closed"},
    ],
)
def test_report_counts_only_an_issue_the_workflow_opened(ci, theirs):
    """Anyone may open an issue with the title a release will get, and close it."""
    ci.world["issues"].append(an_issue(1, **{"author": "someone", **theirs}))
    result = ci.run("report", "2.12.0", "2.11.9", "failed", RUN)
    assert result.returncode == 0, result.stderr
    theirs, ours = ci.world["issues"]
    assert theirs["comments"] == []
    assert (ours["author"], ours["title"], ours["state"]) == (BOT, TITLE, "open")
    assert "failed" in ours["body"]


def test_report_takes_no_pull_request_for_its_issue(ci):
    ci.world["issues"].append(an_issue(1, state="closed", pull_request={"url": "x"}))
    ci.run("report", "2.12.0", "2.11.9", "failed", RUN)
    assert len(ci.world["issues"]) == 2


@pytest.mark.parametrize(("said", "comments"), [("passed", 1), ("failed", 0)])
def test_report_reads_the_last_verdict_only_from_its_own_comments(ci, said, comments):
    """A comment anyone adds in the report's words changes nothing: the verdict
    that differs from the workflow's last is still said, and the same one isn't."""
    ci.run("report", "2.12.0", "2.11.9", "failed", RUN)
    [ours] = ci.world["issues"]
    ours["comments"].append(
        {
            "author": "someone",
            "body": f"The real-Caddy tests {said} against Caddy 2.12.0 ({RUN}).",
        }
    )
    ci.run("report", "2.12.0", "2.11.9", said, RUN)
    [ours] = ci.world["issues"]
    assert [c["author"] for c in ours["comments"]].count(BOT) == comments
