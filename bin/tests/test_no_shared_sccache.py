"""bin/tests/conftest.py's guard: no test's build reaches sccache, whose server every
session on the machine shares and a build starts again once it has gone idle. A bash
the test starts sources /etc/sandbox-persistent.sh, which sets RUSTC_WRAPPER=sccache
again when it is unset but keeps an empty one."""

import subprocess


def test_a_shell_a_test_starts_has_an_empty_rustc_wrapper():
    seen = subprocess.run(
        ["bash", "-c", 'printf %s "${RUSTC_WRAPPER-unset}"'],
        capture_output=True,
        text=True,
        check=True,
    )
    assert seen.stdout == ""
