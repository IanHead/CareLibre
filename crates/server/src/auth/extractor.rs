//! Axum extractor that authenticates a bearer token and exposes the caller's
//! identity and organisation scope to handlers.

use axum::{
    extract::FromRequestParts,
    http::{header, request::Parts},
};
use serde::Serialize;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::{auth::role::Role, auth::token, error::AppError, state::AppState};

/// The authenticated caller, scoped to exactly one organisation.
#[derive(Debug, Clone, Copy, Serialize, ToSchema)]
pub struct AuthContext {
    pub user_id: Uuid,
    pub organisation_id: Uuid,
    pub role: Role,
}

impl FromRequestParts<AppState> for AuthContext {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let header_value = parts
            .headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .ok_or_else(|| AppError::unauthorized("missing Authorization header"))?;

        let token = header_value
            .strip_prefix("Bearer ")
            .or_else(|| header_value.strip_prefix("bearer "))
            .map(str::trim)
            .filter(|token| !token.is_empty())
            .ok_or_else(|| {
                AppError::unauthorized("Authorization header must be `Bearer <token>`")
            })?;

        let claims = token::verify_access_token(&state.config.jwt_secret, token)
            .map_err(|_| AppError::unauthorized("invalid or expired access token"))?;

        let role = claims
            .role
            .parse::<Role>()
            .map_err(|_| AppError::unauthorized("token contains an unknown role"))?;

        Ok(Self {
            user_id: claims.sub,
            organisation_id: claims.org,
            role,
        })
    }
}
