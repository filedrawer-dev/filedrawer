"""What the workflows run besides this repo's own code.

Every action is pinned to a commit, with its version in a comment for the reader and
for Dependabot (.github/dependabot.yml), which moves the pins by pull request: a tag
or a branch (`@v2`, `@stable`) is whatever was last pushed there, and a job that
builds the server's binary or a published package would run it unread.

An action that installs a tool is told which version: dtolnay/rust-toolchain gets
its `toolchain`, which the pinned commit no longer takes from a branch's name, and
taiki-e/install-action a version for each tool, which it installs by the checksum
its pinned commit lists.

A job that builds what ships, or that can publish or deploy, restores no Actions
cache: build-staging's binary goes to the VPS and on to production, and the wheel
and sdist jobs' files go to PyPI and npm. A cache entry on main belongs to whichever
job on main saved its key first, so restoring one would make every such job's tools
part of what built the shipped file, or put them beside a publishing identity.
astral-sh/setup-uv saves and restores uv's cache unless `enable-cache: false`, and
actions/setup-node npm's, for a package.json that names npm, unless
`package-manager-cache: false`.

This reads lines, not YAML, as test_workflow_run_blocks.py does: a job is the lines
from its key at two spaces to the next one, and a step's settings are the lines
after its `uses:` up to the next step.
"""

import re
import tomllib
from pathlib import Path

import pytest

REPO = Path(__file__).resolve().parents[2]
WORKFLOWS = REPO / ".github/workflows"
DEPENDABOT = REPO / ".github/dependabot.yml"
# (workflow, job): what it builds, or what it can do.
SHIPPED = {
    ("ci.yml", "build-staging"): "builds the server binary staging and production run",
    ("ci.yml", "release-skills"): "tags the skills",
    ("ci.yml", "deploy-staging"): "deploys staging",
    ("release-cli.yml", "wheel"): "builds the wheels and the npm packages' binaries",
    ("release-cli.yml", "sdist"): "builds the sdist",
    ("release-cli.yml", "crates"): "publishes to crates.io",
    ("release-cli.yml", "publish"): "publishes to PyPI",
    ("release-cli.yml", "npm"): "publishes to npm",
}
# An action that restores a cache unless told not to, and the setting that tells it.
CACHE_OFF = {
    "astral-sh/setup-uv": "enable-cache: false",
    "actions/setup-node": "package-manager-cache: false",
}
CACHING_ACTIONS = (
    "Swatinem/rust-cache",
    "actions/cache",
    "mozilla-actions/sccache-action",
)
USES = re.compile(r"^\s*(?:- )?uses:\s*(\S+)(.*)$")
PINNED = re.compile(r"^[\w.-]+/[\w./-]+@[0-9a-f]{40}$")
# What a pin's comment names: a version tag, or the branch a commit with no tag is on.
VERSION_COMMENT = re.compile(r"^\s+# (v\d+(\.\d+){0,2}|master|main)$")


def unpinned(name: str, text: str) -> list[str]:
    """Each `uses:` of `text` that isn't a commit with a version comment."""
    found = []
    for number, line in enumerate(text.splitlines(), 1):
        used = USES.match(line)
        if not used:
            continue
        action, comment = used.groups()
        if action.startswith("./"):
            continue
        if not PINNED.match(action):
            found.append(f"{name}:{number} {action} isn't pinned to a commit")
        elif not VERSION_COMMENT.match(comment):
            found.append(f"{name}:{number} {action} has no `# vX.Y.Z` comment")
    return found


def steps_using(lines: list[str]) -> list[tuple[str, list[str]]]:
    """Each `uses:` step's action, without its ref, and the lines after it up to the
    next step."""
    found = []
    for i, line in enumerate(lines):
        used = USES.match(line)
        if not used:
            continue
        rest = []
        for later in lines[i + 1 :]:
            if re.match(r"^\s*- ", later):
                break
            rest.append(later.strip())
        found.append((used.group(1).split("@")[0], rest))
    return found


def job_lines(workflow: str, job: str) -> list[str]:
    lines = (WORKFLOWS / workflow).read_text().splitlines()
    start = lines.index(f"  {job}:")
    end = next(
        (
            i
            for i in range(start + 1, len(lines))
            if re.match(r"^  [\w-]+:\s*$", lines[i])
        ),
        len(lines),
    )
    return lines[start:end]


def every_step() -> list[tuple[str, str, list[str]]]:
    return [
        (path.name, action, settings)
        for path in sorted(WORKFLOWS.glob("*.y*ml"))
        for action, settings in steps_using(path.read_text().splitlines())
    ]


def test_unpinned_finds_tags_branches_short_commits_and_missing_comments():
    sha = "3d3c42e5aac5ba805825da76410c181273ba90b1"
    text = (
        "    steps:\n"
        f"      - uses: actions/checkout@{sha} # v7.0.1\n"
        "      - uses: actions/checkout@v7\n"
        "      - if: always()\n"
        "        uses: dtolnay/rust-toolchain@stable\n"
        f"      - uses: actions/checkout@{sha[:12]} # v7.0.1\n"
        f"      - uses: actions/checkout@{sha}\n"
        f"      - uses: dtolnay/rust-toolchain@{sha} # master\n"
        "      - uses: ./.github/actions/local\n"
        f"      - uses: docker://alpine@{sha} # v3\n"
    )
    assert unpinned("t.yml", text) == [
        "t.yml:3 actions/checkout@v7 isn't pinned to a commit",
        "t.yml:5 dtolnay/rust-toolchain@stable isn't pinned to a commit",
        f"t.yml:6 actions/checkout@{sha[:12]} isn't pinned to a commit",
        f"t.yml:7 actions/checkout@{sha} has no `# vX.Y.Z` comment",
        f"t.yml:10 docker://alpine@{sha} isn't pinned to a commit",
    ]


@pytest.mark.parametrize("path", sorted(WORKFLOWS.glob("*.y*ml")), ids=lambda p: p.name)
def test_every_action_is_pinned_to_a_commit(path):
    assert unpinned(path.name, path.read_text()) == [], (
        "resolve the tag to its commit (gh api repos/OWNER/REPO/git/ref/tags/TAG, "
        "following an annotated tag) and write `uses: OWNER/REPO@<commit> # vX.Y.Z`"
    )


def test_the_rust_toolchain_action_is_told_rust_toolchain_tomls_version():
    # Each step, not the day's stable: CI took 1.100 alone while the sandbox had
    # 1.98, and tests that passed at commit failed only there (#146's CI run).
    channel = tomllib.loads((REPO / "rust-toolchain.toml").read_text())["toolchain"][
        "channel"
    ]
    steps = [s for s in every_step() if s[1] == "dtolnay/rust-toolchain"]
    assert steps, "no workflow uses dtolnay/rust-toolchain; drop this test"
    told = [
        (name, [line for line in settings if line.startswith("toolchain: ")])
        for name, _, settings in steps
    ]
    wrong = [
        (name, lines) for name, lines in told if lines != [f"toolchain: {channel}"]
    ]
    assert wrong == [], f"dtolnay/rust-toolchain not told {channel}"


def test_install_action_installs_each_tool_at_a_version_and_builds_none():
    steps = [s for s in every_step() if s[1] == "taiki-e/install-action"]
    assert steps, "no workflow uses taiki-e/install-action; drop this test"
    for name, _, settings in steps:
        tool_lines = [line for line in settings if line.startswith("tool: ")]
        assert len(tool_lines) == 1, f"{name}: taiki-e/install-action without `tool:`"
        tools = tool_lines[0].removeprefix("tool: ")
        unversioned = [
            t for t in tools.split(",") if not re.fullmatch(r"[\w-]+@\d+\.\d+\.\d+", t)
        ]
        assert unversioned == [], f"{name} installs the latest {unversioned}"
        # A tool the pinned commit has no checksum for fails the step, where the
        # default would fetch or build it some other way.
        assert "fallback: none" in settings, name


@pytest.mark.parametrize(("workflow", "job"), sorted(SHIPPED))
def test_a_job_that_builds_what_ships_or_publishes_restores_no_cache(workflow, job):
    restored = []
    for action, settings in steps_using(job_lines(workflow, job)):
        if action in CACHING_ACTIONS:
            restored.append(action)
        if action in CACHE_OFF and CACHE_OFF[action] not in settings:
            restored.append(f"{action} without `{CACHE_OFF[action]}`")
    assert not restored, (
        f"{workflow}'s {job} {SHIPPED[(workflow, job)]}, and restores a cache any "
        f"job on main can write: {', '.join(restored)}"
    )


@pytest.mark.parametrize("ecosystem", ["github-actions", "cargo"])
def test_dependabot_moves_what_is_pinned(ecosystem):
    """A pin nothing moves goes stale: deleting .github/dependabot.yml means moving
    the actions' commits and Cargo.lock by hand, and dropping this test."""
    assert f"  - package-ecosystem: {ecosystem}\n" in DEPENDABOT.read_text()
