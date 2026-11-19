"""No script in bin/ or ops/ that sets `pipefail` pipes into a reader that stops
early. `cmd | grep -q PATTERN` and `cmd | head -1` fail there whenever the reader
exits before `cmd` is done writing: `cmd` dies of SIGPIPE, and the pipeline reports
failure though the reader found what it wanted. On 2026-11-09 that refused a landing:
`ops/protect-skill-tags` read a secret that exists as missing in 11 of 400 runs (#39).

Read the output whole instead: `out=$(cmd)`, then `grep -q PATTERN <<<"$out"`; for the
first line, `cmd | sed -n 1p`, which reads to the end; for the first characters of a
variable, `${var:0:300}`.

This reads lines, not shell: it joins a line ending in `|` or `\\` with the next, and
skips comment lines. It knows the plain spellings only, `| head` and `| grep` with its
options before the pattern, so it misses a reader inside a function the pipe calls,
one behind `command` or `sudo`, `egrep -q`, `grep PATTERN -q`, `sed 1q`, and `read`."""

import re
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]

# After a pipe, not `||`: head, or grep with -q or -m among its options.
EARLY_READER = re.compile(
    r"""(?<!\|)\|\s*(?:
        head\b
      | grep\s+(?:-\S+\s+)*-[a-zA-Z]*[qm]
      | grep\s+(?:-\S+\s+)*--(?:quiet|silent|max-count)
    )""",
    re.VERBOSE,
)


def early_readers(source: str) -> list[int]:
    """The line of each pipe in `source` into a reader that stops early."""
    found = []
    joined, start = "", 0
    for number, line in enumerate(source.splitlines(), 1):
        if line.lstrip().startswith("#"):
            continue
        if not joined:
            start = number
        joined += line
        if line.rstrip().endswith(("|", "\\")):
            joined = joined.rstrip().removesuffix("\\")
            continue
        if EARLY_READER.search(joined):
            found.append(start)
        joined = ""
    return found


def pipefail_scripts() -> list[Path]:
    """The shell scripts in bin/, ops/ and ops/remote/ that set `pipefail`, and the
    parts ops/provision-vps sources after setting it."""
    scripts = sorted((REPO / "ops/provision-vps.d").glob("*.sh"))
    for folder in (REPO / "bin", REPO / "ops", REPO / "ops/remote"):
        for path in sorted(folder.iterdir()):
            if not path.is_file():
                continue
            try:
                text = path.read_text()
            except UnicodeDecodeError:
                continue
            first = text.split("\n", 1)[0]
            shell = path.suffix == ".sh" or (first.startswith("#!") and "sh" in first)
            if shell and re.search(r"^\s*set .*pipefail", text, re.MULTILINE):
                scripts.append(path)
    return scripts


def test_early_readers_finds_head_and_quiet_grep_after_a_pipe_but_not_whole_reads():
    source = (
        "set -euo pipefail\n"
        'gh secret list | grep -qx "$1"\n'
        "# gh secret list | grep -q commented\n"
        "first=$(sed -n 's/^tag //p' \"$file\" | head -1)\n"
        "swapon --show=NAME |\n"
        "  grep -x -m 1 /swapfile\n"
        "printf '%s' \"$BODY\" | head -c 300\n"
        'names=$(gh secret list) && grep -qx "$1" <<<"$names"\n'
        "first=$(sed -n 's/^tag //p' \"$file\" | sed -n 1p)\n"
        "head -n 1 <<<\"$heads\" | awk '{print $2}'\n"
        'gh run list | grep --quiet "$sha"\n'
        'cat "$log" | grep -c failed\n'
        'grep -m 1 "^v" <<<"$said" || head -n 1 <<<"$said"\n'
    )
    assert early_readers(source) == [2, 4, 5, 7, 11]


def test_the_scripts_checked_include_the_ones_that_misfired():
    names = {str(path.relative_to(REPO)) for path in pipefail_scripts()}
    assert {
        "ops/protect-skill-tags",
        "ops/provision-vps",
        "ops/provision-vps.d/staging.sh",
        "ops/smoke-staging",
        "ops/remote/base.sh",
    } <= names


def test_no_pipefail_script_pipes_into_a_reader_that_stops_early():
    found = [
        f"{path.relative_to(REPO)}:{line}"
        for path in pipefail_scripts()
        for line in early_readers(path.read_text())
    ]
    assert not found, (
        "under pipefail, a reader that exits early fails its pipeline; read the output "
        "whole first (this file's docstring says how): " + ", ".join(found)
    )
