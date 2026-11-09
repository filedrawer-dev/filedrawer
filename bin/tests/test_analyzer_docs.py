"""Each source file of the analyzer opens with a `//!` header that points to a
section of docs/entry-format.md or docs/design.md, as `(entry-format.md, Fields)`, and
a section of entry-format.md exists (CODING_STANDARDS.md, Style). rustc's missing_docs
lint, which clippy turns into an error, checks that every public item, field and
variant has a doc comment; this checks what it can't, the pointers. A heading renamed in
entry-format.md fails here until the headers follow it. design.md's headings aren't
read, so a commit of design.md alone still runs no check (the owner's decision,
2026-11-09)."""

import re
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
SOURCES = REPO / "crates/analyzer/src"
FORMAT = REPO / "docs/entry-format.md"
POINTER = re.compile(r"(?:docs/)?(entry-format|design)\.md, ")


def headings(text: str) -> list[str]:
    """The markdown headings of `text`, leaving out those in fenced blocks, such as
    the entry an Example section shows."""
    found, fenced = [], False
    for line in text.splitlines():
        if line.startswith("```"):
            fenced = not fenced
        elif line.startswith("#") and not fenced:
            found.append(re.sub(r"^#+\s*", "", line).strip())
    return found


def names(rest: str, heading: str) -> bool:
    """Whether the text after a pointer's comma names `heading` whole: the heading,
    then the text's end or punctuation, so "Results" doesn't name "Results table"."""
    return rest.startswith(heading) and (
        len(rest) == len(heading) or rest[len(heading)] in ").;:,"
    )


def problems(name: str, source: str, sections: list[str]) -> list[str]:
    """What's wrong with the `//!` header of the file `name`, whose text is `source`,
    given entry-format.md's `sections`."""
    header = " ".join(
        line.removeprefix("//!").strip()
        for line in source.splitlines()
        if line.startswith("//!")
    )
    pointers = [(m[1], header[m.end() :]) for m in POINTER.finditer(header)]
    if not pointers:
        return [
            f"{name}: its //! header names no section of docs/entry-format.md or "
            "docs/design.md; add one, as `(entry-format.md, Fields)`"
        ]
    return [
        f"{name}: `entry-format.md, {rest[:40]}` names no heading of docs/entry-format.md"
        for doc, rest in pointers
        if doc == "entry-format"
        and not any(names(rest, heading) for heading in sections)
    ]


def test_every_analyzer_file_points_to_a_section_that_exists():
    sections = headings(FORMAT.read_text())
    found = [
        problem
        for path in sorted(SOURCES.glob("*.rs"))
        for problem in problems(path.name, path.read_text(), sections)
    ]
    assert found == [], "\n".join(found)


SECTIONS = ["Fields", "Accept, warn, and re-render"]


def test_a_pointer_to_a_heading_passes_after_inner_attributes():
    source = "#![forbid(unsafe_code)]\n//! Fields (entry-format.md, Fields).\npub fn f() {}\n"
    assert problems("a.rs", source, SECTIONS) == []


def test_a_heading_with_commas_is_matched_whole():
    source = "//! Warnings\n//! (docs/entry-format.md, Accept, warn, and re-render).\n"
    assert problems("a.rs", source, SECTIONS) == []


def test_a_pointer_to_no_heading_is_named():
    found = problems("a.rs", "//! Things (entry-format.md, Hosting).\n", SECTIONS)
    assert found == [
        "a.rs: `entry-format.md, Hosting).` names no heading of docs/entry-format.md"
    ]


def test_a_pointer_into_design_md_counts_unread():
    assert problems("a.rs", "//! Hosts (design.md, Any heading).\n", SECTIONS) == []


def test_a_header_with_no_pointer_is_named():
    for source in ("//! Reads docs/entry-format.md: everything.\n", "pub fn f() {}\n"):
        found = problems("a.rs", source, SECTIONS)
        assert len(found) == 1 and "names no section" in found[0], found


def test_a_heading_only_counts_whole_and_outside_fenced_examples():
    doc = (
        "# Format\n\n## The Results table\n\n## Example\n\n```markdown\n## Setup\n```\n"
    )
    sections = headings(doc)
    assert "Setup" not in sections
    for wrong in ("Results table", "The Results tableau", "Setup"):
        found = problems("a.rs", f"//! x (entry-format.md, {wrong}).\n", sections)
        assert len(found) == 1, (wrong, found)
    assert (
        problems("a.rs", "//! x (entry-format.md, The Results table).\n", sections)
        == []
    )
    assert (
        problems("a.rs", "//! x (entry-format.md, The Results table: `n`).\n", sections)
        == []
    )
