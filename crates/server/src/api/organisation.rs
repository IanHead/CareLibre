//! Organisation profile and branding API.
//!
//! Desktop and mobile clients fetch the organisation name and logo after
//! sign-in. The logo is cached by SHA-256 ETag.

use axum::{
    extract::{Multipart, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use serde_json::json;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::{
    auth::extractor::AuthContext,
    error::{AppError, ErrorBody},
    organisation::{
        delete_logo, load_logo, load_profile, save_logo, update_name, validate_logo, LogoInfo,
        OrganisationProfileDto,
    },
    permissions::Capability,
    state::AppState,
};

#[derive(Debug, Deserialize, ToSchema)]
pub struct OrganisationNameWrite {
    pub name: String,
}

/// `GET /api/v1/organisation`
#[utoipa::path(
    get,
    path = "/api/v1/organisation",
    tag = "organisation",
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Current organisation profile and branding", body = OrganisationProfileDto),
        (status = 401, description = "Missing or invalid access token", body = ErrorBody)
    )
)]
pub async fn get_profile(
    State(state): State<AppState>,
    auth: AuthContext,
) -> Result<Json<OrganisationProfileDto>, AppError> {
    let profile = load_profile(&state.db, auth.organisation_id)
        .await?
        .ok_or_else(|| AppError::not_found("organisation not found"))?;
    Ok(Json(profile))
}

/// `PATCH /api/v1/organisation`
#[utoipa::path(
    patch,
    path = "/api/v1/organisation",
    tag = "organisation",
    security(("bearer_auth" = [])),
    request_body = OrganisationNameWrite,
    responses(
        (status = 200, description = "Organisation name updated", body = OrganisationProfileDto),
        (status = 401, description = "Missing or invalid access token", body = ErrorBody),
        (status = 403, description = "Missing settings.manage capability", body = ErrorBody),
        (status = 422, description = "Validation failed", body = ErrorBody)
    )
)]
pub async fn update_profile(
    State(state): State<AppState>,
    auth: AuthContext,
    Json(payload): Json<OrganisationNameWrite>,
) -> Result<Json<OrganisationProfileDto>, AppError> {
    auth.require(Capability::SettingsManage)?;
    let name = payload.name.trim().to_owned();
    if name.is_empty() || name.chars().count() > 200 {
        return Err(AppError::validation(
            "name must be between 1 and 200 characters",
        ));
    }

    let changed = update_name(&state.db, auth.organisation_id, &name).await?;
    if changed == 0 {
        return Err(AppError::not_found("organisation not found"));
    }

    write_audit(
        &state,
        auth.organisation_id,
        auth.user_id,
        "organisation.renamed",
        json!({ "name": name }),
    )
    .await;

    let profile = load_profile(&state.db, auth.organisation_id)
        .await?
        .ok_or_else(|| AppError::not_found("organisation not found"))?;
    Ok(Json(profile))
}

/// `GET /api/v1/organisation/logo`
#[utoipa::path(
    get,
    path = "/api/v1/organisation/logo",
    tag = "organisation",
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Logo image bytes", content_type = "image/*"),
        (status = 304, description = "Logo unchanged (ETag matched)"),
        (status = 401, description = "Missing or invalid access token", body = ErrorBody),
        (status = 404, description = "No logo has been uploaded", body = ErrorBody)
    )
)]
pub async fn get_logo(
    State(state): State<AppState>,
    auth: AuthContext,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let logo = load_logo(&state.db, auth.organisation_id)
        .await?
        .ok_or_else(|| AppError::not_found("this organisation has no logo"))?;

    let etag = format!("\"{}\"", logo.sha256);
    if headers
        .get(header::IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value == etag)
    {
        return Ok(StatusCode::NOT_MODIFIED.into_response());
    }

    let mut response_headers = HeaderMap::new();
    response_headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(&logo.content_type)
            .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream")),
    );
    response_headers.insert(
        header::ETAG,
        HeaderValue::from_str(&etag)
            .map_err(|error| AppError::Internal(anyhow::anyhow!("invalid logo etag: {error}")))?,
    );
    response_headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, max-age=3600"),
    );
    response_headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );

    Ok((response_headers, logo.bytes).into_response())
}

/// `PUT /api/v1/organisation/logo`
#[utoipa::path(
    put,
    path = "/api/v1/organisation/logo",
    tag = "organisation",
    security(("bearer_auth" = [])),
    request_body(content = String, content_type = "multipart/form-data", description = "Multipart field `logo` containing a PNG, JPEG, WebP or GIF image (max 2 MB)"),
    responses(
        (status = 200, description = "Logo uploaded", body = LogoInfo),
        (status = 401, description = "Missing or invalid access token", body = ErrorBody),
        (status = 403, description = "Missing settings.manage capability", body = ErrorBody),
        (status = 422, description = "Validation failed", body = ErrorBody)
    )
)]
pub async fn upload_logo(
    State(state): State<AppState>,
    auth: AuthContext,
    mut multipart: Multipart,
) -> Result<Json<LogoInfo>, AppError> {
    auth.require(Capability::SettingsManage)?;

    let mut bytes = None;
    let mut filename = None;
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|error| AppError::bad_request(format!("the upload could not be read: {error}")))?
    {
        if matches!(field.name(), Some("logo" | "file")) {
            filename = field.file_name().map(sanitise_filename);
            bytes = Some(
                field
                    .bytes()
                    .await
                    .map_err(|error| {
                        AppError::bad_request(format!("the logo could not be read: {error}"))
                    })?
                    .to_vec(),
            );
            break;
        }
    }

    let bytes = bytes.ok_or_else(|| AppError::validation("choose a logo file to upload"))?;
    let content_type = validate_logo(&bytes).map_err(AppError::validation)?;

    let logo = save_logo(
        &state.db,
        auth.organisation_id,
        auth.user_id,
        &bytes,
        content_type,
        filename.as_deref(),
    )
    .await?;

    write_audit(
        &state,
        auth.organisation_id,
        auth.user_id,
        "organisation.logo_uploaded",
        json!({
            "content_type": content_type,
            "filename": filename,
            "bytes": bytes.len(),
            "sha256": logo.sha256,
        }),
    )
    .await;

    Ok(Json(logo))
}

/// `DELETE /api/v1/organisation/logo`
#[utoipa::path(
    delete,
    path = "/api/v1/organisation/logo",
    tag = "organisation",
    security(("bearer_auth" = [])),
    responses(
        (status = 204, description = "Logo removed"),
        (status = 401, description = "Missing or invalid access token", body = ErrorBody),
        (status = 403, description = "Missing settings.manage capability", body = ErrorBody),
        (status = 404, description = "No logo to remove", body = ErrorBody)
    )
)]
pub async fn delete_logo_handler(
    State(state): State<AppState>,
    auth: AuthContext,
) -> Result<StatusCode, AppError> {
    auth.require(Capability::SettingsManage)?;
    let removed = delete_logo(&state.db, auth.organisation_id).await?;
    if removed == 0 {
        return Err(AppError::not_found("this organisation has no logo"));
    }

    write_audit(
        &state,
        auth.organisation_id,
        auth.user_id,
        "organisation.logo_deleted",
        json!({}),
    )
    .await;

    Ok(StatusCode::NO_CONTENT)
}

fn sanitise_filename(name: &str) -> String {
    let base = std::path::Path::new(name)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("logo");
    base.chars().take(255).collect()
}

async fn write_audit(
    state: &AppState,
    organisation_id: Uuid,
    actor_user_id: Uuid,
    action: &str,
    metadata: serde_json::Value,
) {
    if let Err(error) = sqlx::query(
        "INSERT INTO audit_log
            (organisation_id, actor_user_id, action, entity_type, entity_id, metadata)
         VALUES ($1, $2, $3, 'organisation', $1, $4)",
    )
    .bind(organisation_id)
    .bind(actor_user_id)
    .bind(action)
    .bind(metadata)
    .execute(&state.db)
    .await
    {
        tracing::warn!(error = %error, action, "audit write failed");
    }
}
