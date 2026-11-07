//! Seam: the router and `jobs::publish_due`, for the owner removing an attached file
//! (design.md, Attached code; the owner's decisions, 2026-11-06): a removed file is
//! served for no entry; one removed as a hazard is also banned, refused at upload and
//! when cited, and its uploaders go under the review guard.

mod common;

use axum::http::StatusCode;
use common::FILES_HOST;
use common::attachments::{citing, get_on, one_file, set_with, sha256, upload};
use common::fake_models::FakeModels;
use common::{TestApp, browser_account, entry, field, id_in, owner, test_app, test_app_with};
use filedrawer_server::jobs::{publish_due, tidy_files};
use serde_json::json;
use time::{Duration, OffsetDateTime};

const SCRIPT: &str = "import sys\nprint(sys.argv)\n";

/// Runs the screening queue now, as the worker does.
async fn run_worker(app: &TestApp) {
    publish_due(&app.state, OffsetDateTime::now_utc())
        .await
        .unwrap();
}

/// Submits an entry citing `set` with `token`, ends its hold and runs the worker,
/// which screens entries with attachments; returns its ID.
async fn placed(app: &TestApp, token: &str, title: &str, set: &str) -> String {
    let held = app
        .post_as("/entries", Some(token), citing(title, set))
        .await;
    assert_eq!(held.status, StatusCode::CREATED, "{}", held.body);
    let id = id_in(&held.body);
    let publish = app
        .post_as(&format!("/e/{id}/publish"), Some(token), "")
        .await;
    assert_eq!(publish.status, StatusCode::OK, "{}", publish.body);
    run_worker(app).await;
    id
}

async fn status_with(app: &TestApp, token: &str, id: &str) -> String {
    common::author_status(app, token, id)
        .await
        .unwrap_or_default()
}

/// The owner's POST of `action` (`remove` or `remove-hazard`) on file `sha`.
async fn owner_does(app: &TestApp, cookie: &str, sha: &str, action: &str) {
    let reply = app
        .form(&format!("/admin/f/{sha}/{action}"), &[], Some(cookie))
        .await;
    assert_eq!(reply.status, StatusCode::SEE_OTHER, "{}", reply.body);
    assert_eq!(reply.location(), Some(format!("/admin/f/{sha}").as_str()));
}

#[tokio::test]
async fn a_removed_file_is_served_for_no_entry_that_pins_it() {
    let app = test_app().await;
    let other = app.other_token().await;
    let mine = set_with(&app, &app.token, "steer.py", SCRIPT).await;
    let theirs = set_with(&app, &other, "steer.py", SCRIPT).await;
    let id = placed(&app, &app.token, "Steering", &mine).await;
    placed(&app, &other, "Steering too", &theirs).await;
    let sha = sha256(SCRIPT);
    let path = format!("/f/{sha}");
    assert_eq!(get_on(&app, FILES_HOST, &path).await.status, StatusCode::OK);

    let cookie = owner(&app).await;
    owner_does(&app, &cookie, &sha, "remove").await;
    assert_eq!(
        get_on(&app, FILES_HOST, &path).await.status,
        StatusCode::NOT_FOUND
    );
    // The entry stays, and says the file was removed.
    let page = app.get(&format!("/e/{id}")).await;
    assert_eq!(page.status, StatusCode::OK, "{}", page.body);
    assert!(
        page.body.contains("removed_file: src/steer.py"),
        "{}",
        page.body
    );
    assert!(!page.body.contains(&path), "{}", page.body);
    let json = app.get_json(&format!("/e/{id}")).await;
    assert_eq!(json["attachments"]["files"][0]["removed"], true);
    assert_eq!(
        json["attachments"]["files"][0]["url"],
        serde_json::Value::Null
    );

    // The same bytes may be attached again, and still aren't served.
    let again = set_with(&app, &app.token, "steer.py", SCRIPT).await;
    let held = app.post("/entries", citing("Steering again", &again)).await;
    assert_eq!(held.status, StatusCode::CREATED, "{}", held.body);
    let own = app
        .get_as_author(&format!("/e/{}", id_in(&held.body)))
        .await;
    assert!(
        own.body.contains("removed_file: src/steer.py"),
        "{}",
        own.body
    );
}

#[tokio::test]
async fn a_removed_file_outlives_the_sets_that_held_it() {
    let app = test_app().await;
    set_with(&app, &app.token, "steer.py", SCRIPT).await;
    let sha = sha256(SCRIPT);
    let cookie = owner(&app).await;
    owner_does(&app, &cookie, &sha, "remove").await;
    // The set is cited by no entry, so the tidy deletes it after 7 days.
    tidy_files(&app.state, OffsetDateTime::now_utc() + Duration::days(8))
        .await
        .unwrap();
    let page = app.page(&format!("/admin/f/{sha}"), Some(&cookie)).await;
    assert_eq!(page.status, StatusCode::OK, "{}", page.body);
    assert!(page.body.contains("print(sys.argv)"), "{}", page.body);

    let other = app.other_token().await;
    let again = set_with(&app, &other, "steer.py", SCRIPT).await;
    placed(&app, &other, "Steering", &again).await;
    assert_eq!(
        get_on(&app, FILES_HOST, &format!("/f/{sha}")).await.status,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn removing_a_file_as_a_hazard_keeps_the_screens_verdict() {
    let app = test_app().await;
    let set = set_with(&app, &app.token, "steer.py", SCRIPT).await;
    placed(&app, &app.token, "Steering", &set).await;
    let sha = sha256(SCRIPT);
    let cookie = owner(&app).await;
    owner_does(&app, &cookie, &sha, "remove-hazard").await;
    let page = app.page(&format!("/admin/f/{sha}"), Some(&cookie)).await;
    assert!(page.body.contains("screened: pass"), "{}", page.body);
}

#[tokio::test]
async fn a_refusal_naming_a_banned_file_keeps_its_path_on_one_line() {
    let app = test_app().await;
    let set = json!({"files": [{"origin": "author:a\nstatus: public", "text": SCRIPT}]});
    let stored = upload(&app, &app.token, set).await;
    assert_eq!(stored.status, StatusCode::CREATED, "{}", stored.body);
    let id = serde_json::from_str::<serde_json::Value>(&stored.body).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let cookie = owner(&app).await;
    owner_does(&app, &cookie, &sha256(SCRIPT), "remove-hazard").await;
    let cited = app.post("/entries", citing("Steering", &id)).await;
    assert_eq!(
        cited.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        cited.body
    );
    assert!(
        !cited.body.lines().any(|line| line.starts_with("status:")),
        "{}",
        cited.body
    );
}

#[tokio::test]
async fn a_file_removed_as_a_hazard_is_refused_at_upload_and_when_cited() {
    let app = test_app().await;
    let set = set_with(&app, &app.token, "steer.py", SCRIPT).await;
    let pending = id_in(&app.post("/entries", entry("Pending", "")).await.body);
    let public = app.submit(&entry("Public", "")).await;
    let cookie = owner(&app).await;
    owner_does(&app, &cookie, &sha256(SCRIPT), "remove-hazard").await;

    // From any account, under any name.
    let other = app.other_token().await;
    let upload = upload(&app, &other, one_file("renamed.py", SCRIPT)).await;
    assert_eq!(
        upload.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        upload.body
    );
    assert!(upload.body.contains("src/renamed.py"), "{}", upload.body);
    assert!(upload.body.contains("removed"), "{}", upload.body);

    let cited = app.post("/entries", citing("Steering", &set)).await;
    assert_eq!(
        cited.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        cited.body
    );
    assert!(cited.body.contains("src/steer.py"), "{}", cited.body);
    let batch = json!({"entries": [
        entry("A", &format!("slug: a\nattachments: {set}\n")),
    ]});
    let batch = app.post("/entries/batch", batch.to_string()).await;
    assert_eq!(
        batch.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        batch.body
    );
    let put = app
        .put_as(
            &format!("/e/{pending}"),
            &app.token,
            citing("Pending", &set),
        )
        .await;
    assert_eq!(put.status, StatusCode::UNPROCESSABLE_ENTITY, "{}", put.body);
    let version = app
        .post(&format!("/e/{public}/versions"), citing("Public", &set))
        .await;
    assert_eq!(
        version.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        version.body
    );
}

#[tokio::test]
async fn removing_a_file_as_a_hazard_guards_every_account_that_uploaded_it() {
    let app = test_app().await;
    let other = app.other_token().await;
    let bystander = common::series::third_token(&app).await;
    set_with(&app, &app.token, "steer.py", SCRIPT).await;
    set_with(&app, &other, "steer.py", SCRIPT).await;
    set_with(&app, &bystander, "plot.py", "print('plot')\n").await;
    let cookie = owner(&app).await;
    owner_does(&app, &cookie, &sha256(SCRIPT), "remove-hazard").await;

    for (token, wanted) in [
        (&app.token, "review"),
        (&other, "review"),
        (&bystander, "public"),
    ] {
        let id = common::series::submit_with(&app, token, &entry("Steering", "")).await;
        assert_eq!(status_with(&app, token, &id).await, wanted);
    }
}

#[tokio::test]
async fn a_waiting_entry_citing_a_banned_file_is_rejected_when_screened() {
    let app = test_app().await;
    let set = set_with(&app, &app.token, "steer.py", SCRIPT).await;
    let held = app.post("/entries", citing("Steering", &set)).await;
    assert_eq!(held.status, StatusCode::CREATED, "{}", held.body);
    let id = id_in(&held.body);
    let cookie = owner(&app).await;
    owner_does(&app, &cookie, &sha256(SCRIPT), "remove-hazard").await;
    // Cleared, so the entry is screened rather than sent to review unscreened.
    let clear = app
        .form(
            &format!("/admin/accounts/{}/clear-guard", app.account),
            &[],
            Some(&cookie),
        )
        .await;
    assert_eq!(clear.status, StatusCode::SEE_OTHER, "{}", clear.body);

    app.post(&format!("/e/{id}/publish"), "").await;
    run_worker(&app).await;
    let own = app.get_as_author(&format!("/e/{id}")).await;
    assert_eq!(field(&own.body, "status"), Some("rejected"), "{}", own.body);
    assert!(
        own.body.contains("removed by the archive's owner"),
        "{}",
        own.body
    );
}

#[tokio::test]
async fn the_owners_page_for_a_file_shows_it_and_where_it_is_used() {
    let app = test_app().await;
    let set = set_with(&app, &app.token, "steer.py", SCRIPT).await;
    let id = placed(&app, &app.token, "Steering", &set).await;
    let sha = sha256(SCRIPT);
    let path = format!("/admin/f/{sha}");

    let cookie = owner(&app).await;
    let page = app.page(&path, Some(&cookie)).await;
    assert_eq!(page.status, StatusCode::OK, "{}", page.body);
    for wanted in [
        "print(sys.argv)",
        set.as_str(),
        app.account.as_str(),
        id.as_str(),
        "public",
        &format!("{path}/remove\""),
        &format!("{path}/remove-hazard\""),
    ] {
        assert!(page.body.contains(wanted), "{wanted}:\n{}", page.body);
    }
    let missing = app
        .page(&format!("/admin/f/{}", sha256("nothing")), Some(&cookie))
        .await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);

    owner_does(&app, &cookie, &sha, "remove").await;
    let page = app.page(&path, Some(&cookie)).await;
    assert!(page.body.contains("Removed"), "{}", page.body);
    // A file removed plainly can still be removed as a hazard, which bans it.
    owner_does(&app, &cookie, &sha, "remove-hazard").await;
    let upload = upload(&app, &app.token, one_file("steer.py", SCRIPT)).await;
    assert_eq!(
        upload.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        upload.body
    );
}

#[tokio::test]
async fn only_the_admins_session_removes_a_file() {
    let app = test_app().await;
    let set = set_with(&app, &app.token, "steer.py", SCRIPT).await;
    placed(&app, &app.token, "Steering", &set).await;
    let sha = sha256(SCRIPT);
    let (someone, _) = browser_account(&app, "grace@example.com").await;
    let page = app.page(&format!("/admin/f/{sha}"), Some(&someone)).await;
    assert_eq!(page.status, StatusCode::NOT_FOUND);
    for action in ["remove", "remove-hazard"] {
        let path = format!("/admin/f/{sha}/{action}");
        let forged = app.form(&path, &[], Some(&someone)).await;
        assert_eq!(forged.status, StatusCode::NOT_FOUND, "{path}");
        let with_token = app
            .send(
                axum::http::Request::post(&path)
                    .header("authorization", format!("Bearer {}", app.token))
                    .header("origin", common::PUBLIC_URL)
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await;
        assert_eq!(with_token.location(), Some("/login"), "{path}");
    }
    let served = get_on(&app, FILES_HOST, &format!("/f/{sha}")).await;
    assert_eq!(served.status, StatusCode::OK);
}

#[tokio::test]
async fn the_review_page_links_each_attached_file_to_its_page() {
    let models = FakeModels::default();
    let app = test_app_with(models.screener(), 1_000_000_000).await;
    let hazard = "FAKE-HAZARD FAKE-SONNET-HAZARD\n";
    let set = set_with(&app, &app.token, "attack.py", hazard).await;
    let id = placed(&app, &app.token, "Steering", &set).await;
    assert_eq!(status_with(&app, &app.token, &id).await, "review");
    let cookie = owner(&app).await;
    let page = app.page(&format!("/admin/e/{id}"), Some(&cookie)).await;
    let sha = sha256(hazard);
    assert!(
        page.body.contains(&format!("/admin/f/{sha}")),
        "{}",
        page.body
    );

    // Banning the file leaves the entry for the owner, and its page says so.
    owner_does(&app, &cookie, &sha, "remove-hazard").await;
    run_worker(&app).await;
    assert_eq!(status_with(&app, &app.token, &id).await, "review");
    let page = app.page(&format!("/admin/e/{id}"), Some(&cookie)).await;
    assert!(page.body.contains("removed as a hazard"), "{}", page.body);
}
