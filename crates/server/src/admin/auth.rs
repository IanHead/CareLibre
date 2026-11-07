//! Browser session handling for the admin area.
//!
//! The admin UI reuses the `auth_sessions` table, but stores the opaque refresh
//! token in an HttpOnly cookie instead of returning it to JavaScript. Each page
//! load resolves the cookie to a live session and checks the member's role.

use std::str::FromStr;

use axum::{
    extract::{FromRequestParts, State},
    http::{header, request::Parts, HeaderMap, StatusCode},
    response::{Html, IntoResponse, Redirect, Response},
    Form,
};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use chrono::Utc;
use serde::Deserialize;
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

use crate::{
    admin::html,
    auth::{password, refresh, role::Role},
    state::AppState,
};

pub const COOKIE_NAME: &str = "carelibre_admin";

/// A signed-in browser session with enough context to render the chrome.
#[derive(Debug, Clone)]
pub struct AdminSession {
    pub session_id: Uuid,
    pub user_id: Uuid,
    pub organisation_id: Uuid,
    pub role: Role,
    pub email: String,
    pub full_name: String,
    pub organisation_name: String,
    pub organisation_slug: String,
}

#[derive(Debug, FromRow)]
struct SessionRow {
    session_id: Uuid,
    user_id: Uuid,
    organisation_id: Uuid,
    role: String,
    email: String,
    full_name: String,
    organisation_name: String,
    organisation_slug: String,
}

pub enum AdminRejection {
    Unauthenticated,
    Forbidden { role: String },
    Internal,
}

impl IntoResponse for AdminRejection {
    fn into_response(self) -> Response {
        match self {
            Self::Unauthenticated => Redirect::to("/admin/login").into_response(),
            Self::Forbidden { role } => {
                let body = html::error_page(
                    "Access denied",
                    &format!(
                        "Your role ({role}) cannot access the admin area. \
                         Ask an owner or administrator to change your role."
                    ),
                );
                (StatusCode::FORBIDDEN, Html(body)).into_response()
            }
            Self::Internal => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Html(html::error_page(
                    "Server error",
                    "The admin area could not be loaded. Check the server logs.",
                )),
            )
                .into_response(),
        }
    }
}

impl FromRequestParts<AppState> for AdminSession {
    type Rejection = AdminRejection;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let jar = CookieJar::from_headers(&parts.headers);
        let token = jar
            .get(COOKIE_NAME)
            .map(|cookie| cookie.value().to_owned())
            .ok_or(AdminRejection::Unauthenticated)?;

        let row = sqlx::query_as::<_, SessionRow>(
            "SELECT s.id AS session_id,
                    s.user_id,
                    s.organisation_id,
                    m.role,
                    u.email,
                    u.full_name,
                    o.name AS organisation_name,
                    o.slug AS organisation_slug
             FROM auth_sessions s
             JOIN users u ON u.id = s.user_id
             JOIN organisations o ON o.id = s.organisation_id
             JOIN organisation_members m
               ON m.user_id = s.user_id AND m.organisation_id = s.organisation_id
             WHERE s.refresh_token_hash = $1
               AND s.revoked_at IS NULL
               AND s.expires_at > now()
               AND u.is_active",
        )
        .bind(refresh::hash(&token))
        .fetch_optional(&state.db)
        .await
        .map_err(|err| {
            tracing::error!(error = %err, "admin session lookup failed");
            AdminRejection::Internal
        })?
        .ok_or(AdminRejection::Unauthenticated)?;

        let role = Role::from_str(&row.role).map_err(|_| AdminRejection::Internal)?;
        if !role.can_manage_organisation() {
            return Err(AdminRejection::Forbidden {
                role: role.to_string(),
            });
        }

        Ok(Self {
            session_id: row.session_id,
            user_id: row.user_id,
            organisation_id: row.organisation_id,
            role,
            email: row.email,
            full_name: row.full_name,
            organisation_name: row.organisation_name,
            organisation_slug: row.organisation_slug,
        })
    }
}

impl AdminSession {
    #[must_use]
    pub fn who(&self) -> html::Who<'_> {
        html::Who {
            email: &self.email,
            organisation: &self.organisation_name,
            role: self.role.as_str(),
        }
    }
}

// ---------------------------------------------------------------------------
// Login / logout
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct LoginForm {
    pub email: String,
    pub password: String,
    pub organisation_slug: Option<String>,
}

pub async fn login_form() -> Html<String> {
    Html(login_page(None))
}

pub async fn login(
    State(state): State<AppState>,
    jar: CookieJar,
    headers: HeaderMap,
    Form(form): Form<LoginForm>,
) -> Response {
    let user_agent = headers
        .get(header::USER_AGENT)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.chars().take(512).collect::<String>());

    match authenticate(&state, &form, user_agent).await {
        Ok(token) => {
            let cookie = Cookie::build((COOKIE_NAME, token))
                .http_only(true)
                .same_site(SameSite::Lax)
                .path("/")
                .secure(state.config.environment.is_production())
                .build();

            (jar.add(cookie), Redirect::to("/admin")).into_response()
        }
        Err(LoginFailure::Invalid(message)) => {
            (StatusCode::UNAUTHORIZED, Html(login_page(Some(&message)))).into_response()
        }
        Err(LoginFailure::Forbidden(role)) => {
            let body = html::error_page(
                "Access denied",
                &format!("Your role ({role}) cannot access the admin area."),
            );
            (StatusCode::FORBIDDEN, Html(body)).into_response()
        }
        Err(LoginFailure::Internal(err)) => {
            tracing::error!(error = %err, "admin login failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Html(html::error_page(
                    "Server error",
                    "Sign-in could not be completed. Check the server logs.",
                )),
            )
                .into_response()
        }
    }
}

pub async fn logout(State(state): State<AppState>, jar: CookieJar) -> Response {
    if let Some(cookie) = jar.get(COOKIE_NAME) {
        let hash = refresh::hash(cookie.value());
        if let Err(err) = sqlx::query(
            "UPDATE auth_sessions SET revoked_at = now()
             WHERE refresh_token_hash = $1 AND revoked_at IS NULL",
        )
        .bind(hash)
        .execute(&state.db)
        .await
        {
            tracing::warn!(error = %err, "failed to revoke admin session");
        }
    }

    let jar = jar.remove(Cookie::build(COOKIE_NAME).path("/").build());
    (jar, Redirect::to("/admin/login")).into_response()
}

#[derive(Debug)]
enum LoginFailure {
    Invalid(String),
    Forbidden(String),
    Internal(sqlx::Error),
}

#[derive(Debug, FromRow)]
struct UserRow {
    id: Uuid,
    password_hash: String,
    is_active: bool,
}

#[derive(Debug, FromRow)]
struct MembershipRow {
    organisation_id: Uuid,
    organisation_slug: String,
    role: String,
}

async fn authenticate(
    state: &AppState,
    form: &LoginForm,
    user_agent: Option<String>,
) -> Result<String, LoginFailure> {
    let email = form.email.trim().to_ascii_lowercase();
    let organisation_slug = form
        .organisation_slug
        .as_deref()
        .map(|slug| slug.trim().to_ascii_lowercase())
        .filter(|slug| !slug.is_empty());

    if email.is_empty() || form.password.is_empty() {
        return Err(LoginFailure::Invalid(
            "Enter your email and password.".to_owned(),
        ));
    }

    let user = sqlx::query_as::<_, UserRow>(
        "SELECT id, password_hash, is_active FROM users WHERE lower(email) = $1",
    )
    .bind(&email)
    .fetch_optional(&state.db)
    .await
    .map_err(LoginFailure::Internal)?;

    let Some(user) = user else {
        return Err(LoginFailure::Invalid(
            "Invalid email or password.".to_owned(),
        ));
    };

    if !user.is_active {
        return Err(LoginFailure::Invalid(
            "This account has been disabled.".to_owned(),
        ));
    }

    if !password::verify_password(form.password.clone(), user.password_hash).await {
        return Err(LoginFailure::Invalid(
            "Invalid email or password.".to_owned(),
        ));
    }

    let memberships = sqlx::query_as::<_, MembershipRow>(
        "SELECT o.id AS organisation_id, o.slug AS organisation_slug, m.role
         FROM organisation_members m
         JOIN organisations o ON o.id = m.organisation_id
         WHERE m.user_id = $1
         ORDER BY o.name",
    )
    .bind(user.id)
    .fetch_all(&state.db)
    .await
    .map_err(LoginFailure::Internal)?;

    let membership = select_membership(memberships, organisation_slug.as_deref())?;

    let role = Role::from_str(&membership.role)
        .map_err(|_| LoginFailure::Internal(sqlx::Error::Protocol("unknown role".to_owned())))?;

    if !role.can_manage_organisation() {
        return Err(LoginFailure::Forbidden(role.to_string()));
    }

    let (token, token_hash) = refresh::generate();
    let ttl = i64::try_from(state.config.refresh_token_ttl.as_secs()).unwrap_or(2_592_000);

    sqlx::query(
        "INSERT INTO auth_sessions
            (id, user_id, organisation_id, refresh_token_hash, user_agent, expires_at)
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(Uuid::now_v7())
    .bind(user.id)
    .bind(membership.organisation_id)
    .bind(&token_hash)
    .bind(user_agent)
    .bind(Utc::now() + chrono::Duration::seconds(ttl))
    .execute(&state.db)
    .await
    .map_err(LoginFailure::Internal)?;

    let _ = sqlx::query("UPDATE users SET last_login_at = now() WHERE id = $1")
        .bind(user.id)
        .execute(&state.db)
        .await;

    Ok(token)
}

fn select_membership(
    memberships: Vec<MembershipRow>,
    organisation_slug: Option<&str>,
) -> Result<MembershipRow, LoginFailure> {
    if let Some(slug) = organisation_slug {
        return memberships
            .into_iter()
            .find(|membership| membership.organisation_slug == slug)
            .ok_or_else(|| {
                LoginFailure::Invalid("No membership for that organisation.".to_owned())
            });
    }

    match memberships.len() {
        0 => Err(LoginFailure::Invalid(
            "This account does not belong to any organisation.".to_owned(),
        )),
        1 => memberships
            .into_iter()
            .next()
            .ok_or_else(|| LoginFailure::Invalid("No membership found.".to_owned())),
        _ => Err(LoginFailure::Invalid(
            "This account belongs to several organisations; enter an organisation slug.".to_owned(),
        )),
    }
}

fn login_page(message: Option<&str>) -> String {
    let alert = message.map_or_else(String::new, |message| {
        format!("<div class=\"alert\">{}</div>", html::escape(message))
    });

    let body = format!(
        "<div class=\"page-head\"><div><h1>Sign in</h1>\
         <p class=\"muted\">Administration requires an owner or administrator account.</p></div></div>\
         <form class=\"form\" method=\"post\" action=\"/admin/login\">\
         {alert}\
         <label for=\"organisation_slug\">Organisation slug <span class=\"muted\">(optional)</span></label>\
         <input id=\"organisation_slug\" name=\"organisation_slug\" type=\"text\" autocomplete=\"organization\" placeholder=\"example-care\">\
         <label for=\"email\">Email</label>\
         <input id=\"email\" name=\"email\" type=\"email\" autocomplete=\"username\" required>\
         <label for=\"password\">Password</label>\
         <input id=\"password\" name=\"password\" type=\"password\" autocomplete=\"current-password\" required>\
         <button type=\"submit\">Sign in</button>\
         </form>"
    );

    html::layout("Sign in", "", None, &body)
}

/// Best-effort audit helper shared by the admin pages.
pub async fn write_audit(
    db: &PgPool,
    organisation_id: Uuid,
    actor_user_id: Uuid,
    action: &str,
    entity_type: &str,
    entity_id: Option<Uuid>,
) {
    if let Err(err) = sqlx::query(
        "INSERT INTO audit_log (organisation_id, actor_user_id, action, entity_type, entity_id)
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(organisation_id)
    .bind(actor_user_id)
    .bind(action)
    .bind(entity_type)
    .bind(entity_id)
    .execute(db)
    .await
    {
        tracing::warn!(error = %err, "failed to write audit log entry");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn membership(slug: &str) -> MembershipRow {
        MembershipRow {
            organisation_id: Uuid::now_v7(),
            organisation_slug: slug.to_owned(),
            role: "admin".to_owned(),
        }
    }

    #[test]
    fn selects_single_membership() {
        let selected = select_membership(vec![membership("acme")], None).unwrap();
        assert_eq!(selected.organisation_slug, "acme");
    }

    #[test]
    fn requires_slug_with_multiple_memberships() {
        let result = select_membership(vec![membership("one"), membership("two")], None);
        assert!(matches!(result, Err(LoginFailure::Invalid(_))));
    }

    #[test]
    fn filters_by_slug() {
        let selected =
            select_membership(vec![membership("one"), membership("two")], Some("two")).unwrap();
        assert_eq!(selected.organisation_slug, "two");
    }
}
