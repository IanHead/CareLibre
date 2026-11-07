//! Shared application state handed to every handler.

use std::{sync::Arc, time::Instant};

use sqlx::PgPool;

use crate::config::Config;

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub config: Arc<Config>,
    /// Process start time, used by the status page.
    pub started_at: Instant,
}

impl AppState {
    #[must_use]
    pub fn new(db: PgPool, config: Config) -> Self {
        Self {
            db,
            config: Arc::new(config),
            started_at: Instant::now(),
        }
    }
}
