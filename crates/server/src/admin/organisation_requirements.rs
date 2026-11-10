//! Organisation-wide worker requirements, shown in Organisation Preferences.
//!
//! These are checks every worker must hold before being rostered at all —
//! NDIS Worker Screening, Police check, Working With Children Check — as
//! opposed to requirements a particular client places on their workers.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{Html, IntoResponse, Redirect, Response},
    Form,
};
use axum_extra::extract::cookie::CookieJar;
use serde::Deserialize;
use serde_json::Value;
use sqlx::{types::Json as SqlJson, FromRow};
use uuid::Uuid;

use crate::{
    admin::{
        auth::{write_audit, AdminSession},
        csrf, html,
    },
    api::worker_qualities::load_catalogue,
    error::AppError,
    permissions::Capability,
    state::AppState,
    worker_qualities::{validate_requirement, Quality},
};

#[derive(Debug, Deserialize)]
pub struct RequirementForm {
    csrf: String,
    quality_id: Uuid,
    operator: String,
    #[serde(default)]
    value: String,
    severity: String,
    #[serde(default)]
    reason: String,
}

#[derive(Debug, Deserialize)]
pub struct DeleteForm {
    csrf: String,
}

#[derive(Debug, FromRow)]
struct RequirementRow {
    id: Uuid,
    quality_name: String,
    category: String,
    operator: String,
    value: Option<SqlJson<Value>>,
    severity: String,
    reason: Option<String>,
}

/// Render the requirements section for the Organisation Preferences page.
pub async fn section(
    state: &AppState,
    organisation_id: Uuid,
    token: &str,
) -> Result<String, AppError> {
    let catalogue = load_catalogue(&state.db, organisation_id).await?;
    let requirements = load_requirements(&state.db, organisation_id).await?;

    let mut body = String::new();
    body.push_str("<h2>Worker requirements</h2>");
    body.push_str(
        "<p class=\"sub\">Checks every worker must hold before being rostered, regardless of client. \
         Record the worker's check on their profile with an expiry; unmet requirements warn and rank lower, \
         or block when you choose Block.</p>",
    );

    body.push_str("<div class=\"card\"><h3>Add a requirement</h3>");
    body.push_str(
        "<form method=\"post\" action=\"/admin/organisation/requirements\" class=\"form-grid\">",
    );
    body.push_str(&format!(
        "<input type=\"hidden\" name=\"csrf\" value=\"{}\">",
        html::escape(token)
    ));
    body.push_str("<label>Quality<select name=\"quality_id\" required>");
    for quality in catalogue.iter().filter(|quality| quality.is_active) {
        body.push_str(&format!(
            "<option value=\"{}\">{} · {}</option>",
            quality.id,
            html::escape(&quality.name),
            html::escape(category_label(&quality.category)),
        ));
    }
    body.push_str("</select></label>");
    body.push_str(
        "<label>Rule<select name=\"operator\">\
         <option value=\"exists\">must be held</option>\
         <option value=\"equals\">must equal</option>\
         <option value=\"not_equals\">must not equal</option>\
         <option value=\"in\">must be one of</option>\
         <option value=\"contains\">must include</option>\
         </select></label>",
    );
    body.push_str(
        "<label>Value<input name=\"value\" placeholder=\"Blank for “must be held”; comma-separate for “one of”\"></label>",
    );
    body.push_str(
        "<label>If unmet<select name=\"severity\">\
         <option value=\"warn\">Warn and rank lower</option>\
         <option value=\"block\">Block</option>\
         </select></label>",
    );
    body.push_str(
        "<label>Reason<input name=\"reason\" placeholder=\"e.g. NDIS practice standard\"></label>",
    );
    body.push_str("<button type=\"submit\">Add requirement</button></form></div>");

    if requirements.is_empty() {
        body.push_str(
            "<div class=\"card\"><p class=\"sub\">No organisation-wide requirements yet. \
             Add NDIS Worker Screening, Police check or Working With Children Check to make them mandatory for every worker.</p></div>",
        );
        return Ok(body);
    }

    body.push_str(
        "<div class=\"card\"><h3>Current requirements</h3><table><thead><tr>\
        <th>Quality</th><th>Rule</th><th>If unmet</th><th>Reason</th><th></th>\
        </tr></thead><tbody>",
    );
    for requirement in requirements {
        body.push_str(&format!(
            "<tr><td><strong>{}</strong><br><span class=\"sub\">{}</span></td><td>{}</td>\
             <td>{}</td><td>{}</td><td>\
             <form method=\"post\" action=\"/admin/organisation/requirements/{}/delete\">\
             <input type=\"hidden\" name=\"csrf\" value=\"{}\">\
             <button type=\"submit\" class=\"link\">Remove</button></form></td></tr>",
            html::escape(&requirement.quality_name),
            html::escape(category_label(&requirement.category)),
            html::escape(&requirement_summary(&requirement)),
            if requirement.severity == "block" {
                "<span class=\"pill danger\">Block</span>"
            } else {
                "<span class=\"pill\">Warn</span>"
            },
            html::escape(requirement.reason.as_deref().unwrap_or("")),
            requirement.id,
            html::escape(token),
        ));
    }
    body.push_str("</tbody></table></div>");

    Ok(body)
}

pub async fn add(
    State(state): State<AppState>,
    session: AdminSession,
    jar: CookieJar,
    Form(form): Form<RequirementForm>,
) -> Response {
    if let Some(response) = session.guard(Capability::SettingsManage) {
        return response;
    }
    if !csrf::verify(&jar, &form.csrf) {
        return csrf_failure();
    }

    let result = add_requirement(&state, &session, &form).await;
    redirect_after(jar, result)
}

pub async fn delete(
    State(state): State<AppState>,
    session: AdminSession,
    Path(id): Path<Uuid>,
    jar: CookieJar,
    Form(form): Form<DeleteForm>,
) -> Response {
    if let Some(response) = session.guard(Capability::SettingsManage) {
        return response;
    }
    if !csrf::verify(&jar, &form.csrf) {
        return csrf_failure();
    }

    let result = sqlx::query(
        "DELETE FROM organisation_quality_requirements
         WHERE organisation_id = $1 AND id = $2",
    )
    .bind(session.organisation_id)
    .bind(id)
    .execute(&state.db)
    .await
    .map_err(AppError::from)
    .and_then(|result| {
        if result.rows_affected() == 0 {
            Err(AppError::not_found("requirement not found"))
        } else {
            Ok(())
        }
    });

    match result {
        Ok(()) => {
            write_audit(
                &state.db,
                session.organisation_id,
                session.user_id,
                "organisation.requirement_removed",
                "organisation_quality_requirement",
                Some(id),
            )
            .await;
            redirect_after(jar, Ok(()))
        }
        Err(error) => redirect_after(jar, Err(error)),
    }
}

async fn add_requirement(
    state: &AppState,
    session: &AdminSession,
    form: &RequirementForm,
) -> Result<(), AppError> {
    let catalogue = load_catalogue(&state.db, session.organisation_id).await?;
    let quality = catalogue
        .iter()
        .find(|quality| quality.id == form.quality_id)
        .ok_or_else(|| AppError::validation("choose a quality from the catalogue"))?;
    if !quality.is_active {
        return Err(AppError::validation(format!(
            "{} has been retired",
            quality.name
        )));
    }

    let operator = form.operator.trim().to_ascii_lowercase();
    let severity = form.severity.trim().to_ascii_lowercase();
    let value = parse_value(quality, &operator, &form.value)?;
    validate_requirement(quality, &operator, value.as_ref(), &severity)?;

    let reason = form.reason.trim();
    if quality.is_sensitive && reason.is_empty() {
        return Err(AppError::validation(format!(
            "{} is sensitive; record a reason or legal basis",
            quality.name
        )));
    }

    sqlx::query(
        "INSERT INTO organisation_quality_requirements
            (id, organisation_id, quality_id, operator, value, severity, reason, created_by)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8)
         ON CONFLICT (organisation_id, quality_id) DO UPDATE SET
             operator = excluded.operator, value = excluded.value,
             severity = excluded.severity, reason = excluded.reason,
             updated_at = now()",
    )
    .bind(Uuid::now_v7())
    .bind(session.organisation_id)
    .bind(form.quality_id)
    .bind(&operator)
    .bind(value.map(SqlJson))
    .bind(&severity)
    .bind((!reason.is_empty()).then(|| reason.to_owned()))
    .bind(session.user_id)
    .execute(&state.db)
    .await?;

    write_audit(
        &state.db,
        session.organisation_id,
        session.user_id,
        "organisation.requirement_saved",
        "organisation_quality_requirement",
        Some(form.quality_id),
    )
    .await;

    Ok(())
}

fn parse_value(quality: &Quality, operator: &str, text: &str) -> Result<Option<Value>, AppError> {
    if operator == "exists" {
        return Ok(None);
    }
    let text = text.trim();
    if text.is_empty() {
        return Err(AppError::validation(
            "enter a value for this rule, or choose “must be held”",
        ));
    }

    if operator == "in" {
        let values = text
            .split(',')
            .map(str::trim)
            .filter(|part| !part.is_empty())
            .map(|part| parse_scalar(quality, part))
            .collect::<Result<Vec<_>, _>>()?;
        if values.is_empty() {
            return Err(AppError::validation("enter at least one allowed value"));
        }
        return Ok(Some(Value::Array(values)));
    }

    if matches!(operator, "equals" | "not_equals") && quality.data_type == "multiselect" {
        let values = text
            .split(',')
            .map(str::trim)
            .filter(|part| !part.is_empty())
            .map(|part| Value::String(part.to_owned()))
            .collect::<Vec<_>>();
        return Ok(Some(Value::Array(values)));
    }

    Ok(Some(parse_scalar(quality, text)?))
}

fn parse_scalar(quality: &Quality, text: &str) -> Result<Value, AppError> {
    match quality.data_type.as_str() {
        "boolean" => match text.trim().to_ascii_lowercase().as_str() {
            "true" | "yes" | "held" => Ok(Value::Bool(true)),
            "false" | "no" | "not held" => Ok(Value::Bool(false)),
            _ => Err(AppError::validation(format!(
                "{} is a held/not-held check; enter true or false",
                quality.name
            ))),
        },
        _ => Ok(Value::String(text.trim().to_owned())),
    }
}

async fn load_requirements(
    db: &sqlx::PgPool,
    organisation_id: Uuid,
) -> Result<Vec<RequirementRow>, AppError> {
    Ok(sqlx::query_as::<_, RequirementRow>(
        "SELECT r.id, q.name AS quality_name, q.category,
                r.operator, r.value, r.severity, r.reason
         FROM organisation_quality_requirements r
         JOIN worker_qualities q
           ON q.organisation_id = r.organisation_id AND q.id = r.quality_id
         WHERE r.organisation_id = $1
         ORDER BY q.name",
    )
    .bind(organisation_id)
    .fetch_all(db)
    .await?)
}

fn requirement_summary(requirement: &RequirementRow) -> String {
    let value = requirement
        .value
        .as_ref()
        .map(|value| describe_value(&value.0));
    match (requirement.operator.as_str(), value) {
        ("exists", _) => format!("{} must be held", requirement.quality_name),
        ("equals", Some(value)) => format!("{} must be {value}", requirement.quality_name),
        ("not_equals", Some(value)) => {
            format!("{} must not be {value}", requirement.quality_name)
        }
        ("in", Some(value)) => format!("{} must be one of {value}", requirement.quality_name),
        ("contains", Some(value)) => format!("{} must include {value}", requirement.quality_name),
        (operator, _) => format!("{} ({operator})", requirement.quality_name),
    }
}

fn describe_value(value: &Value) -> String {
    match value {
        Value::Bool(true) => "held".to_owned(),
        Value::Bool(false) => "not held".to_owned(),
        Value::String(value) => value.replace('_', " "),
        Value::Array(values) => values
            .iter()
            .map(describe_value)
            .collect::<Vec<_>>()
            .join(", "),
        Value::Null => "not recorded".to_owned(),
        other => other.to_string(),
    }
}

fn category_label(category: &str) -> &'static str {
    match category {
        "skill" => "Skill",
        "training" => "Training",
        "qualification" => "Qualification",
        "screening" => "Screening and checks",
        "attribute" => "Worker detail",
        _ => "Preference",
    }
}

fn redirect_after(jar: CookieJar, result: Result<(), AppError>) -> Response {
    let query = match result {
        Ok(()) => "saved=requirement".to_owned(),
        Err(error) => {
            let message = match error {
                AppError::Database(_) | AppError::Internal(_) => {
                    tracing::error!(error = %error, "organisation requirement change failed");
                    "The requirement could not be saved.".to_owned()
                }
                other => other.to_string(),
            };
            format!("error={}", percent_encode(&message))
        }
    };
    (jar, Redirect::to(&format!("/admin/organisation?{query}"))).into_response()
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

fn csrf_failure() -> Response {
    (
        StatusCode::FORBIDDEN,
        Html(html::error_page(
            "Organisation preferences",
            "The form expired. Reload the page and try again.",
        )),
    )
        .into_response()
}
