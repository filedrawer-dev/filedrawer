"""Tests for bin/probe-copy that open no browser: its help, and where it clicks. What
it finds in a browser is the tool's own job, run by hand (its docstring)."""

import importlib.machinery
import importlib.util
import subprocess
from pathlib import Path

import pytest

REPO = Path(__file__).resolve().parents[2]
SCRIPT = REPO / "bin" / "probe-copy"
loader = importlib.machinery.SourceFileLoader("probe_copy", str(SCRIPT))
spec = importlib.util.spec_from_loader("probe_copy", loader)
probe_copy = importlib.util.module_from_spec(spec)
loader.exec_module(probe_copy)


@pytest.mark.parametrize(
    ("slip", "button"),
    [
        # Beside the line, as on a desktop, and under it, as on a phone.
        ([342.4, 410.4, 595.2, 69.5], [817.6, 411.9, 118.5, 66.5]),
        ([32, 532.6, 336, 123.5], [33.5, 620.6, 333, 34]),
    ],
)
def test_every_click_lands_inside_the_slip_and_the_edge_clicks_within_a_pixel_of_it(
    slip, button
):
    x, y, width, height = slip
    points = probe_copy.points(slip, button)
    for name, (at_x, at_y) in points.items():
        assert x <= at_x <= x + width and y <= at_y <= y + height, name
    assert points["1px inside the slip's top edge"][1] == y + 1
    assert points["1px inside the slip's left edge"][0] == x + 1
    assert points["1px inside the slip's bottom edge"][1] == y + height - 1
    assert points["1px inside the slip's right edge"][0] == x + width - 1
    assert points["1px inside the button's top edge"][1] == button[1] + 1


def test_help_says_when_to_run_it_and_what_its_exit_status_means():
    done = subprocess.run([SCRIPT, "-h"], capture_output=True, text=True, check=False)
    assert done.returncode == 0, done.stderr
    for words in ("--root", "copy.js", "site.css", "Exit status"):
        assert words in done.stdout, words


def test_a_checkout_without_the_pages_is_refused_in_one_line(tmp_path):
    done = subprocess.run(
        [SCRIPT, "--root", tmp_path], capture_output=True, text=True, check=False
    )
    assert done.returncode == 2, done
    assert done.stdout == "" and len(done.stderr.strip().splitlines()) == 1, done
    assert "home.html" in done.stderr
