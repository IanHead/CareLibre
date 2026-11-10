//! Worker quality catalogue, worker values and client requirements API.
//!
//! Values are edited in the ordinary contact editor. Intrinsic qualities
//! (gender, languages, cultural identity) are stored on the contact itself and
//! are never written here; only `quality_value` qualities accept values.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use chrono::{NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{types::Json as SqlJson, FromRow, PgPool};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::{
    auth::extractor::AuthContext,
    error::{AppError, ErrorBody},
    permissions::Capability,
    state::AppState,
    worker_qualities::{
        self, normalise_key, validate_requirement, validate_value, Quality, Storage,
    },
};

// ---------------------------------------------------------------------------
// DTOs
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, ToSchema)]
pub struct WorkerQualityDto {
    pub id: Uuid,
    pub key: String,
    pub name: String,
    pub description: Option<String>,
    /// `skill`, `training`, `qualification`, `screening`, `attribute` or `preference`.
    pub category: String,
    /// `boolean`, `select`, `multiselect`, `text` or `date`.
    pub data_type: String,
    pub options: Option<Value>,
    /// `contact_field` (intrinsic, read from the contact) or `quality_value`.
    pub storage: String,
    pub contact_field: Option<String>,
    pub tracks_expiry: bool,
    pub is_sensitive: bool,
    pub is_system: bool,
    pub is_active: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct WorkerQualityList {
    pub items: Vec<WorkerQualityDto>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct WorkerQualityWrite {
    pub name: String,
    /// Stable key; derived from the name when omitted.
    pub key: Option<String>,
    pub category: Option<String>,
    pub data_type: Option<String>,
    pub options: Option<Value>,
    pub tracks_expiry: Option<bool>,
    pub is_sensitive: Option<bool>,
    pub description: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct WorkerQualityStatus {
    /// `false` retires the quality; `true` restores it.
    pub is_active: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct WorkerQualityValueDto {
    pub id: Uuid,
    pub contact_id: Uuid,
    pub quality_id: Uuid,
    pub quality_key: String,
    pub quality_name: String,
    pub category: String,
    pub data_type: String,
    pub tracks_expiry: bool,
    pub value: Value,
    pub expires_on: Option<NaiveDate>,
    pub evidence: Option<String>,
    pub notes: Option<String>,
    /// `current`, `expiring` (within 30 days) or `expired`.
    pub status: String,
    pub days_remaining: Option<i64>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct WorkerQualityValueList {
    pub items: Vec<WorkerQualityValueDto>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct WorkerQualityValueWrite {
    pub quality_id: Uuid,
    pub value: Value,
    pub expires_on: Option<NaiveDate>,
    pub evidence: Option<String>,
    pub notes: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ClientRequirementDto {
    pub id: Uuid,
    pub contact_id: Uuid,
    pub quality_id: Uuid,
    pub quality_key: String,
    pub quality_name: String,
    pub category: String,
    pub data_type: String,
    pub is_sensitive: bool,
    pub operator: String,
    pub value: Option<Value>,
    pub severity: String,
    pub reason: Option<String>,
    pub legal_basis: Option<String>,
    pub service_id: Option<Uuid>,
    pub service_name: Option<String>,
    pub start_date: Option<NaiveDate>,
    pub end_date: Option<NaiveDate>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ClientRequirementList {
    pub items: Vec<ClientRequirementDto>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ClientRequirementWrite {
    pub quality_id: Uuid,
    /// `exists`, `equals`, `not_equals`, `in` or `contains`.
    pub operator: String,
    pub value: Option<Value>,
    /// `warn` (default) or `block`.
    pub severity: Option<String>,
    pub reason: Option<String>,
    pub legal_basis: Option<String>,
    /// Optional scope: only shifts for this service.
    pub service_id: Option<Uuid>,
    pub start_date: Option<NaiveDate>,
    pub end_date: Option<NaiveDate>,
}

// ---------------------------------------------------------------------------
// Rows
// ---------------------------------------------------------------------------

#[derive(Debug, FromRow)]
struct ValueRow {
    id: Uuid,
    worker_contact_id: Uuid,
    quality_id: Uuid,
    quality_key: String,
    quality_name: String,
    category: String,
    data_type: String,
    tracks_expiry: bool,
    value: SqlJson<Value>,
    expires_on: Option<NaiveDate>,
    evidence: Option<String>,
    notes: Option<String>,
}

#[derive(Debug, FromRow)]
struct RequirementRow {
    id: Uuid,
    client_contact_id: Uuid,
    quality_id: Uuid,
    quality_key: String,
    quality_name: String,
    category: String,
    data_type: String,
    is_sensitive: bool,
    operator: String,
    value: Option<SqlJson<Value>>,
    severity: String,
    reason: Option<String>,
    legal_basis: Option<String>,
    service_id: Option<Uuid>,
    service_name: Option<String>,
    start_date: Option<NaiveDate>,
    end_date: Option<NaiveDate>,
}

// ---------------------------------------------------------------------------
// Catalogue
// ---------------------------------------------------------------------------

/// `GET /api/v1/worker-qualities`
#[utoipa::path(
    get,
    path = "/api/v1/worker-qualities",
    tag = "worker qualities",
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "The quality catalogue", body = WorkerQualityList),
        (status = 401, description = "Missing or invalid access token", body = ErrorBody),
        (status = 403, description = "Missing contacts.read capability", body = ErrorBody)
    )
)]
pub async fn list(
    State(state): State<AppState>,
    auth: AuthContext,
) -> Result<Json<WorkerQualityList>, AppError> {
    auth.require(Capability::ContactsRead)?;
    let items = load_catalogue(&state.db, auth.organisation_id).await?;
    Ok(Json(WorkerQualityList {
        items: items.into_iter().map(quality_dto).collect(),
    }))
}

/// `POST /api/v1/worker-qualities`
#[utoipa::path(
    post,
    path = "/api/v1/worker-qualities",
    tag = "worker qualities",
    security(("bearer_auth" = [])),
    request_body = WorkerQualityWrite,
    responses(
        (status = 201, description = "Quality created", body = WorkerQualityDto),
        (status = 401, description = "Missing or invalid access token", body = ErrorBody),
        (status = 403, description = "Missing worker_qualities.manage capability", body = ErrorBody),
        (status = 409, description = "Key already exists", body = ErrorBody),
        (status = 422, description = "Validation failed", body = ErrorBody)
    )
)]
pub async fn create(
    State(state): State<AppState>,
    auth: AuthContext,
    Json(payload): Json<WorkerQualityWrite>,
) -> Result<(StatusCode, Json<WorkerQualityDto>), AppError> {
    auth.require(Capability::WorkerQualitiesManage)?;

    let name = payload.name.trim().to_owned();
    if name.is_empty() || name.chars().count() > 160 {
        return Err(AppError::validation(
            "name must be between 1 and 160 characters",
        ));
    }
    let key = payload
        .key
        .as_deref()
        .and_then(normalise_key)
        .or_else(|| normalise_key(&name))
        .ok_or_else(|| AppError::validation("key needs letters, starting with one"))?;
    let category = payload.category.as_deref().unwrap_or("training");
    if !worker_qualities::CATEGORIES
        .iter()
        .any(|(value, _)| *value == category)
    {
        return Err(AppError::validation(
            "category must be skill, training, qualification, screening, attribute or preference",
        ));
    }
    let data_type = payload.data_type.as_deref().unwrap_or("boolean");
    if !worker_qualities::DATA_TYPES.contains(&data_type) {
        return Err(AppError::validation(
            "data_type must be boolean, select, multiselect, text or date",
        ));
    }
    let options = payload.options.clone();
    if matches!(data_type, "select" | "multiselect") {
        let Some(options) = options.as_ref().and_then(Value::as_array) else {
            return Err(AppError::validation(
                "select and multiselect qualities need an options list",
            ));
        };
        if options.is_empty() || !options.iter().all(Value::is_string) {
            return Err(AppError::validation(
                "options must be a non-empty list of text values",
            ));
        }
    } else if options.is_some() {
        return Err(AppError::validation(
            "only select and multiselect qualities take options",
        ));
    }

    let id = Uuid::now_v7();
    let inserted = sqlx::query(
        "INSERT INTO worker_qualities
            (id, organisation_id, key, name, description, category, data_type, options,
             storage, tracks_expiry, is_sensitive, is_system)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,'quality_value',$9,$10,false)
         ON CONFLICT DO NOTHING",
    )
    .bind(id)
    .bind(auth.organisation_id)
    .bind(&key)
    .bind(&name)
    .bind(trimmed(payload.description.as_deref()))
    .bind(category)
    .bind(data_type)
    .bind(options.map(SqlJson))
    .bind(payload.tracks_expiry.unwrap_or(false))
    .bind(payload.is_sensitive.unwrap_or(false))
    .execute(&state.db)
    .await?
    .rows_affected();
    if inserted == 0 {
        return Err(AppError::conflict(format!(
            "{key} is already a worker quality"
        )));
    }

    let quality = load_quality(&state.db, auth.organisation_id, id).await?;
    Ok((StatusCode::CREATED, Json(quality_dto(quality))))
}

/// `PATCH /api/v1/worker-qualities/{id}`
#[utoipa::path(
    patch,
    path = "/api/v1/worker-qualities/{id}",
    tag = "worker qualities",
    security(("bearer_auth" = [])),
    params(("id" = Uuid, Path, description = "Quality id")),
    request_body = WorkerQualityStatus,
    responses(
        (status = 200, description = "Quality updated", body = WorkerQualityDto),
        (status = 401, description = "Missing or invalid access token", body = ErrorBody),
        (status = 403, description = "Missing worker_qualities.manage capability", body = ErrorBody),
        (status = 404, description = "Quality not found", body = ErrorBody),
        (status = 422, description = "System qualities are read-only", body = ErrorBody)
    )
)]
pub async fn update(
    State(state): State<AppState>,
    auth: AuthContext,
    Path(id): Path<Uuid>,
    Json(payload): Json<WorkerQualityStatus>,
) -> Result<Json<WorkerQualityDto>, AppError> {
    auth.require(Capability::WorkerQualitiesManage)?;
    let quality = load_quality(&state.db, auth.organisation_id, id).await?;
    if quality.is_system {
        return Err(AppError::validation(
            "system qualities are read-only; edit the contact field instead",
        ));
    }
    sqlx::query(
        "UPDATE worker_qualities SET is_active = $3
         WHERE organisation_id = $1 AND id = $2",
    )
    .bind(auth.organisation_id)
    .bind(id)
    .bind(payload.is_active)
    .execute(&state.db)
    .await?;
    Ok(Json(quality_dto(
        load_quality(&state.db, auth.organisation_id, id).await?,
    )))
}

// ---------------------------------------------------------------------------
// Worker values
// ---------------------------------------------------------------------------

/// `GET /api/v1/contacts/{id}/qualities`
#[utoipa::path(
    get,
    path = "/api/v1/contacts/{id}/qualities",
    tag = "worker qualities",
    security(("bearer_auth" = [])),
    params(("id" = Uuid, Path, description = "Worker contact id")),
    responses(
        (status = 200, description = "Qualities held by the worker", body = WorkerQualityValueList),
        (status = 401, description = "Missing or invalid access token", body = ErrorBody),
        (status = 403, description = "Missing contacts.read capability", body = ErrorBody),
        (status = 404, description = "Contact not found", body = ErrorBody)
    )
)]
pub async fn list_values(
    State(state): State<AppState>,
    auth: AuthContext,
    Path(id): Path<Uuid>,
) -> Result<Json<WorkerQualityValueList>, AppError> {
    auth.require(Capability::ContactsRead)?;
    ensure_contact(&state.db, auth.organisation_id, id).await?;
    Ok(Json(WorkerQualityValueList {
        items: load_values(&state.db, auth.organisation_id, id).await?,
    }))
}

/// `POST /api/v1/contacts/{id}/qualities`
#[utoipa::path(
    post,
    path = "/api/v1/contacts/{id}/qualities",
    tag = "worker qualities",
    security(("bearer_auth" = [])),
    params(("id" = Uuid, Path, description = "Worker contact id")),
    request_body = WorkerQualityValueWrite,
    responses(
        (status = 201, description = "Quality value saved", body = WorkerQualityValueDto),
        (status = 401, description = "Missing or invalid access token", body = ErrorBody),
        (status = 403, description = "Missing contacts.write capability", body = ErrorBody),
        (status = 404, description = "Contact or quality not found", body = ErrorBody),
        (status = 422, description = "Validation failed", body = ErrorBody)
    )
)]
pub async fn set_value(
    State(state): State<AppState>,
    auth: AuthContext,
    Path(id): Path<Uuid>,
    Json(payload): Json<WorkerQualityValueWrite>,
) -> Result<(StatusCode, Json<WorkerQualityValueDto>), AppError> {
    auth.require(Capability::ContactsWrite)?;
    ensure_contact(&state.db, auth.organisation_id, id).await?;
    let quality = load_quality(&state.db, auth.organisation_id, payload.quality_id).await?;
    if Storage::parse(&quality.storage) != Some(Storage::QualityValue) {
        return Err(AppError::validation(
            "intrinsic qualities are edited on the contact, not as a quality value",
        ));
    }
    if !quality.is_active {
        return Err(AppError::validation(format!(
            "{} has been retired",
            quality.name
        )));
    }
    validate_value(&quality, &payload.value)?;
    if payload.expires_on.is_some() && !quality.tracks_expiry {
        return Err(AppError::validation(format!(
            "{} does not track expiry",
            quality.name
        )));
    }

    sqlx::query(
        "INSERT INTO worker_quality_values
            (id, organisation_id, worker_contact_id, quality_id, value, expires_on, evidence, notes)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8)
         ON CONFLICT (organisation_id, worker_contact_id, quality_id)
             WHERE removed_at IS NULL
         DO UPDATE SET value = excluded.value, expires_on = excluded.expires_on,
                       evidence = excluded.evidence, notes = excluded.notes,
                       removed_at = NULL, updated_at = now()",
    )
    .bind(Uuid::now_v7())
    .bind(auth.organisation_id)
    .bind(id)
    .bind(payload.quality_id)
    .bind(SqlJson(payload.value))
    .bind(payload.expires_on)
    .bind(trimmed(payload.evidence.as_deref()))
    .bind(trimmed(payload.notes.as_deref()))
    .execute(&state.db)
    .await?;

    let value = load_value(&state.db, auth.organisation_id, id, payload.quality_id)
        .await?
        .ok_or_else(|| AppError::not_found("quality value not found after saving"))?;
    Ok((StatusCode::CREATED, Json(value)))
}

/// `DELETE /api/v1/contacts/{id}/qualities/{quality_id}`
#[utoipa::path(
    delete,
    path = "/api/v1/contacts/{id}/qualities/{quality_id}",
    tag = "worker qualities",
    security(("bearer_auth" = [])),
    params(
        ("id" = Uuid, Path, description = "Worker contact id"),
        ("quality_id" = Uuid, Path, description = "Quality id")
    ),
    responses(
        (status = 204, description = "Quality value removed"),
        (status = 401, description = "Missing or invalid access token", body = ErrorBody),
        (status = 403, description = "Missing contacts.write capability", body = ErrorBody),
        (status = 404, description = "Quality value not found", body = ErrorBody)
    )
)]
pub async fn delete_value(
    State(state): State<AppState>,
    auth: AuthContext,
    Path((id, quality_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, AppError> {
    auth.require(Capability::ContactsWrite)?;
    let removed = sqlx::query(
        "UPDATE worker_quality_values SET removed_at = now()
         WHERE organisation_id = $1 AND worker_contact_id = $2 AND quality_id = $3
           AND removed_at IS NULL",
    )
    .bind(auth.organisation_id)
    .bind(id)
    .bind(quality_id)
    .execute(&state.db)
    .await?
    .rows_affected();
    if removed == 0 {
        return Err(AppError::not_found("quality value not found"));
    }
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// Client requirements
// ---------------------------------------------------------------------------

/// `GET /api/v1/contacts/{id}/requirements`
#[utoipa::path(
    get,
    path = "/api/v1/contacts/{id}/requirements",
    tag = "worker qualities",
    security(("bearer_auth" = [])),
    params(("id" = Uuid, Path, description = "Client contact id")),
    responses(
        (status = 200, description = "Requirements the client places on workers", body = ClientRequirementList),
        (status = 401, description = "Missing or invalid access token", body = ErrorBody),
        (status = 403, description = "Missing contacts.read capability", body = ErrorBody),
        (status = 404, description = "Contact not found", body = ErrorBody)
    )
)]
pub async fn list_requirements(
    State(state): State<AppState>,
    auth: AuthContext,
    Path(id): Path<Uuid>,
) -> Result<Json<ClientRequirementList>, AppError> {
    auth.require(Capability::ContactsRead)?;
    ensure_contact(&state.db, auth.organisation_id, id).await?;
    Ok(Json(ClientRequirementList {
        items: load_requirements(&state.db, auth.organisation_id, id).await?,
    }))
}

/// `POST /api/v1/contacts/{id}/requirements`
#[utoipa::path(
    post,
    path = "/api/v1/contacts/{id}/requirements",
    tag = "worker qualities",
    security(("bearer_auth" = [])),
    params(("id" = Uuid, Path, description = "Client contact id")),
    request_body = ClientRequirementWrite,
    responses(
        (status = 201, description = "Requirement saved", body = ClientRequirementDto),
        (status = 401, description = "Missing or invalid access token", body = ErrorBody),
        (status = 403, description = "Missing contacts.write capability", body = ErrorBody),
        (status = 404, description = "Contact or quality not found", body = ErrorBody),
        (status = 422, description = "Validation failed", body = ErrorBody)
    )
)]
pub async fn set_requirement(
    State(state): State<AppState>,
    auth: AuthContext,
    Path(id): Path<Uuid>,
    Json(payload): Json<ClientRequirementWrite>,
) -> Result<(StatusCode, Json<ClientRequirementDto>), AppError> {
    auth.require(Capability::ContactsWrite)?;
    ensure_contact(&state.db, auth.organisation_id, id).await?;
    let quality = load_quality(&state.db, auth.organisation_id, payload.quality_id).await?;
    if !quality.is_active {
        return Err(AppError::validation(format!(
            "{} has been retired",
            quality.name
        )));
    }

    let operator = payload.operator.trim().to_ascii_lowercase();
    let severity = payload.severity.as_deref().unwrap_or("warn");
    validate_requirement(&quality, &operator, payload.value.as_ref(), severity)?;
    validate_dates(payload.start_date, payload.end_date)?;

    let reason = trimmed(payload.reason.as_deref());
    let legal_basis = trimmed(payload.legal_basis.as_deref());
    if quality.is_sensitive && reason.is_none() && legal_basis.is_none() {
        return Err(AppError::validation(format!(
            "{} is sensitive; record a reason or legal basis for the requirement",
            quality.name
        )));
    }

    if let Some(service_id) = payload.service_id {
        ensure_service(&state.db, auth.organisation_id, service_id).await?;
    }

    sqlx::query(
        "INSERT INTO client_quality_requirements
            (id, organisation_id, client_contact_id, quality_id, operator, value,
             severity, reason, legal_basis, service_id, start_date, end_date, created_by)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13)
         ON CONFLICT (organisation_id, client_contact_id, quality_id,
                      COALESCE(service_id, '00000000-0000-0000-0000-000000000000'::uuid))
         DO UPDATE SET operator = excluded.operator, value = excluded.value,
                       severity = excluded.severity, reason = excluded.reason,
                       legal_basis = excluded.legal_basis, start_date = excluded.start_date,
                       end_date = excluded.end_date, updated_at = now()",
    )
    .bind(Uuid::now_v7())
    .bind(auth.organisation_id)
    .bind(id)
    .bind(payload.quality_id)
    .bind(&operator)
    .bind(payload.value.clone().map(SqlJson))
    .bind(severity)
    .bind(reason)
    .bind(legal_basis)
    .bind(payload.service_id)
    .bind(payload.start_date)
    .bind(payload.end_date)
    .bind(auth.user_id)
    .execute(&state.db)
    .await?;

    let requirement = load_requirement_for_quality(
        &state.db,
        auth.organisation_id,
        id,
        payload.quality_id,
        payload.service_id,
    )
    .await?
    .ok_or_else(|| AppError::not_found("requirement not found after saving"))?;
    Ok((StatusCode::CREATED, Json(requirement)))
}

/// `DELETE /api/v1/contacts/{id}/requirements/{requirement_id}`
#[utoipa::path(
    delete,
    path = "/api/v1/contacts/{id}/requirements/{requirement_id}",
    tag = "worker qualities",
    security(("bearer_auth" = [])),
    params(
        ("id" = Uuid, Path, description = "Client contact id"),
        ("requirement_id" = Uuid, Path, description = "Requirement id")
    ),
    responses(
        (status = 204, description = "Requirement removed"),
        (status = 401, description = "Missing or invalid access token", body = ErrorBody),
        (status = 403, description = "Missing contacts.write capability", body = ErrorBody),
        (status = 404, description = "Requirement not found", body = ErrorBody)
    )
)]
pub async fn delete_requirement(
    State(state): State<AppState>,
    auth: AuthContext,
    Path((id, requirement_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, AppError> {
    auth.require(Capability::ContactsWrite)?;
    let removed = sqlx::query(
        "DELETE FROM client_quality_requirements
         WHERE organisation_id = $1 AND client_contact_id = $2 AND id = $3",
    )
    .bind(auth.organisation_id)
    .bind(id)
    .bind(requirement_id)
    .execute(&state.db)
    .await?
    .rows_affected();
    if removed == 0 {
        return Err(AppError::not_found("requirement not found"));
    }
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// Loading
// ---------------------------------------------------------------------------

pub async fn load_catalogue(db: &PgPool, organisation_id: Uuid) -> Result<Vec<Quality>, AppError> {
    Ok(sqlx::query_as::<_, Quality>(
        "SELECT id, key, name, description, category, data_type, options,
                storage, contact_field, tracks_expiry, is_sensitive, is_system, is_active
         FROM worker_qualities
         WHERE organisation_id = $1
         ORDER BY CASE category
                      WHEN 'attribute' THEN 1 WHEN 'skill' THEN 2 WHEN 'training' THEN 3
                      WHEN 'qualification' THEN 4 WHEN 'screening' THEN 5 ELSE 6 END,
                  name, id",
    )
    .bind(organisation_id)
    .fetch_all(db)
    .await?)
}

pub async fn load_values(
    db: &PgPool,
    organisation_id: Uuid,
    contact_id: Uuid,
) -> Result<Vec<WorkerQualityValueDto>, AppError> {
    let rows = sqlx::query_as::<_, ValueRow>(
        "SELECT v.id, v.worker_contact_id, v.quality_id, q.key AS quality_key,
                q.name AS quality_name, q.category, q.data_type, q.tracks_expiry,
                v.value, v.expires_on, v.evidence, v.notes
         FROM worker_quality_values v
         JOIN worker_qualities q
           ON q.organisation_id = v.organisation_id AND q.id = v.quality_id
         WHERE v.organisation_id = $1 AND v.worker_contact_id = $2
           AND v.removed_at IS NULL
         ORDER BY CASE q.category
                      WHEN 'attribute' THEN 1 WHEN 'skill' THEN 2 WHEN 'training' THEN 3
                      WHEN 'qualification' THEN 4 WHEN 'screening' THEN 5 ELSE 6 END,
                  q.name",
    )
    .bind(organisation_id)
    .bind(contact_id)
    .fetch_all(db)
    .await?;

    let today = Utc::now().date_naive();
    Ok(rows.into_iter().map(|row| value_dto(row, today)).collect())
}

pub async fn load_requirements(
    db: &PgPool,
    organisation_id: Uuid,
    contact_id: Uuid,
) -> Result<Vec<ClientRequirementDto>, AppError> {
    let rows = sqlx::query_as::<_, RequirementRow>(
        "SELECT r.id, r.client_contact_id, r.quality_id, q.key AS quality_key,
                q.name AS quality_name, q.category, q.data_type, q.is_sensitive,
                r.operator, r.value, r.severity, r.reason, r.legal_basis,
                r.service_id, sv.name AS service_name, r.start_date, r.end_date
         FROM client_quality_requirements r
         JOIN worker_qualities q
           ON q.organisation_id = r.organisation_id AND q.id = r.quality_id
         LEFT JOIN services sv
           ON sv.organisation_id = r.organisation_id AND sv.id = r.service_id
         WHERE r.organisation_id = $1 AND r.client_contact_id = $2
         ORDER BY q.name, r.service_id NULLS FIRST, r.id",
    )
    .bind(organisation_id)
    .bind(contact_id)
    .fetch_all(db)
    .await?;
    Ok(rows.into_iter().map(requirement_dto).collect())
}

async fn load_quality(db: &PgPool, organisation_id: Uuid, id: Uuid) -> Result<Quality, AppError> {
    sqlx::query_as::<_, Quality>(
        "SELECT id, key, name, description, category, data_type, options,
                storage, contact_field, tracks_expiry, is_sensitive, is_system, is_active
         FROM worker_qualities
         WHERE organisation_id = $1 AND id = $2",
    )
    .bind(organisation_id)
    .bind(id)
    .fetch_optional(db)
    .await?
    .ok_or_else(|| AppError::not_found("worker quality not found"))
}

async fn load_value(
    db: &PgPool,
    organisation_id: Uuid,
    contact_id: Uuid,
    quality_id: Uuid,
) -> Result<Option<WorkerQualityValueDto>, AppError> {
    let row = sqlx::query_as::<_, ValueRow>(
        "SELECT v.id, v.worker_contact_id, v.quality_id, q.key AS quality_key,
                q.name AS quality_name, q.category, q.data_type, q.tracks_expiry,
                v.value, v.expires_on, v.evidence, v.notes
         FROM worker_quality_values v
         JOIN worker_qualities q
           ON q.organisation_id = v.organisation_id AND q.id = v.quality_id
         WHERE v.organisation_id = $1 AND v.worker_contact_id = $2
           AND v.quality_id = $3 AND v.removed_at IS NULL",
    )
    .bind(organisation_id)
    .bind(contact_id)
    .bind(quality_id)
    .fetch_optional(db)
    .await?;
    Ok(row.map(|row| value_dto(row, Utc::now().date_naive())))
}

async fn load_requirement_for_quality(
    db: &PgPool,
    organisation_id: Uuid,
    contact_id: Uuid,
    quality_id: Uuid,
    service_id: Option<Uuid>,
) -> Result<Option<ClientRequirementDto>, AppError> {
    let row = sqlx::query_as::<_, RequirementRow>(
        "SELECT r.id, r.client_contact_id, r.quality_id, q.key AS quality_key,
                q.name AS quality_name, q.category, q.data_type, q.is_sensitive,
                r.operator, r.value, r.severity, r.reason, r.legal_basis,
                r.service_id, sv.name AS service_name, r.start_date, r.end_date
         FROM client_quality_requirements r
         JOIN worker_qualities q
           ON q.organisation_id = r.organisation_id AND q.id = r.quality_id
         LEFT JOIN services sv
           ON sv.organisation_id = r.organisation_id AND sv.id = r.service_id
         WHERE r.organisation_id = $1 AND r.client_contact_id = $2
           AND r.quality_id = $3 AND r.service_id IS NOT DISTINCT FROM $4",
    )
    .bind(organisation_id)
    .bind(contact_id)
    .bind(quality_id)
    .bind(service_id)
    .fetch_optional(db)
    .await?;
    Ok(row.map(requirement_dto))
}

// ---------------------------------------------------------------------------
// Mapping and validation
// ---------------------------------------------------------------------------

pub(crate) fn quality_dto(quality: Quality) -> WorkerQualityDto {
    WorkerQualityDto {
        id: quality.id,
        key: quality.key,
        name: quality.name,
        description: quality.description,
        category: quality.category,
        data_type: quality.data_type,
        options: quality.options.map(|options| options.0),
        storage: quality.storage,
        contact_field: quality.contact_field,
        tracks_expiry: quality.tracks_expiry,
        is_sensitive: quality.is_sensitive,
        is_system: quality.is_system,
        is_active: quality.is_active,
    }
}

fn value_dto(row: ValueRow, today: NaiveDate) -> WorkerQualityValueDto {
    let days_remaining = row.expires_on.map(|date| (date - today).num_days());
    let status = if row.tracks_expiry {
        match days_remaining {
            Some(days) if days < 0 => "expired",
            Some(days) if days <= 30 => "expiring",
            _ => "current",
        }
    } else {
        "current"
    };

    WorkerQualityValueDto {
        id: row.id,
        contact_id: row.worker_contact_id,
        quality_id: row.quality_id,
        quality_key: row.quality_key,
        quality_name: row.quality_name,
        category: row.category,
        data_type: row.data_type,
        tracks_expiry: row.tracks_expiry,
        value: row.value.0,
        expires_on: row.expires_on,
        evidence: row.evidence,
        notes: row.notes,
        status: status.to_owned(),
        days_remaining,
    }
}

fn requirement_dto(row: RequirementRow) -> ClientRequirementDto {
    ClientRequirementDto {
        id: row.id,
        contact_id: row.client_contact_id,
        quality_id: row.quality_id,
        quality_key: row.quality_key,
        quality_name: row.quality_name,
        category: row.category,
        data_type: row.data_type,
        is_sensitive: row.is_sensitive,
        operator: row.operator,
        value: row.value.map(|value| value.0),
        severity: row.severity,
        reason: row.reason,
        legal_basis: row.legal_basis,
        service_id: row.service_id,
        service_name: row.service_name,
        start_date: row.start_date,
        end_date: row.end_date,
    }
}

async fn ensure_contact(db: &PgPool, organisation_id: Uuid, id: Uuid) -> Result<(), AppError> {
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM contacts WHERE organisation_id = $1 AND id = $2)",
    )
    .bind(organisation_id)
    .bind(id)
    .fetch_one(db)
    .await?;
    if exists {
        Ok(())
    } else {
        Err(AppError::not_found("contact not found"))
    }
}

async fn ensure_service(db: &PgPool, organisation_id: Uuid, id: Uuid) -> Result<(), AppError> {
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM services WHERE organisation_id = $1 AND id = $2)",
    )
    .bind(organisation_id)
    .bind(id)
    .fetch_one(db)
    .await?;
    if exists {
        Ok(())
    } else {
        Err(AppError::not_found("service not found"))
    }
}

fn validate_dates(start: Option<NaiveDate>, end: Option<NaiveDate>) -> Result<(), AppError> {
    if let (Some(start), Some(end)) = (start, end) {
        if end < start {
            return Err(AppError::validation("end_date cannot be before start_date"));
        }
    }
    Ok(())
}

fn trimmed(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}
