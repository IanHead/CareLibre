//! Seed a realistic demo dataset for "Blue Dreams Care".
//!
//! Refuses to run without `--confirm`, and refuses when `APP_ENV=production`.
//! The generated data lives in [`carelibre_server::demo`]; the server admin
//! exposes the same seed through **System → Demo data**.
//!
//! Run with:
//! ```sh
//! DATABASE_URL=postgres://... cargo run -p carelibre-server --bin seed-demo -- --confirm
//! ```

use anyhow::{bail, Context};
use sqlx::postgres::PgPoolOptions;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    if !std::env::args().any(|argument| argument == "--confirm") {
        bail!("refusing to seed without `--confirm`");
    }

    let environment = std::env::var("APP_ENV").unwrap_or_else(|_| "development".to_owned());
    if environment.eq_ignore_ascii_case("production") || environment.eq_ignore_ascii_case("prod") {
        bail!("refusing to seed demo data when APP_ENV=production");
    }

    let url = std::env::var("DATABASE_URL").context("DATABASE_URL is required")?;
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .context("connect to PostgreSQL")?;

    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .context("apply migrations")?;

    let report = carelibre_server::demo::seed(&pool).await?;
    for line in report.summary_lines() {
        println!("{line}");
    }

    Ok(())
}
