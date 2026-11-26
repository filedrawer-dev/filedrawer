"""Tests for `bin/skills merge`, the merge driver .gitattributes names for
skills/versions.txt and install.py registers. Each builds a scratch clone holding
two skills, the repo's .gitattributes and a copy of bin/skills, with the driver
registered as install.py does it, and an `origin` bare repo beside it for sync's
fetch."""

import importlib.util
import os
import shutil
import subprocess
from pathlib import Path

import pytest

REPO = Path(__file__).resolve().parents[2]
SKILL = """---
name: {name}
description: A demo skill.
metadata:
  version: "1"
---

# Top

one

# Middle

two

# Bottom

three
"""
# Without the driver: what a clone set up before it gets.
NO_DRIVER = ("-c", "merge.skills-versions.driver=false")


def git(cwd: Path, *args: str, check: bool = True) -> subprocess.CompletedProcess:
    return subprocess.run(
        ["git", *args],
        cwd=cwd,
        capture_output=True,
        text=True,
        check=check,
        env=os.environ | {"GIT_SEQUENCE_EDITOR": ":", "GIT_EDITOR": "true"},
    )


def install_module():
    spec = importlib.util.spec_from_file_location("install", REPO / "install.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class Clone:
    def __init__(self, root: Path):
        self.dir = root / "repo"
        origin = root / "origin.git"
        git(root, "init", "-q", "--bare", "-b", "main", str(origin))
        self.dir.mkdir()
        git(self.dir, "init", "-q", "-b", "main")
        git(self.dir, "config", "user.email", "t@example.com")
        git(self.dir, "config", "user.name", "t")
        git(self.dir, "remote", "add", "origin", str(origin))
        install_module().register_merge_driver(self.dir)
        (self.dir / "bin").mkdir()
        shutil.copy2(REPO / "bin/skills", self.dir / "bin/skills")
        shutil.copy2(REPO / ".gitattributes", self.dir / ".gitattributes")
        (self.dir / "docs").mkdir()
        (self.dir / "docs/entry-format.md").write_text("# Format\n")
        for name in ("alpha", "beta"):
            (self.dir / "skills" / name).mkdir(parents=True)
            (self.dir / "skills" / name / "SKILL.md").write_text(
                SKILL.format(name=name)
            )
        self.sync()
        git(self.dir, "add", "-A")
        git(self.dir, "commit", "-q", "-m", "start")
        git(self.dir, "push", "-q", "origin", "main")

    def sync(self, *args: str) -> subprocess.CompletedProcess:
        return subprocess.run(
            [str(self.dir / "bin/skills"), "sync", *args],
            cwd=self.dir,
            capture_output=True,
            text=True,
            check=False,
        )

    def change(self, skill: str, old: str, new: str, message: str = "") -> None:
        """Change a line of a skill, sync, and commit."""
        path = self.dir / "skills" / skill / "SKILL.md"
        path.write_text(path.read_text().replace(f"\n{old}\n", f"\n{new}\n"))
        assert self.sync().returncode == 0
        git(self.dir, "commit", "-q", "-am", message or f"{skill}: {new}")

    def checks_at_each_commit(self, since: str) -> None:
        """Each commit after `since` passes `bin/skills sync --check`."""
        commits = git(self.dir, "rev-list", "--reverse", f"{since}..HEAD").stdout
        assert commits.split()
        for commit in commits.split():
            git(self.dir, "checkout", "-q", "--detach", commit)
            done = self.sync("--check")
            assert done.returncode == 0, (commit, done.stdout, done.stderr)

    def unmerged(self) -> list[str]:
        return git(self.dir, "diff", "--name-only", "--diff-filter=U").stdout.split()


@pytest.fixture
def clone(tmp_path):
    return Clone(tmp_path)


def fold_a_fixup(clone: Clone, *config: str) -> tuple[str, subprocess.CompletedProcess]:
    """Two commits that change the alpha skill, and a fixup of the first, which
    also changes it, folded in as `rebase --autosquash` does: the case #140 met.
    Returns the commit they start from, and the rebase."""
    start = git(clone.dir, "rev-parse", "HEAD").stdout.strip()
    clone.change("alpha", "one", "one A", "A")
    clone.change("alpha", "three", "three B", "B")
    clone.change("alpha", "two", "two F", "fixup! A")
    rebase = ("rebase", "-q", "-i", "--autosquash", start)
    return start, git(clone.dir, *config, *rebase, check=False)


def test_without_the_driver_folding_a_fixup_conflicts_on_the_record(clone):
    _, done = fold_a_fixup(clone, *NO_DRIVER)
    assert done.returncode != 0
    assert clone.unmerged() == ["skills/versions.txt"]


def folds_cleanly(clone: Clone) -> None:
    start, done = fold_a_fixup(clone)
    assert done.returncode == 0, done.stdout + done.stderr
    subjects = git(clone.dir, "log", "--format=%s", f"{start}..HEAD").stdout
    assert subjects.split() == ["B", "A"]
    assert "two F" in (clone.dir / "skills/alpha/SKILL.md").read_text()
    clone.checks_at_each_commit(start)


def test_folding_a_fixup_recomputes_the_record_at_each_commit(clone):
    folds_cleanly(clone)


def test_folding_a_fixup_in_a_linked_worktree_recomputes_the_record(clone, tmp_path):
    # Every rebase wt-ship makes runs in one, sharing the clone's config.
    tree = tmp_path / "repo-wt-topic"
    git(clone.dir, "worktree", "add", "-q", "--detach", str(tree), "main")
    clone.dir = tree
    folds_cleanly(clone)


def branches_that_change_each_skill(clone: Clone) -> None:
    """Branch `a` changes the alpha skill and `b` the beta one, whose lines in the
    record are adjacent, so git's own merge conflicts there."""
    git(clone.dir, "checkout", "-q", "-b", "a")
    clone.change("alpha", "one", "one a")
    git(clone.dir, "checkout", "-q", "-b", "b", "main")
    clone.change("beta", "one", "one b")


def test_a_branch_rebased_on_one_that_changed_another_skill_needs_no_resolving(clone):
    branches_that_change_each_skill(clone)
    assert git(clone.dir, *NO_DRIVER, "rebase", "-q", "a", check=False).returncode
    git(clone.dir, "rebase", "--abort")
    done = git(clone.dir, "rebase", "-q", "a", check=False)
    assert done.returncode == 0, done.stdout + done.stderr
    clone.checks_at_each_commit("main")


def branches_that_change_one_skill(clone: Clone) -> None:
    """Branches `a` and `b` that change different lines of the alpha skill."""
    git(clone.dir, "checkout", "-q", "-b", "a")
    clone.change("alpha", "one", "one a")
    git(clone.dir, "checkout", "-q", "-b", "b", "main")
    clone.change("alpha", "three", "three b")


def test_a_merge_from_a_subfolder_recomputes_a_skill_both_sides_changed(clone):
    branches_that_change_one_skill(clone)
    done = git(clone.dir / "skills/alpha", "merge", "-q", "--no-edit", "a", check=False)
    assert done.returncode == 0, done.stdout + done.stderr
    assert clone.sync("--check").returncode == 0


def test_a_cherry_pick_recomputes_a_skill_both_sides_changed(clone):
    branches_that_change_one_skill(clone)
    done = git(clone.dir, "cherry-pick", "a", check=False)
    assert done.returncode == 0, done.stdout + done.stderr
    assert clone.sync("--check").returncode == 0


def test_a_bin_skills_that_fails_leaves_git_merge_files_conflict(clone):
    branches_that_change_one_skill(clone)
    script = clone.dir / "bin/skills"
    script.write_text("#!/bin/sh\nexit 2\n")
    done = git(clone.dir, "merge", "-q", "--no-edit", "a", check=False)
    assert done.returncode != 0
    assert clone.unmerged() == ["skills/versions.txt"]
    record = (clone.dir / "skills/versions.txt").read_text()
    assert "<<<<<<< HEAD" in record
    assert ">>>>>>> a" in record


def test_a_rebase_where_a_skill_file_conflicts_leaves_the_record_to_resolve(clone):
    git(clone.dir, "checkout", "-q", "-b", "a")
    clone.change("alpha", "one", "one a")
    git(clone.dir, "checkout", "-q", "-b", "b", "main")
    clone.change("alpha", "one", "one b")
    done = git(clone.dir, "rebase", "-q", "a", check=False)
    assert done.returncode != 0
    assert sorted(clone.unmerged()) == ["skills/alpha/SKILL.md", "skills/versions.txt"]
    assert "<<<<<<<" in (clone.dir / "skills/versions.txt").read_text()


def merge_by_hand(clone: Clone, base: str, ours: str, theirs: str):
    """`bin/skills merge` on three records, with labels that name no commit, as a
    revert's or a criss-cross merge's may; returns the run and OURS after it."""
    paths = []
    for name, text in (("base", base), ("ours", ours), ("theirs", theirs)):
        path = clone.dir.parent / name
        path.write_text(text)
        paths.append(str(path))
    done = subprocess.run(
        [str(clone.dir / "bin/skills"), "merge", *paths, "no such", "commit"],
        cwd=clone.dir,
        capture_output=True,
        text=True,
        check=False,
    )
    return done, Path(paths[1]).read_text()


RECORD = "# header\nalpha {a} {ha}\nbeta {b} {hb}\n"


def test_without_commits_it_takes_each_skills_line_from_the_side_that_changed_it(
    clone,
):
    base = RECORD.format(a=1, ha="x", b=1, hb="y")
    ours = RECORD.format(a=2, ha="x2", b=1, hb="y")
    theirs = RECORD.format(a=1, ha="x", b=3, hb="y3")
    done, merged = merge_by_hand(clone, base, ours, theirs)
    assert done.returncode == 0, done.stderr
    assert merged.splitlines()[-2:] == ["alpha 2 x2", "beta 3 y3"]


def test_without_commits_a_skill_both_sides_changed_is_left_as_ours(clone):
    base = RECORD.format(a=1, ha="x", b=1, hb="y")
    ours = RECORD.format(a=2, ha="x2", b=1, hb="y")
    theirs = RECORD.format(a=3, ha="x3", b=1, hb="y")
    done, merged = merge_by_hand(clone, base, ours, theirs)
    assert done.returncode == 1
    assert merged == ours
