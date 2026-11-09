//! Relationships API: two-sided links between contacts and organisations.
//!
//! Every relationship stores the role of *both* endpoints. A doctor/patient
//! link is one row: `from_role = "general_practitioner"`,
//! `to_role = "patient"`. The graph endpoint returns the same labels on each
//! edge so clients can render the connection from either side.
//!
//! Relationships change over time, and every change is kept:
//!
//! * **end** sets an end date (today, past or future) with a reason;
//! * **transfer** ends the relationship and starts the same one with a
//!   different party on one side, e.g. a new support coordinator;
//! * **change role** ends it and starts one with new roles, e.g. family member
//!   becomes guardian;
//! * **correct** fixes a mistake in place, recorded as a correction;
//! * **remove** hides one recorded in error (`relationships.remove`).
//!
//! Each action appends to `relationship_events` and writes a summary line to
//! the audit log. Adding a relationship is recorded by a database trigger.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use chrono::{DateTime, Duration, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::{FromRow, PgPool, Postgres, Transaction};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::{
    auth::extractor::AuthContext,
    error::{AppError, ErrorBody},
    permissions::Capability,
    relationship_types,
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

/// Why a relationship ends. Required whenever an end date is set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum EndReason {
    Offboarded,
    MovedAway,
    PlanEnded,
    Deceased,
    ChangedProvider,
    CircumstancesChanged,
    Incident,
    Other,
}

impl EndReason {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Offboarded => "offboarded",
            Self::MovedAway => "moved_away",
            Self::PlanEnded => "plan_ended",
            Self::Deceased => "deceased",
            Self::ChangedProvider => "changed_provider",
            Self::CircumstancesChanged => "circumstances_changed",
            Self::Incident => "incident",
            Self::Other => "other",
        }
    }
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct RelationshipWrite {
    pub from_kind: PartyKind,
    pub from_id: Uuid,
    pub from_role: String,
    pub to_kind: PartyKind,
    pub to_id: Uuid,
    pub to_role: String,
    pub start_date: Option<NaiveDate>,
    pub end_date: Option<NaiveDate>,
    /// Required when `end_date` is set.
    pub end_reason: Option<EndReason>,
    pub notes: Option<String>,
}

/// `POST /relationships/{id}/end`
#[derive(Debug, Deserialize, ToSchema)]
pub struct EndRequest {
    /// Last day of the relationship. May be in the future.
    pub end_date: NaiveDate,
    pub reason: EndReason,
    pub note: Option<String>,
}

/// Which endpoint a transfer replaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    From,
    To,
}

/// `POST /relationships/{id}/transfer`
#[derive(Debug, Deserialize, ToSchema)]
pub struct TransferRequest {
    /// First day with the new party. The current relationship ends the day before.
    pub effective_date: NaiveDate,
    /// The endpoint being replaced.
    pub replace: Side,
    pub new_kind: PartyKind,
    pub new_id: Uuid,
    pub reason: EndReason,
    pub note: Option<String>,
}

/// `POST /relationships/{id}/change-role`
#[derive(Debug, Deserialize, ToSchema)]
pub struct ChangeRoleRequest {
    /// First day in the new roles. The current relationship ends the day before.
    pub effective_date: NaiveDate,
    pub from_role: String,
    pub to_role: String,
    pub reason: EndReason,
    pub note: Option<String>,
}

/// `PATCH /relationships/{id}`: the corrected values, replacing the old ones.
#[derive(Debug, Deserialize, ToSchema)]
pub struct CorrectRequest {
    #[serde(flatten)]
    pub relationship: RelationshipWrite,
    /// What was wrong. Required.
    pub note: String,
}

/// `POST /relationships/{id}/remove`
#[derive(Debug, Deserialize, ToSchema)]
pub struct RemoveRequest {
    /// Why the relationship was recorded in error. Required.
    pub reason: String,
}

/// One entry in a relationship's history.
#[derive(Debug, Serialize, ToSchema)]
pub struct RelationshipEvent {
    pub id: Uuid,
    pub relationship_id: Uuid,
    /// `added`, `ended`, `transferred`, `role_changed`, `corrected` or `removed`.
    pub action: String,
    pub effective_date: Option<NaiveDate>,
    pub reason: Option<String>,
    /// Hidden for incidents unless the caller holds `relationships.incident_notes`.
    pub note: Option<String>,
    /// True when a note exists but was withheld.
    pub note_hidden: bool,
    pub before: Option<Value>,
    pub after: Option<Value>,
    pub related_relationship_id: Option<Uuid>,
    pub actor_name: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RelationshipHistory {
    pub items: Vec<RelationshipEvent>,
}

/// The result of a transfer or role change.
#[derive(Debug, Serialize, ToSchema)]
pub struct RelationshipSuccession {
    /// The relationship that ended.
    pub ended: RelationshipEdge,
    /// The relationship that replaces it.
    pub started: RelationshipEdge,
}

// ---------------------------------------------------------------------------
// Read
// ---------------------------------------------------------------------------

/// `GET /api/v1/relationships`
#[utoipa::path(
    get,
    path = "/api/v1/relationships",
    tag = "relationships",
    security(("bearer_auth" = [])),
    params(ListParams),
    responses(
        (status = 200, description = "Relationships touching the filtered party, including ended ones", body = RelationshipList),
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
        (status = 200, description = "Nodes and current two-sided relationship edges", body = RelationshipGraph),
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

    // The network shows how people are connected now; ended links live in
    // each relationship's history instead.
    let mut edges = load_edges(&state.db, auth.organisation_id).await?;
    edges.retain(|edge| !edge.is_ended());

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

#[derive(Debug, FromRow)]
struct EventRow {
    id: Uuid,
    relationship_id: Uuid,
    action: String,
    effective_date: Option<NaiveDate>,
    reason: Option<String>,
    note: Option<String>,
    before: Option<Value>,
    after: Option<Value>,
    related_relationship_id: Option<Uuid>,
    actor_name: Option<String>,
    created_at: DateTime<Utc>,
}

/// `GET /api/v1/relationships/{id}/history`
///
/// The relationship's own events, plus those of the relationships it replaced
/// or that replaced it, oldest first.
#[utoipa::path(
    get,
    path = "/api/v1/relationships/{id}/history",
    tag = "relationships",
    security(("bearer_auth" = [])),
    params(("id" = Uuid, Path, description = "Relationship id")),
    responses(
        (status = 200, description = "History, oldest first", body = RelationshipHistory),
        (status = 401, description = "Missing or invalid access token", body = ErrorBody),
        (status = 403, description = "Missing relationships.read capability", body = ErrorBody),
        (status = 404, description = "Relationship not found", body = ErrorBody)
    )
)]
pub async fn history(
    State(state): State<AppState>,
    auth: AuthContext,
    Path(id): Path<Uuid>,
) -> Result<Json<RelationshipHistory>, AppError> {
    auth.require(Capability::RelationshipsRead)?;

    let rows = sqlx::query_as::<_, EventRow>(
        "WITH RECURSIVE chain(id) AS (
             SELECT $2::uuid
             UNION
             -- Walk both ways: to the relationship this one replaced, and to
             -- any that replaced it.
             SELECT CASE WHEN r.id = chain.id THEN r.previous_id ELSE r.id END
             FROM relationships r
             JOIN chain
               ON (r.id = chain.id AND r.previous_id IS NOT NULL)
               OR r.previous_id = chain.id
             WHERE r.organisation_id = $1
         )
         SELECT e.id, e.relationship_id, e.action, e.effective_date, e.reason, e.note,
                e.before, e.after, e.related_relationship_id,
                u.full_name AS actor_name, e.created_at
         FROM relationship_events e
         LEFT JOIN users u ON u.id = e.actor_user_id
         WHERE e.organisation_id = $1 AND e.relationship_id IN (SELECT id FROM chain)
         ORDER BY e.created_at, e.id",
    )
    .bind(auth.organisation_id)
    .bind(id)
    .fetch_all(&state.db)
    .await?;

    if rows.is_empty() {
        return Err(AppError::not_found("relationship not found"));
    }

    let can_read_incidents = auth.can(Capability::RelationshipsIncidentNotes);
    let items = rows
        .into_iter()
        .map(|row| {
            let incident = row.reason.as_deref() == Some(EndReason::Incident.as_str());
            let hide = incident && !can_read_incidents && row.note.is_some();
            RelationshipEvent {
                id: row.id,
                relationship_id: row.relationship_id,
                action: row.action,
                effective_date: row.effective_date,
                reason: row.reason,
                note: if hide { None } else { row.note },
                note_hidden: hide,
                before: row.before,
                after: row.after,
                related_relationship_id: row.related_relationship_id,
                actor_name: row.actor_name,
                created_at: row.created_at,
            }
        })
        .collect();

    Ok(Json(RelationshipHistory { items }))
}

// ---------------------------------------------------------------------------
// Write
// ---------------------------------------------------------------------------

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
        (status = 409, description = "The same relationship is already current", body = ErrorBody),
        (status = 422, description = "Validation failed", body = ErrorBody)
    )
)]
pub async fn create(
    State(state): State<AppState>,
    auth: AuthContext,
    Json(payload): Json<RelationshipWrite>,
) -> Result<(StatusCode, Json<RelationshipEdge>), AppError> {
    auth.require(Capability::RelationshipsWrite)?;
    let write = validate_write(&state.db, auth.organisation_id, &payload).await?;
    relationship_types::ensure_allowed(
        &state.db,
        auth.organisation_id,
        &write.from_role,
        &write.to_role,
    )
    .await?;
    ensure_not_duplicate(&state.db, auth.organisation_id, &write, None).await?;

    let mut tx = begin_as(&state, &auth).await?;
    let id = insert_relationship(&mut tx, auth.organisation_id, &write, None).await?;
    audit(&mut tx, &auth, "relationship.added", id, json!({})).await?;
    tx.commit().await?;

    let edge = load_existing(&state.db, auth.organisation_id, id).await?;
    Ok((StatusCode::CREATED, Json(edge)))
}

/// `POST /api/v1/relationships/{id}/end`
#[utoipa::path(
    post,
    path = "/api/v1/relationships/{id}/end",
    tag = "relationships",
    security(("bearer_auth" = [])),
    params(("id" = Uuid, Path, description = "Relationship id")),
    request_body = EndRequest,
    responses(
        (status = 200, description = "Relationship ended", body = RelationshipEdge),
        (status = 403, description = "Missing relationships.write capability", body = ErrorBody),
        (status = 404, description = "Relationship not found", body = ErrorBody),
        (status = 409, description = "Relationship already has an end date", body = ErrorBody),
        (status = 422, description = "Validation failed", body = ErrorBody)
    )
)]
pub async fn end(
    State(state): State<AppState>,
    auth: AuthContext,
    Path(id): Path<Uuid>,
    Json(payload): Json<EndRequest>,
) -> Result<Json<RelationshipEdge>, AppError> {
    auth.require(Capability::RelationshipsWrite)?;
    let edge = load_existing(&state.db, auth.organisation_id, id).await?;
    if edge.end_date.is_some() {
        return Err(AppError::conflict(
            "this relationship already has an end date; correct it instead",
        ));
    }
    validate_dates(edge.start_date, Some(payload.end_date))?;

    let mut tx = begin_as(&state, &auth).await?;
    sqlx::query(
        "UPDATE relationships SET end_date = $3, end_reason = $4
         WHERE organisation_id = $1 AND id = $2",
    )
    .bind(auth.organisation_id)
    .bind(id)
    .bind(payload.end_date)
    .bind(payload.reason.as_str())
    .execute(&mut *tx)
    .await?;

    record_event(
        &mut tx,
        &auth,
        Event {
            relationship_id: id,
            action: "ended",
            effective_date: Some(payload.end_date),
            reason: Some(payload.reason.as_str()),
            note: clean(payload.note.as_deref()),
            before: Some(snapshot(&edge)),
            after: Some(json!({
                "end_date": payload.end_date,
                "end_reason": payload.reason.as_str(),
            })),
            related: None,
        },
    )
    .await?;
    tx.commit().await?;

    Ok(Json(
        load_existing(&state.db, auth.organisation_id, id).await?,
    ))
}

/// `POST /api/v1/relationships/{id}/transfer`
#[utoipa::path(
    post,
    path = "/api/v1/relationships/{id}/transfer",
    tag = "relationships",
    security(("bearer_auth" = [])),
    params(("id" = Uuid, Path, description = "Relationship id")),
    request_body = TransferRequest,
    responses(
        (status = 200, description = "Relationship passed to the new party", body = RelationshipSuccession),
        (status = 403, description = "Missing relationships.write capability", body = ErrorBody),
        (status = 404, description = "Relationship not found", body = ErrorBody),
        (status = 409, description = "Relationship has already ended", body = ErrorBody),
        (status = 422, description = "Validation failed", body = ErrorBody)
    )
)]
pub async fn transfer(
    State(state): State<AppState>,
    auth: AuthContext,
    Path(id): Path<Uuid>,
    Json(payload): Json<TransferRequest>,
) -> Result<Json<RelationshipSuccession>, AppError> {
    auth.require(Capability::RelationshipsWrite)?;
    let edge = load_existing(&state.db, auth.organisation_id, id).await?;

    let mut next = write_from_edge(&edge);
    match payload.replace {
        Side::From => {
            next.from_kind = payload.new_kind;
            next.from_id = payload.new_id;
        }
        Side::To => {
            next.to_kind = payload.new_kind;
            next.to_id = payload.new_id;
        }
    }
    if next.from_kind == edge.from_kind
        && next.from_id == edge.from_id
        && next.to_kind == edge.to_kind
        && next.to_id == edge.to_id
    {
        return Err(AppError::validation(
            "choose a different party to transfer to",
        ));
    }

    succeed(
        &state,
        &auth,
        &edge,
        next,
        "transferred",
        payload.effective_date,
        payload.reason,
        clean(payload.note.as_deref()),
    )
    .await
    .map(Json)
}

/// `POST /api/v1/relationships/{id}/change-role`
#[utoipa::path(
    post,
    path = "/api/v1/relationships/{id}/change-role",
    tag = "relationships",
    security(("bearer_auth" = [])),
    params(("id" = Uuid, Path, description = "Relationship id")),
    request_body = ChangeRoleRequest,
    responses(
        (status = 200, description = "Relationship continued in new roles", body = RelationshipSuccession),
        (status = 403, description = "Missing relationships.write capability", body = ErrorBody),
        (status = 404, description = "Relationship not found", body = ErrorBody),
        (status = 409, description = "Relationship has already ended", body = ErrorBody),
        (status = 422, description = "Validation failed", body = ErrorBody)
    )
)]
pub async fn change_role(
    State(state): State<AppState>,
    auth: AuthContext,
    Path(id): Path<Uuid>,
    Json(payload): Json<ChangeRoleRequest>,
) -> Result<Json<RelationshipSuccession>, AppError> {
    auth.require(Capability::RelationshipsWrite)?;
    let edge = load_existing(&state.db, auth.organisation_id, id).await?;

    let mut next = write_from_edge(&edge);
    next.from_role = payload.from_role;
    next.to_role = payload.to_role;
    let (from_role, to_role) = validate_roles(&next)?;
    if from_role == edge.from_role && to_role == edge.to_role {
        return Err(AppError::validation("choose different roles"));
    }

    succeed(
        &state,
        &auth,
        &edge,
        next,
        "role_changed",
        payload.effective_date,
        payload.reason,
        clean(payload.note.as_deref()),
    )
    .await
    .map(Json)
}

/// `PATCH /api/v1/relationships/{id}`: correct a mistake.
#[utoipa::path(
    patch,
    path = "/api/v1/relationships/{id}",
    tag = "relationships",
    security(("bearer_auth" = [])),
    params(("id" = Uuid, Path, description = "Relationship id")),
    request_body = CorrectRequest,
    responses(
        (status = 200, description = "Relationship corrected", body = RelationshipEdge),
        (status = 403, description = "Missing relationships.write capability", body = ErrorBody),
        (status = 404, description = "Relationship not found", body = ErrorBody),
        (status = 409, description = "The corrected relationship duplicates a current one", body = ErrorBody),
        (status = 422, description = "Validation failed or nothing changed", body = ErrorBody)
    )
)]
pub async fn correct(
    State(state): State<AppState>,
    auth: AuthContext,
    Path(id): Path<Uuid>,
    Json(payload): Json<CorrectRequest>,
) -> Result<Json<RelationshipEdge>, AppError> {
    auth.require(Capability::RelationshipsWrite)?;
    let note = clean(Some(&payload.note))
        .ok_or_else(|| AppError::validation("explain what was wrong in the note"))?;
    let edge = load_existing(&state.db, auth.organisation_id, id).await?;
    let write = validate_write(&state.db, auth.organisation_id, &payload.relationship).await?;
    // A retired type may stay on a relationship, but changed roles must be
    // in the catalogue.
    if write.from_role != edge.from_role || write.to_role != edge.to_role {
        relationship_types::ensure_allowed(
            &state.db,
            auth.organisation_id,
            &write.from_role,
            &write.to_role,
        )
        .await?;
    }

    let before = snapshot(&edge);
    let after = write_snapshot(&write);
    if comparable(&before) == after {
        return Err(AppError::validation("nothing changed"));
    }
    ensure_not_duplicate(&state.db, auth.organisation_id, &write, Some(id)).await?;

    let mut tx = begin_as(&state, &auth).await?;
    sqlx::query(
        "UPDATE relationships SET
            from_contact_id = $3, from_organisation_id = $4,
            to_contact_id = $5, to_organisation_id = $6,
            from_role = $7, to_role = $8,
            start_date = $9, end_date = $10, end_reason = $11, notes = $12
         WHERE organisation_id = $1 AND id = $2",
    )
    .bind(auth.organisation_id)
    .bind(id)
    .bind(contact_endpoint(write.from_kind, write.from_id))
    .bind(organisation_endpoint(write.from_kind, write.from_id))
    .bind(contact_endpoint(write.to_kind, write.to_id))
    .bind(organisation_endpoint(write.to_kind, write.to_id))
    .bind(&write.from_role)
    .bind(&write.to_role)
    .bind(write.start_date)
    .bind(write.end_date)
    .bind(write.end_reason.map(EndReason::as_str))
    .bind(write.notes.as_deref())
    .execute(&mut *tx)
    .await?;

    record_event(
        &mut tx,
        &auth,
        Event {
            relationship_id: id,
            action: "corrected",
            effective_date: None,
            reason: None,
            note: Some(note),
            before: Some(before),
            after: Some(after),
            related: None,
        },
    )
    .await?;
    tx.commit().await?;

    Ok(Json(
        load_existing(&state.db, auth.organisation_id, id).await?,
    ))
}

/// `POST /api/v1/relationships/{id}/remove`: hide one recorded in error.
#[utoipa::path(
    post,
    path = "/api/v1/relationships/{id}/remove",
    tag = "relationships",
    security(("bearer_auth" = [])),
    params(("id" = Uuid, Path, description = "Relationship id")),
    request_body = RemoveRequest,
    responses(
        (status = 204, description = "Relationship removed; its history is kept"),
        (status = 403, description = "Missing relationships.remove capability", body = ErrorBody),
        (status = 404, description = "Relationship not found", body = ErrorBody),
        (status = 422, description = "A reason is required", body = ErrorBody)
    )
)]
pub async fn remove(
    State(state): State<AppState>,
    auth: AuthContext,
    Path(id): Path<Uuid>,
    Json(payload): Json<RemoveRequest>,
) -> Result<StatusCode, AppError> {
    auth.require(Capability::RelationshipsRemove)?;
    let reason = clean(Some(&payload.reason))
        .ok_or_else(|| AppError::validation("give a reason for removing this relationship"))?;
    let edge = load_existing(&state.db, auth.organisation_id, id).await?;

    let mut tx = begin_as(&state, &auth).await?;
    sqlx::query(
        "UPDATE relationships SET removed_at = now(), removed_by = $3, remove_reason = $4
         WHERE organisation_id = $1 AND id = $2",
    )
    .bind(auth.organisation_id)
    .bind(id)
    .bind(auth.user_id)
    .bind(&reason)
    .execute(&mut *tx)
    .await?;

    record_event(
        &mut tx,
        &auth,
        Event {
            relationship_id: id,
            action: "removed",
            effective_date: None,
            reason: None,
            note: Some(reason),
            before: Some(snapshot(&edge)),
            after: None,
            related: None,
        },
    )
    .await?;
    tx.commit().await?;

    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// A validated relationship write.
struct ValidWrite {
    from_kind: PartyKind,
    from_id: Uuid,
    from_role: String,
    to_kind: PartyKind,
    to_id: Uuid,
    to_role: String,
    start_date: Option<NaiveDate>,
    end_date: Option<NaiveDate>,
    end_reason: Option<EndReason>,
    notes: Option<String>,
}

async fn validate_write(
    db: &PgPool,
    organisation_id: Uuid,
    payload: &RelationshipWrite,
) -> Result<ValidWrite, AppError> {
    let (from_role, to_role) = validate_roles(payload)?;
    validate_endpoints(payload)?;
    validate_dates(payload.start_date, payload.end_date)?;
    if payload.end_date.is_some() != payload.end_reason.is_some() {
        return Err(AppError::validation(
            "give an end_reason whenever end_date is set (and only then)",
        ));
    }
    check_endpoints_exist(db, organisation_id, payload.from_kind, payload.from_id).await?;
    check_endpoints_exist(db, organisation_id, payload.to_kind, payload.to_id).await?;

    Ok(ValidWrite {
        from_kind: payload.from_kind,
        from_id: payload.from_id,
        from_role,
        to_kind: payload.to_kind,
        to_id: payload.to_id,
        to_role,
        start_date: payload.start_date,
        end_date: payload.end_date,
        end_reason: payload.end_reason,
        notes: clean(payload.notes.as_deref()),
    })
}

/// The same parties in the same roles may only have one live relationship.
async fn ensure_not_duplicate(
    db: &PgPool,
    organisation_id: Uuid,
    write: &ValidWrite,
    except: Option<Uuid>,
) -> Result<(), AppError> {
    let duplicate: bool = sqlx::query_scalar(
        "SELECT EXISTS(
            SELECT 1 FROM relationships
            WHERE organisation_id = $1
              AND removed_at IS NULL
              AND (end_date IS NULL OR end_date >= CURRENT_DATE)
              AND id IS DISTINCT FROM $8
              AND from_contact_id IS NOT DISTINCT FROM $2
              AND from_organisation_id IS NOT DISTINCT FROM $3
              AND to_contact_id IS NOT DISTINCT FROM $4
              AND to_organisation_id IS NOT DISTINCT FROM $5
              AND from_role = $6
              AND to_role = $7
        )",
    )
    .bind(organisation_id)
    .bind(contact_endpoint(write.from_kind, write.from_id))
    .bind(organisation_endpoint(write.from_kind, write.from_id))
    .bind(contact_endpoint(write.to_kind, write.to_id))
    .bind(organisation_endpoint(write.to_kind, write.to_id))
    .bind(&write.from_role)
    .bind(&write.to_role)
    .bind(except)
    .fetch_one(db)
    .await?;

    if duplicate {
        return Err(AppError::conflict(
            "the same relationship is already current",
        ));
    }
    Ok(())
}

/// End `edge` the day before `effective_date` and start `next` on that date,
/// linked to it. Used by transfer and change role.
#[allow(clippy::too_many_arguments)]
async fn succeed(
    state: &AppState,
    auth: &AuthContext,
    edge: &RelationshipEdge,
    mut next: RelationshipWrite,
    action: &'static str,
    effective_date: NaiveDate,
    reason: EndReason,
    note: Option<String>,
) -> Result<RelationshipSuccession, AppError> {
    if edge.is_ended() {
        return Err(AppError::conflict("this relationship has already ended"));
    }
    let last_day = effective_date - Duration::days(1);
    if edge.start_date.is_some_and(|start| last_day < start) {
        return Err(AppError::validation(
            "the change must take effect at least a day after the relationship started",
        ));
    }
    if edge.end_date.is_some_and(|end| last_day > end) {
        return Err(AppError::validation(
            "the change must take effect on or before the relationship's end date",
        ));
    }

    next.start_date = Some(effective_date);
    next.end_date = None;
    next.end_reason = None;
    let write = validate_write(&state.db, auth.organisation_id, &next).await?;
    if action == "role_changed" {
        relationship_types::ensure_allowed(
            &state.db,
            auth.organisation_id,
            &write.from_role,
            &write.to_role,
        )
        .await?;
    }
    ensure_not_duplicate(&state.db, auth.organisation_id, &write, None).await?;

    let mut tx = begin_as(state, auth).await?;
    sqlx::query(
        "UPDATE relationships SET end_date = $3, end_reason = $4
         WHERE organisation_id = $1 AND id = $2",
    )
    .bind(auth.organisation_id)
    .bind(edge.id)
    .bind(last_day)
    .bind(reason.as_str())
    .execute(&mut *tx)
    .await?;

    // The trigger records the new relationship's "added" event with this
    // reason and note, linked back through previous_id.
    set_local(&mut tx, "carelibre.event_reason", reason.as_str()).await?;
    set_local(
        &mut tx,
        "carelibre.event_note",
        note.as_deref().unwrap_or(""),
    )
    .await?;
    let started_id =
        insert_relationship(&mut tx, auth.organisation_id, &write, Some(edge.id)).await?;
    set_local(&mut tx, "carelibre.event_reason", "").await?;
    set_local(&mut tx, "carelibre.event_note", "").await?;

    record_event(
        &mut tx,
        auth,
        Event {
            relationship_id: edge.id,
            action,
            effective_date: Some(effective_date),
            reason: Some(reason.as_str()),
            note,
            before: Some(snapshot(edge)),
            after: Some(write_snapshot(&write)),
            related: Some(started_id),
        },
    )
    .await?;
    tx.commit().await?;

    Ok(RelationshipSuccession {
        ended: load_existing(&state.db, auth.organisation_id, edge.id).await?,
        started: load_existing(&state.db, auth.organisation_id, started_id).await?,
    })
}

async fn insert_relationship(
    tx: &mut Transaction<'_, Postgres>,
    organisation_id: Uuid,
    write: &ValidWrite,
    previous_id: Option<Uuid>,
) -> Result<Uuid, AppError> {
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO relationships
            (id, organisation_id, from_contact_id, from_organisation_id,
             to_contact_id, to_organisation_id, from_role, to_role,
             start_date, end_date, end_reason, notes, previous_id)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13)",
    )
    .bind(id)
    .bind(organisation_id)
    .bind(contact_endpoint(write.from_kind, write.from_id))
    .bind(organisation_endpoint(write.from_kind, write.from_id))
    .bind(contact_endpoint(write.to_kind, write.to_id))
    .bind(organisation_endpoint(write.to_kind, write.to_id))
    .bind(&write.from_role)
    .bind(&write.to_role)
    .bind(write.start_date)
    .bind(write.end_date)
    .bind(write.end_reason.map(EndReason::as_str))
    .bind(write.notes.as_deref())
    .bind(previous_id)
    .execute(&mut **tx)
    .await?;
    Ok(id)
}

/// Start a transaction that names the acting user for history triggers.
async fn begin_as<'a>(
    state: &'a AppState,
    auth: &AuthContext,
) -> Result<Transaction<'a, Postgres>, AppError> {
    let mut tx = state.db.begin().await?;
    set_local(
        &mut tx,
        "carelibre.actor_user_id",
        &auth.user_id.to_string(),
    )
    .await?;
    Ok(tx)
}

async fn set_local(
    tx: &mut Transaction<'_, Postgres>,
    key: &str,
    value: &str,
) -> Result<(), AppError> {
    sqlx::query("SELECT set_config($1, $2, true)")
        .bind(key)
        .bind(value)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

struct Event<'a> {
    relationship_id: Uuid,
    action: &'static str,
    effective_date: Option<NaiveDate>,
    reason: Option<&'a str>,
    note: Option<String>,
    before: Option<Value>,
    after: Option<Value>,
    related: Option<Uuid>,
}

async fn record_event(
    tx: &mut Transaction<'_, Postgres>,
    auth: &AuthContext,
    event: Event<'_>,
) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO relationship_events
            (id, organisation_id, relationship_id, action, effective_date, reason, note,
             before, after, related_relationship_id, actor_user_id)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)",
    )
    .bind(Uuid::now_v7())
    .bind(auth.organisation_id)
    .bind(event.relationship_id)
    .bind(event.action)
    .bind(event.effective_date)
    .bind(event.reason)
    .bind(event.note.as_deref())
    .bind(event.before)
    .bind(event.after)
    .bind(event.related)
    .bind(auth.user_id)
    .execute(&mut **tx)
    .await?;

    // The audit log gets a summary only; notes may be sensitive.
    audit(
        tx,
        auth,
        &format!("relationship.{}", event.action),
        event.relationship_id,
        json!({
            "effective_date": event.effective_date,
            "reason": event.reason,
            "related_relationship_id": event.related,
        }),
    )
    .await
}

async fn audit(
    tx: &mut Transaction<'_, Postgres>,
    auth: &AuthContext,
    action: &str,
    relationship_id: Uuid,
    metadata: Value,
) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO audit_log
            (organisation_id, actor_user_id, action, entity_type, entity_id, metadata)
         VALUES ($1, $2, $3, 'relationship', $4, $5)",
    )
    .bind(auth.organisation_id)
    .bind(auth.user_id)
    .bind(action)
    .bind(relationship_id)
    .bind(metadata)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn load_existing(
    db: &PgPool,
    organisation_id: Uuid,
    id: Uuid,
) -> Result<RelationshipEdge, AppError> {
    load_edge(db, organisation_id, id)
        .await?
        .ok_or_else(|| AppError::not_found("relationship not found"))
}

fn write_from_edge(edge: &RelationshipEdge) -> RelationshipWrite {
    RelationshipWrite {
        from_kind: edge.from_kind,
        from_id: edge.from_id,
        from_role: edge.from_role.clone(),
        to_kind: edge.to_kind,
        to_id: edge.to_id,
        to_role: edge.to_role.clone(),
        start_date: edge.start_date,
        end_date: edge.end_date,
        end_reason: None,
        notes: edge.notes.clone(),
    }
}

/// The relationship as it stood, with party names, for history.
fn snapshot(edge: &RelationshipEdge) -> Value {
    json!({
        "from_kind": edge.from_kind,
        "from_id": edge.from_id,
        "from_name": edge.from_name,
        "from_role": edge.from_role,
        "to_kind": edge.to_kind,
        "to_id": edge.to_id,
        "to_name": edge.to_name,
        "to_role": edge.to_role,
        "start_date": edge.start_date,
        "end_date": edge.end_date,
        "end_reason": edge.end_reason,
        "notes": edge.notes,
    })
}

/// A snapshot without names, comparable with [`write_snapshot`].
fn comparable(snapshot: &Value) -> Value {
    let mut value = snapshot.clone();
    if let Some(fields) = value.as_object_mut() {
        fields.remove("from_name");
        fields.remove("to_name");
    }
    value
}

/// The values a write will store, in the same shape as [`snapshot`].
fn write_snapshot(write: &ValidWrite) -> Value {
    json!({
        "from_kind": write.from_kind,
        "from_id": write.from_id,
        "from_role": write.from_role,
        "to_kind": write.to_kind,
        "to_id": write.to_id,
        "to_role": write.to_role,
        "start_date": write.start_date,
        "end_date": write.end_date,
        "end_reason": write.end_reason.map(EndReason::as_str),
        "notes": write.notes,
    })
}

fn clean(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
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
