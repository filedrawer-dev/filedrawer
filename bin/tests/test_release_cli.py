"""Tests for ops/release-cli, the owner's command that makes a whole release of the
`filedrawer` command: bump, commit, push, CI's check, the release workflow, and
the registries.

Each runs it in a made-up repo with the two crates' Cargo.tomls and Cargo.lock,
whose `origin` is a bare repo beside it, so a push lands in a temp folder. gh,
cargo and the repo's bin/release-plan are fakes answering from a JSON state: gh's
runs, jobs, dispatch and watch; release-plan's registries (`published`,
`analyzer`, `plan`) and CI (`ci`). A state value that is a list is answered one
item a call, the last one again once the rest are used ([status, stdout] is one answer). No test pushes to GitHub,
dispatches or publishes.
"""

import json
import os
import subprocess
import sys
from pathlib import Path

import pytest

SCRIPT = Path(__file__).resolve().parents[2] / "ops/release-cli"
SLUG = "filedrawer-dev/filedrawer"

FAKE = r'''
"""gh, cargo or bin/release-plan, by the name it was called with, against $FAKE_STATE."""
import json, os, subprocess, sys
from pathlib import Path

state_file = Path(os.environ["FAKE_STATE"])
tool, args = Path(sys.argv[0]).name, sys.argv[1:]
with (state_file.parent / "log").open("a") as log:
    log.write(" ".join([tool, *args]) + "\n")
state = json.loads(state_file.read_text())


def take(key):
    """The state's KEY, one item a call when it is a list of them (a list that
    starts with a number is one [status, stdout] answer)."""
    value = state[key]
    if isinstance(value, list) and not isinstance(value[0], int):
        counts = state.setdefault("_taken", {})
        n = counts.get(key, 0)
        counts[key] = n + 1
        state_file.write_text(json.dumps(state))
        return value[min(n, len(value) - 1)]
    return value


def answer(reply):
    """[status, stdout]"""
    sys.stdout.write(reply[1])
    sys.exit(reply[0])


def head():
    return subprocess.run(["git", "rev-parse", "HEAD"], capture_output=True, text=True).stdout.strip()


if tool == "cargo":
    answer(take("cargo"))
if tool == "release-plan":
    command = args[0]
    if command == "ci":
        if state.get("on_ci"):
            subprocess.run(state["on_ci"], shell=True, check=True)
        answer([0, f"state={take('ci')}\nurl=https://ci/run\n"])
    answer(take(command))
if tool == "gh":
    if args[:2] == ["auth", "status"]:
        answer(take("auth"))
    if args[:2] == ["workflow", "run"]:
        if state.get("dispatch_fails"):
            answer([1, ""])
        run_id = 100 + len(state["runs"])
        state["runs"].append({
            "id": run_id, "head_sha": head(), "event": "workflow_dispatch",
            "head_branch": "main", "html_url": f"https://run/{run_id}",
            "status": "completed", "conclusion": take("dispatch_conclusion"),
        })
        state_file.write_text(json.dumps(state))
        answer([0, ""])
    if args[:2] == ["run", "watch"]:
        # A run still going ends after its `watches` watches, with `final`; a watch
        # that ends before the run does exits 1, as one cut off by the network.
        run = next(r for r in state["runs"] if str(r["id"]) == args[2])
        if run["status"] != "completed":
            run["watches"] -= 1
            if run["watches"] == 0:
                run["status"], run["conclusion"] = "completed", run["final"]
            state_file.write_text(json.dumps(state))
        answer([0 if run["conclusion"] == "success" else 1, ""])
    if args[0] == "api":
        path = args[1]
        if "/actions/workflows/release-cli.yml/runs?" in path:
            sha = path.split("head_sha=")[1].split("&")[0]
            runs = [r for r in state["runs"] if r["head_sha"] == sha]
            answer([0, json.dumps({"workflow_runs": runs})])
        if "/actions/runs/" in path:
            run = next(r for r in state["runs"] if path.endswith(f"/{r['id']}"))
            answer([0, json.dumps(run)])
    sys.exit(9)
'''


class Box:
    def __init__(self, root: Path, *, cli: str = "0.1.1", analyzer: str = "0.1.1"):
        self.root = root
        self.repo = root / "repo"
        self.origin = root / "origin.git"
        self.state_file = root / "state.json"
        fakes = root / "fakes"
        fakes.mkdir()
        for tool in ("gh", "cargo"):
            self.fake(fakes / tool)
        self.env = {
            **os.environ,
            "PATH": f"{fakes}{os.pathsep}{os.environ['PATH']}",
            "FAKE_STATE": str(self.state_file),
            "RELEASE_CLI_POLL_SECONDS": "0",
            "GIT_AUTHOR_NAME": "Owner",
            "GIT_AUTHOR_EMAIL": "owner@example.com",
            "GIT_COMMITTER_NAME": "Owner",
            "GIT_COMMITTER_EMAIL": "owner@example.com",
        }
        self.env.pop("AI_AGENT", None)
        self.state = {
            "auth": [0, ""],
            "cargo": [0, ""],
            "published": [0, ""],
            "analyzer": [0, "changed=false\n"],
            "plan": [0, "crates_cli=true\n"],
            "ci": "passed",
            "dispatch_conclusion": "success",
            "runs": [],
        }
        self.save()
        subprocess.run(
            ["git", "init", "-q", "--bare", "-b", "main", str(self.origin)], check=True
        )
        self.git("init", "-q", "-b", "main", str(self.repo), cwd=root)
        (self.repo / "crates/cli").mkdir(parents=True)
        (self.repo / "crates/analyzer").mkdir(parents=True)
        (self.repo / "bin").mkdir()
        self.fake(self.repo / "bin/release-plan")
        (self.repo / "crates/cli/Cargo.toml").write_text(
            f'[package]\nname = "filedrawer"\nversion = "{cli}"\nedition.workspace = true\n\n'
            f'[dependencies]\nfiledrawer-analyzer = {{ path = "../analyzer", version = "={analyzer}" }}\n'
        )
        (self.repo / "crates/analyzer/Cargo.toml").write_text(
            f'[package]\nname = "filedrawer-analyzer"\nversion = "{analyzer}"\n'
        )
        (self.repo / "Cargo.lock").write_text(
            f'[[package]]\nname = "filedrawer"\nversion = "{cli}"\n\n'
            f'[[package]]\nname = "filedrawer-analyzer"\nversion = "{analyzer}"\n\n'
            '[[package]]\nname = "zip"\nversion = "9.5.1"\n'
        )
        self.git("add", "-A")
        self.git("commit", "-q", "-m", f"Release filedrawer {cli}")
        self.git("remote", "add", "origin", str(self.origin))
        self.git("push", "-q", "origin", "main")
        self.git("branch", "-q", "--set-upstream-to=origin/main")

    def fake(self, path: Path) -> None:
        path.write_text(f"#!{sys.executable} -IS\n{FAKE}")
        path.chmod(0o755)

    def git(self, *args: str, cwd: Path | None = None) -> str:
        return subprocess.run(
            ["git", *args],
            cwd=cwd or self.repo,
            env=getattr(self, "env", os.environ),
            capture_output=True,
            text=True,
            check=True,
        ).stdout.strip()

    def save(self) -> None:
        self.state_file.write_text(json.dumps(self.state))

    def set(self, **values) -> None:
        self.state = json.loads(self.state_file.read_text())
        self.state.update(values)
        self.state.pop("_taken", None)
        self.save()

    def run(self, *args: str, answer: str = "y\n"):
        return subprocess.run(
            [str(SCRIPT), *args],
            cwd=self.repo,
            env=self.env,
            input=answer,
            capture_output=True,
            text=True,
            check=False,
        )

    def calls(self, tool: str) -> list[str]:
        log = self.root / "log"
        lines = log.read_text().splitlines() if log.exists() else []
        return [line[len(tool) + 1 :] for line in lines if line.startswith(f"{tool} ")]

    def dispatches(self) -> list[str]:
        return [c for c in self.calls("gh") if c.startswith("workflow run")]

    def version(self, crate: str) -> str:
        text = (self.repo / f"crates/{crate}/Cargo.toml").read_text()
        return text.split('version = "')[1].split('"')[0]

    def origin_main(self) -> str:
        return self.git("rev-parse", "main", cwd=self.origin)

    def commit(self, path: str, text: str, message: str) -> None:
        (self.repo / path).parent.mkdir(parents=True, exist_ok=True)
        (self.repo / path).write_text(text)
        self.git("add", "-A")
        self.git("commit", "-q", "-m", message)


@pytest.fixture
def box(tmp_path):
    b = Box(tmp_path)
    # A change to the CLI since 0.1.1, which every registry has.
    b.commit("crates/cli/src/main.rs", "fn main() {}\n", "The CLI does more")
    b.git("push", "-q", "origin", "main")
    return b


def test_a_release_bumps_commits_pushes_waits_dispatches_and_confirms(box):
    done = box.run()
    assert done.returncode == 0, done.stdout + done.stderr
    assert box.version("cli") == "0.1.2"
    assert box.version("analyzer") == "0.1.1"
    assert box.git("log", "-1", "--format=%s") == "Release filedrawer 0.1.2"
    assert box.origin_main() == box.git("rev-parse", "HEAD")
    lock = (box.repo / "Cargo.lock").read_text()
    assert 'name = "filedrawer"\nversion = "0.1.2"' in lock
    assert 'name = "zip"\nversion = "9.5.1"' in lock
    assert box.dispatches() == [
        f"workflow run release-cli.yml --repo {SLUG} --ref main -f version=0.1.2"
    ]
    assert "plan --version 0.1.2" in box.calls("release-plan")
    # Not --offline: the owner's checkout may never have fetched the crates.
    assert box.calls("cargo") == ["metadata --locked --format-version 1"]
    assert "deploys staging" in done.stdout
    assert "0.1.2" in done.stdout.splitlines()[-1]


@pytest.mark.parametrize(
    ("level", "cli"), [("patch", "0.1.2"), ("minor", "0.2.0"), ("major", "1.0.0")]
)
def test_each_level_bumps_the_cli_and_a_changed_analyzer_alike(box, level, cli):
    box.set(analyzer=[0, "changed=true\n"])
    done = box.run(level)
    assert done.returncode == 0, done.stdout + done.stderr
    analyzer = {"patch": "0.1.2", "minor": "0.2.0", "major": "1.0.0"}[level]
    assert (box.version("cli"), box.version("analyzer")) == (cli, analyzer)
    manifest = (box.repo / "crates/cli/Cargo.toml").read_text()
    assert f'version = "={analyzer}"' in manifest
    assert box.git("log", "-1", "--format=%s") == (
        f"Release filedrawer {cli} and filedrawer-analyzer {analyzer}"
    )
    lock = (box.repo / "Cargo.lock").read_text()
    assert f'name = "filedrawer-analyzer"\nversion = "{analyzer}"' in lock


def test_a_no_to_the_push_drops_the_commit_it_made(box):
    head = box.git("rev-parse", "HEAD")
    done = box.run(answer="n\n")
    assert done.returncode == 1
    assert box.git("rev-parse", "HEAD") == head
    assert box.version("cli") == "0.1.1"
    assert box.git("status", "--porcelain") == ""
    assert box.dispatches() == []


MISSING = [1, "npm @filedrawer/cli 0.1.2\n"]


def test_a_rerun_after_an_earlier_runs_commit_pushes_it_without_bumping_again(box):
    box.set(published=[MISSING, [0, ""]])
    box.commit(
        "crates/cli/Cargo.toml",
        (box.repo / "crates/cli/Cargo.toml")
        .read_text()
        .replace('version = "0.1.1"', 'version = "0.1.2"', 1),
        "Release filedrawer 0.1.2",
    )
    head = box.git("rev-parse", "HEAD")
    done = box.run()
    assert done.returncode == 0, done.stdout + done.stderr
    assert box.git("rev-parse", "HEAD") == head == box.origin_main()
    assert len(box.dispatches()) == 1


def test_a_no_to_an_earlier_runs_commit_keeps_it(box):
    box.set(published=MISSING)
    box.commit("crates/cli/Cargo.toml", '[package]\nversion = "0.1.2"\n', "Release")
    head = box.git("rev-parse", "HEAD")
    assert box.run(answer="n\n").returncode == 1
    assert box.git("rev-parse", "HEAD") == head


def bumped_and_pushed(box: Box, published=(MISSING, [0, ""])) -> None:
    box.set(published=list(published))
    box.commit(
        "crates/cli/Cargo.toml",
        (box.repo / "crates/cli/Cargo.toml")
        .read_text()
        .replace('version = "0.1.1"', 'version = "0.1.2"', 1),
        "Release filedrawer 0.1.2",
    )
    box.git("push", "-q", "origin", "main")


def test_a_rerun_once_pushed_waits_for_check_then_dispatches(box):
    bumped_and_pushed(box)
    box.set(ci=["absent", "pending", "passed"])
    done = box.run(answer="")
    assert done.returncode == 0, done.stdout + done.stderr
    assert box.calls("release-plan").count(f"ci --repo {SLUG} {box.origin_main()}") == 3
    assert len(box.dispatches()) == 1


def test_a_failed_check_stops_with_the_runs_page(box):
    bumped_and_pushed(box)
    box.set(ci="failed")
    done = box.run()
    assert done.returncode == 1
    assert "https://ci/run" in done.stdout + done.stderr
    assert box.dispatches() == []


def test_a_release_run_in_progress_is_watched_not_dispatched_again(box):
    bumped_and_pushed(box)
    head = box.origin_main()
    box.set(
        runs=[
            {
                "id": 50,
                "head_sha": head,
                "event": "workflow_dispatch",
                "head_branch": "main",
                "html_url": "https://run/50",
                "status": "in_progress",
                "conclusion": None,
                "watches": 2,
                "final": "success",
            }
        ]
    )
    done = box.run()
    assert done.returncode == 0, done.stdout + done.stderr
    assert box.dispatches() == []
    assert [c for c in box.calls("gh") if c.startswith("run watch")] == [
        f"run watch 50 --repo {SLUG} --exit-status -i 10"
    ] * 2


def test_a_run_still_going_after_its_watches_stops_saying_so(box):
    bumped_and_pushed(box)
    run = {
        "id": 50,
        "head_sha": box.origin_main(),
        "event": "workflow_dispatch",
        "head_branch": "main",
        "html_url": "https://run/50",
        "status": "in_progress",
        "conclusion": None,
        "watches": 99,
        "final": "success",
    }
    box.set(runs=[run])
    done = box.run()
    assert done.returncode == 1
    assert "in_progress" in done.stderr and "https://run/50" in done.stderr


def test_a_failed_release_run_stops_with_its_page_and_a_rerun_dispatches_again(box):
    bumped_and_pushed(box, published=[MISSING, MISSING, [0, ""]])
    box.set(dispatch_conclusion=["failure", "success"])
    done = box.run()
    assert done.returncode == 1
    assert "https://run/100" in done.stdout + done.stderr
    assert box.run().returncode == 0
    assert len(box.dispatches()) == 2


def test_the_registries_are_asked_again_until_they_list_the_version(box):
    """npm answered 404 for some minutes after its job passed, on 2026-11-21."""
    box.set(
        published=[
            [0, ""],
            [1, "npm @filedrawer/cli 0.1.2\n"],
            [3, "::error::registry.npmjs.org answered 503\n"],
            [0, ""],
        ]
    )
    done = box.run()
    assert done.returncode == 0, done.stdout + done.stderr
    assert box.calls("release-plan").count("published") == 4


def test_registries_that_never_list_it_stop_with_a_rerun_later(box):
    box.set(published=[[0, ""], [1, "npm @filedrawer/cli 0.1.2\n"]])
    done = box.run()
    assert done.returncode == 1
    out = done.stdout + done.stderr
    assert "npm @filedrawer/cli 0.1.2" in out and "rerun" in out


def test_a_version_out_everywhere_with_nothing_shipped_since_is_nothing_to_do(tmp_path):
    """A rerun after the laptop slept through the last step."""
    box = Box(tmp_path)
    box.commit("docs/x.md", "docs\n", "Docs only")
    done = box.run()
    assert done.returncode == 0, done.stdout + done.stderr
    assert "nothing to do" in done.stdout
    assert box.version("cli") == "0.1.1"
    assert box.dispatches() == []


def test_a_partly_released_version_with_shipped_changes_since_is_refused(box):
    """Finishing it would publish one version from two commits."""
    bumped_and_pushed(box)
    box.commit("crates/cli/src/main.rs", "fn main() { more() }\n", "Even more")
    done = box.run()
    assert done.returncode == 1
    assert "two commits" in done.stderr
    assert box.dispatches() == []


def test_a_registry_that_cant_be_told_stops_before_any_bump(box):
    box.set(published=[3, "::error::pypi.org answered 503\n"])
    done = box.run()
    assert done.returncode == 1
    assert box.version("cli") == "0.1.1"


@pytest.mark.parametrize("step", ["plan", "cargo"])
def test_a_bump_the_checks_refuse_is_put_back(box, step):
    box.set(**{step: [1, "::error::no\n"]})
    head = box.git("rev-parse", "HEAD")
    done = box.run()
    assert done.returncode == 1
    assert box.git("rev-parse", "HEAD") == head
    assert box.git("status", "--porcelain") == ""


def test_main_moving_before_the_dispatch_stops_it(box):
    """--ref main would build the new tip, not the commit check passed on."""
    bumped_and_pushed(box)
    other = box.root / "other"
    box.git("clone", "-q", str(box.origin), str(other), cwd=box.root)
    (other / "y").write_text("y\n")
    box.git("add", "y", cwd=other)
    box.git("commit", "-q", "-m", "y", cwd=other)
    # Someone else's push lands while the tool waits for check.
    box.set(on_ci=f"git -C {other} push -q origin main")
    done = box.run()
    assert done.returncode == 1
    assert "main moved" in done.stderr
    assert box.dispatches() == []


@pytest.mark.parametrize(
    "spoil",
    [
        lambda b: (b.repo / "Cargo.lock").open("a").write("# edited\n"),
        lambda b: b.git("checkout", "-q", "-b", "topic"),
        lambda b: (b.repo / "CLAUDE_WORKING").write_text("held\n"),
        lambda b: b.env.update(AI_AGENT="1"),
        lambda b: b.set(auth=[1, ""]),
        lambda b: b.git("reset", "-q", "--hard", "HEAD~1"),
        lambda b: (
            b.git("reset", "-q", "--hard", "HEAD~1"),
            b.commit("other", "x\n", "A commit origin lacks"),
        ),
        lambda b: b.git("checkout", "-q", "--detach"),
    ],
    ids=[
        "dirty",
        "not-main",
        "landing",
        "agent",
        "gh-logged-out",
        "behind-origin",
        "diverged",
        "detached",
    ],
)
def test_it_refuses_before_doing_anything(box, spoil):
    spoil(box)
    head = box.git("rev-parse", "HEAD")
    done = box.run()
    assert done.returncode == 1, done.stdout
    assert done.stderr.strip()
    assert box.git("rev-parse", "HEAD") == head
    assert box.dispatches() == []


def test_a_commit_the_hooks_refuse_is_put_back(box):
    hook = box.repo / ".git/hooks/pre-commit"
    hook.write_text("#!/bin/sh\necho 'COMMIT REFUSED: no'\nexit 1\n")
    hook.chmod(0o755)
    head = box.git("rev-parse", "HEAD")
    done = box.run()
    assert done.returncode == 1
    assert "refused" in done.stderr
    assert box.git("rev-parse", "HEAD") == head
    assert box.git("status", "--porcelain") == ""


def test_a_release_plan_that_crashes_is_couldnt_tell_not_released(box):
    """Exit 1 with nothing named is a crash, not a version every registry has."""
    box.set(published=[1, ""])
    done = box.run()
    assert done.returncode == 1
    assert box.version("cli") == "0.1.1"
    assert "Released" not in done.stdout and "nothing to do" not in done.stdout


def test_a_pushed_bump_crates_io_lacks_can_take_a_fix_before_its_release(box):
    """check failed on the bump, and a fix landed: nothing of it is published yet,
    so no version would come from two commits."""
    bumped_and_pushed(box, published=[[1, "crates.io filedrawer 0.1.2\n"], [0, ""]])
    box.commit("crates/cli/src/main.rs", "fn main() { fixed() }\n", "The fix")
    box.git("push", "-q", "origin", "main")
    done = box.run()
    assert done.returncode == 0, done.stdout + done.stderr
    assert len(box.dispatches()) == 1


def test_a_change_to_the_workspace_manifest_is_a_change_to_what_ships(tmp_path):
    """Its edition and license go into both packages."""
    box = Box(tmp_path)
    box.commit("Cargo.toml", '[workspace.package]\nedition = "2024"\n', "Edition")
    box.git("push", "-q", "origin", "main")
    done = box.run()
    assert done.returncode == 0, done.stdout + done.stderr
    assert box.version("cli") == "0.1.2"


def test_untracked_files_dont_count_as_dirty(box):
    (box.repo / "next-prompt.txt").write_text("notes\n")
    assert box.run().returncode == 0


def test_another_argument_is_refused(box):
    done = box.run("huge")
    assert done.returncode == 2
    assert "patch" in done.stderr


def test_help_gives_the_steps_and_how_to_release_by_hand(box):
    done = box.run("-h")
    assert done.returncode == 0
    for word in ("patch", "minor", "major", "gh workflow run release-cli.yml"):
        assert word in done.stdout
