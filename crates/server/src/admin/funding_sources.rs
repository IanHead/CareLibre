//! Admin screen for the organisation's funding source catalogue.

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
    funding::{self, CLAIMING_MODELS, SOURCE_KINDS},
    permissions::Capability,
    state::AppState,
    worker_qualities::normalise_key,
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
    name: String,
    #[serde(default)]
    key: String,
    kind: String,
    claiming_model: String,
    #[serde(default)]
    description: String,
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
    if let Some(response) = session.guard(Capability::SettingsManage) {
        return response;
    }
    let sources = match funding::load_sources(&state.db, session.organisation_id).await {
        Ok(sources) => sources,
        Err(err) => {
            tracing::error!(error = %err, "funding source list failed");
            return failure("Funding sources could not be loaded.");
        }
    };
    let (jar, token) = csrf::ensure(jar, state.config.environment.is_production());

    let mut body = html::page_head(
        "Funding sources",
        "Programmes the organisation bills against. Packages and agreements are created from these sources on each client.",
    );
    if params.added.is_some() {
        body.push_str("<div class=\"notice\">Funding source added.</div>");
    }
    if params.updated.is_some() {
        body.push_str("<div class=\"notice\">Funding source updated.</div>");
    }
    if let Some(error) = params.error.as_deref() {
        body.push_str(&format!(
            "<div class=\"notice error\">{}</div>",
            html::escape(error)
        ));
    }

    body.push_str(
        "<div class=\"card\"><h2>Add a source</h2>\
         <p class=\"muted\">NDIS, Support at Home, CHSP, private fee-for-service and TAC are seeded. \
         Add another source for any other programme.</p>\
         <form method=\"post\" action=\"/admin/funding-sources\" class=\"form-grid\">",
    );
    body.push_str(&format!(
        "<input type=\"hidden\" name=\"csrf\" value=\"{}\">",
        html::escape(&token)
    ));
    body.push_str(
        "<label>Name<input name=\"name\" required placeholder=\"e.g. TAC funding\"></label>\
         <label>Key<input name=\"key\" placeholder=\"derived from the name\"></label>\
         <label>Kind<select name=\"kind\">",
    );
    for kind in SOURCE_KINDS {
        body.push_str(&format!(
            "<option value=\"{kind}\">{}</option>",
            html::escape(kind_label(kind))
        ));
    }
    body.push_str("</select></label><label>Claiming model<select name=\"claiming_model\">");
    for model in CLAIMING_MODELS {
        body.push_str(&format!(
            "<option value=\"{model}\">{}</option>",
            html::escape(model_label(model))
        ));
    }
    body.push_str(
        "</select></label>\
         <label>Description<input name=\"description\" placeholder=\"Optional\"></label>\
         <button type=\"submit\">Add source</button></form></div>",
    );

    body.push_str(
        "<div class=\"card\"><h2>Sources</h2><table><thead><tr>\
         <th>Source</th><th>Kind</th><th>Claiming model</th><th>Status</th><th></th>\
         </tr></thead><tbody>",
    );
    for source in sources {
        let (status, action, value) = if source.is_active {
            ("Active", "Retire", "false")
        } else {
            ("Retired", "Restore", "true")
        };
        body.push_str(&format!(
            "<tr><td><strong>{}</strong><br><span class=\"sub\">{} · {}</span></td>\
             <td>{}</td><td>{}</td><td>{status}</td><td>\
             <form method=\"post\" action=\"/admin/funding-sources/{}/active\">\
             <input type=\"hidden\" name=\"csrf\" value=\"{}\">\
             <input type=\"hidden\" name=\"active\" value=\"{value}\">\
             <button type=\"submit\" class=\"link\">{action}</button></form></td></tr>",
            html::escape(&source.name),
            html::escape(&source.key),
            html::escape(source.description.as_deref().unwrap_or("")),
            html::escape(kind_label(&source.kind)),
            html::escape(model_label(&source.claiming_model)),
            source.id,
            html::escape(&token),
        ));
    }
    body.push_str("</tbody></table></div>");

    (
        jar,
        Html(html::layout(
            "Funding sources",
            "/admin/funding-sources",
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
    if let Some(response) = session.guard(Capability::SettingsManage) {
        return response;
    }
    if !csrf::verify(&jar, &form.csrf) {
        return csrf_failure();
    }

    let name = form.name.trim().to_owned();
    if name.is_empty() || name.chars().count() > 160 {
        return redirect_after(jar, Err(AppError::validation("name is required")));
    }
    let key = if form.key.trim().is_empty() {
        normalise_key(&name)
    } else {
        normalise_key(&form.key)
    };
    let Some(key) = key else {
        return redirect_after(
            jar,
            Err(AppError::validation("key needs letters, starting with one")),
        );
    };
    if !SOURCE_KINDS.contains(&form.kind.as_str()) {
        return redirect_after(jar, Err(AppError::validation("invalid kind")));
    }
    if !CLAIMING_MODELS.contains(&form.claiming_model.as_str()) {
        return redirect_after(jar, Err(AppError::validation("invalid claiming model")));
    }

    let result = sqlx::query(
        "INSERT INTO funding_sources
            (id, organisation_id, key, name, kind, claiming_model, description,
             configuration, is_active, is_system)
         VALUES ($1,$2,$3,$4,$5,$6,$7,'{}'::jsonb,true,false)
         ON CONFLICT DO NOTHING",
    )
    .bind(Uuid::now_v7())
    .bind(session.organisation_id)
    .bind(&key)
    .bind(&name)
    .bind(&form.kind)
    .bind(&form.claiming_model)
    .bind((!form.description.trim().is_empty()).then(|| form.description.trim().to_owned()))
    .execute(&state.db)
    .await
    .map_err(AppError::from)
    .and_then(|result| {
        if result.rows_affected() == 0 {
            Err(AppError::conflict(format!(
                "{key} is already a funding source"
            )))
        } else {
            Ok(())
        }
    });

    redirect_after(jar, result)
}

pub async fn set_active(
    State(state): State<AppState>,
    session: AdminSession,
    Path(id): Path<Uuid>,
    jar: CookieJar,
    Form(form): Form<ToggleForm>,
) -> Response {
    if let Some(response) = session.guard(Capability::SettingsManage) {
        return response;
    }
    if !csrf::verify(&jar, &form.csrf) {
        return csrf_failure();
    }
    let result = sqlx::query(
        "UPDATE funding_sources SET is_active = $3
         WHERE organisation_id = $1 AND id = $2",
    )
    .bind(session.organisation_id)
    .bind(id)
    .bind(form.active == "true")
    .execute(&state.db)
    .await
    .map_err(AppError::from)
    .and_then(|result| {
        if result.rows_affected() == 0 {
            Err(AppError::not_found("funding source not found"))
        } else {
            Ok(())
        }
    });
    redirect_after(jar, result)
}

fn redirect_after(jar: CookieJar, result: Result<(), AppError>) -> Response {
    let query = match result {
        Ok(()) => "updated=1".to_owned(),
        Err(error) => {
            let message = match error {
                AppError::Database(_) | AppError::Internal(_) => {
                    tracing::error!(error = %error, "funding source change failed");
                    "The funding source could not be saved.".to_owned()
                }
                other => other.to_string(),
            };
            format!("error={}", percent_encode(&message))
        }
    };
    (
        jar,
        Redirect::to(&format!("/admin/funding-sources?{query}")),
    )
        .into_response()
}

fn kind_label(kind: &str) -> &'static str {
    match kind {
        "ndis" => "NDIS",
        "support_at_home" => "Support at Home",
        "chsp" => "CHSP",
        "private" => "Private / fee for service",
        _ => "Other",
    }
}

fn model_label(model: &str) -> &'static str {
    match model {
        "ndis_price_guide" => "NDIS price guide",
        "support_at_home" => "Support at Home",
        "chsp" => "CHSP",
        "fee_for_service" => "Fee for service",
        _ => "Manual",
    }
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

fn failure(message: &str) -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Html(html::error_page("Funding sources", message)),
    )
        .into_response()
}

fn csrf_failure() -> Response {
    (
        StatusCode::FORBIDDEN,
        Html(html::error_page(
            "Funding sources",
            "The form expired. Reload the page and try again.",
        )),
    )
        .into_response()
}
