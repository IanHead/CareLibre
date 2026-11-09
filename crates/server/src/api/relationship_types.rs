//! Relationship types API: the catalogue of allowed role pairs.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::{
    auth::extractor::AuthContext,
    error::{AppError, ErrorBody},
    permissions::Capability,
    relationship_types::{self, RelationshipType},
    state::AppState,
};

#[derive(Debug, Serialize, ToSchema)]
pub struct RelationshipTypeList {
    pub items: Vec<RelationshipType>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct RelationshipTypeWrite {
    /// Typed or key form: "Support Coordinator" becomes `support_coordinator`.
    pub from_role: String,
    pub to_role: String,
    /// `care`, `family`, `health`, `services`, `work` or `other`.
    pub category: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct RelationshipTypeStatus {
    /// `false` retires the type; `true` restores it.
    pub is_active: bool,
}

/// `GET /api/v1/relationship-types`
#[utoipa::path(
    get,
    path = "/api/v1/relationship-types",
    tag = "relationships",
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Allowed role pairs, active and retired", body = RelationshipTypeList),
        (status = 401, description = "Missing or invalid access token", body = ErrorBody),
        (status = 403, description = "Missing relationships.read capability", body = ErrorBody)
    )
)]
pub async fn list(
    State(state): State<AppState>,
    auth: AuthContext,
) -> Result<Json<RelationshipTypeList>, AppError> {
    auth.require(Capability::RelationshipsRead)?;
    Ok(Json(RelationshipTypeList {
        items: relationship_types::load(&state.db, auth.organisation_id).await?,
    }))
}

/// `POST /api/v1/relationship-types`
#[utoipa::path(
    post,
    path = "/api/v1/relationship-types",
    tag = "relationships",
    security(("bearer_auth" = [])),
    request_body = RelationshipTypeWrite,
    responses(
        (status = 201, description = "Type added", body = RelationshipType),
        (status = 403, description = "Missing relationship_types.manage capability", body = ErrorBody),
        (status = 409, description = "The pair already exists", body = ErrorBody),
        (status = 422, description = "Validation failed", body = ErrorBody)
    )
)]
pub async fn create(
    State(state): State<AppState>,
    auth: AuthContext,
    Json(payload): Json<RelationshipTypeWrite>,
) -> Result<(StatusCode, Json<RelationshipType>), AppError> {
    auth.require(Capability::RelationshipTypesManage)?;
    let created = relationship_types::create(
        &state.db,
        auth.organisation_id,
        &payload.from_role,
        &payload.to_role,
        payload.category.trim(),
    )
    .await?;
    Ok((StatusCode::CREATED, Json(created)))
}

/// `PATCH /api/v1/relationship-types/{id}`: retire or restore.
#[utoipa::path(
    patch,
    path = "/api/v1/relationship-types/{id}",
    tag = "relationships",
    security(("bearer_auth" = [])),
    params(("id" = Uuid, Path, description = "Relationship type id")),
    request_body = RelationshipTypeStatus,
    responses(
        (status = 204, description = "Updated"),
        (status = 403, description = "Missing relationship_types.manage capability", body = ErrorBody),
        (status = 404, description = "Relationship type not found", body = ErrorBody)
    )
)]
pub async fn update(
    State(state): State<AppState>,
    auth: AuthContext,
    Path(id): Path<Uuid>,
    Json(payload): Json<RelationshipTypeStatus>,
) -> Result<StatusCode, AppError> {
    auth.require(Capability::RelationshipTypesManage)?;
    relationship_types::set_active(&state.db, auth.organisation_id, id, payload.is_active).await?;
    Ok(StatusCode::NO_CONTENT)
}
