"""Tests for the shellcheck hook in .pre-commit-config.yaml: which files it checks and
at what severity, run by pre-commit 4.7.0 offline from uv's cache in a scratch repo
whose config holds this repo's hook; and that pre-commit counts each of this repo's
shell scripts as one, so the hook checks it. Skipped when uv's cache lacks pre-commit
4.7.0 or the shellcheck-py the hook pins.
"""

import os
import re
import shutil
import subprocess
from pathlib import Path

import pytest

REPO = Path(__file__).resolve().parents[2]
PRE_COMMIT = "pre-commit==4.7.0"
# A first line that runs one of the shells shellcheck reads.
SHELL_SHEBANG = re.compile(rb"#!.*\b(sh|bash|dash|ksh)\b")


def hook_block() -> str:
    config = (REPO / ".pre-commit-config.yaml").read_text()
    return re.search(r"^      - id: shellcheck\n(?:        .*\n)+", config, re.M).group(
        0
    )


def cached(*argv: str) -> bool:
    if shutil.which("uvx") is None:
        return False
    done = subprocess.run(
        ["uvx", "--offline", "-q", *argv], capture_output=True, check=False
    )
    return done.returncode == 0


def needs_the_cache() -> None:
    pin = re.search(r"shellcheck-py==[\w.]+", hook_block()).group(0)
    if not (
        cached("--from", PRE_COMMIT, "pre-commit", "--version")
        and cached("--from", pin, "shellcheck", "--version")
    ):
        pytest.skip(f"needs {PRE_COMMIT} and {pin} in uv's cache")


def write(path: Path, text: str, executable: bool = False) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text)
    if executable:
        path.chmod(0o755)  # pre-commit reads the shebang of an executable only


def test_it_checks_the_shell_scripts_in_bin_and_ops_down_to_infos(tmp_path):
    needs_the_cache()
    root = tmp_path / "repo"
    hooks = "repos:\n  - repo: local\n    hooks:\n" + hook_block()
    write(root / ".pre-commit-config.yaml", hooks)
    # A warning, in a script found by its shebang.
    write(root / "bin/unused", "#!/usr/bin/env bash\nx=1\n", executable=True)
    # An info, in a script found by its name.
    write(root / "ops/remote/unquoted.sh", "#!/usr/bin/env bash\necho $1\n")
    # A style nit only, which passes.
    write(root / "ops/backticks", '#!/bin/sh\nx=`date`\necho "$x"\n', executable=True)
    # Not checked: outside bin/ and ops/, zsh, which shellcheck can't read, and Python.
    write(root / "docs/unused.sh", "#!/usr/bin/env bash\nx=1\n")
    write(root / "bin/zsh-tool", "#!/usr/bin/env zsh\nx=1\n", executable=True)
    write(root / "bin/tool.py", "#!/usr/bin/env python3\nx = 1\n", executable=True)
    # A .shellcheckrc that would silence everything is ignored.
    write(root / ".shellcheckrc", "disable=all\n")
    subprocess.run(["git", "init", "-q", str(root)], check=True)
    subprocess.run(["git", "-C", str(root), "add", "."], check=True)
    env = {k: v for k, v in os.environ.items() if not k.startswith("GIT_")}
    env["PRE_COMMIT_HOME"] = str(tmp_path / "pre-commit-home")
    env["UV_OFFLINE"] = "1"
    done = subprocess.run(
        ["uvx", "-q", "--from", PRE_COMMIT, "pre-commit", "run", "--all-files"],
        cwd=root,
        env=env,
        capture_output=True,
        text=True,
        check=False,
    )
    shown = done.stdout + done.stderr
    assert done.returncode == 1, shown
    assert re.search(r"In bin/unused line 2:\n.*\n.*SC2034 \(warning\)", shown), shown
    assert re.search(
        r"In ops/remote/unquoted.sh line 2:\n.*\n.*SC2086 \(info\)", shown
    ), shown
    for unchecked in ("ops/backticks", "docs/unused.sh", "bin/zsh-tool", "tool.py"):
        assert unchecked not in shown, shown


def test_pre_commit_counts_every_shell_script_here_as_one():
    needs_the_cache()
    scripts = [
        path
        for folder in (REPO / "bin", REPO / "ops")
        for path in sorted(folder.rglob("*"))
        if path.is_file() and SHELL_SHEBANG.match(path.read_bytes().split(b"\n")[0])
    ]
    assert REPO / "ops/smoke-staging" in scripts, scripts
    # pre-commit's own identify, as the hook's `types: [shell]` uses it.
    done = subprocess.run(
        [
            "uvx",
            "--offline",
            "-q",
            "--from",
            PRE_COMMIT,
            "python",
            "-c",
            "import sys\n"
            "from identify.identify import tags_from_path\n"
            "for path in sys.argv[1:]:\n"
            "    if 'shell' not in tags_from_path(path):\n"
            "        print(path)\n",
            *map(str, scripts),
        ],
        capture_output=True,
        text=True,
        check=False,
    )
    assert done.returncode == 0, done.stderr
    # Each needs its exec bit and a shebang pre-commit reads (`#!/bin/bash` or
    # `#!/usr/bin/env bash`, not `#!/bin/env bash`), or a name ending .sh.
    assert done.stdout == "", done.stdout
