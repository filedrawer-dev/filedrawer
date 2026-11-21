//! Serving the router over TCP, with bounds of the server's own (uncertainties.md,
//! The server's own bounds): how long a client may take over a request's headers
//! and over its body, and how many connections are open at once. Caddy stands in
//! front in staging and production, but the server holds these whatever the proxy
//! does, so requests a client starts and never finishes can't use up the process's
//! file descriptors.

use std::future::Future;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use axum::Router;
use axum::body::{Body, HttpBody};
use axum::extract::{ConnectInfo, Request, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use futures::StreamExt;
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper_util::rt::{TokioIo, TokioTimer};
use hyper_util::server::graceful::GracefulShutdown;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tower::ServiceExt;

const MIB: u64 = 1024 * 1024;

/// What the server allows a client's connection.
#[derive(Debug, Clone, Copy)]
pub struct Bounds {
    /// How long a connection may take to send a request's headers, from when it
    /// opens or from the reply before; so also how long it may sit idle.
    pub headers: Duration,
    /// How long a request's body may take to arrive, for each MiB its
    /// `Content-Length` starts: at least one, and at most as many as the largest
    /// body any route reads. A body with no declared length counts as one.
    pub body_per_mib: Duration,
    /// How many connections are open at once; more wait to be accepted.
    pub connections: usize,
}

impl Bounds {
    /// The site's. An idle connection outlasts the two minutes Caddy keeps one to
    /// reuse, so Caddy is the one to close it and never sends a request into a
    /// connection the server just closed. A body gets a minute a MiB, which a link
    /// of 140 kbit/s keeps up with. The connections leave half of the 1,024 file
    /// descriptors systemd gives the service for the database, attached files and
    /// outbound calls.
    pub fn site() -> Bounds {
        Bounds {
            headers: Duration::from_secs(150),
            body_per_mib: Duration::from_secs(60),
            connections: 512,
        }
    }
}

/// Serves `router` on `listener` within `bounds` until `shutdown` resolves, then
/// stops accepting and returns once the requests in progress have their replies.
/// Each request carries its connection's peer, which the rate limits key on.
pub async fn serve(
    listener: TcpListener,
    router: Router,
    bounds: Bounds,
    shutdown: impl Future<Output = ()>,
) {
    let deadline = axum::middleware::from_fn_with_state(bounds.body_per_mib, body_deadline);
    let router = router.layer(deadline);
    // HTTP/1, as Caddy speaks to it. hyper times the headers only when it has a
    // timer, and then from the moment the connection opens or goes idle.
    let mut builder = http1::Builder::new();
    builder
        .timer(TokioTimer::new())
        .header_read_timeout(bounds.headers);
    let places = Arc::new(Semaphore::new(bounds.connections));
    let connections = GracefulShutdown::new();
    let mut shutdown = std::pin::pin!(shutdown);
    loop {
        let (stream, peer, place) = tokio::select! {
            () = &mut shutdown => break,
            accepted = accept(&listener, &places) => accepted,
        };
        let router = router.clone();
        let service = service_fn(move |mut request: hyper::Request<Incoming>| {
            request.extensions_mut().insert(ConnectInfo(peer));
            router.clone().oneshot(request.map(Body::new))
        });
        let connection = builder.serve_connection(TokioIo::new(stream), service);
        let connection = connections.watch(connection);
        tokio::spawn(async move {
            let _place = place;
            if let Err(error) = connection.await {
                tracing::debug!(%peer, %error, "connection ended with an error");
            }
        });
    }
    drop(listener);
    connections.shutdown().await;
}

/// The next connection, once there is a place for it: a connection past the bound
/// waits in the listener's queue, unaccepted, and holds no descriptor here.
async fn accept(
    listener: &TcpListener,
    places: &Arc<Semaphore>,
) -> (TcpStream, SocketAddr, OwnedSemaphorePermit) {
    let place = places
        .clone()
        .acquire_owned()
        .await
        .expect("the semaphore is never closed");
    loop {
        match listener.accept().await {
            Ok((stream, peer)) => return (stream, peer, place),
            Err(error) if gone_before_accepted(&error) => {}
            Err(error) => {
                // Out of descriptors, most likely: wait for some to come free
                // instead of spinning.
                tracing::error!(%error, "can't accept a connection");
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }
    }
}

/// Whether `error` is of one connection that went away while it waited, not of the
/// listener.
fn gone_before_accepted(error: &std::io::Error) -> bool {
    use std::io::ErrorKind::{ConnectionAborted, ConnectionRefused, ConnectionReset};
    matches!(
        error.kind(),
        ConnectionRefused | ConnectionAborted | ConnectionReset
    )
}

/// How long the body of a request with `headers` may take to arrive.
fn body_time(per_mib: Duration, headers: &HeaderMap) -> Duration {
    let largest = (crate::MAX_ATTACHMENT_BODY_BYTES as u64).div_ceil(MIB);
    let declared = headers
        .get(header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .map_or(1, |bytes| bytes.div_ceil(MIB).clamp(1, largest));
    per_mib.saturating_mul(u32::try_from(declared).unwrap_or(u32::MAX))
}

/// Middleware: a request's body arrives within its time, counted from its headers,
/// or the reply is a 408 and the connection closes. Bytes that keep trickling in
/// buy no more time. Only a read of the body can run out of it: a handler that has
/// its whole body takes as long as it needs. The 408 is the connection's answer, made
/// here and not by a handler, so it is plain text whatever the client asked for.
async fn body_deadline(State(per_mib): State<Duration>, request: Request, next: Next) -> Response {
    if request.body().is_end_stream() {
        return next.run(request).await;
    }
    let allowed = body_time(per_mib, request.headers());
    let deadline = tokio::time::Instant::now() + allowed;
    let ran_out = Arc::new(AtomicBool::new(false));
    let (parts, body) = request.into_parts();
    let chunks = futures::stream::unfold(
        (Some(body.into_data_stream()), ran_out.clone()),
        move |(chunks, ran_out)| async move {
            let mut chunks = chunks?;
            let chunk = match tokio::time::timeout_at(deadline, chunks.next()).await {
                Ok(None) => return None,
                Ok(Some(Ok(chunk))) => return Some((Ok(chunk), (Some(chunks), ran_out))),
                Ok(Some(Err(error))) => Err(std::io::Error::other(error)),
                Err(_) => {
                    ran_out.store(true, Ordering::Relaxed);
                    Err(std::io::ErrorKind::TimedOut.into())
                }
            };
            Some((chunk, (None, ran_out)))
        },
    );
    let request = Request::from_parts(parts, Body::from_stream(chunks));
    let response = next.run(request).await;
    if !ran_out.load(Ordering::Relaxed) {
        return response;
    }
    let text = format!(
        "request timeout: the request's body took longer than {} s to arrive\n",
        allowed.as_secs()
    );
    (
        StatusCode::REQUEST_TIMEOUT,
        [(header::CONNECTION, "close")],
        text,
    )
        .into_response()
}
