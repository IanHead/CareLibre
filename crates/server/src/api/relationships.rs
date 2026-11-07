//! Relationships API: two-sided links between contacts and organisations.
//!
//! Every relationship stores the role of *both* endpoints. A doctor/patient
//! link is one row: `from_role = "general_practitioner"`,
//! `to_role = "patient"`. The graph endpoint returns the same labels on each
//! edge so clients can render the connection from either side.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::{
    auth::extractor::AuthContext,
    error::{AppError, ErrorBody},
    permissions::Capability,
    relationships::{
        ego_graph, filter_edges, load_edge, load_edges, whole_graph, NodeKey, PartyKind,
        RelationshipEdge, RelationshipGraph,
    },
    state::AppState,
};

#[derive(Debug, Deserialize, IntoParams)]
pub struct ListParams {
    /// Only relationships where this contact is either endpoint.
    pub contact_id: Option<Uuid>,
    /// Only relationships where this organisation is either endpoint.
    pub organisation_id: Option<Uuid>,
    /// Maximum number of relationships to return (default 200, max 1000).
    pub limit: Option<usize>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RelationshipList {
    pub items: Vec<RelationshipEdge>,
    pub total: usize,
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct GraphParams {
    /// `contact` or `organisation`. Omit for the whole tenant network.
    pub focus_kind: Option<String>,
    /// Id of the focused contact or organisation.
    pub focus_id: Option<Uuid>,
    /// Hops around the focus (default 2, max 4). Ignored without a focus.
    pub depth: Option<usize>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct RelationshipWrite {
    pub from_kind: PartyKind,
    pub from_id: Uuid,
    pub from_role: String,
    pub to_kind: PartyKind,
    pub to_id: Uuid,
    pub to_role: String,
    pub start_date: Option<NaiveDate>,
    pub end_date: Option<NaiveDate>,
    pub notes: Option<String>,
}

/// `GET /api/v1/relationships`
#[utoipa::path(
    get,
    path = "/api/v1/relationships",
    tag = "relationships",
    security(("bearer_auth" = [])),
    params(ListParams),
    responses(
        (status = 200, description = "Relationships touching the filtered party", body = RelationshipList),
        (status = 401, description = "Missing or invalid access token", body = ErrorBody),
        (status = 403, description = "Missing relationships.read capability", body = ErrorBody),
        (status = 422, description = "Validation failed", body = ErrorBody)
    )
)]
pub async fn list(
    State(state): State<AppState>,
    auth: AuthContext,
    Query(params): Query<ListParams>,
) -> Result<Json<RelationshipList>, AppError> {
    auth.require(Capability::RelationshipsRead)?;

    if params.contact_id.is_none() && params.organisation_id.is_none() {
        return Err(AppError::validation(
            "provide contact_id or organisation_id to list relationships",
        ));
    }

    let edges = load_edges(&state.db, auth.organisation_id).await?;
    let filtered = filter_edges(&edges, params.contact_id, params.organisation_id);
    let total = filtered.len();
    let limit = params.limit.unwrap_or(200).clamp(1, 1000);

    Ok(Json(RelationshipList {
        items: filtered.into_iter().take(limit).collect(),
        total,
    }))
}

/// `GET /api/v1/relationships/graph`
#[utoipa::path(
    get,
    path = "/api/v1/relationships/graph",
    tag = "relationships",
    security(("bearer_auth" = [])),
    params(GraphParams),
    responses(
        (status = 200, description = "Nodes and two-sided relationship edges", body = RelationshipGraph),
        (status = 401, description = "Missing or invalid access token", body = ErrorBody),
        (status = 403, description = "Missing relationships.read capability", body = ErrorBody),
        (status = 422, description = "Validation failed", body = ErrorBody)
    )
)]
pub async fn graph(
    State(state): State<AppState>,
    auth: AuthContext,
    Query(params): Query<GraphParams>,
) -> Result<Json<RelationshipGraph>, AppError> {
    auth.require(Capability::RelationshipsRead)?;

    let edges = load_edges(&state.db, auth.organisation_id).await?;

    let graph = match (params.focus_kind.as_deref(), params.focus_id) {
        (None, None) => whole_graph(&edges),
        (Some(kind), Some(id)) => {
            let kind = PartyKind::parse(kind).ok_or_else(|| {
                AppError::validation("focus_kind must be 'contact' or 'organisation'")
            })?;
            let depth = params.depth.unwrap_or(2).clamp(1, 4);
            ego_graph(&edges, NodeKey { kind, id }, depth)
        }
        (Some(_), None) => {
            return Err(AppError::validation(
                "focus_id is required when focus_kind is provided",
            ));
        }
        (None, Some(_)) => {
            return Err(AppError::validation(
                "focus_kind is required when focus_id is provided",
            ));
        }
    };

    Ok(Json(graph))
}

/// `POST /api/v1/relationships`
#[utoipa::path(
    post,
    path = "/api/v1/relationships",
    tag = "relationships",
    security(("bearer_auth" = [])),
    request_body = RelationshipWrite,
    responses(
        (status = 201, description = "Relationship created", body = RelationshipEdge),
        (status = 401, description = "Missing or invalid access token", body = ErrorBody),
        (status = 403, description = "Missing relationships.write capability", body = ErrorBody),
        (status = 409, description = "Identical relationship already exists", body = ErrorBody),
        (status = 422, description = "Validation failed", body = ErrorBody)
    )
)]
pub async fn create(
    State(state): State<AppState>,
    auth: AuthContext,
    Json(payload): Json<RelationshipWrite>,
) -> Result<(StatusCode, Json<RelationshipEdge>), AppError> {
    auth.require(Capability::RelationshipsWrite)?;
    let (from_role, to_role) = validate_roles(&payload)?;
    validate_endpoints(&payload)?;
    validate_dates(payload.start_date, payload.end_date)?;

    check_endpoints_exist(
        &state.db,
        auth.organisation_id,
        payload.from_kind,
        payload.from_id,
    )
    .await?;
    check_endpoints_exist(
        &state.db,
        auth.organisation_id,
        payload.to_kind,
        payload.to_id,
    )
    .await?;

    let duplicate: bool = sqlx::query_scalar(
        "SELECT EXISTS(
            SELECT 1 FROM relationships
            WHERE organisation_id = $1
              AND from_contact_id IS NOT DISTINCT FROM $2
              AND from_organisation_id IS NOT DISTINCT FROM $3
              AND to_contact_id IS NOT DISTINCT FROM $4
              AND to_organisation_id IS NOT DISTINCT FROM $5
              AND from_role = $6
              AND to_role = $7
        )",
    )
    .bind(auth.organisation_id)
    .bind(contact_endpoint(payload.from_kind, payload.from_id))
    .bind(organisation_endpoint(payload.from_kind, payload.from_id))
    .bind(contact_endpoint(payload.to_kind, payload.to_id))
    .bind(organisation_endpoint(payload.to_kind, payload.to_id))
    .bind(&from_role)
    .bind(&to_role)
    .fetch_one(&state.db)
    .await?;

    if duplicate {
        return Err(AppError::conflict(
            "an identical relationship already exists",
        ));
    }

    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO relationships
            (id, organisation_id, from_contact_id, from_organisation_id,
             to_contact_id, to_organisation_id, from_role, to_role,
             start_date, end_date, notes)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)",
    )
    .bind(id)
    .bind(auth.organisation_id)
    .bind(contact_endpoint(payload.from_kind, payload.from_id))
    .bind(organisation_endpoint(payload.from_kind, payload.from_id))
    .bind(contact_endpoint(payload.to_kind, payload.to_id))
    .bind(organisation_endpoint(payload.to_kind, payload.to_id))
    .bind(&from_role)
    .bind(&to_role)
    .bind(payload.start_date)
    .bind(payload.end_date)
    .bind(
        payload
            .notes
            .as_deref()
            .map(str::trim)
            .filter(|notes| !notes.is_empty()),
    )
    .execute(&state.db)
    .await?;

    let edge = load_edge(&state.db, auth.organisation_id, id)
        .await?
        .ok_or_else(|| AppError::not_found("relationship not found after creation"))?;

    Ok((StatusCode::CREATED, Json(edge)))
}

/// `DELETE /api/v1/relationships/{id}`
#[utoipa::path(
    delete,
    path = "/api/v1/relationships/{id}",
    tag = "relationships",
    security(("bearer_auth" = [])),
    params(("id" = Uuid, Path, description = "Relationship id")),
    responses(
        (status = 204, description = "Relationship deleted"),
        (status = 401, description = "Missing or invalid access token", body = ErrorBody),
        (status = 403, description = "Missing relationships.write capability", body = ErrorBody),
        (status = 404, description = "Relationship not found", body = ErrorBody)
    )
)]
pub async fn delete(
    State(state): State<AppState>,
    auth: AuthContext,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    auth.require(Capability::RelationshipsWrite)?;

    let removed = sqlx::query("DELETE FROM relationships WHERE organisation_id = $1 AND id = $2")
        .bind(auth.organisation_id)
        .bind(id)
        .execute(&state.db)
        .await?
        .rows_affected();

    if removed == 0 {
        return Err(AppError::not_found("relationship not found"));
    }

    Ok(StatusCode::NO_CONTENT)
}

fn validate_roles(payload: &RelationshipWrite) -> Result<(String, String), AppError> {
    let from_role = payload.from_role.trim().to_owned();
    let to_role = payload.to_role.trim().to_owned();

    for (name, value) in [("from_role", &from_role), ("to_role", &to_role)] {
        let length = value.chars().count();
        if length == 0 || length > 64 {
            return Err(AppError::validation(format!(
                "{name} must be between 1 and 64 characters"
            )));
        }
    }

    Ok((from_role, to_role))
}

fn validate_endpoints(payload: &RelationshipWrite) -> Result<(), AppError> {
    if payload.from_kind == payload.to_kind && payload.from_id == payload.to_id {
        return Err(AppError::validation(
            "a relationship cannot point at itself",
        ));
    }
    Ok(())
}

fn validate_dates(start: Option<NaiveDate>, end: Option<NaiveDate>) -> Result<(), AppError> {
    if let (Some(start), Some(end)) = (start, end) {
        if end < start {
            return Err(AppError::validation(
                "end_date must be on or after start_date",
            ));
        }
    }
    Ok(())
}

async fn check_endpoints_exist(
    db: &PgPool,
    organisation_id: Uuid,
    kind: PartyKind,
    id: Uuid,
) -> Result<(), AppError> {
    let exists: bool = match kind {
        PartyKind::Contact => {
            sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM contacts WHERE organisation_id=$1 AND id=$2)",
            )
            .bind(organisation_id)
            .bind(id)
            .fetch_one(db)
            .await?
        }
        PartyKind::Organisation => {
            sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM organisations WHERE tenant_id=$1 AND id=$2)",
            )
            .bind(organisation_id)
            .bind(id)
            .fetch_one(db)
            .await?
        }
    };

    if !exists {
        return Err(AppError::validation(format!(
            "{} {} was not found in this organisation",
            kind.as_str(),
            id
        )));
    }

    Ok(())
}

const fn contact_endpoint(kind: PartyKind, id: Uuid) -> Option<Uuid> {
    match kind {
        PartyKind::Contact => Some(id),
        PartyKind::Organisation => None,
    }
}

const fn organisation_endpoint(kind: PartyKind, id: Uuid) -> Option<Uuid> {
    match kind {
        PartyKind::Contact => None,
        PartyKind::Organisation => Some(id),
    }
}
