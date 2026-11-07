//! Admin screens for CSV import: templates, upload, review and commit.

use std::collections::HashMap;

use axum::{
    extract::{Multipart, Path, State},
    http::{header, HeaderValue, StatusCode},
    response::{Html, IntoResponse, Redirect, Response},
    Form,
};
use axum_extra::extract::cookie::CookieJar;
use chrono::{DateTime, Utc};
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::types::Json as SqlJson;
use sqlx::FromRow;
use uuid::Uuid;

use crate::{
    admin::{
        auth::{write_audit, AdminSession},
        csrf, html,
    },
    import::{self, ObjectType},
    permissions::Capability,
    state::AppState,
};

#[derive(Debug, FromRow)]
struct BatchRow {
    id: Uuid,
    object_type: String,
    filename: String,
    row_count: i32,
    created_count: i32,
    updated_count: i32,
    error_count: i32,
    conflict_policy: String,
    status: String,
    created_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
    staged_rows: SqlJson<Value>,
}

#[derive(Debug, FromRow)]
struct ErrorRow {
    row_number: i32,
    column_name: Option<String>,
    message: String,
    raw_value: Option<String>,
}

pub async fn list(
    State(state): State<AppState>,
    session: AdminSession,
    jar: CookieJar,
) -> Response {
    if let Some(response) = session.guard(Capability::DataImport) {
        return response;
    }

    let recent = sqlx::query_as::<_, (Uuid, String, String, String, i32, i32, i32, DateTime<Utc>)>(
        "SELECT id, object_type, filename, status, row_count, created_count, updated_count, created_at
         FROM import_batches
         WHERE organisation_id = $1
         ORDER BY created_at DESC
         LIMIT 10",
    )
    .bind(session.organisation_id)
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();

    let (jar, token) = csrf::ensure(jar, state.config.environment.is_production());
    let mut body = String::new();
    body.push_str(&html::page_head(
        "Import data",
        "Upload one CSV per object. Nothing is written until you review and confirm.",
    ));
    body.push_str("<div class=\"notice\"><strong>Download the template first.</strong> Unknown columns are rejected, and the whole file fails if any row is invalid. Relationships must be imported after the contacts and organisations they reference.</div>");

    body.push_str("<h2>Objects</h2><div class=\"grid\">");
    for object in ObjectType::ALL {
        body.push_str("<div class=\"card\">");
        body.push_str(&format!("<h3>{}</h3>", html::escape(object.label())));
        body.push_str(&format!(
            "<p class=\"sub\"><a href=\"/admin/import/templates/{}\">Template</a> · <a href=\"/admin/import/spec/{}\">Column spec</a></p>",
            object.key(),
            object.key()
        ));
        body.push_str(&format!(
            "<form method=\"post\" action=\"/admin/import/upload/{}\" enctype=\"multipart/form-data\" style=\"margin-top:10px\">\
             <input type=\"hidden\" name=\"csrf\" value=\"{}\">\
             <input type=\"file\" name=\"file\" accept=\".csv,text/csv\" required style=\"margin-bottom:8px\">\
             <select name=\"conflict_policy\" style=\"width:100%; margin-bottom:8px\">\
             <option value=\"create_only\">Create only (fail on existing external_id)</option>\
             <option value=\"update_by_external_id\">Update by external_id</option>\
             </select>\
             <button type=\"submit\">Upload &amp; review</button>\
             </form></div>",
            object.key(),
            html::escape(&token)
        ));
    }
    body.push_str("</div>");

    if !recent.is_empty() {
        body.push_str("<h2>Recent imports</h2><table><thead><tr><th>When (UTC)</th><th>Object</th><th>File</th><th>Status</th><th>Rows</th><th>Created</th><th>Updated</th></tr></thead><tbody>");
        for (id, object, filename, status, rows, created, updated, created_at) in &recent {
            body.push_str(&format!(
                "<tr><td class=\"muted\">{}</td><td><a href=\"/admin/import/batch/{}\">{}</a></td><td>{}</td><td>{}</td><td>{rows}</td><td>{created}</td><td>{updated}</td></tr>",
                created_at.format("%Y-%m-%d %H:%M"),
                id,
                html::escape(object),
                html::escape(filename),
                html::escape(status),
            ));
        }
        body.push_str("</tbody></table>");
    }

    let html_body = html::layout("Import data", "/admin/import", Some(&session.who()), &body);
    (jar, Html(html_body)).into_response()
}

pub async fn template(session: AdminSession, Path(object): Path<String>) -> Response {
    if let Some(response) = session.guard(Capability::DataImport) {
        return response;
    }
    let Some(object) = ObjectType::parse(&object) else {
        return not_found();
    };

    let csv = import::template_csv(object);
    let mut response = csv.into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/csv; charset=utf-8"),
    );
    if let Ok(value) =
        HeaderValue::from_str(&format!("attachment; filename=\"{}.csv\"", object.key()))
    {
        response
            .headers_mut()
            .insert(header::CONTENT_DISPOSITION, value);
    }
    response
}

pub async fn spec(session: AdminSession, Path(object): Path<String>) -> Response {
    if let Some(response) = session.guard(Capability::DataImport) {
        return response;
    }
    let Some(object) = ObjectType::parse(&object) else {
        return not_found();
    };

    let mut body = String::new();
    body.push_str(&html::page_head(
        &format!("{} template", object.label()),
        "Exact headers. Unknown columns are rejected.",
    ));
    body.push_str(&format!(
        "<p><a class=\"button\" href=\"/admin/import/templates/{}\">Download CSV template</a></p>",
        object.key()
    ));
    body.push_str("<table><thead><tr><th>Column</th><th>Required</th><th>Format</th><th>Notes</th></tr></thead><tbody>");
    for column in object.columns() {
        let format = match column.kind {
            import::Kind::Text => "text".to_owned(),
            import::Kind::Email => "email".to_owned(),
            import::Kind::Date => "YYYY-MM-DD".to_owned(),
            import::Kind::Decimal => "decimal".to_owned(),
            import::Kind::Bool => "true/false".to_owned(),
            import::Kind::Enum(options) => options.join(" | "),
            import::Kind::ExternalId => "A-Z a-z 0-9 . _ -".to_owned(),
        };
        body.push_str(&format!(
            "<tr><td><code>{}</code></td><td>{}</td><td>{}</td><td class=\"muted\">{}</td></tr>",
            html::escape(column.name),
            if column.required { "yes" } else { "no" },
            html::escape(&format),
            html::escape(column.note),
        ));
    }
    body.push_str("</tbody></table>");
    body.push_str("<h2>Example row</h2><pre>");
    body.push_str(&html::escape(&object.example().join(", ")));
    body.push_str("</pre>");

    Html(html::layout(
        &format!("{} template", object.label()),
        "/admin/import",
        Some(&session.who()),
        &body,
    ))
    .into_response()
}

pub async fn upload(
    State(state): State<AppState>,
    session: AdminSession,
    Path(object_key): Path<String>,
    jar: CookieJar,
    mut multipart: Multipart,
) -> Response {
    if let Some(response) = session.guard(Capability::DataImport) {
        return response;
    }
    let Some(object) = ObjectType::parse(&object_key) else {
        return not_found();
    };

    let mut csrf_token = None;
    let mut policy = "create_only".to_owned();
    let mut filename = "upload.csv".to_owned();
    let mut bytes = None;

    loop {
        match multipart.next_field().await {
            Ok(Some(field)) => match field.name() {
                Some("csrf") => csrf_token = field.text().await.ok(),
                Some("conflict_policy") => {
                    if let Ok(value) = field.text().await {
                        if value == "update_by_external_id" {
                            policy = value;
                        }
                    }
                }
                Some("file") => {
                    if let Some(name) = field.file_name() {
                        filename = name.to_owned();
                    }
                    match field.bytes().await {
                        Ok(data) => bytes = Some(data),
                        Err(err) => {
                            tracing::warn!(error = %err, "import file read failed");
                            return bad_request("The uploaded file could not be read.");
                        }
                    }
                }
                _ => {}
            },
            Ok(None) => break,
            Err(err) => {
                tracing::warn!(error = %err, "multipart read failed");
                return bad_request("The upload could not be read.");
            }
        }
    }

    if !csrf::verify(&jar, csrf_token.as_deref().unwrap_or("")) {
        return csrf_failure();
    }

    let Some(bytes) = bytes else {
        return bad_request("Choose a CSV file to upload.");
    };
    if bytes.len() > import::MAX_FILE_BYTES {
        return bad_request("The file is larger than 20 MB.");
    }

    let parsed = match import::parse(object, &bytes) {
        Ok(parsed) => parsed,
        Err(message) => return bad_request(&message),
    };

    let mut errors = parsed.errors;
    if errors.is_empty() {
        match import::prevalidate(
            &state.db,
            session.organisation_id,
            object,
            &parsed.rows,
            policy == "create_only",
        )
        .await
        {
            Ok(more) => errors = more,
            Err(err) => {
                tracing::error!(error = %err, "import prevalidation failed");
                return failure("The file could not be validated.");
            }
        }
    }

    let batch_id = Uuid::now_v7();
    let hash = hex::encode(Sha256::digest(&bytes));
    let status = if errors.is_empty() {
        "validated"
    } else {
        "failed"
    };
    let staged = if errors.is_empty() {
        import::store_rows(&parsed.rows)
    } else {
        Value::Array(Vec::new())
    };

    let insert = sqlx::query(
        "INSERT INTO import_batches
            (id, organisation_id, object_type, filename, file_sha256, row_count,
             error_count, conflict_policy, status, staged_rows, created_by)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)",
    )
    .bind(batch_id)
    .bind(session.organisation_id)
    .bind(object.key())
    .bind(&filename)
    .bind(&hash)
    .bind(i32::try_from(parsed.rows.len()).unwrap_or(i32::MAX))
    .bind(i32::try_from(errors.len()).unwrap_or(i32::MAX))
    .bind(&policy)
    .bind(status)
    .bind(staged)
    .bind(session.user_id)
    .execute(&state.db)
    .await;

    if let Err(err) = insert {
        tracing::error!(error = %err, "import batch insert failed");
        return failure("The upload could not be recorded.");
    }

    for error in &errors {
        if let Err(db_error) = sqlx::query(
            "INSERT INTO import_batch_errors
                (batch_id, row_number, column_name, message, raw_value)
             VALUES ($1,$2,$3,$4,$5)",
        )
        .bind(batch_id)
        .bind(i32::try_from(error.row).unwrap_or(i32::MAX))
        .bind(&error.column)
        .bind(&error.message)
        .bind(&error.value)
        .execute(&state.db)
        .await
        {
            tracing::warn!(error = %db_error, "import error row insert failed");
        }
    }

    write_audit(
        &state.db,
        session.organisation_id,
        session.user_id,
        "import.uploaded",
        "import_batch",
        Some(batch_id),
    )
    .await;

    (Redirect::to(&format!("/admin/import/batch/{batch_id}"))).into_response()
}

pub async fn review(
    State(state): State<AppState>,
    session: AdminSession,
    Path(id): Path<Uuid>,
    jar: CookieJar,
) -> Response {
    if let Some(response) = session.guard(Capability::DataImport) {
        return response;
    }

    let batch = match load_batch(&state, &session, id).await {
        Ok(Some(batch)) => batch,
        Ok(None) => return not_found(),
        Err(err) => {
            tracing::error!(error = %err, "import batch lookup failed");
            return failure("The import could not be loaded.");
        }
    };
    let errors = load_errors(&state, id).await.unwrap_or_default();
    let (jar, token) = csrf::ensure(jar, state.config.environment.is_production());

    let mut body = String::new();
    body.push_str(&html::page_head(
        "Import review",
        &format!("{} · {}", batch.filename, batch.object_type),
    ));

    body.push_str(&format!(
        "<div class=\"grid\"><div class=\"card\"><h3>Rows</h3><p class=\"big\">{}</p></div>\
         <div class=\"card\"><h3>Errors</h3><p class=\"big\">{}</p></div>\
         <div class=\"card\"><h3>Policy</h3><p>{}</p></div>\
         <div class=\"card\"><h3>Status</h3><p>{}</p></div></div>",
        batch.row_count,
        batch.error_count,
        html::escape(&batch.conflict_policy),
        html::escape(&batch.status),
    ));

    if batch.status == "validated" {
        body.push_str("<div class=\"notice\"><strong>Dry run passed.</strong> Nothing has been written yet. Importing is all-or-nothing: if any row fails at commit, the whole file is rolled back.</div>");
        body.push_str(&format!(
            "<form method=\"post\" action=\"/admin/import/batch/{}/commit\" style=\"display:inline\">\
             <input type=\"hidden\" name=\"csrf\" value=\"{}\">\
             <button type=\"submit\">Import {} rows</button></form> ",
            batch.id,
            html::escape(&token),
            batch.row_count
        ));
        body.push_str(&format!(
            "<form method=\"post\" action=\"/admin/import/batch/{}/cancel\" style=\"display:inline\">\
             <input type=\"hidden\" name=\"csrf\" value=\"{}\">\
             <button type=\"submit\" class=\"secondary\">Cancel</button></form>",
            batch.id,
            html::escape(&token)
        ));
    } else if batch.status == "committed" {
        body.push_str(&format!(
            "<div class=\"warn-box\"><strong>Imported.</strong> Created {} and updated {}.</div>",
            batch.created_count, batch.updated_count
        ));
    } else if batch.status == "failed" {
        body.push_str("<div class=\"alert\"><strong>No rows were written.</strong> Fix the errors and upload again.</div>");
        body.push_str(&format!(
            "<form method=\"post\" action=\"/admin/import/batch/{}/cancel\" style=\"display:inline\">\
             <input type=\"hidden\" name=\"csrf\" value=\"{}\">\
             <button type=\"submit\" class=\"secondary\">Discard</button></form>",
            batch.id,
            html::escape(&token)
        ));
    }

    if !errors.is_empty() {
        body.push_str("<h2>Errors</h2>");
        body.push_str(&format!(
            "<p><a href=\"/admin/import/batch/{}/errors.csv\">Download error CSV</a></p>",
            batch.id
        ));
        body.push_str("<table><thead><tr><th>Row</th><th>Column</th><th>Message</th><th>Value</th></tr></thead><tbody>");
        for error in errors.iter().take(200) {
            body.push_str(&format!(
                "<tr><td>{}</td><td>{}</td><td>{}</td><td><code>{}</code></td></tr>",
                error.row_number,
                html::escape(error.column_name.as_deref().unwrap_or("—")),
                html::escape(&error.message),
                html::escape(error.raw_value.as_deref().unwrap_or("")),
            ));
        }
        body.push_str("</tbody></table>");
        if errors.len() > 200 {
            body.push_str("<p class=\"sub\">Showing the first 200 errors; download the CSV for all of them.</p>");
        }
    }

    body.push_str(&format!(
        "<p class=\"sub\">Uploaded {}. Staged rows expire {}.</p>",
        batch.created_at.format("%Y-%m-%d %H:%M UTC"),
        batch.expires_at.format("%Y-%m-%d %H:%M UTC"),
    ));
    body.push_str("<p><a href=\"/admin/import\">Back to import</a></p>");

    let html_body = html::layout(
        "Import review",
        "/admin/import",
        Some(&session.who()),
        &body,
    );
    (jar, Html(html_body)).into_response()
}

pub async fn commit(
    State(state): State<AppState>,
    session: AdminSession,
    Path(id): Path<Uuid>,
    jar: CookieJar,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    if let Some(response) = session.guard(Capability::DataImport) {
        return response;
    }
    if !csrf::verify(&jar, form.get("csrf").map(String::as_str).unwrap_or("")) {
        return csrf_failure();
    }

    let batch = match load_batch(&state, &session, id).await {
        Ok(Some(batch)) => batch,
        Ok(None) => return not_found(),
        Err(err) => {
            tracing::error!(error = %err, "import batch lookup failed");
            return failure("The import could not be loaded.");
        }
    };

    if batch.status != "validated" {
        return bad_request("This import is not ready to commit.");
    }
    if batch.expires_at <= Utc::now() {
        return bad_request("This import has expired. Upload the file again.");
    }
    let Some(object) = ObjectType::parse(&batch.object_type) else {
        return failure("Unknown import object.");
    };
    let rows = import::load_rows(&batch.staged_rows.0);
    let create_only = batch.conflict_policy == "create_only";

    match import::commit(
        &state.db,
        session.organisation_id,
        object,
        &rows,
        create_only,
    )
    .await
    {
        Ok((created, updated)) => {
            let _ = sqlx::query(
                "UPDATE import_batches
                 SET status='committed', created_count=$2, updated_count=$3,
                     committed_at=now(), staged_rows='[]'::jsonb
                 WHERE id=$1 AND organisation_id=$4",
            )
            .bind(id)
            .bind(i32::try_from(created).unwrap_or(i32::MAX))
            .bind(i32::try_from(updated).unwrap_or(i32::MAX))
            .bind(session.organisation_id)
            .execute(&state.db)
            .await;

            write_audit(
                &state.db,
                session.organisation_id,
                session.user_id,
                "import.committed",
                "import_batch",
                Some(id),
            )
            .await;

            (Redirect::to(&format!("/admin/import/batch/{id}"))).into_response()
        }
        Err(message) => {
            tracing::warn!(error = %message, "import commit failed");
            let _ = sqlx::query(
                "UPDATE import_batches SET status='failed', error_count=error_count+1
                 WHERE id=$1 AND organisation_id=$2",
            )
            .bind(id)
            .bind(session.organisation_id)
            .execute(&state.db)
            .await;
            let _ = sqlx::query(
                "INSERT INTO import_batch_errors
                    (batch_id, row_number, message)
                 VALUES ($1, 0, $2)",
            )
            .bind(id)
            .bind(&message)
            .execute(&state.db)
            .await;

            (Redirect::to(&format!("/admin/import/batch/{id}"))).into_response()
        }
    }
}

pub async fn cancel(
    State(state): State<AppState>,
    session: AdminSession,
    Path(id): Path<Uuid>,
    jar: CookieJar,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    if let Some(response) = session.guard(Capability::DataImport) {
        return response;
    }
    if !csrf::verify(&jar, form.get("csrf").map(String::as_str).unwrap_or("")) {
        return csrf_failure();
    }

    let _ = sqlx::query(
        "UPDATE import_batches
         SET status='cancelled', staged_rows='[]'::jsonb
         WHERE id=$1 AND organisation_id=$2 AND status <> 'committed'",
    )
    .bind(id)
    .bind(session.organisation_id)
    .execute(&state.db)
    .await;

    (Redirect::to("/admin/import")).into_response()
}

pub async fn errors_csv(
    State(state): State<AppState>,
    session: AdminSession,
    Path(id): Path<Uuid>,
) -> Response {
    if let Some(response) = session.guard(Capability::DataImport) {
        return response;
    }

    if !matches!(load_batch(&state, &session, id).await, Ok(Some(_))) {
        return not_found();
    }

    let errors = load_errors(&state, id).await.unwrap_or_default();
    let mut output = Vec::new();
    {
        let mut writer = csv::Writer::from_writer(&mut output);
        let _ = writer.write_record(["row_number", "column", "message", "value"]);
        for error in &errors {
            let _ = writer.write_record([
                error.row_number.to_string(),
                error.column_name.clone().unwrap_or_default(),
                error.message.clone(),
                error.raw_value.clone().unwrap_or_default(),
            ]);
        }
        let _ = writer.flush();
    }

    let mut response = String::from_utf8_lossy(&output)
        .into_owned()
        .into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/csv; charset=utf-8"),
    );
    if let Ok(value) =
        HeaderValue::from_str(&format!("attachment; filename=\"import-{id}-errors.csv\""))
    {
        response
            .headers_mut()
            .insert(header::CONTENT_DISPOSITION, value);
    }
    response
}

async fn load_batch(
    state: &AppState,
    session: &AdminSession,
    id: Uuid,
) -> Result<Option<BatchRow>, sqlx::Error> {
    sqlx::query_as::<_, BatchRow>(
        "SELECT id, object_type, filename, row_count, created_count, updated_count,
                error_count, conflict_policy, status, created_at,
                expires_at, staged_rows
         FROM import_batches
         WHERE id = $1 AND organisation_id = $2",
    )
    .bind(id)
    .bind(session.organisation_id)
    .fetch_optional(&state.db)
    .await
}

async fn load_errors(state: &AppState, id: Uuid) -> Result<Vec<ErrorRow>, sqlx::Error> {
    sqlx::query_as::<_, ErrorRow>(
        "SELECT row_number, column_name, message, raw_value
         FROM import_batch_errors
         WHERE batch_id = $1
         ORDER BY row_number, id",
    )
    .bind(id)
    .fetch_all(&state.db)
    .await
}

fn bad_request(message: &str) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Html(html::error_page("Import", message)),
    )
        .into_response()
}

fn not_found() -> Response {
    (
        StatusCode::NOT_FOUND,
        Html(html::error_page(
            "Import not found",
            "It may have been removed, or it belongs to another organisation.",
        )),
    )
        .into_response()
}

fn failure(message: &str) -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Html(html::error_page("Import", message)),
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
