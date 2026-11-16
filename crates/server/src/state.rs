//! Shared application state handed to every handler.

use std::sync::Arc;
use std::time::Instant;

use sqlx::PgPool;
use tokio::sync::Mutex;

use crate::config::Config;

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub config: Arc<Config>,
    /// Process start time, used by the status page.
    pub started_at: Instant,
    /// Serialises demo reseeds started from the admin UI, so two requests
    /// cannot rebuild the demo tenant at the same time.
    pub demo_seed_lock: Arc<Mutex<()>>,
}

impl AppState {
    #[must_use]
    pub fn new(db: PgPool, config: Config) -> Self {
        Self {
            db,
            config: Arc::new(config),
            started_at: Instant::now(),
            demo_seed_lock: Arc::new(Mutex::new(())),
        }
    }
}
