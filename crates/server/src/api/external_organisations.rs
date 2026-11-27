//! External organisation records.
//!
//! These are counterparties such as providers, brokers, health services and
//! agencies. They are deliberately separate from the CareLibre tenant
//! organisation profile managed by [`super::organisation`].

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, Postgres, QueryBuilder};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::{
    auth::extractor::AuthContext, error::AppError, permissions::Capability, state::AppState,
};

#[derive(Debug, Serialize, ToSchema)]
pub struct ExternalOrganisationDto {
    pub id: Uuid,
    pub name: String,
    pub kind: Option<String>,
    pub abn: Option<String>,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub website: Option<String>,
    pub notes: Option<String>,
    pub is_active: bool,
    pub relationship_count: i64,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ExternalOrganisationList {
    pub items: Vec<ExternalOrganisationDto>,
    pub total: i64,
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct ListParams {
    /// Match name, kind, ABN, email, phone or website.
    pub search: Option<String>,
    /// `active`, `inactive` or `all` (default).
    pub status: Option<String>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ExternalOrganisationWrite {
    pub name: String,
    pub kind: Option<String>,
    pub abn: Option<String>,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub website: Option<String>,
    pub notes: Option<String>,
    #[serde(default = "default_active")]
    pub is_active: bool,
}

fn default_active() -> bool {
    true
}

#[derive(Debug, FromRow)]
struct OrganisationRow {
    id: Uuid,
    name: String,
    kind: Option<String>,
    abn: Option<String>,
    email: Option<String>,
    phone: Option<String>,
    website: Option<String>,
    notes: Option<String>,
    is_active: bool,
    relationship_count: i64,
    updated_at: DateTime<Utc>,
}

impl From<OrganisationRow> for ExternalOrganisationDto {
    fn from(row: OrganisationRow) -> Self {
        Self {
            id: row.id,
            name: row.name,
            kind: row.kind,
            abn: row.abn,
            email: row.email,
            phone: row.phone,
            website: row.website,
            notes: row.notes,
            is_active: row.is_active,
            relationship_count: row.relationship_count,
            updated_at: row.updated_at,
        }
    }
}

struct ValidatedWrite {
    name: String,
    kind: Option<String>,
    abn: Option<String>,
    email: Option<String>,
    phone: Option<String>,
    website: Option<String>,
    notes: Option<String>,
    is_active: bool,
}

const SELECT: &str = "SELECT eo.id, eo.name, eo.kind, eo.abn, eo.email, eo.phone, eo.website, \
    eo.notes, eo.is_active, eo.updated_at, \
    (SELECT count(*) FROM relationships r \
      WHERE r.organisation_id = eo.tenant_id \
        AND r.removed_at IS NULL \
        AND (r.from_organisation_id = eo.id OR r.to_organisation_id = eo.id)) \
      AS relationship_count \
    FROM organisations eo";

/// `GET /api/v1/organisations`
#[utoipa::path(
    get,
    path = "/api/v1/organisations",
    tag = "organisations",
    security(("bearer_auth" = [])),
    params(ListParams),
    responses(
        (status = 200, description = "External organisations", body = ExternalOrganisationList),
        (status = 401, description = "Missing or invalid access token", body = crate::error::ErrorBody),
        (status = 403, description = "Missing organisations.read capability", body = crate::error::ErrorBody)
    )
)]
pub async fn list(
    State(state): State<AppState>,
    auth: AuthContext,
    Query(params): Query<ListParams>,
) -> Result<Json<ExternalOrganisationList>, AppError> {
    auth.require(Capability::OrganisationsRead)?;

    let search = params.search.unwrap_or_default().trim().to_owned();
    let status = params.status.unwrap_or_else(|| "all".to_owned());
    let limit = i64::from(params.limit.unwrap_or(500).clamp(1, 1000));
    let offset = i64::from(params.offset.unwrap_or(0));

    let mut query = QueryBuilder::<Postgres>::new(SELECT);
    query.push(" WHERE eo.tenant_id = ");
    query.push_bind(auth.organisation_id);
    push_filters(&mut query, &search, &status);
    query.push(" ORDER BY lower(eo.name), eo.id LIMIT ");
    query.push_bind(limit);
    query.push(" OFFSET ");
    query.push_bind(offset);

    let rows = query
        .build_query_as::<OrganisationRow>()
        .fetch_all(&state.db)
        .await?;

    let mut count = QueryBuilder::<Postgres>::new(
        "SELECT count(*) FROM organisations eo WHERE eo.tenant_id = ",
    );
    count.push_bind(auth.organisation_id);
    push_filters(&mut count, &search, &status);
    let total: i64 = count.build_query_scalar().fetch_one(&state.db).await?;

    Ok(Json(ExternalOrganisationList {
        items: rows
            .into_iter()
            .map(ExternalOrganisationDto::from)
            .collect(),
        total,
    }))
}

/// `GET /api/v1/organisations/{id}`
#[utoipa::path(
    get,
    path = "/api/v1/organisations/{id}",
    tag = "organisations",
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "External organisation", body = ExternalOrganisationDto),
        (status = 401, description = "Missing or invalid access token", body = crate::error::ErrorBody),
        (status = 404, description = "Organisation not found", body = crate::error::ErrorBody)
    )
)]
pub async fn get(
    State(state): State<AppState>,
    auth: AuthContext,
    Path(id): Path<Uuid>,
) -> Result<Json<ExternalOrganisationDto>, AppError> {
    auth.require(Capability::OrganisationsRead)?;
    Ok(Json(load(&state, auth.organisation_id, id).await?))
}

/// `POST /api/v1/organisations`
#[utoipa::path(
    post,
    path = "/api/v1/organisations",
    tag = "organisations",
    security(("bearer_auth" = [])),
    request_body = ExternalOrganisationWrite,
    responses(
        (status = 201, description = "Organisation created", body = ExternalOrganisationDto),
        (status = 401, description = "Missing or invalid access token", body = crate::error::ErrorBody),
        (status = 403, description = "Missing organisations.write capability", body = crate::error::ErrorBody),
        (status = 409, description = "An organisation with that name already exists", body = crate::error::ErrorBody),
        (status = 422, description = "Validation failed", body = crate::error::ErrorBody)
    )
)]
pub async fn create(
    State(state): State<AppState>,
    auth: AuthContext,
    Json(payload): Json<ExternalOrganisationWrite>,
) -> Result<(StatusCode, Json<ExternalOrganisationDto>), AppError> {
    auth.require(Capability::OrganisationsWrite)?;
    let write = validate_write(payload)?;
    ensure_unique_name(&state, auth.organisation_id, &write.name, None).await?;

    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO organisations
            (id, tenant_id, name, kind, abn, email, phone, website, notes, is_active)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)",
    )
    .bind(id)
    .bind(auth.organisation_id)
    .bind(&write.name)
    .bind(&write.kind)
    .bind(&write.abn)
    .bind(&write.email)
    .bind(&write.phone)
    .bind(&write.website)
    .bind(&write.notes)
    .bind(write.is_active)
    .execute(&state.db)
    .await
    .map_err(map_write_error)?;

    Ok((
        StatusCode::CREATED,
        Json(load(&state, auth.organisation_id, id).await?),
    ))
}

/// `PATCH /api/v1/organisations/{id}`
#[utoipa::path(
    patch,
    path = "/api/v1/organisations/{id}",
    tag = "organisations",
    security(("bearer_auth" = [])),
    request_body = ExternalOrganisationWrite,
    responses(
        (status = 200, description = "Organisation updated", body = ExternalOrganisationDto),
        (status = 401, description = "Missing or invalid access token", body = crate::error::ErrorBody),
        (status = 403, description = "Missing organisations.write capability", body = crate::error::ErrorBody),
        (status = 404, description = "Organisation not found", body = crate::error::ErrorBody),
        (status = 409, description = "An organisation with that name already exists", body = crate::error::ErrorBody),
        (status = 422, description = "Validation failed", body = crate::error::ErrorBody)
    )
)]
pub async fn update(
    State(state): State<AppState>,
    auth: AuthContext,
    Path(id): Path<Uuid>,
    Json(payload): Json<ExternalOrganisationWrite>,
) -> Result<Json<ExternalOrganisationDto>, AppError> {
    auth.require(Capability::OrganisationsWrite)?;
    let write = validate_write(payload)?;
    ensure_unique_name(&state, auth.organisation_id, &write.name, Some(id)).await?;

    let changed = sqlx::query(
        "UPDATE organisations SET
            name = $1, kind = $2, abn = $3, email = $4, phone = $5,
            website = $6, notes = $7, is_active = $8
         WHERE id = $9 AND tenant_id = $10 AND id <> tenant_id",
    )
    .bind(&write.name)
    .bind(&write.kind)
    .bind(&write.abn)
    .bind(&write.email)
    .bind(&write.phone)
    .bind(&write.website)
    .bind(&write.notes)
    .bind(write.is_active)
    .bind(id)
    .bind(auth.organisation_id)
    .execute(&state.db)
    .await
    .map_err(map_write_error)?
    .rows_affected();

    if changed == 0 {
        return Err(AppError::not_found("organisation not found"));
    }

    Ok(Json(load(&state, auth.organisation_id, id).await?))
}

/// `DELETE /api/v1/organisations/{id}`
///
/// The record is archived rather than physically deleted, so relationship
/// history keeps its display names. The list hides inactive records by
/// default.
#[utoipa::path(
    delete,
    path = "/api/v1/organisations/{id}",
    tag = "organisations",
    security(("bearer_auth" = [])),
    responses(
        (status = 204, description = "Organisation archived"),
        (status = 401, description = "Missing or invalid access token", body = crate::error::ErrorBody),
        (status = 403, description = "Missing organisations.write capability", body = crate::error::ErrorBody),
        (status = 404, description = "Organisation not found", body = crate::error::ErrorBody)
    )
)]
pub async fn remove(
    State(state): State<AppState>,
    auth: AuthContext,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    auth.require(Capability::OrganisationsWrite)?;

    let changed = sqlx::query(
        "UPDATE organisations SET is_active = false
         WHERE id = $1 AND tenant_id = $2 AND id <> tenant_id AND is_active = true",
    )
    .bind(id)
    .bind(auth.organisation_id)
    .execute(&state.db)
    .await?
    .rows_affected();

    if changed == 0 {
        // Idempotent for an already-archived record, but still scoped to the
        // tenant so another organisation's id is a 404.
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM organisations WHERE id = $1 AND tenant_id = $2 AND id <> tenant_id)",
        )
        .bind(id)
        .bind(auth.organisation_id)
        .fetch_one(&state.db)
        .await?;
        if !exists {
            return Err(AppError::not_found("organisation not found"));
        }
    }

    Ok(StatusCode::NO_CONTENT)
}

async fn load(
    state: &AppState,
    organisation_id: Uuid,
    id: Uuid,
) -> Result<ExternalOrganisationDto, AppError> {
    let row = sqlx::query_as::<_, OrganisationRow>(&format!(
        "{SELECT} WHERE eo.id = $1 AND eo.tenant_id = $2 AND eo.id <> eo.tenant_id"
    ))
    .bind(id)
    .bind(organisation_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::not_found("organisation not found"))?;
    Ok(row.into())
}

fn push_filters(query: &mut QueryBuilder<'_, Postgres>, search: &str, status: &str) {
    // The tenant root is the signed-in organisation itself, not a counterparty.
    query.push(" AND eo.id <> eo.tenant_id");
    if !search.is_empty() {
        let pattern = format!("%{search}%");
        query.push(" AND (eo.name ILIKE ");
        query.push_bind(pattern.clone());
        query.push(" OR coalesce(eo.kind, '') ILIKE ");
        query.push_bind(pattern.clone());
        query.push(" OR coalesce(eo.abn, '') ILIKE ");
        query.push_bind(pattern.clone());
        query.push(" OR coalesce(eo.email, '') ILIKE ");
        query.push_bind(pattern.clone());
        query.push(" OR coalesce(eo.phone, '') ILIKE ");
        query.push_bind(pattern.clone());
        query.push(" OR coalesce(eo.website, '') ILIKE ");
        query.push_bind(pattern);
        query.push(")");
    }
    match status {
        "active" => {
            query.push(" AND eo.is_active");
        }
        "inactive" => {
            query.push(" AND NOT eo.is_active");
        }
        _ => {}
    }
}

async fn ensure_unique_name(
    state: &AppState,
    organisation_id: Uuid,
    name: &str,
    exclude: Option<Uuid>,
) -> Result<(), AppError> {
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(
            SELECT 1 FROM organisations
            WHERE tenant_id = $1 AND id <> tenant_id AND lower(name) = lower($2)
              AND ($3::uuid IS NULL OR id <> $3)
        )",
    )
    .bind(organisation_id)
    .bind(name)
    .bind(exclude)
    .fetch_one(&state.db)
    .await?;
    if exists {
        return Err(AppError::conflict(
            "an organisation with that name already exists",
        ));
    }
    Ok(())
}

fn map_write_error(error: sqlx::Error) -> AppError {
    if let sqlx::Error::Database(database) = &error {
        if database.is_unique_violation() {
            return AppError::conflict("an organisation with that name already exists");
        }
    }
    error.into()
}

fn validate_write(payload: ExternalOrganisationWrite) -> Result<ValidatedWrite, AppError> {
    let name = payload.name.trim().to_owned();
    if name.is_empty() || name.chars().count() > 200 {
        return Err(AppError::validation(
            "name must be between 1 and 200 characters",
        ));
    }

    let abn = trim_optional(payload.abn.as_deref()).map(|value| {
        value
            .chars()
            .filter(|character| !character.is_whitespace() && *character != '-')
            .collect::<String>()
    });
    if let Some(abn) = &abn {
        if abn.chars().count() != 11 || !abn.chars().all(|character| character.is_ascii_digit()) {
            return Err(AppError::validation("ABN must be exactly 11 digits"));
        }
    }

    let email = trim_optional(payload.email.as_deref());
    if let Some(email) = &email {
        if !email.contains('@') || email.starts_with('@') || email.ends_with('@') {
            return Err(AppError::validation("email address is not valid"));
        }
    }

    let website = trim_optional(payload.website.as_deref());
    if let Some(website) = &website {
        if !(website.starts_with("http://") || website.starts_with("https://")) {
            return Err(AppError::validation(
                "website must start with http:// or https://",
            ));
        }
    }

    Ok(ValidatedWrite {
        name,
        kind: trim_optional(payload.kind.as_deref()),
        abn,
        email,
        phone: trim_optional(payload.phone.as_deref()),
        website,
        notes: trim_optional(payload.notes.as_deref()),
        is_active: payload.is_active,
    })
}

fn trim_optional(value: Option<&str>) -> Option<String> {
    let value = value?.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload(name: &str) -> ExternalOrganisationWrite {
        ExternalOrganisationWrite {
            name: name.to_owned(),
            kind: None,
            abn: None,
            email: None,
            phone: None,
            website: None,
            notes: None,
            is_active: true,
        }
    }

    #[test]
    fn trims_and_validates_names() {
        let valid = validate_write(payload("  Northside Care  ")).unwrap();
        assert_eq!(valid.name, "Northside Care");
        assert!(validate_write(payload("   ")).is_err());
    }

    #[test]
    fn rejects_bad_abn_and_website() {
        let mut bad_abn = payload("Care Co");
        bad_abn.abn = Some("123".to_owned());
        assert!(validate_write(bad_abn).is_err());

        let mut good_abn = payload("Care Co");
        good_abn.abn = Some("12 345 678 901".to_owned());
        assert_eq!(
            validate_write(good_abn).unwrap().abn.as_deref(),
            Some("12345678901")
        );

        let mut bad_site = payload("Care Co");
        bad_site.website = Some("care.example".to_owned());
        assert!(validate_write(bad_site).is_err());

        let mut good_site = payload("Care Co");
        good_site.website = Some("https://care.example".to_owned());
        assert!(validate_write(good_site).is_ok());
    }
}
