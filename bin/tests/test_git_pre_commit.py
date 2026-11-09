"""Tests for the commit hook: that an agent's commit tells agent-commit-gate it runs
for a commit, and the hook's formatting: this repo's formatter hooks write, and
bin/pre-commit-run (a copy of llm-toolbox's) restages an agent's files they rewrote
when each was fully staged. Each runs the real bin/git-pre-commit, bin/pre-commit-run
and pre-commit 4.7.0, offline from uv's cache, in a scratch repo whose config holds
this repo's ruff-format, cargo-fmt and sentence-lines hooks. Skipped when uv's cache
lacks pre-commit or ruff, or cargo is missing.
"""

import os
import re
import shutil
import subprocess
from pathlib import Path

import pytest

REPO = Path(__file__).resolve().parents[2]
UGLY_PY = "x = {'a':1}\n"
TIDY_PY = 'x = {"a": 1}\n'
UGLY_RS = "pub fn a( ){}\n"
TIDY_RS = "pub fn a() {}\n"
UGLY_MD = "One. Two.\n"
TIDY_MD = "One.\nTwo.\n"


def cached(*argv: str) -> bool:
    if shutil.which("uvx") is None:
        return False
    done = subprocess.run(
        ["uvx", "--offline", "-q", *argv], capture_output=True, check=False
    )
    return done.returncode == 0


def formatter_hooks() -> str:
    """This repo's ruff-format, cargo-fmt and sentence-lines hooks, as a config of
    their own."""
    config = (REPO / ".pre-commit-config.yaml").read_text()
    blocks = [
        re.search(rf"^      - id: {hook}\n(?:        .*\n)+", config, re.M).group(0)
        for hook in ("ruff-format", "cargo-fmt", "sentence-lines")
    ]
    return "repos:\n  - repo: local\n    hooks:\n" + "".join(blocks)


def git(repo: Path, *args: str) -> str:
    done = subprocess.run(
        ["git", "-C", str(repo), *args], capture_output=True, text=True, check=True
    )
    return done.stdout


@pytest.fixture
def repo(tmp_path):
    if not (
        cached("pre-commit@4.7.0", "--version") and cached("ruff@0.14.4", "--version")
    ):
        pytest.skip("needs pre-commit 4.7.0 and ruff 0.14.4 in uv's cache")
    if shutil.which("cargo") is None:
        pytest.skip("needs cargo")
    root = tmp_path / "repo"
    (root / "src").mkdir(parents=True)
    (root / "bin").mkdir()
    (root / "Cargo.toml").write_text(
        '[package]\nname = "x"\nversion = "0.1.0"\nedition = "2024"\n'
    )
    (root / "src/lib.rs").write_text(TIDY_RS)
    (root / "tool.py").write_text(TIDY_PY)
    (root / ".pre-commit-config.yaml").write_text(formatter_hooks())
    for script in ("git-pre-commit", "pre-commit-run", "sentence-lines"):
        shutil.copy(REPO / "bin" / script, root / "bin" / script)
    git(root, "init", "-q")
    git(root, "-c", "user.name=t", "-c", "user.email=t@t", "add", ".")
    git(root, "-c", "user.name=t", "-c", "user.email=t@t", "commit", "-qm", "init")
    return root


def hook(
    repo: Path, agent: bool, gate: Path | None = None
) -> subprocess.CompletedProcess:
    """Run the hook as git would. With `gate`, a folder holding a fake agent-commit-gate,
    the agent check's line runs it; without, the line is skipped."""
    env = {k: v for k, v in os.environ.items() if not k.startswith("GIT_")}
    env.pop("AI_AGENT", None)
    if gate is None:
        env["AGENT_PRE_COMMIT"] = "1"  # the slow agent check isn't under test
    else:
        env.pop("AGENT_PRE_COMMIT", None)
        env["PATH"] = f"{gate}{os.pathsep}{env['PATH']}"
    env["PRE_COMMIT_HOME"] = str(repo.parent / "pre-commit-home")
    env["UV_OFFLINE"] = "1"
    if agent:
        env["AI_AGENT"] = "test"
    return subprocess.run(
        [str(repo / "bin/git-pre-commit")],
        cwd=repo,
        env=env,
        capture_output=True,
        text=True,
        check=False,
    )


def stage(repo: Path, name: str, text: str) -> None:
    (repo / name).write_text(text)
    git(repo, "add", name)


def test_an_agents_staged_python_and_rust_are_formatted_and_staged(repo):
    stage(repo, "tool.py", UGLY_PY)
    stage(repo, "src/lib.rs", UGLY_RS)
    done = hook(repo, agent=True)
    assert done.returncode == 0, done.stdout + done.stderr
    assert git(repo, "show", ":tool.py") == TIDY_PY
    assert git(repo, "show", ":src/lib.rs") == TIDY_RS
    assert "staged the result" in done.stderr


def test_a_persons_commit_is_refused_with_the_files_rewritten(repo):
    stage(repo, "tool.py", UGLY_PY)
    stage(repo, "src/lib.rs", UGLY_RS)
    done = hook(repo, agent=False)
    assert done.returncode == 1
    assert (repo / "tool.py").read_text() == TIDY_PY
    assert (repo / "src/lib.rs").read_text() == TIDY_RS
    assert git(repo, "show", ":tool.py") == UGLY_PY, "the person stages it"
    assert git(repo, "show", ":src/lib.rs") == UGLY_RS


def test_an_untracked_file_cargo_fmt_rewrites_refuses_an_agents_commit(repo):
    (repo / "tests").mkdir()
    (repo / "tests/new.rs").write_text("fn t( ){}\n")
    stage(repo, "src/lib.rs", UGLY_RS)
    done = hook(repo, agent=True)
    assert done.returncode == 1
    assert git(repo, "show", ":src/lib.rs") == UGLY_RS


def test_the_copies_from_llm_toolbox_are_left_unformatted(repo):
    copy = repo / "bin/slop-lint"
    copy.write_text("#!/usr/bin/env python3\n" + UGLY_PY)
    copy.chmod(0o755)  # pre-commit reads the shebang of an executable only
    git(repo, "add", "bin/slop-lint")
    done = hook(repo, agent=True)
    assert done.returncode == 0, done.stdout + done.stderr
    assert git(repo, "show", ":bin/slop-lint") == "#!/usr/bin/env python3\n" + UGLY_PY


def test_an_agents_staged_markdown_is_rewritten_and_staged(repo):
    stage(repo, "notes.md", UGLY_MD)
    done = hook(repo, agent=True)
    assert done.returncode == 0, done.stdout + done.stderr
    assert git(repo, "show", ":notes.md") == TIDY_MD


def test_a_persons_markdown_is_refused_with_the_file_rewritten(repo):
    stage(repo, "notes.md", UGLY_MD)
    done = hook(repo, agent=False)
    assert done.returncode == 1
    assert (repo / "notes.md").read_text() == TIDY_MD
    assert git(repo, "show", ":notes.md") == UGLY_MD, "the person stages it"


@pytest.mark.parametrize(
    "entry",
    [
        "crates/x/tests/fixtures/entry.md",
        "ops/screen-eval/public/case.md",
        "ops/smoke-fixture.md",
    ],
)
def test_an_entry_is_left_as_its_author_wrote_it(repo, entry):
    (repo / entry).parent.mkdir(parents=True, exist_ok=True)
    stage(repo, entry, UGLY_MD)
    done = hook(repo, agent=True)
    assert done.returncode == 0, done.stdout + done.stderr
    assert git(repo, "show", f":{entry}") == UGLY_MD


def test_an_agents_file_with_unstaged_changes_is_refused_and_left_staged_as_it_was(
    repo,
):
    stage(repo, "tool.py", UGLY_PY)
    unstaged = UGLY_PY + "y = {'b':2}  # not for this commit\n"
    (repo / "tool.py").write_text(unstaged)
    done = hook(repo, agent=True)
    assert done.returncode == 1
    assert git(repo, "show", ":tool.py") == UGLY_PY
    assert "not for this commit" in (repo / "tool.py").read_text()


def test_an_agents_commit_tells_the_gate_it_runs_for_a_commit(repo, tmp_path):
    """The line sets AGENT_CHECK_AT_COMMIT, so the gate may run agent-commit-check in
    agent-pre-commit's place (llm-toolbox's agent-commit-gate -h)."""
    fake = tmp_path / "fake-bin"
    fake.mkdir()
    seen = tmp_path / "seen"
    gate = fake / "agent-commit-gate"
    gate.write_text(f'#!/bin/sh\necho "${{AGENT_CHECK_AT_COMMIT:-unset}}" > "{seen}"\n')
    gate.chmod(0o755)

    done = hook(repo, agent=True, gate=fake)

    assert done.returncode == 0, done.stderr
    assert seen.read_text().strip() == "1"
