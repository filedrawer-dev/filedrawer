// Rebuilds when a migration is added or changed, since `sqlx::migrate!` embeds them
// at compile time and Cargo doesn't otherwise see a new file in the folder.
fn main() {
    println!("cargo:rerun-if-changed=migrations");
}
