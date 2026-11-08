"""Tests for ops/protect-skill-tags, which lets only CI's deploy key create, move or
delete the repo's skill-* tags. It runs with a fake gh that logs each call and its
stdin, and the real ssh-keygen."""

import json
import subprocess
from pathlib import Path

import pytest

SCRIPT = Path(__file__).resolve().parents[2] / "ops/protect-skill-tags"

FAKE_GH = """#!/bin/sh
n=$(($(ls "$GH_DIR/calls" | wc -l) + 1))
printf '%s\\n' "$*" > "$GH_DIR/calls/$n"
case "$*" in
  *"--input -"*|"secret set"*) cat > "$GH_DIR/stdin/$n" ;;
esac
case "$*" in
  "api repos/{owner}/{repo}/keys --jq"*) cat "$GH_DIR/key-ids" ;;
  "api repos/{owner}/{repo}/rulesets --jq"*) cat "$GH_DIR/ruleset-ids" ;;
  "api repos/{owner}/{repo}/environments/skill-tags/deployment-branch-policies --jq"*) cat "$GH_DIR/policies" ;;
esac
"""


class Gh:
    def __init__(self, root: Path):
        self.dir = root / "gh"
        for sub in ("bin", "calls", "stdin"):
            (self.dir / sub).mkdir(parents=True)
        fake = self.dir / "bin/gh"
        fake.write_text(FAKE_GH)
        fake.chmod(0o755)
        (self.dir / "key-ids").write_text("")
        (self.dir / "ruleset-ids").write_text("")
        (self.dir / "policies").write_text("")

    def run(self, *args):
        env = {
            "PATH": f"{self.dir / 'bin'}:/usr/bin:/bin",
            "GH_DIR": str(self.dir),
            "HOME": str(self.dir),
        }
        return subprocess.run(
            [str(SCRIPT), *args], env=env, capture_output=True, text=True, check=False
        )

    def calls(self) -> list[tuple[str, str | None]]:
        """Each call's arguments, and its stdin if it read one, in order."""
        out = []
        for n in sorted(int(p.name) for p in (self.dir / "calls").iterdir()):
            stdin = self.dir / "stdin" / str(n)
            out.append(
                (
                    (self.dir / "calls" / str(n)).read_text().strip(),
                    stdin.read_text() if stdin.exists() else None,
                )
            )
        return out

    def call(self, start: str) -> tuple[str, str | None]:
        found = [c for c in self.calls() if c[0].startswith(start)]
        assert len(found) == 1, (start, self.calls())
        return found[0]


@pytest.fixture
def gh(tmp_path):
    return Gh(tmp_path)


def test_help_names_what_it_does(gh):
    done = gh.run("-h")
    assert done.returncode == 0
    assert "skill-*" in done.stdout
    assert gh.calls() == []


def test_the_deploy_key_ci_holds_is_the_one_github_trusts(gh):
    done = gh.run()
    assert done.returncode == 0, done.stderr
    added, _ = gh.call("api -X POST repos/{owner}/{repo}/keys")
    assert "read_only=false" in added
    public = added.split("key=", 1)[1].split(" -F")[0]
    _, private = gh.call("secret set SKILL_TAGS_SSH_KEY --env skill-tags")
    key = gh.dir / "key"
    key.write_text(private)
    key.chmod(0o600)
    derived = subprocess.run(
        ["ssh-keygen", "-y", "-f", str(key)], capture_output=True, text=True, check=True
    ).stdout.split()[:2]
    assert public.split()[:2] == derived


def test_a_rerun_replaces_the_old_deploy_keys(gh):
    (gh.dir / "key-ids").write_text("11\n12\n")
    done = gh.run()
    assert done.returncode == 0, done.stderr
    calls = [c for c, _ in gh.calls()]
    assert "api -X DELETE repos/{owner}/{repo}/keys/11" in calls
    assert "api -X DELETE repos/{owner}/{repo}/keys/12" in calls


def test_only_main_may_use_the_environment(gh):
    gh.run()
    _, body = gh.call("api -X PUT repos/{owner}/{repo}/environments/skill-tags")
    assert json.loads(body)["deployment_branch_policy"]["custom_branch_policies"]
    policy, _ = gh.call(
        "api -X POST repos/{owner}/{repo}/environments/skill-tags/deployment-branch-policies"
    )
    assert "name=main" in policy


def test_a_rerun_keeps_the_main_branch_policy(gh):
    (gh.dir / "policies").write_text("main\n")
    done = gh.run()
    assert done.returncode == 0, done.stderr
    assert not [
        c for c, _ in gh.calls() if c.startswith("api -X POST") and "policies" in c
    ]


def ruleset(body: str) -> dict:
    rules = json.loads(body)
    assert rules["name"] == "skill tags"
    assert rules["target"] == "tag"
    assert rules["enforcement"] == "active"
    assert rules["conditions"]["ref_name"]["include"] == ["refs/tags/skill-*"]
    assert sorted(r["type"] for r in rules["rules"]) == [
        "creation",
        "deletion",
        "update",
    ]
    assert rules["bypass_actors"] == [
        {"actor_id": None, "actor_type": "DeployKey", "bypass_mode": "always"}
    ]
    return rules


def test_it_creates_the_ruleset(gh):
    done = gh.run()
    assert done.returncode == 0, done.stderr
    _, body = gh.call("api -X POST repos/{owner}/{repo}/rulesets")
    ruleset(body)


def test_a_rerun_updates_the_ruleset_in_place(gh):
    (gh.dir / "ruleset-ids").write_text("7\n")
    done = gh.run()
    assert done.returncode == 0, done.stderr
    _, body = gh.call("api -X PUT repos/{owner}/{repo}/rulesets/7")
    ruleset(body)
    assert not [
        c
        for c, _ in gh.calls()
        if c.startswith("api -X POST repos/{owner}/{repo}/rulesets")
    ]
