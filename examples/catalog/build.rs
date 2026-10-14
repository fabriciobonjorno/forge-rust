//! Rebuilds the binary when migrations change, so `sqlx::migrate!()` embeds
//! the current `migrations/` directory.

fn main() {
    println!("cargo:rerun-if-changed=migrations");
}
