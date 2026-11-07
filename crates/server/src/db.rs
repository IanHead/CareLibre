//! Database pool construction and migration runner.

use std::time::Duration;

use anyhow::Context;
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;

use crate::config::Config;

/// Connect to PostgreSQL and, if enabled, apply pending migrations.
pub async fn connect(config: &Config) -> anyhow::Result<PgPool> {
    let pool = PgPoolOptions::new()
        .max_connections(config.database_max_connections)
        .min_connections(1)
        .acquire_timeout(Duration::from_secs(10))
        .connect(&config.database_url)
        .await
        .context("failed to connect to PostgreSQL")?;

    if config.run_migrations {
        sqlx::migrate!("../../migrations")
            .run(&pool)
            .await
            .context("failed to apply database migrations")?;
        tracing::info!("database migrations are up to date");
    }

    Ok(pool)
}
