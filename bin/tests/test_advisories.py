"""Tests for bin/advisories, which .github/workflows/advisories.yml runs, with a fake
cargo-audit (a script that prints a report and exits as told, through
$FILEDRAWER_CARGO_AUDIT) and fake_gh_issues.py as gh, which lists every issue whoever
opened it, so these tests show what the script itself counts as its own.

The advisories are made up; the report's shape is cargo-audit 0.23.0's `--json`.
"""

import json
import subprocess
from pathlib import Path
from types import SimpleNamespace

import pytest

REPO = Path(__file__).resolve().parents[2]
SCRIPT = REPO / "bin/advisories"
FAKE_GH = Path(__file__).with_name("fake_gh_issues.py")
RUN = "https://github.com/filedrawer-dev/filedrawer/actions/runs/7"
BOT = "github-actions[bot]"
TITLE = "Cargo.lock holds crates with security advisories"
MAILER = "RUSTSEC-2026-9001"
PARSER = "RUSTSEC-2026-9002"
SQLITE = "RUSTSEC-2025-9003"


def vulnerability(
    advisory: str, crate: str, version: str, patched: list[str] | None = None
) -> dict:
    return {
        "advisory": {
            "id": advisory,
            "package": crate,
            "title": f"A title that mentions @someone and {crate}",
            "description": "See https://example.com/more.",
            "url": f"https://example.com/{advisory}",
        },
        "versions": {
            "patched": [">=9.9.9"] if patched is None else patched,
            "unaffected": [],
        },
        "package": {"name": crate, "version": version},
    }


def audit_report(*vulnerabilities: dict) -> dict:
    return {
        "database": {"advisory-count": 1555},
        "lockfile": {"dependency-count": 298},
        "vulnerabilities": {
            "found": bool(vulnerabilities),
            "count": len(vulnerabilities),
            "list": list(vulnerabilities),
        },
        "warnings": {"unmaintained": [vulnerability(SQLITE, "old-toml", "0.1.0")]},
    }


def found(advisory: str, crate: str, version: str, patched: str = ">=9.9.9") -> dict:
    """An advisory as `check` hands it to `report`."""
    return {
        "id": advisory,
        "crate": crate,
        "version": version,
        "patched": [patched] if patched else [],
    }


MAILER_FOUND = found(MAILER, "mailer", "0.11.23")
PARSER_FOUND = found(PARSER, "parser", "1.2.3")


@pytest.fixture
def ci(tmp_path):
    """A checkout with a fake cargo-audit, and a GitHub with no issues."""
    fakes = tmp_path / "fakes"
    fakes.mkdir()
    world_path = tmp_path / "world.json"
    audit_out = tmp_path / "audit.out"
    audit_args = tmp_path / "audit.args"
    audit = fakes / "cargo-audit"
    audit.write_text(
        f'#!/bin/sh\necho "$*" > {audit_args}\ncat {audit_out}\n'
        'echo "fake cargo-audit: on stderr" >&2\nexit "${FAKE_AUDIT_EXIT:-0}"\n'
    )
    gh = fakes / "gh"
    gh.write_text(
        f'#!/bin/sh\nexec python3 {FAKE_GH} {world_path} {tmp_path / "gh.log"} "$@"\n'
    )
    audit.chmod(0o755)
    gh.chmod(0o755)
    state = SimpleNamespace(issues=[], audit_args=audit_args)

    def run(*args: str, audit_exit: int = 0) -> subprocess.CompletedProcess:
        world_path.write_text(json.dumps({"api": {}, "issues": state.issues}))
        result = subprocess.run(
            [str(SCRIPT), *args],
            cwd=tmp_path,
            env={
                "PATH": f"{fakes}:/usr/bin:/bin",
                "GH_REPO": "filedrawer-dev/filedrawer",
                "FILEDRAWER_CARGO_AUDIT": str(audit),
                "FAKE_AUDIT_EXIT": str(audit_exit),
            },
            stdin=subprocess.DEVNULL,
            capture_output=True,
            text=True,
            check=False,
        )
        state.issues = json.loads(world_path.read_text())["issues"]
        return result

    def check(report: dict | str, audit_exit: int = 0) -> subprocess.CompletedProcess:
        text = report if isinstance(report, str) else json.dumps(report)
        audit_out.write_text(text)
        return run("check", audit_exit=audit_exit)

    def report(*advisories: dict) -> subprocess.CompletedProcess:
        return run("report", json.dumps(list(advisories)), RUN)

    state.check, state.report, state.run = check, report, run
    return state


def an_issue(number: int, **fields) -> dict:
    """An issue as the fake gh keeps it, the workflow's own unless `author` says."""
    return {
        "number": number,
        "title": TITLE,
        "state": "open",
        "body": "x",
        "author": BOT,
        "comments": [],
    } | fields


def advisories(result: subprocess.CompletedProcess) -> list[dict]:
    [line] = result.stdout.splitlines()
    name, _, value = line.partition("=")
    assert name == "advisories"
    return json.loads(value)


# check


def test_check_prints_each_advisory_that_applies_and_says_so_in_the_log(ci):
    result = ci.check(
        audit_report(
            vulnerability(PARSER, "parser", "1.2.3"),
            vulnerability(MAILER, "mailer", "0.11.23", [">=0.11.24"]),
        ),
        audit_exit=1,
    )
    assert result.returncode == 0, result.stderr
    assert advisories(result) == [
        found(MAILER, "mailer", "0.11.23", ">=0.11.24"),
        PARSER_FOUND,
    ]
    # The run's log shows what was found; $GITHUB_OUTPUT gets only stdout.
    assert "2 advisories apply" in result.stderr
    assert MAILER in result.stderr and PARSER in result.stderr
    assert "1555" in result.stderr


def test_check_asks_cargo_audit_for_json_and_no_yanked_crates(ci):
    ci.check(audit_report())
    assert ci.audit_args.read_text().split() == ["audit", "--json", "--no-yanked"]


def test_check_with_no_advisory_prints_an_empty_list(ci):
    result = ci.check(audit_report())
    assert result.returncode == 0, result.stderr
    assert advisories(result) == []
    assert "No advisory applies" in result.stderr


def test_check_reports_vulnerabilities_not_cargo_audits_warnings(ci):
    assert SQLITE not in ci.check(audit_report()).stdout


def test_check_passes_on_no_text_an_advisory_wrote(ci):
    result = ci.check(audit_report(vulnerability(MAILER, "mailer", "0.11.23")), 1)
    assert "@someone" not in result.stdout + result.stderr
    assert "example.com" not in result.stdout + result.stderr


@pytest.mark.parametrize(
    "report",
    [
        "",
        "error: error loading advisory database",
        "[]",
        json.dumps({"vulnerabilities": {"found": False, "count": 0, "list": []}}),
        # A database with nothing in it finds nothing.
        json.dumps(audit_report() | {"database": {"advisory-count": 0}}),
        json.dumps(audit_report() | {"lockfile": {"dependency-count": 0}}),
        # The count and the list disagree.
        json.dumps(
            audit_report()
            | {"vulnerabilities": {"found": True, "count": 2, "list": []}}
        ),
        json.dumps(
            audit_report()
            | {"vulnerabilities": {"found": False, "count": 0, "list": [{}]}}
        ),
    ],
)
def test_check_fails_when_cargo_audit_gave_no_report_it_can_trust(ci, report):
    """cargo-audit exits 1 for advisories found and for a database it couldn't
    load alike, so the report decides, not the exit status."""
    result = ci.check(report, audit_exit=1)
    assert result.returncode == 1
    assert result.stdout == ""
    assert result.stderr.startswith("advisories: ")
    assert "fake cargo-audit: on stderr" in result.stderr


@pytest.mark.parametrize(
    ("advisory", "crate", "version"),
    [
        ("RUSTSEC-2026-9001 @someone", "mailer", "0.11.23"),
        ("CVE-2026-1", "mailer", "0.11.23"),
        (MAILER, "mailer](https://example.com)", "0.11.23"),
        (MAILER, "mailer", "0.11.23 and more"),
        (MAILER + "\n", "mailer", "0.11.23"),
    ],
)
def test_check_fails_on_an_advisory_whose_fields_arent_what_they_should_be(
    ci, advisory, crate, version
):
    result = ci.check(audit_report(vulnerability(advisory, crate, version)), 1)
    assert result.returncode == 1
    assert result.stdout == ""


@pytest.mark.parametrize(
    "patched", [[">=1.0.0 @someone"], [">=1.0.0", "[x](y)"], ["x" * 80], [7]]
)
def test_check_keeps_an_advisory_whose_patched_versions_it_cant_read(ci, patched):
    result = ci.check(
        audit_report(vulnerability(MAILER, "mailer", "0.11.23", patched)), 1
    )
    assert result.returncode == 0, result.stderr
    assert advisories(result) == [found(MAILER, "mailer", "0.11.23", "")]


def test_check_takes_ranges_and_prereleases_as_patched_versions(ci):
    patched = [">=0.5.3, <0.6.0", "^1.2", ">=2.0.0-rc.1+build5"]
    result = ci.check(
        audit_report(vulnerability(MAILER, "mailer", "0.11.23-beta.1", patched)), 1
    )
    assert advisories(result) == [
        {"id": MAILER, "crate": "mailer", "version": "0.11.23-beta.1"}
        | {"patched": patched}
    ]


# report


def test_report_opens_an_issue_listing_each_advisory_and_the_run(ci):
    result = ci.report(MAILER_FOUND, found(PARSER, "parser", "1.2.3", ""))
    assert result.returncode == 0, result.stderr
    [issue] = ci.issues
    assert (issue["title"], issue["state"]) == (TITLE, "open")
    body = issue["body"]
    assert (
        f"- [{MAILER}](https://rustsec.org/advisories/{MAILER}): `mailer` `0.11.23`, "
        "patched in `>=9.9.9`\n"
    ) in body
    assert (
        f"- [{PARSER}](https://rustsec.org/advisories/{PARSER}): `parser` `1.2.3`, "
        "no patched version\n"
    ) in body
    assert RUN in body
    assert ".cargo/audit.toml" in body


def test_report_with_the_same_advisories_adds_nothing(ci):
    ci.report(MAILER_FOUND, PARSER_FOUND)
    before = json.dumps(ci.issues)
    result = ci.report(PARSER_FOUND, MAILER_FOUND)
    assert result.returncode == 0, result.stderr
    assert json.dumps(ci.issues) == before


def test_report_rewrites_the_list_and_comments_when_the_advisories_change(ci):
    ci.report(MAILER_FOUND)
    ci.report(PARSER_FOUND)
    [issue] = ci.issues
    assert issue["state"] == "open"
    assert PARSER in issue["body"] and MAILER not in issue["body"]
    [comment] = issue["comments"]
    assert PARSER in comment["body"] and MAILER not in comment["body"]
    assert RUN in comment["body"]


def test_report_closes_its_issue_once_no_advisory_applies(ci):
    ci.report(MAILER_FOUND)
    result = ci.report()
    assert result.returncode == 0, result.stderr
    [issue] = ci.issues
    assert issue["state"] == "closed"
    assert MAILER not in issue["body"]
    [comment] = issue["comments"]
    assert "No advisory applies" in comment["body"] and RUN in comment["body"]


def test_report_closes_an_open_issue_that_lists_nothing(ci):
    """As when the close failed after the list was emptied, or someone opened the
    issue again."""
    ci.issues = [an_issue(1, body="No advisory applies.")]
    result = ci.report()
    assert result.returncode == 0, result.stderr
    [issue] = ci.issues
    assert issue["state"] == "closed"


def test_report_with_no_advisory_and_no_issue_does_nothing(ci):
    result = ci.report()
    assert result.returncode == 0, result.stderr
    assert ci.issues == []


def test_report_opens_a_new_issue_after_the_one_it_closed(ci):
    ci.report(MAILER_FOUND)
    ci.report()
    ci.report(MAILER_FOUND)
    closed, opened = ci.issues
    assert (closed["state"], opened["state"]) == ("closed", "open")
    assert MAILER in opened["body"]


def test_an_issue_closed_by_hand_accepts_the_advisories_it_lists(ci):
    """Closing the issue says its advisories are known; the same ones, or fewer,
    open nothing again."""
    ci.report(MAILER_FOUND, PARSER_FOUND)
    ci.issues[0]["state"] = "closed"
    before = json.dumps(ci.issues)
    ci.report(MAILER_FOUND, PARSER_FOUND)
    ci.report(MAILER_FOUND)
    ci.report()
    assert json.dumps(ci.issues) == before


def test_an_advisory_a_closed_issue_doesnt_list_opens_a_new_one_listing_all(ci):
    ci.report(MAILER_FOUND)
    ci.issues[0]["state"] = "closed"
    ci.report(MAILER_FOUND, PARSER_FOUND)
    closed, opened = ci.issues
    assert (closed["state"], closed["comments"]) == ("closed", [])
    assert opened["state"] == "open"
    assert MAILER in opened["body"] and PARSER in opened["body"]


def test_report_goes_by_its_newest_issue(ci):
    ci.issues = [
        an_issue(1, state="closed", body=f"- [{MAILER}](x)"),
        an_issue(2, state="closed", body="No advisory applies."),
    ]
    ci.report(MAILER_FOUND)
    assert [i["state"] for i in ci.issues] == ["closed", "closed", "open"]


@pytest.mark.parametrize(
    "theirs",
    [
        {"author": "someone"},
        {"author": "someone", "state": "closed"},
        # A user can't take the workflow's name, but another app could have a like one.
        {"author": "github-actions-x[bot]", "state": "closed"},
        {"title": TITLE + " "},
        {"pull_request": {"url": "x"}, "state": "closed"},
    ],
)
def test_report_counts_only_an_issue_the_workflow_opened(ci, theirs):
    """Anyone may open an issue with the report's title, list the advisories in
    it, and close it."""
    theirs = an_issue(1, body=f"- [{MAILER}](x)") | theirs
    ci.issues = [theirs]
    result = ci.report(MAILER_FOUND)
    assert result.returncode == 0, result.stderr
    kept, ours = ci.issues
    assert kept == theirs
    assert (ours["author"], ours["title"], ours["state"]) == (BOT, TITLE, "open")
    assert MAILER in ours["body"]


@pytest.mark.parametrize(
    "advisories",
    [
        "",
        "not json",
        "{}",
        json.dumps([{"id": MAILER}]),
        json.dumps([found(MAILER + " @someone", "mailer", "0.11.23")]),
        json.dumps([found(MAILER, "mailer @someone", "0.11.23")]),
        json.dumps([found(MAILER, "mailer", "0.11.23](x)")]),
    ],
)
def test_report_refuses_advisories_check_wouldnt_have_printed(ci, advisories):
    """`report` holds the token that writes issues, so it reads its argument as
    strictly as `check` reads cargo-audit: an empty one isn't "none found"."""
    ci.issues = [an_issue(1, body=f"- [{MAILER}](x)")]
    before = json.dumps(ci.issues)
    result = ci.run("report", advisories, RUN)
    assert result.returncode == 1
    assert result.stderr.startswith("advisories: ")
    assert json.dumps(ci.issues) == before


def test_report_leaves_out_patched_versions_it_cant_read(ci):
    ci.report(found(MAILER, "mailer", "0.11.23", ">=1 @someone"))
    [issue] = ci.issues
    assert "`mailer` `0.11.23`, no patched version\n" in issue["body"]
    assert "@someone" not in issue["body"]


def test_help_and_a_wrong_call(ci):
    assert "bin/advisories check" in ci.run("-h").stdout
    assert ci.run("report").returncode == 2
