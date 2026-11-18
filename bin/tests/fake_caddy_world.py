"""The server's tools as caddy-upgrade-check sees them, for test_caddy_upgrades.py.

Usage: fake_caddy_world.py WORLD TOOL ARG...

WORLD is a directory holding world.json, the server's state: the Caddy version
installed, whether it's held, which versions apt can install, which versions accept
the config and serve the sites, which backends answer, apt's candidate, and whether
the dpkg lock or mail is failing. Each call is noted in WORLD/log.jsonl with its
arguments and stdin; a mail's message goes to WORLD/mails.jsonl.
"""

import json
import re
import sys
from pathlib import Path

PORTS = {"8080": "production", "8081": "staging"}


def main(world_dir: Path, tool: str, args: list[str]) -> int:
    path = world_dir / "world.json"
    world = json.loads(path.read_text())
    stdin = "" if sys.stdin.isatty() else sys.stdin.read()
    with (world_dir / "log.jsonl").open("a") as log:
        log.write(json.dumps({"tool": tool, "args": args, "stdin": stdin}) + "\n")
    status = TOOLS[tool](world, args, stdin, world_dir)
    path.write_text(json.dumps(world))
    return status


def caddy(world, args, stdin, world_dir):
    if args[:1] == ["version"]:
        print(f"v{world['installed']} h1:abc=")
        return 0
    if args[:1] == ["validate"]:
        if world["installed"] in world["config_ok"]:
            return 0
        # Caddy quotes the line it refused, which may hold the staging key.
        print(
            f'Error: adapting config: unrecognized directive: header X-Staging-Key "{world["key"]}"'
        )
        return 1
    return 2


def dpkg_query(world, args, stdin, world_dir):
    print(world["installed"], end="")
    return 0


def apt_get(world, args, stdin, world_dir):
    if world.get("lock_busy"):
        print(
            "E: Could not get lock /var/lib/dpkg/lock-frontend. It is held by process 1"
        )
        return 100
    [wanted] = [a.split("=", 1)[1] for a in args if a.startswith("caddy=")]
    if wanted not in world["available"]:
        print(f"E: Version '{wanted}' for 'caddy' was not found")
        return 100
    world["installed"] = wanted
    return 0


def version_key(version: str) -> tuple[int, ...]:
    return tuple(int(part) for part in version.split("."))


COMPARE = {
    "lt": lambda a, b: a < b,
    "gt": lambda a, b: a > b,
    "eq": lambda a, b: a == b,
}


def dpkg(world, args, stdin, world_dir):
    if args[:1] == ["--compare-versions"]:
        a, op, b = args[1:]
        return 0 if COMPARE[op](version_key(a), version_key(b)) else 1
    deb = args[-1]
    match = re.search(r"caddy_([^_]+)_", deb)
    if "-i" not in args or not match or not Path(deb).exists():
        return 2
    world["installed"] = match.group(1)
    return 0


def apt_mark(world, args, stdin, world_dir):
    if args == ["hold", "caddy"]:
        world["held"] = True
    elif args == ["unhold", "caddy"]:
        world["held"] = False
    elif args == ["showhold", "caddy"]:
        if world["held"]:
            print("caddy")
    else:
        return 2
    return 0


def apt_cache(world, args, stdin, world_dir):
    print("caddy:")
    print(f"  Installed: {world['installed']}")
    print(f"  Candidate: {world.get('candidate', world['installed'])}")
    return 0


def nothing(world, args, stdin, world_dir):
    return 0


def curl(world, args, stdin, world_dir):
    url = next((a for a in args if re.match(r"(https?|smtps)://", a)), None)
    config_url = re.search(r'^url = "([^"]*)"', stdin, re.M)
    if url is None and config_url:
        url = config_url.group(1)
    if url and url.startswith("smtps://"):
        if world.get("mail_fails"):
            print("curl: (67) Login denied", file=sys.stderr)
            return 67
        message = Path(args[args.index("-T") + 1]).read_text()
        with (world_dir / "mails.jsonl").open("a") as mails:
            mails.write(json.dumps(message) + "\n")
        return 0
    if url and url.startswith("https://hc-ping.com/"):
        return 0
    backend = re.match(r"http://127\.0\.0\.1:(\d+)/tags$", url or "")
    if backend:
        up = world["backends"].get(PORTS[backend.group(1)], False)
        print("200" if up else "000", end="")
        return 0 if up else 7
    site = re.match(r"https://([^/]+)(/.*)$", url or "")
    if site and "--resolve" in args:
        print(through_caddy(world, site.group(1), site.group(2), stdin), end="")
        return 0
    return 2


def through_caddy(world, host, path, stdin) -> str:
    """What `-w '%{http_code} %{redirect_url}'` prints for a request through Caddy."""
    if world["installed"] not in world["sites_ok"]:
        return "502 "
    environment = "staging" if host.endswith("staging.filedrawer.dev") else "production"
    if not world["backends"].get(environment):
        return "502 "
    if host == "www.filedrawer.dev":
        return "301 https://filedrawer.dev/"
    if environment == "production":
        return "200 "
    if path == "/robots.txt":
        return "200 "
    keyed = f"X-Staging-Key: {world['key']}" in stdin
    return "200 " if keyed else "401 "


TOOLS = {
    "caddy": caddy,
    "dpkg-query": dpkg_query,
    "apt-get": apt_get,
    "dpkg": dpkg,
    "apt-mark": apt_mark,
    "apt-cache": apt_cache,
    "systemctl": nothing,
    "systemd-run": nothing,
    "curl": curl,
}

if __name__ == "__main__":
    sys.exit(main(Path(sys.argv[1]), sys.argv[2], sys.argv[3:]))
