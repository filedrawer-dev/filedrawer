//! Seam: `serve::serve`, over loopback connections, with bounds short enough to
//! reach (uncertainties.md, The server's own bounds): a client that starts a
//! request and doesn't finish it loses the connection, and only so many
//! connections are open at once.

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use axum::Router;
use axum::extract::ConnectInfo;
use axum::routing::{get, post};
use filedrawer_server::serve::{Bounds, serve};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;

/// Longer than any reply here takes on a busy machine; reaching it fails the test.
const TOO_LONG: Duration = Duration::from_secs(30);
/// A bound a test waits out.
const SHORT: Duration = Duration::from_millis(300);
/// A bound no test reaches.
const NEVER: Duration = Duration::from_secs(3600);

const SLOW_HANDLER: Duration = Duration::from_millis(900);

/// A served router: `/echo` answers a POST with its body, `/slow` only after three
/// times `SHORT`, and `/peer` with the address its connection came from. Stopped
/// when dropped, or by `stop`.
struct Served {
    at: SocketAddr,
    stop: Option<oneshot::Sender<()>>,
    done: tokio::task::JoinHandle<()>,
}

async fn served(bounds: Bounds) -> Served {
    let router = Router::new()
        .route("/echo", post(|body: String| async move { body }))
        .route(
            "/slow",
            post(|body: String| async move {
                tokio::time::sleep(SLOW_HANDLER).await;
                body
            }),
        )
        .route(
            "/peer",
            get(|ConnectInfo(peer): ConnectInfo<SocketAddr>| async move { peer.to_string() }),
        )
        .route("/", get(|| async { "here" }));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let at = listener.local_addr().unwrap();
    let (stop, stopped) = oneshot::channel::<()>();
    let done = tokio::spawn(serve(listener, router, bounds, async {
        let _ = stopped.await;
    }));
    Served {
        at,
        stop: Some(stop),
        done,
    }
}

fn bounds() -> Bounds {
    Bounds {
        headers: NEVER,
        body_per_mib: NEVER,
        connections: 100,
    }
}

impl Served {
    async fn connect(&self) -> TcpStream {
        TcpStream::connect(self.at).await.unwrap()
    }

    /// A connection that has sent `start`.
    async fn sent(&self, start: &str) -> TcpStream {
        let mut connection = self.connect().await;
        connection.write_all(start.as_bytes()).await.unwrap();
        connection
    }
}

/// Everything the server sends until it closes the connection.
async fn until_closed(connection: &mut TcpStream) -> String {
    let mut reply = String::new();
    tokio::time::timeout(TOO_LONG, connection.read_to_string(&mut reply))
        .await
        .expect("the server closes the connection")
        .unwrap();
    reply
}

/// The reply to one request: its head, and a body as long as its Content-Length.
async fn one_reply(connection: &mut TcpStream) -> String {
    let read = async {
        let mut reply = Vec::new();
        loop {
            let text = String::from_utf8_lossy(&reply).into_owned();
            if let Some((head, body)) = text.split_once("\r\n\r\n") {
                let length: usize = head
                    .lines()
                    .find_map(|line| line.strip_prefix("content-length: "))
                    .expect("a reply says how long it is")
                    .parse()
                    .unwrap();
                if body.len() >= length {
                    return text;
                }
            }
            let mut more = [0u8; 1024];
            let count = connection.read(&mut more).await.unwrap();
            assert!(count > 0, "closed mid-reply: {text}");
            reply.extend_from_slice(&more[..count]);
        }
    };
    tokio::time::timeout(TOO_LONG, read)
        .await
        .expect("the server replies")
}

/// Whether the server sends nothing on `connection` for a while.
async fn stays_silent(connection: &mut TcpStream) -> bool {
    let mut byte = [0u8; 1];
    tokio::time::timeout(SHORT, connection.read(&mut byte))
        .await
        .is_err()
}

const POST_5_OF_11: &str = "POST /echo HTTP/1.1\r\nhost: x\r\ncontent-length: 11\r\n\r\nhello";
const GET: &str = "GET / HTTP/1.1\r\nhost: x\r\n\r\n";

/// Whether a new connection is still served.
async fn still_serves(server: &Served) -> bool {
    let mut connection = server.sent(GET).await;
    one_reply(&mut connection).await.ends_with("\r\n\r\nhere")
}

#[tokio::test]
async fn a_request_carries_the_address_its_connection_came_from() {
    let server = served(bounds()).await;
    let mut connection = server.sent("GET /peer HTTP/1.1\r\nhost: x\r\n\r\n").await;
    let from = connection.local_addr().unwrap().to_string();
    let reply = one_reply(&mut connection).await;
    assert!(reply.ends_with(&format!("\r\n\r\n{from}")), "{reply}");
}

#[tokio::test]
async fn a_request_whose_parts_arrive_in_time_is_served_and_the_connection_kept() {
    let server = served(Bounds {
        headers: TOO_LONG,
        body_per_mib: TOO_LONG,
        ..bounds()
    })
    .await;
    let mut connection = server.sent("POST /echo HTTP/1.1\r\nhost: x\r\n").await;
    tokio::time::sleep(Duration::from_millis(20)).await;
    connection
        .write_all(b"content-length: 11\r\n\r\nhello")
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(20)).await;
    connection.write_all(b" world").await.unwrap();
    let reply = one_reply(&mut connection).await;
    assert!(reply.starts_with("HTTP/1.1 200 OK\r\n"), "{reply}");
    assert!(reply.ends_with("\r\n\r\nhello world"), "{reply}");
    connection
        .write_all(b"GET / HTTP/1.1\r\nhost: x\r\n\r\n")
        .await
        .unwrap();
    let second = one_reply(&mut connection).await;
    assert!(second.ends_with("\r\n\r\nhere"), "{second}");
}

#[tokio::test]
async fn a_connection_that_sends_nothing_is_closed_once_the_time_for_headers_is_up() {
    let server = served(Bounds {
        headers: SHORT,
        ..bounds()
    })
    .await;
    let start = Instant::now();
    let mut connection = server.connect().await;
    assert_eq!(until_closed(&mut connection).await, "");
    assert!(start.elapsed() >= SHORT, "{:?}", start.elapsed());
    assert!(still_serves(&server).await);
}

#[tokio::test]
async fn unfinished_headers_are_closed_once_their_time_is_up() {
    let server = served(Bounds {
        headers: SHORT,
        ..bounds()
    })
    .await;
    let start = Instant::now();
    let mut connection = server.sent("GET / HTTP/1.1\r\nhost: x\r\n").await;
    let reply = until_closed(&mut connection).await;
    assert!(!reply.contains("here"), "{reply}");
    assert!(start.elapsed() >= SHORT, "{:?}", start.elapsed());
    assert!(still_serves(&server).await);
}

#[tokio::test]
async fn an_idle_connection_is_closed_once_the_time_for_headers_is_up() {
    let server = served(Bounds {
        headers: SHORT,
        ..bounds()
    })
    .await;
    let mut connection = server.sent(GET).await;
    let reply = one_reply(&mut connection).await;
    assert!(reply.ends_with("\r\n\r\nhere"), "{reply}");
    let replied = Instant::now();
    assert_eq!(until_closed(&mut connection).await, "");
    assert!(replied.elapsed() >= SHORT, "{:?}", replied.elapsed());
}

#[tokio::test]
async fn a_body_that_doesnt_arrive_in_its_time_gets_a_408_and_the_connection_closes() {
    let server = served(Bounds {
        body_per_mib: SHORT,
        ..bounds()
    })
    .await;
    let mut connection = server.sent(POST_5_OF_11).await;
    let reply = until_closed(&mut connection).await;
    assert!(
        reply.starts_with("HTTP/1.1 408 Request Timeout\r\n"),
        "{reply}"
    );
    assert!(reply.contains("connection: close\r\n"), "{reply}");
    assert!(
        reply.ends_with("request timeout: the request's body took longer than 0 s to arrive\n"),
        "{reply}"
    );
}

#[tokio::test]
async fn bytes_that_keep_trickling_in_buy_a_body_no_more_time() {
    let server = served(Bounds {
        body_per_mib: SHORT,
        ..bounds()
    })
    .await;
    let mut connection = server
        .sent("POST /echo HTTP/1.1\r\nhost: x\r\ncontent-length: 1000\r\n\r\n")
        .await;
    let trickle = async {
        // Stops when the server has closed the connection.
        while connection.write_all(b"a").await.is_ok() {
            tokio::time::sleep(SHORT / 6).await;
        }
    };
    tokio::time::timeout(TOO_LONG, trickle)
        .await
        .expect("the server stops reading");
}

#[tokio::test]
async fn a_larger_declared_body_gets_time_for_each_mib_it_starts() {
    let server = served(Bounds {
        body_per_mib: SHORT,
        ..bounds()
    })
    .await;
    // 3 MiB and a byte start a fourth MiB, so four times the bound.
    let head = format!(
        "POST /echo HTTP/1.1\r\nhost: x\r\ncontent-length: {}\r\n\r\n",
        3 * 1024 * 1024 + 1
    );
    let start = Instant::now();
    let mut large = server.sent(&head).await;
    let reply = one_reply(&mut large).await;
    assert!(reply.starts_with("HTTP/1.1 408 "), "{reply}");
    assert!(reply.contains("longer than 1 s"), "{reply}");
    assert!(start.elapsed() >= 4 * SHORT, "{:?}", start.elapsed());
}

#[tokio::test]
async fn a_body_gets_no_more_time_than_the_largest_body_a_route_reads() {
    let server = served(Bounds {
        body_per_mib: SHORT,
        ..bounds()
    })
    .await;
    // A gigabyte declared: time for 4 MiB, the most any route reads.
    let mut connection = server
        .sent("POST /echo HTTP/1.1\r\nhost: x\r\ncontent-length: 1073741824\r\n\r\n")
        .await;
    let reply = one_reply(&mut connection).await;
    assert!(reply.contains("longer than 1 s"), "{reply}");
}

#[tokio::test]
async fn a_handler_with_its_whole_body_takes_as_long_as_it_needs() {
    let server = served(Bounds {
        body_per_mib: SHORT,
        ..bounds()
    })
    .await;
    let mut connection = server
        .sent("POST /slow HTTP/1.1\r\nhost: x\r\ncontent-length: 5\r\n\r\nhello")
        .await;
    let reply = one_reply(&mut connection).await;
    assert!(reply.starts_with("HTTP/1.1 200 OK\r\n"), "{reply}");
    assert!(reply.ends_with("\r\n\r\nhello"), "{reply}");
}

#[tokio::test]
async fn a_connection_past_the_bound_waits_until_another_closes() {
    let server = served(Bounds {
        connections: 2,
        ..bounds()
    })
    .await;
    let first = server.sent("GET / HTTP/1.1\r\n").await;
    let _second = server.sent("GET / HTTP/1.1\r\n").await;
    let mut third = server.sent("GET / HTTP/1.1\r\nhost: x\r\n\r\n").await;
    assert!(stays_silent(&mut third).await);
    drop(first);
    let reply = one_reply(&mut third).await;
    assert!(reply.ends_with("\r\n\r\nhere"), "{reply}");
}

#[tokio::test]
async fn shutdown_closes_idle_connections_and_lets_a_request_in_progress_finish() {
    let mut server = served(bounds()).await;
    let mut idle = server.connect().await;
    let mut busy = server
        .sent("POST /slow HTTP/1.1\r\nhost: x\r\ncontent-length: 5\r\n\r\nhello")
        .await;
    // The request is with its handler before the server is told to stop.
    tokio::time::sleep(SLOW_HANDLER / 3).await;
    server.stop.take().unwrap().send(()).unwrap();
    let reply = until_closed(&mut busy).await;
    assert!(reply.ends_with("\r\n\r\nhello"), "{reply}");
    assert_eq!(until_closed(&mut idle).await, "");
    tokio::time::timeout(TOO_LONG, &mut server.done)
        .await
        .expect("serve returns once its connections are done")
        .unwrap();
    assert!(TcpStream::connect(server.at).await.is_err());
}
