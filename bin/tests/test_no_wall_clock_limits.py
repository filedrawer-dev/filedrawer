"""No test in bin/tests sets a wall-clock limit of its own. A limit sized to a quiet
machine fails code that works on a loaded one: on 2026-11-08 a wizard test that takes
2 s alone ran past its 60 s limit beside another session's check, and refused the
commit. pytest.ini's per-test timeout is the one hang guard, set far above any loaded
run. A slow machine may let a bug pass, but must never fail a test.

This refuses a call's `timeout=` keyword, as subprocess.run takes it, and any
`pytest.mark.timeout` or `mark.timeout`. It can't see a limit passed by position, such
as `thread.join(5)`, nor code a test runs from a string."""

import ast
from pathlib import Path

TESTS = Path(__file__).resolve().parent


def limit_lines(source: str) -> list[int]:
    """The line of each call in `source` that passes `timeout=`, and of each timeout
    mark."""
    found = []
    for node in ast.walk(ast.parse(source)):
        if isinstance(node, ast.Call) and any(
            keyword.arg == "timeout" for keyword in node.keywords
        ):
            found.append(node.lineno)
        elif (
            isinstance(node, ast.Attribute)
            and node.attr == "timeout"
            and (
                (isinstance(node.value, ast.Attribute) and node.value.attr == "mark")
                or (isinstance(node.value, ast.Name) and node.value.id == "mark")
            )
        ):
            found.append(node.lineno)
    return sorted(found)


def test_limit_lines_finds_call_timeouts_and_timeout_marks_but_not_text():
    source = (
        "import pytest, subprocess\n"
        "from pytest import mark\n"
        "pytestmark = pytest.mark.timeout(900)\n"
        "@mark.timeout(5)\n"
        "def test_a():\n"
        "    subprocess.run(\n"
        "        ['true'],\n"
        "        timeout=60,\n"
        "    )\n"
        "    script = 'subprocess.run([], timeout=1)'\n"
        "    subprocess.run(['sleep', '1'], check=True)\n"
    )
    assert limit_lines(source) == [3, 4, 6]


def test_no_test_sets_a_wall_clock_limit_of_its_own():
    found = [
        f"{path.name}:{line}"
        for path in sorted(TESTS.glob("*.py"))
        for line in limit_lines(path.read_text())
    ]
    assert not found, (
        "pytest.ini's timeout is the hang guard; bin/tests sets no limit of its own"
        " (test_no_wall_clock_limits.py says why). Drop these: " + ", ".join(found)
    )
