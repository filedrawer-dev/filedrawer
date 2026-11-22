//! Waiting for a `filedrawer-server serve` a test started to listen, so the test's
//! first request doesn't race the server's start.

use std::net::{TcpListener, TcpStream};
use std::process::Child;
use std::time::{Duration, Instant};

/// How long a test waits for its server to listen: far above any start under load.
pub const START_DEADLINE: Duration = Duration::from_secs(60);

/// A free loopback port: one the system just handed out and nobody holds.
pub fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// Returns once something listens on 127.0.0.1:`port`. Panics if `child` exits
/// first, with its exit status, or if nothing listens within `deadline`.
pub fn wait_until_listening(child: &mut Child, port: u16, deadline: Duration) {
    let end = Instant::now() + deadline;
    while TcpStream::connect(("127.0.0.1", port)).is_err() {
        if let Some(status) = child.try_wait().unwrap() {
            panic!("the server on port {port} exited before it listened: {status}");
        }
        assert!(
            Instant::now() < end,
            "the server never listened on port {port} within {deadline:?}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    /// A privileged port, which no test can bind, so nothing ever listens there.
    const UNBOUND: u16 = 1;

    struct Killed(Child);

    impl Drop for Killed {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    #[test]
    fn returns_once_the_port_listens() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let mut child = Killed(Command::new("sleep").arg("30").spawn().unwrap());
        wait_until_listening(&mut child.0, port, START_DEADLINE);
    }

    #[test]
    #[should_panic(expected = "exited before it listened")]
    fn panics_when_the_server_exits_first() {
        let mut child = Killed(Command::new("false").spawn().unwrap());
        wait_until_listening(&mut child.0, UNBOUND, START_DEADLINE);
    }

    #[test]
    #[should_panic(expected = "never listened")]
    fn panics_when_nothing_listens_in_time() {
        let mut child = Killed(Command::new("sleep").arg("30").spawn().unwrap());
        wait_until_listening(&mut child.0, UNBOUND, Duration::from_millis(300));
    }
}
