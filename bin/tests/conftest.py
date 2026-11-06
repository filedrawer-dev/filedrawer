"""Fixtures every test in bin/tests gets.

No test opens a browser: the wizard's open_url calls the first of wslview,
explorer.exe, xdg-open or open on PATH, and on 2026-11-06 wizard tests opened tabs on
the owner's machine. Every test runs with no-op fakes of all four first on PATH, so
a test that builds its subprocess's environment from os.environ, as they do, can't
reach a real one. Each fake notes the URL in NO_BROWSER_DIR/opened. A test may still
put its own recording fakes ahead of these.
"""

import os

import pytest

OPENERS = ("wslview", "explorer.exe", "xdg-open", "open")


@pytest.fixture(autouse=True)
def no_browser(tmp_path_factory, monkeypatch):
    fakes = tmp_path_factory.mktemp("no-browser")
    for opener in OPENERS:
        fake = fakes / opener
        fake.write_text(f'#!/bin/sh\necho "$1" >> "{fakes}/opened"\n')
        fake.chmod(0o755)
    monkeypatch.setenv("PATH", f"{fakes}{os.pathsep}{os.environ['PATH']}")
    monkeypatch.setenv("NO_BROWSER_DIR", str(fakes))
