"""rust-toolchain.toml pins the Rust that builds this repo everywhere: the sandbox, the
owner's host and CI, at the version that built production's binary (1.100.0 on
2026-11-25). rustup reads the file. Each dtolnay/rust-toolchain step names the same
version (test_workflow_supply_chain.py holds them equal), and a job that uses the
runner's own rustup installs the file's toolchain, by `rustup toolchain install` with
no argument, before it adds a target."""

import re
import tomllib
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
WORKFLOWS = REPO / ".github" / "workflows"
# A job's block: from its name at two spaces to the next job's.
JOB = re.compile(r"^  [\w-]+:\n(?:(?!  [\w-]+:\n).*\n?)*", re.MULTILINE)


def test_the_toolchain_file_names_an_exact_version():
    channel = tomllib.loads((REPO / "rust-toolchain.toml").read_text())["toolchain"][
        "channel"
    ]
    assert re.fullmatch(r"\d+\.\d+\.\d+", channel), channel


def test_a_job_that_adds_targets_with_rustup_installs_the_files_toolchain_first():
    jobs = [
        (path.name, job[0])
        for path in sorted(WORKFLOWS.glob("*.yml"))
        for job in JOB.finditer(path.read_text())
        if "rustup target add" in job[0]
    ]
    assert jobs, "no job adds a target with rustup; drop this test"
    for name, job in jobs:
        install = re.search(
            r"^\s*- run: rustup toolchain install\s*$", job, re.MULTILINE
        )
        assert install, f"{name}: a job adds a target without the file's toolchain"
        assert install.start() < job.index("rustup target add"), name
