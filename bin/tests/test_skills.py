"""Tests for bin/skills, which keeps skills/versions.txt on record (`sync`) and tags
each new skill version (`release`). Each runs the script as a process in a scratch
git repo whose `origin` is a bare repo beside it."""

import os
import shutil
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
        return run_in(self.dir, *args)

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


def other_clone(repo: Repo, name: str = "other") -> Path:
    """A second clone of origin, as the owner's host checkout is, set up to commit."""
    clone = repo.dir.parent / name
    git(repo.dir.parent, "clone", "-q", str(repo.origin), str(clone))
    git(clone, "config", "user.email", "o@example.com")
    git(clone, "config", "user.name", "o")
    return clone


def run_in(folder: Path, *args, env: dict | None = None):
    return subprocess.run(
        [str(SCRIPT), *args],
        cwd=folder,
        capture_output=True,
        text=True,
        check=False,
        env=env,
    )


def push_a_version_from(clone: Path, text: str, tagged: bool = True) -> None:
    """In `clone`, change the demo skill to `text`, sync, commit, push, and unless
    `tagged` is false, tag it as CI's release-skills does."""
    (clone / "skills/demo/SKILL.md").write_text(
        (clone / "skills/demo/SKILL.md").read_text().replace("# Demo", text)
    )
    assert run_in(clone, "sync").returncode == 0
    git(clone, "commit", "-qam", text)
    git(clone, "push", "-q", "origin", "main")
    if tagged:
        done = run_in(clone, "release", "--push-url", str(clone / "../origin.git"))
        assert done.returncode == 0, done.stderr


def published(repo: Repo) -> None:
    """Version 1 on record, committed, pushed and tagged."""
    repo.run("sync")
    repo.commit()
    repo.push()
    assert repo.run("release", "--push-url", str(repo.origin)).returncode == 0


def test_check_is_silent_and_passes_when_in_step(repo):
    published(repo)
    done = repo.run("sync", "--check")
    assert done.returncode == 0, done.stdout + done.stderr
    assert done.stdout == ""


def test_check_fetches_and_names_a_version_another_clone_tagged_with_other_text(repo):
    published(repo)
    repo.skill.write_text(SKILL.replace("# Demo", "# Demo, mine"))
    repo.run("sync")
    repo.commit("mine, version 2, not pushed")
    push_a_version_from(other_clone(repo), "# Demo, theirs")
    # This clone hasn't fetched: origin/main here still records version 1.
    done = repo.run("sync", "--check")
    assert done.returncode == 1, done.stdout + done.stderr
    assert "demo: version 2" in done.stdout
    assert "bin/skills sync" in done.stdout
    assert '  version: "2"' in repo.skill.read_text(), "the check writes nothing"


def test_check_names_a_version_pushed_with_other_text_before_it_is_tagged(repo):
    published(repo)
    repo.skill.write_text(SKILL.replace("# Demo", "# Demo, mine"))
    repo.run("sync")
    repo.commit("mine")
    push_a_version_from(other_clone(repo), "# Demo, theirs", tagged=False)
    done = repo.run("sync", "--check")
    assert done.returncode == 1, done.stdout + done.stderr
    assert "demo: version 2" in done.stdout


def test_sync_bumps_past_the_highest_version_tagged(repo):
    published(repo)
    repo.skill.write_text(SKILL.replace("# Demo", "# Demo, mine"))
    repo.run("sync")
    repo.commit("mine, version 2")
    clone = other_clone(repo)
    push_a_version_from(clone, "# Demo, theirs")
    push_a_version_from(clone, "# Demo, theirs again")
    done = repo.run("sync")
    assert done.returncode == 0, done.stderr
    assert '  version: "4"' in repo.skill.read_text()
    assert repo.run("sync", "--check").returncode == 0


def test_an_older_tree_whose_tag_holds_the_same_folder_passes(repo):
    published(repo)
    push_a_version_from(other_clone(repo), "# Demo, newer")
    # repo is still at version 1, which its tag holds as it is.
    done = repo.run("sync", "--check")
    assert done.returncode == 0, done.stdout + done.stderr


def test_check_names_a_stale_reference_copy_and_record(repo):
    published(repo)
    (repo.dir / "docs/entry-format.md").write_text("# Format, revised\n")
    repo.skill.write_text(repo.skill.read_text().replace("# Demo", "# Demo, edited"))
    done = repo.run("sync", "--check")
    assert done.returncode == 1, done.stdout + done.stderr
    assert "skills/demo/references/entry-format.md" in done.stdout
    assert "skills/versions.txt: demo's line" in done.stdout
    assert (
        repo.dir / "skills/demo/references/entry-format.md"
    ).read_text() == "# Format\n"


def test_with_origin_unreachable_it_says_so_and_passes_a_tree_in_step(repo):
    published(repo)
    git(repo.dir, "remote", "set-url", "origin", str(repo.dir.parent / "gone.git"))
    done = repo.run("sync", "--check")
    assert done.returncode == 0, done.stdout + done.stderr
    assert "couldn't fetch" in done.stderr


def test_with_origin_unreachable_the_tags_fetched_before_still_count(repo):
    published(repo)
    repo.skill.write_text(SKILL.replace("# Demo", "# Demo, mine"))
    repo.run("sync")
    repo.commit("mine, version 2")
    clone = other_clone(repo)
    push_a_version_from(clone, "# Demo, theirs")
    # Then version 3, so origin/main's record no longer names version 2: only the
    # tag fetched before shows the clash.
    push_a_version_from(clone, "# Demo, theirs again")
    # An old commit on origin/main: the age said is the fetch's, not the commit's.
    (repo.dir.parent / "other/old.txt").write_text("old\n")
    other = repo.dir.parent / "other"
    git(other, "add", "old.txt")
    subprocess.run(
        ["git", "commit", "-qm", "old"],
        cwd=other,
        check=True,
        env=os.environ | {"GIT_COMMITTER_DATE": "2020-01-01T00:00:00"},
    )
    git(other, "push", "-q", "origin", "main")
    assert repo.run("sync", "--check").returncode == 1  # it fetches the tag
    git(repo.dir, "remote", "set-url", "origin", str(repo.dir.parent / "gone.git"))
    done = repo.run("sync", "--check")
    assert done.returncode == 1, done.stdout + done.stderr
    assert "demo: version 2" in done.stdout
    assert "couldn't fetch" in done.stderr
    assert "years" not in done.stderr, done.stderr


def test_a_fetch_that_never_answers_is_stopped_at_the_limit(repo, tmp_path):
    published(repo)
    fakes = tmp_path / "fakes"
    fakes.mkdir()
    real = shutil.which("git")
    (fakes / "git").write_text(
        f'#!/bin/sh\nfor a; do [ "$a" = fetch ] && exec sleep 600; done\nexec {real} "$@"\n'
    )
    (fakes / "git").chmod(0o755)
    env = os.environ | {
        "PATH": f"{fakes}{os.pathsep}{os.environ['PATH']}",
        "FILEDRAWER_FETCH_SECONDS": "1",
    }
    done = run_in(repo.dir, "sync", "--check", env=env)
    assert done.returncode == 0, done.stdout + done.stderr
    assert "no answer within 1 s" in done.stderr


def test_an_unreadable_record_is_exit_2(repo):
    published(repo)
    (repo.dir / "skills/versions.txt").write_text("demo one two three\n")
    done = repo.run("sync", "--check")
    assert done.returncode == 2, done.stdout + done.stderr
    assert "versions.txt" in done.stderr


def test_root_checks_another_folder_while_git_runs_in_the_repo(repo, tmp_path):
    published(repo)
    push_a_version_from(other_clone(repo), "# Demo, theirs")
    # A tree at version 2 with other text and no .git, as agent-pre-commit's copy of
    # the index is: the repo's tags decide, and the repo's own files don't count.
    tree = tmp_path / "tree"
    shutil.copytree(repo.dir / "docs", tree / "docs")
    shutil.copytree(repo.dir / "skills", tree / "skills")
    skill = tree / "skills/demo/SKILL.md"
    skill.write_text(
        skill.read_text()
        .replace("# Demo", "# Demo, mine")
        .replace('version: "1"', 'version: "2"')
    )
    done = repo.run("sync", "--check", "--root", str(tree))
    assert done.returncode == 1, done.stdout + done.stderr
    assert "demo: version 2" in done.stdout
    assert '  version: "1"' in repo.skill.read_text()


def test_sync_names_untracked_files_in_a_skill_folder(repo):
    published(repo)
    (repo.dir / "skills/demo/notes.txt").write_text("a stray note\n")
    done = repo.run("sync")
    assert done.returncode == 0, done.stderr
    assert "skills/demo/notes.txt" in done.stdout
