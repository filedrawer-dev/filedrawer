"""No crate holds a doctest, since no check runs one. The Rust tests run under
cargo-nextest, which skips doctests, and the full check dropped its `cargo test --doc`
run, a build of its own at every landing, once the crates were found to have none
(#36). A doctest added since would pass every check unrun, so this fails on one.

It reads every .rs file under crates/ for each place rustdoc could find a doctest. In
a doc comment (`///`, `//!`, `/** */` or `/*! */`): a fence (``` or ~~~) whose info
string rustdoc reads as Rust, that is empty, naming `rust`, or holding only rustdoc's
attributes (`ignore`, `no_run`, `edition2021` and the like), and an indented code
block, a line indented four spaces or a tab past the comment's own space, first in its
comment or after a blank comment line. And a `#[doc = ...]` or `#![doc = ...]`
attribute that includes a file (`include_str!`), whose text this can't see, or holds a
fence on its line.

It is a heuristic, a line-by-line reading of what rustdoc's Markdown parser does, and
misses rarer forms: a `#[doc]` attribute split over lines, a fence inside a block
quote or a list item. It reads binaries' docs too, which rustdoc never tests, so it
may refuse a fence that would never run: name the fence's language, as ```text. To
keep a real doctest, bring `cargo test --doc` back into bin/agent-pre-commit's Rust
checks and drop this test."""

import re
from pathlib import Path

import pytest

REPO = Path(__file__).resolve().parents[2]

# The fence attributes rustdoc reads as Rust, beyond `rust`, edition* and ignore-*.
RUST_ATTRIBUTES = {
    "ignore",
    "should_panic",
    "no_run",
    "compile_fail",
    "test_harness",
    "standalone_crate",
    "allow_fail",
}
LINE_DOC = re.compile(r"^\s*//[/!](?![/!])")
BLOCK_DOC_START = re.compile(r"^\s*/\*[*!](?![*/])")
# A doc attribute, alone or inside another, as #![cfg_attr(doctest, doc = ...)].
DOC_ATTRIBUTE = re.compile(r"#!?\[.*?\bdoc\s*=")
FENCE = re.compile(r"^\s{0,3}(`{3,}|~{3,})(.*)$")


def is_rust_fence(info: str) -> bool:
    """Whether rustdoc may test a fenced block with this info string: it has none, or
    any token rustdoc reads as Rust's (rustdoc's own rule also weighs their order,
    which this ignores, erring toward a doctest)."""
    tokens = [t for t in re.split(r"[\s,]+", info.strip().strip("{}")) if t]
    return not tokens or any(
        t == "rust"
        or t in RUST_ATTRIBUTES
        or t.startswith(("edition", "ignore-"))
        or re.fullmatch(r"E\d{4}", t)
        for t in tokens
    )


def unspaced(body: str) -> str:
    """A doc comment's text without the one space after its marker."""
    return body[1:] if body.startswith(" ") else body


def doc_lines(text: str):
    """Each doc comment line as (line number, its text past the comment marker and
    one space), and (line number, None) for each line that ends a run of doc comment
    or holds none."""
    in_block = False
    for number, line in enumerate(text.splitlines(), 1):
        if in_block:
            body, closed, _ = line.partition("*/")
            yield number, unspaced(re.sub(r"^\s*\*(?!/)", "", body))
        elif line_doc := LINE_DOC.match(line):
            yield number, unspaced(line[line_doc.end() :])
            continue
        elif block_doc := BLOCK_DOC_START.match(line):
            body, closed, _ = line[block_doc.end() :].partition("*/")
            yield number, unspaced(body)
        else:
            yield number, None
            continue
        in_block = not closed
        if closed:
            yield number, None


def doctests(text: str) -> list[tuple[int, str]]:
    """Each line of a source file's text where a doctest may start, with why."""
    found = []
    fence = None
    # Whether an indented code block may start here: it can't interrupt a
    # paragraph, but may follow a blank line, a heading or a closed fence.
    block_may_start = True
    for number, body in doc_lines(text):
        if body is None:
            fence, block_may_start = None, True
            continue
        mark = FENCE.match(body)
        if fence is not None:
            if mark and mark.group(1).startswith(fence) and not mark.group(2).strip():
                fence, block_may_start = None, True
            continue
        if mark:
            fence = mark.group(1)
            if is_rust_fence(mark.group(2)):
                found.append((number, f"a Rust fence: {body.strip()}"))
        elif block_may_start and re.match(r"( {4}|\t)\S", body):
            found.append((number, "an indented code block"))
        block_may_start = fence is None and (
            not body.strip() or re.match(r" {0,3}#{1,6}(\s|$)", body) is not None
        )
    for number, line in enumerate(text.splitlines(), 1):
        attribute = DOC_ATTRIBUTE.search(line)
        if attribute and "include_str!" in line[attribute.end() :]:
            found.append((number, "a doc attribute that includes a file"))
        elif attribute and re.search(r"```|~~~", line[attribute.end() :]):
            found.append((number, "a doc attribute holding a fence"))
    return sorted(found)


def test_no_crate_holds_a_doctest():
    found = [
        f"{path.relative_to(REPO)}:{number}: {why}"
        for path in sorted((REPO / "crates").glob("**/*.rs"))
        if "target" not in path.relative_to(REPO).parts
        for number, why in doctests(path.read_text(errors="replace"))
    ]
    assert not found, (
        "a doctest no check runs, since nextest skips doctests and the full check"
        " doesn't run cargo test --doc: name the fence's language (```text) or write"
        " the example as a test (this file's docstring says how to keep a real one):\n"
        + "\n".join(found)
    )


@pytest.mark.parametrize(
    "source",
    [
        "/// ```\n/// assert!(true);\n/// ```\nfn f() {}\n",
        "//! ```rust\n//! assert!(true);\n//! ```\n",
        "    /// ```no_run\n    /// f();\n    /// ```\n",
        "/// ```ignore,edition2021\n/// f();\n/// ```\n",
        "/// ~~~\n/// f();\n/// ~~~\n",
        "/// ```should_panic\n/// f();\n/// ```\n",
        "/**\n * ```\n * f();\n * ```\n */\n",
        "/*! ```\n f();\n ``` */\n",
        "/// Docs.\n///\n///     f();\n",
        "///     f();\n",
        '#![doc = include_str!("../README.md")]\n',
        '#![cfg_attr(doctest, doc = include_str!("../README.md"))]\n',
        '#[doc = "```\\nf();\\n```"]\n',
        '/// ```compile_fail,E0308\n/// let x: u8 = "";\n/// ```\n',
        "/// ```no_run,text\n/// f();\n/// ```\n",
        "/// # Examples\n///     f();\n",
        "/// ```text\n/// a\n/// ```\n///     f();\n",
    ],
)
def test_a_block_rustdoc_tests_is_found(source):
    assert doctests(source), source


@pytest.mark.parametrize(
    "source",
    [
        "/// ```text\n/// assert!(true);\n/// ```\nfn f() {}\n",
        "/// ```sh\n/// cargo run\n/// ```\n",
        "// ```\n// f();\n// ```\n",
        "//// ```\n",
        "/* ```\n f();\n ``` */\n",
        '#[doc = concat!("The values of `", $field, "`.")]\n',
        "#[doc(hidden)]\n",
        "/// - a list item\n///   continued\n",
        "/// Text\n///     still the paragraph.\n",
        'let s = "/// ```";\n',
    ],
)
def test_text_rustdoc_never_tests_passes(source):
    assert doctests(source) == [], source


def test_a_fence_closes_so_text_after_it_isnt_a_new_fence():
    source = "/// ```text\n/// a\n/// ```\n///\n/// ```sh\n/// b\n/// ```\n"
    assert doctests(source) == []


def test_a_doctest_is_named_by_its_line():
    source = "/// Docs.\n///\n/// ```\n/// f();\n/// ```\nfn f() {}\n"
    assert doctests(source) == [(3, "a Rust fence: ```")]


def test_a_fence_ends_with_its_doc_comment():
    # An unclosed ```text fence doesn't hide a doctest in the next item's docs.
    source = "/// ```text\nfn f() {}\n/// ```\n/// g();\n/// ```\nfn g() {}\n"
    assert doctests(source) == [(3, "a Rust fence: ```")]
