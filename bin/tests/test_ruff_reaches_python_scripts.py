"""Every Python script in bin/ and ops/ reaches ruff's hooks in .pre-commit-config.yaml.

pre-commit gives a hook the files whose identify tags match its `types`, and identify
reads a shebang's interpreter: `python3` is Python, but `uv run --script` names no
interpreter it knows. Until 2026-11-09 bin/live-preview and bin/field-test-batches went
through every commit unchecked. A script like them goes in the `files` of the
ruff-uv-scripts and ruff-format-uv-scripts hooks.

The test asks pre-commit 4.7.0 itself which files each ruff hook gets, offline from
uv's cache: a scratch repo holds a copy of each script and the config's ruff hooks,
each hook's entry swapped for a fake that notes the files it's given. It is skipped
when uv's cache lacks pre-commit.
"""

import os
import re
import shutil
import subprocess
from pathlib import Path

import pytest
from conftest import hook_blocks, local_config, uv_cached

REPO = Path(__file__).resolve().parents[2]
RUFF_ENTRY = re.compile(r"^(        entry: )uvx ruff@\S+ (check|format)$", re.M)
PYTHON_SHEBANG = re.compile(rb"#!.*\b(python[0-9.]*|uv run)\b")
# ruff-format leaves them alone: they are formatted in llm-toolbox (CODING_STANDARDS.md).
LLM_TOOLBOX_COPIES = {
    "bin/pre-commit-run",
    "bin/slop-lint",
    "bin/tests/leftover_processes.py",
}
FAKE_RUFF = """#!/bin/sh
command=$1
shift
for file in "$@"; do echo "$command $file"; done >> "$LOG"
"""


def python_scripts(*folders: Path) -> list[Path]:
    """The Python files under `folders`: each named *.py, or whose shebang runs python
    or `uv run`. Dotfiles, such as ops/.screen-test.env and its secrets, go unread."""
    found = []
    for folder in folders:
        for path in sorted(folder.rglob("*")):
            if not path.is_file() or path.name.startswith("."):
                continue
            with path.open("rb") as file:
                first = file.readline(200)
            if path.suffix == ".py" or PYTHON_SHEBANG.match(first):
                found.append(path)
    return found


def ruff_hooks(fake: Path) -> str:
    """The config's hooks that run ruff, as a config of their own, each entry `fake`
    followed by the ruff command it stands for."""
    hooks = hook_blocks((REPO / ".pre-commit-config.yaml").read_text())
    ruff = {hook: block for hook, block in hooks.items() if "ruff@" in block}
    unread = [hook for hook, block in ruff.items() if not RUFF_ENTRY.search(block)]
    assert not unread, f"RUFF_ENTRY doesn't read the entries of {unread}"
    return local_config(
        RUFF_ENTRY.sub(rf"\g<1>{fake} \g<2>", block) for block in ruff.values()
    )


def test_python_scripts_are_found_by_name_or_shebang(tmp_path):
    scripts = {
        "uv-tool": "#!/usr/bin/env -S uv run --quiet --script\n",
        "plain-tool": "#!/usr/bin/env python3\n",
        "pinned-tool": "#!/usr/bin/python3.12\n",
        "module.py": "import os\n",
        "shell-tool": "#!/usr/bin/env bash\n",
        "notes.md": "# python\n",
        ".screen-test.env": "#!/usr/bin/env python3\n",
        "tests/test_tool.py": "",
    }
    for name, text in scripts.items():
        (tmp_path / name).parent.mkdir(parents=True, exist_ok=True)
        (tmp_path / name).write_text(text)
    found = [path.relative_to(tmp_path).as_posix() for path in python_scripts(tmp_path)]
    assert found == [
        "module.py",
        "pinned-tool",
        "plain-tool",
        "tests/test_tool.py",
        "uv-tool",
    ]


def test_every_python_script_reaches_ruff_check_and_ruff_format(tmp_path):
    if not uv_cached("pre-commit@4.7.0", "--version"):
        pytest.skip("needs pre-commit 4.7.0 in uv's cache")
    assert (REPO / "ops").is_dir(), "ops/ moved; point the finder at its new place"
    found = python_scripts(REPO / "bin", REPO / "ops")
    assert Path(__file__).resolve() in found, "the finder reads bin/"
    scratch = tmp_path / "repo"
    scripts = []
    for path in found:
        script = path.relative_to(REPO).as_posix()
        (scratch / script).parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(path, scratch / script)
        scripts.append(script)
    fake = tmp_path / "fake-ruff"
    fake.write_text(FAKE_RUFF)
    fake.chmod(0o755)
    (scratch / ".pre-commit-config.yaml").write_text(ruff_hooks(fake))
    subprocess.run(["git", "init", "-q", str(scratch)], check=True)
    log = tmp_path / "log"
    log.touch()
    env = {k: v for k, v in os.environ.items() if not k.startswith("GIT_")}
    env.update(
        LOG=str(log), PRE_COMMIT_HOME=str(tmp_path / "pre-commit"), UV_OFFLINE="1"
    )
    done = subprocess.run(
        ["uvx", "-q", "pre-commit@4.7.0", "run", "--files", *scripts],
        cwd=scratch,
        env=env,
        capture_output=True,
        text=True,
        check=False,
    )
    assert done.returncode == 0, done.stdout + done.stderr
    reached = set(log.read_text().splitlines())
    missed = [
        f"{command} {script}"
        for script in scripts
        for command in ("check", "format")
        if f"{command} {script}" not in reached
        and not (command == "format" and script in LLM_TOOLBOX_COPIES)
    ]
    assert not missed, (
        "ruff's hooks in .pre-commit-config.yaml never see these Python scripts; add"
        " each to the files of ruff-uv-scripts and ruff-format-uv-scripts: "
        + ", ".join(missed)
    )
