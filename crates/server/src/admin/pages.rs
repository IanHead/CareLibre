//! Admin pages: dashboard, users and sessions.
//!
//! All page content is organisation-scoped and read-only in this milestone.

use std::collections::HashMap;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{Html, IntoResponse, Redirect, Response},
    Form,
};
use axum_extra::extract::cookie::CookieJar;
use chrono::{DateTime, Utc};
use sqlx::FromRow;
use uuid::Uuid;

use crate::{
    admin::{
        auth::{write_audit, AdminSession},
        csrf, html,
    },
    permissions::Capability,
    state::AppState,
    util::format_duration,
};

pub async fn dashboard(State(state): State<AppState>, session: AdminSession) -> Response {
    let users = count(
        &state,
        "SELECT count(*) FROM organisation_users WHERE organisation_id = $1",
        session.organisation_id,
    )
    .await;

    let active_sessions = count(
        &state,
        "SELECT count(*) FROM auth_sessions
         WHERE organisation_id = $1 AND revoked_at IS NULL AND expires_at > now()",
        session.organisation_id,
    )
    .await;

    let (users, active_sessions) = match (users, active_sessions) {
        (Ok(users), Ok(sessions)) => (users, sessions),
        (Err(err), _) | (_, Err(err)) => {
            tracing::error!(error = %err, "dashboard query failed");
            return failure("The dashboard could not be loaded.");
        }
    };

    let activity = if session.can(Capability::AuditRead) {
        let activity = sqlx::query_as::<_, ActivityRow>(
            "SELECT action, entity_type, created_at
             FROM audit_log
             WHERE organisation_id = $1
             ORDER BY created_at DESC
             LIMIT 10",
        )
        .bind(session.organisation_id)
        .fetch_all(&state.db)
        .await;

        match activity {
            Ok(activity) => activity,
            Err(err) => {
                tracing::error!(error = %err, "dashboard activity query failed");
                return failure("The dashboard could not be loaded.");
            }
        }
    } else {
        Vec::new()
    };

    let mut body = String::new();
    body.push_str(&html::page_head(
        "Dashboard",
        &format!(
            "{} · signed in as {}",
            session.organisation_name,
            session.role_label()
        ),
    ));

    body.push_str("<div class=\"grid\">");
    body.push_str(&metric("Users", &users.to_string()));
    body.push_str(&metric("Active sessions", &active_sessions.to_string()));
    body.push_str(&metric("Your access", session.role_label()));
    body.push_str(&metric(
        "Uptime",
        &format_duration(state.started_at.elapsed()),
    ));
    body.push_str("</div>");

    if session.can(Capability::AuditRead) {
        body.push_str("<h2>Recent activity</h2>");
        if activity.is_empty() {
            body.push_str("<p class=\"sub\">No audit events recorded yet.</p>");
        } else {
            body.push_str("<table><thead><tr><th>When (UTC)</th><th>Action</th><th>Entity</th></tr></thead><tbody>");
            for row in &activity {
                body.push_str(&format!(
                    "<tr><td class=\"muted\">{}</td><td><code>{}</code></td><td>{}</td></tr>",
                    format_time(row.created_at),
                    html::escape(&row.action),
                    html::escape(&row.entity_type),
                ));
            }
            body.push_str("</tbody></table>");
        }
    }

    body.push_str("<h2>Jump to</h2><div class=\"grid\">");
    if session.can(Capability::UsersManage) {
        body.push_str(&link_card(
            "Users",
            "Accounts and permission sets for this organisation.",
            "/admin/users",
        ));
        body.push_str(&link_card(
            "Sessions",
            "Live browser and API sessions for this organisation.",
            "/admin/sessions",
        ));
    } else if session.can(Capability::AuditRead) {
        body.push_str(&link_card(
            "Sessions",
            "Live browser and API sessions for this organisation.",
            "/admin/sessions",
        ));
    }
    if session.can(Capability::PermissionSetsManage) {
        body.push_str(&link_card(
            "Permission sets",
            "Capabilities and the OOTB personas.",
            "/admin/permission-sets",
        ));
    }
    if session.can(Capability::CustomFieldsManage) {
        body.push_str(&link_card(
            "Custom fields",
            "Extend objects with your own fields.",
            "/admin/custom-fields",
        ));
    }
    if session.can(Capability::SchemaView) {
        body.push_str(&link_card(
            "Database explorer",
            "Tables, columns, keys and relationships.",
            "/admin/schema",
        ));
    }
    body.push_str("</div>");

    page("Dashboard", "/admin", &session, body)
}

pub async fn users(State(state): State<AppState>, session: AdminSession) -> Response {
    if let Some(response) = session.guard(Capability::UsersManage) {
        return response;
    }

    let rows = sqlx::query_as::<_, UserRow>(
        "SELECT u.id, u.email, u.full_name, u.is_active, u.last_login_at,
                m.is_owner, m.created_at,
                COALESCE(string_agg(ps.name, ', ' ORDER BY ps.name), '') AS permission_sets
         FROM organisation_users m
         JOIN users u ON u.id = m.user_id
         LEFT JOIN organisation_user_permission_sets oup
           ON oup.organisation_id = m.organisation_id AND oup.user_id = m.user_id
         LEFT JOIN permission_sets ps ON ps.id = oup.permission_set_id
         WHERE m.organisation_id = $1
         GROUP BY u.id, u.email, u.full_name, u.is_active, u.last_login_at, m.is_owner, m.created_at
         ORDER BY m.is_owner DESC, u.full_name",
    )
    .bind(session.organisation_id)
    .fetch_all(&state.db)
    .await;

    let rows = match rows {
        Ok(rows) => rows,
        Err(err) => {
            tracing::error!(error = %err, "users query failed");
            return failure("Users could not be loaded.");
        }
    };

    let mut body = String::new();
    body.push_str(&html::page_head(
        "Users",
        "Accounts and permission sets for this organisation.",
    ));

    body.push_str("<p class=\"sub\">Owners bypass capability checks. Everyone else sees exactly what their permission sets allow.</p>");
    body.push_str("<table><thead><tr><th>Name</th><th>Email</th><th>Access</th><th>Status</th><th>Last login (UTC)</th><th>User since (UTC)</th><th></th></tr></thead><tbody>");
    for row in &rows {
        let access = if row.is_owner {
            "<span class=\"badge pk\">owner</span>".to_owned()
        } else if row.permission_sets.is_empty() {
            "<span class=\"badge yes\">no access</span>".to_owned()
        } else {
            format!(
                "<span class=\"pill\">{}</span>",
                html::escape(&row.permission_sets)
            )
        };

        body.push_str(&format!(
            "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td class=\"muted\">{}</td><td class=\"muted\">{}</td><td><a href=\"/admin/users/{}/access\">Edit access</a></td></tr>",
            html::escape(&row.full_name),
            html::escape(&row.email),
            access,
            if row.is_active {
                "<span class=\"badge pk\">active</span>"
            } else {
                "<span class=\"badge yes\">disabled</span>"
            },
            row.last_login_at
                .map_or_else(|| "—".to_owned(), format_time),
            format_time(row.created_at),
            row.id,
        ));
    }
    body.push_str("</tbody></table>");

    page("Users", "/admin/users", &session, body)
}

pub async fn user_access(
    State(state): State<AppState>,
    session: AdminSession,
    Path(user_id): Path<Uuid>,
    jar: CookieJar,
) -> Response {
    if let Some(response) = session.guard(Capability::UsersManage) {
        return response;
    }

    let user = match load_access_user(&state, &session, user_id).await {
        Ok(Some(user)) => user,
        Ok(None) => return not_found("User not found"),
        Err(err) => {
            tracing::error!(error = %err, "access user lookup failed");
            return failure("Access could not be loaded.");
        }
    };

    let sets = match load_sets(&state, &session).await {
        Ok(sets) => sets,
        Err(err) => {
            tracing::error!(error = %err, "permission set lookup failed");
            return failure("Permission sets could not be loaded.");
        }
    };

    let selected = match load_user_sets(&state, &session, user_id).await {
        Ok(selected) => selected,
        Err(err) => {
            tracing::error!(error = %err, "user permission set lookup failed");
            return failure("Access could not be loaded.");
        }
    };

    let (jar, token) = csrf::ensure(jar, state.config.environment.is_production());
    let mut body = String::new();
    body.push_str(&html::page_head(
        "Edit access",
        &format!("{} · {}", user.full_name, user.email),
    ));

    if user.is_owner {
        body.push_str("<div class=\"notice\"><strong>Owner.</strong> This account bypasses capability checks and always has full access, regardless of the sets selected below.</div>");
    }

    body.push_str(&format!(
        "<form class=\"form wide\" method=\"post\" action=\"/admin/users/{user_id}/access\">         <input type=\"hidden\" name=\"csrf\" value=\"{}\">",
        html::escape(&token)
    ));

    if sets.is_empty() {
        body.push_str("<p class=\"sub\">No permission sets exist.</p>");
    } else {
        body.push_str("<h2>Permission sets</h2>");
        for set in &sets {
            let checked = selected.contains(&set.id);
            body.push_str(&format!(
                "<div class=\"checkbox\"><input id=\"set_{id}\" name=\"set_{id}\" type=\"checkbox\"{}>                 <label for=\"set_{id}\"><strong>{name}</strong>{desc}</label></div>",
                if checked { " checked" } else { "" },
                id = set.id,
                name = html::escape(&set.name),
                desc = set
                    .description
                    .as_deref()
                    .filter(|text| !text.is_empty())
                    .map(|text| format!("<br><span class=\"muted\">{}</span>", html::escape(text)))
                    .unwrap_or_default(),
            ));
        }
        body.push_str("<p class=\"hint\">Assigning no sets leaves the account able to sign in but unable to see anything except its own dashboard.</p>");
    }

    body.push_str("<button type=\"submit\">Save access</button></form>");
    let html_body = html::layout("Edit access", "/admin/users", Some(&session.who()), &body);
    (jar, Html(html_body)).into_response()
}

pub async fn update_user_access(
    State(state): State<AppState>,
    session: AdminSession,
    Path(user_id): Path<Uuid>,
    jar: CookieJar,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    if let Some(response) = session.guard(Capability::UsersManage) {
        return response;
    }

    let csrf_token = form.get("csrf").cloned().unwrap_or_default();
    if !csrf::verify(&jar, &csrf_token) {
        return csrf_failure();
    }

    let _user = match load_access_user(&state, &session, user_id).await {
        Ok(Some(user)) => user,
        Ok(None) => return not_found("User not found"),
        Err(err) => {
            tracing::error!(error = %err, "access user lookup failed");
            return failure("Access could not be updated.");
        }
    };

    let mut selected = Vec::new();
    for key in form.keys() {
        if let Some(raw) = key.strip_prefix("set_") {
            if let Ok(id) = raw.parse::<Uuid>() {
                selected.push(id);
            }
        }
    }

    let mut tx = match state.db.begin().await {
        Ok(tx) => tx,
        Err(err) => {
            tracing::error!(error = %err, "access update transaction failed");
            return failure("Access could not be updated.");
        }
    };

    if let Err(err) = sqlx::query(
        "DELETE FROM organisation_user_permission_sets
         WHERE organisation_id = $1 AND user_id = $2",
    )
    .bind(session.organisation_id)
    .bind(user_id)
    .execute(&mut *tx)
    .await
    {
        tracing::error!(error = %err, "clearing permission sets failed");
        return failure("Access could not be updated.");
    }

    for set_id in &selected {
        if let Err(err) = sqlx::query(
            "INSERT INTO organisation_user_permission_sets
                (organisation_id, user_id, permission_set_id)
             SELECT $1, $2, $3
             WHERE EXISTS (
                 SELECT 1 FROM permission_sets WHERE id = $3 AND organisation_id = $1
             )",
        )
        .bind(session.organisation_id)
        .bind(user_id)
        .bind(set_id)
        .execute(&mut *tx)
        .await
        {
            tracing::error!(error = %err, "assigning permission set failed");
            return failure("Access could not be updated.");
        }
    }

    if let Err(err) = tx.commit().await {
        tracing::error!(error = %err, "access update commit failed");
        return failure("Access could not be updated.");
    }

    write_audit(
        &state.db,
        session.organisation_id,
        session.user_id,
        "permission_sets.assigned",
        "user",
        Some(user_id),
    )
    .await;

    (Redirect::to("/admin/users")).into_response()
}

async fn load_access_user(
    state: &AppState,
    session: &AdminSession,
    user_id: Uuid,
) -> Result<Option<AccessUser>, sqlx::Error> {
    sqlx::query_as::<_, AccessUser>(
        "SELECT u.id, u.email, u.full_name, m.is_owner
         FROM organisation_users m
         JOIN users u ON u.id = m.user_id
         WHERE m.organisation_id = $1 AND m.user_id = $2",
    )
    .bind(session.organisation_id)
    .bind(user_id)
    .fetch_optional(&state.db)
    .await
}

async fn load_sets(
    state: &AppState,
    session: &AdminSession,
) -> Result<Vec<AccessSet>, sqlx::Error> {
    sqlx::query_as::<_, AccessSet>(
        "SELECT id, name, description
         FROM permission_sets
         WHERE organisation_id = $1
         ORDER BY sort_order, name",
    )
    .bind(session.organisation_id)
    .fetch_all(&state.db)
    .await
}

async fn load_user_sets(
    state: &AppState,
    session: &AdminSession,
    user_id: Uuid,
) -> Result<Vec<Uuid>, sqlx::Error> {
    sqlx::query_scalar::<_, Uuid>(
        "SELECT permission_set_id FROM organisation_user_permission_sets
         WHERE organisation_id = $1 AND user_id = $2",
    )
    .bind(session.organisation_id)
    .bind(user_id)
    .fetch_all(&state.db)
    .await
}

#[derive(Debug, FromRow)]
struct AccessUser {
    #[allow(dead_code)]
    id: Uuid,
    email: String,
    full_name: String,
    is_owner: bool,
}

#[derive(Debug, FromRow)]
struct AccessSet {
    id: Uuid,
    name: String,
    description: Option<String>,
}

pub async fn sessions(State(state): State<AppState>, session: AdminSession) -> Response {
    if let Some(response) = session.guard_any(&[Capability::UsersManage, Capability::AuditRead]) {
        return response;
    }

    let rows = sqlx::query_as::<_, SessionListRow>(
        "SELECT s.id, u.email, s.user_agent, s.created_at, s.expires_at
         FROM auth_sessions s
         JOIN users u ON u.id = s.user_id
         WHERE s.organisation_id = $1
           AND s.revoked_at IS NULL
           AND s.expires_at > now()
         ORDER BY s.created_at DESC
         LIMIT 100",
    )
    .bind(session.organisation_id)
    .fetch_all(&state.db)
    .await;

    let rows = match rows {
        Ok(rows) => rows,
        Err(err) => {
            tracing::error!(error = %err, "sessions query failed");
            return failure("Sessions could not be loaded.");
        }
    };

    let mut body = String::new();
    body.push_str(&html::page_head(
        "Sessions",
        "Active browser and API sessions. Revocation is not exposed yet.",
    ));

    body.push_str("<table><thead><tr><th>User</th><th>Client</th><th>Started (UTC)</th><th>Expires (UTC)</th><th></th></tr></thead><tbody>");
    for row in &rows {
        let current = row.id == session.session_id;
        body.push_str(&format!(
            "<tr><td>{}</td><td class=\"muted\">{}</td><td class=\"muted\">{}</td><td class=\"muted\">{}</td><td>{}</td></tr>",
            html::escape(&row.email),
            row.user_agent
                .as_deref()
                .map(|agent| html::escape(&truncate(agent, 70)))
                .unwrap_or_else(|| "—".to_owned()),
            format_time(row.created_at),
            format_time(row.expires_at),
            if current {
                "<span class=\"badge pk\">this session</span>"
            } else {
                ""
            },
        ));
    }
    body.push_str("</tbody></table>");

    page("Sessions", "/admin/sessions", &session, body)
}

#[derive(Debug, FromRow)]
struct ActivityRow {
    action: String,
    entity_type: String,
    created_at: DateTime<Utc>,
}

#[derive(Debug, FromRow)]
struct UserRow {
    id: Uuid,
    email: String,
    full_name: String,
    is_active: bool,
    last_login_at: Option<DateTime<Utc>>,
    is_owner: bool,
    permission_sets: String,
    created_at: DateTime<Utc>,
}

#[derive(Debug, FromRow)]
struct SessionListRow {
    id: Uuid,
    email: String,
    user_agent: Option<String>,
    created_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
}

fn metric(label: &str, value: &str) -> String {
    format!(
        "<div class=\"card\"><h3>{}</h3><p class=\"big\">{}</p></div>",
        html::escape(label),
        html::escape(value)
    )
}

fn link_card(title: &str, description: &str, href: &str) -> String {
    format!(
        "<div class=\"card\"><h3><a href=\"{}\">{}</a></h3><p class=\"sub\">{}</p></div>",
        html::escape(href),
        html::escape(title),
        html::escape(description)
    )
}

fn page(title: &str, active: &str, session: &AdminSession, body: String) -> Response {
    Html(html::layout(title, active, Some(&session.who()), &body)).into_response()
}

fn failure(message: &str) -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Html(html::error_page("Admin", message)),
    )
        .into_response()
}

async fn count(state: &AppState, sql: &str, organisation_id: Uuid) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar::<_, i64>(sql)
        .bind(organisation_id)
        .fetch_one(&state.db)
        .await
}

fn format_time(value: DateTime<Utc>) -> String {
    value.format("%Y-%m-%d %H:%M").to_string()
}

fn truncate(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        value.to_owned()
    } else {
        let mut out: String = value.chars().take(max.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

fn not_found(message: &str) -> Response {
    (
        StatusCode::NOT_FOUND,
        Html(html::error_page("Not found", message)),
    )
        .into_response()
}

fn csrf_failure() -> Response {
    (
        StatusCode::FORBIDDEN,
        Html(html::error_page(
            "Request expired",
            "Reload the page and submit the form again.",
        )),
    )
        .into_response()
}
