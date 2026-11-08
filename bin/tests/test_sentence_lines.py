"""bin/sentence-lines: it breaks markdown prose after each sentence and leaves
everything else as it was, so the page renders the same. The cases run through the
script as a process, all in one run, each its own file."""

import subprocess
from pathlib import Path

import pytest

SCRIPT = Path(__file__).resolve().parents[1] / "sentence-lines"

# name: (text, the text after a rewrite)
SPLIT = {
    # Paragraphs, list items and block quotes split.
    "paragraph": (
        "One sentence. Two sentence? Three!\n",
        "One sentence.\nTwo sentence?\nThree!\n",
    ),
    "bullet": ("- First. Second.\n", "- First.\n  Second.\n"),
    "ordered": ("10. First. Second.\n", "10. First.\n    Second.\n"),
    "nested item": ("- a\n  - First. Second.\n", "- a\n  - First.\n    Second.\n"),
    "item paragraph": (
        "- a\n\n  First. Second.\n",
        "- a\n\n  First.\n  Second.\n",
    ),
    "item continuation": (
        "- a\n  First. Second.\n",
        "- a\n  First.\n  Second.\n",
    ),
    "quote": ("> First. Second.\n", "> First.\n> Second.\n"),
    "nested quote": ("> > First. Second.\n", "> > First.\n> > Second.\n"),
    "item in a quote": ("> - First. Second.\n", "> - First.\n>   Second.\n"),
    "closers": (
        '"Stop." Then (fine.) Next **bold.** After it.\n',
        '"Stop."\nThen (fine.)\nNext **bold.**\nAfter it.\n',
    ),
    "emphasis across sentences": ("*One. Two.*\n", "*One.\nTwo.*\n"),
    "two spaces": ("One.  Two.   Three.\n", "One.\nTwo.\nThree.\n"),
    "a code span starts a sentence": (
        "Done. `cargo fmt` runs.\n",
        "Done.\n`cargo fmt` runs.\n",
    ),
    "a link starts a sentence": (
        "Done. [Next](n.md) runs.\n",
        "Done.\n[Next](n.md) runs.\n",
    ),
    "a digit starts a sentence": ("Done. 3 agents ran.\n", "Done.\n3 agents ran.\n"),
    "after a closing fence": (
        "- a\n  ```sh\n  x. Y\n  ```\n  `-i` shows it. Then go.\n",
        "- a\n  ```sh\n  x. Y\n  ```\n  `-i` shows it.\n  Then go.\n",
    ),
    "a table interrupting an item's paragraph": (
        "- Item. Next\n  name | v\n  --- | ---\n  A. B | x\n",
        "- Item.\n  Next\n  name | v\n  --- | ---\n  A. B | x\n",
    ),
    "front matter, then prose": (
        "---\ntitle: A. B.\n---\nOne. Two.\n",
        "---\ntitle: A. B.\n---\nOne.\nTwo.\n",
    ),
    "a fence opener can't hold a backtick": (
        "```a``` is odd. Next.\n\n```\nC. D.\n```\n",
        "```a``` is odd.\nNext.\n\n```\nC. D.\n```\n",
    ),
    # Never inside inline code, links, link destinations, HTML or bare URLs.
    "inline code": ("Run `a. B` now. Then go.\n", "Run `a. B` now.\nThen go.\n"),
    "link text": (
        "See [this. That](https://x.y/a) here. Next.\n",
        "See [this. That](https://x.y/a) here.\nNext.\n",
    ),
    "link destination": (
        "See [x](<a. B>) here. Next.\n",
        "See [x](<a. B>) here.\nNext.\n",
    ),
    "reference link": (
        "See [One. Two][ref] here. Next.\n",
        "See [One. Two][ref] here.\nNext.\n",
    ),
    "inline html": (
        'A <span title="A. B">y</span>. Next.\n',
        'A <span title="A. B">y</span>.\nNext.\n',
    ),
    "bare url": (
        "Go to https://x.y/a.B now. Next.\n",
        "Go to https://x.y/a.B now.\nNext.\n",
    ),
    "inside a parenthesis or a quotation": (
        '(See it. Then go.) Said "Go. Now." Done.\n',
        '(See it. Then go.)\nSaid "Go. Now."\nDone.\n',
    ),
    # Not after abbreviations, decimals, versions, file names or initials.
    "abbreviations": (
        "Use e.g. Foo, i.e. Bar, etc. Baz vs. Qux, cf. Quux, Smith et al. Found it.\n",
        "Use e.g. Foo, i.e. Bar, etc. Baz vs. Qux, cf. Quux, Smith et al. Found it.\n",
    ),
    "versions and file names": (
        "Ruff 0.14.4 reads design.md and AGENTS.md now. Then go.\n",
        "Ruff 0.14.4 reads design.md and AGENTS.md now.\nThen go.\n",
    ),
    "a file name ends a sentence": (
        "Read design.md. Then go.\n",
        "Read design.md.\nThen go.\n",
    ),
    "initials": (
        "J. R. Smith wrote it. The U.S. Army too. Then.\n",
        "J. R. Smith wrote it.\nThe U.S. Army too.\nThen.\n",
    ),
    "possessives and units": (
        "The org's. The day's. In 30 s. Then.\n",
        "The org's.\nThe day's.\nIn 30 s.\nThen.\n",
    ),
    "other abbreviations": (
        "Mr. Smith, Mrs. Jones, Ms. Lee and Dr. Who met. See Fig. Two at approx. Ten. Done.\n",
        "Mr. Smith, Mrs. Jones, Ms. Lee and Dr. Who met.\nSee Fig. Two at approx. Ten.\nDone.\n",
    ),
    # A new line never starts what could read as a block.
    "starts that could open a block": (
        "Ends here. lower. # x. - x. + x. * x. *\tx. *** x. ___ x. > x. 2. x. 1.\tx. "
        "= x. | x. <b> x. ```x``` x. Okay.\n",
        "Ends here. lower. # x. - x. + x. * x. *\tx. *** x. ___ x. > x. 2. x. 1.\tx. "
        "= x. | x. <b> x. ```x``` x.\nOkay.\n",
    ),
    "a byte order mark": (
        "﻿# One. Two.\nThree. Four.\n",
        "﻿# One. Two.\nThree.\nFour.\n",
    ),
    "an html attribute holding >": (
        'A <a title="x > y. Z">b</a>. Done.\n',
        'A <a title="x > y. Z">b</a>.\nDone.\n',
    ),
    # Lines holding an HTML comment, or slop-lint's marker, stay whole.
    "html comment": (
        "One. Two. <!-- slop-lint: ignore -->\n",
        "One. Two. <!-- slop-lint: ignore -->\n",
    ),
    "slop-lint marker in code": (
        "Note `slop-lint: ignore` here. Two.\n",
        "Note `slop-lint: ignore` here. Two.\n",
    ),
    # Hard line breaks stay.
    "hard breaks": (
        "One. Two.  \nThree. Four.\\\nFive.\n",
        "One.\nTwo.  \nThree.\nFour.\\\nFive.\n",
    ),
    # Line endings and odd characters.
    "crlf": ("One. Two.\r\nThree.\r\n", "One.\r\nTwo.\r\nThree.\r\n"),
    "no final newline": ("One. Two.", "One.\nTwo."),
    "line separator": ("One. Two. Three.\n", "One. Two.\nThree.\n"),
    "no-break space": ("One. Two. Three.\n", "One. Two.\nThree.\n"),
}

# Text the rewrite leaves as it is.
LEFT = {
    "atx heading": "# One. Two.\n",
    "heading in a quote": "> # Head. Tail\n",
    "setext heading": "One. Two.\n===\n\nThree. Four\n---\n",
    "table": "| a. B | c |\n|---|---|\n| d. E | f |\n",
    "table without edge pipes": "name. Two | value\n--- | ---\nFirst. Second | x\n",
    "table after a paragraph line": "Some text\nname. Two | v\n--- | ---\nA. B | x\n",
    "a sentence right after a table": "| a | b |\n|---|---|\n| c | d |\nText after. More here.\n",
    "table in a quote": "> | a |\n> |---|\n> c. Done\n",
    "table on an item's line": "- | a |\n  |---|\n  b. Done\n",
    "fence": "```\nOne. Two.\n```\n",
    "tilde fence": "~~~\nOne. Two.\n~~~\n",
    "fence with an info string": "```sh\nOne. Two.\n```\n",
    "fence in a quote": "> ```\n> One. Two.\n> ```\n",
    "fence under an item": "- a\n  ```\n  One. Two.\n  ```\n",
    "a closer can't hold an info string": "```\nGo. Stop.\n``` sh\nGo. Stop.\n```\n",
    "a closer indented four is code": "```\nx\n    ```\nGo. Stop.\n```\n",
    "a closer indented four in an item is code": (
        "- a\n\n  ```\n  x\n      ```\n  Go. Stop.\n  ```\n"
    ),
    "a fence ends with its quote": "> ```\n> code\n```\nInside. Code.\n```\n",
    "a fence ends with its item": "- a\n\n  ```\n  x\n ```\n  Three. Four.\n  ```\n",
    "a fence on an item's line": "- ```\n  One. Two.\n  ```\n",
    "a fence in a nested item": "- - ```\n    Code. More.\n    ```\n",
    "indented code": "Text\n\n    One. Two.\n",
    "tab-indented code": "Text\n\n\tOne. Two.\n",
    "indented code in an item": "- a\n\n      One. Two.\n",
    "code after a marker's five spaces": "-     One. Two.\n",
    "a tab after a marker": "-\tItem\n\n        Code. More.\n",
    "html block": "<div>\nOne. Two.\n</div>\n",
    "pre past a blank line": "<pre>\nOne. Two.\n\nThree. Four.\n</pre>\n",
    "pre on an item's line": "- <pre>\n  One. Two.\n\n  Three. Four.\n  </pre>\n",
    "a quote on an item's line": "- > ```\n  > One. Two.\n",
    "html comment block": "<!--\nOne. Two.\n\nThree. Four.\n-->\n",
    "processing instruction": "<?php\nOne. Two.\n\nThree. Four.\n?>\n",
    "declaration": "<!DOCTYPE html\nOne. Two.\n\nThree. Four.\n>\n",
    "cdata": "<![CDATA[\nOne. Two.\n\nThree. Four.\n]]>\n",
    "front matter": "---\ntitle: One. Two.\n...\n",
    "link reference definition": "[a]: https://x.y/ 'Go. Stop.'\n",
    "a definition's title on its next line": "[a]: /url\n'Title. Two'\n",
    "a link title over two lines": "[x](/u 'One.\nTwo. Three') end.\n",
}


def run(*args: str) -> subprocess.CompletedProcess:
    return subprocess.run(
        [str(SCRIPT), *args], capture_output=True, text=True, timeout=30, check=False
    )


def write(path: Path, text: str) -> Path:
    path.write_bytes(text.encode())
    return path


@pytest.fixture(scope="module")
def files(tmp_path_factory) -> dict[str, Path]:
    """Each case in a file of its own, after one run over all of them."""
    folder = tmp_path_factory.mktemp("cases")
    cases = {**{n: t for n, (t, _) in SPLIT.items()}, **LEFT}
    files = {
        name: write(folder / f"{i}.md", text)
        for i, (name, text) in enumerate(cases.items())
    }
    done = run(*map(str, files.values()))
    assert done.returncode == 0, done.stdout + done.stderr
    assert done.stdout == ""
    return files


@pytest.fixture(scope="module")
def rewritten(files) -> dict[str, str]:
    return {name: path.read_bytes().decode() for name, path in files.items()}


@pytest.mark.parametrize("name", SPLIT)
def test_prose_breaks_after_each_sentence(rewritten, name):
    assert rewritten[name] == SPLIT[name][1]


@pytest.mark.parametrize("name", LEFT)
def test_everything_else_is_left_as_it_was(rewritten, name):
    assert rewritten[name] == LEFT[name]


def test_a_second_run_changes_nothing_and_check_passes(files, rewritten):
    paths = [str(path) for path in files.values()]
    assert run("--check", *paths).stdout == ""
    assert run(*paths).returncode == 0
    assert {name: path.read_bytes().decode() for name, path in files.items()} == (
        rewritten
    )


def test_check_lists_the_lines_it_would_split_and_changes_nothing(tmp_path):
    text = "# Title\n\nOne. Two.\n\nAlone.\n\n- Go. Stop.\n"
    path = write(tmp_path / "doc.md", text)
    done = run("--check", str(path))
    assert done.returncode == 1
    assert done.stdout.splitlines() == [
        f"{path}:3: One. Two.",
        f"{path}:7: - Go. Stop.",
    ]
    assert path.read_bytes().decode() == text


def test_check_passes_a_file_with_nothing_to_split(tmp_path):
    path = write(tmp_path / "doc.md", "One.\nTwo.\n")
    done = run("--check", str(path))
    assert (done.returncode, done.stdout) == (0, "")


def test_check_cuts_a_long_line_to_80_characters(tmp_path):
    path = write(tmp_path / "doc.md", "A" * 100 + ". Two.\n")
    line = run("--check", str(path)).stdout.strip()
    assert line == f"{path}:1: " + "A" * 80


def test_a_file_that_isnt_utf8_fails_in_one_line(tmp_path):
    path = tmp_path / "doc.md"
    path.write_bytes(b"One. \xff Two.\n")
    done = run(str(path))
    assert done.returncode == 2
    assert len(done.stderr.splitlines()) == 1, done.stderr
    assert path.read_bytes() == b"One. \xff Two.\n"


def test_a_megabyte_line_is_quick(tmp_path):
    path = write(tmp_path / "doc.md", "Word one. " * 100_000 + "\n")
    done = subprocess.run(
        [str(SCRIPT), str(path)],
        capture_output=True,
        text=True,
        timeout=20,
        check=False,
    )
    assert done.returncode == 0, done.stderr
    assert path.read_text().count("\n") == 100_000


def test_help_says_what_it_does():
    done = run("-h")
    assert done.returncode == 0
    assert "--check" in done.stdout
