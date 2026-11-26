//! Seam: the router, with the owner's browser session, and `purge::reapply`, for the
//! owner's purge and the removal of a whole chain (design.md, Admin; Entry
//! lifecycle; #15): a purge deletes every version of an entry's chain, or an
//! account's personal data with its GitHub ID, and the files no other entry uses,
//! logs only IDs, and holds after a restore of the database.

use crate::common;

use axum::http::StatusCode;
use common::attachments::{sha256, upload};
use common::scratch::Scratch;
use common::{
    Reply, TestApp, account_id, browser_account, entry, id_in, new_token, owner, stored_texts,
    test_app,
};
use filedrawer_server::{db, purge};
use serde_json::json;

const FIRST: &str = "import os\nprint('only here')\n";
const SHARED: &str = "import sys\nprint('shared')\n";

/// Posts a new version of public entry `old` with `token` and publishes it.
async fn new_version(app: &TestApp, token: &str, old: &str, title: &str) -> String {
    let reply = app
        .post_as(&format!("/e/{old}/versions"), Some(token), entry(title, ""))
        .await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
    let new = id_in(&reply.body);
    let publish = app
        .post_as(&format!("/e/{new}/publish"), Some(token), "")
        .await;
    assert_eq!(publish.status, StatusCode::OK, "{}", publish.body);
    new
}

async fn count(app: &TestApp, sql: &'static str, bind: &str) -> i64 {
    sqlx::query_scalar(sql)
        .bind(bind)
        .fetch_one(&app.pool)
        .await
        .unwrap()
}

async fn entries_with_id(app: &TestApp, id: &str) -> i64 {
    count(app, "SELECT COUNT(*) FROM entries WHERE id = ?", id).await
}

fn log_text(app: &TestApp) -> String {
    std::fs::read_to_string(&app.state.purge_log).unwrap_or_default()
}

async fn purged(reply: Reply) -> Reply {
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    reply
}

#[tokio::test]
async fn purging_an_entry_deletes_every_version_of_its_chain_and_logs_only_ids() {
    let app = test_app().await;
    let cookie = owner(&app).await;
    let first = app.submit(&entry("Steering vectors", "")).await;
    let second = new_version(&app, &app.token, &first, "Steering vectors again").await;
    let kept = app.submit(&entry("Probes", "")).await;

    let confirm = app
        .page(&format!("/admin/e/{second}/purge"), Some(&cookie))
        .await;
    assert_eq!(confirm.status, StatusCode::OK, "{}", confirm.body);
    assert!(confirm.has_class("purge-versions"), "{}", confirm.body);
    for id in [&first, &second] {
        assert!(confirm.body.contains(id.as_str()), "{}", confirm.body);
    }
    assert!(!confirm.body.contains(&kept), "{}", confirm.body);

    let done = purged(
        app.form(&format!("/admin/e/{second}/purge"), &[], Some(&cookie))
            .await,
    )
    .await;
    assert!(done.has_class("purged-entries"), "{}", done.body);

    for id in [&first, &second] {
        assert_eq!(entries_with_id(&app, id).await, 0, "{id}");
        assert_eq!(
            app.get(&format!("/e/{id}")).await.status,
            StatusCode::NOT_FOUND
        );
        // Its author's token reads nothing of it either, not even that it was removed.
        assert_eq!(
            app.get_as_author(&format!("/e/{id}")).await.status,
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            app.page(&format!("/admin/e/{id}"), Some(&cookie))
                .await
                .status,
            StatusCode::NOT_FOUND
        );
    }
    let search = app.get("/search?q=steering").await.body;
    assert!(!search.contains(&first), "{search}");
    let cards = app.get("/cards.txt").await.body;
    assert!(cards.contains(&kept), "{cards}");
    assert!(!cards.contains(&second), "{cards}");
    assert_eq!(app.get("/tags").await.body, "1 refusal\n1 steering\n");

    let log = log_text(&app);
    assert_eq!(log.lines().count(), 1, "{log}");
    assert!(log.contains(&first) && log.contains(&second), "{log}");
    assert!(!log.contains("Steering"), "{log}");
    assert!(!log.contains(&app.account), "{log}");
}

/// Uploads a set holding `files`, `(name, text)` each, with `token`; returns its ID.
async fn set_of(app: &TestApp, token: &str, files: &[(&str, &str)]) -> String {
    let files: Vec<_> = files
        .iter()
        .map(|(name, text)| {
            json!({
                "origin": format!("main@{}:src/{name}", common::attachments::COMMIT),
                "text": text,
            })
        })
        .collect();
    let reply = upload(
        app,
        token,
        json!({"repos": {"main": "no remote"}, "files": files}),
    )
    .await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
    serde_json::from_str::<serde_json::Value>(&reply.body).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string()
}

/// Submits an entry citing `set` with `token`, held as pending; returns its ID.
async fn citing(app: &TestApp, token: &str, title: &str, set: &str) -> String {
    let reply = app
        .post_as(
            "/entries",
            Some(token),
            common::attachments::citing(title, set),
        )
        .await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
    id_in(&reply.body)
}

async fn banned(app: &TestApp, sha: &str) -> bool {
    count(
        app,
        "SELECT COUNT(*) FROM banned_files WHERE sha256 = ?",
        sha,
    )
    .await
        == 1
}

async fn recorded(app: &TestApp, sha: &str) -> bool {
    count(app, "SELECT COUNT(*) FROM files WHERE sha256 = ?", sha).await == 1
}

#[tokio::test]
async fn a_hazards_purge_bans_its_files_and_deletes_those_no_other_entry_uses() {
    let app = test_app().await;
    let cookie = owner(&app).await;
    let other = app.other_token().await;
    let mine = set_of(
        &app,
        &app.token,
        &[("first.py", FIRST), ("shared.py", SHARED)],
    )
    .await;
    let theirs = set_of(&app, &other, &[("shared.py", SHARED)]).await;
    let id = citing(&app, &app.token, "Steering", &mine).await;
    let kept = citing(&app, &other, "Probes", &theirs).await;

    let confirm = app
        .page(&format!("/admin/e/{id}/purge"), Some(&cookie))
        .await;
    assert!(confirm.has_class("purge-files"), "{}", confirm.body);
    assert!(confirm.has_class("file-shared"), "{}", confirm.body);
    // What the hazard box does beyond the chain: the shared file is banned for the
    // other entry too, and both uploaders are guarded.
    let (first, shared) = (sha256(FIRST), sha256(SHARED));
    let effects = confirm
        .body
        .split("class=\"hazard-effects\"")
        .nth(1)
        .unwrap_or_default();
    assert_eq!(
        effects.matches("class=\"hazard-shared\"").count(),
        1,
        "{effects}"
    );
    assert!(effects.contains(&format!("/admin/f/{shared}")), "{effects}");
    assert!(!effects.contains(&format!("/admin/f/{first}")), "{effects}");
    assert_eq!(
        effects.matches("class=\"hazard-guarded\"").count(),
        2,
        "{effects}"
    );
    assert!(effects.contains(&app.account), "{effects}");

    purged(
        app.form(
            &format!("/admin/e/{id}/purge"),
            &[("hazard", "on")],
            Some(&cookie),
        )
        .await,
    )
    .await;

    assert!(banned(&app, &first).await);
    assert!(banned(&app, &shared).await);
    assert!(!recorded(&app, &first).await);
    assert!(recorded(&app, &shared).await);
    let texts = stored_texts(&app);
    assert!(!texts.contains(&FIRST.to_string()), "{texts:?}");
    assert!(texts.contains(&SHARED.to_string()), "{texts:?}");
    assert_eq!(entries_with_id(&app, &kept).await, 1);
    let log = log_text(&app);
    assert!(log.contains(&first) && log.contains(&shared), "{log}");
}

#[tokio::test]
async fn a_plain_purge_bans_nothing_and_deletes_a_file_the_owner_removed() {
    let app = test_app().await;
    let cookie = owner(&app).await;
    let set = set_of(&app, &app.token, &[("first.py", FIRST)]).await;
    let id = citing(&app, &app.token, "Steering", &set).await;
    let first = sha256(FIRST);
    let removed = app
        .form(&format!("/admin/f/{first}/remove"), &[], Some(&cookie))
        .await;
    assert_eq!(removed.status, StatusCode::SEE_OTHER, "{}", removed.body);

    purged(
        app.form(&format!("/admin/e/{id}/purge"), &[], Some(&cookie))
            .await,
    )
    .await;

    assert!(!banned(&app, &first).await);
    assert!(!recorded(&app, &first).await);
    assert!(stored_texts(&app).is_empty());
}

/// Links `account` to GitHub ID `github_id`, as its first link, with a sign-in still
/// pending for the same GitHub account.
async fn link_github(app: &TestApp, account: &str, github_id: i64) {
    let at = common::stamp(time::OffsetDateTime::now_utc());
    for (sql, with_account) in [
        (
            "INSERT INTO github_links (account_id, github_id, login, linked_at, refreshed_at)
             VALUES (?1, ?2, 'grace-gh', ?3, ?3)",
            true,
        ),
        (
            "INSERT INTO github_exemptions (account_id, github_id) VALUES (?1, ?2)",
            true,
        ),
        (
            "INSERT INTO github_signups
                 (key_hash, github_id, login, emails, created_at, expires_at)
             VALUES (x'01', ?2, 'grace-gh', '[\"grace@example.com\"]', ?3, '2999-01-01T00:00:00Z')",
            false,
        ),
    ] {
        let query = sqlx::query(sql);
        let query = if with_account {
            query.bind(account)
        } else {
            query.bind("")
        };
        query
            .bind(github_id)
            .bind(&at)
            .execute(&app.pool)
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn purging_an_account_deletes_its_personal_data_github_id_and_entries() {
    let app = test_app().await;
    let cookie = owner(&app).await;
    let (grace, _) = browser_account(&app, "grace@example.com").await;
    let account = account_id(&app, &grace).await;
    let token = new_token(&app, &grace).await;
    let public = app
        .post_as("/entries", Some(&token), entry("Steering", ""))
        .await;
    let public = id_in(&public.body);
    app.post_as(&format!("/e/{public}/publish"), Some(&token), "")
        .await;
    link_github(&app, &account, 7).await;
    sqlx::query(
        "INSERT INTO email_changes (account_id, new_email, created_at, expires_at)
         VALUES (?, 'grace@new.example', '2026-01-01T00:00:00Z', '2999-01-01T00:00:00Z')",
    )
    .bind(&account)
    .execute(&app.pool)
    .await
    .unwrap();
    let kept = app.submit(&entry("Probes", "")).await;

    let confirm = app
        .page(&format!("/admin/accounts/{account}/purge"), Some(&cookie))
        .await;
    assert_eq!(confirm.status, StatusCode::OK, "{}", confirm.body);
    assert!(confirm.has_class("github-linked"), "{}", confirm.body);
    assert!(
        confirm.body.contains("grace@example.com"),
        "{}",
        confirm.body
    );

    purged(
        app.form(
            &format!("/admin/accounts/{account}/purge"),
            &[("entries", "on"), ("github_id", "")],
            Some(&cookie),
        )
        .await,
    )
    .await;

    let email: Option<String> = sqlx::query_scalar("SELECT email FROM accounts WHERE id = ?")
        .bind(&account)
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(email, None);
    for sql in [
        "SELECT COUNT(*) FROM github_links WHERE account_id = ?",
        "SELECT COUNT(*) FROM email_changes WHERE account_id = ?",
        "SELECT COUNT(*) FROM sessions WHERE account_id = ?",
        "SELECT COUNT(*) FROM tokens WHERE account_id = ?",
    ] {
        assert_eq!(count(&app, sql, &account).await, 0, "{sql}");
    }
    let github: i64 = sqlx::query_scalar(
        "SELECT (SELECT COUNT(*) FROM github_exemptions WHERE github_id = 7)
              + (SELECT COUNT(*) FROM github_signups WHERE github_id = 7)",
    )
    .fetch_one(&app.pool)
    .await
    .unwrap();
    assert_eq!(github, 0);
    assert_eq!(entries_with_id(&app, &public).await, 0);
    assert_eq!(entries_with_id(&app, &kept).await, 1);
    // The old session is gone with the account.
    let login = app.page("/account", Some(&grace)).await;
    assert!(
        login.location().is_some_and(|to| to.starts_with("/login")),
        "{login:?}"
    );
    let log = log_text(&app);
    assert!(log.contains(&account) && log.contains(&public), "{log}");
    assert!(!log.contains("grace"), "{log}");

    // A start applies only lines this database hasn't, so a later sign-in with the
    // same GitHub account survives it.
    sqlx::query(
        "INSERT INTO github_signups (key_hash, github_id, login, emails, created_at, expires_at)
         VALUES (x'02', 7, 'grace-gh', '[]', '2026-01-01T00:00:00Z', '2999-01-01T00:00:00Z')",
    )
    .execute(&app.pool)
    .await
    .unwrap();
    assert_eq!(purge::reapply(&app.state).await.unwrap(), 0);
    let signups: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM github_signups")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(signups, 1);
}

#[tokio::test]
async fn a_deleted_accounts_purge_forgets_only_an_unclaimed_github_id() {
    let app = test_app().await;
    let cookie = owner(&app).await;
    let (grace, _) = browser_account(&app, "grace@example.com").await;
    let account = account_id(&app, &grace).await;
    let public = app.submit(&entry("Kept without the account", "")).await;
    // Deleted with its GitHub ID kept bare, as account deletion leaves it; another
    // account's record names ID 10.
    sqlx::query("UPDATE accounts SET deleted_at = '2026-01-01T00:00:00Z' WHERE id = ?")
        .bind(&account)
        .execute(&app.pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO github_exemptions (github_id, account_id) VALUES (9, NULL), (10, ?)")
        .bind(&app.account)
        .execute(&app.pool)
        .await
        .unwrap();
    let path = format!("/admin/accounts/{account}/purge");

    let confirm = app.page(&path, Some(&cookie)).await;
    assert!(confirm.has_class("account-deleted"), "{}", confirm.body);
    let bad = app
        .form(&path, &[("github_id", "grace-gh")], Some(&cookie))
        .await;
    assert_eq!(bad.status, StatusCode::BAD_REQUEST, "{}", bad.body);

    purged(app.form(&path, &[("github_id", "9")], Some(&cookie)).await).await;
    purged(app.form(&path, &[("github_id", "10")], Some(&cookie)).await).await;

    let left: Vec<i64> = sqlx::query_scalar("SELECT github_id FROM github_exemptions")
        .fetch_all(&app.pool)
        .await
        .unwrap();
    assert_eq!(left, vec![10]);
    // The log ties no GitHub ID to the account, as the privacy page promises.
    let log = log_text(&app);
    assert_eq!(log.lines().count(), 2, "{log}");
    assert!(!log.contains(":9") && !log.contains("github"), "{log}");
    // Without "entries", the account's public entry stays, under "account deleted".
    assert_eq!(entries_with_id(&app, &public).await, 1);
}

#[tokio::test]
async fn a_database_restored_to_before_a_purge_loses_the_data_again() {
    // On a file, as production's is, so `VACUUM INTO` copies it as a backup does.
    let live = Scratch::database("purge-live");
    let pool = db::open(live.to_str().unwrap()).await.unwrap();
    let app = common::test_app_on(pool, filedrawer_server::screening::Screener::PassAll).await;
    let cookie = owner(&app).await;
    let id = app.submit(&entry("Steering vectors", "")).await;
    let kept = app.submit(&entry("Probes", "")).await;
    let backup = Scratch::database("purge-restore");
    sqlx::query("VACUUM INTO ?")
        .bind(backup.to_str().unwrap())
        .execute(&app.pool)
        .await
        .unwrap();

    purged(
        app.form(&format!("/admin/e/{id}/purge"), &[], Some(&cookie))
            .await,
    )
    .await;

    let restored = db::open(backup.to_str().unwrap()).await.unwrap();
    let state = common::state(restored.clone(), &app.outbox, app.files_dir.path());
    assert_eq!(state.purge_log, app.state.purge_log);
    assert_eq!(purge::reapply(&state).await.unwrap(), 1);
    let ids: Vec<String> = sqlx::query_scalar("SELECT id FROM entries")
        .fetch_all(&restored)
        .await
        .unwrap();
    assert_eq!(ids, vec![kept]);
    // Each line is applied once: the next start, and the live database, skip it.
    assert_eq!(purge::reapply(&state).await.unwrap(), 0);
    assert_eq!(purge::reapply(&app.state).await.unwrap(), 0);
}

#[tokio::test]
async fn the_log_cuts_a_torn_last_line_and_refuses_any_other_bad_line() {
    let app = test_app().await;
    let cookie = owner(&app).await;
    let id = app.submit(&entry("Steering vectors", "")).await;
    // A crash mid-write leaves a line without its end, for a purge that never
    // committed.
    std::fs::write(&app.state.purge_log, "{\"at\":\"2026-").unwrap();
    assert!(
        purge::read_log(&app.state.purge_log)
            .unwrap()
            .records
            .is_empty()
    );

    purged(
        app.form(&format!("/admin/e/{id}/purge"), &[], Some(&cookie))
            .await,
    )
    .await;
    let log = purge::read_log(&app.state.purge_log).unwrap();
    assert_eq!(log.records.len(), 1);
    assert_eq!(log.records[0].record.entries, vec![id]);
    assert!(log.bad_lines.is_empty());
}

#[tokio::test]
async fn a_bad_line_is_skipped_mailed_to_the_owner_and_refuses_new_purges() {
    let app = test_app().await;
    let cookie = owner(&app).await;
    let logged = app.submit(&entry("Logged before the damage", "")).await;
    let next = app.submit(&entry("Purged after the fix", "")).await;
    let good = format!("{{\"at\":\"2026-11-26T00:00:00Z\",\"entries\":[\"{logged}\"]}}");
    std::fs::write(&app.state.purge_log, format!("not a record\n{good}\n")).unwrap();

    // The start goes on, applies the good line, and tells the owner which line.
    assert_eq!(purge::reapply(&app.state).await.unwrap(), 1);
    assert_eq!(entries_with_id(&app, &logged).await, 0);
    let notices =
        |mail: Vec<filedrawer_server::mail::Email>| -> Vec<filedrawer_server::mail::Email> {
            mail.into_iter()
                .filter(|email| email.subject.contains("purge log"))
                .collect()
        };
    let mail = notices(app.sent().await);
    assert_eq!(mail.len(), 1, "{mail:?}");
    assert_eq!(mail[0].to, "owner@example.com");
    assert!(mail[0].body.contains("lines 1 of"), "{}", mail[0].body);
    assert!(!mail[0].body.contains("not a record"), "{}", mail[0].body);
    // Once a day.
    purge::reapply(&app.state).await.unwrap();
    assert_eq!(notices(app.sent().await).len(), 1);

    let path = format!("/admin/e/{next}/purge");
    let refused = app.form(&path, &[], Some(&cookie)).await;
    assert_eq!(refused.status, StatusCode::CONFLICT, "{}", refused.body);
    assert!(
        refused.body.contains("lines 1 of the purge log"),
        "{}",
        refused.body
    );
    assert_eq!(entries_with_id(&app, &next).await, 1);

    std::fs::write(&app.state.purge_log, format!("{good}\n")).unwrap();
    purged(app.form(&path, &[], Some(&cookie)).await).await;
    assert_eq!(entries_with_id(&app, &next).await, 0);
}

#[tokio::test]
async fn removing_the_chain_removes_every_version_and_each_restores() {
    let app = test_app().await;
    let cookie = owner(&app).await;
    let first = app.submit(&entry("Steering vectors", "")).await;
    let second = new_version(&app, &app.token, &first, "Steering vectors again").await;
    let path = format!("/admin/e/{first}/remove-chain");

    let page = app.page(&format!("/admin/e/{first}"), Some(&cookie)).await;
    assert!(page.body.contains(&path), "{}", page.body);
    let done = app.form(&path, &[], Some(&cookie)).await;
    assert_eq!(done.status, StatusCode::SEE_OTHER, "{}", done.body);
    for id in [&first, &second] {
        assert_eq!(
            common::author_status(&app, &app.token, id).await.as_deref(),
            Some("removed")
        );
    }
    assert_eq!(
        app.form(&path, &[], Some(&cookie)).await.status,
        StatusCode::CONFLICT
    );
    for id in [&first, &second] {
        let restored = app
            .form(&format!("/admin/e/{id}/restore"), &[], Some(&cookie))
            .await;
        assert_eq!(restored.status, StatusCode::SEE_OTHER, "{}", restored.body);
    }
    assert_eq!(
        app.get(&format!("/e/{second}")).await.status,
        StatusCode::OK
    );
}

#[tokio::test]
async fn only_the_admins_session_purges_or_removes_a_chain() {
    let app = test_app().await;
    let id = app.submit(&entry("Steering vectors", "")).await;
    let (someone, _) = browser_account(&app, "grace@example.com").await;
    let account = app.account.clone();
    for path in [
        format!("/admin/e/{id}/purge"),
        format!("/admin/e/{id}/remove-chain"),
        format!("/admin/accounts/{account}/purge"),
    ] {
        let anonymous = app.form(&path, &[], None).await;
        assert_eq!(anonymous.location(), Some("/login"), "{path}");
        let forged = app.form(&path, &[], Some(&someone)).await;
        assert_eq!(forged.status, StatusCode::NOT_FOUND, "{path}");
        let with_token = app.post(&path, "").await;
        assert!(!with_token.status.is_success(), "{path}");
        assert_ne!(with_token.status, StatusCode::SEE_OTHER, "{path}");
        if path.ends_with("/purge") {
            let page = app.page(&path, Some(&someone)).await;
            assert_eq!(page.status, StatusCode::NOT_FOUND, "{path}");
        }
    }
    assert_eq!(entries_with_id(&app, &id).await, 1);
    assert!(log_text(&app).is_empty());
}

#[tokio::test]
async fn unknown_ids_get_404() {
    let app = test_app().await;
    let cookie = owner(&app).await;
    for path in [
        "/admin/e/e-0000000000000000/purge",
        "/admin/accounts/a-0000000000000000/purge",
    ] {
        assert_eq!(
            app.page(path, Some(&cookie)).await.status,
            StatusCode::NOT_FOUND,
            "{path}"
        );
        assert_eq!(
            app.form(path, &[], Some(&cookie)).await.status,
            StatusCode::NOT_FOUND,
            "{path}"
        );
    }
    assert!(log_text(&app).is_empty());
}
