"""The autouse fixtures of conftest.py, which every bin test runs under."""

import fcntl
import os
import subprocess
from pathlib import Path

import pytest


def test_each_test_gets_an_empty_tmpdir_under_pytests_base_temp(tmp_path_factory):
    tmpdir = Path(os.environ["TMPDIR"])
    assert tmpdir.is_relative_to(tmp_path_factory.getbasetemp())
    assert list(tmpdir.iterdir()) == []
    seen = subprocess.run(
        ["sh", "-c", 'printf %s "$TMPDIR"'], capture_output=True, text=True, check=True
    )
    assert seen.stdout == str(tmpdir)


@pytest.mark.parametrize("case", ["one", "two"])
def test_no_two_tests_share_a_tmpdir(case, tmp_path_factory):
    # A record every xdist worker of this run shares: their base temps are siblings.
    run = tmp_path_factory.getbasetemp()
    if os.environ.get("PYTEST_XDIST_WORKER"):
        run = run.parent
    mine = os.environ["TMPDIR"]
    with (run / "tmpdirs-seen").open("a+") as record:
        fcntl.flock(record, fcntl.LOCK_EX)
        record.seek(0)
        assert mine not in record.read().splitlines()
        record.write(mine + "\n")
