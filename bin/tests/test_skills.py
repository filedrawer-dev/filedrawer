"""Tests for bin/skills, which keeps skills/versions.txt on record (`sync`) and tags
each new skill version (`release`). Each runs the script as a process in a scratch
git repo whose `origin` is a bare repo beside it."""

import subprocess
from pathlib import Path

import pytest

SCRIPT = Path(__file__).resolve().parents[1] / "skills"

SKILL = """---
name: demo
description: A demo skill.
metadata:
  version: "1"
  files: "SKILL.md references/entry-format.md"
---

# Demo

```sh
curl -sS -H @"$HOME/.config/filedrawer/{{HOST}}.header" -H "X-Filedrawer-Skill: demo/1" {{BASE}}/tags
```
"""


def git(cwd, *args):
    return subprocess.run(
        ["git", *args], cwd=cwd, capture_output=True, text=True, check=True
    ).stdout


def shell_hash(folder: Path) -> str:
    """The folder's hash as versions.txt's header says to compute it."""
    done = subprocess.run(
        "find . -type f | LC_ALL=C sort | xargs sha256sum | sha256sum",
        shell=True,
        cwd=folder,
        capture_output=True,
        text=True,
        check=True,
    )
    return done.stdout.split()[0]


class Repo:
    def __init__(self, root: Path):
        self.origin = root / "origin.git"
        self.dir = root / "repo"
        subprocess.run(
            ["git", "init", "-q", "--bare", "-b", "main", str(self.origin)], check=True
        )
        self.dir.mkdir()
        git(self.dir, "init", "-q", "-b", "main")
        git(self.dir, "config", "user.email", "t@example.com")
        git(self.dir, "config", "user.name", "t")
        git(self.dir, "remote", "add", "origin", str(self.origin))
        (self.dir / "docs").mkdir()
        (self.dir / "docs/entry-format.md").write_text("# Format\n")
        (self.dir / "skills/demo/references").mkdir(parents=True)
        (self.dir / "skills/demo/SKILL.md").write_text(SKILL)
        (self.dir / "skills/demo/references/entry-format.md").write_text("# Format\n")

    @property
    def skill(self) -> Path:
        return self.dir / "skills/demo/SKILL.md"

    def run(self, *args):
        return subprocess.run(
            [str(SCRIPT), *args],
            cwd=self.dir,
            capture_output=True,
            text=True,
            check=False,
        )

    def record(self) -> list[str]:
        lines = (self.dir / "skills/versions.txt").read_text().splitlines()
        return [line for line in lines if line and not line.startswith("#")]

    def commit(self, message="change"):
        git(self.dir, "add", "-A")
        git(self.dir, "commit", "-q", "-m", message)

    def push(self):
        git(self.dir, "push", "-q", "origin", "main")

    def tags(self) -> str:
        return git(self.origin, "tag", "--list")


@pytest.fixture
def repo(tmp_path):
    return Repo(tmp_path)


def test_sync_records_a_new_skill_at_its_version(repo):
    done = repo.run("sync")
    assert done.returncode == 0, done.stderr
    assert repo.record() == [f"demo 1 {shell_hash(repo.dir / 'skills/demo')}"]
    assert "find . -type f" in (repo.dir / "skills/versions.txt").read_text()


def test_sync_bumps_a_version_already_pushed(repo):
    repo.run("sync")
    repo.commit()
    repo.push()
    repo.skill.write_text(SKILL.replace("# Demo", "# Demo, changed"))
    done = repo.run("sync")
    assert done.returncode == 0, done.stderr
    text = repo.skill.read_text()
    assert '  version: "2"' in text
    assert '"X-Filedrawer-Skill: demo/2"' in text
    assert "demo/1" not in text
    assert repo.record() == [f"demo 2 {shell_hash(repo.dir / 'skills/demo')}"]


def test_sync_keeps_a_version_not_pushed_yet(repo):
    repo.run("sync")
    repo.commit()
    repo.push()
    repo.skill.write_text(SKILL.replace("# Demo", "# Demo, changed"))
    repo.run("sync")
    repo.commit()
    repo.skill.write_text(repo.skill.read_text().replace("changed", "changed again"))
    done = repo.run("sync")
    assert done.returncode == 0, done.stderr
    assert '  version: "2"' in repo.skill.read_text()
    assert repo.record() == [f"demo 2 {shell_hash(repo.dir / 'skills/demo')}"]


def test_sync_bumps_past_a_version_pushed_with_other_text(repo):
    # This work synced while version 2 was unpushed, and kept 2; then the owner pushed
    # version 2 without this work, so 2 is tagged with other text.
    repo.run("sync")
    repo.commit()
    repo.push()
    repo.skill.write_text(SKILL.replace("# Demo", "# Demo, theirs"))
    repo.run("sync")
    repo.commit("theirs")
    repo.skill.write_text(repo.skill.read_text().replace("theirs", "theirs and mine"))
    repo.run("sync")
    repo.commit("mine")
    git(repo.dir, "push", "-q", "origin", "HEAD~1:main")
    assert '  version: "2"' in repo.skill.read_text()
    done = repo.run("sync")
    assert done.returncode == 0, done.stderr
    assert '  version: "3"' in repo.skill.read_text()
    assert repo.record() == [f"demo 3 {shell_hash(repo.dir / 'skills/demo')}"]


def test_sync_leaves_a_skill_on_record_alone(repo):
    repo.run("sync")
    repo.commit()
    repo.push()
    before = repo.skill.read_text()
    done = repo.run("sync")
    assert done.returncode == 0, done.stderr
    assert repo.skill.read_text() == before
    assert repo.record()[0].startswith("demo 1 ")


def test_sync_copies_the_entry_format_and_bumps(repo):
    repo.run("sync")
    repo.commit()
    repo.push()
    (repo.dir / "docs/entry-format.md").write_text("# Format, revised\n")
    done = repo.run("sync")
    assert done.returncode == 0, done.stderr
    bundled = repo.dir / "skills/demo/references/entry-format.md"
    assert bundled.read_text() == "# Format, revised\n"
    assert '  version: "2"' in repo.skill.read_text()


def test_release_tags_a_new_version_at_this_commit(repo):
    repo.run("sync")
    repo.commit()
    repo.push()
    done = repo.run("release", "--push-url", str(repo.origin))
    assert done.returncode == 0, done.stderr
    assert repo.tags().split() == ["skill-demo-v1"]
    head = git(repo.dir, "rev-parse", "HEAD").strip()
    assert git(repo.origin, "rev-parse", "skill-demo-v1^{commit}").strip() == head
    assert "skill-demo-v1" in done.stdout


def test_release_accepts_a_tag_that_holds_the_same_folder(repo):
    repo.run("sync")
    repo.commit()
    repo.push()
    repo.run("release", "--push-url", str(repo.origin))
    (repo.dir / "README.md").write_text("unrelated\n")
    repo.commit("unrelated")
    repo.push()
    done = repo.run("release", "--push-url", str(repo.origin))
    assert done.returncode == 0, done.stderr
    assert repo.tags().split() == ["skill-demo-v1"]


def test_release_fails_when_a_tagged_version_changed(repo):
    repo.run("sync")
    repo.commit()
    repo.push()
    repo.run("release", "--push-url", str(repo.origin))
    repo.skill.write_text(SKILL.replace("# Demo", "# Demo, changed"))
    repo.commit("changed without a bump")
    repo.push()
    done = repo.run("release", "--push-url", str(repo.origin))
    assert done.returncode != 0
    assert "skill-demo-v1" in done.stderr
    assert "bin/skills sync" in done.stderr
