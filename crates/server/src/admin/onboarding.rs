//! Onboarding dashboard.

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

use crate::{
    admin::{
        auth::{write_audit, AdminSession},
        csrf, html,
    },
    onboarding::{self, Stage, Task, TASKS},
    permissions::Capability,
    state::AppState,
};

#[derive(Debug, FromRow)]
struct OnboardingRow {
    mode: String,
    completed_at: Option<DateTime<Utc>>,
}

pub async fn index(
    State(state): State<AppState>,
    session: AdminSession,
    jar: CookieJar,
) -> Response {
    if let Some(response) = session.guard(Capability::SettingsManage) {
        return response;
    }

    let summary = match onboarding::summary(&state.db, session.organisation_id).await {
        Ok(summary) => summary,
        Err(err) => {
            tracing::error!(error = %err, "onboarding summary failed");
            return failure("Setup progress could not be loaded.");
        }
    };
    let states = onboarding::manual_states(&state.db, session.organisation_id)
        .await
        .unwrap_or_default();
    let row = sqlx::query_as::<_, OnboardingRow>(
        "SELECT mode, completed_at FROM organisation_onboarding WHERE organisation_id = $1",
    )
    .bind(session.organisation_id)
    .fetch_optional(&state.db)
    .await
    .ok()
    .flatten();

    let mode = row.as_ref().map_or("guided", |row| row.mode.as_str());
    let completed_at = row.as_ref().and_then(|row| row.completed_at);
    let (jar, token) = csrf::ensure(jar, state.config.environment.is_production());

    let mut body = String::new();
    body.push_str(&html::page_head(
        "Setup",
        "Guided checklist from a fresh install to a first completed shift.",
    ));

    body.push_str(&format!(
        "<div class=\"card\"><h3>Progress</h3>\
         <p class=\"big\">{}/{} required</p>\
         <div style=\"height:8px;background:var(--border);border-radius:999px;overflow:hidden;margin:10px 0\">\
         <div style=\"height:100%;width:{}%;background:var(--ok)\"></div></div>\
         <p class=\"sub\">{} of {} optional done{}.</p></div>",
        summary.required_done,
        summary.required_total,
        summary.percent,
        summary.optional_done,
        summary.optional_total,
        if summary.required_skipped > 0 {
            format!(" · {} required skipped", summary.required_skipped)
        } else {
            String::new()
        },
    ));

    if let Some(completed_at) = completed_at {
        body.push_str(&format!(
            "<div class=\"warn-box\"><strong>Live since {}.</strong> Setup is complete; you can still revisit tasks.</div>",
            completed_at.format("%Y-%m-%d %H:%M UTC")
        ));
    }

    body.push_str(&format!(
        "<div class=\"card\"><h3>Mode</h3>\
         <p class=\"sub\">Migration mode prioritises imports and a parallel run.</p>\
         <form method=\"post\" action=\"/admin/onboarding/mode\" style=\"margin-top:8px\">\
         <input type=\"hidden\" name=\"csrf\" value=\"{}\">\
         <select name=\"mode\" style=\"padding:7px;border-radius:6px;border:1px solid var(--border);background:var(--bg);color:var(--text)\">\
         <option value=\"guided\"{}>Guided (fresh start)</option>\
         <option value=\"migration\"{}>Migration from another system</option>\
         </select>\
         <button type=\"submit\" class=\"secondary\">Save mode</button></form></div>",
        html::escape(&token),
        if mode == "guided" { " selected" } else { "" },
        if mode == "migration" { " selected" } else { "" },
    ));

    for stage in Stage::ALL {
        let tasks = TASKS
            .iter()
            .filter(|task| task.stage == stage)
            .collect::<Vec<_>>();
        if tasks.is_empty() {
            continue;
        }
        body.push_str(&format!("<h2>{}</h2>", html::escape(stage.label())));
        body.push_str("<div class=\"card\">");
        for task in tasks {
            body.push_str(&render_task(&state, &session, task, states.get(task.key), &token).await);
        }
        body.push_str("</div>");
    }

    if summary.complete {
        body.push_str(&format!(
            "<h2>Go live</h2><div class=\"card\"><p>Every required task is complete.</p>\
             <form method=\"post\" action=\"/admin/onboarding/signoff\">\
             <input type=\"hidden\" name=\"csrf\" value=\"{}\">\
             <button type=\"submit\">Mark organisation live</button></form></div>",
            html::escape(&token)
        ));
    } else if let Some(next) = summary.next {
        body.push_str(&format!(
            "<h2>Next up</h2><div class=\"card\"><p><strong>{}</strong> — {}</p></div>",
            html::escape(next.title),
            html::escape(next.why)
        ));
    }

    let html_body = html::layout("Setup", "/admin/onboarding", Some(&session.who()), &body);
    (jar, Html(html_body)).into_response()
}

async fn render_task(
    state: &AppState,
    session: &AdminSession,
    task: &Task,
    manual: Option<&onboarding::TaskState>,
    token: &str,
) -> String {
    let detected = match task.detection {
        onboarding::Detection::Manual => false,
        detection => onboarding::is_detected(&state.db, session.organisation_id, detection)
            .await
            .unwrap_or(false),
    };
    let done = detected || manual.is_some_and(|state| state.status == "done");
    let skipped = !done && manual.is_some_and(|state| state.status == "skipped");

    let icon = if done {
        "<span class=\"badge pk\">done</span>"
    } else if skipped {
        "<span class=\"badge yes\">skipped</span>"
    } else {
        "<span class=\"badge id\">todo</span>"
    };

    let mut actions = String::new();
    if !done && !skipped && task.detection == onboarding::Detection::Manual {
        actions.push_str(&format!(
            "<form method=\"post\" action=\"/admin/onboarding/{}/done\" style=\"display:inline\">\
             <input type=\"hidden\" name=\"csrf\" value=\"{}\">\
             <button type=\"submit\" class=\"secondary\">Mark done</button></form>",
            task.key,
            html::escape(token)
        ));
        actions.push_str(&format!(
            "<form method=\"post\" action=\"/admin/onboarding/{}/skip\" style=\"display:inline;margin-left:6px\">\
             <input type=\"hidden\" name=\"csrf\" value=\"{}\">\
             <input name=\"reason\" placeholder=\"Reason\" style=\"padding:6px;border-radius:6px;border:1px solid var(--border);background:var(--bg);color:var(--text)\">\
             <button type=\"submit\" class=\"secondary\">Skip</button></form>",
            task.key,
            html::escape(token)
        ));
    }
    if let Some(href) = task.href {
        actions.push_str(&format!(
            " <a href=\"{}\" style=\"margin-left:8px\">Open</a>",
            html::escape(href)
        ));
    }

    let detection_note = match task.detection {
        onboarding::Detection::Manual if skipped => manual
            .and_then(|state| state.reason.clone())
            .map(|reason| format!("<p class=\"sub\">Skipped: {}</p>", html::escape(&reason)))
            .unwrap_or_default(),
        onboarding::Detection::Manual => String::new(),
        _ if detected => "<p class=\"sub\">Detected automatically.</p>".to_owned(),
        _ => "<p class=\"sub\">Complete the underlying data and this ticks itself.</p>".to_owned(),
    };

    format!(
        "<div style=\"padding:12px 0;border-bottom:1px solid var(--border)\">\
         <div style=\"display:flex;justify-content:space-between;gap:12px;align-items:flex-start\">\
         <div><strong>{}</strong> {} <span class=\"muted\">· {}</span>\
         <p class=\"sub\">{}</p>{}</div>\
         <div style=\"text-align:right;white-space:nowrap\">{}</div>\
         </div></div>",
        html::escape(task.title),
        icon,
        html::escape(task.estimate),
        html::escape(task.why),
        detection_note,
        actions,
    )
}

pub async fn done(
    State(state): State<AppState>,
    session: AdminSession,
    Path(key): Path<String>,
    jar: CookieJar,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    if let Some(response) = session.guard(Capability::SettingsManage) {
        return response;
    }
    if !csrf::verify(&jar, form.get("csrf").map(String::as_str).unwrap_or("")) {
        return csrf_failure();
    }
    let Some(task) = onboarding::task(&key) else {
        return not_found();
    };
    if task.detection != onboarding::Detection::Manual {
        return bad_request("This task is detected automatically.");
    }

    if let Err(err) = sqlx::query(
        "INSERT INTO onboarding_task_states
            (organisation_id, task_key, status, marked_by)
         VALUES ($1,$2,'done',$3)
         ON CONFLICT (organisation_id, task_key)
         DO UPDATE SET status='done', reason=NULL, marked_by=$3, marked_at=now()",
    )
    .bind(session.organisation_id)
    .bind(task.key)
    .bind(session.user_id)
    .execute(&state.db)
    .await
    {
        tracing::error!(error = %err, "onboarding done failed");
        return failure("The task could not be updated.");
    }

    write_audit(
        &state.db,
        session.organisation_id,
        session.user_id,
        "onboarding.done",
        "onboarding_task",
        None,
    )
    .await;

    (jar, Redirect::to("/admin/onboarding")).into_response()
}

pub async fn skip(
    State(state): State<AppState>,
    session: AdminSession,
    Path(key): Path<String>,
    jar: CookieJar,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    if let Some(response) = session.guard(Capability::SettingsManage) {
        return response;
    }
    if !csrf::verify(&jar, form.get("csrf").map(String::as_str).unwrap_or("")) {
        return csrf_failure();
    }
    let Some(task) = onboarding::task(&key) else {
        return not_found();
    };
    if task.detection != onboarding::Detection::Manual {
        return bad_request("This task is detected automatically.");
    }
    let reason = form
        .get("reason")
        .map(|reason| reason.trim())
        .filter(|reason| !reason.is_empty());
    let Some(reason) = reason else {
        return bad_request("Give a short reason for skipping.");
    };

    if let Err(err) = sqlx::query(
        "INSERT INTO onboarding_task_states
            (organisation_id, task_key, status, reason, marked_by)
         VALUES ($1,$2,'skipped',$3,$4)
         ON CONFLICT (organisation_id, task_key)
         DO UPDATE SET status='skipped', reason=$3, marked_by=$4, marked_at=now()",
    )
    .bind(session.organisation_id)
    .bind(task.key)
    .bind(reason)
    .bind(session.user_id)
    .execute(&state.db)
    .await
    {
        tracing::error!(error = %err, "onboarding skip failed");
        return failure("The task could not be updated.");
    }

    write_audit(
        &state.db,
        session.organisation_id,
        session.user_id,
        "onboarding.skipped",
        "onboarding_task",
        None,
    )
    .await;

    (jar, Redirect::to("/admin/onboarding")).into_response()
}

pub async fn signoff(
    State(state): State<AppState>,
    session: AdminSession,
    jar: CookieJar,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    if let Some(response) = session.guard(Capability::SettingsManage) {
        return response;
    }
    if !csrf::verify(&jar, form.get("csrf").map(String::as_str).unwrap_or("")) {
        return csrf_failure();
    }

    let summary = match onboarding::summary(&state.db, session.organisation_id).await {
        Ok(summary) => summary,
        Err(err) => {
            tracing::error!(error = %err, "onboarding summary failed");
            return failure("Setup progress could not be loaded.");
        }
    };
    if !summary.complete {
        return bad_request("Complete every required task before going live.");
    }

    let _ = sqlx::query(
        "UPDATE organisation_onboarding SET completed_at = now()
         WHERE organisation_id = $1",
    )
    .bind(session.organisation_id)
    .execute(&state.db)
    .await;

    write_audit(
        &state.db,
        session.organisation_id,
        session.user_id,
        "onboarding.completed",
        "organisation",
        Some(session.organisation_id),
    )
    .await;

    (jar, Redirect::to("/admin/onboarding")).into_response()
}

pub async fn mode(
    State(state): State<AppState>,
    session: AdminSession,
    jar: CookieJar,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    if let Some(response) = session.guard(Capability::SettingsManage) {
        return response;
    }
    if !csrf::verify(&jar, form.get("csrf").map(String::as_str).unwrap_or("")) {
        return csrf_failure();
    }
    let mode = match form.get("mode").map(String::as_str) {
        Some("migration") => "migration",
        _ => "guided",
    };

    let _ = sqlx::query(
        "INSERT INTO organisation_onboarding (organisation_id, mode) VALUES ($1,$2)
         ON CONFLICT (organisation_id) DO UPDATE SET mode=$2",
    )
    .bind(session.organisation_id)
    .bind(mode)
    .execute(&state.db)
    .await;

    (jar, Redirect::to("/admin/onboarding")).into_response()
}

fn bad_request(message: &str) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Html(html::error_page("Setup", message)),
    )
        .into_response()
}

fn not_found() -> Response {
    (
        StatusCode::NOT_FOUND,
        Html(html::error_page("Setup", "Unknown setup task.")),
    )
        .into_response()
}

fn failure(message: &str) -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Html(html::error_page("Setup", message)),
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
