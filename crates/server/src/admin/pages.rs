//! Admin pages: dashboard, members and sessions.
//!
//! All page content is organisation-scoped and read-only in this milestone.

use axum::{
    extract::State,
    http::StatusCode,
    response::{Html, IntoResponse, Response},
};
use chrono::{DateTime, Utc};
use sqlx::FromRow;
use uuid::Uuid;

use crate::{
    admin::{auth::AdminSession, html},
    state::AppState,
    util::format_duration,
};

pub async fn dashboard(State(state): State<AppState>, session: AdminSession) -> Response {
    let members = count(
        &state,
        "SELECT count(*) FROM organisation_members WHERE organisation_id = $1",
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

    let (members, active_sessions) = match (members, active_sessions) {
        (Ok(members), Ok(sessions)) => (members, sessions),
        (Err(err), _) | (_, Err(err)) => {
            tracing::error!(error = %err, "dashboard query failed");
            return failure("The dashboard could not be loaded.");
        }
    };

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

    let activity = match activity {
        Ok(activity) => activity,
        Err(err) => {
            tracing::error!(error = %err, "dashboard activity query failed");
            return failure("The dashboard could not be loaded.");
        }
    };

    let mut body = String::new();
    body.push_str(&html::page_head(
        "Dashboard",
        &format!(
            "{} · signed in as {}",
            session.organisation_name, session.role
        ),
    ));

    body.push_str("<div class=\"grid\">");
    body.push_str(&metric("Members", &members.to_string()));
    body.push_str(&metric("Active sessions", &active_sessions.to_string()));
    body.push_str(&metric("Your role", session.role.as_str()));
    body.push_str(&metric(
        "Uptime",
        &format_duration(state.started_at.elapsed()),
    ));
    body.push_str("</div>");

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

    body.push_str("<h2>Jump to</h2><div class=\"grid\">");
    body.push_str(&link_card(
        "Members",
        "People with access to this organisation.",
        "/admin/members",
    ));
    body.push_str(&link_card(
        "Sessions",
        "Live browser and API sessions for this organisation.",
        "/admin/sessions",
    ));
    body.push_str(&link_card(
        "Database explorer",
        "Tables, columns, keys and relationships.",
        "/admin/schema",
    ));
    body.push_str("</div>");

    page("Dashboard", "/admin", &session, body)
}

pub async fn members(State(state): State<AppState>, session: AdminSession) -> Response {
    let rows = sqlx::query_as::<_, MemberRow>(
        "SELECT u.id, u.email, u.full_name, u.is_active, u.last_login_at, m.role, m.created_at
         FROM organisation_members m
         JOIN users u ON u.id = m.user_id
         WHERE m.organisation_id = $1
         ORDER BY m.role, u.full_name",
    )
    .bind(session.organisation_id)
    .fetch_all(&state.db)
    .await;

    let rows = match rows {
        Ok(rows) => rows,
        Err(err) => {
            tracing::error!(error = %err, "members query failed");
            return failure("Members could not be loaded.");
        }
    };

    let mut body = String::new();
    body.push_str(&html::page_head(
        "Members",
        "Everyone with access to this organisation.",
    ));

    body.push_str("<table><thead><tr><th>Name</th><th>Email</th><th>Role</th><th>Status</th><th>Last login (UTC)</th><th>Member since (UTC)</th></tr></thead><tbody>");
    for row in &rows {
        body.push_str(&format!(
            "<tr><td>{}</td><td>{}</td><td><span class=\"pill\">{}</span></td><td>{}</td><td class=\"muted\">{}</td><td class=\"muted\">{}</td></tr>",
            html::escape(&row.full_name),
            html::escape(&row.email),
            html::escape(&row.role),
            if row.is_active {
                "<span class=\"badge pk\">active</span>"
            } else {
                "<span class=\"badge yes\">disabled</span>"
            },
            row.last_login_at
                .map_or_else(|| "—".to_owned(), format_time),
            format_time(row.created_at),
        ));
    }
    body.push_str("</tbody></table>");

    page("Members", "/admin/members", &session, body)
}

pub async fn sessions(State(state): State<AppState>, session: AdminSession) -> Response {
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
struct MemberRow {
    #[allow(dead_code)]
    id: Uuid,
    email: String,
    full_name: String,
    is_active: bool,
    last_login_at: Option<DateTime<Utc>>,
    role: String,
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
