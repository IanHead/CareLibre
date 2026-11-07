//! Axum extractor that authenticates a bearer token and exposes the caller's
//! identity, organisation scope and effective capabilities to handlers.

use axum::{
    extract::FromRequestParts,
    http::{header, request::Parts},
};
use uuid::Uuid;

use crate::{
    auth::role::Role,
    auth::token,
    error::AppError,
    permissions::{Capability, CapabilitySet},
    state::AppState,
};

/// The authenticated caller, scoped to exactly one organisation.
#[derive(Debug, Clone, Copy)]
pub struct AuthContext {
    pub user_id: Uuid,
    pub organisation_id: Uuid,
    pub role: Role,
    pub is_owner: bool,
    pub capabilities: CapabilitySet,
}

impl AuthContext {
    /// Owners bypass capability checks.
    #[must_use]
    pub fn can(&self, capability: Capability) -> bool {
        self.is_owner || self.capabilities.contains(capability)
    }

    pub fn require(&self, capability: Capability) -> Result<(), AppError> {
        if self.can(capability) {
            Ok(())
        } else {
            Err(AppError::forbidden(
                "your permission sets do not allow this action",
            ))
        }
    }
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

        let is_owner = sqlx::query_scalar::<_, bool>(
            "SELECT is_owner FROM organisation_users
             WHERE organisation_id = $1 AND user_id = $2",
        )
        .bind(claims.org)
        .bind(claims.sub)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| AppError::unauthorized("account is not a member of this organisation"))?;

        let capabilities = if is_owner {
            CapabilitySet::all()
        } else {
            let keys = sqlx::query_scalar::<_, String>(
                "SELECT c.capability
                 FROM organisation_user_permission_sets oup
                 JOIN permission_set_capabilities c
                   ON c.permission_set_id = oup.permission_set_id
                 WHERE oup.organisation_id = $1 AND oup.user_id = $2",
            )
            .bind(claims.org)
            .bind(claims.sub)
            .fetch_all(&state.db)
            .await?;

            let mut set = CapabilitySet::empty();
            for key in keys {
                match Capability::parse(&key) {
                    Some(capability) => set.insert(capability),
                    None => tracing::warn!(key, "unknown capability in database"),
                }
            }
            set
        };

        Ok(Self {
            user_id: claims.sub,
            organisation_id: claims.org,
            role,
            is_owner,
            capabilities,
        })
    }
}
