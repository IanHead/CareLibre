//! API error type and the uniform JSON error envelope.

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Serialize;
use utoipa::ToSchema;

/// Envelope returned for every non-2xx API response.
#[derive(Debug, Serialize, ToSchema)]
pub struct ErrorBody {
    pub error: ErrorDetail,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ErrorDetail {
    /// Stable, machine-readable error code (e.g. `validation_failed`).
    pub code: String,
    /// Human-readable message, safe to surface to end users.
    pub message: String,
    /// Optional structured context (e.g. per-field validation errors).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

/// Every failure that can escape a handler.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("{0}")]
    BadRequest(String),

    #[error("{0}")]
    Unauthorized(String),

    #[error("{0}")]
    Forbidden(String),

    #[error("{0}")]
    NotFound(String),

    #[error("{0}")]
    Conflict(String),

    #[error("{message}")]
    Validation {
        message: String,
        details: Option<serde_json::Value>,
    },

    #[error("{0}")]
    ServiceUnavailable(String),

    #[error(transparent)]
    Database(#[from] sqlx::Error),

    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

impl AppError {
    #[must_use]
    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::BadRequest(message.into())
    }

    #[must_use]
    pub fn unauthorized(message: impl Into<String>) -> Self {
        Self::Unauthorized(message.into())
    }

    #[must_use]
    pub fn forbidden(message: impl Into<String>) -> Self {
        Self::Forbidden(message.into())
    }

    #[must_use]
    pub fn not_found(message: impl Into<String>) -> Self {
        Self::NotFound(message.into())
    }

    #[must_use]
    pub fn conflict(message: impl Into<String>) -> Self {
        Self::Conflict(message.into())
    }

    #[must_use]
    pub fn validation(message: impl Into<String>) -> Self {
        Self::Validation {
            message: message.into(),
            details: None,
        }
    }

    #[must_use]
    pub fn validation_with_details(message: impl Into<String>, details: serde_json::Value) -> Self {
        Self::Validation {
            message: message.into(),
            details: Some(details),
        }
    }

    #[must_use]
    pub fn service_unavailable(message: impl Into<String>) -> Self {
        Self::ServiceUnavailable(message.into())
    }

    fn parts(&self) -> (StatusCode, &'static str, String, Option<serde_json::Value>) {
        match self {
            Self::BadRequest(message) => (
                StatusCode::BAD_REQUEST,
                "bad_request",
                message.clone(),
                None,
            ),
            Self::Unauthorized(message) => (
                StatusCode::UNAUTHORIZED,
                "unauthorized",
                message.clone(),
                None,
            ),
            Self::Forbidden(message) => (StatusCode::FORBIDDEN, "forbidden", message.clone(), None),
            Self::NotFound(message) => (StatusCode::NOT_FOUND, "not_found", message.clone(), None),
            Self::Conflict(message) => (StatusCode::CONFLICT, "conflict", message.clone(), None),
            Self::Validation { message, details } => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "validation_failed",
                message.clone(),
                details.clone(),
            ),
            Self::ServiceUnavailable(message) => (
                StatusCode::SERVICE_UNAVAILABLE,
                "service_unavailable",
                message.clone(),
                None,
            ),
            // Never leak database or internal details to clients.
            Self::Database(err) => {
                tracing::error!(error = %err, "database error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal_error",
                    "an internal error occurred".to_owned(),
                    None,
                )
            }
            Self::Internal(err) => {
                tracing::error!(error = ?err, "internal error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal_error",
                    "an internal error occurred".to_owned(),
                    None,
                )
            }
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, code, message, details) = self.parts();
        let body = ErrorBody {
            error: ErrorDetail {
                code: code.to_owned(),
                message,
                details,
            },
        };
        (status, Json(body)).into_response()
    }
}

impl From<jsonwebtoken::errors::Error> for AppError {
    fn from(err: jsonwebtoken::errors::Error) -> Self {
        Self::Internal(anyhow::Error::new(err))
    }
}

impl From<std::str::ParseBoolError> for AppError {
    fn from(err: std::str::ParseBoolError) -> Self {
        Self::Internal(anyhow::Error::new(err))
    }
}
