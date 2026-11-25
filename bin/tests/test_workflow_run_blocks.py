"""A `run:` block in .github/workflows runs a few commands in a row (CODING_STANDARDS.md,
GitHub workflows). Logic that loops, parses or builds output goes in a script in bin/
or ops/ with tests, as bin/caddy-latest does for caddy-latest.yml: on 2026-11-18 a
first draft of that workflow parsed with sed and jq in the YAML, and a review sent it
back. So a block may not call sed, jq or awk, loop with for or while, or run more than
5 command lines, unless EXEMPT names it with a reason (the owner's decision,
2026-11-18).

This reads lines, not YAML: a block scalar after `run:` is the lines indented past
the `run` key, and a one-line `run:` is one command line. Blank and comment lines
don't count, and a line ending in `\\` joins the next. An exemption waives the line
cutoff only. It misses: `sed` and the rest behind `command`, `xargs` or a variable;
a heredoc's lines, which it counts as commands; a folded `>` block's lines, which it
counts one by one; and it takes a `run:` key anywhere for a step's, one under `with:`
or inside another multi-line string included."""

import re
from pathlib import Path

import pytest

REPO = Path(__file__).resolve().parents[2]
WORKFLOWS = REPO / ".github/workflows"
MAX_LINES = 5
# After a separator, or `--` as in gh's --jq. Quoted text is taken out first, but a
# command substitution in it still runs, so it is read on its own too.
PARSING = re.compile(r"(?:^|[\s|;&(`]|--)(sed|jq|awk|for|while|until)\b")
QUOTED = re.compile(r""""[^"]*"|'[^']*'""")
SUBSTITUTION = re.compile(r"\$\(([^()]*)\)")
# A block scalar's header: | or >, its chomping and indentation indicators in either
# order, and maybe a comment.
BLOCK_HEADER = re.compile(r"^[|>](?:[+-]?\d?|\d[+-])\s*(?:#.*)?$")

# (workflow, the block's first command line): why it may stay in the YAML.
EXEMPT = {
    (
        "ci.yml",
        'if [ -z "$SKILL_TAGS_SSH_KEY" ]; then',
    ): "a refusal, then the key written for ssh and one bin/skills call",
    ("ci.yml", "key=$(mktemp)"): "the key and known_hosts written for one ssh call",
    (
        "ci.yml",
        'if [ -z "$FILEDRAWER_STAGING_SCREEN_API_KEY" ]; then',
    ): "one ops/smoke-staging call and what its exit status means",
    (
        "mutants.yml",
        "ulimit -d 3145728",
    ): "one cargo mutants call and what its exit status means",
    (
        "release-cli.yml",
        "exe=target/${{ matrix.target }}/release/filedrawer",
    ): "the npm packages packed, installed and run once",
}


def run_blocks(text: str) -> list[tuple[int, list[str]]]:
    """Each `run:` block's first line number and its command lines."""
    lines = text.splitlines()
    blocks = []
    i = 0
    while i < len(lines):
        found = re.match(r"^(\s*(?:- )?)run:\s*(.*)$", lines[i])
        if not found:
            i += 1
            continue
        # The column of `run`: a step's other keys sit there, its block past it.
        indent = len(found.group(1))
        rest = found.group(2).strip()
        body = []
        if BLOCK_HEADER.match(rest):
            j = i + 1
            while j < len(lines) and (
                not lines[j].strip() or len(lines[j]) - len(lines[j].lstrip()) > indent
            ):
                body.append(lines[j].strip())
                j += 1
        else:
            body, j = [rest], i + 1
        commands, joined = [], ""
        for line in body:
            if not line or line.startswith("#"):
                continue
            joined += line
            if line.endswith("\\"):
                joined = joined.removesuffix("\\") + " "
                continue
            commands.append(joined)
            joined = ""
        if joined:
            commands.append(joined.rstrip())
        blocks.append((i + 1, commands))
        i = j
    return blocks


def parsing_commands(command: str) -> set[str]:
    """The parsing commands `command` runs, outside quotes or in a substitution."""
    texts = [QUOTED.sub('""', command), *SUBSTITUTION.findall(command)]
    return {m.group(1) for text in texts for m in PARSING.finditer(text)}


def problems(name: str, text: str) -> list[str]:
    """What each run block breaks. An exemption waives only the line cutoff."""
    found = []
    for line, commands in run_blocks(text):
        if not commands:
            continue
        parsing = sorted(set().union(*map(parsing_commands, commands)))
        if parsing:
            found.append(f"{name}:{line} runs {', '.join(parsing)}")
        elif len(commands) > MAX_LINES and (name, commands[0]) not in EXEMPT:
            found.append(f"{name}:{line} runs {len(commands)} command lines")
    return found


def workflows() -> list[Path]:
    return sorted(WORKFLOWS.glob("*.yml"))


def test_run_blocks_finds_parsing_and_long_blocks_but_not_short_sequences():
    text = (
        "jobs:\n"
        "  a:\n"
        "    steps:\n"
        '      - run: bin/caddy-latest versions >> "$GITHUB_OUTPUT"\n'
        "      - run: |\n"
        "          tag=$(gh api x --jq .tag_name)\n"
        "      - name: long\n"
        "        run: |\n"
        "          a\n"
        "          # a comment\n"
        "          b \\\n"
        "            --flag\n"
        "\n"
        "          c\n"
        "          d\n"
        "          e\n"
        "          f\n"
        "      - run: |\n"
        "          for x in 1 2; do echo $x; done\n"
        "      - run: echo forward sedan\n"
        '      - run: echo "waits for the lock"\n'
    )
    assert problems("t.yml", text) == [
        "t.yml:5 runs jq",
        "t.yml:8 runs 6 command lines",
        "t.yml:18 runs for",
    ]


@pytest.mark.parametrize(
    ("text", "expected"),
    [
        # A step's other keys after its block aren't commands.
        (
            "      - run: |\n          a\n        env:\n          A: 1\n          B: 2\n"
            "          C: 3\n          D: 4\n          E: 5\n",
            [],
        ),
        # Every block header YAML has, a comment after it included.
        ("      - run: | # why\n          sed -n 1p f\n", ["t.yml:1 runs sed"]),
        ("      - run: |+\n          sed -n 1p f\n", ["t.yml:1 runs sed"]),
        ("      - run: |2\n          sed -n 1p f\n", ["t.yml:1 runs sed"]),
        # A command substitution inside quotes still runs.
        (
            '      - run: echo "tag=$(jq -r .tag f)" >> "$GITHUB_OUTPUT"\n',
            ["t.yml:1 runs jq"],
        ),
        # A name with a hyphen isn't the command.
        ("      - run: bin/release-for x\n", []),
        ("      - run: until false; do :; done\n", ["t.yml:1 runs until"]),
        # A last line ending in a backslash is still a command.
        (
            "      - run: |\n" + "          a\n" * 5 + "          b \\\n",
            ["t.yml:1 runs 6 command lines"],
        ),
    ],
)
def test_run_blocks_reads_yaml_spellings(text, expected):
    assert problems("t.yml", text) == expected


def test_an_exemption_waives_the_line_cutoff_but_never_the_parsing():
    text = "      - run: |\n          key=$(mktemp)\n          jq . f\n"
    assert problems("ci.yml", text) == ["ci.yml:1 runs jq"]


@pytest.mark.parametrize("path", workflows(), ids=lambda p: p.name)
def test_each_run_block_is_a_few_commands(path):
    assert problems(path.name, path.read_text()) == [], (
        "move the logic into a script in bin/ or ops/ with tests, or, for a sequence "
        "of commands, name the block in EXEMPT with why"
    )


def test_each_exemption_names_a_block_there_is():
    firsts = {
        (path.name, commands[0])
        for path in workflows()
        for _, commands in run_blocks(path.read_text())
        if commands
    }
    assert set(EXEMPT) - firsts == set()
