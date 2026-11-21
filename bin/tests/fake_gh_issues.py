"""A fake gh for the scripts that report by issue (bin/caddy-latest, bin/advisories).

Usage: fake_gh_issues.py WORLD LOG ARG...

WORLD is a JSON file: `api`, a map from an API path to its answer, and `issues`, each
with number, title, state (open or closed), body, author (a login) and comments, each
an author and a body. LOG gets every call's arguments, a JSON list a line.

It answers `gh api` for a path in `api`, for the repo's issues and for an issue's
comments, as --paginate --slurp prints a list: a list of pages. It lists every issue
and comment whatever `creator` the call names, so a test shows what the script itself
counts as its own. What `gh issue create`, `comment`, `edit --body` and `close
[--comment]` write is the workflow's, as on GitHub.
"""

import json
import sys
from pathlib import Path

BOT = "github-actions[bot]"


def user(login: str) -> dict:
    return {"login": login, "type": "Bot" if login.endswith("[bot]") else "User"}


def option(args: list[str], name: str) -> str:
    return args[args.index(name) + 1]


def main(world_path: Path, log: Path, args: list[str]) -> int:
    world = json.loads(world_path.read_text())
    with log.open("a") as out:
        out.write(json.dumps(args) + "\n")
    if args[:1] == ["api"]:
        [path] = [a for a in args[1:] if a.startswith("repos/")]
        if path in world["api"]:
            answer = world["api"][path]
        elif "--slurp" in args and path == "repos/{owner}/{repo}/issues":
            issues = [dict(i, user=user(i["author"])) for i in world["issues"]]
            answer = [issues[:1], issues[1:]]
        elif "--slurp" in args and path.endswith("/comments"):
            [issue] = [
                i for i in world["issues"] if str(i["number"]) == path.split("/")[-2]
            ]
            answer = [
                [
                    {"user": user(c["author"]), "body": c["body"]}
                    for c in issue["comments"]
                ]
            ]
        else:
            print("gh: Not Found (HTTP 404)", file=sys.stderr)
            return 1
        print(json.dumps(answer))
        return 0
    match args:
        case ["issue", "create", *_]:
            world["issues"].append(
                {
                    "number": len(world["issues"]) + 1,
                    "title": option(args, "--title"),
                    "state": "open",
                    "body": option(args, "--body"),
                    "author": BOT,
                    "comments": [],
                }
            )
        case ["issue", "comment" | "edit" | "close" as verb, number, *_]:
            [issue] = [i for i in world["issues"] if str(i["number"]) == number]
            if verb == "edit":
                issue["body"] = option(args, "--body")
            if verb == "comment":
                issue["comments"].append(
                    {"author": BOT, "body": option(args, "--body")}
                )
            if verb == "close":
                if "--comment" in args:
                    issue["comments"].append(
                        {"author": BOT, "body": option(args, "--comment")}
                    )
                issue["state"] = "closed"
        case _:
            return 2
    world_path.write_text(json.dumps(world))
    return 0


if __name__ == "__main__":
    sys.exit(main(Path(sys.argv[1]), Path(sys.argv[2]), sys.argv[3:]))
