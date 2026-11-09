//! Admin screen for the relationship type catalogue: list, add, retire, restore.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{Html, IntoResponse, Redirect, Response},
    Form,
};
use axum_extra::extract::cookie::CookieJar;
use serde::Deserialize;
use uuid::Uuid;

use crate::{
    admin::{auth::AdminSession, csrf, html},
    error::AppError,
    permissions::Capability,
    relationship_types::{self, CATEGORIES},
    state::AppState,
};

#[derive(Debug, Default, Deserialize)]
pub struct ListParams {
    added: Option<String>,
    updated: Option<String>,
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct AddForm {
    csrf: String,
    from_role: String,
    to_role: String,
    category: String,
}

#[derive(Debug, Deserialize)]
pub struct ToggleForm {
    csrf: String,
    active: String,
}

pub async fn list(
    State(state): State<AppState>,
    session: AdminSession,
    Query(params): Query<ListParams>,
    jar: CookieJar,
) -> Response {
    if let Some(response) = session.guard(Capability::RelationshipTypesManage) {
        return response;
    }
    let types = match relationship_types::load(&state.db, session.organisation_id).await {
        Ok(types) => types,
        Err(err) => {
            tracing::error!(error = %err, "relationship type list failed");
            return failure("Relationship types could not be loaded.");
        }
    };
    let (jar, token) = csrf::ensure(jar, state.config.environment.is_production());

    let mut body = html::page_head(
        "Relationship types",
        "The role pairs people can choose when linking records. Roles are never typed freely, so the same relationship is always recorded the same way.",
    );
    if params.added.is_some() {
        body.push_str("<div class=\"notice\">Relationship type added.</div>");
    }
    if params.updated.is_some() {
        body.push_str("<div class=\"notice\">Relationship type updated.</div>");
    }
    if let Some(error) = params.error.as_deref() {
        body.push_str(&format!(
            "<div class=\"notice error\">{}</div>",
            html::escape(error)
        ));
    }

    body.push_str(
        "<div class=\"card\"><h2>Add a type</h2>\
         <p class=\"muted\">Type each role as words, e.g. <em>Support Coordinator</em>. \
         A pair works in both directions, so add it once.</p>\
         <form method=\"post\" action=\"/admin/relationship-types\" class=\"form-grid\">",
    );
    body.push_str(&format!(
        "<input type=\"hidden\" name=\"csrf\" value=\"{}\">",
        html::escape(&token)
    ));
    body.push_str(
        "<label>One side<input name=\"from_role\" required placeholder=\"Participant\"></label>\
         <label>Other side<input name=\"to_role\" required placeholder=\"Support Coordinator\"></label>\
         <label>Group<select name=\"category\">",
    );
    for (key, label) in CATEGORIES {
        body.push_str(&format!("<option value=\"{key}\">{label}</option>"));
    }
    body.push_str("</select></label><button type=\"submit\">Add type</button></form></div>");

    for (category, label) in CATEGORIES {
        let in_group: Vec<_> = types.iter().filter(|t| t.category == category).collect();
        if in_group.is_empty() {
            continue;
        }
        body.push_str(&format!(
            "<div class=\"card\"><h2>{label}</h2><table><thead><tr>\
             <th>One side</th><th>Other side</th><th>In use</th><th>Status</th><th></th>\
             </tr></thead><tbody>"
        ));
        for t in in_group {
            let (status, action, value) = if t.is_active {
                ("Active", "Retire", "false")
            } else {
                ("Retired", "Restore", "true")
            };
            body.push_str(&format!(
                "<tr><td>{}</td><td>{}</td><td>{}</td><td>{status}</td><td>\
                 <form method=\"post\" action=\"/admin/relationship-types/{}/active\">\
                 <input type=\"hidden\" name=\"csrf\" value=\"{}\">\
                 <input type=\"hidden\" name=\"active\" value=\"{value}\">\
                 <button type=\"submit\" class=\"link\">{action}</button></form></td></tr>",
                html::escape(&words(&t.from_role)),
                html::escape(&words(&t.to_role)),
                t.in_use,
                t.id,
                html::escape(&token),
            ));
        }
        body.push_str("</tbody></table></div>");
    }

    (
        jar,
        Html(html::layout(
            "Relationship types",
            "/admin/relationship-types",
            Some(&session.who()),
            &body,
        )),
    )
        .into_response()
}

pub async fn add(
    State(state): State<AppState>,
    session: AdminSession,
    jar: CookieJar,
    Form(form): Form<AddForm>,
) -> Response {
    if let Some(response) = session.guard(Capability::RelationshipTypesManage) {
        return response;
    }
    if !csrf::verify(&jar, &form.csrf) {
        return csrf_failure();
    }
    let result = relationship_types::create(
        &state.db,
        session.organisation_id,
        &form.from_role,
        &form.to_role,
        &form.category,
    )
    .await;
    redirect_after(jar, result.map(|_| "added=1"))
}

pub async fn set_active(
    State(state): State<AppState>,
    session: AdminSession,
    Path(id): Path<Uuid>,
    jar: CookieJar,
    Form(form): Form<ToggleForm>,
) -> Response {
    if let Some(response) = session.guard(Capability::RelationshipTypesManage) {
        return response;
    }
    if !csrf::verify(&jar, &form.csrf) {
        return csrf_failure();
    }
    let result = relationship_types::set_active(
        &state.db,
        session.organisation_id,
        id,
        form.active == "true",
    )
    .await;
    redirect_after(jar, result.map(|()| "updated=1"))
}

fn redirect_after(jar: CookieJar, result: Result<&str, AppError>) -> Response {
    let query = match result {
        Ok(flag) => flag.to_owned(),
        Err(error) => {
            let message = match error {
                AppError::Database(_) | AppError::Internal(_) => {
                    tracing::error!(error = %error, "relationship type change failed");
                    "The relationship type could not be saved.".to_owned()
                }
                other => other.to_string(),
            };
            format!("error={}", percent_encode(&message))
        }
    };
    (
        jar,
        Redirect::to(&format!("/admin/relationship-types?{query}")),
    )
        .into_response()
}

fn percent_encode(text: &str) -> String {
    text.bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (byte as char).to_string()
            }
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

/// `support_coordinator` → `Support coordinator`.
fn words(role: &str) -> String {
    let text = role.replace('_', " ");
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

fn failure(message: &str) -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Html(html::error_page("Relationship types", message)),
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
