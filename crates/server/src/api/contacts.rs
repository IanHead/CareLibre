//! Contacts API: list, detail, create and update, with custom-field metadata.
//!
//! Contacts are intentionally untyped on the server. The optional
//! `details.contact_role` hint lets native clients offer client/worker views
//! without changing the core model; relationships remain the source of truth
//! for care roles.

use std::collections::{BTreeMap, HashMap};

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::{types::Json as SqlJson, FromRow, Postgres, QueryBuilder};
use uuid::Uuid;

use crate::{
    auth::{extractor::AuthContext, role::Role},
    custom_fields::{validate_value, FieldRules, FieldType},
    error::AppError,
    state::AppState,
};

// ---------------------------------------------------------------------------
// DTOs
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct ContactDto {
    pub id: Uuid,
    pub display_name: String,
    pub given_name: Option<String>,
    pub family_name: Option<String>,
    pub preferred_name: Option<String>,
    pub pronouns: Option<String>,
    pub date_of_birth: Option<NaiveDate>,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub notes: Option<String>,
    pub is_active: bool,
    pub details: Value,
    pub role: String,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct ContactCountsDto {
    pub all: i64,
    pub clients: i64,
    pub workers: i64,
    pub dual: i64,
    pub other: i64,
}

#[derive(Debug, Serialize)]
pub struct ContactCursorDto {
    pub name: String,
    pub id: Uuid,
}

#[derive(Debug, Serialize)]
pub struct ContactListDto {
    pub items: Vec<ContactDto>,
    pub total: i64,
    pub counts: ContactCountsDto,
    pub next: Option<ContactCursorDto>,
}

#[derive(Debug, Serialize)]
pub struct CustomFieldDto {
    pub id: Uuid,
    pub key: String,
    pub label: String,
    pub help_text: Option<String>,
    pub data_type: String,
    pub is_required: bool,
    pub options: Option<Value>,
    pub sort_order: i32,
}

#[derive(Debug, Serialize)]
pub struct ContactDetailDto {
    pub contact: ContactDto,
    pub custom_fields: Vec<CustomFieldDto>,
    pub custom_values: BTreeMap<String, Value>,
}

#[derive(Debug, Deserialize)]
pub struct ListParams {
    pub search: Option<String>,
    pub role: Option<String>,
    pub limit: Option<u32>,
    pub after_name: Option<String>,
    pub after_id: Option<Uuid>,
}

#[derive(Debug, Deserialize)]
pub struct ContactWrite {
    pub display_name: String,
    pub given_name: Option<String>,
    pub family_name: Option<String>,
    pub preferred_name: Option<String>,
    pub pronouns: Option<String>,
    pub date_of_birth: Option<NaiveDate>,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub notes: Option<String>,
    pub details: Option<Value>,
    pub is_active: Option<bool>,
    pub custom_fields: Option<HashMap<String, Value>>,
}

// ---------------------------------------------------------------------------
// Rows
// ---------------------------------------------------------------------------

#[derive(Debug, FromRow)]
struct ContactRow {
    id: Uuid,
    display_name: String,
    given_name: Option<String>,
    family_name: Option<String>,
    preferred_name: Option<String>,
    pronouns: Option<String>,
    date_of_birth: Option<NaiveDate>,
    email: Option<String>,
    phone: Option<String>,
    notes: Option<String>,
    is_active: bool,
    details: SqlJson<Value>,
    updated_at: DateTime<Utc>,
}

#[derive(Debug, FromRow)]
struct CountsRow {
    all_count: i64,
    clients: i64,
    workers: i64,
    dual: i64,
    other: i64,
}

#[derive(Debug, FromRow)]
struct DefinitionRow {
    id: Uuid,
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
    options: Option<SqlJson<Value>>,
    sort_order: i32,
}

#[derive(Debug, FromRow)]
struct ValueRow {
    definition_id: Uuid,
    value: SqlJson<Value>,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

pub async fn list(
    State(state): State<AppState>,
    auth: AuthContext,
    Query(params): Query<ListParams>,
) -> Result<Json<ContactListDto>, AppError> {
    let limit = i64::from(params.limit.unwrap_or(200).clamp(1, 500));
    let search = params.search.unwrap_or_default().trim().to_owned();
    let role = params.role.unwrap_or_else(|| "all".to_owned());

    let counts = load_counts(&state, auth.organisation_id, &search).await?;

    let mut query = QueryBuilder::<Postgres>::new(CONTACT_SELECT);
    query.push(" WHERE organisation_id = ");
    query.push_bind(auth.organisation_id);
    push_search(&mut query, &search);
    push_role(&mut query, &role);

    if let (Some(name), Some(id)) = (params.after_name.as_deref(), params.after_id) {
        query.push(" AND (display_name > ");
        query.push_bind(name.to_owned());
        query.push(" OR (display_name = ");
        query.push_bind(name.to_owned());
        query.push(" AND id > ");
        query.push_bind(id);
        query.push("))");
    }

    query.push(" ORDER BY display_name, id LIMIT ");
    query.push_bind(limit);

    let rows = query
        .build_query_as::<ContactRow>()
        .fetch_all(&state.db)
        .await?;

    let next = if rows.len() as i64 == limit {
        rows.last().map(|row| ContactCursorDto {
            name: row.display_name.clone(),
            id: row.id,
        })
    } else {
        None
    };

    let total = match role.as_str() {
        "client" => counts.clients,
        "worker" => counts.workers,
        "dual" => counts.dual,
        "other" => counts.other,
        _ => counts.all_count,
    };

    Ok(Json(ContactListDto {
        items: rows.into_iter().map(contact_dto).collect(),
        total,
        counts: ContactCountsDto {
            all: counts.all_count,
            clients: counts.clients,
            workers: counts.workers,
            dual: counts.dual,
            other: counts.other,
        },
        next,
    }))
}

pub async fn get(
    State(state): State<AppState>,
    auth: AuthContext,
    Path(id): Path<Uuid>,
) -> Result<Json<ContactDetailDto>, AppError> {
    Ok(Json(load_detail(&state, auth.organisation_id, id).await?))
}

pub async fn create(
    State(state): State<AppState>,
    auth: AuthContext,
    Json(payload): Json<ContactWrite>,
) -> Result<(StatusCode, Json<ContactDetailDto>), AppError> {
    ensure_write(auth.role)?;

    let (display_name, details, active) = validate_write(&payload)?;
    let values =
        validate_custom_values(&state, auth.organisation_id, payload.custom_fields.as_ref())
            .await?;

    let id = Uuid::now_v7();
    let mut tx = state.db.begin().await?;

    sqlx::query(
        "INSERT INTO contacts
            (id, organisation_id, display_name, given_name, family_name, preferred_name,
             pronouns, date_of_birth, email, phone, notes, details, is_active)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13)",
    )
    .bind(id)
    .bind(auth.organisation_id)
    .bind(&display_name)
    .bind(trim_optional(payload.given_name.as_deref()))
    .bind(trim_optional(payload.family_name.as_deref()))
    .bind(trim_optional(payload.preferred_name.as_deref()))
    .bind(trim_optional(payload.pronouns.as_deref()))
    .bind(payload.date_of_birth)
    .bind(trim_optional(payload.email.as_deref()))
    .bind(trim_optional(payload.phone.as_deref()))
    .bind(trim_optional(payload.notes.as_deref()))
    .bind(SqlJson(details))
    .bind(active)
    .execute(&mut *tx)
    .await?;

    apply_custom_values(&mut tx, auth.organisation_id, id, values).await?;
    tx.commit().await?;

    let detail = load_detail(&state, auth.organisation_id, id).await?;
    Ok((StatusCode::CREATED, Json(detail)))
}

pub async fn update(
    State(state): State<AppState>,
    auth: AuthContext,
    Path(id): Path<Uuid>,
    Json(payload): Json<ContactWrite>,
) -> Result<Json<ContactDetailDto>, AppError> {
    ensure_write(auth.role)?;

    let (display_name, details, active) = validate_write(&payload)?;
    let values =
        validate_custom_values(&state, auth.organisation_id, payload.custom_fields.as_ref())
            .await?;

    let mut tx = state.db.begin().await?;

    let changed = sqlx::query(
        "UPDATE contacts SET
            display_name = $1, given_name = $2, family_name = $3, preferred_name = $4,
            pronouns = $5, date_of_birth = $6, email = $7, phone = $8, notes = $9,
            details = $10, is_active = $11
         WHERE id = $12 AND organisation_id = $13",
    )
    .bind(&display_name)
    .bind(trim_optional(payload.given_name.as_deref()))
    .bind(trim_optional(payload.family_name.as_deref()))
    .bind(trim_optional(payload.preferred_name.as_deref()))
    .bind(trim_optional(payload.pronouns.as_deref()))
    .bind(payload.date_of_birth)
    .bind(trim_optional(payload.email.as_deref()))
    .bind(trim_optional(payload.phone.as_deref()))
    .bind(trim_optional(payload.notes.as_deref()))
    .bind(SqlJson(details))
    .bind(active)
    .bind(id)
    .bind(auth.organisation_id)
    .execute(&mut *tx)
    .await?
    .rows_affected();

    if changed == 0 {
        return Err(AppError::not_found("contact not found"));
    }

    apply_custom_values(&mut tx, auth.organisation_id, id, values).await?;
    tx.commit().await?;

    Ok(Json(load_detail(&state, auth.organisation_id, id).await?))
}

pub async fn list_custom_fields(
    State(state): State<AppState>,
    auth: AuthContext,
) -> Result<Json<Vec<CustomFieldDto>>, AppError> {
    let definitions = load_definitions(&state, auth.organisation_id).await?;
    Ok(Json(definitions.into_iter().map(definition_dto).collect()))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

const CONTACT_SELECT: &str = "SELECT id, display_name, given_name, family_name, preferred_name,
    pronouns, date_of_birth, email, phone, notes, is_active, details, updated_at
    FROM contacts";

async fn load_detail(
    state: &AppState,
    organisation_id: Uuid,
    id: Uuid,
) -> Result<ContactDetailDto, AppError> {
    let row = sqlx::query_as::<_, ContactRow>(&format!(
        "{CONTACT_SELECT} WHERE id = $1 AND organisation_id = $2"
    ))
    .bind(id)
    .bind(organisation_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::not_found("contact not found"))?;

    let definitions = load_definitions(state, organisation_id).await?;
    let values = sqlx::query_as::<_, ValueRow>(
        "SELECT definition_id, value
         FROM custom_field_values
         WHERE organisation_id = $1 AND entity_type = 'contact' AND entity_id = $2",
    )
    .bind(organisation_id)
    .bind(id)
    .fetch_all(&state.db)
    .await?;

    let custom_values = values
        .into_iter()
        .map(|row| (row.definition_id.to_string(), row.value.0))
        .collect();

    Ok(ContactDetailDto {
        contact: contact_dto(row),
        custom_fields: definitions.into_iter().map(definition_dto).collect(),
        custom_values,
    })
}

async fn load_definitions(
    state: &AppState,
    organisation_id: Uuid,
) -> Result<Vec<DefinitionRow>, AppError> {
    Ok(sqlx::query_as::<_, DefinitionRow>(
        "SELECT id, key, label, help_text, data_type, is_required,
                min_length, max_length,
                min_value::float8 AS min_value, max_value::float8 AS max_value,
                pattern, options, sort_order
         FROM custom_field_definitions
         WHERE organisation_id = $1 AND entity_type = 'contact' AND is_active
         ORDER BY sort_order, label, id",
    )
    .bind(organisation_id)
    .fetch_all(&state.db)
    .await?)
}

async fn load_counts(
    state: &AppState,
    organisation_id: Uuid,
    search: &str,
) -> Result<CountsRow, AppError> {
    let mut query = QueryBuilder::<Postgres>::new(
        "SELECT COUNT(*) AS all_count,
                COUNT(*) FILTER (WHERE details->>'contact_role' IN ('client','dual')) AS clients,
                COUNT(*) FILTER (WHERE details->>'contact_role' IN ('worker','dual')) AS workers,
                COUNT(*) FILTER (WHERE details->>'contact_role' = 'dual') AS dual,
                COUNT(*) FILTER (WHERE details->>'contact_role' = 'other') AS other
         FROM contacts WHERE organisation_id = ",
    );
    query.push_bind(organisation_id);
    push_search(&mut query, search);
    Ok(query
        .build_query_as::<CountsRow>()
        .fetch_one(&state.db)
        .await?)
}

fn push_search(query: &mut QueryBuilder<'_, Postgres>, search: &str) {
    if search.is_empty() {
        return;
    }
    let pattern = format!("%{search}%");
    query.push(" AND (display_name ILIKE ");
    query.push_bind(pattern.clone());
    query.push(" OR email ILIKE ");
    query.push_bind(pattern.clone());
    query.push(" OR phone ILIKE ");
    query.push_bind(pattern);
    query.push(")");
}

fn push_role(query: &mut QueryBuilder<'_, Postgres>, role: &str) {
    match role {
        "client" => {
            query.push(" AND details->>'contact_role' IN ('client','dual')");
        }
        "worker" => {
            query.push(" AND details->>'contact_role' IN ('worker','dual')");
        }
        "dual" => {
            query.push(" AND details->>'contact_role' = 'dual'");
        }
        "other" => {
            query.push(" AND details->>'contact_role' = 'other'");
        }
        _ => {}
    }
}

fn contact_dto(row: ContactRow) -> ContactDto {
    let details = row.details.0;
    let role = details
        .get("contact_role")
        .and_then(Value::as_str)
        .unwrap_or("contact")
        .to_owned();

    ContactDto {
        id: row.id,
        display_name: row.display_name,
        given_name: row.given_name,
        family_name: row.family_name,
        preferred_name: row.preferred_name,
        pronouns: row.pronouns,
        date_of_birth: row.date_of_birth,
        email: row.email,
        phone: row.phone,
        notes: row.notes,
        is_active: row.is_active,
        details,
        role,
        updated_at: row.updated_at,
    }
}

fn definition_dto(row: DefinitionRow) -> CustomFieldDto {
    CustomFieldDto {
        id: row.id,
        key: row.key,
        label: row.label,
        help_text: row.help_text,
        data_type: row.data_type,
        is_required: row.is_required,
        options: row.options.map(|options| options.0),
        sort_order: row.sort_order,
    }
}

fn validate_write(payload: &ContactWrite) -> Result<(String, Value, bool), AppError> {
    let display_name = payload.display_name.trim().to_owned();
    let length = display_name.chars().count();
    if length == 0 || length > 160 {
        return Err(AppError::validation(
            "display_name must be between 1 and 160 characters",
        ));
    }

    let details = payload.details.clone().unwrap_or_else(|| json!({}));
    if !details.is_object() {
        return Err(AppError::validation("details must be a JSON object"));
    }

    Ok((display_name, details, payload.is_active.unwrap_or(true)))
}

async fn validate_custom_values(
    state: &AppState,
    organisation_id: Uuid,
    provided: Option<&HashMap<String, Value>>,
) -> Result<Vec<(Uuid, Value)>, AppError> {
    let provided = provided.cloned().unwrap_or_default();
    let definitions = load_definitions(state, organisation_id).await?;
    let mut known = HashMap::new();
    let mut values = Vec::new();

    for definition in &definitions {
        known.insert(definition.id.to_string(), ());
        let value = provided
            .get(&definition.id.to_string())
            .cloned()
            .unwrap_or(Value::Null);
        let kind = FieldType::parse(&definition.data_type).ok_or_else(|| {
            AppError::validation(format!(
                "custom field {} has an unsupported type",
                definition.label
            ))
        })?;
        let rules = rules_for(definition);
        validate_value(kind, &rules, &value).map_err(|message| {
            AppError::validation_with_details(
                message,
                json!({ "field": definition.label, "definition_id": definition.id }),
            )
        })?;
        if !value.is_null() {
            values.push((definition.id, value));
        }
    }

    for key in provided.keys() {
        if !known.contains_key(key) {
            return Err(AppError::validation(format!(
                "unknown custom field definition {key}"
            )));
        }
    }

    Ok(values)
}

fn rules_for(definition: &DefinitionRow) -> FieldRules {
    FieldRules {
        required: definition.is_required,
        min_length: definition.min_length,
        max_length: definition.max_length,
        min_value: definition.min_value,
        max_value: definition.max_value,
        pattern: definition.pattern.clone(),
        options: definition
            .options
            .as_ref()
            .and_then(|options| options.0.as_array())
            .map(|options| {
                options
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            }),
    }
}

async fn apply_custom_values(
    tx: &mut sqlx::Transaction<'_, Postgres>,
    organisation_id: Uuid,
    contact_id: Uuid,
    values: Vec<(Uuid, Value)>,
) -> Result<(), AppError> {
    sqlx::query(
        "DELETE FROM custom_field_values
         WHERE organisation_id = $1 AND entity_type = 'contact' AND entity_id = $2",
    )
    .bind(organisation_id)
    .bind(contact_id)
    .execute(&mut **tx)
    .await?;

    for (definition_id, value) in values {
        sqlx::query(
            "INSERT INTO custom_field_values
                (id, organisation_id, definition_id, entity_type, entity_id, value)
             VALUES ($1, $2, $3, 'contact', $4, $5)",
        )
        .bind(Uuid::now_v7())
        .bind(organisation_id)
        .bind(definition_id)
        .bind(contact_id)
        .bind(SqlJson(value))
        .execute(&mut **tx)
        .await?;
    }

    Ok(())
}

fn ensure_write(role: Role) -> Result<(), AppError> {
    if role == Role::Viewer {
        Err(AppError::forbidden("viewer role cannot edit contacts"))
    } else {
        Ok(())
    }
}

fn trim_optional(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}
