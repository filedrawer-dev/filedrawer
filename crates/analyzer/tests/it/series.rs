//! Seam: `read_series` (design.md, Series).

use filedrawer_analyzer::read_series;

#[test]
fn a_series_title_that_shows_nothing_is_refused() {
    for title in ["\"\u{200b}\"", "\"\u{feff}\u{2060}\""] {
        let refused = read_series(&format!("---\ntitle: {title}\n---\n\nShared setup."));
        assert!(refused.is_err(), "{title}: {refused:?}");
    }
}

#[test]
fn a_series_description_that_shows_nothing_is_refused() {
    let refused = read_series("---\ntitle: A family\n---\n\n\u{200b}\u{2060}\n")
        .err()
        .unwrap_or_default();
    assert!(
        refused.iter().any(|reason| reason.contains("description")),
        "{refused:?}"
    );
}
