//! Rostering API: sites, shifts, assignments, explicit constraints and
//! evidence-based worker suitability.
//!
//! Suitability is advisory. It never auto-assigns a worker and returns the
//! reasons behind each score so a coordinator can judge them.

use std::collections::{HashMap, HashSet};

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, Postgres, QueryBuilder};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::{
    auth::extractor::AuthContext,
    error::{AppError, ErrorBody},
    permissions::Capability,
    rostering::{
        score_candidate, ConstraintEvidence, ConstraintKind, ContextEvidence, HistorySignal,
        ScoreInput,
    },
    state::AppState,
};

// ---------------------------------------------------------------------------
// DTOs
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, ToSchema)]
pub struct SiteDto {
    pub id: Uuid,
    pub name: String,
    pub division_id: Option<Uuid>,
    pub division_name: Option<String>,
    pub address: Option<String>,
    pub notes: Option<String>,
    pub is_active: bool,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct SiteWrite {
    pub name: String,
    pub division_id: Option<Uuid>,
    pub address: Option<String>,
    pub notes: Option<String>,
    pub is_active: Option<bool>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DivisionDto {
    pub id: Uuid,
    pub name: String,
    pub code: Option<String>,
    pub parent_id: Option<Uuid>,
    pub is_active: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ServiceDto {
    pub id: Uuid,
    pub name: String,
    pub code: Option<String>,
    pub division_id: Option<Uuid>,
    pub division_name: Option<String>,
    pub is_active: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CostCentreDto {
    pub id: Uuid,
    pub code: String,
    pub name: String,
    pub parent_id: Option<Uuid>,
    pub is_active: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ShiftDto {
    pub id: Uuid,
    pub title: Option<String>,
    pub kind: String,
    pub starts_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
    pub location: Option<String>,
    pub status: String,
    pub division_id: Option<Uuid>,
    pub division_name: Option<String>,
    pub service_id: Option<Uuid>,
    pub service_name: Option<String>,
    pub site_id: Option<Uuid>,
    pub site_name: Option<String>,
    pub cost_centre_id: Option<Uuid>,
    pub cost_centre_name: Option<String>,
    pub cost_centre_code: Option<String>,
    pub series_id: Option<Uuid>,
    pub recurrence: Option<String>,
    pub client_count: i64,
    pub worker_count: i64,
    pub clients: Vec<PartyRef>,
    pub workers: Vec<PartyRef>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ShiftList {
    pub items: Vec<ShiftDto>,
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct ShiftListParams {
    pub from: Option<DateTime<Utc>>,
    pub to: Option<DateTime<Utc>>,
    pub status: Option<String>,
    pub site_id: Option<Uuid>,
    pub service_id: Option<Uuid>,
    pub division_id: Option<Uuid>,
    pub cost_centre_id: Option<Uuid>,
    pub worker_id: Option<Uuid>,
    pub client_id: Option<Uuid>,
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ShiftWrite {
    pub title: Option<String>,
    pub kind: Option<String>,
    pub starts_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
    pub location: Option<String>,
    pub status: Option<String>,
    pub division_id: Option<Uuid>,
    pub service_id: Option<Uuid>,
    pub site_id: Option<Uuid>,
    pub notes: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct BatchShiftWrite {
    pub shifts: Vec<ShiftWrite>,
    pub recurrence: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CopyShiftWrite {
    pub starts_at: Option<DateTime<Utc>>,
    pub ends_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub include_assignments: bool,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ShiftUpdate {
    pub title: Option<String>,
    pub kind: Option<String>,
    pub starts_at: Option<DateTime<Utc>>,
    pub ends_at: Option<DateTime<Utc>>,
    pub location: Option<String>,
    pub status: Option<String>,
    pub division_id: Option<Uuid>,
    pub service_id: Option<Uuid>,
    pub site_id: Option<Uuid>,
    pub notes: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AssignmentDto {
    pub id: Uuid,
    pub role: String,
    pub contact_id: Uuid,
    pub contact_name: String,
    pub session_id: Option<Uuid>,
    pub status: String,
    pub outcome: Option<String>,
    pub outcome_rating: Option<i16>,
    pub notes: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ShiftDetail {
    pub shift: ShiftDto,
    pub assignments: Vec<AssignmentDto>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct AssignmentWrite {
    pub contact_id: Uuid,
    /// `worker` or `client`.
    pub role: String,
    pub session_id: Option<Uuid>,
    pub status: Option<String>,
    pub notes: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct AssignmentUpdate {
    pub status: Option<String>,
    pub outcome: Option<String>,
    pub outcome_rating: Option<i16>,
    pub notes: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ContextRef {
    pub id: Uuid,
    pub name: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
pub struct PartyRef {
    pub id: Uuid,
    pub display_name: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SuitabilityCandidate {
    pub contact_id: Uuid,
    pub display_name: String,
    pub score: f64,
    pub reasons: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SuitabilityResponse {
    pub shift_id: Uuid,
    pub site: Option<ContextRef>,
    pub service: Option<ContextRef>,
    pub division: Option<ContextRef>,
    pub clients: Vec<PartyRef>,
    pub assigned_workers: Vec<PartyRef>,
    pub candidates: Vec<SuitabilityCandidate>,
    pub blocked: Vec<SuitabilityCandidate>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RosterConstraintDto {
    pub id: Uuid,
    pub worker_contact_id: Uuid,
    pub worker_name: String,
    pub target_kind: String,
    pub target_id: Uuid,
    pub target_name: String,
    pub kind: String,
    pub strength: i16,
    pub reason: Option<String>,
    pub source: String,
    pub is_active: bool,
    pub start_date: Option<NaiveDate>,
    pub end_date: Option<NaiveDate>,
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct ConstraintListParams {
    pub worker_id: Option<Uuid>,
    pub target_contact_id: Option<Uuid>,
    pub site_id: Option<Uuid>,
    pub service_id: Option<Uuid>,
    pub division_id: Option<Uuid>,
    pub include_inactive: Option<bool>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct RosterConstraintWrite {
    pub worker_contact_id: Uuid,
    /// `contact`, `site`, `service` or `division`.
    pub target_kind: String,
    pub target_id: Uuid,
    /// `preferred`, `avoid` or `blocked`.
    pub kind: String,
    pub strength: Option<i16>,
    pub reason: Option<String>,
    pub source: Option<String>,
    pub start_date: Option<NaiveDate>,
    pub end_date: Option<NaiveDate>,
    pub is_active: Option<bool>,
}

// ---------------------------------------------------------------------------
// Rows
// ---------------------------------------------------------------------------

#[derive(Debug, FromRow)]
struct SiteRow {
    id: Uuid,
    name: String,
    division_id: Option<Uuid>,
    division_name: Option<String>,
    address: Option<String>,
    notes: Option<String>,
    is_active: bool,
}

#[derive(Debug, FromRow)]
struct DivisionRow {
    id: Uuid,
    name: String,
    code: Option<String>,
    parent_id: Option<Uuid>,
    is_active: bool,
}

#[derive(Debug, FromRow)]
struct ServiceRow {
    id: Uuid,
    name: String,
    code: Option<String>,
    division_id: Option<Uuid>,
    division_name: Option<String>,
    is_active: bool,
}

#[derive(Debug, FromRow)]
struct CostCentreRow {
    id: Uuid,
    code: String,
    name: String,
    parent_id: Option<Uuid>,
    is_active: bool,
}

#[derive(Debug, FromRow)]
struct ShiftRow {
    id: Uuid,
    title: Option<String>,
    kind: String,
    starts_at: DateTime<Utc>,
    ends_at: DateTime<Utc>,
    location: Option<String>,
    status: String,
    division_id: Option<Uuid>,
    division_name: Option<String>,
    service_id: Option<Uuid>,
    service_name: Option<String>,
    site_id: Option<Uuid>,
    site_name: Option<String>,
    cost_centre_id: Option<Uuid>,
    cost_centre_name: Option<String>,
    cost_centre_code: Option<String>,
    series_id: Option<Uuid>,
    recurrence: Option<String>,
    client_count: i64,
    worker_count: i64,
    clients: sqlx::types::Json<Vec<PartyRef>>,
    workers: sqlx::types::Json<Vec<PartyRef>>,
    notes: Option<String>,
}

#[derive(Debug, FromRow)]
struct AssignmentRow {
    id: Uuid,
    role: String,
    contact_id: Uuid,
    contact_name: String,
    session_id: Option<Uuid>,
    status: String,
    outcome: Option<String>,
    outcome_rating: Option<i16>,
    notes: Option<String>,
}

#[derive(Debug, FromRow)]
struct ConstraintRow {
    id: Uuid,
    worker_contact_id: Uuid,
    worker_name: String,
    target_kind: String,
    target_id: Uuid,
    target_name: String,
    kind: String,
    strength: i16,
    reason: Option<String>,
    source: String,
    is_active: bool,
    start_date: Option<NaiveDate>,
    end_date: Option<NaiveDate>,
}

#[derive(Debug, FromRow)]
struct ContactRow {
    id: Uuid,
    display_name: String,
}

#[derive(Debug, FromRow)]
struct WorkerClientHistoryRow {
    worker_contact_id: Uuid,
    client_contact_id: Uuid,
    attended_shifts: i64,
    no_show_shifts: i64,
    cancelled_shifts: i64,
    last_worked_at: Option<DateTime<Utc>>,
    average_rating: Option<f64>,
    positive_outcomes: i64,
    negative_outcomes: i64,
}

#[derive(Debug, FromRow)]
struct WorkerWorkerHistoryRow {
    worker_a_id: Uuid,
    worker_b_id: Uuid,
    attended_shifts: i64,
    no_show_shifts: i64,
    last_worked_at: Option<DateTime<Utc>>,
}

#[derive(Debug, FromRow)]
struct WorkerContextHistoryRow {
    worker_contact_id: Uuid,
    context_kind: String,
    context_id: Uuid,
    attended_shifts: i64,
    last_worked_at: Option<DateTime<Utc>>,
}

const SHIFT_SELECT: &str = r"
SELECT s.id, s.title, s.kind, s.starts_at, s.ends_at, s.location, s.status,
       s.division_id, d.name AS division_name,
       s.service_id, sv.name AS service_name,
       s.site_id, st.name AS site_name,
       cc.id AS cost_centre_id,
       cc.name AS cost_centre_name,
       cc.code AS cost_centre_code,
       s.series_id,
       s.recurrence,
       (SELECT count(*) FROM client_assignments ca
         WHERE ca.organisation_id = s.organisation_id AND ca.shift_id = s.id) AS client_count,
       (SELECT count(*) FROM worker_assignments wa
         WHERE wa.organisation_id = s.organisation_id AND wa.shift_id = s.id) AS worker_count,
       COALESCE((
         SELECT jsonb_agg(jsonb_build_object('id', c.id, 'display_name', c.display_name)
                          ORDER BY c.display_name)
         FROM client_assignments ca
         JOIN contacts c
           ON c.organisation_id = ca.organisation_id AND c.id = ca.contact_id
         WHERE ca.organisation_id = s.organisation_id AND ca.shift_id = s.id
       ), '[]'::jsonb) AS clients,
       COALESCE((
         SELECT jsonb_agg(jsonb_build_object('id', c.id, 'display_name', c.display_name)
                          ORDER BY c.display_name)
         FROM worker_assignments wa
         JOIN contacts c
           ON c.organisation_id = wa.organisation_id AND c.id = wa.contact_id
         WHERE wa.organisation_id = s.organisation_id AND wa.shift_id = s.id
       ), '[]'::jsonb) AS workers,
       s.notes
FROM shifts s
LEFT JOIN divisions d ON d.organisation_id = s.organisation_id AND d.id = s.division_id
LEFT JOIN services sv ON sv.organisation_id = s.organisation_id AND sv.id = s.service_id
LEFT JOIN sites st ON st.organisation_id = s.organisation_id AND st.id = s.site_id
LEFT JOIN LATERAL (
    SELECT cc.id, cc.name, cc.code
    FROM cost_centres cc
    WHERE cc.organisation_id = s.organisation_id
      AND cc.id = COALESCE(
        (SELECT cca.cost_centre_id
         FROM cost_centre_assignments cca
         WHERE cca.organisation_id = s.organisation_id
           AND cca.entity_type = 'shift' AND cca.entity_id = s.id),
        (SELECT cca.cost_centre_id
         FROM cost_centre_assignments cca
         WHERE cca.organisation_id = s.organisation_id
           AND cca.entity_type = 'division' AND cca.entity_id = s.division_id)
      )
) cc ON TRUE
";

const ASSIGNMENT_SELECT: &str = r"
SELECT ca.id, 'client'::text AS role, ca.contact_id, c.display_name AS contact_name,
       ca.session_id, ca.status, NULL::text AS outcome, NULL::smallint AS outcome_rating,
       ca.notes
FROM client_assignments ca
JOIN contacts c ON c.organisation_id = ca.organisation_id AND c.id = ca.contact_id
WHERE ca.organisation_id = $1 AND ca.shift_id = $2
UNION ALL
SELECT wa.id, 'worker'::text AS role, wa.contact_id, c.display_name AS contact_name,
       wa.session_id, wa.status, wa.outcome, wa.outcome_rating, wa.notes
FROM worker_assignments wa
JOIN contacts c ON c.organisation_id = wa.organisation_id AND c.id = wa.contact_id
WHERE wa.organisation_id = $1 AND wa.shift_id = $2
ORDER BY role, contact_name, id
";

const CONSTRAINT_SELECT: &str = r"
SELECT rc.id, rc.worker_contact_id, wc.display_name AS worker_name,
       CASE
         WHEN rc.target_contact_id IS NOT NULL THEN 'contact'
         WHEN rc.target_site_id IS NOT NULL THEN 'site'
         WHEN rc.target_service_id IS NOT NULL THEN 'service'
         ELSE 'division'
       END AS target_kind,
       COALESCE(rc.target_contact_id, rc.target_site_id, rc.target_service_id,
                rc.target_division_id) AS target_id,
       COALESCE(tc.display_name, st.name, sv.name, dv.name, 'Unknown') AS target_name,
       rc.kind, rc.strength, rc.reason, rc.source, rc.is_active,
       rc.start_date, rc.end_date
FROM roster_constraints rc
JOIN contacts wc ON wc.organisation_id = rc.organisation_id AND wc.id = rc.worker_contact_id
LEFT JOIN contacts tc
       ON tc.organisation_id = rc.organisation_id AND tc.id = rc.target_contact_id
LEFT JOIN sites st
       ON st.organisation_id = rc.organisation_id AND st.id = rc.target_site_id
LEFT JOIN services sv
       ON sv.organisation_id = rc.organisation_id AND sv.id = rc.target_service_id
LEFT JOIN divisions dv
       ON dv.organisation_id = rc.organisation_id AND dv.id = rc.target_division_id
";

// ---------------------------------------------------------------------------
// Sites
// ---------------------------------------------------------------------------

#[utoipa::path(
    get,
    path = "/api/v1/sites",
    tag = "rostering",
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Sites in this organisation", body = Vec<SiteDto>),
        (status = 401, description = "Missing or invalid access token", body = ErrorBody),
        (status = 403, description = "Missing rosters.read capability", body = ErrorBody)
    )
)]
pub async fn list_sites(
    State(state): State<AppState>,
    auth: AuthContext,
) -> Result<Json<Vec<SiteDto>>, AppError> {
    auth.require(Capability::RostersRead)?;
    let rows = sqlx::query_as::<_, SiteRow>(
        "SELECT st.id, st.name, st.division_id, d.name AS division_name,
                st.address, st.notes, st.is_active
         FROM sites st
         LEFT JOIN divisions d ON d.organisation_id = st.organisation_id AND d.id = st.division_id
         WHERE st.organisation_id = $1
         ORDER BY st.name",
    )
    .bind(auth.organisation_id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(rows.into_iter().map(site_dto).collect()))
}

#[utoipa::path(
    post,
    path = "/api/v1/sites",
    tag = "rostering",
    security(("bearer_auth" = [])),
    request_body = SiteWrite,
    responses(
        (status = 201, description = "Site created", body = SiteDto),
        (status = 401, description = "Missing or invalid access token", body = ErrorBody),
        (status = 403, description = "Missing rosters.write capability", body = ErrorBody),
        (status = 409, description = "Site name already exists", body = ErrorBody),
        (status = 422, description = "Validation failed", body = ErrorBody)
    )
)]
pub async fn create_site(
    State(state): State<AppState>,
    auth: AuthContext,
    Json(payload): Json<SiteWrite>,
) -> Result<(StatusCode, Json<SiteDto>), AppError> {
    auth.require(Capability::RostersWrite)?;
    let name = payload.name.trim().to_owned();
    if name.is_empty() || name.chars().count() > 200 {
        return Err(AppError::validation(
            "name must be between 1 and 200 characters",
        ));
    }
    if let Some(division_id) = payload.division_id {
        ensure_division(&state, auth.organisation_id, division_id).await?;
    }

    let duplicate: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM sites WHERE organisation_id=$1 AND lower(name)=lower($2))",
    )
    .bind(auth.organisation_id)
    .bind(&name)
    .fetch_one(&state.db)
    .await?;
    if duplicate {
        return Err(AppError::conflict("a site with that name already exists"));
    }

    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO sites (id, organisation_id, division_id, name, address, notes, is_active)
         VALUES ($1,$2,$3,$4,$5,$6,$7)",
    )
    .bind(id)
    .bind(auth.organisation_id)
    .bind(payload.division_id)
    .bind(&name)
    .bind(
        payload
            .address
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty()),
    )
    .bind(
        payload
            .notes
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty()),
    )
    .bind(payload.is_active.unwrap_or(true))
    .execute(&state.db)
    .await?;

    let row = sqlx::query_as::<_, SiteRow>(
        "SELECT st.id, st.name, st.division_id, d.name AS division_name,
                st.address, st.notes, st.is_active
         FROM sites st
         LEFT JOIN divisions d ON d.organisation_id = st.organisation_id AND d.id = st.division_id
         WHERE st.organisation_id=$1 AND st.id=$2",
    )
    .bind(auth.organisation_id)
    .bind(id)
    .fetch_one(&state.db)
    .await?;

    Ok((StatusCode::CREATED, Json(site_dto(row))))
}

// ---------------------------------------------------------------------------
// Filter lists
// ---------------------------------------------------------------------------

#[utoipa::path(
    get,
    path = "/api/v1/divisions",
    tag = "rostering",
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Divisions in this organisation", body = Vec<DivisionDto>),
        (status = 401, description = "Missing or invalid access token", body = ErrorBody),
        (status = 403, description = "Missing divisions.read capability", body = ErrorBody)
    )
)]
pub async fn list_divisions(
    State(state): State<AppState>,
    auth: AuthContext,
) -> Result<Json<Vec<DivisionDto>>, AppError> {
    auth.require(Capability::RostersRead)?;
    let rows = sqlx::query_as::<_, DivisionRow>(
        "SELECT id, name, code, parent_id, is_active
         FROM divisions WHERE organisation_id=$1 ORDER BY name",
    )
    .bind(auth.organisation_id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(
        rows.into_iter()
            .map(|row| DivisionDto {
                id: row.id,
                name: row.name,
                code: row.code,
                parent_id: row.parent_id,
                is_active: row.is_active,
            })
            .collect(),
    ))
}

#[utoipa::path(
    get,
    path = "/api/v1/services",
    tag = "rostering",
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Services in this organisation", body = Vec<ServiceDto>),
        (status = 401, description = "Missing or invalid access token", body = ErrorBody),
        (status = 403, description = "Missing services.read capability", body = ErrorBody)
    )
)]
pub async fn list_services(
    State(state): State<AppState>,
    auth: AuthContext,
) -> Result<Json<Vec<ServiceDto>>, AppError> {
    auth.require(Capability::RostersRead)?;
    let rows = sqlx::query_as::<_, ServiceRow>(
        "SELECT sv.id, sv.name, sv.code, sv.division_id, d.name AS division_name, sv.is_active
         FROM services sv
         LEFT JOIN divisions d
           ON d.organisation_id = sv.organisation_id AND d.id = sv.division_id
         WHERE sv.organisation_id=$1 ORDER BY sv.name",
    )
    .bind(auth.organisation_id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(
        rows.into_iter()
            .map(|row| ServiceDto {
                id: row.id,
                name: row.name,
                code: row.code,
                division_id: row.division_id,
                division_name: row.division_name,
                is_active: row.is_active,
            })
            .collect(),
    ))
}

#[utoipa::path(
    get,
    path = "/api/v1/cost-centres",
    tag = "rostering",
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Cost centres in this organisation", body = Vec<CostCentreDto>),
        (status = 401, description = "Missing or invalid access token", body = ErrorBody),
        (status = 403, description = "Missing cost_centres.read capability", body = ErrorBody)
    )
)]
pub async fn list_cost_centres(
    State(state): State<AppState>,
    auth: AuthContext,
) -> Result<Json<Vec<CostCentreDto>>, AppError> {
    auth.require(Capability::RostersRead)?;
    let rows = sqlx::query_as::<_, CostCentreRow>(
        "SELECT id, code, name, parent_id, is_active
         FROM cost_centres WHERE organisation_id=$1 ORDER BY code, name",
    )
    .bind(auth.organisation_id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(
        rows.into_iter()
            .map(|row| CostCentreDto {
                id: row.id,
                code: row.code,
                name: row.name,
                parent_id: row.parent_id,
                is_active: row.is_active,
            })
            .collect(),
    ))
}

// ---------------------------------------------------------------------------
// Shifts
// ---------------------------------------------------------------------------

#[utoipa::path(
    get,
    path = "/api/v1/shifts",
    tag = "rostering",
    security(("bearer_auth" = [])),
    params(ShiftListParams),
    responses(
        (status = 200, description = "Shifts in this organisation", body = ShiftList),
        (status = 401, description = "Missing or invalid access token", body = ErrorBody),
        (status = 403, description = "Missing rosters.read capability", body = ErrorBody)
    )
)]
pub async fn list_shifts(
    State(state): State<AppState>,
    auth: AuthContext,
    Query(params): Query<ShiftListParams>,
) -> Result<Json<ShiftList>, AppError> {
    auth.require(Capability::RostersRead)?;
    let limit = i64::try_from(params.limit.unwrap_or(200).clamp(1, 1000)).unwrap_or(1000);

    let mut query = QueryBuilder::<Postgres>::new(SHIFT_SELECT);
    query.push(" WHERE s.organisation_id = ");
    query.push_bind(auth.organisation_id);

    if let Some(from) = params.from {
        query.push(" AND s.starts_at >= ");
        query.push_bind(from);
    }
    if let Some(to) = params.to {
        query.push(" AND s.starts_at < ");
        query.push_bind(to);
    }
    if let Some(status) = params.status.as_deref() {
        if status != "all" {
            query.push(" AND s.status = ");
            query.push_bind(status.to_owned());
        }
    }
    if let Some(site_id) = params.site_id {
        query.push(" AND s.site_id = ");
        query.push_bind(site_id);
    }
    if let Some(service_id) = params.service_id {
        query.push(" AND s.service_id = ");
        query.push_bind(service_id);
    }
    if let Some(division_id) = params.division_id {
        query.push(" AND s.division_id = ");
        query.push_bind(division_id);
    }
    if let Some(cost_centre_id) = params.cost_centre_id {
        query.push(
            " AND EXISTS (
                SELECT 1 FROM cost_centre_assignments cca
                WHERE cca.organisation_id = s.organisation_id
                  AND cca.cost_centre_id = ",
        );
        query.push_bind(cost_centre_id);
        query.push(
            " AND ((cca.entity_type = 'shift' AND cca.entity_id = s.id)
                    OR (cca.entity_type = 'division' AND cca.entity_id = s.division_id)))",
        );
    }
    if let Some(worker_id) = params.worker_id {
        query.push(
            " AND EXISTS (
                SELECT 1 FROM worker_assignments wa
                WHERE wa.organisation_id = s.organisation_id
                  AND wa.shift_id = s.id AND wa.contact_id = ",
        );
        query.push_bind(worker_id);
        query.push(")");
    }
    if let Some(client_id) = params.client_id {
        query.push(
            " AND EXISTS (
                SELECT 1 FROM client_assignments ca
                WHERE ca.organisation_id = s.organisation_id
                  AND ca.shift_id = s.id AND ca.contact_id = ",
        );
        query.push_bind(client_id);
        query.push(")");
    }

    query.push(" ORDER BY s.starts_at, s.id LIMIT ");
    query.push_bind(limit);

    let rows = query
        .build_query_as::<ShiftRow>()
        .fetch_all(&state.db)
        .await?;
    Ok(Json(ShiftList {
        items: rows.into_iter().map(shift_dto).collect(),
    }))
}

#[utoipa::path(
    post,
    path = "/api/v1/shifts",
    tag = "rostering",
    security(("bearer_auth" = [])),
    request_body = ShiftWrite,
    responses(
        (status = 201, description = "Shift created", body = ShiftDto),
        (status = 401, description = "Missing or invalid access token", body = ErrorBody),
        (status = 403, description = "Missing rosters.write capability", body = ErrorBody),
        (status = 422, description = "Validation failed", body = ErrorBody)
    )
)]
pub async fn create_shift(
    State(state): State<AppState>,
    auth: AuthContext,
    Json(payload): Json<ShiftWrite>,
) -> Result<(StatusCode, Json<ShiftDto>), AppError> {
    auth.require(Capability::RostersWrite)?;
    let kind = validate_kind(payload.kind.as_deref())?;
    let status = validate_status(payload.status.as_deref())?;
    validate_times(payload.starts_at, payload.ends_at)?;

    if let Some(division_id) = payload.division_id {
        ensure_division(&state, auth.organisation_id, division_id).await?;
    }
    if let Some(service_id) = payload.service_id {
        ensure_service(&state, auth.organisation_id, service_id).await?;
    }
    if let Some(site_id) = payload.site_id {
        ensure_site(&state, auth.organisation_id, site_id).await?;
    }

    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO shifts
            (id, organisation_id, division_id, service_id, site_id, title, kind,
             starts_at, ends_at, location, status, notes)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)",
    )
    .bind(id)
    .bind(auth.organisation_id)
    .bind(payload.division_id)
    .bind(payload.service_id)
    .bind(payload.site_id)
    .bind(
        payload
            .title
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty()),
    )
    .bind(kind)
    .bind(payload.starts_at)
    .bind(payload.ends_at)
    .bind(
        payload
            .location
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty()),
    )
    .bind(status)
    .bind(
        payload
            .notes
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty()),
    )
    .execute(&state.db)
    .await?;

    Ok((
        StatusCode::CREATED,
        Json(load_shift(&state, auth.organisation_id, id).await?),
    ))
}

#[utoipa::path(
    get,
    path = "/api/v1/shifts/{id}",
    tag = "rostering",
    security(("bearer_auth" = [])),
    params(("id" = Uuid, Path, description = "Shift id")),
    responses(
        (status = 200, description = "Shift with assignments", body = ShiftDetail),
        (status = 401, description = "Missing or invalid access token", body = ErrorBody),
        (status = 403, description = "Missing rosters.read capability", body = ErrorBody),
        (status = 404, description = "Shift not found", body = ErrorBody)
    )
)]
pub async fn get_shift(
    State(state): State<AppState>,
    auth: AuthContext,
    Path(id): Path<Uuid>,
) -> Result<Json<ShiftDetail>, AppError> {
    auth.require(Capability::RostersRead)?;
    let shift = load_shift(&state, auth.organisation_id, id).await?;
    let assignments = load_assignments(&state, auth.organisation_id, id).await?;
    Ok(Json(ShiftDetail {
        shift,
        assignments: assignments.into_iter().map(assignment_dto).collect(),
    }))
}

#[utoipa::path(
    patch,
    path = "/api/v1/shifts/{id}",
    tag = "rostering",
    security(("bearer_auth" = [])),
    params(("id" = Uuid, Path, description = "Shift id")),
    request_body = ShiftUpdate,
    responses(
        (status = 200, description = "Shift updated", body = ShiftDto),
        (status = 401, description = "Missing or invalid access token", body = ErrorBody),
        (status = 403, description = "Missing rosters.write capability", body = ErrorBody),
        (status = 404, description = "Shift not found", body = ErrorBody),
        (status = 422, description = "Validation failed", body = ErrorBody)
    )
)]
pub async fn update_shift(
    State(state): State<AppState>,
    auth: AuthContext,
    Path(id): Path<Uuid>,
    Json(payload): Json<ShiftUpdate>,
) -> Result<Json<ShiftDto>, AppError> {
    auth.require(Capability::RostersWrite)?;
    let existing = load_shift_row(&state, auth.organisation_id, id).await?;

    let kind = match payload.kind.as_deref() {
        Some(value) => validate_kind(Some(value))?.to_owned(),
        None => existing.row.kind.clone(),
    };
    let status = match payload.status.as_deref() {
        Some(value) => validate_status(Some(value))?.to_owned(),
        None => existing.row.status.clone(),
    };
    let starts_at = payload.starts_at.unwrap_or(existing.row.starts_at);
    let ends_at = payload.ends_at.unwrap_or(existing.row.ends_at);
    validate_times(starts_at, ends_at)?;

    if let Some(division_id) = payload.division_id {
        ensure_division(&state, auth.organisation_id, division_id).await?;
    }
    if let Some(service_id) = payload.service_id {
        ensure_service(&state, auth.organisation_id, service_id).await?;
    }
    if let Some(site_id) = payload.site_id {
        ensure_site(&state, auth.organisation_id, site_id).await?;
    }

    sqlx::query(
        "UPDATE shifts SET
            division_id = $1, service_id = $2, site_id = $3, title = $4, kind = $5,
            starts_at = $6, ends_at = $7, location = $8, status = $9, notes = $10
         WHERE organisation_id = $11 AND id = $12",
    )
    .bind(payload.division_id.or(existing.row.division_id))
    .bind(payload.service_id.or(existing.row.service_id))
    .bind(payload.site_id.or(existing.row.site_id))
    .bind(payload.title.or(existing.row.title))
    .bind(kind)
    .bind(starts_at)
    .bind(ends_at)
    .bind(payload.location.or(existing.row.location))
    .bind(status)
    .bind(payload.notes.or(existing.row.notes))
    .bind(auth.organisation_id)
    .bind(id)
    .execute(&state.db)
    .await?;

    Ok(Json(load_shift(&state, auth.organisation_id, id).await?))
}

#[utoipa::path(
    post,
    path = "/api/v1/shifts/batch",
    tag = "rostering",
    security(("bearer_auth" = [])),
    request_body = BatchShiftWrite,
    responses(
        (status = 201, description = "Shifts created atomically", body = ShiftList),
        (status = 401, description = "Missing or invalid access token", body = ErrorBody),
        (status = 403, description = "Missing rosters.write capability", body = ErrorBody),
        (status = 422, description = "Validation failed", body = ErrorBody)
    )
)]
pub async fn create_shifts(
    State(state): State<AppState>,
    auth: AuthContext,
    Json(payload): Json<BatchShiftWrite>,
) -> Result<(StatusCode, Json<ShiftList>), AppError> {
    auth.require(Capability::RostersWrite)?;
    if payload.shifts.is_empty() || payload.shifts.len() > 100 {
        return Err(AppError::validation(
            "a batch must contain between 1 and 100 shifts",
        ));
    }
    let recurrence = payload
        .recurrence
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty() && *value != "none")
        .map(str::to_owned);
    if recurrence
        .as_deref()
        .is_some_and(|value| value.chars().count() > 64)
    {
        return Err(AppError::validation(
            "recurrence must be 64 characters or fewer",
        ));
    }

    let mut validated = Vec::with_capacity(payload.shifts.len());
    for shift in &payload.shifts {
        let kind = validate_kind(shift.kind.as_deref())?;
        let status = validate_status(shift.status.as_deref())?;
        validate_times(shift.starts_at, shift.ends_at)?;
        if let Some(division_id) = shift.division_id {
            ensure_division(&state, auth.organisation_id, division_id).await?;
        }
        if let Some(service_id) = shift.service_id {
            ensure_service(&state, auth.organisation_id, service_id).await?;
        }
        if let Some(site_id) = shift.site_id {
            ensure_site(&state, auth.organisation_id, site_id).await?;
        }
        validated.push((kind, status));
    }

    let series_id = (payload.shifts.len() > 1).then(Uuid::now_v7);
    let mut tx = state.db.begin().await?;
    let mut ids = Vec::with_capacity(payload.shifts.len());
    for (shift, (kind, status)) in payload.shifts.iter().zip(validated) {
        let id = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO shifts
                (id, organisation_id, division_id, service_id, site_id, title, kind,
                 starts_at, ends_at, location, status, notes, series_id, recurrence)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14)",
        )
        .bind(id)
        .bind(auth.organisation_id)
        .bind(shift.division_id)
        .bind(shift.service_id)
        .bind(shift.site_id)
        .bind(
            shift
                .title
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty()),
        )
        .bind(kind)
        .bind(shift.starts_at)
        .bind(shift.ends_at)
        .bind(
            shift
                .location
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty()),
        )
        .bind(status)
        .bind(
            shift
                .notes
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty()),
        )
        .bind(series_id)
        .bind(&recurrence)
        .execute(&mut *tx)
        .await?;
        ids.push(id);
    }
    tx.commit().await?;

    let mut items = Vec::with_capacity(ids.len());
    for id in ids {
        items.push(load_shift(&state, auth.organisation_id, id).await?);
    }
    Ok((StatusCode::CREATED, Json(ShiftList { items })))
}

#[utoipa::path(
    post,
    path = "/api/v1/shifts/{id}/copy",
    tag = "rostering",
    security(("bearer_auth" = [])),
    params(("id" = Uuid, Path, description = "Source shift id")),
    request_body = CopyShiftWrite,
    responses(
        (status = 201, description = "Shift copied", body = ShiftDetail),
        (status = 401, description = "Missing or invalid access token", body = ErrorBody),
        (status = 403, description = "Missing rosters.write capability", body = ErrorBody),
        (status = 404, description = "Shift not found", body = ErrorBody),
        (status = 422, description = "Validation failed", body = ErrorBody)
    )
)]
pub async fn copy_shift(
    State(state): State<AppState>,
    auth: AuthContext,
    Path(id): Path<Uuid>,
    Json(payload): Json<CopyShiftWrite>,
) -> Result<(StatusCode, Json<ShiftDetail>), AppError> {
    auth.require(Capability::RostersWrite)?;
    let existing = load_shift_row(&state, auth.organisation_id, id).await?.row;
    let duration = existing.ends_at - existing.starts_at;
    let starts_at = payload.starts_at.unwrap_or(existing.starts_at);
    let ends_at = payload.ends_at.unwrap_or(starts_at + duration);
    validate_times(starts_at, ends_at)?;

    let new_id = Uuid::now_v7();
    let mut tx = state.db.begin().await?;
    sqlx::query(
        "INSERT INTO shifts
            (id, organisation_id, division_id, service_id, site_id, title, kind,
             starts_at, ends_at, location, status, notes)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,'draft',$11)",
    )
    .bind(new_id)
    .bind(auth.organisation_id)
    .bind(existing.division_id)
    .bind(existing.service_id)
    .bind(existing.site_id)
    .bind(&existing.title)
    .bind(&existing.kind)
    .bind(starts_at)
    .bind(ends_at)
    .bind(&existing.location)
    .bind(&existing.notes)
    .execute(&mut *tx)
    .await?;

    if payload.include_assignments {
        for (table, role) in [
            ("client_assignments", "client"),
            ("worker_assignments", "worker"),
        ] {
            let query = format!(
                "INSERT INTO {table}
                    (id, organisation_id, shift_id, session_id, contact_id, status, notes)
                 SELECT gen_random_uuid(), organisation_id, $1, NULL, contact_id,
                        'assigned', notes
                 FROM {table}
                 WHERE organisation_id=$2 AND shift_id=$3"
            );
            sqlx::query(&query)
                .bind(new_id)
                .bind(auth.organisation_id)
                .bind(id)
                .execute(&mut *tx)
                .await?;
            let _ = role;
        }
    }
    tx.commit().await?;

    let shift = load_shift(&state, auth.organisation_id, new_id).await?;
    let assignments = load_assignments(&state, auth.organisation_id, new_id).await?;
    Ok((
        StatusCode::CREATED,
        Json(ShiftDetail {
            shift,
            assignments: assignments.into_iter().map(assignment_dto).collect(),
        }),
    ))
}

#[utoipa::path(
    delete,
    path = "/api/v1/shifts/{id}",
    tag = "rostering",
    security(("bearer_auth" = [])),
    params(("id" = Uuid, Path, description = "Shift id")),
    responses(
        (status = 204, description = "Shift deleted"),
        (status = 401, description = "Missing or invalid access token", body = ErrorBody),
        (status = 403, description = "Missing rosters.write capability", body = ErrorBody),
        (status = 404, description = "Shift not found", body = ErrorBody)
    )
)]
pub async fn delete_shift(
    State(state): State<AppState>,
    auth: AuthContext,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    auth.require(Capability::RostersWrite)?;
    let removed = sqlx::query("DELETE FROM shifts WHERE organisation_id=$1 AND id=$2")
        .bind(auth.organisation_id)
        .bind(id)
        .execute(&state.db)
        .await?
        .rows_affected();
    if removed == 0 {
        return Err(AppError::not_found("shift not found"));
    }
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// Assignments
// ---------------------------------------------------------------------------

#[utoipa::path(
    post,
    path = "/api/v1/shifts/{id}/assignments",
    tag = "rostering",
    security(("bearer_auth" = [])),
    params(("id" = Uuid, Path, description = "Shift id")),
    request_body = AssignmentWrite,
    responses(
        (status = 201, description = "Assignment created", body = AssignmentDto),
        (status = 401, description = "Missing or invalid access token", body = ErrorBody),
        (status = 403, description = "Missing rosters.write capability", body = ErrorBody),
        (status = 409, description = "Contact already assigned in this scope", body = ErrorBody),
        (status = 422, description = "Validation failed", body = ErrorBody)
    )
)]
pub async fn create_assignment(
    State(state): State<AppState>,
    auth: AuthContext,
    Path(shift_id): Path<Uuid>,
    Json(payload): Json<AssignmentWrite>,
) -> Result<(StatusCode, Json<AssignmentDto>), AppError> {
    auth.require(Capability::RostersWrite)?;
    load_shift_row(&state, auth.organisation_id, shift_id).await?;
    let status = validate_assignment_status(payload.status.as_deref())?;
    let role = payload.role.trim().to_ascii_lowercase();
    if role != "worker" && role != "client" {
        return Err(AppError::validation("role must be 'worker' or 'client'"));
    }
    ensure_contact(&state, auth.organisation_id, payload.contact_id).await?;
    if let Some(session_id) = payload.session_id {
        ensure_session(&state, auth.organisation_id, shift_id, session_id).await?;
    }

    let duplicate: bool = if role == "worker" {
        sqlx::query_scalar(
            "SELECT EXISTS(
                SELECT 1 FROM worker_assignments
                WHERE organisation_id=$1 AND shift_id=$2 AND contact_id=$3
                  AND session_id IS NOT DISTINCT FROM $4
            )",
        )
        .bind(auth.organisation_id)
        .bind(shift_id)
        .bind(payload.contact_id)
        .bind(payload.session_id)
        .fetch_one(&state.db)
        .await?
    } else {
        sqlx::query_scalar(
            "SELECT EXISTS(
                SELECT 1 FROM client_assignments
                WHERE organisation_id=$1 AND shift_id=$2 AND contact_id=$3
                  AND session_id IS NOT DISTINCT FROM $4
            )",
        )
        .bind(auth.organisation_id)
        .bind(shift_id)
        .bind(payload.contact_id)
        .bind(payload.session_id)
        .fetch_one(&state.db)
        .await?
    };
    if duplicate {
        return Err(AppError::conflict(
            "this contact is already assigned in that scope",
        ));
    }

    let id = Uuid::now_v7();
    if role == "worker" {
        sqlx::query(
            "INSERT INTO worker_assignments
                (id, organisation_id, shift_id, session_id, contact_id, status, notes)
             VALUES ($1,$2,$3,$4,$5,$6,$7)",
        )
        .bind(id)
        .bind(auth.organisation_id)
        .bind(shift_id)
        .bind(payload.session_id)
        .bind(payload.contact_id)
        .bind(status)
        .bind(
            payload
                .notes
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty()),
        )
        .execute(&state.db)
        .await?;
    } else {
        sqlx::query(
            "INSERT INTO client_assignments
                (id, organisation_id, shift_id, session_id, contact_id, status, notes)
             VALUES ($1,$2,$3,$4,$5,$6,$7)",
        )
        .bind(id)
        .bind(auth.organisation_id)
        .bind(shift_id)
        .bind(payload.session_id)
        .bind(payload.contact_id)
        .bind(status)
        .bind(
            payload
                .notes
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty()),
        )
        .execute(&state.db)
        .await?;
    }

    let assignment = load_assignment(&state, auth.organisation_id, id)
        .await?
        .ok_or_else(|| AppError::not_found("assignment not found after creation"))?;
    Ok((StatusCode::CREATED, Json(assignment_dto(assignment))))
}

#[utoipa::path(
    patch,
    path = "/api/v1/assignments/{id}",
    tag = "rostering",
    security(("bearer_auth" = [])),
    params(("id" = Uuid, Path, description = "Assignment id")),
    request_body = AssignmentUpdate,
    responses(
        (status = 200, description = "Assignment updated", body = AssignmentDto),
        (status = 401, description = "Missing or invalid access token", body = ErrorBody),
        (status = 403, description = "Missing rosters.write capability", body = ErrorBody),
        (status = 404, description = "Assignment not found", body = ErrorBody),
        (status = 422, description = "Validation failed", body = ErrorBody)
    )
)]
pub async fn update_assignment(
    State(state): State<AppState>,
    auth: AuthContext,
    Path(id): Path<Uuid>,
    Json(payload): Json<AssignmentUpdate>,
) -> Result<Json<AssignmentDto>, AppError> {
    auth.require(Capability::RostersWrite)?;
    let status = payload
        .status
        .as_deref()
        .map(|value| validate_assignment_status(Some(value)))
        .transpose()?;
    let outcome = payload
        .outcome
        .as_deref()
        .map(validate_outcome)
        .transpose()?;
    if let Some(rating) = payload.outcome_rating {
        if !(1..=5).contains(&rating) {
            return Err(AppError::validation(
                "outcome_rating must be between 1 and 5",
            ));
        }
    }

    let role: Option<String> = sqlx::query_scalar(
        "SELECT 'worker'::text FROM worker_assignments WHERE organisation_id=$1 AND id=$2",
    )
    .bind(auth.organisation_id)
    .bind(id)
    .fetch_optional(&state.db)
    .await?;

    if role.as_deref() == Some("worker") {
        sqlx::query(
            "UPDATE worker_assignments SET
                status = COALESCE($1, status),
                outcome = COALESCE($2, outcome),
                outcome_rating = COALESCE($3, outcome_rating),
                notes = COALESCE($4, notes)
             WHERE organisation_id=$5 AND id=$6",
        )
        .bind(status)
        .bind(outcome)
        .bind(payload.outcome_rating)
        .bind(
            payload
                .notes
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty()),
        )
        .bind(auth.organisation_id)
        .bind(id)
        .execute(&state.db)
        .await?;
    } else {
        let is_client: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM client_assignments WHERE organisation_id=$1 AND id=$2)",
        )
        .bind(auth.organisation_id)
        .bind(id)
        .fetch_one(&state.db)
        .await?;
        if !is_client {
            return Err(AppError::not_found("assignment not found"));
        }
        if outcome.is_some() || payload.outcome_rating.is_some() {
            return Err(AppError::validation(
                "outcome and outcome_rating only apply to worker assignments",
            ));
        }
        sqlx::query(
            "UPDATE client_assignments SET
                status = COALESCE($1, status),
                notes = COALESCE($2, notes)
             WHERE organisation_id=$3 AND id=$4",
        )
        .bind(status)
        .bind(
            payload
                .notes
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty()),
        )
        .bind(auth.organisation_id)
        .bind(id)
        .execute(&state.db)
        .await?;
    }

    let assignment = load_assignment(&state, auth.organisation_id, id)
        .await?
        .ok_or_else(|| AppError::not_found("assignment not found"))?;
    Ok(Json(assignment_dto(assignment)))
}

#[utoipa::path(
    delete,
    path = "/api/v1/assignments/{id}",
    tag = "rostering",
    security(("bearer_auth" = [])),
    params(("id" = Uuid, Path, description = "Assignment id")),
    responses(
        (status = 204, description = "Assignment deleted"),
        (status = 401, description = "Missing or invalid access token", body = ErrorBody),
        (status = 403, description = "Missing rosters.write capability", body = ErrorBody),
        (status = 404, description = "Assignment not found", body = ErrorBody)
    )
)]
pub async fn delete_assignment(
    State(state): State<AppState>,
    auth: AuthContext,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    auth.require(Capability::RostersWrite)?;
    let removed = sqlx::query("DELETE FROM worker_assignments WHERE organisation_id=$1 AND id=$2")
        .bind(auth.organisation_id)
        .bind(id)
        .execute(&state.db)
        .await?
        .rows_affected();

    let removed = if removed > 0 {
        removed
    } else {
        sqlx::query("DELETE FROM client_assignments WHERE organisation_id=$1 AND id=$2")
            .bind(auth.organisation_id)
            .bind(id)
            .execute(&state.db)
            .await?
            .rows_affected()
    };

    if removed == 0 {
        return Err(AppError::not_found("assignment not found"));
    }
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// Roster constraints
// ---------------------------------------------------------------------------

#[utoipa::path(
    get,
    path = "/api/v1/roster-constraints",
    tag = "rostering",
    security(("bearer_auth" = [])),
    params(ConstraintListParams),
    responses(
        (status = 200, description = "Roster constraints", body = Vec<RosterConstraintDto>),
        (status = 401, description = "Missing or invalid access token", body = ErrorBody),
        (status = 403, description = "Missing rosters.read capability", body = ErrorBody)
    )
)]
pub async fn list_constraints(
    State(state): State<AppState>,
    auth: AuthContext,
    Query(params): Query<ConstraintListParams>,
) -> Result<Json<Vec<RosterConstraintDto>>, AppError> {
    auth.require(Capability::RostersRead)?;
    let rows = load_constraints(&state, auth.organisation_id, &params).await?;
    Ok(Json(rows.into_iter().map(constraint_dto).collect()))
}

#[utoipa::path(
    post,
    path = "/api/v1/roster-constraints",
    tag = "rostering",
    security(("bearer_auth" = [])),
    request_body = RosterConstraintWrite,
    responses(
        (status = 201, description = "Constraint created", body = RosterConstraintDto),
        (status = 401, description = "Missing or invalid access token", body = ErrorBody),
        (status = 403, description = "Missing rosters.write capability", body = ErrorBody),
        (status = 409, description = "Identical constraint already exists", body = ErrorBody),
        (status = 422, description = "Validation failed", body = ErrorBody)
    )
)]
pub async fn create_constraint(
    State(state): State<AppState>,
    auth: AuthContext,
    Json(payload): Json<RosterConstraintWrite>,
) -> Result<(StatusCode, Json<RosterConstraintDto>), AppError> {
    auth.require(Capability::RostersWrite)?;
    let kind = ConstraintKind::parse(&payload.kind)
        .ok_or_else(|| AppError::validation("kind must be 'preferred', 'avoid' or 'blocked'"))?;
    let target_kind = payload.target_kind.trim().to_ascii_lowercase();
    if !matches!(
        target_kind.as_str(),
        "contact" | "site" | "service" | "division"
    ) {
        return Err(AppError::validation(
            "target_kind must be 'contact', 'site', 'service' or 'division'",
        ));
    }
    let strength = payload.strength.unwrap_or(3);
    if !(1..=5).contains(&strength) {
        return Err(AppError::validation("strength must be between 1 and 5"));
    }
    validate_dates(payload.start_date, payload.end_date)?;
    ensure_contact(&state, auth.organisation_id, payload.worker_contact_id).await?;

    if target_kind == "contact" && payload.target_id == payload.worker_contact_id {
        return Err(AppError::validation("a worker cannot constrain themselves"));
    }
    match target_kind.as_str() {
        "contact" => ensure_contact(&state, auth.organisation_id, payload.target_id).await?,
        "site" => ensure_site(&state, auth.organisation_id, payload.target_id).await?,
        "service" => ensure_service(&state, auth.organisation_id, payload.target_id).await?,
        "division" => ensure_division(&state, auth.organisation_id, payload.target_id).await?,
        _ => {}
    }

    let contact_target = (target_kind == "contact").then_some(payload.target_id);
    let site_target = (target_kind == "site").then_some(payload.target_id);
    let service_target = (target_kind == "service").then_some(payload.target_id);
    let division_target = (target_kind == "division").then_some(payload.target_id);

    let duplicate: bool = sqlx::query_scalar(
        "SELECT EXISTS(
            SELECT 1 FROM roster_constraints
            WHERE organisation_id=$1 AND worker_contact_id=$2
              AND target_contact_id IS NOT DISTINCT FROM $3
              AND target_site_id IS NOT DISTINCT FROM $4
              AND target_service_id IS NOT DISTINCT FROM $5
              AND target_division_id IS NOT DISTINCT FROM $6
              AND kind=$7
        )",
    )
    .bind(auth.organisation_id)
    .bind(payload.worker_contact_id)
    .bind(contact_target)
    .bind(site_target)
    .bind(service_target)
    .bind(division_target)
    .bind(kind.as_str())
    .fetch_one(&state.db)
    .await?;
    if duplicate {
        return Err(AppError::conflict(
            "an identical roster constraint already exists",
        ));
    }

    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO roster_constraints
            (id, organisation_id, worker_contact_id, target_contact_id, target_site_id,
             target_service_id, target_division_id, kind, strength, reason, source,
             is_active, start_date, end_date)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14)",
    )
    .bind(id)
    .bind(auth.organisation_id)
    .bind(payload.worker_contact_id)
    .bind(contact_target)
    .bind(site_target)
    .bind(service_target)
    .bind(division_target)
    .bind(kind.as_str())
    .bind(strength)
    .bind(
        payload
            .reason
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty()),
    )
    .bind(
        payload
            .source
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("manual"),
    )
    .bind(payload.is_active.unwrap_or(true))
    .bind(payload.start_date)
    .bind(payload.end_date)
    .execute(&state.db)
    .await?;

    let row = load_constraint(&state, auth.organisation_id, id)
        .await?
        .ok_or_else(|| AppError::not_found("constraint not found after creation"))?;
    Ok((StatusCode::CREATED, Json(constraint_dto(row))))
}

#[utoipa::path(
    delete,
    path = "/api/v1/roster-constraints/{id}",
    tag = "rostering",
    security(("bearer_auth" = [])),
    params(("id" = Uuid, Path, description = "Constraint id")),
    responses(
        (status = 204, description = "Constraint deleted"),
        (status = 401, description = "Missing or invalid access token", body = ErrorBody),
        (status = 403, description = "Missing rosters.write capability", body = ErrorBody),
        (status = 404, description = "Constraint not found", body = ErrorBody)
    )
)]
pub async fn delete_constraint(
    State(state): State<AppState>,
    auth: AuthContext,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    auth.require(Capability::RostersWrite)?;
    let removed = sqlx::query("DELETE FROM roster_constraints WHERE organisation_id=$1 AND id=$2")
        .bind(auth.organisation_id)
        .bind(id)
        .execute(&state.db)
        .await?
        .rows_affected();
    if removed == 0 {
        return Err(AppError::not_found("roster constraint not found"));
    }
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// Suitability
// ---------------------------------------------------------------------------

#[utoipa::path(
    get,
    path = "/api/v1/shifts/{id}/suitability",
    tag = "rostering",
    security(("bearer_auth" = [])),
    params(("id" = Uuid, Path, description = "Shift id")),
    responses(
        (status = 200, description = "Ranked worker candidates with explanations", body = SuitabilityResponse),
        (status = 401, description = "Missing or invalid access token", body = ErrorBody),
        (status = 403, description = "Missing rosters.read capability", body = ErrorBody),
        (status = 404, description = "Shift not found", body = ErrorBody)
    )
)]
pub async fn suitability(
    State(state): State<AppState>,
    auth: AuthContext,
    Path(shift_id): Path<Uuid>,
) -> Result<Json<SuitabilityResponse>, AppError> {
    auth.require(Capability::RostersRead)?;
    let shift = load_shift_row(&state, auth.organisation_id, shift_id).await?;
    let assignments = load_assignments(&state, auth.organisation_id, shift_id).await?;

    let clients = assignments
        .iter()
        .filter(|assignment| assignment.role == "client")
        .map(|assignment| PartyRef {
            id: assignment.contact_id,
            display_name: assignment.contact_name.clone(),
        })
        .collect::<Vec<_>>();
    let assigned_workers = assignments
        .iter()
        .filter(|assignment| assignment.role == "worker")
        .map(|assignment| PartyRef {
            id: assignment.contact_id,
            display_name: assignment.contact_name.clone(),
        })
        .collect::<Vec<_>>();

    let client_ids = clients
        .iter()
        .map(|client| client.id)
        .collect::<HashSet<_>>();
    let assigned_worker_ids = assigned_workers
        .iter()
        .map(|worker| worker.id)
        .collect::<HashSet<_>>();

    let candidates = sqlx::query_as::<_, ContactRow>(
        "SELECT c.id, c.display_name
         FROM contacts c
         WHERE c.organisation_id=$1 AND c.is_active
           AND (c.details->>'contact_role' IN ('worker', 'dual')
                OR EXISTS (
                    SELECT 1 FROM worker_assignments wa
                    WHERE wa.organisation_id=c.organisation_id AND wa.contact_id=c.id
                ))
         ORDER BY c.display_name",
    )
    .bind(auth.organisation_id)
    .fetch_all(&state.db)
    .await?;
    let candidate_ids = candidates
        .iter()
        .map(|candidate| candidate.id)
        .filter(|id| !assigned_worker_ids.contains(id))
        .collect::<Vec<_>>();

    let mut client_history: HashMap<(Uuid, Uuid), HistorySignal> = HashMap::new();
    let mut coworker_history: HashMap<(Uuid, Uuid), HistorySignal> = HashMap::new();
    let mut context_history: HashMap<(Uuid, String, Uuid), HistorySignal> = HashMap::new();

    if !candidate_ids.is_empty() && !client_ids.is_empty() {
        let client_id_list = client_ids.iter().copied().collect::<Vec<_>>();
        let rows = sqlx::query_as::<_, WorkerClientHistoryRow>(
            "SELECT worker_contact_id, client_contact_id, attended_shifts, no_show_shifts,
                    cancelled_shifts, last_worked_at, average_rating,
                    positive_outcomes, negative_outcomes
             FROM worker_client_history
             WHERE organisation_id=$1
               AND worker_contact_id = ANY($2)
               AND client_contact_id = ANY($3)",
        )
        .bind(auth.organisation_id)
        .bind(candidate_ids.as_slice())
        .bind(client_id_list.as_slice())
        .fetch_all(&state.db)
        .await?;
        for row in rows {
            client_history.insert(
                (row.worker_contact_id, row.client_contact_id),
                HistorySignal {
                    attended_shifts: row.attended_shifts,
                    no_show_shifts: row.no_show_shifts,
                    cancelled_shifts: row.cancelled_shifts,
                    last_worked_at: row.last_worked_at,
                    average_rating: row.average_rating,
                    positive_outcomes: row.positive_outcomes,
                    negative_outcomes: row.negative_outcomes,
                },
            );
        }
    }

    if !candidate_ids.is_empty() && !assigned_worker_ids.is_empty() {
        let worker_ids = assigned_worker_ids.iter().copied().collect::<Vec<_>>();
        let rows = sqlx::query_as::<_, WorkerWorkerHistoryRow>(
            "SELECT worker_a_id, worker_b_id, attended_shifts, no_show_shifts, last_worked_at
             FROM worker_worker_history
             WHERE organisation_id=$1
               AND (worker_a_id = ANY($2) OR worker_b_id = ANY($2))
               AND (worker_a_id = ANY($3) OR worker_b_id = ANY($3))",
        )
        .bind(auth.organisation_id)
        .bind(candidate_ids.as_slice())
        .bind(worker_ids.as_slice())
        .fetch_all(&state.db)
        .await?;
        for row in rows {
            let signal = HistorySignal {
                attended_shifts: row.attended_shifts,
                no_show_shifts: row.no_show_shifts,
                last_worked_at: row.last_worked_at,
                ..HistorySignal::default()
            };
            coworker_history.insert((row.worker_a_id, row.worker_b_id), signal.clone());
            coworker_history.insert((row.worker_b_id, row.worker_a_id), signal);
        }
    }

    if !candidate_ids.is_empty() {
        let rows = sqlx::query_as::<_, WorkerContextHistoryRow>(
            "SELECT worker_contact_id, context_kind, context_id, attended_shifts, last_worked_at
             FROM worker_context_history
             WHERE organisation_id=$1 AND worker_contact_id = ANY($2)",
        )
        .bind(auth.organisation_id)
        .bind(candidate_ids.as_slice())
        .fetch_all(&state.db)
        .await?;
        for row in rows {
            context_history.insert(
                (row.worker_contact_id, row.context_kind, row.context_id),
                HistorySignal {
                    attended_shifts: row.attended_shifts,
                    last_worked_at: row.last_worked_at,
                    ..HistorySignal::default()
                },
            );
        }
    }

    let constraints = load_constraints(
        &state,
        auth.organisation_id,
        &ConstraintListParams {
            worker_id: None,
            target_contact_id: None,
            site_id: None,
            service_id: None,
            division_id: None,
            include_inactive: Some(false),
        },
    )
    .await?;
    let today = Utc::now().date_naive();
    let constraints = constraints
        .into_iter()
        .filter(|constraint| {
            constraint.start_date.is_none_or(|start| start <= today)
                && constraint.end_date.is_none_or(|end| end >= today)
        })
        .collect::<Vec<_>>();

    let mut ranked = Vec::new();
    let mut blocked = Vec::new();

    for candidate in candidates {
        if assigned_worker_ids.contains(&candidate.id) {
            continue;
        }

        let client_evidence = clients
            .iter()
            .filter_map(|client| {
                client_history
                    .get(&(candidate.id, client.id))
                    .filter(|signal| signal.has_evidence())
                    .map(|signal| (client.display_name.clone(), signal.clone()))
            })
            .collect::<Vec<_>>();

        let coworker_evidence = assigned_workers
            .iter()
            .filter_map(|worker| {
                coworker_history
                    .get(&(candidate.id, worker.id))
                    .filter(|signal| signal.has_evidence())
                    .map(|signal| (worker.display_name.clone(), signal.clone()))
            })
            .collect::<Vec<_>>();

        let mut context_evidence = Vec::new();
        if let Some(site) = &shift.site {
            if let Some(signal) = context_history.get(&(candidate.id, "site".to_owned(), site.id)) {
                context_evidence.push(ContextEvidence {
                    label: format!("site {}", site.name),
                    signal: signal.clone(),
                    multiplier: 1.5,
                });
            }
        }
        if let Some(service) = &shift.service {
            if let Some(signal) =
                context_history.get(&(candidate.id, "service".to_owned(), service.id))
            {
                context_evidence.push(ContextEvidence {
                    label: format!("service {}", service.name),
                    signal: signal.clone(),
                    multiplier: 1.0,
                });
            }
        }
        if let Some(division) = &shift.division {
            if let Some(signal) =
                context_history.get(&(candidate.id, "division".to_owned(), division.id))
            {
                context_evidence.push(ContextEvidence {
                    label: format!("division {}", division.name),
                    signal: signal.clone(),
                    multiplier: 0.5,
                });
            }
        }

        let constraint_evidence = constraints
            .iter()
            .filter_map(|constraint| {
                constraint_evidence_for(
                    constraint,
                    candidate.id,
                    &shift,
                    &client_ids,
                    &assigned_worker_ids,
                )
            })
            .collect::<Vec<_>>();

        let result = score_candidate(ScoreInput {
            client_history: &client_evidence,
            coworker_history: &coworker_evidence,
            context_history: &context_evidence,
            constraints: &constraint_evidence,
        });

        let candidate = SuitabilityCandidate {
            contact_id: candidate.id,
            display_name: candidate.display_name,
            score: result.score,
            reasons: result.reasons,
            warnings: result.warnings,
        };

        if result.blocked {
            blocked.push(candidate);
        } else {
            ranked.push(candidate);
        }
    }

    ranked.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.display_name.cmp(&b.display_name))
    });
    blocked.sort_by(|a, b| a.display_name.cmp(&b.display_name));

    Ok(Json(SuitabilityResponse {
        shift_id,
        site: shift.site,
        service: shift.service,
        division: shift.division,
        clients,
        assigned_workers,
        candidates: ranked,
        blocked,
    }))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

struct ShiftContext {
    row: ShiftRow,
    site: Option<ContextRef>,
    service: Option<ContextRef>,
    division: Option<ContextRef>,
}

async fn load_shift(
    state: &AppState,
    organisation_id: Uuid,
    id: Uuid,
) -> Result<ShiftDto, AppError> {
    Ok(shift_dto(
        load_shift_row(state, organisation_id, id).await?.row,
    ))
}

async fn load_shift_row(
    state: &AppState,
    organisation_id: Uuid,
    id: Uuid,
) -> Result<ShiftContext, AppError> {
    let row = sqlx::query_as::<_, ShiftRow>(&format!(
        "{SHIFT_SELECT} WHERE s.organisation_id = $1 AND s.id = $2"
    ))
    .bind(organisation_id)
    .bind(id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::not_found("shift not found"))?;

    let site = row
        .site_id
        .zip(row.site_name.clone())
        .map(|(id, name)| ContextRef { id, name });
    let service = row
        .service_id
        .zip(row.service_name.clone())
        .map(|(id, name)| ContextRef { id, name });
    let division = row
        .division_id
        .zip(row.division_name.clone())
        .map(|(id, name)| ContextRef { id, name });

    Ok(ShiftContext {
        row,
        site,
        service,
        division,
    })
}

async fn load_assignments(
    state: &AppState,
    organisation_id: Uuid,
    shift_id: Uuid,
) -> Result<Vec<AssignmentRow>, AppError> {
    Ok(sqlx::query_as::<_, AssignmentRow>(ASSIGNMENT_SELECT)
        .bind(organisation_id)
        .bind(shift_id)
        .fetch_all(&state.db)
        .await?)
}

async fn load_assignment(
    state: &AppState,
    organisation_id: Uuid,
    id: Uuid,
) -> Result<Option<AssignmentRow>, AppError> {
    let row = sqlx::query_as::<_, AssignmentRow>(
        "SELECT * FROM (
            SELECT ca.id, 'client'::text AS role, ca.contact_id,
                   c.display_name AS contact_name, ca.session_id, ca.status,
                   NULL::text AS outcome, NULL::smallint AS outcome_rating, ca.notes
            FROM client_assignments ca
            JOIN contacts c ON c.organisation_id = ca.organisation_id AND c.id = ca.contact_id
            WHERE ca.organisation_id=$1 AND ca.id=$2
            UNION ALL
            SELECT wa.id, 'worker'::text AS role, wa.contact_id,
                   c.display_name AS contact_name, wa.session_id, wa.status,
                   wa.outcome, wa.outcome_rating, wa.notes
            FROM worker_assignments wa
            JOIN contacts c ON c.organisation_id = wa.organisation_id AND c.id = wa.contact_id
            WHERE wa.organisation_id=$1 AND wa.id=$2
        ) assignment",
    )
    .bind(organisation_id)
    .bind(id)
    .fetch_optional(&state.db)
    .await?;
    Ok(row)
}

async fn load_constraints(
    state: &AppState,
    organisation_id: Uuid,
    params: &ConstraintListParams,
) -> Result<Vec<ConstraintRow>, AppError> {
    let mut query = QueryBuilder::<Postgres>::new(CONSTRAINT_SELECT);
    query.push(" WHERE rc.organisation_id = ");
    query.push_bind(organisation_id);
    if !params.include_inactive.unwrap_or(false) {
        query.push(" AND rc.is_active");
    }
    if let Some(worker_id) = params.worker_id {
        query.push(" AND rc.worker_contact_id = ");
        query.push_bind(worker_id);
    }
    if let Some(contact_id) = params.target_contact_id {
        query.push(" AND rc.target_contact_id = ");
        query.push_bind(contact_id);
    }
    if let Some(site_id) = params.site_id {
        query.push(" AND rc.target_site_id = ");
        query.push_bind(site_id);
    }
    if let Some(service_id) = params.service_id {
        query.push(" AND rc.target_service_id = ");
        query.push_bind(service_id);
    }
    if let Some(division_id) = params.division_id {
        query.push(" AND rc.target_division_id = ");
        query.push_bind(division_id);
    }
    query.push(" ORDER BY worker_name, target_name, rc.kind");
    Ok(query
        .build_query_as::<ConstraintRow>()
        .fetch_all(&state.db)
        .await?)
}

async fn load_constraint(
    state: &AppState,
    organisation_id: Uuid,
    id: Uuid,
) -> Result<Option<ConstraintRow>, AppError> {
    let row = sqlx::query_as::<_, ConstraintRow>(&format!(
        "{CONSTRAINT_SELECT} WHERE rc.organisation_id = $1 AND rc.id = $2"
    ))
    .bind(organisation_id)
    .bind(id)
    .fetch_optional(&state.db)
    .await?;
    Ok(row)
}

fn constraint_evidence_for(
    constraint: &ConstraintRow,
    candidate_id: Uuid,
    shift: &ShiftContext,
    client_ids: &HashSet<Uuid>,
    assigned_worker_ids: &HashSet<Uuid>,
) -> Option<ConstraintEvidence> {
    let direct = constraint.worker_contact_id == candidate_id;
    let reverse = constraint.worker_contact_id != candidate_id
        && assigned_worker_ids.contains(&constraint.worker_contact_id)
        && constraint.target_kind == "contact"
        && constraint.target_id == candidate_id;
    if !direct && !reverse {
        return None;
    }

    let matches_shift = match constraint.target_kind.as_str() {
        "site" => shift
            .site
            .as_ref()
            .is_some_and(|site| site.id == constraint.target_id),
        "service" => shift
            .service
            .as_ref()
            .is_some_and(|service| service.id == constraint.target_id),
        "division" => shift
            .division
            .as_ref()
            .is_some_and(|division| division.id == constraint.target_id),
        "contact" => {
            client_ids.contains(&constraint.target_id)
                || assigned_worker_ids.contains(&constraint.target_id)
        }
        _ => false,
    };
    if !matches_shift {
        return None;
    }

    let kind = ConstraintKind::parse(&constraint.kind)?;
    Some(ConstraintEvidence {
        kind,
        strength: constraint.strength,
        target_label: constraint.target_name.clone(),
        reason: constraint.reason.clone(),
    })
}

fn site_dto(row: SiteRow) -> SiteDto {
    SiteDto {
        id: row.id,
        name: row.name,
        division_id: row.division_id,
        division_name: row.division_name,
        address: row.address,
        notes: row.notes,
        is_active: row.is_active,
    }
}

fn shift_dto(row: ShiftRow) -> ShiftDto {
    ShiftDto {
        id: row.id,
        title: row.title,
        kind: row.kind,
        starts_at: row.starts_at,
        ends_at: row.ends_at,
        location: row.location,
        status: row.status,
        division_id: row.division_id,
        division_name: row.division_name,
        service_id: row.service_id,
        service_name: row.service_name,
        site_id: row.site_id,
        site_name: row.site_name,
        cost_centre_id: row.cost_centre_id,
        cost_centre_name: row.cost_centre_name,
        cost_centre_code: row.cost_centre_code,
        series_id: row.series_id,
        recurrence: row.recurrence,
        client_count: row.client_count,
        worker_count: row.worker_count,
        clients: row.clients.0,
        workers: row.workers.0,
    }
}

fn assignment_dto(row: AssignmentRow) -> AssignmentDto {
    AssignmentDto {
        id: row.id,
        role: row.role,
        contact_id: row.contact_id,
        contact_name: row.contact_name,
        session_id: row.session_id,
        status: row.status,
        outcome: row.outcome,
        outcome_rating: row.outcome_rating,
        notes: row.notes,
    }
}

fn constraint_dto(row: ConstraintRow) -> RosterConstraintDto {
    RosterConstraintDto {
        id: row.id,
        worker_contact_id: row.worker_contact_id,
        worker_name: row.worker_name,
        target_kind: row.target_kind,
        target_id: row.target_id,
        target_name: row.target_name,
        kind: row.kind,
        strength: row.strength,
        reason: row.reason,
        source: row.source,
        is_active: row.is_active,
        start_date: row.start_date,
        end_date: row.end_date,
    }
}

fn validate_kind(value: Option<&str>) -> Result<&'static str, AppError> {
    match value.unwrap_or("service_delivery") {
        "service_delivery" => Ok("service_delivery"),
        "opening_hours" => Ok("opening_hours"),
        "respite_program" => Ok("respite_program"),
        _ => Err(AppError::validation(
            "kind must be service_delivery, opening_hours or respite_program",
        )),
    }
}

fn validate_status(value: Option<&str>) -> Result<&'static str, AppError> {
    match value.unwrap_or("draft") {
        "draft" => Ok("draft"),
        "published" => Ok("published"),
        "cancelled" => Ok("cancelled"),
        "completed" => Ok("completed"),
        _ => Err(AppError::validation(
            "status must be draft, published, cancelled or completed",
        )),
    }
}

fn validate_times(starts_at: DateTime<Utc>, ends_at: DateTime<Utc>) -> Result<(), AppError> {
    if ends_at <= starts_at {
        return Err(AppError::validation("ends_at must be after starts_at"));
    }
    Ok(())
}

fn validate_assignment_status(value: Option<&str>) -> Result<&'static str, AppError> {
    match value.unwrap_or("assigned") {
        "assigned" => Ok("assigned"),
        "cancelled" => Ok("cancelled"),
        "attended" => Ok("attended"),
        "no_show" => Ok("no_show"),
        _ => Err(AppError::validation(
            "status must be assigned, cancelled, attended or no_show",
        )),
    }
}

fn validate_outcome(value: &str) -> Result<&'static str, AppError> {
    match value {
        "positive" => Ok("positive"),
        "neutral" => Ok("neutral"),
        "negative" => Ok("negative"),
        _ => Err(AppError::validation(
            "outcome must be positive, neutral or negative",
        )),
    }
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

async fn ensure_contact(
    state: &AppState,
    organisation_id: Uuid,
    contact_id: Uuid,
) -> Result<(), AppError> {
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM contacts WHERE organisation_id=$1 AND id=$2)",
    )
    .bind(organisation_id)
    .bind(contact_id)
    .fetch_one(&state.db)
    .await?;
    if !exists {
        return Err(AppError::validation(
            "contact not found in this organisation",
        ));
    }
    Ok(())
}

async fn ensure_division(
    state: &AppState,
    organisation_id: Uuid,
    division_id: Uuid,
) -> Result<(), AppError> {
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM divisions WHERE organisation_id=$1 AND id=$2)",
    )
    .bind(organisation_id)
    .bind(division_id)
    .fetch_one(&state.db)
    .await?;
    if !exists {
        return Err(AppError::validation(
            "division not found in this organisation",
        ));
    }
    Ok(())
}

async fn ensure_service(
    state: &AppState,
    organisation_id: Uuid,
    service_id: Uuid,
) -> Result<(), AppError> {
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM services WHERE organisation_id=$1 AND id=$2)",
    )
    .bind(organisation_id)
    .bind(service_id)
    .fetch_one(&state.db)
    .await?;
    if !exists {
        return Err(AppError::validation(
            "service not found in this organisation",
        ));
    }
    Ok(())
}

async fn ensure_site(
    state: &AppState,
    organisation_id: Uuid,
    site_id: Uuid,
) -> Result<(), AppError> {
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM sites WHERE organisation_id=$1 AND id=$2)")
            .bind(organisation_id)
            .bind(site_id)
            .fetch_one(&state.db)
            .await?;
    if !exists {
        return Err(AppError::validation("site not found in this organisation"));
    }
    Ok(())
}

async fn ensure_session(
    state: &AppState,
    organisation_id: Uuid,
    shift_id: Uuid,
    session_id: Uuid,
) -> Result<(), AppError> {
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(
            SELECT 1 FROM shift_sessions
            WHERE organisation_id=$1 AND shift_id=$2 AND id=$3
        )",
    )
    .bind(organisation_id)
    .bind(shift_id)
    .bind(session_id)
    .fetch_one(&state.db)
    .await?;
    if !exists {
        return Err(AppError::validation("session not found on this shift"));
    }
    Ok(())
}
