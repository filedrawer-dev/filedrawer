"""bin/tests/conftest.py's guard: no test opens a browser, whether or not it fakes
the openers itself (the owner's machine got tabs from wizard tests on 2026-11-06)."""

import os
import subprocess

from conftest import opened_urls


def test_every_browser_opener_a_test_reaches_is_a_no_op_fake():
    for opener in ("xdg-open", "wslview", "explorer.exe", "open"):
        done = subprocess.run(
            ["bash", "-c", f"command -v {opener}"],
            capture_output=True,
            text=True,
            env=os.environ,
            check=False,
        )
        assert done.returncode == 0, opener
        assert done.stdout.strip().startswith(os.environ["NO_BROWSER_DIR"]), done.stdout
    opened = subprocess.run(
        ["xdg-open", "https://example.com"], capture_output=True, text=True, check=False
    )
    assert opened.returncode == 0
    assert opened_urls().read_text() == "https://example.com\n"
