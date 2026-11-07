//! Organisation settings: name and logo used by desktop/mobile clients.

use axum::{
    extract::{Multipart, Query, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{Html, IntoResponse, Redirect, Response},
    Form,
};
use axum_extra::extract::cookie::CookieJar;
use serde::Deserialize;

use crate::{
    admin::{
        auth::{write_audit, AdminSession},
        csrf, html,
    },
    organisation::{load_logo, load_profile, save_logo, update_name as save_name, validate_logo},
    permissions::Capability,
    state::AppState,
};

#[derive(Debug, Default, Deserialize)]
pub struct PageParams {
    saved: Option<String>,
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct NameForm {
    csrf: String,
    name: String,
}

#[derive(Debug, Deserialize)]
pub struct CsrfForm {
    csrf: String,
}

pub async fn page(
    State(state): State<AppState>,
    session: AdminSession,
    jar: CookieJar,
    Query(params): Query<PageParams>,
) -> Response {
    if let Some(response) = session.guard(Capability::SettingsManage) {
        return response;
    }

    let profile = match load_profile(&state.db, session.organisation_id).await {
        Ok(Some(profile)) => profile,
        Ok(None) => return failure("The organisation could not be loaded."),
        Err(err) => {
            tracing::error!(error = %err, "organisation profile query failed");
            return failure("The organisation could not be loaded.");
        }
    };

    let (jar, token) = csrf::ensure(jar, state.config.environment.is_production());
    let mut body = String::new();
    body.push_str(&html::page_head(
        "Organisation",
        "Name and logo shown in the desktop and mobile clients.",
    ));

    if params.saved.is_some() {
        body.push_str("<div class=\"notice\"><strong>Saved.</strong> The change is available to clients immediately.</div>");
    }
    if params.error.is_some() {
        body.push_str(&format!(
            "<div class=\"alert\">{}</div>",
            html::escape(error_message(params.error.as_deref().unwrap_or_default()))
        ));
    }

    body.push_str("<h2>Identity</h2>");
    body.push_str("<form class=\"form wide\" method=\"post\" action=\"/admin/organisation\">");
    body.push_str(&format!(
        "<input type=\"hidden\" name=\"csrf\" value=\"{}\">",
        html::escape(&token)
    ));
    body.push_str("<label for=\"name\">Organisation name</label>");
    body.push_str(&format!(
        "<input id=\"name\" name=\"name\" type=\"text\" required maxlength=\"200\" value=\"{}\">",
        html::escape(&profile.name)
    ));
    body.push_str("<p class=\"hint\">Slug <code>");
    body.push_str(&html::escape(&profile.slug));
    body.push_str("</code> · edition <code>");
    body.push_str(&html::escape(&profile.edition));
    body.push_str("</code>. The slug is part of API URLs and is not changed here.</p>");
    body.push_str("<button type=\"submit\">Save name</button></form>");

    body.push_str("<h2>Logo</h2>");
    body.push_str("<div class=\"card\">");
    if let Some(logo) = &profile.logo {
        body.push_str(&format!(
            "<p><img src=\"/admin/organisation/logo?v={}\" alt=\"{}\" style=\"max-width:320px;max-height:140px;object-fit:contain;background:var(--bg);border:1px solid var(--border);border-radius:8px;padding:8px\"></p>",
            html::escape(&logo.sha256),
            html::escape(&profile.name),
        ));
        body.push_str(&format!(
            "<p class=\"sub\"><code>{}</code> · {} · updated {}</p>",
            html::escape(&logo.content_type),
            html::escape(logo.filename.as_deref().unwrap_or("unnamed")),
            logo.updated_at.format("%Y-%m-%d %H:%M UTC"),
        ));
    } else {
        body.push_str("<p class=\"sub\">No logo uploaded. Clients will fall back to the organisation name.</p>");
    }

    body.push_str(&format!(
        "<form method=\"post\" action=\"/admin/organisation/logo\" enctype=\"multipart/form-data\" style=\"margin-top:14px\">\
         <input type=\"hidden\" name=\"csrf\" value=\"{}\">\
         <input type=\"file\" name=\"logo\" accept=\"image/png,image/jpeg,image/webp,image/gif\" required>\
         <p class=\"hint\">PNG, JPEG, WebP or GIF, up to 2 MB.</p>\
         <button type=\"submit\">Upload logo</button></form>",
        html::escape(&token)
    ));

    if profile.logo.is_some() {
        body.push_str(&format!(
            "<form method=\"post\" action=\"/admin/organisation/logo/delete\" style=\"margin-top:8px\">\
             <input type=\"hidden\" name=\"csrf\" value=\"{}\">\
             <button class=\"secondary\" type=\"submit\">Remove logo</button></form>",
            html::escape(&token)
        ));
    }
    body.push_str("</div>");

    body.push_str("<h2>Client access</h2>");
    body.push_str("<p class=\"sub\">Clients fetch the profile with <code>GET /api/v1/organisation</code> and the logo bytes with <code>GET /api/v1/organisation/logo</code>, both using their bearer token.</p>");

    let html_body = html::layout(
        "Organisation",
        "/admin/organisation",
        Some(&session.who()),
        &body,
    );
    (jar, Html(html_body)).into_response()
}

pub async fn update_name(
    State(state): State<AppState>,
    session: AdminSession,
    jar: CookieJar,
    Form(form): Form<NameForm>,
) -> Response {
    if let Some(response) = session.guard(Capability::SettingsManage) {
        return response;
    }
    if !csrf::verify(&jar, &form.csrf) {
        return csrf_failure();
    }

    let name = form.name.trim().to_owned();
    if name.is_empty() || name.chars().count() > 200 {
        return Redirect::to("/admin/organisation?error=invalid_name").into_response();
    }

    match save_name(&state.db, session.organisation_id, &name).await {
        Ok(0) => failure("The organisation could not be updated."),
        Ok(_) => {
            write_audit(
                &state.db,
                session.organisation_id,
                session.user_id,
                "organisation.renamed",
                "organisation",
                Some(session.organisation_id),
            )
            .await;
            Redirect::to("/admin/organisation?saved=name").into_response()
        }
        Err(err) => {
            tracing::error!(error = %err, "organisation name update failed");
            failure("The organisation could not be updated.")
        }
    }
}

pub async fn upload_logo(
    State(state): State<AppState>,
    session: AdminSession,
    jar: CookieJar,
    mut multipart: Multipart,
) -> Response {
    if let Some(response) = session.guard(Capability::SettingsManage) {
        return response;
    }

    let mut csrf_token = None;
    let mut bytes = None;
    let mut filename = None;
    loop {
        match multipart.next_field().await {
            Ok(Some(field)) => match field.name() {
                Some("csrf") => csrf_token = field.text().await.ok(),
                Some("logo") => {
                    filename = field.file_name().map(sanitise_filename);
                    match field.bytes().await {
                        Ok(data) => bytes = Some(data.to_vec()),
                        Err(err) => {
                            tracing::warn!(error = %err, "logo read failed");
                            return Redirect::to("/admin/organisation?error=invalid_logo")
                                .into_response();
                        }
                    }
                }
                _ => {}
            },
            Ok(None) => break,
            Err(err) => {
                tracing::warn!(error = %err, "logo multipart read failed");
                return Redirect::to("/admin/organisation?error=invalid_logo").into_response();
            }
        }
    }

    if !csrf::verify(&jar, csrf_token.as_deref().unwrap_or("")) {
        return csrf_failure();
    }
    let Some(bytes) = bytes else {
        return Redirect::to("/admin/organisation?error=missing_logo").into_response();
    };
    let content_type = match validate_logo(&bytes) {
        Ok(content_type) => content_type,
        Err(_) => return Redirect::to("/admin/organisation?error=invalid_logo").into_response(),
    };

    match save_logo(
        &state.db,
        session.organisation_id,
        session.user_id,
        &bytes,
        content_type,
        filename.as_deref(),
    )
    .await
    {
        Ok(logo) => {
            write_audit(
                &state.db,
                session.organisation_id,
                session.user_id,
                "organisation.logo_uploaded",
                "organisation",
                Some(session.organisation_id),
            )
            .await;
            tracing::info!(
                organisation_id = %session.organisation_id,
                sha256 = %logo.sha256,
                bytes = bytes.len(),
                "organisation logo updated"
            );
            Redirect::to("/admin/organisation?saved=logo").into_response()
        }
        Err(err) => {
            tracing::error!(error = %err, "logo save failed");
            failure("The logo could not be saved.")
        }
    }
}

pub async fn delete_logo(
    State(state): State<AppState>,
    session: AdminSession,
    jar: CookieJar,
    Form(form): Form<CsrfForm>,
) -> Response {
    if let Some(response) = session.guard(Capability::SettingsManage) {
        return response;
    }
    if !csrf::verify(&jar, &form.csrf) {
        return csrf_failure();
    }

    match crate::organisation::delete_logo(&state.db, session.organisation_id).await {
        Ok(0) => Redirect::to("/admin/organisation?error=missing_logo").into_response(),
        Ok(_) => {
            write_audit(
                &state.db,
                session.organisation_id,
                session.user_id,
                "organisation.logo_deleted",
                "organisation",
                Some(session.organisation_id),
            )
            .await;
            Redirect::to("/admin/organisation?saved=logo_deleted").into_response()
        }
        Err(err) => {
            tracing::error!(error = %err, "logo delete failed");
            failure("The logo could not be removed.")
        }
    }
}

pub async fn logo(State(state): State<AppState>, session: AdminSession) -> Response {
    if let Some(response) = session.guard(Capability::SettingsManage) {
        return response;
    }

    let logo = match load_logo(&state.db, session.organisation_id).await {
        Ok(Some(logo)) => logo,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(err) => {
            tracing::error!(error = %err, "logo query failed");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(&logo.content_type)
            .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream")),
    );
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, no-cache"),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    (headers, logo.bytes).into_response()
}

fn error_message(code: &str) -> &'static str {
    match code {
        "invalid_name" => "Organisation name must be between 1 and 200 characters.",
        "invalid_logo" => "The logo must be a PNG, JPEG, WebP or GIF image up to 2 MB.",
        "missing_logo" => "Choose a logo file before uploading, or there is no logo to remove.",
        _ => "The requested change could not be applied.",
    }
}

fn sanitise_filename(name: &str) -> String {
    std::path::Path::new(name)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("logo")
        .chars()
        .take(255)
        .collect()
}

fn csrf_failure() -> Response {
    (
        StatusCode::FORBIDDEN,
        Html(html::error_page(
            "Organisation",
            "The form expired. Reload the page and try again.",
        )),
    )
        .into_response()
}

fn failure(message: &str) -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Html(html::error_page("Organisation", message)),
    )
        .into_response()
}
