//! Admin screens for permission sets (capabilities and personas).

use std::collections::HashMap;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{Html, IntoResponse, Redirect, Response},
    Form,
};
use axum_extra::extract::cookie::CookieJar;
use sqlx::FromRow;
use uuid::Uuid;

use crate::{
    admin::{
        auth::{write_audit, AdminSession},
        csrf, html,
    },
    permissions::{Capability, CapabilitySet},
    state::AppState,
};

#[derive(Debug, FromRow)]
struct SetRow {
    id: Uuid,
    name: String,
    description: Option<String>,
    persona_key: Option<String>,
    is_system: bool,
    is_editable: bool,
    capability_count: i64,
    user_count: i64,
}

const SELECT_SETS: &str = "
    SELECT ps.id, ps.name, ps.description, ps.persona_key, ps.is_system,
           ps.is_editable,
           (SELECT count(*) FROM permission_set_capabilities c
            WHERE c.permission_set_id = ps.id) AS capability_count,
           (SELECT count(*) FROM organisation_user_permission_sets oup
            WHERE oup.permission_set_id = ps.id) AS user_count
    FROM permission_sets ps
    WHERE ps.organisation_id = $1";

pub async fn list(
    State(state): State<AppState>,
    session: AdminSession,
    jar: CookieJar,
) -> Response {
    if let Some(response) = session.guard(Capability::PermissionSetsManage) {
        return response;
    }

    let sets =
        sqlx::query_as::<_, SetRow>(&format!("{SELECT_SETS} ORDER BY ps.sort_order, ps.name"))
            .bind(session.organisation_id)
            .fetch_all(&state.db)
            .await;

    let sets = match sets {
        Ok(sets) => sets,
        Err(err) => {
            tracing::error!(error = %err, "permission set list failed");
            return failure("Permission sets could not be loaded.");
        }
    };

    let (jar, token) = csrf::ensure(jar, state.config.environment.is_production());

    let mut body = String::new();
    body.push_str(&html::page_head(
        "Permission sets",
        "Capabilities grouped into jobs. Users may hold several sets; access is the union.",
    ));
    body.push_str(
        "<div class=\"notice\"><strong>Least privilege.</strong> Start with the OOTB persona that matches the job, then add only what is missing. The Administrator set is immutable and always has every capability.</div>",
    );
    body.push_str(
        "<p><a class=\"button\" href=\"/admin/permission-sets/new\">New permission set</a></p>",
    );

    body.push_str("<table><thead><tr><th>Name</th><th>Description</th><th>Type</th><th>Capabilities</th><th>Users</th><th></th></tr></thead><tbody>");
    for set in &sets {
        let kind = if set.persona_key.is_some() {
            "<span class=\"badge pk\">persona</span>"
        } else {
            "<span class=\"badge id\">custom</span>"
        };

        let actions = if set.is_editable {
            format!(
                "<a href=\"/admin/permission-sets/{}/edit\">Edit</a> \
                 <form method=\"post\" action=\"/admin/permission-sets/{}/delete\" style=\"display:inline\">\
                 <input type=\"hidden\" name=\"csrf\" value=\"{}\">\
                 <button class=\"link-button\" type=\"submit\" onclick=\"return confirm('Delete this permission set?')\">Delete</button></form>",
                set.id,
                set.id,
                html::escape(&token)
            )
        } else {
            "<span class=\"muted\">locked</span>".to_owned()
        };

        body.push_str(&format!(
            "<tr><td><strong>{}</strong></td><td class=\"muted\">{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
            html::escape(&set.name),
            html::escape(set.description.as_deref().unwrap_or("—")),
            kind,
            set.capability_count,
            set.user_count,
            actions,
        ));
    }
    body.push_str("</tbody></table>");

    let html_body = html::layout(
        "Permission sets",
        "/admin/permission-sets",
        Some(&session.who()),
        &body,
    );
    (jar, Html(html_body)).into_response()
}

pub async fn new_form(
    State(state): State<AppState>,
    session: AdminSession,
    jar: CookieJar,
) -> Response {
    if let Some(response) = session.guard(Capability::PermissionSetsManage) {
        return response;
    }

    let (jar, token) = csrf::ensure(jar, state.config.environment.is_production());
    let body = render_form(FormPage {
        session: &session,
        csrf: &token,
        existing: None,
        values: FormValues::default(),
        capabilities: CapabilitySet::empty(),
        errors: Vec::new(),
    });

    (jar, Html(body)).into_response()
}

pub async fn create(
    State(state): State<AppState>,
    session: AdminSession,
    jar: CookieJar,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    if let Some(response) = session.guard(Capability::PermissionSetsManage) {
        return response;
    }

    let token = form.get("csrf").cloned().unwrap_or_default();
    if !csrf::verify(&jar, &token) {
        return csrf_failure();
    }

    let values = FormValues::from_form(&form);
    let capabilities = selected_capabilities(&form);

    if let Err(errors) = validate(&values, capabilities) {
        let (jar, token) = csrf::ensure(jar, state.config.environment.is_production());
        let body = render_form(FormPage {
            session: &session,
            csrf: &token,
            existing: None,
            values,
            capabilities,
            errors,
        });
        return (StatusCode::UNPROCESSABLE_ENTITY, jar, Html(body)).into_response();
    }

    let id = Uuid::now_v7();
    let mut tx = match state.db.begin().await {
        Ok(tx) => tx,
        Err(err) => {
            tracing::error!(error = %err, "permission set transaction failed");
            return failure("The permission set could not be saved.");
        }
    };

    if let Err(err) = sqlx::query(
        "INSERT INTO permission_sets
            (id, organisation_id, name, description, is_system, is_editable, sort_order)
         VALUES ($1, $2, $3, $4, false, true, 100)",
    )
    .bind(id)
    .bind(session.organisation_id)
    .bind(values.name.trim())
    .bind(trim_optional(&values.description))
    .execute(&mut *tx)
    .await
    {
        return db_error(&session, &err, values, capabilities, None, &token);
    }

    for capability in capabilities.iter() {
        if let Err(err) = sqlx::query(
            "INSERT INTO permission_set_capabilities
                (organisation_id, permission_set_id, capability)
             VALUES ($1, $2, $3)",
        )
        .bind(session.organisation_id)
        .bind(id)
        .bind(capability.as_str())
        .execute(&mut *tx)
        .await
        {
            return db_error(&session, &err, values, capabilities, None, &token);
        }
    }

    if let Err(err) = tx.commit().await {
        tracing::error!(error = %err, "permission set commit failed");
        return failure("The permission set could not be saved.");
    }

    write_audit(
        &state.db,
        session.organisation_id,
        session.user_id,
        "permission_set.created",
        "permission_set",
        Some(id),
    )
    .await;

    (jar, Redirect::to("/admin/permission-sets")).into_response()
}

pub async fn edit_form(
    State(state): State<AppState>,
    session: AdminSession,
    Path(id): Path<Uuid>,
    jar: CookieJar,
) -> Response {
    if let Some(response) = session.guard(Capability::PermissionSetsManage) {
        return response;
    }

    let Some(set) = load_set(&state, &session, id).await else {
        return not_found();
    };

    let capabilities = match load_capabilities(&state, &session, id).await {
        Ok(capabilities) => capabilities,
        Err(err) => {
            tracing::error!(error = %err, "permission set capability lookup failed");
            return failure("The permission set could not be loaded.");
        }
    };

    let (jar, token) = csrf::ensure(jar, state.config.environment.is_production());
    let body = render_form(FormPage {
        session: &session,
        csrf: &token,
        existing: Some(&set),
        values: FormValues {
            name: set.name.clone(),
            description: set.description.clone().unwrap_or_default(),
        },
        capabilities,
        errors: Vec::new(),
    });

    (jar, Html(body)).into_response()
}

pub async fn update(
    State(state): State<AppState>,
    session: AdminSession,
    Path(id): Path<Uuid>,
    jar: CookieJar,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    if let Some(response) = session.guard(Capability::PermissionSetsManage) {
        return response;
    }

    let token = form.get("csrf").cloned().unwrap_or_default();
    if !csrf::verify(&jar, &token) {
        return csrf_failure();
    }

    let Some(set) = load_set(&state, &session, id).await else {
        return not_found();
    };
    if !set.is_editable {
        return forbidden("The Administrator set is immutable.");
    }

    let values = FormValues::from_form(&form);
    let capabilities = selected_capabilities(&form);

    if let Err(errors) = validate(&values, capabilities) {
        let (jar, token) = csrf::ensure(jar, state.config.environment.is_production());
        let body = render_form(FormPage {
            session: &session,
            csrf: &token,
            existing: Some(&set),
            values,
            capabilities,
            errors,
        });
        return (StatusCode::UNPROCESSABLE_ENTITY, jar, Html(body)).into_response();
    }

    let mut tx = match state.db.begin().await {
        Ok(tx) => tx,
        Err(err) => {
            tracing::error!(error = %err, "permission set transaction failed");
            return failure("The permission set could not be saved.");
        }
    };

    if let Err(err) = sqlx::query(
        "UPDATE permission_sets SET name = $3, description = $4
         WHERE id = $1 AND organisation_id = $2",
    )
    .bind(id)
    .bind(session.organisation_id)
    .bind(values.name.trim())
    .bind(trim_optional(&values.description))
    .execute(&mut *tx)
    .await
    {
        return db_error(&session, &err, values, capabilities, Some(&set), &token);
    }

    if let Err(err) = sqlx::query(
        "DELETE FROM permission_set_capabilities
         WHERE organisation_id = $1 AND permission_set_id = $2",
    )
    .bind(session.organisation_id)
    .bind(id)
    .execute(&mut *tx)
    .await
    {
        tracing::error!(error = %err, "clearing capabilities failed");
        return failure("The permission set could not be saved.");
    }

    for capability in capabilities.iter() {
        if let Err(err) = sqlx::query(
            "INSERT INTO permission_set_capabilities
                (organisation_id, permission_set_id, capability)
             VALUES ($1, $2, $3)",
        )
        .bind(session.organisation_id)
        .bind(id)
        .bind(capability.as_str())
        .execute(&mut *tx)
        .await
        {
            return db_error(&session, &err, values, capabilities, Some(&set), &token);
        }
    }

    if let Err(err) = tx.commit().await {
        tracing::error!(error = %err, "permission set commit failed");
        return failure("The permission set could not be saved.");
    }

    write_audit(
        &state.db,
        session.organisation_id,
        session.user_id,
        "permission_set.updated",
        "permission_set",
        Some(id),
    )
    .await;

    (jar, Redirect::to("/admin/permission-sets")).into_response()
}

pub async fn delete(
    State(state): State<AppState>,
    session: AdminSession,
    Path(id): Path<Uuid>,
    jar: CookieJar,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    if let Some(response) = session.guard(Capability::PermissionSetsManage) {
        return response;
    }

    let token = form.get("csrf").cloned().unwrap_or_default();
    if !csrf::verify(&jar, &token) {
        return csrf_failure();
    }

    let Some(set) = load_set(&state, &session, id).await else {
        return not_found();
    };
    if !set.is_editable || set.is_system {
        return forbidden("Built-in permission sets cannot be deleted. Edit them instead.");
    }

    match sqlx::query("DELETE FROM permission_sets WHERE id = $1 AND organisation_id = $2")
        .bind(id)
        .bind(session.organisation_id)
        .execute(&state.db)
        .await
    {
        Ok(_) => {
            write_audit(
                &state.db,
                session.organisation_id,
                session.user_id,
                "permission_set.deleted",
                "permission_set",
                Some(id),
            )
            .await;
            (jar, Redirect::to("/admin/permission-sets")).into_response()
        }
        Err(err) => {
            tracing::warn!(error = %err, "permission set delete failed");
            let message = if is_foreign_key_violation(&err) {
                "This set is assigned to users. Reassign them before deleting it."
            } else {
                "The permission set could not be deleted."
            };
            (
                StatusCode::UNPROCESSABLE_ENTITY,
                Html(html::error_page("Permission sets", message)),
            )
                .into_response()
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

async fn load_set(state: &AppState, session: &AdminSession, id: Uuid) -> Option<SetRow> {
    match sqlx::query_as::<_, SetRow>(&format!("{SELECT_SETS} AND ps.id = $2"))
        .bind(session.organisation_id)
        .bind(id)
        .fetch_optional(&state.db)
        .await
    {
        Ok(set) => set,
        Err(err) => {
            tracing::error!(error = %err, "permission set lookup failed");
            None
        }
    }
}

async fn load_capabilities(
    state: &AppState,
    session: &AdminSession,
    id: Uuid,
) -> Result<CapabilitySet, sqlx::Error> {
    let keys = sqlx::query_scalar::<_, String>(
        "SELECT capability FROM permission_set_capabilities
         WHERE organisation_id = $1 AND permission_set_id = $2",
    )
    .bind(session.organisation_id)
    .bind(id)
    .fetch_all(&state.db)
    .await?;

    let mut set = CapabilitySet::empty();
    for key in keys {
        if let Some(capability) = Capability::parse(&key) {
            set.insert(capability);
        }
    }
    Ok(set)
}

fn selected_capabilities(form: &HashMap<String, String>) -> CapabilitySet {
    let mut set = CapabilitySet::empty();
    for key in form.keys() {
        if let Some(raw) = key.strip_prefix("cap.") {
            if let Some(capability) = Capability::parse(raw) {
                set.insert(capability);
            }
        }
    }
    set
}

#[derive(Debug, Default, Clone)]
struct FormValues {
    name: String,
    description: String,
}

impl FormValues {
    fn from_form(form: &HashMap<String, String>) -> Self {
        Self {
            name: form.get("name").cloned().unwrap_or_default(),
            description: form.get("description").cloned().unwrap_or_default(),
        }
    }
}

fn validate(values: &FormValues, capabilities: CapabilitySet) -> Result<(), Vec<String>> {
    let mut errors = Vec::new();
    let name = values.name.trim();
    if name.is_empty() || name.chars().count() > 80 {
        errors.push("Name must be between 1 and 80 characters.".to_owned());
    }
    if capabilities.is_empty() {
        errors.push("Select at least one capability.".to_owned());
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

fn trim_optional(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

struct FormPage<'a> {
    session: &'a AdminSession,
    csrf: &'a str,
    existing: Option<&'a SetRow>,
    values: FormValues,
    #[allow(dead_code)]
    capabilities: CapabilitySet,
    errors: Vec<String>,
}

fn render_form(page: FormPage<'_>) -> String {
    let editing = page.existing.is_some();
    let immutable = page
        .existing
        .is_some_and(|set| set.is_system && !set.is_editable);

    let mut body = String::new();
    body.push_str(&html::page_head(
        if editing {
            "Edit permission set"
        } else {
            "New permission set"
        },
        "Capabilities are additive across all sets assigned to a user.",
    ));

    if immutable {
        body.push_str("<div class=\"warn-box\"><strong>Administrator is immutable.</strong> It always contains every capability and cannot be renamed, edited or deleted.</div>");
    }

    if !page.errors.is_empty() {
        body.push_str("<div class=\"alert\"><strong>Please fix the following:</strong><ul>");
        for error in &page.errors {
            body.push_str(&format!("<li>{}</li>", html::escape(error)));
        }
        body.push_str("</ul></div>");
    }

    let action = page.existing.map_or_else(
        || "/admin/permission-sets".to_owned(),
        |set| format!("/admin/permission-sets/{}", set.id),
    );

    if immutable {
        body.push_str("<div class=\"form wide\">");
    } else {
        body.push_str(&format!(
            "<form class=\"form wide\" method=\"post\" action=\"{action}\">"
        ));
        body.push_str(&format!(
            "<input type=\"hidden\" name=\"csrf\" value=\"{}\">",
            html::escape(page.csrf)
        ));
    }

    body.push_str("<label for=\"name\">Name</label>");
    if immutable {
        body.push_str(&format!(
            "<input id=\"name\" value=\"{}\" disabled>",
            html::escape(&page.values.name)
        ));
    } else {
        body.push_str(&format!(
            "<input id=\"name\" name=\"name\" value=\"{}\" required maxlength=\"80\">",
            html::escape(&page.values.name)
        ));
    }

    body.push_str("<label for=\"description\">Description</label>");
    if immutable {
        body.push_str(&format!(
            "<input id=\"description\" value=\"{}\" disabled>",
            html::escape(&page.values.description)
        ));
    } else {
        body.push_str(&format!(
            "<input id=\"description\" name=\"description\" value=\"{}\">",
            html::escape(&page.values.description)
        ));
    }

    let selected = page.capabilities;
    let mut current_category = "";
    for capability in Capability::ALL {
        if capability.category() != current_category {
            if !current_category.is_empty() {
                body.push_str("</div>");
            }
            current_category = capability.category();
            body.push_str("<h2>");
            body.push_str(&html::escape(current_category));
            body.push_str("</h2><div class=\"grid-2\">");
        }

        body.push_str(&format!(
            "<div class=\"checkbox\"><input id=\"cap_{key}\" name=\"cap.{key}\" type=\"checkbox\"{}{}>\
             <label for=\"cap_{key}\">{label}</label></div>",
            if selected.contains(capability) {
                " checked"
            } else {
                ""
            },
            if immutable { " disabled" } else { "" },
            key = capability.as_str(),
            label = html::escape(capability.label()),
        ));
    }
    if !current_category.is_empty() {
        body.push_str("</div>");
    }

    if immutable {
        body.push_str("</div>");
    } else {
        body.push_str("<button type=\"submit\">Save permission set</button></form>");
    }

    html::layout(
        if editing {
            "Edit permission set"
        } else {
            "New permission set"
        },
        "/admin/permission-sets",
        Some(&page.session.who()),
        &body,
    )
}

fn db_error(
    session: &AdminSession,
    err: &sqlx::Error,
    values: FormValues,
    capabilities: CapabilitySet,
    existing: Option<&SetRow>,
    csrf_token: &str,
) -> Response {
    tracing::warn!(error = %err, "permission set write failed");
    let message = if let sqlx::Error::Database(database) = err {
        if database.code().as_deref() == Some("23505") {
            "A permission set with that name already exists.".to_owned()
        } else {
            "The permission set could not be saved.".to_owned()
        }
    } else {
        "The permission set could not be saved.".to_owned()
    };

    let body = render_form(FormPage {
        session,
        csrf: csrf_token,
        existing,
        values,
        capabilities,
        errors: vec![message],
    });
    (StatusCode::UNPROCESSABLE_ENTITY, Html(body)).into_response()
}

fn failure(message: &str) -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Html(html::error_page("Permission sets", message)),
    )
        .into_response()
}

fn not_found() -> Response {
    (
        StatusCode::NOT_FOUND,
        Html(html::error_page(
            "Permission set not found",
            "It may have been deleted, or it belongs to another organisation.",
        )),
    )
        .into_response()
}

fn forbidden(message: &str) -> Response {
    (
        StatusCode::FORBIDDEN,
        Html(html::error_page("Not allowed", message)),
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

fn is_foreign_key_violation(error: &sqlx::Error) -> bool {
    matches!(error, sqlx::Error::Database(database) if database.code().as_deref() == Some("23503"))
}
