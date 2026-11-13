//! Seam: `Reference::parse` and `Reference::key`, the keys search's `cites` filter
//! matches (docs/entry-format.md, Fields).

use filedrawer_analyzer::Reference;
use proptest::prelude::*;

fn key(text: &str) -> Option<String> {
    Reference::parse(text).map(|reference| reference.key())
}

#[test]
fn every_way_of_writing_an_arxiv_paper_has_one_key() {
    for written in [
        "arXiv:2507.01413",
        "arxiv:2507.01413v2",
        "ARXIV:2507.01413",
        " arXiv:2507.01413 ",
        "https://arxiv.org/abs/2507.01413v3",
        "http://www.arxiv.org/abs/2507.01413",
        "https://export.arxiv.org/abs/2507.01413",
        "arxiv.org/abs/2507.01413",
        "www.arxiv.org/abs/2507.01413",
        "Export.arXiv.org/abs/2507.01413",
        "https://arxiv.org/pdf/2507.01413v2.pdf",
        "https://arxiv.org/pdf/2507.01413",
        "https://arxiv.org/html/2507.01413v1",
        "10.48550/arXiv.2507.01413",
        "10.48550/ARXIV.2507.01413",
        "https://doi.org/10.48550/arXiv.2507.01413",
    ] {
        assert_eq!(
            key(written).as_deref(),
            Some("arXiv:2507.01413"),
            "{written}"
        );
    }
    assert_eq!(
        Reference::parse("arXiv:2507.01413"),
        Some(Reference::Arxiv("2507.01413".into()))
    );
}

#[test]
fn arxiv_ids_follow_the_new_and_old_schemes() {
    assert_eq!(
        key("arXiv:2507.012345").as_deref(),
        Some("arXiv:2507.012345")
    );
    assert_eq!(
        key("arXiv:hep-th/9901001v2").as_deref(),
        Some("arXiv:hep-th/9901001")
    );
    // The old scheme's subject class isn't part of the ID.
    assert_eq!(
        key("arXiv:math.GT/0309136").as_deref(),
        Some("arXiv:math/0309136")
    );
    assert_eq!(
        key("https://arxiv.org/abs/math.GT/0309136").as_deref(),
        Some("arXiv:math/0309136")
    );
    for not_an_id in [
        "arXiv:",
        "arXiv:see body",
        "arXiv:2507.014",
        "arXiv:25070.1413",
        "arXiv:2507.01413, arXiv:2603.20281",
        "arXiv:hep-th/990100",
    ] {
        assert_eq!(key(not_an_id), None, "{not_an_id}");
    }
}

#[test]
fn a_doi_is_keyed_bare_and_lowercased_however_it_is_written() {
    for written in [
        "10.2139/ssrn.7439840",
        "10.2139/SSRN.7439840",
        "doi:10.2139/ssrn.7439840",
        "DOI:10.2139/ssrn.7439840",
        "https://doi.org/10.2139/ssrn.7439840",
        "http://dx.doi.org/10.2139/ssrn.7439840",
        "https://doi.org/10.2139%2Fssrn.7439840",
    ] {
        assert_eq!(
            key(written).as_deref(),
            Some("10.2139/ssrn.7439840"),
            "{written}"
        );
    }
    assert_eq!(
        key("10.1002/(SICI)1097-4571(199806)49:8<693::AID-ASI4>3.0.CO;2-0"),
        None,
        "angle brackets aren't in the DOI characters the format takes"
    );
    assert_eq!(
        key("10.1016/S0140-6736(97)11096-0").as_deref(),
        Some("10.1016/s0140-6736(97)11096-0")
    );
    // Only a doi.org link arrives percent-encoded; a DOI written out holds no `%`.
    for not_a_doi in [
        "10./x",
        "10.123/",
        "10.1/x; null; rows: 40 null",
        "11.1234/x",
        "10.1234/a%2Fb",
    ] {
        assert_eq!(key(not_a_doi), None, "{not_a_doi}");
    }
}

#[test]
fn an_entry_is_named_by_its_id_its_url_or_a_slug() {
    assert_eq!(
        Reference::parse("bcz2bqrf3b"),
        Some(Reference::Entry("bcz2bqrf3b".into()))
    );
    for written in [
        "BCZ2BQRF3B",
        "https://filedrawer.dev/e/bcz2bqrf3b",
        "https://staging.filedrawer.dev/e/bcz2bqrf3b.txt",
        "https://filedrawer.dev/e/bcz2bqrf3b/",
    ] {
        assert_eq!(key(written).as_deref(), Some("bcz2bqrf3b"), "{written}");
    }
    assert_eq!(
        Reference::parse("slug:psim-riemer-code-audit"),
        Some(Reference::Slug("psim-riemer-code-audit".into()))
    );
    assert_eq!(
        key("slug:psim-riemer-code-audit").as_deref(),
        Some("slug:psim-riemer-code-audit")
    );
    // `i`, `l`, `o` and `u` aren't in the ID alphabet, and IDs are 10 characters.
    for not_an_id in [
        "bcz2bqrf3i",
        "bcz2bqrf3",
        "bcz2bqrf3bb",
        "slug:",
        "slug:a b",
    ] {
        assert_eq!(Reference::parse(not_an_id), None, "{not_an_id}");
    }
}

#[test]
fn any_other_web_page_is_keyed_by_its_normalised_url() {
    for written in [
        "https://www.lesswrong.com/posts/abc/title",
        "HTTPS://LessWrong.com/posts/abc/title/",
        "https://lesswrong.com/posts/abc/title#comments",
    ] {
        assert_eq!(
            key(written).as_deref(),
            Some("https://lesswrong.com/posts/abc/title"),
            "{written}"
        );
    }
    assert_eq!(
        key("https://openreview.net/forum?id=AbC123xyz").as_deref(),
        Some("https://openreview.net/forum?id=AbC123xyz")
    );
    assert_eq!(
        Reference::parse("https://transformer-circuits.pub/2025/attribution-graphs/"),
        Some(Reference::Url(
            "https://transformer-circuits.pub/2025/attribution-graphs".into()
        ))
    );
    // A page on arXiv's or doi.org's host that names no paper is still a page.
    for page in [
        "https://arxiv.org/list/cs.AI/recent",
        "https://doi.org/about",
    ] {
        assert_eq!(key(page).as_deref(), Some(page), "{page}");
    }
    for not_a_page in [
        "ftp://example.com/x",
        "file:///etc/passwd",
        "https://",
        "Yun (2026)",
        "",
    ] {
        assert_eq!(key(not_a_page), None, "{not_a_page}");
    }
}

proptest! {
    #[test]
    fn a_key_read_back_gives_itself(
        written in prop_oneof![
            "arXiv:[0-9]{4}\\.[0-9]{4,5}(v[0-9])?",
            "arXiv:[a-z]{2,6}(-[a-z]{2})?(\\.[A-Z]{2})?/[0-9]{7}",
            "(doi:)?10\\.[0-9]{4,9}/[-._;()/:A-Za-z0-9]{1,20}",
            "[0-9a-hjkmnp-tv-z]{10}",
            "slug:[A-Za-z0-9._-]{1,20}",
            "https://(www\\.)?[a-z]{1,10}\\.(com|org|pub)(/[a-zA-Z0-9_-]{1,8}){0,3}/?(\\?q=[a-z]{1,5})?",
        ]
    ) {
        if let Some(first) = key(&written) {
            prop_assert_eq!(key(&first), Some(first.clone()));
        }
    }
}
