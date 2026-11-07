//! Seam: the router, for attached code (design.md, Attached code; the owner's
//! decisions, 2026-11-06): an agent uploads a set of files once, entries cite the
//! set, and the files are served from a host of their own.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::{FILES_URL, Reply, TestApp, test_app};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

fn sha256(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub async fn upload(app: &TestApp, token: &str, set: Value) -> Reply {
    app.send(
        Request::post("/attachments")
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .header("accept", "application/json")
            .body(Body::from(set.to_string()))
            .unwrap(),
    )
    .await
}

fn file(origin: &str, text: &str) -> Value {
    json!({"origin": origin, "text": text})
}

const SCRIPT: &str = "import sys\nprint(sys.argv)\n";
const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";

#[tokio::test]
async fn a_set_is_stored_and_its_files_hashes_are_returned() {
    let app = test_app().await;
    let reply = upload(
        &app,
        &app.token,
        json!({
            "repos": {"main": "https://github.com/example/steering"},
            "files": [
                file(&format!("main@{COMMIT}:src/steer.py"), SCRIPT),
                file("author:analysis/plot.py", "print('plot')\n"),
            ],
        }),
    )
    .await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
    let reply: Value = serde_json::from_str(&reply.body).unwrap();
    assert!(reply["id"].as_str().unwrap().starts_with("as-"), "{reply}");
    assert_eq!(reply["files"][0]["path"], "src/steer.py");
    assert_eq!(reply["files"][0]["sha256"], sha256(SCRIPT));
    assert_eq!(reply["files"][1]["path"], "analysis/plot.py");
    assert_eq!(reply["bytes"], SCRIPT.len() + "print('plot')\n".len());
}

#[tokio::test]
async fn a_set_with_bad_origins_or_text_is_refused_with_every_reason() {
    let app = test_app().await;
    let reply = upload(
        &app,
        &app.token,
        json!({
            "repos": {"main": "https://github.com/example/steering", "odd": "ftp://example.com"},
            "files": [
                file("nowhere:src/a.py", "a"),
                file(&format!("unknown@{COMMIT}:src/b.py"), "b"),
                file("main@xyz:src/c.py", "c"),
                file("author:src/d.py", "d\u{0}"),
                file("author:src/e.py", "e"),
                file("author:src/e.py", "e again"),
            ],
        }),
    )
    .await;
    assert_eq!(
        reply.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        reply.body
    );
    for reason in [
        "`nowhere:src/a.py`",
        "`unknown`",
        "`main@xyz:src/c.py`",
        "src/d.py",
        "src/e.py",
        "`odd`",
    ] {
        assert!(reply.body.contains(reason), "{reason}: {}", reply.body);
    }
}

#[tokio::test]
async fn a_repo_given_as_a_url_or_no_remote_is_accepted() {
    let app = test_app().await;
    let reply = upload(
        &app,
        &app.token,
        json!({
            "repos": {"local": "no remote"},
            "files": [
                file(&format!("https://github.com/example/steering@{COMMIT}:a.py"), "a"),
                file("local@0123abc:b.py", "b"),
            ],
        }),
    )
    .await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
}

#[tokio::test]
async fn a_set_over_a_mebibyte_is_refused() {
    let app = test_app().await;
    let half = "x".repeat(524_289);
    let reply = upload(
        &app,
        &app.token,
        json!({"files": [file("author:a.txt", &half), file("author:b.txt", &half)]}),
    )
    .await;
    assert_eq!(
        reply.status,
        StatusCode::PAYLOAD_TOO_LARGE,
        "{}",
        reply.body
    );
    assert!(reply.body.contains("1048576"), "{}", reply.body);
}

#[tokio::test]
async fn an_account_may_upload_ten_megabytes_a_day() {
    let app = test_app().await;
    let mebibyte = "y".repeat(1_048_000);
    for n in 0..9 {
        let reply = upload(
            &app,
            &app.token,
            json!({"files": [file("author:big.txt", &format!("{n}{mebibyte}"))]}),
        )
        .await;
        assert_eq!(reply.status, StatusCode::CREATED, "set {n}: {}", reply.body);
    }
    let reply = upload(
        &app,
        &app.token,
        json!({"files": [file("author:big.txt", &format!("9{mebibyte}"))]}),
    )
    .await;
    assert_eq!(
        reply.status,
        StatusCode::TOO_MANY_REQUESTS,
        "{}",
        reply.body
    );
    let other = app.other_token().await;
    let theirs = upload(&app, &other, json!({"files": [file("author:a.txt", "a")]})).await;
    assert_eq!(theirs.status, StatusCode::CREATED, "{}", theirs.body);
}

#[tokio::test]
async fn past_the_sites_storage_cap_uploads_are_refused_and_the_owner_told_once() {
    let mut app = test_app().await;
    app.state.attachment_store_cap_bytes = 10;
    app.router = filedrawer_server::app(app.state.clone());
    let owner = common::owner(&app).await;
    let first = upload(
        &app,
        &app.token,
        json!({"files": [file("author:a.txt", "0123456789")]}),
    )
    .await;
    assert_eq!(first.status, StatusCode::CREATED, "{}", first.body);
    // The same file again is stored once, so it fits.
    let again = upload(
        &app,
        &app.token,
        json!({"files": [file("author:b.txt", "0123456789")]}),
    )
    .await;
    assert_eq!(again.status, StatusCode::CREATED, "{}", again.body);
    for _ in 0..2 {
        let over = upload(
            &app,
            &app.token,
            json!({"files": [file("author:c.txt", "x")]}),
        )
        .await;
        assert_eq!(
            over.status,
            StatusCode::INSUFFICIENT_STORAGE,
            "{}",
            over.body
        );
    }
    drop(owner);
    let told: Vec<_> = app
        .sent()
        .await
        .into_iter()
        .filter(|email| email.to == "owner@example.com" && email.subject.contains("storage"))
        .collect();
    assert_eq!(told.len(), 1, "{told:?}");
}

#[tokio::test]
async fn uploads_that_would_leave_the_disk_under_its_floor_are_refused_and_the_owner_told_once() {
    let mut app = test_app().await;
    app.state.attachment_disk_floor_bytes = u64::MAX;
    app.router = filedrawer_server::app(app.state.clone());
    let owner = common::owner(&app).await;
    for _ in 0..2 {
        let over = upload(
            &app,
            &app.token,
            json!({"files": [file("author:c.txt", "x")]}),
        )
        .await;
        assert_eq!(
            over.status,
            StatusCode::INSUFFICIENT_STORAGE,
            "{}",
            over.body
        );
    }
    drop(owner);
    let told: Vec<_> = app
        .sent()
        .await
        .into_iter()
        .filter(|email| email.to == "owner@example.com" && email.subject.contains("disk"))
        .collect();
    assert_eq!(told.len(), 1, "{told:?}");
    assert_eq!(common::stored_texts(&app), Vec::<String>::new());
}

#[tokio::test]
async fn without_a_files_host_attachments_are_off() {
    let mut app = test_app().await;
    app.state.files_url = None;
    app.router = filedrawer_server::app(app.state.clone());
    let reply = upload(
        &app,
        &app.token,
        json!({"files": [file("author:a.txt", "a")]}),
    )
    .await;
    assert_eq!(reply.status, StatusCode::NOT_IMPLEMENTED, "{}", reply.body);
    assert!(reply.body.contains("aren't enabled"), "{}", reply.body);
}

#[tokio::test]
async fn uploading_needs_a_token() {
    let app = test_app().await;
    let reply = app
        .send(
            Request::post("/attachments")
                .header("content-type", "application/json")
                .body(Body::from(json!({"files": []}).to_string()))
                .unwrap(),
        )
        .await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
    let _ = FILES_URL;
}

/// Uploads one script as the app's account and returns the set's ID.
async fn a_set(app: &TestApp, token: &str) -> String {
    let reply = upload(
        app,
        token,
        json!({"repos": {"main": "no remote"}, "files": [file(&format!("main@{COMMIT}:src/steer.py"), SCRIPT)]}),
    )
    .await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
    serde_json::from_str::<Value>(&reply.body).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string()
}

/// Where the app keeps the bytes of file `sha256` (design.md, Attached code).
fn on_disk(app: &TestApp, sha256: &str) -> std::path::PathBuf {
    app.files_dir
        .path()
        .join("files")
        .join(&sha256[..2])
        .join(sha256)
}

#[tokio::test]
async fn an_uploaded_files_bytes_are_kept_on_disk_by_hash() {
    let app = test_app().await;
    a_set(&app, &app.token).await;
    let kept = std::fs::read_to_string(on_disk(&app, &sha256(SCRIPT))).unwrap();
    assert_eq!(kept, SCRIPT);
}

#[tokio::test]
async fn a_public_file_whose_bytes_are_missing_answers_500_and_says_nothing_of_why() {
    let app = test_app().await;
    let set = a_set(&app, &app.token).await;
    let held = app.post("/entries", citing(&set)).await;
    publish_and_place(&app, &common::id_in(&held.body)).await;
    std::fs::remove_file(on_disk(&app, &sha256(SCRIPT))).unwrap();
    let reply = get_on(&app, "files.localhost", &format!("/f/{}", sha256(SCRIPT))).await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(reply.body, "internal error\n");
}

mod tidy {
    //! The hourly tidy of attached files (design.md, Attached code).

    use super::*;
    use time::{Duration, OffsetDateTime};

    async fn tidy_at(app: &TestApp, now: OffsetDateTime) {
        filedrawer_server::jobs::tidy_files(&app.state, now)
            .await
            .unwrap();
    }

    async fn set_of(app: &TestApp, files: Value) -> String {
        let reply = upload(app, &app.token, json!({"files": files})).await;
        assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
        serde_json::from_str::<Value>(&reply.body).unwrap()["id"]
            .as_str()
            .unwrap()
            .to_string()
    }

    #[tokio::test]
    async fn a_set_no_entry_cites_goes_seven_days_after_upload_with_the_files_no_other_set_holds() {
        let app = test_app().await;
        let uploaded = OffsetDateTime::now_utc();
        let uncited = set_of(
            &app,
            json!([
                file("author:shared.py", "shared\n"),
                file("author:lone.py", "lone\n")
            ]),
        )
        .await;
        let cited = set_of(
            &app,
            json!([
                file("author:shared.py", "shared\n"),
                file("author:own.py", "own\n")
            ]),
        )
        .await;
        let held = app.post("/entries", citing(&cited)).await;
        assert_eq!(held.status, StatusCode::CREATED, "{}", held.body);

        tidy_at(&app, uploaded + Duration::days(6)).await;
        assert_eq!(
            common::stored_texts(&app),
            vec!["lone\n", "own\n", "shared\n"]
        );

        tidy_at(&app, uploaded + Duration::days(7) + Duration::seconds(2)).await;
        assert_eq!(common::stored_texts(&app), vec!["own\n", "shared\n"]);
        let late = app.post("/entries", citing(&uncited)).await;
        assert_eq!(
            late.status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{}",
            late.body
        );
        assert!(
            late.body.contains("names no attachment set"),
            "{}",
            late.body
        );
        // The cited set is as old, and stays.
        let again = app.post("/entries", citing(&cited)).await;
        assert_eq!(again.status, StatusCode::CREATED, "{}", again.body);
    }

    #[tokio::test]
    async fn an_older_version_keeps_its_set_after_a_newer_one_cites_another() {
        // The owner's decision, 2026-11-06: older versions stay readable, so their
        // code stays too.
        let app = test_app().await;
        let uploaded = OffsetDateTime::now_utc();
        let first = set_of(&app, json!([file("author:v1.py", "print('v1')\n")])).await;
        let held = app.post("/entries", citing(&first)).await;
        let v1 = common::id_in(&held.body);
        publish_and_place(&app, &v1).await;
        let second = set_of(&app, json!([file("author:v2.py", "print('v2')\n")])).await;
        let version = app
            .post(&format!("/e/{v1}/versions"), citing(&second))
            .await;
        assert_eq!(version.status, StatusCode::CREATED, "{}", version.body);
        publish_and_place(&app, &common::id_in(&version.body)).await;

        tidy_at(&app, uploaded + Duration::days(8)).await;
        assert_eq!(
            common::stored_texts(&app),
            vec!["print('v1')\n", "print('v2')\n"]
        );
        let served = get_on(
            &app,
            "files.localhost",
            &format!("/f/{}", sha256("print('v1')\n")),
        )
        .await;
        assert_eq!(served.status, StatusCode::OK, "{}", served.body);
        assert_eq!(served.body, "print('v1')\n");
    }

    #[tokio::test]
    async fn bytes_with_no_record_and_leftover_temporary_files_are_swept() {
        let app = test_app().await;
        a_set(&app, &app.token).await;
        let root = app.files_dir.path().join("files");
        let stray = sha256("stray\n");
        std::fs::create_dir_all(root.join(&stray[..2])).unwrap();
        std::fs::write(root.join(&stray[..2]).join(&stray), "stray\n").unwrap();
        std::fs::write(root.join("tmp").join(sha256("half")), "ha").unwrap();
        assert_eq!(common::stored_texts(&app), vec![SCRIPT, "stray\n"]);

        tidy_at(&app, OffsetDateTime::now_utc()).await;
        assert_eq!(common::stored_texts(&app), vec![SCRIPT]);
        assert_eq!(std::fs::read_dir(root.join("tmp")).unwrap().count(), 0);
    }

    #[tokio::test]
    async fn something_the_sweep_cant_remove_doesnt_stop_it() {
        let app = test_app().await;
        let root = app.files_dir.path().join("files");
        // Neither a file nor a hash's name: nothing a write leaves, and remove_file
        // can't remove it.
        std::fs::create_dir_all(root.join("tmp").join("odd")).unwrap();
        let stray = sha256("stray\n");
        std::fs::create_dir_all(root.join(&stray[..2])).unwrap();
        std::fs::write(root.join(&stray[..2]).join(&stray), "stray\n").unwrap();

        tidy_at(&app, OffsetDateTime::now_utc()).await;
        assert_eq!(common::stored_texts(&app), Vec::<String>::new());
    }
}

/// An entry citing `set`, with no `code` link.
fn citing(set: &str) -> String {
    common::entry(
        "Steering with attached code",
        &format!("attachments: {set}\n"),
    )
}

#[tokio::test]
async fn an_entry_citing_its_set_lists_the_files_and_says_code_attached() {
    let app = test_app().await;
    let set = a_set(&app, &app.token).await;
    let held = app.post("/entries", citing(&set)).await;
    assert!(held.body.contains("code attached"), "{}", held.body);
    let id = common::id_in(&held.body);
    let own = app.get_as_author(&format!("/e/{id}")).await;
    assert!(own.body.contains("code attached"), "{}", own.body);
    publish_and_place(&app, &id).await;
    let page = app.get(&format!("/e/{id}")).await;
    assert_eq!(page.status, StatusCode::OK, "{}", page.body);
    assert!(page.body.contains("code attached"), "{}", page.body);
    assert!(
        page.body.contains(&format!(
            "file: {FILES_URL}/f/{} src/steer.py",
            sha256(SCRIPT)
        )),
        "{}",
        page.body
    );
    let json = app.get_json(&format!("/e/{id}")).await;
    assert_eq!(json["attachments"]["id"], set.as_str());
    assert_eq!(json["attachments"]["files"][0]["path"], "src/steer.py");
    assert_eq!(
        json["attachments"]["files"][0]["origin"],
        format!("main@{COMMIT}:src/steer.py")
    );
}

#[tokio::test]
async fn an_entry_may_cite_only_its_own_accounts_set() {
    let app = test_app().await;
    let other = app.other_token().await;
    let theirs = a_set(&app, &other).await;
    for set in [theirs.as_str(), "as-nothing00"] {
        let reply = app.post("/entries", citing(set)).await;
        assert_eq!(
            reply.status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{}",
            reply.body
        );
        assert!(reply.body.contains(set), "{}", reply.body);
    }
}

#[tokio::test]
async fn batch_members_and_new_versions_check_their_set_too() {
    let app = test_app().await;
    let other = app.other_token().await;
    let theirs = a_set(&app, &other).await;
    let mine = a_set(&app, &app.token).await;
    let reply = app
        .send(
            Request::post("/entries/batch")
                .header("authorization", format!("Bearer {}", app.token))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({"entries": [
                        common::entry("A", &format!("slug: a\nattachments: {mine}\n")),
                        common::entry("B", &format!("slug: b\nattachments: {theirs}\n")),
                    ]})
                    .to_string(),
                ))
                .unwrap(),
        )
        .await;
    assert_eq!(
        reply.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        reply.body
    );
    assert!(reply.body.contains(&theirs), "{}", reply.body);
    let id = app.submit(&common::entry("Plain", "")).await;
    let version = app
        .post(&format!("/e/{id}/versions"), citing(&theirs))
        .await;
    assert_eq!(
        version.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        version.body
    );
}

/// A GET of `path` addressed to `host`.
async fn get_on(app: &TestApp, host: &str, path: &str) -> Reply {
    app.send(
        Request::get(path)
            .header("host", host)
            .body(Body::empty())
            .unwrap(),
    )
    .await
}

#[tokio::test]
async fn the_files_host_serves_a_public_entrys_files_as_plain_text_and_nothing_else() {
    let app = test_app().await;
    let set = a_set(&app, &app.token).await;
    let held = app.post("/entries", citing(&set)).await;
    let id = common::id_in(&held.body);
    let path = format!("/f/{}", sha256(SCRIPT));

    // Not while the entry is held back.
    assert_eq!(
        get_on(&app, "files.localhost", &path).await.status,
        StatusCode::NOT_FOUND
    );
    // The author's own view lists the files already.
    let own = app.get_as_author(&format!("/e/{id}")).await;
    assert!(own.body.contains("src/steer.py"), "{}", own.body);

    publish_and_place(&app, &id).await;
    let served = get_on(&app, "files.localhost", &path).await;
    assert_eq!(served.status, StatusCode::OK, "{}", served.body);
    assert_eq!(served.body, SCRIPT);
    let header = |name: &str| {
        served
            .headers
            .get(name)
            .map(|value| value.to_str().unwrap().to_string())
    };
    assert_eq!(
        header("content-type").as_deref(),
        Some("text/plain; charset=utf-8")
    );
    assert_eq!(header("x-content-type-options").as_deref(), Some("nosniff"));
    assert_eq!(header("set-cookie"), None);

    // However the host is written, it is still the files host.
    for host in ["FILES.LOCALHOST", "files.localhost:443", "files.localhost."] {
        assert_eq!(
            get_on(&app, host, &path).await.status,
            StatusCode::OK,
            "{host}"
        );
        let site = get_on(&app, host, "/skill.md").await;
        assert_eq!(site.status, StatusCode::NOT_FOUND, "{host}");
    }
    // Only files, only there.
    assert_eq!(
        get_on(&app, "localhost", &path).await.status,
        StatusCode::NOT_FOUND
    );
    for other in [
        format!("/e/{id}"),
        "/skill.md".to_string(),
        "/f/nothing".to_string(),
    ] {
        let reply = get_on(&app, "files.localhost", &other).await;
        assert_eq!(reply.status, StatusCode::NOT_FOUND, "{other}");
    }
}

mod screened {
    //! Attached files through the screen, with the fake model client.

    use super::*;
    use common::fake_models::FakeModels;
    use common::{field, id_in, test_app_with};
    use filedrawer_server::screening::Model;

    async fn app_with(models: &FakeModels, daily_nano_usd: i64) -> TestApp {
        test_app_with(models.screener(), daily_nano_usd).await
    }

    async fn set_of(app: &TestApp, files: Value) -> String {
        let reply = upload(app, &app.token, json!({"files": files})).await;
        assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
        serde_json::from_str::<Value>(&reply.body).unwrap()["id"]
            .as_str()
            .unwrap()
            .to_string()
    }

    /// Submits an entry citing `set` and ends its hold; returns its ID and status.
    async fn publish(app: &TestApp, set: &str, slug: &str) -> (String, String) {
        let held = app
            .post(
                "/entries",
                common::entry(
                    "Steering with code",
                    &format!("slug: {slug}\nattachments: {set}\n"),
                ),
            )
            .await;
        assert_eq!(held.status, StatusCode::CREATED, "{}", held.body);
        let id = id_in(&held.body);
        // Publishing early leaves an entry with attachments to the worker.
        let placed = app.post(&format!("/e/{id}/publish"), "").await;
        assert_eq!(
            field(&placed.body, "status"),
            Some("screening"),
            "{}",
            placed.body
        );
        run_worker(app).await;
        let own = app.get_as_author(&format!("/e/{id}")).await;
        (id, field(&own.body, "status").unwrap().to_string())
    }

    async fn run_worker(app: &TestApp) {
        filedrawer_server::jobs::publish_due(&app.state, time::OffsetDateTime::now_utc())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn a_file_the_screen_doubts_passes_and_the_owner_sees_the_doubt() {
        let models = FakeModels::default();
        let app = app_with(&models, 1_000_000_000).await;
        let cookie = common::owner(&app).await;
        let set = set_of(
            &app,
            json!([file("author:odd.py", "# FAKE-HAZARD FAKE-SONNET-UNSURE\n")]),
        )
        .await;
        let (id, status) = publish(&app, &set, "a").await;
        assert_eq!(status, "public");
        let queue = app.page("/admin", Some(&cookie)).await;
        assert!(
            queue.body.contains(&format!("/admin/e/{id}/doubt-seen")),
            "{}",
            queue.body
        );
        assert!(queue.body.contains("odd.py"), "{}", queue.body);
    }

    #[tokio::test]
    async fn once_the_owner_publishes_an_entry_its_flagged_files_dont_send_the_next_one_to_review()
    {
        let models = FakeModels::default();
        let app = app_with(&models, 1_000_000_000).await;
        let cookie = common::owner(&app).await;
        let flagged = file("author:attack.py", "# FAKE-HAZARD FAKE-SONNET-HAZARD\n");
        let first = set_of(&app, json!([flagged.clone()])).await;
        let (id, status) = publish(&app, &first, "a").await;
        assert_eq!(status, "review");
        // The owner reads the file on the review page, and publishes.
        let page = app.page(&format!("/admin/e/{id}"), Some(&cookie)).await;
        assert!(page.body.contains("attack.py"), "{}", page.body);
        assert!(
            page.body.contains("FAKE-HAZARD FAKE-SONNET-HAZARD"),
            "{}",
            page.body
        );
        let decided = app
            .form(&format!("/admin/e/{id}/publish"), &[], Some(&cookie))
            .await;
        assert_eq!(decided.status, StatusCode::SEE_OTHER, "{}", decided.body);
        let asked = models.asked().len();
        let second = set_of(&app, json!([flagged, file("author:b.py", "print(1)\n")])).await;
        let (_, status) = publish(&app, &second, "b").await;
        assert_eq!(status, "public");
        // Only b.py and the entry's text were screened.
        assert_eq!(models.asked().len(), asked + 2);
    }

    #[tokio::test]
    async fn the_review_page_says_when_a_files_bytes_are_missing() {
        let models = FakeModels::default();
        let app = app_with(&models, 1_000_000_000).await;
        let cookie = common::owner(&app).await;
        let text = "# FAKE-HAZARD FAKE-SONNET-HAZARD\n";
        let set = set_of(&app, json!([file("author:attack.py", text)])).await;
        let (id, status) = publish(&app, &set, "a").await;
        assert_eq!(status, "review");
        std::fs::remove_file(on_disk(&app, &sha256(text))).unwrap();
        let page = app.page(&format!("/admin/e/{id}"), Some(&cookie)).await;
        assert_eq!(page.status, StatusCode::OK, "{}", page.body);
        assert!(page.body.contains("attack.py"), "{}", page.body);
        assert!(
            page.body
                .contains("Its bytes are missing from the server's disk."),
            "{}",
            page.body
        );
    }

    #[tokio::test]
    async fn a_guarded_accounts_files_show_on_the_owners_review_page() {
        let models = FakeModels::default();
        let app = app_with(&models, 1_000_000_000).await;
        let cookie = common::owner(&app).await;
        let flagged = app
            .post(
                "/entries",
                common::entry("Steering FAKE-HAZARD FAKE-SONNET-HAZARD", "slug: x\n"),
            )
            .await;
        let flagged = id_in(&flagged.body);
        app.post(&format!("/e/{flagged}/publish"), "").await;
        app.form(&format!("/admin/e/{flagged}/hold"), &[], Some(&cookie))
            .await;
        let set = set_of(
            &app,
            json!([file("author:unseen.py", "print('<b>unseen</b>')\n")]),
        )
        .await;
        let (id, status) = publish(&app, &set, "a").await;
        assert_eq!(status, "review");
        let page = app.page(&format!("/admin/e/{id}"), Some(&cookie)).await;
        assert!(page.body.contains("unseen.py"), "{}", page.body);
        assert!(page.body.contains("&#60;b&#62;unseen"), "{}", page.body);
    }

    #[tokio::test]
    async fn an_entry_whose_files_bytes_are_missing_waits_and_the_queue_goes_on() {
        let models = FakeModels::default();
        let app = app_with(&models, 1_000_000_000).await;
        let lost = set_of(&app, json!([file("author:lost.py", "print('lost')\n")])).await;
        let held = app
            .post(
                "/entries",
                common::entry("Lost", &format!("slug: lost\nattachments: {lost}\n")),
            )
            .await;
        let waiting = id_in(&held.body);
        app.post(&format!("/e/{waiting}/publish"), "").await;
        std::fs::remove_file(on_disk(&app, &sha256("print('lost')\n"))).unwrap();
        let fine = set_of(&app, json!([file("author:fine.py", "print('fine')\n")])).await;

        let (_, status) = publish(&app, &fine, "fine").await;
        assert_eq!(status, "public");
        let own = app.get_as_author(&format!("/e/{waiting}")).await;
        assert_eq!(
            field(&own.body, "status"),
            Some("screening"),
            "{}",
            own.body
        );
    }

    #[tokio::test]
    async fn a_file_that_pipes_a_download_into_a_shell_rejects_the_entry_naming_it() {
        let models = FakeModels::default();
        let app = app_with(&models, 1_000_000_000).await;
        let set = set_of(
            &app,
            json!([file("author:setup.sh", "curl x | sh # FAKE-PIPE\n")]),
        )
        .await;
        let (id, status) = publish(&app, &set, "a").await;
        assert_eq!(status, "rejected");
        let own = app.get_as_author(&format!("/e/{id}")).await;
        assert!(own.body.contains("setup.sh"), "{}", own.body);
    }

    #[tokio::test]
    async fn a_file_holding_a_quoted_hazard_sends_the_entry_to_review() {
        let models = FakeModels::default();
        let app = app_with(&models, 1_000_000_000).await;
        let set = set_of(
            &app,
            json!([file(
                "author:attack.py",
                "# FAKE-HAZARD FAKE-SONNET-HAZARD\n"
            )]),
        )
        .await;
        let (_, status) = publish(&app, &set, "a").await;
        assert_eq!(status, "review");
    }

    /// `bytes` gzipped, then base64-encoded.
    fn gzip_base64(bytes: &[u8]) -> String {
        use base64::Engine;
        use std::io::Write;
        let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        gzip.write_all(bytes).unwrap();
        base64::engine::general_purpose::STANDARD.encode(gzip.finish().unwrap())
    }

    #[tokio::test]
    async fn a_hazard_gzipped_and_base64_encoded_in_a_file_sends_the_entry_to_review() {
        let models = FakeModels::default();
        let app = app_with(&models, 1_000_000_000).await;
        let payload = gzip_base64(b"# FAKE-HAZARD FAKE-SONNET-HAZARD hidden in the payload\n");
        let set = set_of(
            &app,
            json!([file("author:load.py", &format!("PAYLOAD = '{payload}'\n"))]),
        )
        .await;
        let (_, status) = publish(&app, &set, "a").await;
        assert_eq!(status, "review");
        // The decoded text is screened with the file, through the code prompt.
        let screened = models.screened();
        assert!(
            screened
                .iter()
                .any(|text| text.contains(&payload) && text.contains("hidden in the payload")),
            "{screened:?}"
        );
    }

    #[tokio::test]
    async fn a_decompression_bomb_goes_to_review_without_spending_on_the_screen() {
        let models = FakeModels::default();
        let app = app_with(&models, 1_000_000_000).await;
        let cookie = common::owner(&app).await;
        let bomb = gzip_base64(&vec![b'a'; 8 * 1024 * 1024]);
        let set = set_of(
            &app,
            json!([file("author:bomb.py", &format!("BOMB = '{bomb}'\n"))]),
        )
        .await;
        let (id, status) = publish(&app, &set, "a").await;
        assert_eq!(status, "review");
        assert_eq!(models.asked(), vec![]);
        let page = app.page(&format!("/admin/e/{id}"), Some(&cookie)).await;
        assert!(
            page.body.contains(
                "Why it's here: attached file bomb.py: it decodes and decompresses to more than 4 MiB"
            ),
            "{}",
            page.body
        );
    }

    #[tokio::test]
    async fn an_entrys_files_share_one_hidden_text_budget() {
        let models = FakeModels::default();
        let app = app_with(&models, 1_000_000_000).await;
        // Each hides 150 KiB, under the 256 KiB floor alone but not together.
        let hidden = |n: u8| {
            let text = format!("fixture {n}\n").repeat(150 * 1024 / 10);
            format!("DATA = '{}'\n", gzip_base64(text.as_bytes()))
        };
        let set = set_of(
            &app,
            json!([
                file("author:a.py", &hidden(1)),
                file("author:b.py", &hidden(2))
            ]),
        )
        .await;
        assert_eq!(publish(&app, &set, "a").await.1, "review");
    }

    #[tokio::test]
    async fn a_file_passed_already_doesnt_fund_the_next_entrys_budget() {
        let models = FakeModels::default();
        let app = app_with(&models, 1_000_000_000).await;
        let line = "plain = 'padding padding padding padding padding'\n";
        let plain = file("author:plain.py", &line.repeat(600_000 / line.len()));
        let first = set_of(&app, json!([plain.clone()])).await;
        assert_eq!(publish(&app, &first, "a").await.1, "public");
        // 300 KiB hidden in a small file: over the floor, and only the passed file's
        // size would cover it.
        let text = "fixture\n".repeat(300 * 1024 / 8);
        let hidden = file(
            "author:hidden.py",
            &format!("DATA = '{}'\n", gzip_base64(text.as_bytes())),
        );
        let second = set_of(&app, json!([plain, hidden])).await;
        assert_eq!(publish(&app, &second, "b").await.1, "review");
    }

    #[tokio::test]
    async fn a_lock_file_with_hashes_passes_with_nothing_decoded() {
        let models = FakeModels::default();
        let app = app_with(&models, 1_000_000_000).await;
        let lock = "numpy==2.3.1 \\\n    --hash=sha256:9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08 \\\n    --hash=sha256:2c26b46b68ffc68ff99b453c1d30413413422d706483bfa0f98a5e886266e7ae\n";
        let set = set_of(&app, json!([file("author:requirements.txt", lock)])).await;
        assert_eq!(publish(&app, &set, "a").await.1, "public");
        let screened = models.screened();
        let file_screen: Vec<_> = screened
            .iter()
            .filter(|text| text.contains("numpy"))
            .collect();
        assert_eq!(file_screen.len(), 1, "{screened:?}");
        assert!(!file_screen[0].contains("decoded"), "{}", file_screen[0]);
    }

    #[tokio::test]
    async fn code_is_never_judged_for_topic_or_spam() {
        let models = FakeModels::default();
        let app = app_with(&models, 1_000_000_000).await;
        let set = set_of(
            &app,
            json!([file("author:util.py", "# FAKE-OFFTOPIC FAKE-SPAM\n")]),
        )
        .await;
        let (_, status) = publish(&app, &set, "a").await;
        assert_eq!(status, "public");
    }

    #[tokio::test]
    async fn a_file_is_screened_once_however_many_entries_attach_it() {
        let models = FakeModels::default();
        let app = app_with(&models, 1_000_000_000).await;
        let shared = file("author:shared.py", "print('SHARED-FILE-MARKER')\n");
        let first = set_of(&app, json!([shared.clone()])).await;
        let second = set_of(&app, json!([shared, file("author:other.py", "print(2)\n")])).await;
        assert_eq!(publish(&app, &first, "a").await.1, "public");
        assert_eq!(publish(&app, &second, "b").await.1, "public");
        let times = models
            .screened()
            .iter()
            .filter(|text| text.contains("SHARED-FILE-MARKER"))
            .count();
        assert_eq!(times, 1);
    }

    #[tokio::test]
    async fn a_big_file_is_screened_in_chunks_under_256_kib() {
        let models = FakeModels::default();
        let app = app_with(&models, 1_000_000_000).await;
        let line = "x = 'CHUNK-MARKER padding padding padding padding'\n";
        let big = line.repeat(600_000 / line.len());
        let set = set_of(&app, json!([file("author:big.py", &big)])).await;
        assert_eq!(publish(&app, &set, "a").await.1, "public");
        let chunks: Vec<_> = models
            .screened()
            .into_iter()
            .filter(|text| text.contains("CHUNK-MARKER"))
            .collect();
        assert_eq!(chunks.len(), 3);
        assert!(chunks.iter().all(|text| text.len() < 262_144 + 1_000));
    }

    #[tokio::test]
    async fn attachments_spend_at_most_half_an_accounts_share_and_then_wait() {
        let models = FakeModels::default();
        // Each fake call costs 200,000; the account's share is 400,000, half of it for
        // attachments, so one attachment call spends that half.
        let app = app_with(&models, 4_000_000).await;
        let line = "y = 'WAIT-MARKER padding padding padding padding'\n";
        let set = set_of(
            &app,
            json!([file("author:big.py", &line.repeat(300_000 / line.len()))]),
        )
        .await;
        let (id, status) = publish(&app, &set, "a").await;
        assert_eq!(status, "screening");
        assert_eq!(models.asked(), vec![Model::Haiku]);
        let own = app.get_as_author(&format!("/e/{id}")).await;
        assert_eq!(
            field(&own.body, "status"),
            Some("screening"),
            "{}",
            own.body
        );
        // An entry without attachments still screens.
        let plain = app
            .post("/entries", common::entry("Plain", "slug: plain\n"))
            .await;
        let plain = id_in(&plain.body);
        let placed = app.post(&format!("/e/{plain}/publish"), "").await;
        assert_eq!(
            field(&placed.body, "status"),
            Some("public"),
            "{}",
            placed.body
        );
    }

    #[tokio::test]
    async fn the_screen_test_account_spends_past_its_share_but_not_past_the_days_half() {
        let models = FakeModels::default();
        let line = "z = 'SHARE-MARKER padding padding padding padding'\n";
        let three_chunks = line.repeat(600_000 / line.len());
        // A cap of 4,000,000: an account's attachment half-share is 200,000, one call;
        // the day's attachment half is 2,000,000, ten calls.
        let mut app = app_with(&models, 4_000_000).await;
        app.state.screen_test_account = Some(app.account.clone());
        app.router = filedrawer_server::app(app.state.clone());
        let set = set_of(&app, json!([file("author:big.py", &three_chunks)])).await;
        assert_eq!(publish(&app, &set, "a").await.1, "public");
        // A cap of 400,000: the day's attachment half, 200,000, is one call.
        let models = FakeModels::default();
        let mut app = app_with(&models, 400_000).await;
        app.state.screen_test_account = Some(app.account.clone());
        app.router = filedrawer_server::app(app.state.clone());
        let set = set_of(&app, json!([file("author:big.py", &three_chunks)])).await;
        assert_eq!(publish(&app, &set, "a").await.1, "screening");
        assert_eq!(models.asked(), vec![Model::Haiku]);
    }

    #[tokio::test]
    async fn entries_with_attachments_wait_behind_entries_without() {
        let models = FakeModels::default();
        let app = app_with(&models, 1_000_000_000).await;
        let set = set_of(&app, json!([file("author:a.py", "print('a')\n")])).await;
        let with = app
            .post(
                "/entries",
                common::entry(
                    "Steering WITH-SET-MARKER",
                    &format!("slug: w\nattachments: {set}\n"),
                ),
            )
            .await;
        assert_eq!(with.status, StatusCode::CREATED, "{}", with.body);
        let without = app
            .post(
                "/entries",
                common::entry("Steering WITHOUT-SET-MARKER", "slug: n\n"),
            )
            .await;
        assert_eq!(without.status, StatusCode::CREATED, "{}", without.body);
        filedrawer_server::jobs::publish_due(
            &app.state,
            time::OffsetDateTime::now_utc() + time::Duration::hours(2),
        )
        .await
        .unwrap();
        let order: Vec<_> = models
            .screened()
            .iter()
            .filter_map(|text| {
                if text.contains("WITHOUT-SET-MARKER") {
                    Some("without")
                } else if text.contains("WITH-SET-MARKER") || text.contains("print('a')") {
                    Some("with")
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(order.first(), Some(&"without"), "{order:?}");
    }
}

/// Ends an entry's hold early and runs the worker, which screens an entry citing a
/// set: publishing early leaves those to it.
async fn publish_and_place(app: &TestApp, id: &str) {
    app.post(&format!("/e/{id}/publish"), "").await;
    filedrawer_server::jobs::publish_due(&app.state, time::OffsetDateTime::now_utc())
        .await
        .unwrap();
}
