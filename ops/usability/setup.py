#!/usr/bin/env python3
"""Prepares a usability run's home before the agent starts: drive-agent runs it as the
run's user, in its box, with the scenario's path as its argument (ops/usability-run).

    setup.py SCENARIO

It writes ~/.config/filedrawer/<host>.header (mode 600) with the staging key line,
and the API token after it when the scenario's `token` is "header", as AGENTS.md
says a run on staging needs. Then, by the scenario's keys:
  install         "installed": run the installer and the mode script /skill.md shows,
                  as an agent would; "none" (default): install nothing
  mode            auto or ask, the mode script's choice (default auto)
  version_down    skill names whose installed copy gets metadata.version lowered by
                  one, so the agent finds an upgrade
  agents_blocked  true: ~/.agents is a file, so an install fails

Environment: FILEDRAWER_STAGING_KEY, FILEDRAWER_USABILITY_BASE (the server),
FILEDRAWER_USABILITY_HARNESS (claude makes ~/.claude, so the installer links the
skills there too) and, for token "header", FILEDRAWER_TOKEN.
"""

import json
import os
import re
import subprocess
import sys
import urllib.parse
from pathlib import Path


def main() -> int:
    scenario = json.loads(Path(sys.argv[1]).read_text())
    home = Path.home()
    base = os.environ["FILEDRAWER_USABILITY_BASE"]
    host = urllib.parse.urlsplit(base).hostname
    key = os.environ.get("FILEDRAWER_STAGING_KEY", "")
    lines = [f"X-Staging-Key: {key}"] if key else []
    if scenario.get("token") == "header":
        if not os.environ.get("FILEDRAWER_TOKEN"):
            sys.exit(
                "setup: the scenario's token is header, but FILEDRAWER_TOKEN is unset"
            )
        lines.append(f"Authorization: Bearer {os.environ['FILEDRAWER_TOKEN']}")
    # The installer links the skills into ~/.claude/skills only when ~/.claude
    # exists, as it does on a machine with Claude Code.
    if os.environ.get("FILEDRAWER_USABILITY_HARNESS") == "claude":
        (home / ".claude").mkdir(exist_ok=True)
    config = home / ".config" / "filedrawer"
    config.mkdir(parents=True, exist_ok=True)
    header = config / f"{host}.header"
    fd = os.open(header, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    with os.fdopen(fd, "w") as f:
        f.write("".join(line + "\n" for line in lines))
    if scenario.get("agents_blocked"):
        (home / ".agents").write_text("")
    if scenario.get("install") == "installed":
        install(scenario, base, key)
    return 0


def install(scenario: dict, base: str, key: str) -> None:
    """Runs the installer and mode script /skill.md shows, its first two sh blocks."""
    curl = ["curl", "-fsS"] + (["-H", f"X-Staging-Key: {key}"] if key else [])
    page = subprocess.run(
        [*curl, f"{base}/skill.md"], check=True, text=True, capture_output=True
    ).stdout
    blocks = re.findall(r"```sh\n(.*?)```", page, re.DOTALL)
    if len(blocks) < 2:
        sys.exit("setup: /skill.md shows fewer than two sh blocks to install with")
    subprocess.run(["sh", "-c", blocks[0]], check=True)
    mode = scenario.get("mode", "auto")
    subprocess.run(
        ["sh", "-c", blocks[1].replace("mode=MODE", f"mode={mode}")], check=True
    )
    for skill in scenario.get("version_down", []):
        path = Path.home() / ".agents" / "skills" / skill / "SKILL.md"
        text = path.read_text()
        version = int(re.search(r'^  version: "(\d+)"$', text, re.MULTILINE).group(1))
        text = text.replace(f'  version: "{version}"', f'  version: "{version - 1}"', 1)
        text = text.replace(f"{skill}/{version}", f"{skill}/{version - 1}")
        path.write_text(text)


if __name__ == "__main__":
    sys.exit(main())
