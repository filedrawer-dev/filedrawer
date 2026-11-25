#!/usr/bin/env python3
"""Prepares a usability run's home before the agent starts: drive-agent runs it as the
run's user, in its box, with the scenario's path as its argument (ops/usability-run).

    setup.py SCENARIO

It writes ~/.config/filedrawer/<host>.header (mode 600) with the staging key line,
and the API token after it when the scenario's `token` is "header", as AGENTS.md
says a run on staging needs. Then, by the scenario's keys:
  install         "installed": download BASE/install.sh and run it, as /skill.md has an
                  agent do; "none" (default): install nothing
  mode            auto or ask, the installer's argument (default auto)
  pin             skill name -> version, such as {"filedrawer-search": 16}: after the
                  install, the skill's folder is replaced by its files at the tag
                  skill-<name>-v<version>, pointed at the site as the installer does,
                  to run an older version's text; with mode auto, the skill's own
                  upgrade check installs the current version once it fires, so an
                  A/B of versions runs with mode ask
  version_down    skill names whose installed copy gets metadata.version lowered by
                  one, so the agent finds an upgrade (after any pin)
  agents_blocked  true: ~/.agents is a file, so an install fails

Environment: FILEDRAWER_STAGING_KEY, FILEDRAWER_USABILITY_BASE (the server),
FILEDRAWER_USABILITY_HARNESS (claude makes ~/.claude, so the installer links the
skills there too), for token "header", FILEDRAWER_TOKEN, and FILEDRAWER_USABILITY_RAW
(tests), the raw host pin fetches tags from in place of raw.githubusercontent.com.
"""

import json
import os
import re
import shutil
import subprocess
import sys
import urllib.parse
import urllib.request
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
    """Downloads the installer /skill.md names and runs it with the scenario's mode."""
    script = Path.home() / ".config" / "filedrawer" / "install.sh"
    curl = ["curl", "-fsS"] + (["-H", f"X-Staging-Key: {key}"] if key else [])
    subprocess.run([*curl, "-o", str(script), f"{base}/install.sh"], check=True)
    subprocess.run(["sh", str(script), scenario.get("mode", "auto")], check=True)
    for skill, version in scenario.get("pin", {}).items():
        pin(skill, int(version), base)
    for skill in scenario.get("version_down", []):
        path = Path.home() / ".agents" / "skills" / skill / "SKILL.md"
        text = path.read_text()
        version = int(re.search(r'^  version: "(\d+)"$', text, re.MULTILINE).group(1))
        text = text.replace(f'  version: "{version}"', f'  version: "{version - 1}"', 1)
        text = text.replace(f"{skill}/{version}", f"{skill}/{version - 1}")
        path.write_text(text)


RAW = "https://raw.githubusercontent.com/filedrawer-dev/filedrawer"


def pin(skill: str, version: int, base: str) -> None:
    """Replaces the installed skill's folder with its files at tag VERSION, pointed
    at BASE as the installer points a copy installed from a site other than
    production."""
    raw = os.environ.get("FILEDRAWER_USABILITY_RAW", RAW)
    tag = f"{raw}/refs/tags/skill-{skill}-v{version}/skills/{skill}"
    host = urllib.parse.urlsplit(base).hostname

    def fetch(name: str) -> str:
        url = f"{tag}/{name}"
        try:
            with urllib.request.urlopen(url, timeout=30) as reply:
                text = reply.read().decode()
        except (OSError, UnicodeDecodeError) as e:
            sys.exit(
                f"setup: couldn't fetch {url} to pin {skill}: {e}. If the network"
                " is down, run again later; if not, that version has no tag"
            )
        return text.replace("https://filedrawer.dev", base).replace(
            "/filedrawer.dev.header", f"/{host}.header"
        )

    skill_md = fetch("SKILL.md")
    listed = re.search(r'^  files: "([^"]*)"$', skill_md, re.MULTILINE)
    if not listed:
        sys.exit(
            f"setup: {tag}/SKILL.md lists no files, so it isn't a skill's tag:"
            f" check the version pinned for {skill}"
        )
    files = listed.group(1).split()
    texts = {name: skill_md if name == "SKILL.md" else fetch(name) for name in files}
    folder = Path.home() / ".agents" / "skills" / skill
    shutil.rmtree(folder, ignore_errors=True)
    for name, text in texts.items():
        (folder / name).parent.mkdir(parents=True, exist_ok=True)
        (folder / name).write_text(text)


if __name__ == "__main__":
    sys.exit(main())
