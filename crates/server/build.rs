//! Ensure a new or edited migration always triggers a rebuild, because
//! `sqlx::migrate!` embeds the migration directory at compile time.

fn main() {
    println!("cargo:rerun-if-changed=../../migrations");
}
