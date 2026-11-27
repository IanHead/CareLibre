//! Shared case-domain helpers: data validation, checklists, lifecycle,
//! assignment and onboarding.

use std::collections::HashMap;

use chrono::NaiveDate;
use serde_json::{json, Map, Value};
use sqlx::types::Json as SqlJson;
use sqlx::{FromRow, PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::{
    custom_fields::{validate_value, FieldRules, FieldType},
    error::AppError,
    relationship_types,
    worker_qualities::{normalise_gender, normalise_languages},
};

#[derive(Debug, Clone)]
pub struct CustomDefinition {
    pub id: Uuid,
    pub key: String,
    pub label: String,
    pub data_type: FieldType,
    pub rules: FieldRules,
}

#[derive(Debug, FromRow)]
struct CustomDefinitionRow {
    id: Uuid,
    key: String,
    label: String,
    data_type: String,
    is_required: bool,
    min_length: Option<i32>,
    max_length: Option<i32>,
    min_value: Option<f64>,
    max_value: Option<f64>,
    pattern: Option<String>,
    options: Option<SqlJson<Value>>,
}

#[derive(Debug, FromRow)]
struct CustomValueRow {
    key: String,
    value: SqlJson<Value>,
}

#[derive(Debug, FromRow)]
struct ChecklistProgressRow {
    total: i64,
    done: i64,
    required_total: i64,
    required_done: i64,
}

#[derive(Debug, FromRow)]
struct AssignmentRuleRow {
    user_id: Option<Uuid>,
    permission_set_id: Option<Uuid>,
    strategy: String,
    notify: bool,
}

#[derive(Debug, FromRow)]
struct IncompleteChecklistRow {
    id: Uuid,
    label: String,
}

#[derive(Debug, FromRow)]
struct CaseDataRow {
    data: SqlJson<Value>,
}

pub async fn custom_definitions(
    db: &PgPool,
    organisation_id: Uuid,
) -> Result<Vec<CustomDefinition>, AppError> {
    let rows = sqlx::query_as::<_, CustomDefinitionRow>(
        "SELECT id, key, label, data_type, is_required,
                min_length, max_length, min_value::float8 AS min_value,
                max_value::float8 AS max_value, pattern, options
         FROM custom_field_definitions
         WHERE organisation_id = $1 AND entity_type = 'contact' AND is_active
         ORDER BY sort_order, key",
    )
    .bind(organisation_id)
    .fetch_all(db)
    .await?;

    rows.into_iter()
        .map(|row| {
            let data_type = FieldType::parse(&row.data_type).ok_or_else(|| {
                AppError::validation(format!(
                    "custom field {} has an unsupported type",
                    row.label
                ))
            })?;
            let options = row.options.and_then(|options| {
                options.0.as_array().map(|items| {
                    items
                        .iter()
                        .filter_map(|item| item.as_str().map(str::to_owned))
                        .collect()
                })
            });
            Ok(CustomDefinition {
                id: row.id,
                key: row.key,
                label: row.label,
                data_type,
                rules: FieldRules {
                    required: row.is_required,
                    min_length: row.min_length,
                    max_length: row.max_length,
                    min_value: row.min_value,
                    max_value: row.max_value,
                    pattern: row.pattern,
                    options,
                },
            })
        })
        .collect()
}

/// Split a flat case-data map into the built-in JSON object and validated
/// custom values.
pub fn split_case_data(
    definitions: &[CustomDefinition],
    input: &Map<String, Value>,
) -> Result<(Value, Vec<(Uuid, Value)>), AppError> {
    let mut builtin = Map::new();
    let mut custom = Vec::new();

    for (key, value) in input {
        if let Some(custom_key) = key.strip_prefix("custom:") {
            let definition = definitions
                .iter()
                .find(|definition| definition.key == custom_key)
                .ok_or_else(|| {
                    AppError::validation(format!("unknown custom field: {custom_key}"))
                })?;
            validate_value(definition.data_type, &definition.rules, value).map_err(|message| {
                AppError::validation_with_details(
                    message,
                    json!({ "field": definition.label, "key": key }),
                )
            })?;
            if !value.is_null() {
                custom.push((definition.id, value.clone()));
            }
        } else if crate::intake::fields::builtin_field(key).is_some() {
            if !value.is_null() {
                builtin.insert(key.clone(), value.clone());
            }
        } else {
            return Err(AppError::validation(format!("unknown field key: {key}")));
        }
    }

    Ok((Value::Object(builtin), custom))
}

/// Merge built-in and custom storage into the flat shape clients use.
pub async fn merge_case_data(
    db: &PgPool,
    organisation_id: Uuid,
    case_id: Uuid,
    builtin: Value,
) -> Result<Value, AppError> {
    let mut object = builtin.as_object().cloned().unwrap_or_default();
    let values = sqlx::query_as::<_, CustomValueRow>(
        "SELECT d.key, v.value
         FROM custom_field_values v
         JOIN custom_field_definitions d
           ON d.organisation_id = v.organisation_id AND d.id = v.definition_id
         WHERE v.organisation_id = $1 AND v.entity_type = 'intake_case'
           AND v.entity_id = $2",
    )
    .bind(organisation_id)
    .bind(case_id)
    .fetch_all(db)
    .await?;
    for value in values {
        object.insert(format!("custom:{}", value.key), value.value.0);
    }
    Ok(Value::Object(object))
}

pub async fn write_case_data(
    tx: &mut Transaction<'_, Postgres>,
    organisation_id: Uuid,
    case_id: Uuid,
    builtin: &Value,
    custom: &[(Uuid, Value)],
) -> Result<(), AppError> {
    sqlx::query("UPDATE intake_cases SET data = $1 WHERE organisation_id = $2 AND id = $3")
        .bind(builtin)
        .bind(organisation_id)
        .bind(case_id)
        .execute(&mut **tx)
        .await?;

    sqlx::query(
        "DELETE FROM custom_field_values
         WHERE organisation_id = $1 AND entity_type = 'intake_case' AND entity_id = $2",
    )
    .bind(organisation_id)
    .bind(case_id)
    .execute(&mut **tx)
    .await?;

    for (definition_id, value) in custom {
        sqlx::query(
            "INSERT INTO custom_field_values
                (id, organisation_id, definition_id, entity_type, entity_id, value)
             VALUES ($1, $2, $3, 'intake_case', $4, $5)",
        )
        .bind(Uuid::now_v7())
        .bind(organisation_id)
        .bind(definition_id)
        .bind(case_id)
        .bind(value)
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

#[must_use]
pub fn value_present(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::String(value) => !value.trim().is_empty(),
        Value::Array(values) => !values.is_empty(),
        Value::Object(values) => !values.is_empty(),
        Value::Bool(_) | Value::Number(_) => true,
    }
}

pub async fn materialise_checklist(
    tx: &mut Transaction<'_, Postgres>,
    organisation_id: Uuid,
    case_id: Uuid,
    stage_id: Uuid,
) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO intake_case_checklist_items
            (id, organisation_id, case_id, stage_id, template_item_id, label,
             description, is_required, sort_order)
         SELECT gen_random_uuid(), t.organisation_id, $1, t.stage_id, t.id, t.label,
                t.description, t.is_required, t.sort_order
         FROM intake_stage_checklist_items t
         WHERE t.organisation_id = $2 AND t.stage_id = $3
           AND NOT EXISTS (
               SELECT 1 FROM intake_case_checklist_items ci
               WHERE ci.organisation_id = t.organisation_id
                 AND ci.case_id = $1 AND ci.template_item_id = t.id
           )
         ORDER BY t.sort_order, t.id",
    )
    .bind(case_id)
    .bind(organisation_id)
    .bind(stage_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

pub async fn checklist_progress(
    tx: &mut Transaction<'_, Postgres>,
    organisation_id: Uuid,
    case_id: Uuid,
    stage_id: Uuid,
) -> Result<(i64, i64, i64, i64), AppError> {
    let row = sqlx::query_as::<_, ChecklistProgressRow>(
        "SELECT count(*) AS total,
                count(*) FILTER (WHERE completed_at IS NOT NULL) AS done,
                count(*) FILTER (WHERE is_required) AS required_total,
                count(*) FILTER (WHERE is_required AND completed_at IS NOT NULL) AS required_done
         FROM intake_case_checklist_items
         WHERE organisation_id = $1 AND case_id = $2 AND stage_id = $3",
    )
    .bind(organisation_id)
    .bind(case_id)
    .bind(stage_id)
    .fetch_one(&mut **tx)
    .await?;
    Ok((row.done, row.total, row.required_done, row.required_total))
}

pub async fn incomplete_required_checklist(
    tx: &mut Transaction<'_, Postgres>,
    organisation_id: Uuid,
    case_id: Uuid,
    stage_id: Uuid,
) -> Result<Vec<(Uuid, String)>, AppError> {
    let rows = sqlx::query_as::<_, IncompleteChecklistRow>(
        "SELECT id, label FROM intake_case_checklist_items
         WHERE organisation_id = $1 AND case_id = $2 AND stage_id = $3
           AND is_required AND completed_at IS NULL
         ORDER BY sort_order, id",
    )
    .bind(organisation_id)
    .bind(case_id)
    .bind(stage_id)
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows.into_iter().map(|row| (row.id, row.label)).collect())
}

/// Resolve and persist the stage assignee, then notify the primary and any
/// watchers. Returns the primary user id when one resolved.
#[allow(clippy::too_many_arguments)]
pub async fn resolve_assignment(
    tx: &mut Transaction<'_, Postgres>,
    organisation_id: Uuid,
    case_id: Uuid,
    case_name: &str,
    stage_id: Uuid,
    stage_name: &str,
    actor_user_id: Uuid,
    event_id: i64,
) -> Result<Option<Uuid>, AppError> {
    let rules = sqlx::query_as::<_, AssignmentRuleRow>(
        "SELECT user_id, permission_set_id, strategy, notify
         FROM intake_stage_assignees
         WHERE organisation_id = $1 AND stage_id = $2
         ORDER BY sort_order, id",
    )
    .bind(organisation_id)
    .bind(stage_id)
    .fetch_all(&mut **tx)
    .await?;

    let mut primary = None;
    let mut primary_notify = true;
    let mut watchers = Vec::new();
    let mut resolved_first = false;

    for rule in &rules {
        let members = if let Some(user_id) = rule.user_id {
            if active_member(tx, organisation_id, user_id).await? {
                vec![user_id]
            } else {
                Vec::new()
            }
        } else if let Some(set_id) = rule.permission_set_id {
            pool_members(tx, organisation_id, set_id).await?
        } else {
            Vec::new()
        };

        if members.is_empty() {
            continue;
        }

        if !resolved_first {
            let chosen = match (rule.user_id, rule.strategy.as_str()) {
                (Some(user_id), _) => user_id,
                (None, "round_robin") => {
                    round_robin_member(tx, organisation_id, stage_id, &members).await?
                }
                (None, _) => first_available_member(tx, organisation_id, &members).await?,
            };
            primary = Some(chosen);
            primary_notify = rule.notify;
            resolved_first = true;
        } else if rule.notify {
            watchers.extend(members);
        }
    }

    let dedupe = format!("case:{case_id}:stage:{stage_id}:event:{event_id}");
    if let Some(user_id) = primary {
        if primary_notify && user_id != actor_user_id {
            notify(
                tx,
                organisation_id,
                user_id,
                case_id,
                &dedupe,
                case_name,
                stage_name,
            )
            .await?;
        }
    }
    for user_id in watchers {
        if user_id != actor_user_id && Some(user_id) != primary {
            notify(
                tx,
                organisation_id,
                user_id,
                case_id,
                &dedupe,
                case_name,
                stage_name,
            )
            .await?;
        }
    }

    Ok(primary)
}

pub async fn notify_user(
    tx: &mut Transaction<'_, Postgres>,
    organisation_id: Uuid,
    user_id: Uuid,
    case_id: Uuid,
    dedupe_key: &str,
    case_name: &str,
    stage_name: &str,
) -> Result<(), AppError> {
    notify(
        tx,
        organisation_id,
        user_id,
        case_id,
        dedupe_key,
        case_name,
        stage_name,
    )
    .await
}

async fn notify(
    tx: &mut Transaction<'_, Postgres>,
    organisation_id: Uuid,
    user_id: Uuid,
    case_id: Uuid,
    dedupe_key: &str,
    case_name: &str,
    stage_name: &str,
) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO notifications
            (organisation_id, user_id, kind, title, body, entity_type, entity_id, dedupe_key)
         VALUES ($1, $2, 'intake.assigned', $3, $4, 'intake_case', $5, $6)
         ON CONFLICT DO NOTHING",
    )
    .bind(organisation_id)
    .bind(user_id)
    .bind(format!("{case_name} moved to {stage_name}"))
    .bind("You are the assignee for this stage.")
    .bind(case_id)
    .bind(dedupe_key)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn active_member(
    tx: &mut Transaction<'_, Postgres>,
    organisation_id: Uuid,
    user_id: Uuid,
) -> Result<bool, AppError> {
    Ok(sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(
             SELECT 1 FROM organisation_users m
             JOIN users u ON u.id = m.user_id
             WHERE m.organisation_id = $1 AND m.user_id = $2 AND u.is_active
         )",
    )
    .bind(organisation_id)
    .bind(user_id)
    .fetch_one(&mut **tx)
    .await?)
}

async fn pool_members(
    tx: &mut Transaction<'_, Postgres>,
    organisation_id: Uuid,
    permission_set_id: Uuid,
) -> Result<Vec<Uuid>, AppError> {
    Ok(sqlx::query_scalar::<_, Uuid>(
        "SELECT u.id
         FROM organisation_user_permission_sets oup
         JOIN users u ON u.id = oup.user_id
         WHERE oup.organisation_id = $1 AND oup.permission_set_id = $2 AND u.is_active
         ORDER BY u.full_name, u.id",
    )
    .bind(organisation_id)
    .bind(permission_set_id)
    .fetch_all(&mut **tx)
    .await?)
}

async fn first_available_member(
    tx: &mut Transaction<'_, Postgres>,
    organisation_id: Uuid,
    members: &[Uuid],
) -> Result<Uuid, AppError> {
    let counts = sqlx::query_as::<_, (Uuid, i64)>(
        "SELECT u.id,
                count(c.id) FILTER (
                    WHERE c.closed_at IS NULL AND c.onboarded_at IS NULL
                ) AS open_cases
         FROM users u
         LEFT JOIN intake_cases c
           ON c.organisation_id = $1 AND c.assignee_user_id = u.id
         WHERE u.id = ANY($2)
         GROUP BY u.id
         ORDER BY open_cases, u.id",
    )
    .bind(organisation_id)
    .bind(members)
    .fetch_all(&mut **tx)
    .await?;
    counts
        .first()
        .map(|(id, _)| *id)
        .or_else(|| members.first().copied())
        .ok_or_else(|| AppError::validation("the assignment pool has no active members"))
}

async fn round_robin_member(
    tx: &mut Transaction<'_, Postgres>,
    organisation_id: Uuid,
    stage_id: Uuid,
    members: &[Uuid],
) -> Result<Uuid, AppError> {
    let last = sqlx::query_scalar::<_, Uuid>(
        "SELECT last_user_id FROM intake_assignment_cursors
         WHERE organisation_id = $1 AND stage_id = $2",
    )
    .bind(organisation_id)
    .bind(stage_id)
    .fetch_optional(&mut **tx)
    .await?;

    let next = last
        .and_then(|last| {
            members
                .iter()
                .position(|member| *member == last)
                .map(|position| members[(position + 1) % members.len()])
        })
        .or_else(|| members.first().copied())
        .ok_or_else(|| AppError::validation("the assignment pool has no active members"))?;

    sqlx::query(
        "INSERT INTO intake_assignment_cursors
            (organisation_id, stage_id, last_user_id, updated_at)
         VALUES ($1, $2, $3, now())
         ON CONFLICT (organisation_id, stage_id)
         DO UPDATE SET last_user_id = excluded.last_user_id, updated_at = now()",
    )
    .bind(organisation_id)
    .bind(stage_id)
    .bind(next)
    .execute(&mut **tx)
    .await?;
    Ok(next)
}

#[must_use]
pub fn allowed_lifecycle_transition(from: &str, to: &str) -> bool {
    match (from, to) {
        (from, to) if from == to => false,
        ("enquiry", "onboarding" | "active" | "intake_cancelled") => true,
        ("onboarding", "active" | "intake_cancelled") => true,
        ("active", "suspended" | "offboarding" | "offboarded") => true,
        ("suspended", "active" | "offboarding" | "offboarded") => true,
        ("offboarding", "offboarded" | "active") => true,
        ("intake_cancelled", "enquiry") => true,
        ("offboarded", "enquiry") => true,
        _ => false,
    }
}

/// Apply a stage-driven lifecycle nudge only when it is legal and does not
/// downgrade an active, suspended or offboarding contact.
pub async fn apply_lifecycle_nudge(
    tx: &mut Transaction<'_, Postgres>,
    organisation_id: Uuid,
    contact_id: Uuid,
    target: &str,
    actor_user_id: Uuid,
    stage_name: &str,
) -> Result<bool, AppError> {
    let current = sqlx::query_scalar::<_, String>(
        "SELECT lifecycle_status FROM contacts WHERE organisation_id = $1 AND id = $2",
    )
    .bind(organisation_id)
    .bind(contact_id)
    .fetch_optional(&mut **tx)
    .await?;
    let Some(current) = current else {
        return Ok(false);
    };
    if current == target {
        return Ok(false);
    }
    if matches!(current.as_str(), "suspended" | "offboarding" | "offboarded")
        || (target == "intake_cancelled" && current == "active")
    {
        return Ok(false);
    }
    if !allowed_lifecycle_transition(&current, target) {
        return Ok(false);
    }
    set_contact_status(
        tx,
        organisation_id,
        contact_id,
        target,
        Some(&format!("Intake stage: {stage_name}")),
        actor_user_id,
    )
    .await?;
    Ok(true)
}

pub async fn set_contact_status(
    tx: &mut Transaction<'_, Postgres>,
    organisation_id: Uuid,
    contact_id: Uuid,
    target: &str,
    reason: Option<&str>,
    actor_user_id: Uuid,
) -> Result<(), AppError> {
    let current = sqlx::query_scalar::<_, String>(
        "SELECT lifecycle_status FROM contacts WHERE organisation_id = $1 AND id = $2",
    )
    .bind(organisation_id)
    .bind(contact_id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or_else(|| AppError::not_found("contact not found"))?;

    if current == target {
        return Ok(());
    }

    sqlx::query(
        "UPDATE contacts SET lifecycle_status = $1
         WHERE organisation_id = $2 AND id = $3",
    )
    .bind(target)
    .bind(organisation_id)
    .bind(contact_id)
    .execute(&mut **tx)
    .await?;

    sqlx::query(
        "INSERT INTO contact_status_history
            (id, organisation_id, contact_id, from_status, to_status, reason, actor_user_id)
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(Uuid::now_v7())
    .bind(organisation_id)
    .bind(contact_id)
    .bind(&current)
    .bind(target)
    .bind(reason)
    .bind(actor_user_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Link the care/employment relationship when a case onboards. Other contacts
/// and organisation cases do not get an organisation relationship.
pub async fn link_role(
    tx: &mut Transaction<'_, Postgres>,
    organisation_id: Uuid,
    contact_id: Uuid,
    subject_kind: &str,
) -> Result<bool, AppError> {
    let (from_role, to_role) = match subject_kind {
        "client" => ("care_recipient", "provider"),
        "worker" => ("employee", "employer"),
        _ => return Ok(false),
    };

    let exists = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(
             SELECT 1 FROM relationships
             WHERE organisation_id = $1 AND from_contact_id = $2
               AND to_organisation_id = $1
               AND from_role = $3 AND to_role = $4
               AND end_date IS NULL AND removed_at IS NULL
         )",
    )
    .bind(organisation_id)
    .bind(contact_id)
    .bind(from_role)
    .bind(to_role)
    .fetch_one(&mut **tx)
    .await?;
    if exists {
        return Ok(false);
    }

    relationship_types::ensure_allowed(&mut **tx, organisation_id, from_role, to_role).await?;

    sqlx::query(
        "INSERT INTO relationships
            (id, organisation_id, from_contact_id, to_organisation_id, from_role, to_role)
         VALUES ($1, $2, $3, $2, $4, $5)",
    )
    .bind(Uuid::now_v7())
    .bind(organisation_id)
    .bind(contact_id)
    .bind(from_role)
    .bind(to_role)
    .execute(&mut **tx)
    .await?;
    Ok(true)
}

/// Copy case field values and custom values to the contact.
pub async fn copy_case_to_contact(
    tx: &mut Transaction<'_, Postgres>,
    organisation_id: Uuid,
    case_id: Uuid,
    contact_id: Uuid,
) -> Result<(), AppError> {
    let row = sqlx::query_as::<_, CaseDataRow>(
        "SELECT data FROM intake_cases WHERE organisation_id = $1 AND id = $2",
    )
    .bind(organisation_id)
    .bind(case_id)
    .fetch_one(&mut **tx)
    .await?;
    let data = row.data.0;

    let text = |key: &str| {
        data.get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    };
    let given_name = text("first_name").or_else(|| text("given_name"));
    let family_name = text("family_name").or_else(|| text("last_name"));
    let display_name = match (&given_name, &family_name) {
        (Some(given), Some(family)) => Some(format!("{given} {family}")),
        (Some(given), None) => Some(given.clone()),
        (None, Some(family)) => Some(family.clone()),
        (None, None) => None,
    };
    let date_of_birth =
        text("date_of_birth").and_then(|value| NaiveDate::parse_from_str(&value, "%Y-%m-%d").ok());
    let gender = text("gender").and_then(|value| normalise_gender(&value).map(str::to_owned));
    let languages = data.get("languages").map(|value| match value {
        Value::Array(values) => normalise_languages(
            values
                .iter()
                .filter_map(|item| item.as_str().map(str::to_owned))
                .collect(),
        ),
        Value::String(value) => normalise_languages(
            value
                .split(',')
                .map(|value| value.trim().to_owned())
                .collect(),
        ),
        _ => Vec::new(),
    });

    let mut details = Map::new();
    for (key, value) in data.as_object().into_iter().flatten() {
        if key.starts_with("custom:") {
            continue;
        }
        if matches!(
            key.as_str(),
            "first_name"
                | "given_name"
                | "last_name"
                | "family_name"
                | "preferred_name"
                | "pronouns"
                | "date_of_birth"
                | "gender"
                | "languages"
                | "cultural_identity"
                | "email"
                | "phone"
                | "notes"
                | "organisation_name"
        ) {
            continue;
        }
        details.insert(key.clone(), value.clone());
    }

    sqlx::query(
        "UPDATE contacts SET
            display_name = COALESCE($1, display_name),
            given_name = COALESCE($2, given_name),
            family_name = COALESCE($3, family_name),
            preferred_name = COALESCE($4, preferred_name),
            pronouns = COALESCE($5, pronouns),
            date_of_birth = COALESCE($6, date_of_birth),
            gender = COALESCE($7, gender),
            languages = CASE WHEN $8::text[] IS NULL THEN languages ELSE $8 END,
            cultural_identity = COALESCE($9, cultural_identity),
            email = COALESCE($10, email),
            phone = COALESCE($11, phone),
            notes = COALESCE($12, notes),
            details = COALESCE(details, '{}'::jsonb) || $13::jsonb
         WHERE organisation_id = $14 AND id = $15",
    )
    .bind(display_name)
    .bind(given_name)
    .bind(family_name)
    .bind(text("preferred_name"))
    .bind(text("pronouns"))
    .bind(date_of_birth)
    .bind(gender)
    .bind(languages)
    .bind(text("cultural_identity"))
    .bind(text("email"))
    .bind(text("phone"))
    .bind(text("notes"))
    .bind(Value::Object(details))
    .bind(organisation_id)
    .bind(contact_id)
    .execute(&mut **tx)
    .await?;

    let custom_values = sqlx::query_as::<_, (Uuid, SqlJson<Value>)>(
        "SELECT definition_id, value FROM custom_field_values
         WHERE organisation_id = $1 AND entity_type = 'intake_case' AND entity_id = $2",
    )
    .bind(organisation_id)
    .bind(case_id)
    .fetch_all(&mut **tx)
    .await?;
    for (definition_id, value) in custom_values {
        sqlx::query(
            "INSERT INTO custom_field_values
                (id, organisation_id, definition_id, entity_type, entity_id, value)
             VALUES ($1, $2, $3, 'contact', $4, $5)
             ON CONFLICT (organisation_id, definition_id, entity_id)
             DO UPDATE SET value = excluded.value, updated_at = now()",
        )
        .bind(Uuid::now_v7())
        .bind(organisation_id)
        .bind(definition_id)
        .bind(contact_id)
        .bind(value.0)
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

/// Build the initial case data from the contact's stored identity.
pub fn contact_identity_data(
    given_name: Option<&str>,
    family_name: Option<&str>,
    email: Option<&str>,
    phone: Option<&str>,
    date_of_birth: Option<NaiveDate>,
) -> Value {
    let mut data = Map::new();
    if let Some(given) = given_name.filter(|value| !value.trim().is_empty()) {
        data.insert(
            "first_name".to_owned(),
            Value::String(given.trim().to_owned()),
        );
    }
    if let Some(family) = family_name.filter(|value| !value.trim().is_empty()) {
        data.insert(
            "family_name".to_owned(),
            Value::String(family.trim().to_owned()),
        );
    }
    if let Some(email) = email.filter(|value| !value.trim().is_empty()) {
        data.insert("email".to_owned(), Value::String(email.trim().to_owned()));
    }
    if let Some(phone) = phone.filter(|value| !value.trim().is_empty()) {
        data.insert("phone".to_owned(), Value::String(phone.trim().to_owned()));
    }
    if let Some(date_of_birth) = date_of_birth {
        data.insert(
            "date_of_birth".to_owned(),
            Value::String(date_of_birth.to_string()),
        );
    }
    Value::Object(data)
}

/// Merge identity data with explicit case data; explicit values win.
pub fn merge_case_maps(base: Value, extra: &Map<String, Value>) -> Map<String, Value> {
    let mut merged = base.as_object().cloned().unwrap_or_default();
    for (key, value) in extra {
        merged.insert(key.clone(), value.clone());
    }
    merged
}

pub fn values_by_key(data: &Value) -> HashMap<String, Value> {
    data.as_object()
        .map(|object| {
            object
                .iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect()
        })
        .unwrap_or_default()
}
