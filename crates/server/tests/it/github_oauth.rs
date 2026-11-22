//! Seam: `HttpGithubOAuth`, GitHub sign-in's real client, against a local server
//! standing in for github.com and api.github.com: the code exchange with its PKCE
//! verifier, the user and their addresses, and the token's revocation, which must
//! follow every exchange that got one.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::extract::{Form, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use filedrawer_server::github_login::{
    ClientSecret, Failure, GithubOAuth, HttpGithubOAuth, OAuthApp,
};
use serde_json::{Value, json};

/// What the stand-in saw: each exchange's form, and each revocation's
/// Authorization header and body.
#[derive(Clone, Default)]
struct Seen {
    exchanges: Arc<Mutex<Vec<std::collections::HashMap<String, String>>>>,
    revoked: Arc<Mutex<Vec<(String, Value)>>>,
}

fn bearer(headers: &HeaderMap) -> String {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_string()
}

/// Codes: `good` and `noscope` get tokens of those names; `creds` answers as GitHub
/// does for a wrong secret; any other is refused as GitHub refuses a used code.
async fn serve(seen: Seen) -> String {
    let app = Router::new()
        .route(
            "/login/oauth/access_token",
            post(
                |State(seen): State<Seen>,
                 Form(form): Form<std::collections::HashMap<String, String>>| async move {
                    seen.exchanges.lock().unwrap().push(form.clone());
                    // GitHub answers refusals with 200 and an `error`.
                    let answer = match form["code"].as_str() {
                        "good" => json!({"access_token": "good", "scope": "user:email"}),
                        "noscope" => json!({"access_token": "noscope", "scope": ""}),
                        "limited" => json!({"access_token": "limited", "scope": "user:email"}),
                        "creds" => json!({"error": "incorrect_client_credentials"}),
                        _ => json!({"error": "bad_verification_code"}),
                    };
                    axum::Json(answer)
                },
            ),
        )
        .route(
            "/user",
            get(|headers: HeaderMap| async move {
                assert!(bearer(&headers).starts_with("Bearer "));
                axum::Json(json!({
                    "id": 583231, "login": "octocat", "name": "  ",
                    "created_at": "2011-01-25T18:44:36Z"
                }))
            }),
        )
        .route(
            "/user/emails",
            get(|headers: HeaderMap| async move {
                if bearer(&headers) == "Bearer noscope" {
                    return StatusCode::NOT_FOUND.into_response();
                }
                // GitHub's rate limit answers 403.
                if bearer(&headers) == "Bearer limited" {
                    return StatusCode::FORBIDDEN.into_response();
                }
                axum::Json(json!([
                    {"email": "Old@Example.com", "verified": true, "primary": false},
                    {"email": "unverified@example.com", "verified": false, "primary": false},
                    {"email": "1+octocat@users.noreply.github.com", "verified": true, "primary": false},
                    {"email": "octo@example.com", "verified": true, "primary": true}
                ]))
                .into_response()
            }),
        )
        .route(
            "/applications/{client}/token",
            delete(
                |State(seen): State<Seen>, headers: HeaderMap, axum::Json(body): axum::Json<Value>| async move {
                    let auth = headers
                        .get(header::AUTHORIZATION)
                        .and_then(|value| value.to_str().ok())
                        .unwrap_or("")
                        .to_string();
                    seen.revoked.lock().unwrap().push((auth, body));
                    StatusCode::NO_CONTENT.into_response() as Response
                },
            ),
        )
        .with_state(seen);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{address}")
}

async fn client(seen: &Seen) -> HttpGithubOAuth {
    let base = serve(seen.clone()).await;
    let app = OAuthApp {
        client_id: "Iv1.app".into(),
        client_secret: ClientSecret("s3cret".into()),
    };
    HttpGithubOAuth::with_bases(app, &base, &base, Duration::from_secs(5))
}

#[tokio::test]
async fn a_good_code_gives_the_user_and_their_usable_addresses_and_the_token_is_revoked() {
    let seen = Seen::default();
    let github = client(&seen).await;
    let identity = github
        .identify(
            "good",
            "the-verifier",
            "https://site.test/login/github/callback",
        )
        .await
        .unwrap();
    assert_eq!(identity.github_id, 583231);
    assert_eq!(identity.login, "octocat");
    // A blank name is no name.
    assert_eq!(identity.name, None);
    assert_eq!(identity.created_at.as_deref(), Some("2011-01-25T18:44:36Z"));
    // Verified ones that get mail, the primary first, lower-cased.
    assert_eq!(identity.emails, vec!["octo@example.com", "old@example.com"]);
    let form = seen.exchanges.lock().unwrap()[0].clone();
    assert_eq!(form["client_id"], "Iv1.app");
    assert_eq!(form["client_secret"], "s3cret");
    assert_eq!(form["code_verifier"], "the-verifier");
    assert_eq!(
        form["redirect_uri"],
        "https://site.test/login/github/callback"
    );
    let revoked = seen.revoked.lock().unwrap().clone();
    assert_eq!(revoked.len(), 1);
    // Basic auth with the app's ID and secret: base64 of "Iv1.app:s3cret".
    assert_eq!(revoked[0].0, "Basic SXYxLmFwcDpzM2NyZXQ=");
    assert_eq!(revoked[0].1, json!({"access_token": "good"}));
}

#[tokio::test]
async fn a_grant_without_the_email_scope_is_told_apart_and_its_token_still_revoked() {
    let seen = Seen::default();
    let github = client(&seen).await;
    let failure = github
        .identify("noscope", "v", "https://site.test/cb")
        .await;
    assert_eq!(failure, Err(Failure::NoEmailAccess));
    assert_eq!(seen.revoked.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn a_rate_limited_answer_is_unavailable_not_a_missing_scope() {
    let seen = Seen::default();
    let github = client(&seen).await;
    let failure = github
        .identify("limited", "v", "https://site.test/cb")
        .await;
    assert!(
        matches!(failure, Err(Failure::Unavailable(_))),
        "{failure:?}"
    );
}

#[tokio::test]
async fn a_refused_code_and_wrong_app_settings_are_told_apart() {
    let seen = Seen::default();
    let github = client(&seen).await;
    let used = github.identify("used", "v", "https://site.test/cb").await;
    assert_eq!(
        used,
        Err(Failure::CodeRefused("bad_verification_code".into()))
    );
    let creds = github.identify("creds", "v", "https://site.test/cb").await;
    assert!(matches!(creds, Err(Failure::Unavailable(_))), "{creds:?}");
    assert!(seen.revoked.lock().unwrap().is_empty());
}

#[tokio::test]
async fn github_not_answering_is_unavailable() {
    let app = OAuthApp {
        client_id: "Iv1.app".into(),
        client_secret: ClientSecret("s".into()),
    };
    // A port nothing listens on.
    let github = HttpGithubOAuth::with_bases(
        app,
        "http://127.0.0.1:9",
        "http://127.0.0.1:9",
        Duration::from_secs(2),
    );
    let failure = github.identify("good", "v", "https://site.test/cb").await;
    assert!(
        matches!(failure, Err(Failure::Unavailable(_))),
        "{failure:?}"
    );
}
