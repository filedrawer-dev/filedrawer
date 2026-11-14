"""ops/usability-run report: a few lines per run, read from the run folder that
drive-agent saved, with keys and tokens blanked.

Each test builds a run folder in tmp_path as drive-agent lays one out: run.json,
scenario.json, turns.md, transcript-*.jsonl and files/.
"""

import json
import os
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "ops" / "usability-run"
KEY = "0123456789abcdef" * 4
TOKEN = "fdt_" + "q7" * 26


def bash(ts, command):
    return {
        "type": "assistant",
        "timestamp": ts,
        "message": {
            "content": [
                {"type": "text", "text": "Working."},
                {"type": "tool_use", "name": "Bash", "input": {"command": command}},
            ]
        },
    }


def tool(ts, name, **given):
    return {
        "type": "assistant",
        "timestamp": ts,
        "message": {"content": [{"type": "tool_use", "name": name, "input": given}]},
    }


def write_jsonl(path, records):
    path.write_text("\n".join(map(json.dumps, records)) + "\n")


def make_run(tmp_path, scenario):
    usability = tmp_path / "usability"
    (usability / "scenarios").mkdir(parents=True)
    (usability / "scenarios" / "s2-search.json").write_text(json.dumps(scenario))
    run = usability / "runs" / "2026-11-13" / "jason5-claude"
    run.mkdir(parents=True)
    (run / "scenario.json").write_text(json.dumps(scenario))
    (run / "run.json").write_text(
        json.dumps(
            {"harness": "claude", "version": "2.1.325 (Claude Code)", "outcome": "done"}
        )
    )
    (run / "turns.md").write_text(
        '## Turn 1 (asks in a dialog, 15:24:32)\n\nScripted: "install"\n\n'
        "Sent: install\n\nThe agent asks in a dialog:\nInstall them?\n\n"
        '## Turn 2 (ended, 15:24:46)\n\nScripted: "yes"\n\n'
        "Answered in the question dialog, to:\nInstall them?\n\nSent: yes\n\n"
        "## Turn 3: new session\n\n"
        "## Turn 4: skipped, condition_met: False\n\nScripted: {}\n\n"
    )
    skills = run / "files" / ".agents" / "skills"
    for name, version in (("filedrawer-search", "7"), ("filedrawer-submit", "9")):
        (skills / name).mkdir(parents=True)
        (skills / name / "SKILL.md").write_text(
            f'---\nname: {name}\nmetadata:\n  version: "{version}"\n---\n\n# Skill\n'
        )
    config = run / "files" / ".config" / "filedrawer"
    config.mkdir(parents=True)
    (config / "upgrade").write_text("auto\n")
    return usability, run


def report(usability, *runs):
    env = {k: v for k, v in os.environ.items() if not k.startswith("FILEDRAWER_")}
    env["FILEDRAWER_USABILITY_DIR"] = str(usability)
    return subprocess.run(
        [str(SCRIPT), "report", *map(str, runs)],
        env=env,
        capture_output=True,
        text=True,
        check=False,
    )


def test_report_sums_up_a_run_in_a_few_lines_without_its_secrets(tmp_path):
    usability, run = make_run(tmp_path, {"gated": ["posting"], "turns": ["install"]})
    site = "https://staging.filedrawer.dev"
    # The file named first holds the session that ran second.
    write_jsonl(
        run / "transcript-a.jsonl",
        [
            {"type": "mode", "mode": "normal"},
            bash(
                "2026-11-13T23:25:00Z",
                "curl -sS -H @$HOME/.config/filedrawer/filedrawer.dev.header "
                "'https://filedrawer.dev/search?q=probe'",
            ),
            bash(
                "2026-11-13T23:25:01Z",
                f"curl -i -X DELETE -H 'Authorization: Bearer {TOKEN}' {site}/e/abc",
            ),
            tool("2026-11-13T23:25:02Z", "Read", file_path="/home/x/NOTES.md"),
        ],
    )
    write_jsonl(
        run / "transcript-b.jsonl",
        [
            bash("2026-11-13T23:24:00Z", f"curl -fsSL '{site}/skill.md?key={KEY}'"),
            tool("2026-11-13T23:24:01Z", "WebFetch", url=f"{site}/skill.md"),
            bash(
                "2026-11-13T23:24:02Z",
                f"K={KEY}; curl --create-dirs -o ~/.config/filedrawer/install.sh "
                f"{site}/install.sh && sh ~/.config/filedrawer/install.sh auto",
            ),
            bash(
                "2026-11-13T23:24:03Z",
                f"curl -i -H 'X-Staging-Key: {KEY}' -H 'Content-Type: text/markdown' "
                f"--data-binary @entry.md {site}/entries",
            ),
            bash("2026-11-13T23:24:04Z", "ls ~/code"),
        ],
    )
    done = report(usability, run)
    assert done.returncode == 0, done.stderr
    out = done.stdout
    for secret in (KEY, TOKEN[4:]):
        assert secret not in out
    lines = out.splitlines()
    assert len(lines) <= 15
    assert lines[0] == "jason5-claude: s2-search, claude 2.1.325, outcome done"
    assert "  skill.md first fetched by: shell curl (Bash)" in lines
    assert "  turns answered in a dialog: 2" in lines
    assert "  skills: filedrawer-search 7, filedrawer-submit 9" in lines
    assert "  upgrade file: auto" in lines
    assert "  requests to production filedrawer.dev: 1" in lines
    assert (
        "  site writes: POST https://staging.filedrawer.dev/entries, "
        "DELETE https://staging.filedrawer.dev/e/abc"
    ) in lines
    assert "  site and installer commands: 6" in lines
    commands = [line for line in lines if line.startswith("    [")]
    # In the order the sessions ran, one line each, without ls or Read.
    assert [c.split()[0] for c in commands] == ["[Bash]", "[WebFetch]"] + ["[Bash]"] * 4
    assert "install.sh auto" in commands[2]
    assert "filedrawer.dev/search" in commands[4]


def test_report_says_what_a_run_never_did(tmp_path):
    usability, run = make_run(tmp_path, {"gated": [], "turns": ["other"]})
    (run / "scenario.json").write_text(json.dumps({"gated": [], "turns": ["edited"]}))
    (run / "turns.md").write_text("## Turn 1 (ended, 15:24:32)\n\nSent: hi\n\n")
    for path in (run / "files").rglob("*"):
        if path.is_file():
            path.unlink()
    (run / "run.json").write_text(
        json.dumps({"harness": "codex", "version": "codex-cli 0.164.0"})
    )
    write_jsonl(
        run / "transcript-a.jsonl",
        [
            # A version probe reads only the headers; a sed names production but
            # sends nothing.
            bash(
                "2026-11-13T23:23:58Z",
                "curl -sSI https://staging.filedrawer.dev/skill.md | grep -i x-file",
            ),
            bash(
                "2026-11-13T23:23:59Z",
                "sed -i 's#https://filedrawer.dev#https://staging.filedrawer.dev#' f",
            ),
            tool(
                "2026-11-13T23:24:00Z",
                "WebFetch",
                url="https://staging.filedrawer.dev/skill.md",
            ),
        ],
    )
    out = report(usability, run).stdout.splitlines()
    assert out[0] == "jason5-claude: ?, codex 0.164.0, outcome None"
    assert "  skill.md first fetched by: fetch tool WebFetch" in out
    assert "  turns answered in a dialog: none" in out
    assert "  skills: none" in out
    assert "  upgrade file: not saved" in out
    assert "  requests to production filedrawer.dev: none" in out
    assert "  site writes: none" in out


def test_report_of_a_folder_with_no_run_is_refused(tmp_path):
    done = report(tmp_path, tmp_path)
    assert done.returncode == 2 and "run.json" in done.stderr
