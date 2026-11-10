// Rebuilds when a migration is added or changed, since `sqlx::migrate!` embeds them
// at compile time and Cargo doesn't otherwise see a new file in the folder.
//
// Embeds the commit the binary is built from, which CI sets as FILEDRAWER_COMMIT
// (.github/workflows/ci.yml); a build without it reports `unknown`.
fn main() {
    println!("cargo:rerun-if-changed=migrations");
    println!("cargo:rerun-if-env-changed=FILEDRAWER_COMMIT");
    let commit = std::env::var("FILEDRAWER_COMMIT").unwrap_or_default();
    let commit = if commit.is_empty() {
        "unknown".to_string()
    } else {
        assert!(
            commit.len() == 40
                && commit
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
            "FILEDRAWER_COMMIT must be a full 40-character lowercase commit SHA, not {commit:?}"
        );
        commit
    };
    println!("cargo:rustc-env=FILEDRAWER_BUILD_COMMIT={commit}");
}
