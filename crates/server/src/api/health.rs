//! Liveness and readiness probes.

use axum::{extract::State, Json};
use serde::Serialize;
use utoipa::ToSchema;

use crate::{error::AppError, error::ErrorBody, state::AppState};

#[derive(Debug, Serialize, ToSchema)]
pub struct Liveness {
    pub status: String,
    pub version: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct Readiness {
    pub status: String,
    pub database: String,
}

/// Process is running. Does not touch the database.
#[utoipa::path(
    get,
    path = "/healthz",
    tag = "system",
    responses((status = 200, description = "Service is alive", body = Liveness))
)]
pub async fn liveness() -> Json<Liveness> {
    Json(Liveness {
        status: "ok".to_owned(),
        version: env!("CARGO_PKG_VERSION").to_owned(),
    })
}

/// Process is ready to serve traffic: the database must be reachable.
#[utoipa::path(
    get,
    path = "/readyz",
    tag = "system",
    responses(
        (status = 200, description = "Service and dependencies are ready", body = Readiness),
        (status = 503, description = "A dependency is unavailable", body = ErrorBody)
    )
)]
pub async fn readiness(State(state): State<AppState>) -> Result<Json<Readiness>, AppError> {
    match sqlx::query_scalar::<_, i32>("SELECT 1")
        .fetch_one(&state.db)
        .await
    {
        Ok(_) => Ok(Json(Readiness {
            status: "ok".to_owned(),
            database: "up".to_owned(),
        })),
        Err(err) => {
            tracing::warn!(error = %err, "readiness check failed");
            Err(AppError::service_unavailable("database is unavailable"))
        }
    }
}
