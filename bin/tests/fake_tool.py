"""One fake of ssh, gh, getent, curl or ssh-keygen, for conftest's FakeHost.

Usage: fake_tool.py ROOT TOOL ARG...

Notes the call in ROOT/log, keeps an ssh call's stdin in ROOT/sent/<n>, and answers
from the latest rule in ROOT/rules.json that matches.
"""

import fnmatch
import json
import subprocess
import sys
from pathlib import Path

# The ssh options that take a value, from ssh(1).
SSH_VALUE_OPTIONS = set("BbcDEeFIiJLlmOoPpQRSWw")


def remote_line(args: list[str]) -> str:
    """The remote command line of an ssh call: its words after the host, joined with
    spaces, as ssh hands them to the server's shell."""
    i = 0
    while i < len(args) and args[i].startswith("-"):
        option = args[i]
        i += 1
        if len(option) == 2 and option[1] in SSH_VALUE_OPTIONS:
            i += 1
    return " ".join(args[i + 1 :])


def main(root: Path, tool: str, args: list[str]) -> int:
    line = remote_line(args) if tool == "ssh" else " ".join(args)
    with (root / "log").open("a") as log:
        log.write(f"{tool} {line}\n")
    stdin = ""
    if tool == "ssh":
        stdin = sys.stdin.read()
        sent = root / "sent"
        (sent / str(len(list(sent.iterdir())) + 1)).write_text(stdin)
    for rule in reversed(json.loads((root / "rules.json").read_text())):
        if (
            rule["tool"] == tool
            and fnmatch.fnmatchcase(line, rule["pattern"])
            and (rule["stdin"] is None or rule["stdin"] in stdin)
        ):
            if rule["run_locally"]:
                return subprocess.run(
                    ["bash", "-c", line], input=stdin, text=True, check=False
                ).returncode
            sys.stdout.write(rule["stdout"])
            return rule["status"]
    if tool == "gh":
        return gh(root / "secrets", args)
    return 0


def gh(secrets: Path, args: list[str]) -> int:
    match args:
        case ["secret", "list", *_]:
            for name in sorted(path.name for path in secrets.iterdir()):
                print(name)
        case ["secret", "set", name, "--env", "staging"]:
            (secrets / name).write_text(sys.stdin.read())
        case ["secret", "set", *_]:
            sys.stdin.read()
    return 0


if __name__ == "__main__":
    sys.exit(main(Path(sys.argv[1]), sys.argv[2], sys.argv[3:]))
