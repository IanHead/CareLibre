//! Authentication and session endpoints.
//!
//! Design notes:
//! - Passwords are hashed with Argon2id.
//! - Access tokens are short-lived HS256 JWTs.
//! - Refresh tokens are opaque random strings, stored only as SHA-256 hashes,
//!   rotated on every use, and revocable.

use std::str::FromStr;

use axum::{
    extract::State,
    http::{header, HeaderMap, StatusCode},
    Json,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::{PgConnection, PgPool};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::{
    auth::extractor::AuthContext,
    auth::{password, refresh, role::Role, token},
    error::{AppError, ErrorBody},
    state::AppState,
};

// ---------------------------------------------------------------------------
// Request / response bodies
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, ToSchema)]
pub struct RegisterRequest {
    /// Display name of the organisation being created.
    pub organisation_name: String,
    /// URL-safe, lowercase identifier (3-63 chars, `a-z0-9-`).
    pub organisation_slug: String,
    pub full_name: String,
    pub email: String,
    /// Minimum 12 characters.
    pub password: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct LoginRequest {
    pub email: String,
    pub password: String,
    /// Required only when the account belongs to more than one organisation.
    pub organisation_slug: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct RefreshRequest {
    pub refresh_token: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct LogoutRequest {
    pub refresh_token: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AuthResponse {
    pub access_token: String,
    pub refresh_token: String,
    pub token_type: String,
    /// Access token lifetime in seconds.
    pub expires_in: i64,
    pub user: UserProfile,
    pub organisation: OrganisationProfile,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct UserProfile {
    pub id: Uuid,
    pub email: String,
    pub full_name: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct OrganisationProfile {
    pub id: Uuid,
    pub slug: String,
    pub name: String,
    pub edition: String,
    pub role: Role,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct MeResponse {
    pub user: UserProfile,
    pub organisation: OrganisationProfile,
}

// ---------------------------------------------------------------------------
// Row types
// ---------------------------------------------------------------------------

#[derive(Debug, sqlx::FromRow)]
struct UserRow {
    id: Uuid,
    password_hash: String,
    is_active: bool,
}

#[derive(Debug, sqlx::FromRow)]
struct MembershipRow {
    organisation_id: Uuid,
    organisation_slug: String,
    role: String,
}

#[derive(Debug, sqlx::FromRow)]
struct ProfileRow {
    email: String,
    full_name: String,
    slug: String,
    name: String,
    edition: String,
}

#[derive(Debug, sqlx::FromRow)]
struct RefreshRow {
    id: Uuid,
    user_id: Uuid,
    organisation_id: Uuid,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// Create an organisation and its first (owner) account.
#[utoipa::path(
    post,
    path = "/api/v1/auth/register",
    tag = "auth",
    request_body = RegisterRequest,
    responses(
        (status = 201, description = "Organisation created and session started", body = AuthResponse),
        (status = 409, description = "Email or organisation slug already in use", body = ErrorBody),
        (status = 422, description = "Validation failed", body = ErrorBody)
    )
)]
pub async fn register(
    State(state): State<AppState>,
    Json(payload): Json<RegisterRequest>,
) -> Result<(StatusCode, Json<AuthResponse>), AppError> {
    let organisation_name = payload.organisation_name.trim().to_owned();
    let slug = payload.organisation_slug.trim().to_ascii_lowercase();
    let full_name = payload.full_name.trim().to_owned();
    let email = payload.email.trim().to_ascii_lowercase();

    validate_organisation_name(&organisation_name)?;
    validate_slug(&slug)?;
    validate_full_name(&full_name)?;
    validate_email(&email)?;
    validate_password(&payload.password)?;

    let password_hash = password::hash_password(payload.password).await?;

    let mut tx = state.db.begin().await?;

    if exists(
        &mut tx,
        "SELECT EXISTS(SELECT 1 FROM organisations WHERE slug = $1)",
        &slug,
    )
    .await?
    {
        return Err(AppError::conflict(
            "that organisation slug is already in use",
        ));
    }

    if exists(
        &mut tx,
        "SELECT EXISTS(SELECT 1 FROM users WHERE lower(email) = $1)",
        &email,
    )
    .await?
    {
        return Err(AppError::conflict(
            "an account with that email address already exists",
        ));
    }

    let organisation_id = Uuid::now_v7();
    let user_id = Uuid::now_v7();

    sqlx::query(
        "INSERT INTO organisations (id, slug, name, edition) VALUES ($1, $2, $3, 'community')",
    )
    .bind(organisation_id)
    .bind(&slug)
    .bind(&organisation_name)
    .execute(&mut *tx)
    .await?;

    sqlx::query("INSERT INTO users (id, email, password_hash, full_name) VALUES ($1, $2, $3, $4)")
        .bind(user_id)
        .bind(&email)
        .bind(&password_hash)
        .bind(&full_name)
        .execute(&mut *tx)
        .await?;

    sqlx::query(
        "INSERT INTO organisation_users (organisation_id, user_id, role) VALUES ($1, $2, 'owner')",
    )
    .bind(organisation_id)
    .bind(user_id)
    .execute(&mut *tx)
    .await?;

    write_audit(
        &mut tx,
        Some(organisation_id),
        Some(user_id),
        "auth.register",
        "organisation",
        Some(organisation_id),
        serde_json::json!({ "slug": slug, "email": email }),
    )
    .await?;

    tx.commit().await?;

    let response = establish_session(&state, user_id, organisation_id, Role::Owner, None).await?;
    Ok((StatusCode::CREATED, Json(response)))
}

/// Exchange email + password for an access/refresh token pair.
#[utoipa::path(
    post,
    path = "/api/v1/auth/login",
    tag = "auth",
    request_body = LoginRequest,
    responses(
        (status = 200, description = "Authenticated", body = AuthResponse),
        (status = 401, description = "Invalid credentials", body = ErrorBody),
        (status = 403, description = "Account disabled or not a member of any organisation", body = ErrorBody)
    )
)]
pub async fn login(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<LoginRequest>,
) -> Result<Json<AuthResponse>, AppError> {
    let email = payload.email.trim().to_ascii_lowercase();
    let organisation_slug = payload
        .organisation_slug
        .as_deref()
        .map(|slug| slug.trim().to_ascii_lowercase())
        .filter(|slug| !slug.is_empty());

    let user = sqlx::query_as::<_, UserRow>(
        "SELECT id, password_hash, is_active FROM users WHERE lower(email) = $1",
    )
    .bind(&email)
    .fetch_optional(&state.db)
    .await?;

    // Same response for unknown user and wrong password to avoid account
    // enumeration.
    let Some(user) = user else {
        return Err(AppError::unauthorized("invalid email or password"));
    };

    if !user.is_active {
        return Err(AppError::forbidden("this account has been disabled"));
    }

    if !password::verify_password(payload.password, user.password_hash).await {
        return Err(AppError::unauthorized("invalid email or password"));
    }

    let memberships = sqlx::query_as::<_, MembershipRow>(
        "SELECT o.id AS organisation_id, o.slug AS organisation_slug, m.role
         FROM organisation_users m
         JOIN organisations o ON o.id = m.organisation_id
         WHERE m.user_id = $1
         ORDER BY o.name",
    )
    .bind(user.id)
    .fetch_all(&state.db)
    .await?;

    let membership = select_membership(memberships, organisation_slug.as_deref())?;

    sqlx::query("UPDATE users SET last_login_at = now() WHERE id = $1")
        .bind(user.id)
        .execute(&state.db)
        .await?;

    let role = Role::from_str(&membership.role)
        .map_err(|_| AppError::Internal(anyhow::anyhow!("invalid role stored in database")))?;

    let response = establish_session(
        &state,
        user.id,
        membership.organisation_id,
        role,
        user_agent(&headers),
    )
    .await?;

    Ok(Json(response))
}

/// Rotate a refresh token and issue a new token pair.
#[utoipa::path(
    post,
    path = "/api/v1/auth/refresh",
    tag = "auth",
    request_body = RefreshRequest,
    responses(
        (status = 200, description = "Token rotated", body = AuthResponse),
        (status = 401, description = "Refresh token invalid, expired or already used", body = ErrorBody)
    )
)]
pub async fn refresh(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<RefreshRequest>,
) -> Result<Json<AuthResponse>, AppError> {
    let token_hash = refresh::hash(&payload.refresh_token);

    let mut tx = state.db.begin().await?;

    let session = sqlx::query_as::<_, RefreshRow>(
        "SELECT id, user_id, organisation_id
         FROM auth_sessions
         WHERE refresh_token_hash = $1
           AND revoked_at IS NULL
           AND expires_at > now()
         FOR UPDATE",
    )
    .bind(&token_hash)
    .fetch_optional(&mut *tx)
    .await?;

    let Some(session) = session else {
        return Err(AppError::unauthorized(
            "refresh token is invalid, expired or already used",
        ));
    };

    sqlx::query("UPDATE auth_sessions SET revoked_at = now() WHERE id = $1")
        .bind(session.id)
        .execute(&mut *tx)
        .await?;

    let role = sqlx::query_scalar::<_, String>(
        "SELECT role FROM organisation_users WHERE user_id = $1 AND organisation_id = $2",
    )
    .bind(session.user_id)
    .bind(session.organisation_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| AppError::forbidden("this account no longer belongs to the organisation"))?;

    tx.commit().await?;

    let role = Role::from_str(&role)
        .map_err(|_| AppError::Internal(anyhow::anyhow!("invalid role stored in database")))?;

    let response = establish_session(
        &state,
        session.user_id,
        session.organisation_id,
        role,
        user_agent(&headers),
    )
    .await?;

    Ok(Json(response))
}

/// Revoke a refresh token. Idempotent.
#[utoipa::path(
    post,
    path = "/api/v1/auth/logout",
    tag = "auth",
    request_body = LogoutRequest,
    responses((status = 204, description = "Session revoked"))
)]
pub async fn logout(
    State(state): State<AppState>,
    Json(payload): Json<LogoutRequest>,
) -> Result<StatusCode, AppError> {
    sqlx::query(
        "UPDATE auth_sessions SET revoked_at = now()
         WHERE refresh_token_hash = $1 AND revoked_at IS NULL",
    )
    .bind(refresh::hash(&payload.refresh_token))
    .execute(&state.db)
    .await?;

    Ok(StatusCode::NO_CONTENT)
}

/// The authenticated user and their active organisation.
#[utoipa::path(
    get,
    path = "/api/v1/me",
    tag = "auth",
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Current identity", body = MeResponse),
        (status = 401, description = "Missing or invalid access token", body = ErrorBody)
    )
)]
pub async fn me(
    State(state): State<AppState>,
    auth: AuthContext,
) -> Result<Json<MeResponse>, AppError> {
    let profile = load_profile(&state.db, auth.user_id, auth.organisation_id).await?;

    Ok(Json(MeResponse {
        user: UserProfile {
            id: auth.user_id,
            email: profile.email,
            full_name: profile.full_name,
        },
        organisation: OrganisationProfile {
            id: auth.organisation_id,
            slug: profile.slug,
            name: profile.name,
            edition: profile.edition,
            role: auth.role,
        },
    }))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

async fn establish_session(
    state: &AppState,
    user_id: Uuid,
    organisation_id: Uuid,
    role: Role,
    request_user_agent: Option<String>,
) -> Result<AuthResponse, AppError> {
    let (refresh_token, refresh_token_hash) = generate_refresh_token();

    let refresh_ttl = i64::try_from(state.config.refresh_token_ttl.as_secs()).unwrap_or(2_592_000);
    let expires_at = Utc::now() + chrono::Duration::seconds(refresh_ttl);

    sqlx::query(
        "INSERT INTO auth_sessions
            (id, user_id, organisation_id, refresh_token_hash, user_agent, expires_at)
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(Uuid::now_v7())
    .bind(user_id)
    .bind(organisation_id)
    .bind(&refresh_token_hash)
    .bind(request_user_agent.as_deref())
    .bind(expires_at)
    .execute(&state.db)
    .await?;

    let profile = load_profile(&state.db, user_id, organisation_id).await?;

    let access_token = token::issue_access_token(
        &state.config.jwt_secret,
        user_id,
        organisation_id,
        role.as_str(),
        state.config.access_token_ttl,
    )?;

    Ok(AuthResponse {
        access_token,
        refresh_token,
        token_type: "Bearer".to_owned(),
        expires_in: i64::try_from(state.config.access_token_ttl.as_secs()).unwrap_or(900),
        user: UserProfile {
            id: user_id,
            email: profile.email,
            full_name: profile.full_name,
        },
        organisation: OrganisationProfile {
            id: organisation_id,
            slug: profile.slug,
            name: profile.name,
            edition: profile.edition,
            role,
        },
    })
}

async fn load_profile(
    db: &PgPool,
    user_id: Uuid,
    organisation_id: Uuid,
) -> Result<ProfileRow, AppError> {
    sqlx::query_as::<_, ProfileRow>(
        "SELECT u.email, u.full_name, o.slug, o.name, o.edition
         FROM users u
         JOIN organisations o ON o.id = $2
         WHERE u.id = $1",
    )
    .bind(user_id)
    .bind(organisation_id)
    .fetch_optional(db)
    .await?
    .ok_or_else(|| AppError::not_found("user or organisation no longer exists"))
}

async fn exists(conn: &mut PgConnection, sql: &str, value: &str) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar::<_, bool>(sql)
        .bind(value)
        .fetch_one(conn)
        .await
}

#[allow(clippy::too_many_arguments)]
async fn write_audit(
    conn: &mut PgConnection,
    organisation_id: Option<Uuid>,
    actor_user_id: Option<Uuid>,
    action: &str,
    entity_type: &str,
    entity_id: Option<Uuid>,
    metadata: serde_json::Value,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO audit_log
            (organisation_id, actor_user_id, action, entity_type, entity_id, metadata)
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(organisation_id)
    .bind(actor_user_id)
    .bind(action)
    .bind(entity_type)
    .bind(entity_id)
    .bind(metadata)
    .execute(conn)
    .await?;

    Ok(())
}

fn select_membership(
    memberships: Vec<MembershipRow>,
    organisation_slug: Option<&str>,
) -> Result<MembershipRow, AppError> {
    if let Some(slug) = organisation_slug {
        return memberships
            .into_iter()
            .find(|membership| membership.organisation_slug == slug)
            .ok_or_else(|| AppError::forbidden("account is not a member of that organisation"));
    }

    match memberships.len() {
        0 => Err(AppError::forbidden(
            "account is not a member of any organisation",
        )),
        1 => memberships
            .into_iter()
            .next()
            .ok_or_else(|| AppError::Internal(anyhow::anyhow!("membership vanished"))),
        _ => Err(AppError::bad_request(
            "account belongs to multiple organisations; provide organisation_slug",
        )),
    }
}

fn generate_refresh_token() -> (String, String) {
    refresh::generate()
}

fn user_agent(headers: &HeaderMap) -> Option<String> {
    headers
        .get(header::USER_AGENT)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.chars().take(512).collect())
}

fn validate_organisation_name(name: &str) -> Result<(), AppError> {
    if name.is_empty() || name.chars().count() > 120 {
        return Err(AppError::validation(
            "organisation_name must be between 1 and 120 characters",
        ));
    }
    Ok(())
}

fn validate_slug(slug: &str) -> Result<(), AppError> {
    let length_ok = (3..=63).contains(&slug.len());
    let chars_ok = slug
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    let edges_ok = slug
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && slug
            .chars()
            .last()
            .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit());

    if !(length_ok && chars_ok && edges_ok) {
        return Err(AppError::validation(
            "organisation_slug must be 3-63 chars of a-z, 0-9 or '-', starting and ending with a letter or digit",
        ));
    }
    Ok(())
}

fn validate_full_name(name: &str) -> Result<(), AppError> {
    if name.is_empty() || name.chars().count() > 200 {
        return Err(AppError::validation(
            "full_name must be between 1 and 200 characters",
        ));
    }
    Ok(())
}

fn validate_email(email: &str) -> Result<(), AppError> {
    let valid = email.len() <= 320 && {
        match email.split_once('@') {
            Some((local, domain)) => {
                !local.is_empty() && domain.contains('.') && !domain.starts_with('.')
            }
            None => false,
        }
    };

    if !valid {
        return Err(AppError::validation("email is not a valid address"));
    }
    Ok(())
}

fn validate_password(password: &str) -> Result<(), AppError> {
    if password.chars().count() < 12 {
        return Err(AppError::validation(
            "password must be at least 12 characters",
        ));
    }
    if password.len() > 1024 {
        return Err(AppError::validation("password is too long"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_validation_accepts_reasonable_values() {
        for slug in ["abc", "care-libre", "a1b2c3", "a-b-c"] {
            assert!(validate_slug(slug).is_ok(), "{slug} should be valid");
        }
    }

    #[test]
    fn slug_validation_rejects_bad_values() {
        for slug in [
            "",
            "ab",
            "-leading",
            "trailing-",
            "UPPER",
            "with space",
            "under_score",
        ] {
            assert!(validate_slug(slug).is_err(), "{slug} should be invalid");
        }
    }

    #[test]
    fn email_validation_is_lenient_but_sane() {
        assert!(validate_email("a@b.co").is_ok());
        assert!(validate_email("no-at-sign").is_err());
        assert!(validate_email("@example.com").is_err());
        assert!(validate_email("user@localhost").is_err());
    }

    #[test]
    fn password_requires_twelve_characters() {
        assert!(validate_password("short").is_err());
        assert!(validate_password("twelve-chars").is_ok());
    }

    #[test]
    fn selects_the_only_membership() {
        let membership = MembershipRow {
            organisation_id: Uuid::now_v7(),
            organisation_slug: "acme".to_owned(),
            role: "owner".to_owned(),
        };
        let selected = select_membership(vec![membership], None).unwrap();
        assert_eq!(selected.organisation_slug, "acme");
    }

    #[test]
    fn requires_slug_when_multiple_memberships() {
        let memberships = vec![
            MembershipRow {
                organisation_id: Uuid::now_v7(),
                organisation_slug: "one".to_owned(),
                role: "staff".to_owned(),
            },
            MembershipRow {
                organisation_id: Uuid::now_v7(),
                organisation_slug: "two".to_owned(),
                role: "staff".to_owned(),
            },
        ];
        let err = select_membership(memberships, None).unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)));
    }
}
