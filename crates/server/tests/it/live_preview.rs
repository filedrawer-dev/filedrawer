//! Seam: `bin/live-preview` run against this build's `filedrawer-server`, in a
//! scratch checkout (#71): a new database's seeding fills each section of the review
//! queue through the server's own forms and its scripted screen, and the preview
//! sends a browser's requests signed in as its admin. bin/tests/test_live_preview.py
//! tests the rest of the script against a fake server.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::process::{Child, Command, Stdio};

use crate::common::crate_path;
use crate::common::listening::free_port;

/// The preview, stopped as Ctrl-C would, so it stops its dev server too.
struct Running(Child);

impl Drop for Running {
    fn drop(&mut self) {
        let _ = Command::new("kill").arg(self.0.id().to_string()).status();
        let _ = self.0.wait();
    }
}

/// What a browser gets for `path` from the preview on `port`.
fn page(port: u16, path: &str) -> String {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nSec-Fetch-Dest: document\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    let mut reply = String::new();
    stream.read_to_string(&mut reply).unwrap();
    reply
}

#[test]
fn the_preview_seeds_every_section_of_the_review_queue_and_signs_in_as_its_admin() {
    let checkout = tempfile::tempdir().unwrap();
    let root = checkout.path();
    // The fixture the seed's entries are made from, in git's index as the script
    // finds its fixtures, and this build's server where the script runs one.
    let fixture = root.join("crates/analyzer/tests/it/fixtures/example.md");
    std::fs::create_dir_all(fixture.parent().unwrap()).unwrap();
    std::fs::copy(
        crate_path("../analyzer/tests/it/fixtures/example.md"),
        &fixture,
    )
    .unwrap();
    for args in [&["init", "-q"][..], &["add", "crates"]] {
        let done = Command::new("git")
            .args(args)
            .current_dir(root)
            .status()
            .unwrap();
        assert!(done.success(), "git {args:?}");
    }
    let binary = root.join("target/debug/filedrawer-server");
    std::fs::create_dir_all(binary.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_filedrawer-server"), &binary).unwrap();

    let script = crate_path("../../bin/live-preview");
    let mut child = Command::new(&script)
        .arg("--root")
        .arg(root)
        .args(["--no-build", "--port", "0", "--server-port"])
        .arg(free_port().to_string())
        // The commit checks run this inside a heavy job, which the preview refuses.
        .env_remove("HEAVY_SLOT")
        .env("LIVE_PREVIEW_KEEP_CACHES", "1")
        .stdout(Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| panic!("{}: {e}", script.display()));
    let stdout = child.stdout.take().unwrap();
    let _running = Running(child);
    let mut printed = Vec::new();
    let mut port = None;
    for line in BufReader::new(stdout).lines() {
        let line = line.unwrap();
        if let Some(rest) = line.split_once("http://127.0.0.1:").map(|(_, rest)| rest) {
            port = rest.split('/').next().and_then(|n| n.parse::<u16>().ok());
        }
        printed.push(line);
        if port.is_some() {
            break;
        }
    }
    let port = port.unwrap_or_else(|| panic!("it exited before serving: {printed:?}"));

    let queue = page(port, "/admin");
    assert!(queue.starts_with("HTTP/1.1 200"), "{queue}");
    // In review, held and frozen, each a row linking to its entry.
    assert!(queue.matches("href=\"/admin/e/").count() >= 3, "{queue}");
    for path in ["/doubt-seen", "/admin/series-refusals/", "/clear-guard"] {
        assert!(queue.contains(path), "no {path}: {queue}");
    }
    let account = page(port, "/account");
    assert!(account.starts_with("HTTP/1.1 200"), "{account}");
    assert!(account.contains("/account/tokens/"), "{account}");
}
