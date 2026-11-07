"""bin/agent-pre-commit runs these tests with a timeout, so a test that hangs fails
instead of hanging the run."""

import re
import shlex
import subprocess
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]


def test_the_hooks_pytest_fails_a_test_that_hangs(tmp_path):
    hook = (REPO / "bin/agent-pre-commit").read_text()
    line = next(
        line for line in hook.splitlines() if re.match(r"\s*uvx .*pytest", line)
    )
    argv = shlex.split(line.split(">")[0])
    assert argv[-1] == "bin/tests", argv
    tests = tmp_path / "bin/tests"
    tests.mkdir(parents=True)
    ini = (REPO / "bin/tests/pytest.ini").read_text()
    (tests / "pytest.ini").write_text(re.sub(r"timeout = \d+", "timeout = 2", ini))
    (tests / "test_hang.py").write_text(
        "import time\n\ndef test_hang():\n    time.sleep(60)\n"
    )
    done = subprocess.run(
        argv, cwd=tmp_path, capture_output=True, text=True, timeout=50, check=False
    )
    assert done.returncode == 1, done.stdout + done.stderr
    assert "Timeout" in done.stdout, done.stdout
