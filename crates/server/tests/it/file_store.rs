//! Seam: `file_store::FileStore`: a name that isn't a SHA-256 in lower-case hex
//! never becomes a path, so none reaches outside the store (design.md, Attached
//! code).

use std::io;
use std::path::Path;

use filedrawer_server::file_store::FileStore;

const SECRET: &str = "outside the store\n";

/// A store two folders down in `dir`, beside a file `secret` in `dir`: the name
/// `../secret` would reach it, by the folder the store makes of a name's first two
/// characters.
fn store_beside_a_secret(dir: &Path) -> FileStore {
    std::fs::write(dir.join("secret"), SECRET).unwrap();
    FileStore::open(dir.join("site").join("files")).unwrap()
}

/// Names that aren't a SHA-256 in lower-case hex, the path of the secret in `dir`
/// among them.
fn not_sha256(dir: &Path) -> Vec<String> {
    vec![
        "..".to_string(),
        "../secret".to_string(),
        dir.join("secret").to_str().unwrap().to_string(),
        "ab".to_string(),
        "a".repeat(63),
        "a".repeat(65),
        "A".repeat(64),
        "g".repeat(64),
        format!("../{}", "a".repeat(61)),
    ]
}

/// Asserts the store refused `name` as no SHA-256.
fn assert_refused<T: std::fmt::Debug>(result: io::Result<T>, name: &str) {
    let error = result.expect_err(name);
    assert_eq!(error.kind(), io::ErrorKind::InvalidInput, "{name}: {error}");
}

#[tokio::test]
async fn reading_a_name_that_isnt_a_sha256_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let store = store_beside_a_secret(dir.path());

    for name in not_sha256(dir.path()) {
        assert_refused(store.read(&name).await, &name);
    }
}

#[tokio::test]
async fn writing_a_name_that_isnt_a_sha256_is_refused_and_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let store = store_beside_a_secret(dir.path());
    let writer = store.write().await;

    for name in not_sha256(dir.path()) {
        assert_refused(
            writer.put_all(&[(name.as_str(), "replaced\n")]).await,
            &name,
        );
    }
    assert_eq!(
        std::fs::read_to_string(dir.path().join("secret")).unwrap(),
        SECRET
    );
    assert_eq!(writer.stored().await.unwrap(), Vec::<String>::new());
}

#[tokio::test]
async fn removing_a_name_that_isnt_a_sha256_is_refused_and_removes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let store = store_beside_a_secret(dir.path());
    let writer = store.write().await;

    for name in not_sha256(dir.path()) {
        assert_refused(writer.remove(&name).await, &name);
    }
    assert!(dir.path().join("secret").exists());
}

#[tokio::test]
async fn a_sha256_in_lower_case_hex_is_stored_read_and_removed() {
    let dir = tempfile::tempdir().unwrap();
    let store = store_beside_a_secret(dir.path());
    let name = format!("0123456789abcdef{}", "f".repeat(48));
    let writer = store.write().await;

    writer.put_all(&[(name.as_str(), "kept\n")]).await.unwrap();

    assert_eq!(store.read(&name).await.unwrap().as_deref(), Some("kept\n"));
    assert_eq!(writer.stored().await.unwrap(), vec![name.clone()]);
    writer.remove(&name).await.unwrap();
    assert_eq!(store.read(&name).await.unwrap(), None);
}

#[tokio::test]
async fn files_stored_together_are_each_kept_and_nothing_is_left_half_written() {
    let dir = tempfile::tempdir().unwrap();
    let store = store_beside_a_secret(dir.path());
    let first = format!("0123456789abcdef{}", "f".repeat(48));
    // The same first two digits, so the two share a folder.
    let second = format!("0123456789abcdef{}", "e".repeat(48));
    let third = "a".repeat(64);
    let writer = store.write().await;

    writer
        .put_all(&[
            (first.as_str(), "one\n"),
            (second.as_str(), "two\n"),
            (third.as_str(), ""),
        ])
        .await
        .unwrap();
    // Stored again, a file keeps its bytes.
    writer.put_all(&[(first.as_str(), "one\n")]).await.unwrap();

    assert_eq!(store.read(&first).await.unwrap().as_deref(), Some("one\n"));
    assert_eq!(store.read(&second).await.unwrap().as_deref(), Some("two\n"));
    assert_eq!(store.read(&third).await.unwrap().as_deref(), Some(""));
    let half_written: Vec<_> = std::fs::read_dir(dir.path().join("site/files/tmp"))
        .unwrap()
        .collect();
    assert!(half_written.is_empty(), "{half_written:?}");
}
