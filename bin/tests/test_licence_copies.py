"""The published crates carry copies of the repo's LICENSE (design.md, Distribution):
cargo and maturin package only files inside a crate's folder, and a `license-file`
outside it makes cargo warn on every build. This keeps each copy the same as the
original."""

from pathlib import Path

import pytest

REPO = Path(__file__).resolve().parents[2]


@pytest.mark.parametrize("crate", ["analyzer", "cli"])
def test_each_published_crate_holds_the_repo_licence(crate):
    assert (REPO / "crates" / crate / "LICENSE").read_bytes() == (
        REPO / "LICENSE"
    ).read_bytes()
