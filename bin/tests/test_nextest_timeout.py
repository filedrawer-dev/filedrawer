"""The Rust tests run under cargo-nextest with the repo's `.config/nextest.toml`, which
kills a test that hangs instead of letting it hold the run, the commit hook and CI."""

import os
import re
import subprocess
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]


def test_nextest_kills_a_test_that_hangs(tmp_path):
    crate = tmp_path / "hang"
    (crate / "src").mkdir(parents=True)
    (crate / "Cargo.toml").write_text(
        '[package]\nname = "hang"\nversion = "0.1.0"\nedition = "2024"\n'
    )
    (crate / "src/lib.rs").write_text(
        "#[test]\nfn hangs() {\n    std::thread::sleep(std::time::Duration::from_secs(40));\n}\n"
    )
    (crate / ".config").mkdir()
    profile = (REPO / ".config/nextest.toml").read_text()
    # The same limits, a second a period instead of a minute.
    (crate / ".config/nextest.toml").write_text(
        re.sub(r'period = "\d+s"', 'period = "1s"', profile)
    )
    done = subprocess.run(
        ["cargo", "nextest", "run", "--cargo-quiet"],
        cwd=crate,
        env={**os.environ, "CARGO_TARGET_DIR": str(tmp_path / "target")},
        capture_output=True,
        text=True,
        timeout=100,
        check=False,
    )
    assert done.returncode != 0, done.stdout + done.stderr
    assert "TIMEOUT" in done.stdout + done.stderr, done.stdout + done.stderr
