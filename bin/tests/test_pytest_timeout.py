"""bin/pytest, which bin/agent-pre-commit runs these tests through, runs them in
pytest-xdist's workers, as pytest.ini says, and with a timeout, so a test that hangs
fails instead of hanging the run."""

import re
import subprocess
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]


def test_the_hooks_pytest_runs_workers_and_fails_a_test_that_hangs(tmp_path):
    argv = [str(REPO / "bin/pytest"), "bin/tests"]
    tests = tmp_path / "bin/tests"
    tests.mkdir(parents=True)
    ini = (REPO / "bin/tests/pytest.ini").read_text()
    ini = re.sub(r"timeout = \d+", "timeout = 2", ini)
    # One worker is enough for one test, and up to 8 cost seconds to start.
    ini = re.sub(r"--maxprocesses=\d+", "--maxprocesses=1", ini)
    (tests / "pytest.ini").write_text(ini)
    (tests / "test_hang.py").write_text(
        "import time\n\ndef test_hang():\n    time.sleep(60)\n"
    )
    done = subprocess.run(
        argv, cwd=tmp_path, capture_output=True, text=True, check=False
    )
    assert done.returncode == 1, done.stdout + done.stderr
    assert "Timeout" in done.stdout, done.stdout
    # xdist names the worker that ran a failed test.
    assert re.search(r"^\[gw\d+\] ", done.stdout, re.M), done.stdout
