//! Funding sources, client packages, agreements and roster generation API.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use chrono::{NaiveDate, NaiveTime};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::{
    auth::extractor::AuthContext,
    error::{AppError, ErrorBody},
    funding::{
        self, Agreement, AgreementLine, AgreementWrite, FundingPackage, FundingSource, LineWrite,
        PackageWrite, CLAIMING_MODELS, SOURCE_KINDS,
    },
    permissions::Capability,
    state::AppState,
    worker_qualities::normalise_key,
};

#[derive(Debug, Serialize, ToSchema)]
pub struct FundingSourceList {
    pub items: Vec<FundingSource>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct FundingSourceWrite {
    pub name: String,
    pub key: Option<String>,
    /// `ndis`, `support_at_home`, `chsp`, `private` or `other`.
    pub kind: String,
    /// `ndis_price_guide`, `support_at_home`, `chsp`, `fee_for_service` or `manual`.
    pub claiming_model: Option<String>,
    pub description: Option<String>,
    pub configuration: Option<Value>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct FundingSourceStatus {
    pub is_active: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct FundingPackageList {
    pub items: Vec<FundingPackage>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct FundingPackageWrite {
    pub funding_source_id: Uuid,
    pub name: String,
    pub reference: Option<String>,
    pub start_date: NaiveDate,
    pub end_date: Option<NaiveDate>,
    pub budget_amount: Option<f64>,
    pub notes: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct FundingPackageStatus {
    /// `draft`, `active`, `exhausted` or `closed`.
    pub status: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AgreementList {
    pub items: Vec<Agreement>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct AgreementWriteDto {
    pub funding_package_id: Uuid,
    pub title: String,
    pub status: Option<String>,
    pub quote_date: Option<NaiveDate>,
    pub start_date: NaiveDate,
    pub end_date: Option<NaiveDate>,
    pub notes: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct AgreementStatus {
    /// `draft`, `quoted` or `accepted`. Finalise activates the agreement.
    pub status: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct AgreementLineWrite {
    pub service_id: Option<Uuid>,
    pub description: String,
    pub quantity: f64,
    /// `hour`, `session`, `each` or `day`.
    pub unit: Option<String>,
    pub unit_rate: f64,
    /// `none`, `daily`, `weekly` or `fortnightly`.
    pub recurrence: Option<String>,
    /// 1 (Monday) to 7 (Sunday).
    #[serde(default)]
    pub weekdays: Vec<i16>,
    pub start_time: Option<NaiveTime>,
    pub duration_minutes: Option<i32>,
    pub site_id: Option<Uuid>,
    pub notes: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AgreementFinalised {
    pub agreement: Agreement,
    pub generated_shifts: usize,
}

/// `GET /api/v1/funding-sources`
#[utoipa::path(
    get,
    path = "/api/v1/funding-sources",
    tag = "funding",
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Funding sources", body = FundingSourceList),
        (status = 401, description = "Missing or invalid access token", body = ErrorBody),
        (status = 403, description = "Missing finance.read capability", body = ErrorBody)
    )
)]
pub async fn list_sources(
    State(state): State<AppState>,
    auth: AuthContext,
) -> Result<Json<FundingSourceList>, AppError> {
    auth.require(Capability::FinanceRead)?;
    Ok(Json(FundingSourceList {
        items: funding::load_sources(&state.db, auth.organisation_id).await?,
    }))
}

/// `POST /api/v1/funding-sources`
#[utoipa::path(
    post,
    path = "/api/v1/funding-sources",
    tag = "funding",
    security(("bearer_auth" = [])),
    request_body = FundingSourceWrite,
    responses(
        (status = 201, description = "Funding source created", body = FundingSource),
        (status = 401, description = "Missing or invalid access token", body = ErrorBody),
        (status = 403, description = "Missing finance.write capability", body = ErrorBody),
        (status = 409, description = "Key already exists", body = ErrorBody),
        (status = 422, description = "Validation failed", body = ErrorBody)
    )
)]
pub async fn create_source(
    State(state): State<AppState>,
    auth: AuthContext,
    Json(payload): Json<FundingSourceWrite>,
) -> Result<(StatusCode, Json<FundingSource>), AppError> {
    auth.require(Capability::FinanceWrite)?;

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
    if !SOURCE_KINDS.contains(&payload.kind.as_str()) {
        return Err(AppError::validation(
            "kind must be ndis, support_at_home, chsp, private or other",
        ));
    }
    let claiming_model = payload.claiming_model.as_deref().unwrap_or("manual");
    if !CLAIMING_MODELS.contains(&claiming_model) {
        return Err(AppError::validation(
            "claiming_model must be ndis_price_guide, support_at_home, chsp, fee_for_service or manual",
        ));
    }
    let configuration = payload
        .configuration
        .clone()
        .unwrap_or_else(|| Value::Object(Default::default()));
    if !configuration.is_object() {
        return Err(AppError::validation("configuration must be a JSON object"));
    }

    let id = Uuid::now_v7();
    let inserted = sqlx::query(
        "INSERT INTO funding_sources
            (id, organisation_id, key, name, kind, claiming_model, description,
             configuration, is_active, is_system)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,true,false)
         ON CONFLICT DO NOTHING",
    )
    .bind(id)
    .bind(auth.organisation_id)
    .bind(&key)
    .bind(&name)
    .bind(&payload.kind)
    .bind(claiming_model)
    .bind(trimmed(payload.description.as_deref()))
    .bind(sqlx::types::Json(configuration))
    .execute(&state.db)
    .await?
    .rows_affected();
    if inserted == 0 {
        return Err(AppError::conflict(format!(
            "{key} is already a funding source"
        )));
    }

    let source = funding::load_sources(&state.db, auth.organisation_id)
        .await?
        .into_iter()
        .find(|source| source.id == id)
        .ok_or_else(|| AppError::not_found("funding source not found after creation"))?;
    Ok((StatusCode::CREATED, Json(source)))
}

/// `PATCH /api/v1/funding-sources/{id}`
#[utoipa::path(
    patch,
    path = "/api/v1/funding-sources/{id}",
    tag = "funding",
    security(("bearer_auth" = [])),
    params(("id" = Uuid, Path, description = "Funding source id")),
    request_body = FundingSourceStatus,
    responses(
        (status = 200, description = "Funding source updated", body = FundingSource),
        (status = 401, description = "Missing or invalid access token", body = ErrorBody),
        (status = 403, description = "Missing finance.write capability", body = ErrorBody),
        (status = 404, description = "Funding source not found", body = ErrorBody)
    )
)]
pub async fn update_source(
    State(state): State<AppState>,
    auth: AuthContext,
    Path(id): Path<Uuid>,
    Json(payload): Json<FundingSourceStatus>,
) -> Result<Json<FundingSource>, AppError> {
    auth.require(Capability::FinanceWrite)?;
    let changed = sqlx::query(
        "UPDATE funding_sources SET is_active = $3
         WHERE organisation_id = $1 AND id = $2",
    )
    .bind(auth.organisation_id)
    .bind(id)
    .bind(payload.is_active)
    .execute(&state.db)
    .await?
    .rows_affected();
    if changed == 0 {
        return Err(AppError::not_found("funding source not found"));
    }
    let source = funding::load_sources(&state.db, auth.organisation_id)
        .await?
        .into_iter()
        .find(|source| source.id == id)
        .ok_or_else(|| AppError::not_found("funding source not found"))?;
    Ok(Json(source))
}

/// `GET /api/v1/contacts/{id}/funding-packages`
#[utoipa::path(
    get,
    path = "/api/v1/contacts/{id}/funding-packages",
    tag = "funding",
    security(("bearer_auth" = [])),
    params(("id" = Uuid, Path, description = "Client contact id")),
    responses(
        (status = 200, description = "Funding packages held by the client", body = FundingPackageList),
        (status = 401, description = "Missing or invalid access token", body = ErrorBody),
        (status = 403, description = "Missing finance.read capability", body = ErrorBody)
    )
)]
pub async fn list_packages(
    State(state): State<AppState>,
    auth: AuthContext,
    Path(id): Path<Uuid>,
) -> Result<Json<FundingPackageList>, AppError> {
    auth.require(Capability::FinanceRead)?;
    Ok(Json(FundingPackageList {
        items: funding::load_packages(&state.db, auth.organisation_id, id).await?,
    }))
}

/// `POST /api/v1/contacts/{id}/funding-packages`
#[utoipa::path(
    post,
    path = "/api/v1/contacts/{id}/funding-packages",
    tag = "funding",
    security(("bearer_auth" = [])),
    params(("id" = Uuid, Path, description = "Client contact id")),
    request_body = FundingPackageWrite,
    responses(
        (status = 201, description = "Funding package created", body = FundingPackage),
        (status = 401, description = "Missing or invalid access token", body = ErrorBody),
        (status = 403, description = "Missing finance.write capability", body = ErrorBody),
        (status = 404, description = "Client or source not found", body = ErrorBody),
        (status = 422, description = "Validation failed", body = ErrorBody)
    )
)]
pub async fn create_package(
    State(state): State<AppState>,
    auth: AuthContext,
    Path(id): Path<Uuid>,
    Json(payload): Json<FundingPackageWrite>,
) -> Result<(StatusCode, Json<FundingPackage>), AppError> {
    auth.require(Capability::FinanceWrite)?;
    let package = funding::create_package(
        &state.db,
        auth.organisation_id,
        id,
        PackageWrite {
            funding_source_id: payload.funding_source_id,
            name: payload.name,
            reference: payload.reference,
            start_date: payload.start_date,
            end_date: payload.end_date,
            budget_amount: payload.budget_amount,
            notes: payload.notes,
        },
    )
    .await?;
    Ok((StatusCode::CREATED, Json(package)))
}

/// `PATCH /api/v1/funding-packages/{id}`
#[utoipa::path(
    patch,
    path = "/api/v1/funding-packages/{id}",
    tag = "funding",
    security(("bearer_auth" = [])),
    params(("id" = Uuid, Path, description = "Funding package id")),
    request_body = FundingPackageStatus,
    responses(
        (status = 204, description = "Package status updated"),
        (status = 401, description = "Missing or invalid access token", body = ErrorBody),
        (status = 403, description = "Missing finance.write capability", body = ErrorBody),
        (status = 404, description = "Funding package not found", body = ErrorBody)
    )
)]
pub async fn update_package_status(
    State(state): State<AppState>,
    auth: AuthContext,
    Path(id): Path<Uuid>,
    Json(payload): Json<FundingPackageStatus>,
) -> Result<StatusCode, AppError> {
    auth.require(Capability::FinanceWrite)?;
    funding::set_package_status(&state.db, auth.organisation_id, id, &payload.status).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `GET /api/v1/contacts/{id}/agreements`
#[utoipa::path(
    get,
    path = "/api/v1/contacts/{id}/agreements",
    tag = "funding",
    security(("bearer_auth" = [])),
    params(("id" = Uuid, Path, description = "Client contact id")),
    responses(
        (status = 200, description = "Quotes and service agreements for the client", body = AgreementList),
        (status = 401, description = "Missing or invalid access token", body = ErrorBody),
        (status = 403, description = "Missing finance.read capability", body = ErrorBody)
    )
)]
pub async fn list_agreements(
    State(state): State<AppState>,
    auth: AuthContext,
    Path(id): Path<Uuid>,
) -> Result<Json<AgreementList>, AppError> {
    auth.require(Capability::FinanceRead)?;
    Ok(Json(AgreementList {
        items: funding::load_agreements(&state.db, auth.organisation_id, id).await?,
    }))
}

/// `POST /api/v1/contacts/{id}/agreements`
#[utoipa::path(
    post,
    path = "/api/v1/contacts/{id}/agreements",
    tag = "funding",
    security(("bearer_auth" = [])),
    params(("id" = Uuid, Path, description = "Client contact id")),
    request_body = AgreementWriteDto,
    responses(
        (status = 201, description = "Agreement created", body = Agreement),
        (status = 401, description = "Missing or invalid access token", body = ErrorBody),
        (status = 403, description = "Missing finance.write capability", body = ErrorBody),
        (status = 404, description = "Client or package not found", body = ErrorBody),
        (status = 422, description = "Validation failed", body = ErrorBody)
    )
)]
pub async fn create_agreement(
    State(state): State<AppState>,
    auth: AuthContext,
    Path(id): Path<Uuid>,
    Json(payload): Json<AgreementWriteDto>,
) -> Result<(StatusCode, Json<Agreement>), AppError> {
    auth.require(Capability::FinanceWrite)?;
    let agreement = funding::create_agreement(
        &state.db,
        auth.organisation_id,
        id,
        AgreementWrite {
            funding_package_id: payload.funding_package_id,
            title: payload.title,
            status: payload.status,
            quote_date: payload.quote_date,
            start_date: payload.start_date,
            end_date: payload.end_date,
            notes: payload.notes,
        },
    )
    .await?;
    Ok((StatusCode::CREATED, Json(agreement)))
}

/// `PATCH /api/v1/agreements/{id}`
#[utoipa::path(
    patch,
    path = "/api/v1/agreements/{id}",
    tag = "funding",
    security(("bearer_auth" = [])),
    params(("id" = Uuid, Path, description = "Agreement id")),
    request_body = AgreementStatus,
    responses(
        (status = 204, description = "Agreement status updated"),
        (status = 401, description = "Missing or invalid access token", body = ErrorBody),
        (status = 403, description = "Missing finance.write capability", body = ErrorBody),
        (status = 404, description = "Agreement not found or finalised", body = ErrorBody)
    )
)]
pub async fn update_agreement_status(
    State(state): State<AppState>,
    auth: AuthContext,
    Path(id): Path<Uuid>,
    Json(payload): Json<AgreementStatus>,
) -> Result<StatusCode, AppError> {
    auth.require(Capability::FinanceWrite)?;
    funding::set_agreement_status(&state.db, auth.organisation_id, id, &payload.status).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /api/v1/agreements/{id}/lines`
#[utoipa::path(
    post,
    path = "/api/v1/agreements/{id}/lines",
    tag = "funding",
    security(("bearer_auth" = [])),
    params(("id" = Uuid, Path, description = "Agreement id")),
    request_body = AgreementLineWrite,
    responses(
        (status = 201, description = "Agreement line created", body = AgreementLine),
        (status = 401, description = "Missing or invalid access token", body = ErrorBody),
        (status = 403, description = "Missing finance.write capability", body = ErrorBody),
        (status = 404, description = "Agreement, service or site not found", body = ErrorBody),
        (status = 422, description = "Validation failed", body = ErrorBody)
    )
)]
pub async fn add_line(
    State(state): State<AppState>,
    auth: AuthContext,
    Path(id): Path<Uuid>,
    Json(payload): Json<AgreementLineWrite>,
) -> Result<(StatusCode, Json<AgreementLine>), AppError> {
    auth.require(Capability::FinanceWrite)?;
    let line = funding::add_line(
        &state.db,
        auth.organisation_id,
        id,
        LineWrite {
            service_id: payload.service_id,
            description: payload.description,
            quantity: payload.quantity,
            unit: payload.unit,
            unit_rate: payload.unit_rate,
            recurrence: payload.recurrence,
            weekdays: payload.weekdays,
            start_time: payload.start_time,
            duration_minutes: payload.duration_minutes,
            site_id: payload.site_id,
            notes: payload.notes,
        },
    )
    .await?;
    Ok((StatusCode::CREATED, Json(line)))
}

/// `DELETE /api/v1/agreements/{id}/lines/{line_id}`
#[utoipa::path(
    delete,
    path = "/api/v1/agreements/{id}/lines/{line_id}",
    tag = "funding",
    security(("bearer_auth" = [])),
    params(
        ("id" = Uuid, Path, description = "Agreement id"),
        ("line_id" = Uuid, Path, description = "Agreement line id")
    ),
    responses(
        (status = 204, description = "Agreement line deleted"),
        (status = 401, description = "Missing or invalid access token", body = ErrorBody),
        (status = 403, description = "Missing finance.write capability", body = ErrorBody),
        (status = 404, description = "Agreement line not found", body = ErrorBody)
    )
)]
pub async fn delete_line(
    State(state): State<AppState>,
    auth: AuthContext,
    Path((id, line_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, AppError> {
    auth.require(Capability::FinanceWrite)?;
    funding::delete_line(&state.db, auth.organisation_id, id, line_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /api/v1/agreements/{id}/finalise`
#[utoipa::path(
    post,
    path = "/api/v1/agreements/{id}/finalise",
    tag = "funding",
    security(("bearer_auth" = [])),
    params(("id" = Uuid, Path, description = "Agreement id")),
    responses(
        (status = 200, description = "Agreement finalised and draft shifts generated", body = AgreementFinalised),
        (status = 401, description = "Missing or invalid access token", body = ErrorBody),
        (status = 403, description = "Missing finance.write or rosters.write capability", body = ErrorBody),
        (status = 404, description = "Agreement not found", body = ErrorBody),
        (status = 409, description = "Agreement already finalised", body = ErrorBody),
        (status = 422, description = "Agreement is not ready to finalise", body = ErrorBody)
    )
)]
pub async fn finalise(
    State(state): State<AppState>,
    auth: AuthContext,
    Path(id): Path<Uuid>,
) -> Result<Json<AgreementFinalised>, AppError> {
    auth.require(Capability::FinanceWrite)?;
    auth.require(Capability::RostersWrite)?;
    let (agreement, generated_shifts) =
        funding::finalise_agreement(&state.db, auth.organisation_id, id).await?;
    Ok(Json(AgreementFinalised {
        agreement,
        generated_shifts,
    }))
}

fn trimmed(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}
