//! Seam: `AnthropicClient`, the screen's real HTTP client, against a local server.
//! It sends the server's API key, so it must never follow a redirect: the key goes
//! to the URL the client was given and nowhere a reply points.

use std::sync::{Arc, Mutex};

use axum::Router;
use axum::http::{HeaderMap, StatusCode, Uri, header};
use axum::response::IntoResponse;
use filedrawer_server::screening::{AnthropicClient, Model, ModelClient, ModelError, ModelRequest};

/// Every request a server saw: its path and its `x-api-key` header.
type Seen = Arc<Mutex<Vec<(String, Option<String>)>>>;

/// A local server that records each request and answers it with `answer`. Returns
/// its base URL.
async fn serve<R>(seen: Seen, answer: R) -> String
where
    R: IntoResponse + Clone + Send + Sync + 'static,
{
    let record = move |uri: Uri, headers: HeaderMap| {
        let key = headers
            .get("x-api-key")
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        seen.lock().unwrap().push((uri.path().to_string(), key));
        std::future::ready(answer.clone())
    };
    let router = Router::new().fallback(record);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    base
}

fn request() -> ModelRequest {
    ModelRequest {
        model: Model::Haiku,
        system: "instructions".into(),
        user: "text".into(),
        schema: serde_json::json!({"type": "object"}),
    }
}

#[tokio::test]
async fn it_answers_a_redirect_as_an_error_without_following_it() {
    let elsewhere_saw = Seen::default();
    let elsewhere = serve(elsewhere_saw.clone(), "ok").await;
    let api_saw = Seen::default();
    // 307 keeps the method and the body, so a client that followed it would repeat
    // the whole call there.
    let api = serve(
        api_saw.clone(),
        (
            StatusCode::TEMPORARY_REDIRECT,
            [(header::LOCATION, format!("{elsewhere}/v1/messages"))],
        ),
    )
    .await;
    let client = AnthropicClient::with_url("sk-not-real".into(), &format!("{api}/v1/messages"));

    let reply = client.call(&request()).await;

    assert_eq!(*elsewhere_saw.lock().unwrap(), []);
    assert_eq!(
        *api_saw.lock().unwrap(),
        [("/v1/messages".to_string(), Some("sk-not-real".to_string()))]
    );
    assert!(
        matches!(reply, Err(ModelError::Status { status: 307, .. })),
        "{reply:?}"
    );
}
