"""A fake systemd-run for the tests of the ops/remote/ scripts that run the binary in a
transient unit: it notes each call's arguments, a tab between each, as one line of
its own in a log, sets each `-p Environment=` it was given, and runs the command
after `--` as the caller. With SYSTEMD_RUN_EXIT set, it runs nothing and exits with
that status, as systemd-run does when the unit fails before or around the command.
"""

import itertools
import stat
from pathlib import Path

SCRIPT = """#!/bin/bash
(IFS=$'\\t'; printf 'systemd-run\\t%s\\n' "$*") >> "{log}"
[[ -n "${{SYSTEMD_RUN_EXIT:-}}" ]] && exit "$SYSTEMD_RUN_EXIT"
while [[ "$1" != -- ]]; do
  if [[ "$1" == -p && "$2" == Environment=* ]]; then export "${{2#Environment=}}"; fi
  shift
done
shift
exec "$@"
"""


def install(fakes: Path, log: Path) -> Path:
    """Writes the fake to FAKES/systemd-run, noting its calls in LOG."""
    path = fakes / "systemd-run"
    path.write_text(SCRIPT.format(log=log))
    path.chmod(path.stat().st_mode | stat.S_IXUSR)
    return path


def calls(log: Path) -> list[list[str]]:
    """Each systemd-run call noted in LOG, as its arguments."""
    return [
        line.split("\t")[1:]
        for line in log.read_text().splitlines()
        if line.startswith("systemd-run\t")
    ]


def properties(call: list[str]) -> list[str]:
    """The `-p` properties of CALL, in order, up to its `--`."""
    options = call[: call.index("--")]
    return [value for flag, value in itertools.pairwise(options) if flag == "-p"]


def command(call: list[str]) -> list[str]:
    """What CALL runs: its words after `--`."""
    return call[call.index("--") + 1 :]
