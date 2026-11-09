//! The pages for people (design.md, Pages for people): HTML for a browser that opens
//! a page, and the plain text agents get for everything else.

use crate::common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::{DELIMITER, PUBLIC_URL, Reply, TestApp, entry, test_app};
use http_body_util::BodyExt;
use tower::ServiceExt;

/// The headers a browser sends when a person opens a page.
fn opened(path: &str) -> Request<Body> {
    Request::get(path)
        .header("sec-fetch-dest", "document")
        .header("sec-fetch-mode", "navigate")
        .header(
            "accept",
            "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
        )
        .body(Body::empty())
        .unwrap()
}

/// What an agent's fetch tool sends that copies a browser's Accept header, without
/// the fetch metadata a browser adds.
fn fetched(path: &str) -> Request<Body> {
    Request::get(path)
        .header(
            "accept",
            "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
        )
        .body(Body::empty())
        .unwrap()
}

fn content_type(reply: &Reply) -> &str {
    reply
        .headers
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
}

fn header<'a>(reply: &'a Reply, name: &str) -> &'a str {
    reply
        .headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
}

async fn public_entry(app: &TestApp) -> String {
    app.submit(&entry("A steering result", "")).await
}

#[tokio::test]
async fn a_browser_opening_an_entry_gets_html_and_an_agent_gets_todays_text() {
    let app = test_app().await;
    let id = public_entry(&app).await;
    let path = format!("/e/{id}");
    let text = app.get(&path).await;
    assert!(content_type(&text).starts_with("text/plain"), "{text:?}");

    let fetched = app.send(fetched(&path)).await;
    assert_eq!(fetched.status, StatusCode::OK);
    assert_eq!(
        fetched.body, text.body,
        "an Accept of text/html alone gets the text"
    );

    let page = app.send(opened(&path)).await;
    assert_eq!(page.status, StatusCode::OK);
    assert!(content_type(&page).starts_with("text/html"), "{page:?}");
    assert!(page.body.starts_with("<!DOCTYPE html>"), "{}", page.body);
}

#[tokio::test]
async fn every_answer_says_it_varies_by_fetch_destination_and_accept() {
    let app = test_app().await;
    let id = public_entry(&app).await;
    for path in [
        "/",
        "/search?q=steering",
        &format!("/e/{id}"),
        "/e/missing",
        "/tags",
    ] {
        for reply in [app.get(path).await, app.send(opened(path)).await] {
            let vary = header(&reply, "vary").to_ascii_lowercase();
            assert!(
                vary.contains("sec-fetch-dest") && vary.contains("accept"),
                "{path}: {vary:?}"
            );
        }
    }
}

#[tokio::test]
async fn json_wins_over_html_when_accept_asks_for_it() {
    let app = test_app().await;
    public_entry(&app).await;
    let reply = app
        .send(
            Request::get("/search?q=steering")
                .header("sec-fetch-dest", "document")
                .header("accept", "application/json")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert!(
        content_type(&reply).starts_with("application/json"),
        "{reply:?}"
    );
}

#[tokio::test]
async fn the_home_page_puts_the_line_for_agents_before_the_first_card() {
    let app = test_app().await;
    let id = public_entry(&app).await;
    let page = app.send(opened("/")).await;
    assert_eq!(page.status, StatusCode::OK);
    assert!(content_type(&page).starts_with("text/html"));
    let line = format!("Read {PUBLIC_URL}/skill.md, then check File Drawer for the experiment we");
    let at_line = page
        .body
        .find(&line)
        .unwrap_or_else(|| panic!("{}", page.body));
    let at_card = page
        .body
        .find(&id)
        .expect("the latest entry is on the page");
    assert!(at_line < at_card);
}

#[tokio::test]
async fn an_agent_opening_the_home_page_gets_a_few_lines_that_point_to_the_skill() {
    let app = test_app().await;
    let reply = app.get("/").await;
    assert_eq!(reply.status, StatusCode::OK);
    assert!(content_type(&reply).starts_with("text/plain"), "{reply:?}");
    for path in ["/skill.md", "/search?q=", "/cards.txt"] {
        assert!(
            reply.body.contains(&format!("{PUBLIC_URL}{path}")),
            "{path}: {}",
            reply.body
        );
    }
    assert!(reply.body.lines().count() <= 8, "{}", reply.body);
    assert!(!reply.body.contains('<'), "{}", reply.body);
}

#[tokio::test]
async fn the_entry_page_shows_no_entry_text_above_the_delimiter() {
    let app = test_app().await;
    let id = public_entry(&app).await;
    let page = app.send(opened(&format!("/e/{id}"))).await;
    assert_eq!(page.body.matches(DELIMITER).count(), 1, "{}", page.body);
    let (above, below) = page.body.split_once(DELIMITER).unwrap();
    for text in [
        "A steering result",
        "Steering at layer 14",
        "Command not recorded",
    ] {
        assert!(
            !above.contains(text),
            "{text} is above the delimiter:\n{above}"
        );
        assert!(below.contains(text), "{text} is missing below it:\n{below}");
    }
    assert!(
        above.contains(&format!("<title>Entry {id} · null · File Drawer</title>")),
        "{above}"
    );
}

/// The page for a public entry whose `## Reproduce` section is `reproduce`.
async fn entry_page_with(app: &TestApp, reproduce: &str) -> (String, Reply) {
    let text = entry("Markup test", "").replace("Command not recorded.", reproduce);
    let id = app.submit(&text).await;
    let page = app.send(opened(&format!("/e/{id}"))).await;
    assert_eq!(page.status, StatusCode::OK, "{}", page.body);
    (id, page)
}

/// The part of a page below its delimiter, where the entry's own text is.
fn below_delimiter(page: &Reply) -> &str {
    page.body
        .split_once(DELIMITER)
        .map_or("", |(_, below)| below)
}

#[tokio::test]
async fn raw_html_in_an_entry_is_dropped_and_the_page_says_so() {
    let app = test_app().await;
    let (id, page) = entry_page_with(
        &app,
        "Words <b>kept</b> inline.\n\n<script>alert(1)</script>\n\n\
         <img src=x onerror=alert(2)>\n\n<div class=\"stamp\">EFFECT</div>",
    )
    .await;
    let below = below_delimiter(&page);
    assert!(!below.contains("<script"), "{below}");
    assert!(!below.contains("<img"), "{below}");
    assert!(!below.contains("class=\"stamp\">EFFECT"), "{below}");
    assert!(below.contains("Words kept inline."), "{below}");
    assert!(below.contains(&format!("href=\"/e/{id}.txt\"")), "{below}");
    let policy = header(&page, "content-security-policy");
    assert!(policy.contains("default-src 'none'"), "{policy:?}");
    assert!(!policy.contains("script-src"), "{policy:?}");
}

#[tokio::test]
async fn an_entry_with_nothing_dropped_has_no_notice() {
    let app = test_app().await;
    let (id, page) = entry_page_with(
        &app,
        "Run `make` and see [the code](https://example.org/code).",
    )
    .await;
    let below = below_delimiter(&page);
    assert!(!below.contains(&format!("href=\"/e/{id}.txt\"")), "{below}");
    assert!(
        below.contains(r#"<a href="https://example.org/code" rel="nofollow ugc">the code</a>"#),
        "{below}"
    );
}

#[tokio::test]
async fn links_keep_only_http_and_https_however_the_scheme_is_written() {
    let app = test_app().await;
    let (id, page) = entry_page_with(
        &app,
        "[a](javascript:alert(1)) [b](<java&#9;script:alert(2)>) [c](&#106;avascript:alert(3)) \
         [d](data:text/html,hi) [e](//elsewhere.example/x) [f](/\\elsewhere.example/y) \
         [g](../search?q=steering) <someone@example.org>",
    )
    .await;
    let below = below_delimiter(&page);
    assert!(!below.to_ascii_lowercase().contains("script:"), "{below}");
    assert!(!below.contains("data:"), "{below}");
    assert!(!below.contains("mailto:"), "{below}");
    assert!(below.contains("someone@example.org"), "{below}");
    assert!(!below.contains("someone@example.org</a>"), "{below}");
    for (href, text) in [
        ("http://elsewhere.example/x", "e"),
        ("http://elsewhere.example/y", "f"),
        ("http://localhost/search?q=steering", "g"),
    ] {
        assert!(
            below.contains(&format!(
                r#"<a href="{href}" rel="nofollow ugc">{text}</a>"#
            )),
            "{href}: {below}"
        );
    }
    assert!(below.contains(&format!("href=\"/e/{id}.txt\"")), "{below}");
}

#[tokio::test]
async fn images_become_links_and_an_image_in_a_link_its_alt_text() {
    let app = test_app().await;
    let (_, page) = entry_page_with(
        &app,
        "![chart](https://tracker.example/pixel.png)\n\n\
         [![build](https://badges.example/b.svg)](https://ci.example/run)",
    )
    .await;
    let below = below_delimiter(&page);
    assert!(!below.contains("<img"), "{below}");
    assert!(
        below.contains(
            r#"<a href="https://tracker.example/pixel.png" rel="nofollow ugc">image: chart</a>"#
        ),
        "{below}"
    );
    assert!(
        below.contains(r#"<a href="https://ci.example/run" rel="nofollow ugc">build</a>"#),
        "{below}"
    );
    assert!(!below.contains("badges.example"), "{below}");
}

#[tokio::test]
async fn author_headings_sit_below_the_pages_own_and_carry_no_attributes() {
    let app = test_app().await;
    let (_, page) = entry_page_with(&app, "# Big claim {.stamp #delimiter}\n\nText.").await;
    let below = below_delimiter(&page);
    assert!(!below.contains("<h1>Big claim"), "{below}");
    assert!(
        below.contains("<h3>Big claim {.stamp #delimiter}</h3>"),
        "{below}"
    );
    assert!(!below.contains(r#"class="stamp">Big"#), "{below}");
}

#[tokio::test]
async fn the_txt_address_is_the_plain_text_for_browsers_and_agents_alike() {
    let app = test_app().await;
    let id = public_entry(&app).await;
    let text = app.get(&format!("/e/{id}")).await;
    for reply in [
        app.get(&format!("/e/{id}.txt")).await,
        app.send(opened(&format!("/e/{id}.txt"))).await,
    ] {
        assert_eq!(reply.status, StatusCode::OK);
        assert!(content_type(&reply).starts_with("text/plain"), "{reply:?}");
        assert_eq!(reply.body, text.body);
    }
}

#[tokio::test]
async fn a_browser_header_with_a_token_still_gets_plain_text() {
    let app = test_app().await;
    let reply = app
        .send(
            Request::get("/")
                .header("sec-fetch-dest", "document")
                .header("authorization", format!("Bearer {}", app.token))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert!(content_type(&reply).starts_with("text/plain"), "{reply:?}");
}

#[tokio::test]
async fn a_browser_behind_stagings_basic_auth_still_gets_the_page() {
    let app = test_app().await;
    // Caddy passes the browser's basic-auth header on to the server
    // (ops/remote/caddy.sh), so staging's pages arrive with it.
    let reply = app
        .send(
            Request::get("/")
                .header("sec-fetch-dest", "document")
                .header("authorization", "Basic b3duZXI6c2VjcmV0")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert!(content_type(&reply).starts_with("text/html"), "{reply:?}");
}

#[tokio::test]
async fn an_unknown_path_is_a_404_with_a_body_and_a_page_for_a_browser() {
    let app = test_app().await;
    let text = app.get("/no/such/page").await;
    assert_eq!(text.status, StatusCode::NOT_FOUND);
    assert!(!text.body.is_empty());
    let page = app.send(opened("/no/such/page")).await;
    assert_eq!(page.status, StatusCode::NOT_FOUND);
    assert!(content_type(&page).starts_with("text/html"), "{page:?}");
}

#[tokio::test]
async fn a_browser_over_the_ip_limit_gets_a_page_that_says_when_to_retry() {
    let app = test_app()
        .await
        .with_limits(filedrawer_server::limits::Limits {
            per_ip: common::through_caddy::per_minute(1),
            ..common::through_caddy::wide()
        });
    app.send_from("192.0.2.7:1000", opened("/")).await;
    let page = app.send_from("192.0.2.7:1000", opened("/")).await;
    assert_eq!(page.status, StatusCode::TOO_MANY_REQUESTS);
    assert!(content_type(&page).starts_with("text/html"), "{page:?}");
    assert!(!header(&page, "retry-after").is_empty(), "{page:?}");
}

#[tokio::test]
async fn every_answer_carries_the_framing_and_sniffing_guards() {
    let app = test_app().await;
    for path in ["/", "/tags", "/skill.md", "/login"] {
        let reply = app.get(path).await;
        assert_eq!(
            header(&reply, "x-content-type-options"),
            "nosniff",
            "{path}"
        );
        assert_eq!(header(&reply, "x-frame-options"), "DENY", "{path}");
        assert!(!header(&reply, "cache-control").is_empty(), "{path}");
    }
    // Every HTML page, including those that are HTML for every client.
    let login = app.get("/login").await;
    let policy = header(&login, "content-security-policy");
    assert!(policy.contains("default-src 'none'"), "{policy:?}");
    assert!(!policy.contains("script-src"), "{policy:?}");
}

#[tokio::test]
async fn an_unknown_filter_value_is_repeated_only_in_part() {
    let app = test_app().await;
    let sentence =
        "Your account is suspended; appeal at once by mailing the address on our help page";
    let reply = app
        .get(&format!("/search?outcome={}", sentence.replace(' ', "+")))
        .await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
    assert!(!reply.body.contains("help page"), "{}", reply.body);
}

#[tokio::test]
async fn a_missing_entry_opened_in_a_browser_is_a_page_with_the_same_status() {
    let app = test_app().await;
    let page = app.send(opened("/e/doesnotexist")).await;
    assert_eq!(page.status, StatusCode::NOT_FOUND);
    assert!(content_type(&page).starts_with("text/html"), "{page:?}");
    let text = app.get("/e/doesnotexist").await;
    assert!(content_type(&text).starts_with("text/plain"), "{text:?}");
}

#[tokio::test]
async fn search_results_opened_in_a_browser_are_cards_with_the_query_kept() {
    let app = test_app().await;
    let id = public_entry(&app).await;
    let page = app
        .send(opened("/search?q=steering&outcome=no_effect"))
        .await;
    assert_eq!(page.status, StatusCode::OK);
    assert!(content_type(&page).starts_with("text/html"));
    assert!(
        page.body.contains(&format!("href=\"/e/{id}\"")),
        "{}",
        page.body
    );
    assert!(page.body.contains(r#"value="steering""#), "{}", page.body);
    // Each outcome tab keeps the words searched for.
    for href in [
        "/search?q=steering",
        "/search?q=steering&#38;outcome=effect",
    ] {
        assert!(
            page.body.contains(&format!("href=\"{href}\"")),
            "{href}: {}",
            page.body
        );
    }
    let text = app
        .send(fetched("/search?q=steering&outcome=no_effect"))
        .await;
    assert!(text.body.contains(&id));
    assert!(content_type(&text).starts_with("text/plain"));
}

#[tokio::test]
async fn the_older_link_shows_only_when_there_is_an_older_page() {
    let app = test_app().await;
    public_entry(&app).await;
    let one = app.send(opened("/search?limit=1")).await;
    assert!(!one.body.contains("Older"), "{}", one.body);
    app.submit(&entry("A second result", "")).await;
    let two = app.send(opened("/search?limit=1")).await;
    assert!(
        two.body.contains(r#"href="/search?limit=1&#38;page=2""#),
        "{}",
        two.body
    );
    let last = app.send(opened("/search?limit=1&page=2")).await;
    assert!(!last.body.contains("Older"), "{}", last.body);
}

#[tokio::test]
async fn the_search_pages_title_repeats_the_words_only_in_part() {
    let app = test_app().await;
    let words = "account suspended appeal now by mailing the address on our help page";
    let page = app
        .send(opened(&format!("/search?q={}", words.replace(' ', "+"))))
        .await;
    let title = page
        .body
        .split_once("<title>")
        .and_then(|(_, rest)| rest.split_once("</title>"))
        .map_or("", |(title, _)| title);
    assert!(title.starts_with("Search: account suspended"), "{title}");
    assert!(!title.contains("help page"), "{title}");
}

#[tokio::test]
async fn the_stylesheet_and_fonts_come_from_the_site() {
    let app = test_app().await;
    let page = app.send(opened("/")).await;
    let start = page
        .body
        .find("href=\"/static/site.css")
        .expect("a stylesheet link");
    let href: String = page.body[start + 6..]
        .chars()
        .take_while(|c| *c != '"')
        .collect();
    let css = app.get(&href).await;
    assert_eq!(css.status, StatusCode::OK);
    assert!(content_type(&css).starts_with("text/css"), "{css:?}");
    assert!(css.body.contains("Courier Prime"));
    // A font isn't UTF-8, so it skips `send`, which reads bodies as text.
    let font = app
        .router
        .clone()
        .oneshot(
            Request::get("/static/fonts/courier-prime-5.2.8-latin-400-normal.woff2")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(font.status(), StatusCode::OK);
    let font_header = |name: &str| {
        font.headers()
            .get(name)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("")
            .to_string()
    };
    assert_eq!(font_header("content-type"), "font/woff2");
    assert!(font_header("cache-control").contains("max-age"));
    let bytes = font.into_body().collect().await.unwrap().to_bytes();
    assert!(bytes.starts_with(b"wOF2"), "not a woff2 file");
    assert!(!page.body.contains("googleapis"), "{}", page.body);
}

#[tokio::test]
async fn a_public_series_opened_in_a_browser_shows_its_description_below_the_delimiter() {
    let app = test_app().await;
    let (cookie, token) = common::browser_account(&app, "grace@example.com").await;
    let reply = app
        .post_as(
            "/series",
            Some(&token),
            common::series::series_doc(
                "Layer sweep",
                "Shared setup.\n\n## Models\n\nmodel-a, <b>bold</b> by the API.\n",
            ),
        )
        .await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
    let id = common::id_in(&reply.body);
    // A version is readable once a public entry cites it (design.md, Series).
    let citing = common::series::citing("Cites the sweep", &format!("{id}@1"), "");
    common::series::submit_with(&app, &token, &citing).await;
    common::series::make_public(&app, &cookie, &id).await;

    let page = app.send(opened(&format!("/series/{id}"))).await;
    assert_eq!(page.status, StatusCode::OK, "{}", page.body);
    assert!(content_type(&page).starts_with("text/html"), "{page:?}");
    let (above, below) = page.body.split_once(DELIMITER).expect("a delimiter");
    assert!(!above.contains("Layer sweep"), "{above}");
    assert!(below.contains("Layer sweep"), "{below}");
    assert!(below.contains("<h4>Models</h4>"), "{below}");
    assert!(below.contains("model-a, bold by the API."), "{below}");
    assert!(
        below.contains(&format!("href=\"/series/{id}.txt\"")),
        "{below}"
    );

    let text = app.get(&format!("/series/{id}")).await;
    let txt = app.send(opened(&format!("/series/{id}.txt"))).await;
    assert!(content_type(&txt).starts_with("text/plain"), "{txt:?}");
    assert_eq!(txt.body, text.body);
}

#[tokio::test]
async fn json_wins_over_the_entry_page_when_accept_asks_for_it() {
    let app = test_app().await;
    let id = public_entry(&app).await;
    let reply = app
        .send(
            Request::get(format!("/e/{id}"))
                .header("sec-fetch-dest", "document")
                .header("accept", "application/json")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert!(
        content_type(&reply).starts_with("application/json"),
        "{reply:?}"
    );
}

#[tokio::test]
async fn an_entry_not_yet_public_is_a_not_found_page_for_a_browser() {
    let app = test_app().await;
    let id = app.submit_held(&entry("Still pending", "")).await;
    let page = app.send(opened(&format!("/e/{id}"))).await;
    assert_eq!(page.status, StatusCode::NOT_FOUND);
    assert!(content_type(&page).starts_with("text/html"), "{page:?}");
    assert!(!page.body.contains("Still pending"), "{}", page.body);
}

#[tokio::test]
async fn aligned_table_columns_carry_no_inline_style_the_policy_would_block() {
    let app = test_app().await;
    let (_, page) = entry_page_with(&app, "| a | b |\n|:--|--:|\n| 1 | 2 |").await;
    let below = below_delimiter(&page);
    assert!(below.contains("<td>1</td>"), "{below}");
    assert!(!below.contains("style="), "{below}");
}

#[tokio::test]
async fn slug_links_go_to_the_entry_they_name_or_stay_text_without_a_notice() {
    let app = test_app().await;
    let earlier = app.submit(&entry("Baseline", "slug: baseline\n")).await;
    let (id, page) = entry_page_with(
        &app,
        "See [the baseline](slug:baseline) and [the next one](slug:not-yet).",
    )
    .await;
    let below = below_delimiter(&page);
    assert!(
        below.contains(&format!(
            r#"<a href="/e/{earlier}" rel="nofollow ugc">the baseline</a>"#
        )),
        "{below}"
    );
    assert!(below.contains("and the next one."), "{below}");
    assert!(!below.contains(&format!("href=\"/e/{id}.txt\"")), "{below}");
}
