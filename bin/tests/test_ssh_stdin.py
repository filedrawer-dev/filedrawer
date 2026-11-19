"""Every `ssh` call in the scripts the owner runs (ops/, but not ops/remote/, which
runs on the server) sets its stdin: a `<` redirect or here-string, `-n`, or a pipe
into it; or it is a terminal session on purpose, with `-t`. ssh forwards whatever
stdin it has to the server, so a call with none reads the owner's terminal: in
ops/provision-vps the host-key fetch once ate the answers meant for the prompts
after it.

This reads lines, not shell: it joins a line ending in `|` or `\\` with the next,
skips comment lines, takes `ssh` only where a command starts (so not in a message
or prose), and takes a `<` anywhere after it in the statement, a quoted one
included, as a redirect. It misses an ssh behind a prefix command (`timeout`,
`command`, `env`) and one whose host is a bare alias (`ssh vps uptime`)."""

import re
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]

# ssh where a command starts (the line's start, or after a pipe, `;`, `&&`, `$(`,
# `{`, `!`, or a keyword such as `if` or `exec`), followed by what a call passes
# first: an option, a quoted or variable host, or user@host.
SSH = re.compile(
    r"(?:^|[;&|({!]|\$\(|\b(?:if|then|else|do|exec|while|until))\s*"
    r"ssh(?=\s+(?:-|[\"'$]|\S*@))"
)
REDIRECT = re.compile(r"<(?!\()")


def unset_stdin(source: str) -> list[int]:
    """The line of each `ssh` call in `source` whose stdin isn't set."""
    found = []
    joined, start = "", 0
    for number, line in enumerate(source.splitlines(), 1):
        if line.lstrip().startswith("#"):
            continue
        if not joined:
            start = number
        joined += line + " "
        if line.rstrip().endswith(("|", "\\")):
            joined = joined.rstrip().removesuffix("\\")
            continue
        for call in SSH.finditer(joined):
            at = call.end() - len("ssh")
            before, after = joined[:at], joined[call.end() :]
            # A pipe into it, not `||`, which runs it on its own stdin.
            piped = re.search(
                r"(?<!\|)\|$", before.rstrip().removesuffix("$(").rstrip()
            )
            flag = re.match(r"\s+-[nt]\b", after)
            if not (piped or flag or REDIRECT.search(after)):
                found.append(start)
        joined = ""
    return found


def owner_scripts() -> list[Path]:
    """The shell scripts under ops/ the owner runs: all but ops/remote/."""
    scripts = []
    for path in sorted((REPO / "ops").rglob("*")):
        if not path.is_file() or (REPO / "ops/remote") in path.parents:
            continue
        try:
            text = path.read_text()
        except UnicodeDecodeError:
            continue
        first = text.split("\n", 1)[0]
        if path.suffix == ".sh" or (first.startswith("#!") and "sh" in first):
            scripts.append(path)
    return scripts


def test_unset_stdin_finds_an_ssh_call_with_nothing_on_its_stdin():
    source = (
        'host_key=$(ssh -o BatchMode=yes "root@$h" cat key.pub | cut -d" " -f1)\n'
        "# ssh root@h uptime\n"
        'ssh -o BatchMode=yes "root@$h" "test -f $1" < /dev/null\n'
        'out=$(ssh -o BatchMode=yes "root@$h" \\\n'
        '  "check $env" < /dev/null 2>&1)\n'
        'printf "%s" "$x" |\n'
        '  ssh "root@$h" "bash -s"\n'
        '{ printf "%s\\n" "$s"; cat "$f"; } | ssh "root@$h" "bash -s"\n'
        'ssh -n "root@$h" uptime\n'
        'ssh "root@$h" "bash -s" <<<"$script"\n'
        'ssh-keygen -q -t ed25519 -f "$dir/key"\n'
        'ssh "root@$h" uptime\n'
        'warn "ssh root@$h failed; reads the server in one ssh round"\n'
        "  ssh round (ops/remote/deploy-facts.sh); notes a Caddy newer\n"
        'exec ssh -t "root@$h" "$remote"\n'
        'if ! ssh "root@$h" true; then :; fi\n'
        'remote_has x || ssh "root@$h" fix\n'
        "remote_has x ||\n"
        '  ssh "root@$h" fix\n'
    )
    assert unset_stdin(source) == [1, 12, 16, 17, 18]


def test_the_scripts_checked_include_the_ones_that_call_ssh():
    names = {str(path.relative_to(REPO)) for path in owner_scripts()}
    assert {
        "ops/provision-vps.d/helpers.sh",
        "ops/provision-vps.d/staging.sh",
        "ops/provision-vps.d/production.sh",
        "ops/deploy-production",
        "ops/field-test-key",
    } <= names
    assert not any(name.startswith("ops/remote/") for name in names)


def test_every_ssh_call_the_owner_runs_sets_its_stdin():
    found = [
        f"{path.relative_to(REPO)}:{line}"
        for path in owner_scripts()
        for line in unset_stdin(path.read_text())
    ]
    assert not found, (
        "ssh forwards its stdin, so a call with none reads the owner's terminal; "
        "give it `< /dev/null` (or what it should read): " + ", ".join(found)
    )
