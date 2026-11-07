//! Seam: the router, with a fake web (`common::fake_web`): the link checks that
//! submissions run on `code` and `logs` links (design.md, Submission and screening,
//! step 2), what they warn and refuse, and the card's flags.

mod common;

use axum::http::StatusCode;
use common::{TestApp, entry, id_in, test_app};

const SHA: &str = "0123456789abcdef0123456789abcdef01234567";

fn github_code() -> String {
    format!("https://github.com/ada/steering/tree/{SHA}")
}

fn with_code(url: &str) -> String {
    entry("Linked", &format!("code: {url}\n"))
}

fn with_logs(url: &str) -> String {
    entry("Linked", &format!("logs: [{url}]\n"))
}

/// The `warning: ` lines of a plain-text reply.
fn warnings(body: &str) -> Vec<&str> {
    body.lines()
        .filter_map(|line| line.strip_prefix("warning: "))
        .collect()
}

/// The card's line of flags in a plain-text reply.
fn flags(body: &str) -> &str {
    body.lines()
        .map(str::trim)
        .find(|line| line.starts_with("analyzer v"))
        .expect("the reply holds a card")
}

async fn public_card(app: &TestApp, id: &str) -> String {
    let reply = app.get(&format!("/e/{id}")).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    flags(&reply.body).to_string()
}

#[tokio::test]
async fn with_link_checks_off_nothing_is_fetched_or_flagged() {
    let mut app = test_app().await;
    app.state.link_checker =
        std::sync::Arc::new(filedrawer_server::link_checks::LinkChecker::off());
    app.router = filedrawer_server::app(app.state.clone());
    let reply = app
        .post("/entries", with_code("https://zenodo.org/records/1"))
        .await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
    assert_eq!(link_warnings(&reply.body), Vec::<&str>::new());
    assert_eq!(flags(&reply.body), "analyzer v1");
}

#[tokio::test]
async fn a_dead_code_link_draws_a_warning_and_the_cards_flag() {
    let app = test_app().await;
    app.web.answer(&github_code(), 404);
    let reply = app.post("/entries", with_code(&github_code())).await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
    let warned = warnings(&reply.body);
    assert_eq!(warned.len(), 2, "{warned:?}");
    assert!(
        warned[0].contains(&format!("`code: {}` answered 404", github_code())),
        "{warned:?}"
    );
    assert!(flags(&reply.body).contains("; code link dead"));
    // A dead link can't take GitHub's check, so the pin isn't verified either.
    assert!(flags(&reply.body).contains("; pin not verified"));

    let id = id_in(&reply.body);
    let own = app.get_as_author(&format!("/e/{id}")).await;
    assert_eq!(warnings(&own.body), warned);
    app.post(&format!("/e/{id}/publish"), "").await;
    assert!(public_card(&app, &id).await.contains("; code link dead"));
}

#[tokio::test]
async fn a_dead_logs_link_flags_the_logs() {
    let app = test_app().await;
    let logs = format!("https://huggingface.co/datasets/ada/logs/tree/{SHA}");
    // Hugging Face answers 401 for a private or missing repository.
    app.web.answer(&logs, 401);
    let reply = app.post("/entries", with_logs(&logs)).await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
    assert_eq!(
        warnings(&reply.body),
        [format!(
            "`logs: {logs}` answered 401, so the card says \"logs link dead\"; the server checks it again each week"
        )]
    );
    assert!(flags(&reply.body).ends_with("; logs link dead"));
}

#[tokio::test]
async fn a_live_link_draws_nothing() {
    let app = test_app().await;
    let reply = app.post("/entries", with_code(&github_code())).await;
    assert_eq!(warnings(&reply.body), Vec::<&str>::new());
    assert_eq!(flags(&reply.body), "analyzer v1");
}

#[tokio::test]
async fn a_redirect_off_the_listed_hosts_is_dead_and_isnt_followed() {
    let app = test_app().await;
    let logs = "https://github.com/ada/steering/releases/download/v1/run.log";
    app.web
        .redirect(logs, 302, "http://169.254.169.254/latest/meta-data");
    let reply = app.post("/entries", with_logs(logs)).await;
    assert_eq!(
        warnings(&reply.body),
        [format!(
            "`logs: {logs}` redirects to 169.254.169.254, which the server doesn't fetch, so the card says \"logs link dead\"; the server checks it again each week"
        )]
    );
    assert_eq!(app.web.fetched(), [logs]);
}

#[tokio::test]
async fn a_redirect_to_a_download_domain_is_live_and_isnt_fetched() {
    let app = test_app().await;
    let logs = format!("https://github.com/ada/steering/archive/{SHA}.zip");
    app.web.redirect(
        &logs,
        302,
        &format!("https://codeload.github.com/ada/steering/zip/{SHA}"),
    );
    let hf = format!("https://huggingface.co/datasets/ada/logs/resolve/{SHA}/run.jsonl");
    app.web.redirect(
        &hf,
        302,
        "https://cas-bridge.xethub.hf.co/xet-bridge-us/abc",
    );
    let text = entry("Linked", &format!("logs: [{logs}, {hf}]\n"));
    let reply = app.post("/entries", text).await;
    assert_eq!(warnings(&reply.body), Vec::<&str>::new());
    let mut fetched = app.web.fetched();
    fetched.sort();
    assert_eq!(fetched, [logs, hf]);
}

#[tokio::test]
async fn a_github_release_asset_is_live_unfetched() {
    let app = test_app().await;
    let logs = "https://github.com/ada/steering/releases/download/v1/run.log";
    app.web.redirect(
        logs,
        302,
        "https://release-assets.githubusercontent.com/github-production-release-asset/1/2",
    );
    let reply = app.post("/entries", with_logs(logs)).await;
    assert_eq!(warnings(&reply.body), Vec::<&str>::new());
    assert_eq!(app.web.fetched(), [logs]);
}

#[tokio::test]
async fn a_link_with_a_user_isnt_fetched_and_counts_as_dead() {
    let app = test_app().await;
    let logs = format!("https://someone@huggingface.co/datasets/ada/logs/tree/{SHA}");
    let reply = app.post("/entries", with_logs(&logs)).await;
    assert!(
        flags(&reply.body).ends_with("; logs link dead"),
        "{}",
        reply.body
    );
    assert_eq!(app.web.fetched(), Vec::<String>::new());
}

#[tokio::test]
async fn a_redirect_within_the_listed_hosts_is_followed() {
    let app = test_app().await;
    let logs = format!("https://huggingface.co/datasets/old/logs/tree/{SHA}");
    let moved = format!("https://huggingface.co/datasets/new/logs/tree/{SHA}");
    app.web
        .redirect(&logs, 307, &format!("/datasets/new/logs/tree/{SHA}"));
    app.web.answer(&moved, 404);
    let reply = app.post("/entries", with_logs(&logs)).await;
    assert!(
        flags(&reply.body).contains("; logs link dead"),
        "{}",
        reply.body
    );
    assert_eq!(app.web.fetched(), [logs, moved]);
}

#[tokio::test]
async fn a_link_that_doesnt_answer_or_answers_5xx_is_not_dead() {
    let app = test_app().await;
    let busy = format!("https://huggingface.co/datasets/ada/a/tree/{SHA}");
    let down = format!("https://huggingface.co/datasets/ada/b/tree/{SHA}");
    app.web.answer(&busy, 503);
    app.web.fail(&down);
    let text = entry("Linked", &format!("logs: [{busy}, {down}]\n"));
    let reply = app.post("/entries", text).await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
    assert_eq!(
        warnings(&reply.body),
        [
            format!(
                "the server couldn't check `logs: {busy}`: it answered 503; it tries again within a week"
            ),
            format!(
                "the server couldn't check `logs: {down}`: it didn't answer (timed out); it tries again within a week"
            ),
        ]
    );
    assert_eq!(flags(&reply.body), "analyzer v1; no code");
}

#[tokio::test]
async fn only_code_and_logs_links_on_listed_hosts_are_fetched() {
    let app = test_app().await;
    let text = entry(
        "Linked",
        &format!(
            "code: https://gitlab.com/ada/steering/-/tree/{SHA}\n\
             logs: [https://wandb.ai/ada/steering/runs/abc, https://github.com:8443/ada/x]\n\
             data:\n  - name: prompts\n    url: https://github.com/ada/prompts\n"
        ),
    )
    .replace(
        "not recorded",
        "See [the repo](https://github.com/ada/steering) and https://huggingface.co/ada.",
    );
    let reply = app.post("/entries", text).await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
    assert_eq!(app.web.fetched(), Vec::<String>::new());
    assert_eq!(app.web.api_calls(), Vec::<String>::new());
}

#[tokio::test]
async fn a_submission_refused_for_anything_else_fetches_nothing() {
    let app = test_app().await;
    let text = entry(
        "Linked",
        &format!("code: {}\nattachments: as-none\n", github_code()),
    );
    let reply = app.post("/entries", text).await;
    assert_eq!(
        reply.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        reply.body
    );
    assert_eq!(app.web.fetched(), Vec::<String>::new());
}

fn compare_path(base: &str, sha: &str) -> String {
    format!("/repos/ada/steering/compare/{base}...{sha}?per_page=1")
}

const BRANCHES: &str = "/repos/ada/steering/branches?per_page=100";

/// A 40-character hash made of `digit`.
fn hash(digit: char) -> String {
    digit.to_string().repeat(40)
}

/// GitHub's API says `SHA` isn't on the default branch, whose head is `aaa…`, and
/// lists `others` more branches, each with its own head.
fn not_on_default(app: &TestApp, others: usize) {
    app.web.api(
        &compare_path("HEAD", SHA),
        200,
        serde_json::json!({"status": "ahead", "base_commit": {"sha": hash('a')}}),
        4000,
    );
    let mut branches = vec![serde_json::json!({"name": "main", "commit": {"sha": hash('a')}})];
    for n in 0..others {
        branches.push(
            serde_json::json!({"name": format!("b{n}"), "commit": {"sha": format!("{n:040}")}}),
        );
    }
    app.web
        .api(BRANCHES, 200, serde_json::json!(branches), 4000);
}

#[tokio::test]
async fn a_commit_on_the_default_branch_is_verified_with_one_call() {
    let app = test_app().await;
    let reply = app.post("/entries", with_code(&github_code())).await;
    assert_eq!(warnings(&reply.body), Vec::<&str>::new());
    assert_eq!(app.web.api_calls(), [compare_path("HEAD", SHA)]);
}

#[tokio::test]
async fn a_commit_on_another_branch_is_verified() {
    let app = test_app().await;
    not_on_default(&app, 3);
    app.web.api(
        &compare_path(&format!("{:040}", 0), SHA),
        200,
        serde_json::json!({"status": "ahead"}),
        4000,
    );
    app.web.api(
        &compare_path(&format!("{:040}", 1), SHA),
        200,
        serde_json::json!({"status": "behind"}),
        4000,
    );
    let reply = app.post("/entries", with_code(&github_code())).await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
    assert_eq!(warnings(&reply.body), Vec::<&str>::new());
    assert_eq!(
        app.web.api_calls(),
        [
            compare_path("HEAD", SHA),
            BRANCHES.to_string(),
            compare_path(&format!("{:040}", 0), SHA),
            compare_path(&format!("{:040}", 1), SHA),
        ]
    );
}

#[tokio::test]
async fn a_commit_on_none_of_the_repositorys_branches_is_refused() {
    let app = test_app().await;
    not_on_default(&app, 12);
    for n in 0..12 {
        app.web.api(
            &compare_path(&format!("{n:040}"), SHA),
            200,
            serde_json::json!({"status": "diverged"}),
            4000,
        );
    }
    let reply = app.post("/entries", with_code(&github_code())).await;
    assert_eq!(
        reply.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        reply.body
    );
    assert!(
        reply.body.contains(&format!(
            "`code: {}`: commit {SHA} isn't on ada/steering's default branch or the 10 other branches the server checked, of 12; GitHub shows a fork's commits under its parent's URL too, so link the repository whose branch holds it",
            github_code()
        )),
        "{}",
        reply.body
    );
    // The default branch, the list, and 10 others.
    assert_eq!(app.web.api_calls().len(), 12);
}

#[tokio::test]
async fn a_small_repositorys_branches_are_all_checked() {
    let app = test_app().await;
    not_on_default(&app, 2);
    for n in 0..2 {
        app.web.api(
            &compare_path(&format!("{n:040}"), SHA),
            200,
            serde_json::json!({"status": "ahead"}),
            4000,
        );
    }
    let reply = app.post("/entries", with_code(&github_code())).await;
    assert_eq!(
        reply.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        reply.body
    );
    assert!(
        reply.body.contains(&format!(
            "commit {SHA} isn't on any branch of ada/steering; GitHub shows a fork's commits"
        )),
        "{}",
        reply.body
    );
}

#[tokio::test]
async fn a_confirmed_commit_costs_no_calls_the_next_time() {
    let app = test_app().await;
    app.submit_held(&with_code(&github_code())).await;
    app.submit_held(&with_code(&github_code())).await;
    assert_eq!(app.web.api_calls().len(), 1);
}

#[tokio::test]
async fn an_account_past_its_share_of_githubs_calls_gets_pin_not_verified() {
    // 30 calls an hour, of which an account may spend 3.
    let app = test_app().await.with_github_hourly(30);
    for digit in ['1', '2', '3'] {
        let url = format!("https://github.com/ada/steering/tree/{}", hash(digit));
        let reply = app.post("/entries", with_code(&url)).await;
        assert_eq!(warnings(&reply.body), Vec::<&str>::new(), "{}", reply.body);
    }
    let fourth = format!("https://github.com/ada/steering/tree/{}", hash('4'));
    let reply = app.post("/entries", with_code(&fourth)).await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
    assert_eq!(
        warnings(&reply.body),
        [format!(
            "the server couldn't confirm the pin of `code: {fourth}`: this account has used its share of the server's GitHub API calls for the hour; the card says \"pin not verified\" until a weekly recheck confirms it"
        )]
    );
    assert!(flags(&reply.body).ends_with("; pin not verified"));
    assert_eq!(app.web.api_calls().len(), 3);

    // Another account still has its own share.
    let other = app.other_token().await;
    let reply = app
        .post_as("/entries", Some(&other), with_code(&fourth))
        .await;
    assert_eq!(warnings(&reply.body), Vec::<&str>::new(), "{}", reply.body);
}

#[tokio::test]
async fn when_githubs_api_cant_answer_the_pin_is_not_verified() {
    let app = test_app().await;
    app.web.api_fails(&compare_path("HEAD", SHA));
    let reply = app.post("/entries", with_code(&github_code())).await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
    assert_eq!(
        warnings(&reply.body),
        [format!(
            "the server couldn't confirm the pin of `code: {}`: GitHub's API didn't answer (timed out); the card says \"pin not verified\" until a weekly recheck confirms it",
            github_code()
        )]
    );
    assert!(flags(&reply.body).ends_with("; pin not verified"));
}

#[tokio::test]
async fn once_githubs_calls_are_spent_the_server_stops_asking_until_they_reset() {
    let app = test_app().await;
    app.web.api(
        &compare_path("HEAD", SHA),
        403,
        serde_json::json!({"message": "API rate limit exceeded"}),
        0,
    );
    let reply = app.post("/entries", with_code(&github_code())).await;
    assert!(
        flags(&reply.body).ends_with("; pin not verified"),
        "{}",
        reply.body
    );
    let other = format!("https://github.com/ada/steering/tree/{}", hash('5'));
    let reply = app.post("/entries", with_code(&other)).await;
    assert!(
        warnings(&reply.body)[0].contains("the server's GitHub API calls for the hour are spent"),
        "{}",
        reply.body
    );
    assert_eq!(app.web.api_calls().len(), 1);
}

/// The warnings the link checks add, without the analyzer's own.
fn link_warnings(body: &str) -> Vec<&str> {
    warnings(body)
        .into_iter()
        .filter(|warning| !warning.starts_with("the server checks at submission"))
        .collect()
}

#[tokio::test]
async fn a_zenodo_version_record_is_verified() {
    let app = test_app().await;
    let reply = app
        .post("/entries", with_code("https://zenodo.org/records/123"))
        .await;
    assert_eq!(link_warnings(&reply.body), Vec::<&str>::new());
    assert_eq!(flags(&reply.body), "analyzer v1");
    let mut fetched = app.web.fetched();
    fetched.sort();
    assert_eq!(
        fetched,
        [
            "https://zenodo.org/api/records/123",
            "https://zenodo.org/records/123"
        ]
    );
}

#[tokio::test]
async fn a_zenodo_concept_record_is_refused() {
    let app = test_app().await;
    app.web.redirect(
        "https://zenodo.org/api/records/122",
        302,
        "https://zenodo.org/api/records/123",
    );
    let reply = app
        .post("/entries", with_code("https://zenodo.org/records/122"))
        .await;
    assert_eq!(
        reply.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        reply.body
    );
    assert!(
        reply.body.contains(
            "`code: https://zenodo.org/records/122`: record 122 is Zenodo's concept record, which moves to each new version; link the version record that ran, such as https://zenodo.org/records/123"
        ),
        "{}",
        reply.body
    );
}

#[tokio::test]
async fn a_zenodo_doi_is_checked_on_zenodo_and_doi_org_is_never_fetched() {
    let app = test_app().await;
    app.web.answer("https://zenodo.org/api/records/999", 404);
    let reply = app
        .post("/entries", with_code("https://doi.org/10.5281/zenodo.999"))
        .await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
    assert_eq!(app.web.fetched(), ["https://zenodo.org/api/records/999"]);
    assert_eq!(
        link_warnings(&reply.body),
        [
            "`code: https://doi.org/10.5281/zenodo.999` names a record Zenodo doesn't have (it answered 404), so the card says \"code link dead\"; the server checks it again each week",
            "the server couldn't confirm the pin of `code: https://doi.org/10.5281/zenodo.999`: Zenodo answered 404 for record 999; the card says \"pin not verified\" until a weekly recheck confirms it",
        ]
    );
    assert!(flags(&reply.body).ends_with("; code link dead; pin not verified"));
}

#[tokio::test]
async fn when_zenodo_doesnt_answer_the_pin_is_not_verified() {
    let app = test_app().await;
    app.web.fail("https://zenodo.org/api/records/123");
    let reply = app
        .post("/entries", with_code("https://zenodo.org/records/123"))
        .await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
    assert_eq!(
        link_warnings(&reply.body),
        [
            "the server couldn't confirm the pin of `code: https://zenodo.org/records/123`: Zenodo didn't answer (timed out); the card says \"pin not verified\" until a weekly recheck confirms it"
        ]
    );
    assert!(flags(&reply.body).ends_with("; pin not verified"));
}

#[tokio::test]
async fn a_put_checks_the_links_again_and_replaces_what_they_found() {
    let app = test_app().await;
    let dead = format!("https://huggingface.co/datasets/ada/old/tree/{SHA}");
    app.web.answer(&dead, 404);
    let id = app.submit_held(&with_logs(&dead)).await;
    let live = format!("https://huggingface.co/datasets/ada/new/tree/{SHA}");
    let reply = app
        .put_as(&format!("/e/{id}"), &app.token, with_logs(&live))
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(warnings(&reply.body), Vec::<&str>::new());
    let own = app.get_as_author(&format!("/e/{id}")).await;
    assert_eq!(warnings(&own.body), Vec::<&str>::new());
    assert_eq!(flags(&own.body), "analyzer v1; no code");
}

#[tokio::test]
async fn a_new_version_checks_its_links() {
    let app = test_app().await;
    let id = app.submit(&with_code(&github_code())).await;
    let moved = format!("https://github.com/ada/steering-v2/tree/{SHA}");
    app.web.answer(&moved, 404);
    let reply = app
        .post(&format!("/e/{id}/versions"), with_code(&moved))
        .await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
    assert!(
        flags(&reply.body).contains("; code link dead"),
        "{}",
        reply.body
    );
}

async fn batch(app: &TestApp, entries: &[String]) -> (StatusCode, serde_json::Value) {
    let body = serde_json::json!({ "entries": entries }).to_string();
    let reply = app
        .send(
            axum::http::Request::post("/entries/batch")
                .header("authorization", format!("Bearer {}", app.token))
                .header("content-type", "application/json")
                .header("accept", "application/json")
                .body(axum::body::Body::from(body))
                .unwrap(),
        )
        .await;
    let json = serde_json::from_str(&reply.body).unwrap_or_default();
    (reply.status, json)
}

fn member(slug: &str, front: &str) -> String {
    entry("Member", &format!("slug: {slug}\n{front}"))
}

#[tokio::test]
async fn a_batch_checks_each_members_links_and_fetches_a_shared_link_once() {
    let app = test_app().await;
    let logs = format!("https://huggingface.co/datasets/ada/logs/tree/{SHA}");
    app.web.answer(&logs, 404);
    let front = format!("code: {}\nlogs: [{logs}]\n", github_code());
    let (status, json) = batch(&app, &[member("a", &front), member("b", &front)]).await;
    assert_eq!(status, StatusCode::CREATED, "{json}");
    for member in json["entries"].as_array().unwrap() {
        assert!(
            member["card"]
                .as_str()
                .unwrap()
                .contains("; logs link dead"),
            "{member}"
        );
        assert_eq!(member["warnings"].as_array().unwrap().len(), 1, "{member}");
    }
    let mut fetched = app.web.fetched();
    fetched.sort();
    assert_eq!(fetched, [github_code(), logs]);
    assert_eq!(app.web.api_calls().len(), 1);
}

#[tokio::test]
async fn a_batch_member_with_a_bad_pin_refuses_the_batch() {
    let app = test_app().await;
    app.web.redirect(
        "https://zenodo.org/api/records/122",
        302,
        "https://zenodo.org/api/records/123",
    );
    let (status, json) = batch(
        &app,
        &[
            member("a", ""),
            member("b", "code: https://zenodo.org/records/122\n"),
        ],
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{json}");
    assert!(
        json.to_string()
            .contains("entry 2 (slug b): `code: https://zenodo.org/records/122`: record 122 is Zenodo's concept record"),
        "{json}"
    );
}

#[tokio::test]
async fn a_batch_refused_for_anything_else_fetches_nothing() {
    let app = test_app().await;
    let front = format!("code: {}\n", github_code());
    let (status, _) = batch(&app, &[member("a", &front), member("a", &front)]).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(app.web.fetched(), Vec::<String>::new());
}

fn days_from_now(days: i64) -> time::OffsetDateTime {
    time::OffsetDateTime::now_utc() + time::Duration::days(days)
}

async fn recheck(app: &TestApp, days: i64) -> usize {
    filedrawer_server::link_checks::recheck_due(&app.state, days_from_now(days))
        .await
        .unwrap()
}

#[tokio::test]
async fn the_weekly_recheck_clears_a_link_that_came_back() {
    let app = test_app().await;
    let logs = format!("https://huggingface.co/datasets/ada/logs/tree/{SHA}");
    app.web.answer(&logs, 404);
    let id = app.submit(&with_logs(&logs)).await;
    app.web.answer(&logs, 200);
    assert_eq!(recheck(&app, 6).await, 0);
    assert!(public_card(&app, &id).await.contains("; logs link dead"));
    assert_eq!(recheck(&app, 8).await, 1);
    assert_eq!(public_card(&app, &id).await, "analyzer v1; no code");
    // Live now, so it leaves the recheck.
    assert_eq!(recheck(&app, 16).await, 0);
}

#[tokio::test]
async fn a_link_still_dead_is_checked_again_a_week_after_its_last_check() {
    let app = test_app().await;
    let logs = format!("https://huggingface.co/datasets/ada/logs/tree/{SHA}");
    app.web.answer(&logs, 404);
    app.submit(&with_logs(&logs)).await;
    assert_eq!(recheck(&app, 8).await, 1);
    assert_eq!(recheck(&app, 10).await, 0);
    assert_eq!(recheck(&app, 15).await, 1);
}

#[tokio::test]
async fn a_recheck_that_finds_a_bad_pin_flags_the_card_and_never_rejects() {
    let app = test_app().await;
    app.web.api_fails(&compare_path("HEAD", SHA));
    let id = app.submit(&with_code(&github_code())).await;
    assert!(public_card(&app, &id).await.ends_with("; pin not verified"));
    not_on_default(&app, 0);
    assert_eq!(recheck(&app, 8).await, 1);
    assert_eq!(public_card(&app, &id).await, "analyzer v1; code pin failed");
}

#[tokio::test]
async fn a_recheck_that_finds_a_bad_pin_leaves_an_embargoed_entry_embargoed() {
    let app = test_app().await;
    app.web.api_fails(&compare_path("HEAD", SHA));
    let text = entry(
        "Linked",
        &format!(
            "code: {}\nembargo_until: {}\n",
            github_code(),
            common::date_in(30)
        ),
    );
    let id = app.submit(&text).await;
    not_on_default(&app, 0);
    assert_eq!(recheck(&app, 8).await, 1);
    assert_eq!(
        common::author_status(&app, &app.token, &id)
            .await
            .as_deref(),
        Some("embargoed")
    );
    let own = app.get_as_author(&format!("/e/{id}")).await;
    assert!(
        flags(&own.body).ends_with("; code pin failed"),
        "{}",
        own.body
    );
    assert!(
        warnings(&own.body).contains(&format!(
            "`code: {}`: commit {SHA} isn't on any branch of ada/steering; GitHub shows a fork's commits under its parent's URL too, so link the repository whose branch holds it; the card says \"code pin failed\"",
            github_code()
        )
        .as_str()),
        "{}",
        own.body
    );
}

#[tokio::test]
async fn a_recheck_spends_no_account_share() {
    // 30 calls an hour, of which an account may spend 3; the server's own rechecks
    // spend none of it.
    let app = test_app().await.with_github_hourly(30);
    for digit in ['1', '2', '3', '4'] {
        let url = format!("https://github.com/ada/steering/tree/{}", hash(digit));
        app.post("/entries", with_code(&url)).await;
    }
    assert_eq!(app.web.api_calls().len(), 3);
    assert_eq!(recheck(&app, 8).await, 1);
    assert_eq!(app.web.api_calls().len(), 4);
}

#[tokio::test]
async fn the_recheck_stops_while_githubs_calls_run_low() {
    let app = test_app().await;
    for digit in ['1', '2'] {
        let sha = hash(digit);
        app.web.api_fails(&compare_path("HEAD", &sha));
        let url = format!("https://github.com/ada/steering/tree/{sha}");
        app.submit_held(&with_code(&url)).await;
        // Answers now, with fewer than half of the hour's 5,000 calls left.
        app.web.api(
            &compare_path("HEAD", &sha),
            200,
            serde_json::json!({"status": "identical"}),
            2000,
        );
    }
    assert_eq!(recheck(&app, 8).await, 1);
}

#[tokio::test]
async fn a_rejected_entry_is_never_rechecked() {
    let models = common::fake_models::FakeModels::default();
    let app = common::test_app_with(models.screener(), 1_000_000_000).await;
    let logs = format!("https://huggingface.co/datasets/ada/logs/tree/{SHA}");
    app.web.answer(&logs, 404);
    let text = entry("Steering FAKE-OFFTOPIC", &format!("logs: [{logs}]\n"));
    let id = app.submit_held(&text).await;
    app.post(&format!("/e/{id}/publish"), "").await;
    assert_eq!(
        common::author_status(&app, &app.token, &id)
            .await
            .as_deref(),
        Some("rejected")
    );
    assert_eq!(recheck(&app, 8).await, 0);
}
