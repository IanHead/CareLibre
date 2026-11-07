//! Admin screens for user-defined custom fields.
//!
//! Definitions are metadata. Creating or editing one never issues DDL; values
//! are stored in `custom_field_values` and validated by both this form handler
//! and a database trigger.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{Html, IntoResponse, Redirect, Response},
    Form,
};
use axum_extra::extract::cookie::CookieJar;
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::FromRow;
use uuid::Uuid;

use crate::{
    admin::{auth::AdminSession, csrf, html},
    custom_fields::{self, DefinitionDraft, FieldRules, FieldType},
    state::AppState,
};

#[derive(Debug, FromRow)]
struct DefinitionRow {
    id: Uuid,
    entity_type: String,
    key: String,
    label: String,
    help_text: Option<String>,
    data_type: String,
    is_required: bool,
    min_length: Option<i32>,
    max_length: Option<i32>,
    min_value: Option<f64>,
    max_value: Option<f64>,
    pattern: Option<String>,
    options: Option<Value>,
    default_value: Option<Value>,
    sort_order: i32,
    is_active: bool,
    value_count: i64,
}

#[derive(Debug, FromRow)]
struct EntityOption {
    entity_type: String,
    label: String,
}

const SELECT_DEFINITIONS: &str = "
    SELECT d.id, d.entity_type, d.key, d.label, d.help_text, d.data_type,
           d.is_required, d.min_length, d.max_length,
           d.min_value::float8 AS min_value, d.max_value::float8 AS max_value,
           d.pattern, d.options, d.default_value, d.sort_order, d.is_active,
           (SELECT count(*) FROM custom_field_values v WHERE v.definition_id = d.id) AS value_count
    FROM custom_field_definitions d
    WHERE d.organisation_id = $1";

#[derive(Debug, Default, Deserialize)]
pub struct ListParams {
    saved: Option<String>,
    archived: Option<String>,
    deleted: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct NewParams {
    entity: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct DefinitionForm {
    csrf: String,
    entity_type: String,
    key: String,
    label: String,
    help_text: Option<String>,
    data_type: String,
    is_required: Option<String>,
    min_length: Option<String>,
    max_length: Option<String>,
    min_value: Option<String>,
    max_value: Option<String>,
    pattern: Option<String>,
    options: Option<String>,
    default_value: Option<String>,
    sort_order: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ConfirmForm {
    csrf: String,
    confirm: Option<String>,
    confirm_key: Option<String>,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

pub async fn list(
    State(state): State<AppState>,
    session: AdminSession,
    Query(params): Query<ListParams>,
) -> Response {
    let definitions = sqlx::query_as::<_, DefinitionRow>(&format!(
        "{} ORDER BY d.entity_type, d.sort_order, d.key",
        SELECT_DEFINITIONS
    ))
    .bind(session.organisation_id)
    .fetch_all(&state.db)
    .await;

    let entities = match entity_options(&state).await {
        Ok(entities) => entities,
        Err(err) => {
            tracing::error!(error = %err, "custom field entity lookup failed");
            return failure("Custom fields could not be loaded.");
        }
    };

    let definitions = match definitions {
        Ok(definitions) => definitions,
        Err(err) => {
            tracing::error!(error = %err, "custom field list failed");
            return failure("Custom fields could not be loaded.");
        }
    };

    let mut body = String::new();
    body.push_str(&html::page_head(
        "Custom fields",
        "Extend every object with fields your organisation needs.",
    ));

    body.push_str(
        "<div class=\"notice\"><strong>Use sparingly.</strong> Custom fields are stored as metadata, not schema — they cannot corrupt the database. But every field has a cost: it must be maintained, included in exports and reports, and understood by staff. Prefer a small set of well-named fields over many speculative ones. If a concept applies to all providers, it belongs in the core model, not here.</div>",
    );

    for (flag, message) in [
        (params.saved.as_deref(), "Custom field saved."),
        (params.archived.as_deref(), "Custom field updated."),
        (
            params.deleted.as_deref(),
            "Custom field and its stored values were deleted.",
        ),
    ] {
        if flag.is_some() {
            body.push_str(&format!(
                "<div class=\"warn-box\">{}</div>",
                html::escape(message)
            ));
        }
    }

    body.push_str(
        "<p><a class=\"button\" href=\"/admin/custom-fields/new\">Add a custom field</a></p>",
    );

    if definitions.is_empty() {
        body.push_str("<p class=\"sub\">No custom fields yet.</p>");
        return page("Custom fields", &session, body);
    }

    let mut labels = std::collections::HashMap::new();
    for entity in &entities {
        labels.insert(entity.entity_type.clone(), entity.label.clone());
    }

    body.push_str("<table><thead><tr><th>Object</th><th>Key</th><th>Label</th><th>Type</th><th>Required</th><th>Status</th><th>Values</th><th></th></tr></thead><tbody>");

    for definition in &definitions {
        let entity_label = labels
            .get(&definition.entity_type)
            .map_or(definition.entity_type.as_str(), String::as_str);
        let type_label = match FieldType::parse(&definition.data_type) {
            Some(kind) => kind.label(),
            None => definition.data_type.as_str(),
        };

        body.push_str(&format!(
            "<tr><td>{}</td><td><code>{}</code></td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td><a href=\"/admin/custom-fields/{}/edit\">Edit</a></td></tr>",
            html::escape(entity_label),
            html::escape(&definition.key),
            html::escape(&definition.label),
            html::escape(type_label),
            if definition.is_required {
                "<span class=\"badge yes\">required</span>"
            } else {
                "<span class=\"muted\">optional</span>"
            },
            if definition.is_active {
                "<span class=\"badge pk\">active</span>"
            } else {
                "<span class=\"badge id\">archived</span>"
            },
            definition.value_count,
            definition.id,
        ));
    }

    body.push_str("</tbody></table>");
    page("Custom fields", &session, body)
}

pub async fn new_form(
    State(state): State<AppState>,
    session: AdminSession,
    jar: CookieJar,
    Query(params): Query<NewParams>,
) -> Response {
    let entities = match entity_options(&state).await {
        Ok(entities) => entities,
        Err(err) => {
            tracing::error!(error = %err, "custom field entity lookup failed");
            return failure("Custom fields could not be loaded.");
        }
    };

    let entity_type = params
        .entity
        .filter(|candidate| {
            entities
                .iter()
                .any(|entity| &entity.entity_type == candidate)
        })
        .unwrap_or_else(|| "contact".to_owned());

    let (jar, token) = csrf::ensure(jar, state.config.environment.is_production());

    let values = FormValues {
        entity_type,
        data_type: FieldType::Text.as_str().to_owned(),
        sort_order: "0".to_owned(),
        ..FormValues::default()
    };

    let body = render_form(FormPage {
        session: &session,
        csrf: &token,
        entities: &entities,
        values,
        errors: Vec::new(),
        existing: None,
    });

    (jar, Html(body)).into_response()
}

pub async fn create(
    State(state): State<AppState>,
    session: AdminSession,
    jar: CookieJar,
    Form(form): Form<DefinitionForm>,
) -> Response {
    if !csrf::verify(&jar, &form.csrf) {
        return csrf_failure();
    }

    let entities = match entity_options(&state).await {
        Ok(entities) => entities,
        Err(err) => {
            tracing::error!(error = %err, "custom field entity lookup failed");
            return failure("Custom fields could not be loaded.");
        }
    };

    let (jar, token) = csrf::ensure(jar, state.config.environment.is_production());

    match parse_form(&form, &entities) {
        Ok((draft, sort_order)) => {
            let result = sqlx::query(
                "INSERT INTO custom_field_definitions
                    (id, organisation_id, entity_type, key, label, help_text, data_type,
                     is_required, min_length, max_length, min_value, max_value, pattern,
                     options, default_value, sort_order)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16)",
            )
            .bind(Uuid::now_v7())
            .bind(session.organisation_id)
            .bind(&draft.entity_type)
            .bind(&draft.key)
            .bind(&draft.label)
            .bind(
                form.help_text
                    .as_deref()
                    .map(str::trim)
                    .filter(|text| !text.is_empty()),
            )
            .bind(draft.data_type.as_str())
            .bind(draft.rules.required)
            .bind(draft.rules.min_length)
            .bind(draft.rules.max_length)
            .bind(draft.rules.min_value)
            .bind(draft.rules.max_value)
            .bind(draft.rules.pattern.as_deref())
            .bind(draft.rules.options.as_ref().map(|options| json!(options)))
            .bind(draft.default_value.clone())
            .bind(sort_order)
            .execute(&state.db)
            .await;

            match result {
                Ok(_) => (jar, Redirect::to("/admin/custom-fields?saved=1")).into_response(),
                Err(err) => {
                    let message = friendly_db_error(&err);
                    tracing::warn!(error = %err, "custom field insert failed");
                    let body = render_form(FormPage {
                        session: &session,
                        csrf: &token,
                        entities: &entities,
                        values: FormValues::from_form(&form),
                        errors: vec![message],
                        existing: None,
                    });
                    (StatusCode::UNPROCESSABLE_ENTITY, jar, Html(body)).into_response()
                }
            }
        }
        Err(errors) => {
            let body = render_form(FormPage {
                session: &session,
                csrf: &token,
                entities: &entities,
                values: FormValues::from_form(&form),
                errors,
                existing: None,
            });
            (StatusCode::UNPROCESSABLE_ENTITY, jar, Html(body)).into_response()
        }
    }
}

pub async fn edit_form(
    State(state): State<AppState>,
    session: AdminSession,
    Path(id): Path<Uuid>,
    jar: CookieJar,
) -> Response {
    let Some(definition) = load_definition(&state, &session, id).await else {
        return not_found();
    };
    let entities = match entity_options(&state).await {
        Ok(entities) => entities,
        Err(err) => {
            tracing::error!(error = %err, "custom field entity lookup failed");
            return failure("Custom fields could not be loaded.");
        }
    };

    let (jar, token) = csrf::ensure(jar, state.config.environment.is_production());
    let body = render_form(FormPage {
        session: &session,
        csrf: &token,
        entities: &entities,
        values: FormValues::from_row(&definition),
        errors: Vec::new(),
        existing: Some(&definition),
    });

    (jar, Html(body)).into_response()
}

pub async fn update(
    State(state): State<AppState>,
    session: AdminSession,
    Path(id): Path<Uuid>,
    jar: CookieJar,
    Form(form): Form<DefinitionForm>,
) -> Response {
    if !csrf::verify(&jar, &form.csrf) {
        return csrf_failure();
    }

    let Some(definition) = load_definition(&state, &session, id).await else {
        return not_found();
    };
    let entities = match entity_options(&state).await {
        Ok(entities) => entities,
        Err(err) => {
            tracing::error!(error = %err, "custom field entity lookup failed");
            return failure("Custom fields could not be loaded.");
        }
    };

    let (jar, token) = csrf::ensure(jar, state.config.environment.is_production());

    match parse_form(&form, &entities) {
        Ok((draft, sort_order)) => {
            if definition.value_count > 0
                && (definition.entity_type != draft.entity_type
                    || definition.data_type != draft.data_type.as_str())
            {
                let body = render_form(FormPage {
                    session: &session,
                    csrf: &token,
                    entities: &entities,
                    values: FormValues::from_form(&form),
                    errors: vec![
                        "Object type and data type cannot change while values exist. Archive this field and create a new one instead.".to_owned(),
                    ],
                    existing: Some(&definition),
                });
                return (StatusCode::UNPROCESSABLE_ENTITY, jar, Html(body)).into_response();
            }

            let result = sqlx::query(
                "UPDATE custom_field_definitions
                 SET entity_type = $3, key = $4, label = $5, help_text = $6,
                     data_type = $7, is_required = $8, min_length = $9, max_length = $10,
                     min_value = $11, max_value = $12, pattern = $13, options = $14,
                     default_value = $15, sort_order = $16
                 WHERE id = $1 AND organisation_id = $2",
            )
            .bind(id)
            .bind(session.organisation_id)
            .bind(&draft.entity_type)
            .bind(&draft.key)
            .bind(&draft.label)
            .bind(
                form.help_text
                    .as_deref()
                    .map(str::trim)
                    .filter(|text| !text.is_empty()),
            )
            .bind(draft.data_type.as_str())
            .bind(draft.rules.required)
            .bind(draft.rules.min_length)
            .bind(draft.rules.max_length)
            .bind(draft.rules.min_value)
            .bind(draft.rules.max_value)
            .bind(draft.rules.pattern.as_deref())
            .bind(draft.rules.options.as_ref().map(|options| json!(options)))
            .bind(draft.default_value.clone())
            .bind(sort_order)
            .execute(&state.db)
            .await;

            match result {
                Ok(_) => (jar, Redirect::to("/admin/custom-fields?saved=1")).into_response(),
                Err(err) => {
                    let message = friendly_db_error(&err);
                    tracing::warn!(error = %err, "custom field update failed");
                    let body = render_form(FormPage {
                        session: &session,
                        csrf: &token,
                        entities: &entities,
                        values: FormValues::from_form(&form),
                        errors: vec![message],
                        existing: Some(&definition),
                    });
                    (StatusCode::UNPROCESSABLE_ENTITY, jar, Html(body)).into_response()
                }
            }
        }
        Err(errors) => {
            let body = render_form(FormPage {
                session: &session,
                csrf: &token,
                entities: &entities,
                values: FormValues::from_form(&form),
                errors,
                existing: Some(&definition),
            });
            (StatusCode::UNPROCESSABLE_ENTITY, jar, Html(body)).into_response()
        }
    }
}

pub async fn archive(
    State(state): State<AppState>,
    session: AdminSession,
    Path(id): Path<Uuid>,
    jar: CookieJar,
    Form(form): Form<ConfirmForm>,
) -> Response {
    if !csrf::verify(&jar, &form.csrf) {
        return csrf_failure();
    }

    if let Err(err) = sqlx::query(
        "UPDATE custom_field_definitions SET is_active = NOT is_active
         WHERE id = $1 AND organisation_id = $2",
    )
    .bind(id)
    .bind(session.organisation_id)
    .execute(&state.db)
    .await
    {
        tracing::error!(error = %err, "custom field archive toggle failed");
        return failure("The field could not be updated.");
    }

    (
        jar,
        Redirect::to(&format!("/admin/custom-fields/{id}/edit?archived=1")),
    )
        .into_response()
}

pub async fn delete(
    State(state): State<AppState>,
    session: AdminSession,
    Path(id): Path<Uuid>,
    jar: CookieJar,
    Form(form): Form<ConfirmForm>,
) -> Response {
    if !csrf::verify(&jar, &form.csrf) {
        return csrf_failure();
    }

    let Some(definition) = load_definition(&state, &session, id).await else {
        return not_found();
    };

    if form.confirm.as_deref() != Some("yes")
        || form.confirm_key.as_deref().map(str::trim) != Some(definition.key.as_str())
    {
        let entities = match entity_options(&state).await {
            Ok(entities) => entities,
            Err(_) => return failure("Custom fields could not be loaded."),
        };
        let (jar, token) = csrf::ensure(jar, state.config.environment.is_production());
        let body = render_form(FormPage {
            session: &session,
            csrf: &token,
            entities: &entities,
            values: FormValues::from_row(&definition),
            errors: vec![
                "Deletion not confirmed. Tick the box and type the field key exactly to confirm."
                    .to_owned(),
            ],
            existing: Some(&definition),
        });
        return (StatusCode::UNPROCESSABLE_ENTITY, jar, Html(body)).into_response();
    }

    if let Err(err) =
        sqlx::query("DELETE FROM custom_field_definitions WHERE id = $1 AND organisation_id = $2")
            .bind(id)
            .bind(session.organisation_id)
            .execute(&state.db)
            .await
    {
        tracing::error!(error = %err, "custom field delete failed");
        return failure("The field could not be deleted.");
    }

    (jar, Redirect::to("/admin/custom-fields?deleted=1")).into_response()
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

async fn entity_options(state: &AppState) -> Result<Vec<EntityOption>, sqlx::Error> {
    sqlx::query_as::<_, EntityOption>(
        "SELECT entity_type, label FROM custom_field_entities ORDER BY sort_order, label",
    )
    .fetch_all(&state.db)
    .await
}

async fn load_definition(
    state: &AppState,
    session: &AdminSession,
    id: Uuid,
) -> Option<DefinitionRow> {
    match sqlx::query_as::<_, DefinitionRow>(&format!("{} AND d.id = $2", SELECT_DEFINITIONS))
        .bind(session.organisation_id)
        .bind(id)
        .fetch_optional(&state.db)
        .await
    {
        Ok(definition) => definition,
        Err(err) => {
            tracing::error!(error = %err, "custom field lookup failed");
            None
        }
    }
}

fn parse_form(
    form: &DefinitionForm,
    entities: &[EntityOption],
) -> Result<(DefinitionDraft, i32), Vec<String>> {
    let mut errors = Vec::new();

    let entity_type = form.entity_type.trim().to_owned();
    if !entities
        .iter()
        .any(|entity| entity.entity_type == entity_type)
    {
        errors.push("Choose a valid object type.".to_owned());
    }

    let data_type = match FieldType::parse(form.data_type.trim()) {
        Some(kind) => kind,
        None => {
            errors.push("Choose a valid data type.".to_owned());
            FieldType::Text
        }
    };

    let min_length =
        parse_optional::<i32>("Minimum length", form.min_length.as_deref(), &mut errors);
    let max_length =
        parse_optional::<i32>("Maximum length", form.max_length.as_deref(), &mut errors);
    let min_value = parse_optional::<f64>("Minimum value", form.min_value.as_deref(), &mut errors);
    let max_value = parse_optional::<f64>("Maximum value", form.max_value.as_deref(), &mut errors);
    let sort_order = form
        .sort_order
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map_or(0, |value| value.parse::<i32>().unwrap_or(0));

    let options = form
        .options
        .as_deref()
        .map(parse_options)
        .filter(|options| !options.is_empty());
    let default_value = match parse_default(data_type, form.default_value.as_deref().unwrap_or(""))
    {
        Ok(value) => value,
        Err(message) => {
            errors.push(message);
            None
        }
    };

    let draft = DefinitionDraft {
        entity_type,
        key: form.key.trim().to_owned(),
        label: form.label.trim().to_owned(),
        data_type,
        rules: FieldRules {
            required: form.is_required.is_some(),
            min_length,
            max_length,
            min_value,
            max_value,
            pattern: trim_optional(form.pattern.as_deref()),
            options,
        },
        default_value,
    };

    errors.extend(custom_fields::validate_definition(&draft));

    if errors.is_empty() {
        Ok((draft, sort_order))
    } else {
        Err(errors)
    }
}

fn parse_optional<T: std::str::FromStr>(
    label: &str,
    raw: Option<&str>,
    errors: &mut Vec<String>,
) -> Option<T> {
    match raw.map(str::trim).filter(|value| !value.is_empty()) {
        None => None,
        Some(value) => match value.parse::<T>() {
            Ok(parsed) => Some(parsed),
            Err(_) => {
                errors.push(format!("{label} is not a valid number."));
                None
            }
        },
    }
}

fn parse_options(raw: &str) -> Vec<String> {
    raw.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn parse_default(kind: FieldType, raw: &str) -> Result<Option<Value>, String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(None);
    }

    match kind {
        FieldType::Integer => raw
            .parse::<i64>()
            .map(|value| Some(json!(value)))
            .map_err(|_| "Default value must be a whole number.".to_owned()),
        FieldType::Decimal => raw
            .parse::<f64>()
            .map(|value| Some(json!(value)))
            .map_err(|_| "Default value must be a number.".to_owned()),
        FieldType::Boolean => match raw {
            "true" => Ok(Some(json!(true))),
            "false" => Ok(Some(json!(false))),
            _ => Err("Default value must be true or false.".to_owned()),
        },
        FieldType::MultiSelect => Ok(Some(json!(raw
            .split(',')
            .map(str::trim)
            .filter(|item| !item.is_empty())
            .collect::<Vec<_>>()))),
        _ => Ok(Some(json!(raw))),
    }
}

fn trim_optional(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn friendly_db_error(error: &sqlx::Error) -> String {
    if let sqlx::Error::Database(database) = error {
        if database.code().as_deref() == Some("23505") {
            return "A field with that key already exists for this object type.".to_owned();
        }
        if database.code().as_deref() == Some("23514") {
            return "The field settings are inconsistent. Check the rules and options.".to_owned();
        }
    }
    "The custom field could not be saved. Check the values and try again.".to_owned()
}

fn page(title: &str, session: &AdminSession, body: String) -> Response {
    Html(html::layout(
        title,
        "/admin/custom-fields",
        Some(&session.who()),
        &body,
    ))
    .into_response()
}

fn failure(message: &str) -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Html(html::error_page("Custom fields", message)),
    )
        .into_response()
}

fn not_found() -> Response {
    (
        StatusCode::NOT_FOUND,
        Html(html::error_page(
            "Custom field not found",
            "It may have been deleted, or it belongs to another organisation.",
        )),
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

// ---------------------------------------------------------------------------
// Form values and rendering
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Clone)]
struct FormValues {
    entity_type: String,
    key: String,
    label: String,
    help_text: String,
    data_type: String,
    is_required: bool,
    min_length: String,
    max_length: String,
    min_value: String,
    max_value: String,
    pattern: String,
    options: String,
    default_value: String,
    sort_order: String,
}

impl FormValues {
    fn from_row(definition: &DefinitionRow) -> Self {
        Self {
            entity_type: definition.entity_type.clone(),
            key: definition.key.clone(),
            label: definition.label.clone(),
            help_text: definition.help_text.clone().unwrap_or_default(),
            data_type: definition.data_type.clone(),
            is_required: definition.is_required,
            min_length: definition
                .min_length
                .map(|value| value.to_string())
                .unwrap_or_default(),
            max_length: definition
                .max_length
                .map(|value| value.to_string())
                .unwrap_or_default(),
            min_value: definition
                .min_value
                .map(|value| value.to_string())
                .unwrap_or_default(),
            max_value: definition
                .max_value
                .map(|value| value.to_string())
                .unwrap_or_default(),
            pattern: definition.pattern.clone().unwrap_or_default(),
            options: definition
                .options
                .as_ref()
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join("\n")
                })
                .unwrap_or_default(),
            default_value: definition
                .default_value
                .as_ref()
                .map(display_value)
                .unwrap_or_default(),
            sort_order: definition.sort_order.to_string(),
        }
    }

    fn from_form(form: &DefinitionForm) -> Self {
        Self {
            entity_type: form.entity_type.clone(),
            key: form.key.clone(),
            label: form.label.clone(),
            help_text: form.help_text.clone().unwrap_or_default(),
            data_type: form.data_type.clone(),
            is_required: form.is_required.is_some(),
            min_length: form.min_length.clone().unwrap_or_default(),
            max_length: form.max_length.clone().unwrap_or_default(),
            min_value: form.min_value.clone().unwrap_or_default(),
            max_value: form.max_value.clone().unwrap_or_default(),
            pattern: form.pattern.clone().unwrap_or_default(),
            options: form.options.clone().unwrap_or_default(),
            default_value: form.default_value.clone().unwrap_or_default(),
            sort_order: form.sort_order.clone().unwrap_or_default(),
        }
    }
}

fn display_value(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Array(items) => items
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join(", "),
        other => other.to_string(),
    }
}

struct FormPage<'a> {
    session: &'a AdminSession,
    csrf: &'a str,
    entities: &'a [EntityOption],
    values: FormValues,
    errors: Vec<String>,
    existing: Option<&'a DefinitionRow>,
}

fn render_form(page: FormPage<'_>) -> String {
    let editing = page.existing.is_some();
    let locked = page
        .existing
        .is_some_and(|definition| definition.value_count > 0);
    let value_count = page.existing.map_or(0, |definition| definition.value_count);
    let is_active = page.existing.is_none_or(|definition| definition.is_active);

    let mut body = String::new();
    body.push_str(&html::page_head(
        if editing {
            "Edit custom field"
        } else {
            "New custom field"
        },
        "Definitions are metadata; they never alter the database schema.",
    ));

    body.push_str(
        "<div class=\"notice\"><strong>Think twice before adding a field.</strong> Every custom field becomes part of your organisation's data contract: reports, exports, mobile forms and staff training must account for it. Add fields deliberately and name them clearly.</div>",
    );

    if locked {
        body.push_str(&format!(
            "<div class=\"warn-box\"><strong>{value_count} stored value(s).</strong> Object type and data type are locked to protect existing data. Create a new field and migrate if the shape must change.<br><br>Changing validation rules does not re-check stored values. Existing values that no longer pass will stay until they are next edited.</div>"
        ));
    }

    if !page.errors.is_empty() {
        body.push_str("<div class=\"alert\"><strong>Please fix the following:</strong><ul>");
        for error in &page.errors {
            body.push_str(&format!("<li>{}</li>", html::escape(error)));
        }
        body.push_str("</ul></div>");
    }

    let action = page.existing.map_or_else(
        || "/admin/custom-fields".to_owned(),
        |definition| format!("/admin/custom-fields/{}", definition.id),
    );

    body.push_str(&format!(
        "<form class=\"form wide\" method=\"post\" action=\"{action}\">"
    ));
    body.push_str(&format!(
        "<input type=\"hidden\" name=\"csrf\" value=\"{}\">",
        html::escape(page.csrf)
    ));

    // Object type
    body.push_str("<label for=\"entity_type\">Object</label>");
    if locked {
        body.push_str(&format!(
            "<input type=\"hidden\" name=\"entity_type\" value=\"{}\"><input id=\"entity_type\" value=\"{}\" disabled>",
            html::escape(&page.values.entity_type),
            html::escape(entity_label(page.entities, &page.values.entity_type)),
        ));
    } else {
        body.push_str("<select id=\"entity_type\" name=\"entity_type\">");
        for entity in page.entities {
            body.push_str(&format!(
                "<option value=\"{}\"{}>{}</option>",
                html::escape(&entity.entity_type),
                if entity.entity_type == page.values.entity_type {
                    " selected"
                } else {
                    ""
                },
                html::escape(&entity.label)
            ));
        }
        body.push_str("</select>");
    }

    body.push_str("<label for=\"key\">Key</label>");
    body.push_str(&format!(
        "<input id=\"key\" name=\"key\" value=\"{}\" placeholder=\"ndis_number\" required pattern=\"[a-z][a-z0-9_]*\"><p class=\"hint\">Stable identifier used by reports and the API. Lowercase letters, digits and underscore.</p>",
        html::escape(&page.values.key)
    ));

    body.push_str("<label for=\"label\">Label</label>");
    body.push_str(&format!(
        "<input id=\"label\" name=\"label\" value=\"{}\" placeholder=\"NDIS number\" required>",
        html::escape(&page.values.label)
    ));

    body.push_str(
        "<label for=\"help_text\">Help text <span class=\"muted\">(optional)</span></label>",
    );
    body.push_str(&format!(
        "<input id=\"help_text\" name=\"help_text\" value=\"{}\">",
        html::escape(&page.values.help_text)
    ));

    // Data type
    body.push_str("<label for=\"data_type\">Data type</label>");
    if locked {
        let type_label = match FieldType::parse(&page.values.data_type) {
            Some(kind) => kind.label(),
            None => page.values.data_type.as_str(),
        };
        body.push_str(&format!(
            "<input type=\"hidden\" name=\"data_type\" value=\"{}\"><input id=\"data_type\" value=\"{}\" disabled>",
            html::escape(&page.values.data_type),
            html::escape(type_label),
        ));
    } else {
        body.push_str("<select id=\"data_type\" name=\"data_type\">");
        for kind in FieldType::ALL {
            body.push_str(&format!(
                "<option value=\"{}\"{}>{}</option>",
                kind.as_str(),
                if kind.as_str() == page.values.data_type {
                    " selected"
                } else {
                    ""
                },
                kind.label()
            ));
        }
        body.push_str("</select>");
    }

    body.push_str(
        "<div class=\"checkbox\"><input id=\"is_required\" name=\"is_required\" type=\"checkbox\"",
    );
    if page.values.is_required {
        body.push_str(" checked");
    }
    body.push_str("><label for=\"is_required\">Required</label></div>");

    body.push_str("<h2>Validation</h2>");
    body.push_str("<div class=\"grid-2\">");
    body.push_str(&format!(
        "<div><label for=\"min_length\">Minimum length <span class=\"muted\">(text)</span></label><input id=\"min_length\" name=\"min_length\" type=\"number\" min=\"0\" value=\"{}\"></div>",
        html::escape(&page.values.min_length)
    ));
    body.push_str(&format!(
        "<div><label for=\"max_length\">Maximum length <span class=\"muted\">(text)</span></label><input id=\"max_length\" name=\"max_length\" type=\"number\" min=\"0\" value=\"{}\"></div>",
        html::escape(&page.values.max_length)
    ));
    body.push_str(&format!(
        "<div><label for=\"min_value\">Minimum value <span class=\"muted\">(numbers)</span></label><input id=\"min_value\" name=\"min_value\" type=\"number\" step=\"any\" value=\"{}\"></div>",
        html::escape(&page.values.min_value)
    ));
    body.push_str(&format!(
        "<div><label for=\"max_value\">Maximum value <span class=\"muted\">(numbers)</span></label><input id=\"max_value\" name=\"max_value\" type=\"number\" step=\"any\" value=\"{}\"></div>",
        html::escape(&page.values.max_value)
    ));
    body.push_str("</div>");

    body.push_str("<label for=\"pattern\">Regular expression <span class=\"muted\">(optional, text)</span></label>");
    body.push_str(&format!(
        "<input id=\"pattern\" name=\"pattern\" value=\"{}\" placeholder=\"^[0-9]{{9}}$\"><p class=\"hint\">Rust regular expression syntax. Values must match.</p>",
        html::escape(&page.values.pattern)
    ));

    body.push_str("<label for=\"options\">Options <span class=\"muted\">(one per line, choice fields only)</span></label>");
    body.push_str(&format!(
        "<textarea id=\"options\" name=\"options\" rows=\"5\">{}</textarea>",
        html::escape(&page.values.options)
    ));

    body.push_str("<label for=\"default_value\">Default value <span class=\"muted\">(optional)</span></label>");
    body.push_str(&format!(
        "<input id=\"default_value\" name=\"default_value\" value=\"{}\">",
        html::escape(&page.values.default_value)
    ));

    body.push_str(&format!(
        "<label for=\"sort_order\">Display order</label><input id=\"sort_order\" name=\"sort_order\" type=\"number\" value=\"{}\">",
        html::escape(&page.values.sort_order)
    ));

    body.push_str("<button type=\"submit\">Save field</button></form>");

    if let Some(definition) = page.existing {
        body.push_str("<h2>Archive</h2>");
        body.push_str("<p class=\"sub\">Archiving hides the field from new forms but keeps all stored values. This is reversible.</p>");
        body.push_str(&format!(
            "<form method=\"post\" action=\"/admin/custom-fields/{}/archive\">\
             <input type=\"hidden\" name=\"csrf\" value=\"{}\">\
             <input type=\"hidden\" name=\"confirm\" value=\"yes\">\
             <button type=\"submit\" class=\"secondary\">{}</button></form>",
            definition.id,
            html::escape(page.csrf),
            if is_active {
                "Archive field"
            } else {
                "Restore field"
            }
        ));

        body.push_str("<h2>Danger zone</h2>");
        body.push_str(&format!(
            "<div class=\"danger-box\"><p><strong>Deleting this field permanently removes {value_count} stored value(s).</strong> This cannot be undone, and any reports or integrations using the key <code>{}</code> will break.</p>\
             <p class=\"sub\">Archive instead unless you are certain. To confirm, tick the box and type the key exactly.</p>\
             <form method=\"post\" action=\"/admin/custom-fields/{}/delete\">\
             <input type=\"hidden\" name=\"csrf\" value=\"{}\">\
             <div class=\"checkbox\"><input id=\"confirm\" name=\"confirm\" type=\"checkbox\" value=\"yes\"><label for=\"confirm\">I understand the stored values will be lost.</label></div>\
             <label for=\"confirm_key\">Type <code>{}</code> to confirm</label>\
             <input id=\"confirm_key\" name=\"confirm_key\" autocomplete=\"off\">\
             <button type=\"submit\" class=\"danger\">Delete permanently</button>\
             </form></div>",
            html::escape(&definition.key),
            definition.id,
            html::escape(page.csrf),
            html::escape(&definition.key),
        ));
    }

    html::layout(
        if editing {
            "Edit custom field"
        } else {
            "New custom field"
        },
        "/admin/custom-fields",
        Some(&page.session.who()),
        &body,
    )
}

fn entity_label<'a>(entities: &'a [EntityOption], entity_type: &'a str) -> &'a str {
    entities
        .iter()
        .find(|entity| entity.entity_type == entity_type)
        .map_or(entity_type, |entity| entity.label.as_str())
}
