//! Intake configuration API.
//!
//! Phase 1 covers flows, stages, stage policy, checklists, assignment rules and
//! the field catalogue. Cases and the board build on these definitions.

use std::collections::{BTreeMap, HashMap, HashSet};

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{FromRow, PgPool, Postgres, Transaction};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::{
    admin::auth::write_audit,
    auth::extractor::AuthContext,
    error::{AppError, ErrorBody},
    intake::{
        self, builtin_fields, field_key_is_valid, is_action_key, is_subject_kind, FieldSpec,
        ACTION_KEYS,
    },
    permissions::Capability,
    state::AppState,
};

// ---------------------------------------------------------------------------
// DTOs
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, ToSchema)]
pub struct FieldDto {
    pub key: String,
    pub label: String,
    pub data_type: String,
    pub section: String,
    /// `builtin` or `custom`.
    pub source: String,
    pub options: Vec<String>,
    pub required_by_default: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct FieldCatalogueDto {
    pub fields: Vec<FieldDto>,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct UserRefDto {
    pub id: Uuid,
    pub full_name: String,
    pub email: String,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct PermissionSetRefDto {
    pub id: Uuid,
    pub name: String,
    pub persona_key: Option<String>,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct StageAssigneeDto {
    pub id: Uuid,
    pub user: Option<UserRefDto>,
    pub permission_set: Option<PermissionSetRefDto>,
    pub strategy: String,
    pub notify: bool,
    pub sort_order: i32,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct ChecklistTemplateDto {
    pub id: Uuid,
    pub label: String,
    pub description: Option<String>,
    pub is_required: bool,
    pub sort_order: i32,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct StageDto {
    pub id: Uuid,
    pub key: String,
    pub name: String,
    pub kind: String,
    pub color: Option<String>,
    pub sort_order: i32,
    pub is_terminal: bool,
    pub on_enter_contact_status: Option<String>,
    pub sla_hours: Option<i32>,
    pub checklist_gate: bool,
    pub visible_fields: Vec<String>,
    pub required_fields: Vec<String>,
    pub actions: BTreeMap<String, bool>,
    pub assignees: Vec<StageAssigneeDto>,
    pub checklist: Vec<ChecklistTemplateDto>,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct FlowDto {
    pub id: Uuid,
    pub key: String,
    pub name: String,
    pub subject_kind: String,
    pub variant: Option<String>,
    pub parent_flow_id: Option<Uuid>,
    pub description: Option<String>,
    pub division_id: Option<Uuid>,
    pub is_default: bool,
    pub is_active: bool,
    pub sort_order: i32,
    pub stages: Vec<StageDto>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct FlowListDto {
    pub items: Vec<FlowDto>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AssigneesDto {
    pub users: Vec<UserRefDto>,
    pub permission_sets: Vec<PermissionSetRefDto>,
}

#[derive(Debug, Default, Deserialize)]
pub struct FlowListParams {
    pub subject_kind: Option<String>,
    pub include_inactive: Option<bool>,
    pub division_id: Option<Uuid>,
    pub include_children: Option<bool>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct FlowCreate {
    pub key: String,
    pub name: String,
    pub subject_kind: String,
    pub variant: Option<String>,
    pub parent_flow_id: Option<Uuid>,
    pub description: Option<String>,
    pub division_id: Option<Uuid>,
    #[serde(default)]
    pub is_default: bool,
    /// Copy stages and policy from this flow. When omitted, the server clones
    /// the first active flow for the same subject kind.
    pub clone_from_flow_id: Option<Uuid>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct FlowUpdate {
    pub name: Option<String>,
    pub variant: Option<String>,
    pub parent_flow_id: Option<Uuid>,
    pub description: Option<String>,
    pub division_id: Option<Uuid>,
    pub is_default: Option<bool>,
    pub is_active: Option<bool>,
    pub sort_order: Option<i32>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct StageCreate {
    pub key: String,
    pub name: String,
    pub kind: String,
    pub color: Option<String>,
    pub on_enter_contact_status: Option<String>,
    pub sla_hours: Option<i32>,
    #[serde(default)]
    pub checklist_gate: bool,
    #[serde(default)]
    pub visible_fields: Vec<String>,
    #[serde(default)]
    pub required_fields: Vec<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct StageUpdate {
    pub name: Option<String>,
    pub kind: Option<String>,
    pub color: Option<String>,
    pub on_enter_contact_status: Option<String>,
    pub sla_hours: Option<i32>,
    pub checklist_gate: Option<bool>,
    pub visible_fields: Option<Vec<String>>,
    pub required_fields: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ActionsWrite {
    pub allowed: Vec<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ChecklistItemWrite {
    pub label: String,
    pub description: Option<String>,
    #[serde(default = "default_true")]
    pub is_required: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ChecklistWrite {
    pub items: Vec<ChecklistItemWrite>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct AssigneeWrite {
    pub user_id: Option<Uuid>,
    pub permission_set_id: Option<Uuid>,
    pub strategy: Option<String>,
    #[serde(default = "default_true")]
    pub notify: bool,
    pub sort_order: Option<i32>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct AssigneesWrite {
    pub assignees: Vec<AssigneeWrite>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ReorderWrite {
    pub stage_ids: Vec<Uuid>,
}

// ---------------------------------------------------------------------------
// Rows
// ---------------------------------------------------------------------------

#[derive(Debug, FromRow)]
struct FlowRow {
    id: Uuid,
    key: String,
    name: String,
    subject_kind: String,
    variant: Option<String>,
    parent_flow_id: Option<Uuid>,
    description: Option<String>,
    division_id: Option<Uuid>,
    is_default: bool,
    is_active: bool,
    sort_order: i32,
}

#[derive(Debug, FromRow)]
struct StageRow {
    id: Uuid,
    flow_id: Uuid,
    key: String,
    name: String,
    stage_kind: String,
    color: Option<String>,
    sort_order: i32,
    is_terminal: bool,
    on_enter_contact_status: Option<String>,
    sla_hours: Option<i32>,
    checklist_gate: bool,
    visible_fields: Value,
    required_fields: Value,
}

#[derive(Debug, FromRow)]
struct ActionRow {
    stage_id: Uuid,
    action_key: String,
    allowed: bool,
}

#[derive(Debug, FromRow)]
struct AssigneeRow {
    id: Uuid,
    stage_id: Uuid,
    strategy: String,
    notify: bool,
    sort_order: i32,
    user_id: Option<Uuid>,
    user_name: Option<String>,
    user_email: Option<String>,
    permission_set_id: Option<Uuid>,
    permission_set_name: Option<String>,
    persona_key: Option<String>,
}

#[derive(Debug, FromRow)]
struct ChecklistRow {
    id: Uuid,
    stage_id: Uuid,
    label: String,
    description: Option<String>,
    is_required: bool,
    sort_order: i32,
}

#[derive(Debug, FromRow)]
struct CustomFieldRow {
    key: String,
    label: String,
    data_type: String,
    options: Option<Value>,
    is_required: bool,
}

#[derive(Debug, FromRow)]
struct UserRow {
    id: Uuid,
    full_name: String,
    email: String,
}

#[derive(Debug, FromRow)]
struct PermissionSetRow {
    id: Uuid,
    name: String,
    persona_key: Option<String>,
}

// ---------------------------------------------------------------------------
// Field catalogue
// ---------------------------------------------------------------------------

#[utoipa::path(
    get,
    path = "/api/v1/intake/field-catalog",
    tag = "intake",
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Built-in and custom intake fields", body = FieldCatalogueDto),
        (status = 401, description = "Missing or invalid access token", body = ErrorBody),
        (status = 403, description = "Missing intake.read capability", body = ErrorBody)
    )
)]
pub async fn field_catalog(
    State(state): State<AppState>,
    auth: AuthContext,
) -> Result<Json<FieldCatalogueDto>, AppError> {
    auth.require(Capability::IntakeRead)?;
    let custom = sqlx::query_as::<_, CustomFieldRow>(
        "SELECT key, label, data_type, options, is_required
         FROM custom_field_definitions
         WHERE organisation_id = $1 AND entity_type = 'contact' AND is_active
         ORDER BY sort_order, label, key",
    )
    .bind(auth.organisation_id)
    .fetch_all(&state.db)
    .await?;

    let mut fields: Vec<FieldDto> = builtin_fields().iter().map(field_dto).collect();
    for definition in custom {
        fields.push(FieldDto {
            key: format!("custom:{}", definition.key),
            label: definition.label,
            data_type: definition.data_type,
            section: "Custom fields".to_owned(),
            source: "custom".to_owned(),
            options: string_array(definition.options.unwrap_or(Value::Array(Vec::new()))),
            required_by_default: definition.is_required,
        });
    }

    Ok(Json(FieldCatalogueDto { fields }))
}

fn field_dto(field: &FieldSpec) -> FieldDto {
    FieldDto {
        key: field.key.to_owned(),
        label: field.label.to_owned(),
        data_type: field.data_type.to_owned(),
        section: field.section.to_owned(),
        source: "builtin".to_owned(),
        options: field
            .options
            .iter()
            .map(|value| (*value).to_owned())
            .collect(),
        required_by_default: field.required_by_default,
    }
}

// ---------------------------------------------------------------------------
// Flow reads
// ---------------------------------------------------------------------------

#[utoipa::path(
    get,
    path = "/api/v1/intake/flows",
    tag = "intake",
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Intake flows with stages and policy", body = FlowListDto),
        (status = 401, description = "Missing or invalid access token", body = ErrorBody),
        (status = 403, description = "Missing intake.read capability", body = ErrorBody)
    )
)]
pub async fn list_flows(
    State(state): State<AppState>,
    auth: AuthContext,
    Query(params): Query<FlowListParams>,
) -> Result<Json<FlowListDto>, AppError> {
    auth.require(Capability::IntakeRead)?;
    let mut items = load_flows(&state.db, auth.organisation_id, true).await?;

    let include_inactive = params.include_inactive.unwrap_or(false);
    let subject_kinds: Option<HashSet<String>> = params.subject_kind.as_deref().map(|raw| {
        raw.split(',')
            .map(|value| value.trim().to_owned())
            .collect()
    });

    let division_filter = if let Some(division_id) = params.division_id {
        let include_children = params.include_children.unwrap_or(true);
        Some(
            descendant_divisions(
                &state.db,
                auth.organisation_id,
                division_id,
                include_children,
            )
            .await?,
        )
    } else {
        None
    };

    items.retain(|flow| {
        (include_inactive || flow.is_active)
            && subject_kinds
                .as_ref()
                .is_none_or(|kinds| kinds.contains(&flow.subject_kind))
            && division_filter
                .as_ref()
                .is_none_or(|divisions| flow.division_id.is_none_or(|id| divisions.contains(&id)))
    });

    Ok(Json(FlowListDto { items }))
}

pub(crate) async fn load_flows(
    db: &PgPool,
    organisation_id: Uuid,
    include_all: bool,
) -> Result<Vec<FlowDto>, AppError> {
    let flows = sqlx::query_as::<_, FlowRow>(
        "SELECT id, key, name, subject_kind, variant, parent_flow_id, description,
                division_id, is_default, is_active, sort_order
         FROM intake_flows
         WHERE organisation_id = $1 AND ($2 OR is_active)
         ORDER BY sort_order, name, id",
    )
    .bind(organisation_id)
    .bind(include_all)
    .fetch_all(db)
    .await?;

    let stages = sqlx::query_as::<_, StageRow>(
        "SELECT id, flow_id, key, name, stage_kind, color, sort_order, is_terminal,
                on_enter_contact_status, sla_hours, checklist_gate,
                visible_fields, required_fields
         FROM intake_stages
         WHERE organisation_id = $1
         ORDER BY sort_order, id",
    )
    .bind(organisation_id)
    .fetch_all(db)
    .await?;

    let actions = sqlx::query_as::<_, ActionRow>(
        "SELECT stage_id, action_key, allowed
         FROM intake_stage_actions
         WHERE organisation_id = $1",
    )
    .bind(organisation_id)
    .fetch_all(db)
    .await?;

    let assignees = sqlx::query_as::<_, AssigneeRow>(
        "SELECT a.id, a.stage_id, a.strategy, a.notify, a.sort_order,
                a.user_id, u.full_name AS user_name, u.email AS user_email,
                a.permission_set_id, ps.name AS permission_set_name, ps.persona_key
         FROM intake_stage_assignees a
         LEFT JOIN users u ON u.id = a.user_id
         LEFT JOIN permission_sets ps
           ON ps.organisation_id = a.organisation_id AND ps.id = a.permission_set_id
         WHERE a.organisation_id = $1
         ORDER BY a.sort_order, a.id",
    )
    .bind(organisation_id)
    .fetch_all(db)
    .await?;

    let checklist = sqlx::query_as::<_, ChecklistRow>(
        "SELECT id, stage_id, label, description, is_required, sort_order
         FROM intake_stage_checklist_items
         WHERE organisation_id = $1
         ORDER BY sort_order, id",
    )
    .bind(organisation_id)
    .fetch_all(db)
    .await?;

    let mut actions_by_stage: HashMap<Uuid, BTreeMap<String, bool>> = HashMap::new();
    for action in actions {
        actions_by_stage
            .entry(action.stage_id)
            .or_default()
            .insert(action.action_key, action.allowed);
    }

    let mut assignees_by_stage: HashMap<Uuid, Vec<StageAssigneeDto>> = HashMap::new();
    for row in assignees {
        let user = match (row.user_id, row.user_name, row.user_email) {
            (Some(id), Some(full_name), Some(email)) => Some(UserRefDto {
                id,
                full_name,
                email,
            }),
            _ => None,
        };
        let permission_set = match (row.permission_set_id, row.permission_set_name) {
            (Some(id), Some(name)) => Some(PermissionSetRefDto {
                id,
                name,
                persona_key: row.persona_key,
            }),
            _ => None,
        };
        assignees_by_stage
            .entry(row.stage_id)
            .or_default()
            .push(StageAssigneeDto {
                id: row.id,
                user,
                permission_set,
                strategy: row.strategy,
                notify: row.notify,
                sort_order: row.sort_order,
            });
    }

    let mut checklist_by_stage: HashMap<Uuid, Vec<ChecklistTemplateDto>> = HashMap::new();
    for row in checklist {
        checklist_by_stage
            .entry(row.stage_id)
            .or_default()
            .push(ChecklistTemplateDto {
                id: row.id,
                label: row.label,
                description: row.description,
                is_required: row.is_required,
                sort_order: row.sort_order,
            });
    }

    let mut stages_by_flow: HashMap<Uuid, Vec<StageDto>> = HashMap::new();
    for row in stages {
        let mut action_map = BTreeMap::new();
        for key in ACTION_KEYS {
            action_map.insert(key.to_owned(), false);
        }
        if let Some(allowed) = actions_by_stage.remove(&row.id) {
            action_map.extend(allowed);
        }

        stages_by_flow
            .entry(row.flow_id)
            .or_default()
            .push(StageDto {
                id: row.id,
                key: row.key,
                name: row.name,
                kind: row.stage_kind,
                color: row.color,
                sort_order: row.sort_order,
                is_terminal: row.is_terminal,
                on_enter_contact_status: row.on_enter_contact_status,
                sla_hours: row.sla_hours,
                checklist_gate: row.checklist_gate,
                visible_fields: string_array(row.visible_fields),
                required_fields: string_array(row.required_fields),
                actions: action_map,
                assignees: assignees_by_stage.remove(&row.id).unwrap_or_default(),
                checklist: checklist_by_stage.remove(&row.id).unwrap_or_default(),
            });
    }

    Ok(flows
        .into_iter()
        .map(|flow| {
            let mut stages = stages_by_flow.remove(&flow.id).unwrap_or_default();
            stages.sort_by_key(|stage| (stage.sort_order, stage.id));
            FlowDto {
                id: flow.id,
                key: flow.key,
                name: flow.name,
                subject_kind: flow.subject_kind,
                variant: flow.variant,
                parent_flow_id: flow.parent_flow_id,
                description: flow.description,
                division_id: flow.division_id,
                is_default: flow.is_default,
                is_active: flow.is_active,
                sort_order: flow.sort_order,
                stages,
            }
        })
        .collect())
}

async fn load_flow(db: &PgPool, organisation_id: Uuid, id: Uuid) -> Result<FlowDto, AppError> {
    load_flows(db, organisation_id, true)
        .await?
        .into_iter()
        .find(|flow| flow.id == id)
        .ok_or_else(|| AppError::not_found("intake flow not found"))
}

async fn descendant_divisions(
    db: &PgPool,
    organisation_id: Uuid,
    division_id: Uuid,
    include_children: bool,
) -> Result<HashSet<Uuid>, AppError> {
    let rows = if include_children {
        sqlx::query_scalar::<_, Uuid>(
            "WITH RECURSIVE tree AS (
                 SELECT id FROM divisions WHERE organisation_id = $1 AND id = $2
                 UNION ALL
                 SELECT d.id FROM divisions d JOIN tree t ON d.parent_id = t.id
                 WHERE d.organisation_id = $1
             )
             SELECT id FROM tree",
        )
        .bind(organisation_id)
        .bind(division_id)
        .fetch_all(db)
        .await?
    } else {
        sqlx::query_scalar::<_, Uuid>(
            "SELECT id FROM divisions WHERE organisation_id = $1 AND id = $2",
        )
        .bind(organisation_id)
        .bind(division_id)
        .fetch_all(db)
        .await?
    };

    if rows.is_empty() {
        return Err(AppError::not_found("division not found"));
    }
    Ok(rows.into_iter().collect())
}

// ---------------------------------------------------------------------------
// Flow writes
// ---------------------------------------------------------------------------

#[utoipa::path(
    post,
    path = "/api/v1/intake/flows",
    tag = "intake",
    security(("bearer_auth" = [])),
    request_body = FlowCreate,
    responses(
        (status = 201, description = "Flow created", body = FlowDto),
        (status = 401, description = "Missing or invalid access token", body = ErrorBody),
        (status = 403, description = "Missing intake.configure capability", body = ErrorBody),
        (status = 409, description = "Key already exists", body = ErrorBody),
        (status = 422, description = "Validation failed", body = ErrorBody)
    )
)]
pub async fn create_flow(
    State(state): State<AppState>,
    auth: AuthContext,
    Json(payload): Json<FlowCreate>,
) -> Result<(StatusCode, Json<FlowDto>), AppError> {
    auth.require(Capability::IntakeConfigure)?;
    let key = payload.key.trim().to_owned();
    validate_key(&key)?;
    let name = validate_name(&payload.name)?;
    if !is_subject_kind(&payload.subject_kind) {
        return Err(AppError::validation("unknown subject_kind"));
    }
    if let Some(parent) = payload.parent_flow_id {
        ensure_flow_exists(&state.db, auth.organisation_id, parent).await?;
    }
    if let Some(division) = payload.division_id {
        ensure_division_exists(&state.db, auth.organisation_id, division).await?;
    }

    let mut tx = state.db.begin().await?;
    let flow_id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO intake_flows
            (id, organisation_id, parent_flow_id, key, name, subject_kind, variant,
             description, division_id, is_default, is_active)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, true)",
    )
    .bind(flow_id)
    .bind(auth.organisation_id)
    .bind(payload.parent_flow_id)
    .bind(&key)
    .bind(&name)
    .bind(&payload.subject_kind)
    .bind(trim_optional(payload.variant.as_deref()))
    .bind(trim_optional(payload.description.as_deref()))
    .bind(payload.division_id)
    .bind(payload.is_default)
    .execute(&mut *tx)
    .await
    .map_err(map_conflict)?;

    if payload.is_default {
        clear_other_defaults(
            &mut tx,
            auth.organisation_id,
            &payload.subject_kind,
            payload.variant.as_deref(),
            flow_id,
        )
        .await?;
    }

    let source_flow = match payload.clone_from_flow_id {
        Some(id) => Some(id),
        None => {
            sqlx::query_scalar::<_, Uuid>(
                "SELECT id FROM intake_flows
                 WHERE organisation_id = $1 AND subject_kind = $2 AND is_active
                   AND id <> $3
                 ORDER BY sort_order, id LIMIT 1",
            )
            .bind(auth.organisation_id)
            .bind(&payload.subject_kind)
            .bind(flow_id)
            .fetch_optional(&mut *tx)
            .await?
        }
    };

    if let Some(source_flow) = source_flow {
        if source_flow != flow_id {
            clone_flow_stages(
                &mut tx,
                auth.organisation_id,
                source_flow,
                flow_id,
                &payload.subject_kind,
            )
            .await?;
        }
    }

    tx.commit().await?;

    write_audit(
        &state.db,
        auth.organisation_id,
        auth.user_id,
        "intake.flow.create",
        "intake_flow",
        Some(flow_id),
    )
    .await;

    let flow = load_flow(&state.db, auth.organisation_id, flow_id).await?;
    Ok((StatusCode::CREATED, Json(flow)))
}

#[utoipa::path(
    patch,
    path = "/api/v1/intake/flows/{id}",
    tag = "intake",
    security(("bearer_auth" = [])),
    request_body = FlowUpdate,
    params(("id" = Uuid, Path, description = "Intake flow id")),
    responses(
        (status = 200, description = "Flow updated", body = FlowDto),
        (status = 401, description = "Missing or invalid access token", body = ErrorBody),
        (status = 403, description = "Missing intake.configure capability", body = ErrorBody),
        (status = 404, description = "Flow not found", body = ErrorBody),
        (status = 422, description = "Validation failed", body = ErrorBody)
    )
)]
pub async fn update_flow(
    State(state): State<AppState>,
    auth: AuthContext,
    Path(id): Path<Uuid>,
    Json(payload): Json<FlowUpdate>,
) -> Result<Json<FlowDto>, AppError> {
    auth.require(Capability::IntakeConfigure)?;
    let existing = load_flow(&state.db, auth.organisation_id, id).await?;

    let mut tx = state.db.begin().await?;
    if let Some(name) = payload.name.as_deref() {
        let name = validate_name(name)?;
        sqlx::query("UPDATE intake_flows SET name = $1 WHERE organisation_id = $2 AND id = $3")
            .bind(name)
            .bind(auth.organisation_id)
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }
    if let Some(variant) = payload.variant.as_deref() {
        sqlx::query("UPDATE intake_flows SET variant = $1 WHERE organisation_id = $2 AND id = $3")
            .bind(trim_optional(Some(variant)))
            .bind(auth.organisation_id)
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }
    if let Some(parent) = payload.parent_flow_id {
        if parent == id {
            return Err(AppError::validation("a flow cannot be its own parent"));
        }
        ensure_flow_exists(&state.db, auth.organisation_id, parent).await?;
        sqlx::query(
            "UPDATE intake_flows SET parent_flow_id = $1 WHERE organisation_id = $2 AND id = $3",
        )
        .bind(parent)
        .bind(auth.organisation_id)
        .bind(id)
        .execute(&mut *tx)
        .await?;
    }
    if let Some(description) = payload.description.as_deref() {
        sqlx::query(
            "UPDATE intake_flows SET description = $1 WHERE organisation_id = $2 AND id = $3",
        )
        .bind(trim_optional(Some(description)))
        .bind(auth.organisation_id)
        .bind(id)
        .execute(&mut *tx)
        .await?;
    }
    if let Some(division) = payload.division_id {
        ensure_division_exists(&state.db, auth.organisation_id, division).await?;
        sqlx::query(
            "UPDATE intake_flows SET division_id = $1 WHERE organisation_id = $2 AND id = $3",
        )
        .bind(division)
        .bind(auth.organisation_id)
        .bind(id)
        .execute(&mut *tx)
        .await?;
    }
    if let Some(is_default) = payload.is_default {
        sqlx::query(
            "UPDATE intake_flows SET is_default = $1 WHERE organisation_id = $2 AND id = $3",
        )
        .bind(is_default)
        .bind(auth.organisation_id)
        .bind(id)
        .execute(&mut *tx)
        .await?;
        if is_default {
            clear_other_defaults(
                &mut tx,
                auth.organisation_id,
                &existing.subject_kind,
                existing.variant.as_deref(),
                id,
            )
            .await?;
        }
    }
    if let Some(is_active) = payload.is_active {
        sqlx::query(
            "UPDATE intake_flows SET is_active = $1 WHERE organisation_id = $2 AND id = $3",
        )
        .bind(is_active)
        .bind(auth.organisation_id)
        .bind(id)
        .execute(&mut *tx)
        .await?;
    }
    if let Some(sort_order) = payload.sort_order {
        sqlx::query(
            "UPDATE intake_flows SET sort_order = $1 WHERE organisation_id = $2 AND id = $3",
        )
        .bind(sort_order)
        .bind(auth.organisation_id)
        .bind(id)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;

    write_audit(
        &state.db,
        auth.organisation_id,
        auth.user_id,
        "intake.flow.update",
        "intake_flow",
        Some(id),
    )
    .await;

    Ok(Json(load_flow(&state.db, auth.organisation_id, id).await?))
}

#[utoipa::path(
    delete,
    path = "/api/v1/intake/flows/{id}",
    tag = "intake",
    security(("bearer_auth" = [])),
    params(("id" = Uuid, Path, description = "Intake flow id")),
    responses(
        (status = 204, description = "Flow archived"),
        (status = 401, description = "Missing or invalid access token", body = ErrorBody),
        (status = 403, description = "Missing intake.configure capability", body = ErrorBody),
        (status = 404, description = "Flow not found", body = ErrorBody)
    )
)]
pub async fn archive_flow(
    State(state): State<AppState>,
    auth: AuthContext,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    auth.require(Capability::IntakeConfigure)?;
    load_flow(&state.db, auth.organisation_id, id).await?;
    let changed = sqlx::query(
        "UPDATE intake_flows SET is_active = false WHERE organisation_id = $1 AND id = $2",
    )
    .bind(auth.organisation_id)
    .bind(id)
    .execute(&state.db)
    .await?
    .rows_affected();
    if changed == 0 {
        return Err(AppError::not_found("intake flow not found"));
    }

    write_audit(
        &state.db,
        auth.organisation_id,
        auth.user_id,
        "intake.flow.archive",
        "intake_flow",
        Some(id),
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

async fn clear_other_defaults(
    tx: &mut Transaction<'_, Postgres>,
    organisation_id: Uuid,
    subject_kind: &str,
    variant: Option<&str>,
    keep: Uuid,
) -> Result<(), AppError> {
    sqlx::query(
        "UPDATE intake_flows SET is_default = false
         WHERE organisation_id = $1 AND subject_kind = $2 AND id <> $3
           AND COALESCE(variant, '') = COALESCE($4, '')",
    )
    .bind(organisation_id)
    .bind(subject_kind)
    .bind(keep)
    .bind(variant)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

pub(crate) async fn clone_flow_stages(
    tx: &mut Transaction<'_, Postgres>,
    organisation_id: Uuid,
    source_flow: Uuid,
    dest_flow: Uuid,
    subject_kind: &str,
) -> Result<(), AppError> {
    let stages = sqlx::query_as::<_, StageRow>(
        "SELECT id, flow_id, key, name, stage_kind, color, sort_order, is_terminal,
                on_enter_contact_status, sla_hours, checklist_gate,
                visible_fields, required_fields
         FROM intake_stages
         WHERE organisation_id = $1 AND flow_id = $2
         ORDER BY sort_order, id",
    )
    .bind(organisation_id)
    .bind(source_flow)
    .fetch_all(&mut **tx)
    .await?;

    for stage in stages {
        let new_stage_id = Uuid::now_v7();
        let visible = adapt_fields(&stage.visible_fields, subject_kind, false);
        let required = adapt_fields(&stage.required_fields, subject_kind, true);
        sqlx::query(
            "INSERT INTO intake_stages
                (id, organisation_id, flow_id, key, name, stage_kind, color, sort_order,
                 is_terminal, on_enter_contact_status, sla_hours, checklist_gate,
                 visible_fields, required_fields)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)",
        )
        .bind(new_stage_id)
        .bind(organisation_id)
        .bind(dest_flow)
        .bind(&stage.key)
        .bind(&stage.name)
        .bind(&stage.stage_kind)
        .bind(&stage.color)
        .bind(stage.sort_order)
        .bind(stage.is_terminal)
        .bind(&stage.on_enter_contact_status)
        .bind(stage.sla_hours)
        .bind(stage.checklist_gate)
        .bind(visible)
        .bind(required)
        .execute(&mut **tx)
        .await?;

        sqlx::query(
            "INSERT INTO intake_stage_actions
                (organisation_id, stage_id, action_key, allowed)
             SELECT organisation_id, $1, action_key, allowed
             FROM intake_stage_actions
             WHERE organisation_id = $2 AND stage_id = $3",
        )
        .bind(new_stage_id)
        .bind(organisation_id)
        .bind(stage.id)
        .execute(&mut **tx)
        .await?;

        sqlx::query(
            "INSERT INTO intake_stage_checklist_items
                (id, organisation_id, stage_id, label, description, is_required, sort_order)
             SELECT gen_random_uuid(), organisation_id, $1, label, description,
                    is_required, sort_order
             FROM intake_stage_checklist_items
             WHERE organisation_id = $2 AND stage_id = $3",
        )
        .bind(new_stage_id)
        .bind(organisation_id)
        .bind(stage.id)
        .execute(&mut **tx)
        .await?;

        sqlx::query(
            "INSERT INTO intake_stage_assignees
                (id, organisation_id, stage_id, user_id, permission_set_id,
                 strategy, notify, sort_order)
             SELECT gen_random_uuid(), organisation_id, $1, user_id, permission_set_id,
                    strategy, notify, sort_order
             FROM intake_stage_assignees
             WHERE organisation_id = $2 AND stage_id = $3",
        )
        .bind(new_stage_id)
        .bind(organisation_id)
        .bind(stage.id)
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

/// Organisation flows do not have first/last names. Swap the field defaults
/// when cloning a person flow into an organisation flow.
fn adapt_fields(fields: &Value, subject_kind: &str, required: bool) -> Value {
    if subject_kind != "organisation" {
        return fields.clone();
    }
    let original = string_array(fields.clone());
    let had_name = original
        .iter()
        .any(|key| matches!(key.as_str(), "first_name" | "family_name" | "last_name"));
    let mut adapted: Vec<String> = original
        .into_iter()
        .filter(|key| !matches!(key.as_str(), "first_name" | "family_name" | "last_name"))
        .collect();
    if ((required && had_name) || !required)
        && !adapted.iter().any(|key| key == "organisation_name")
    {
        adapted.push("organisation_name".to_owned());
    }
    Value::Array(adapted.into_iter().map(Value::String).collect())
}

// ---------------------------------------------------------------------------
// Stage writes
// ---------------------------------------------------------------------------

#[utoipa::path(
    post,
    path = "/api/v1/intake/flows/{id}/stages",
    tag = "intake",
    security(("bearer_auth" = [])),
    request_body = StageCreate,
    params(("id" = Uuid, Path, description = "Intake flow id")),
    responses(
        (status = 201, description = "Stage created", body = FlowDto),
        (status = 401, description = "Missing or invalid access token", body = ErrorBody),
        (status = 403, description = "Missing intake.configure capability", body = ErrorBody),
        (status = 404, description = "Flow not found", body = ErrorBody),
        (status = 422, description = "Validation failed", body = ErrorBody)
    )
)]
pub async fn create_stage(
    State(state): State<AppState>,
    auth: AuthContext,
    Path(flow_id): Path<Uuid>,
    Json(payload): Json<StageCreate>,
) -> Result<(StatusCode, Json<FlowDto>), AppError> {
    auth.require(Capability::IntakeConfigure)?;
    let flow = load_flow(&state.db, auth.organisation_id, flow_id).await?;
    validate_key(&payload.key)?;
    let name = validate_name(&payload.name)?;
    if !intake::is_stage_kind(&payload.kind) {
        return Err(AppError::validation("unknown stage kind"));
    }
    validate_lifecycle_nudge(payload.on_enter_contact_status.as_deref())?;
    validate_sla(payload.sla_hours)?;
    let custom_keys = custom_field_keys(&state.db, auth.organisation_id).await?;
    validate_fields(
        &payload.visible_fields,
        &payload.required_fields,
        &custom_keys,
    )?;
    validate_stage_sequence_candidate(&flow, &payload.key, &payload.kind)?;

    let mut tx = state.db.begin().await?;
    let stage_id = Uuid::now_v7();
    let max_order = flow
        .stages
        .iter()
        .map(|stage| stage.sort_order)
        .max()
        .unwrap_or(0)
        .saturating_add(10);
    // Non-terminal stages belong before the terminal columns. Make room by
    // shifting the terminal stages when inserting a custom stage.
    let (sort_order, shift_from) = if intake::is_terminal_kind(&payload.kind) {
        (max_order, None)
    } else if let Some(first_terminal) = flow.stages.iter().find(|stage| stage.is_terminal) {
        (first_terminal.sort_order, Some(first_terminal.sort_order))
    } else {
        (max_order, None)
    };
    if let Some(threshold) = shift_from {
        sqlx::query(
            "UPDATE intake_stages SET sort_order = sort_order + 10
             WHERE organisation_id = $1 AND flow_id = $2 AND sort_order >= $3",
        )
        .bind(auth.organisation_id)
        .bind(flow_id)
        .bind(threshold)
        .execute(&mut *tx)
        .await?;
    }
    sqlx::query(
        "INSERT INTO intake_stages
            (id, organisation_id, flow_id, key, name, stage_kind, color, sort_order,
             is_terminal, on_enter_contact_status, sla_hours, checklist_gate,
             visible_fields, required_fields)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)",
    )
    .bind(stage_id)
    .bind(auth.organisation_id)
    .bind(flow_id)
    .bind(&payload.key)
    .bind(&name)
    .bind(&payload.kind)
    .bind(payload.color.as_deref())
    .bind(sort_order)
    .bind(intake::is_terminal_kind(&payload.kind))
    .bind(payload.on_enter_contact_status.as_deref())
    .bind(payload.sla_hours)
    .bind(payload.checklist_gate)
    .bind(Value::Array(
        payload
            .visible_fields
            .iter()
            .cloned()
            .map(Value::String)
            .collect(),
    ))
    .bind(Value::Array(
        payload
            .required_fields
            .iter()
            .cloned()
            .map(Value::String)
            .collect(),
    ))
    .execute(&mut *tx)
    .await
    .map_err(map_conflict)?;

    for action in ACTION_KEYS {
        sqlx::query(
            "INSERT INTO intake_stage_actions
                (organisation_id, stage_id, action_key, allowed)
             VALUES ($1, $2, $3, false)",
        )
        .bind(auth.organisation_id)
        .bind(stage_id)
        .bind(action)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;

    write_audit(
        &state.db,
        auth.organisation_id,
        auth.user_id,
        "intake.stage.create",
        "intake_stage",
        Some(stage_id),
    )
    .await;

    Ok((
        StatusCode::CREATED,
        Json(load_flow(&state.db, auth.organisation_id, flow_id).await?),
    ))
}

#[utoipa::path(
    patch,
    path = "/api/v1/intake/stages/{id}",
    tag = "intake",
    security(("bearer_auth" = [])),
    request_body = StageUpdate,
    params(("id" = Uuid, Path, description = "Intake stage id")),
    responses(
        (status = 200, description = "Stage updated", body = FlowDto),
        (status = 401, description = "Missing or invalid access token", body = ErrorBody),
        (status = 403, description = "Missing intake.configure capability", body = ErrorBody),
        (status = 404, description = "Stage not found", body = ErrorBody),
        (status = 409, description = "Stage kind is in use", body = ErrorBody),
        (status = 422, description = "Validation failed", body = ErrorBody)
    )
)]
pub async fn update_stage(
    State(state): State<AppState>,
    auth: AuthContext,
    Path(id): Path<Uuid>,
    Json(payload): Json<StageUpdate>,
) -> Result<Json<FlowDto>, AppError> {
    auth.require(Capability::IntakeConfigure)?;
    let (stage, flow_id) = load_stage(&state.db, auth.organisation_id, id).await?;
    let flow = load_flow(&state.db, auth.organisation_id, flow_id).await?;
    let custom_keys = custom_field_keys(&state.db, auth.organisation_id).await?;

    let kind = payload.kind.as_deref().unwrap_or(&stage.stage_kind);
    if !intake::is_stage_kind(kind) {
        return Err(AppError::validation("unknown stage kind"));
    }
    if kind != stage.stage_kind && stage_has_cases(&state.db, auth.organisation_id, id).await? {
        return Err(AppError::conflict(
            "a stage kind cannot change once cases have entered it",
        ));
    }
    let name = match payload.name.as_deref() {
        Some(name) => validate_name(name)?,
        None => stage.name.clone(),
    };
    let visible = payload
        .visible_fields
        .clone()
        .unwrap_or_else(|| string_array(stage.visible_fields.clone()));
    let required = payload
        .required_fields
        .clone()
        .unwrap_or_else(|| string_array(stage.required_fields.clone()));
    validate_fields(&visible, &required, &custom_keys)?;
    validate_lifecycle_nudge(
        payload
            .on_enter_contact_status
            .as_deref()
            .or(stage.on_enter_contact_status.as_deref()),
    )?;
    validate_sla(payload.sla_hours.or(stage.sla_hours))?;

    let color = payload.color.clone().or(stage.color.clone());
    let lifecycle_nudge = payload
        .on_enter_contact_status
        .clone()
        .or(stage.on_enter_contact_status.clone());
    let sla_hours = payload.sla_hours.or(stage.sla_hours);
    let checklist_gate = payload.checklist_gate.unwrap_or(stage.checklist_gate);
    let is_terminal = intake::is_terminal_kind(kind);

    let mut candidate = flow.clone();
    if let Some(target) = candidate.stages.iter_mut().find(|item| item.id == id) {
        target.name = name.clone();
        target.kind = kind.to_owned();
        target.is_terminal = is_terminal;
        target.color = color.clone();
        target.on_enter_contact_status = lifecycle_nudge.clone();
        target.sla_hours = sla_hours;
        target.checklist_gate = checklist_gate;
        target.visible_fields = visible.clone();
        target.required_fields = required.clone();
    }
    validate_stage_sequence(&candidate.stages)?;

    sqlx::query(
        "UPDATE intake_stages
         SET name = $1, stage_kind = $2, color = $3, is_terminal = $4,
             on_enter_contact_status = $5, sla_hours = $6, checklist_gate = $7,
             visible_fields = $8, required_fields = $9
         WHERE organisation_id = $10 AND id = $11",
    )
    .bind(&name)
    .bind(kind)
    .bind(color.as_deref())
    .bind(is_terminal)
    .bind(lifecycle_nudge.as_deref())
    .bind(sla_hours)
    .bind(checklist_gate)
    .bind(Value::Array(
        visible.into_iter().map(Value::String).collect(),
    ))
    .bind(Value::Array(
        required.into_iter().map(Value::String).collect(),
    ))
    .bind(auth.organisation_id)
    .bind(id)
    .execute(&state.db)
    .await?;

    write_audit(
        &state.db,
        auth.organisation_id,
        auth.user_id,
        "intake.stage.update",
        "intake_stage",
        Some(id),
    )
    .await;

    Ok(Json(
        load_flow(&state.db, auth.organisation_id, flow_id).await?,
    ))
}

#[utoipa::path(
    delete,
    path = "/api/v1/intake/stages/{id}",
    tag = "intake",
    security(("bearer_auth" = [])),
    params(("id" = Uuid, Path, description = "Intake stage id")),
    responses(
        (status = 204, description = "Stage deleted"),
        (status = 401, description = "Missing or invalid access token", body = ErrorBody),
        (status = 403, description = "Missing intake.configure capability", body = ErrorBody),
        (status = 404, description = "Stage not found", body = ErrorBody),
        (status = 409, description = "Stage is in use or required", body = ErrorBody)
    )
)]
pub async fn delete_stage(
    State(state): State<AppState>,
    auth: AuthContext,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    auth.require(Capability::IntakeConfigure)?;
    let (stage, flow_id) = load_stage(&state.db, auth.organisation_id, id).await?;
    let flow = load_flow(&state.db, auth.organisation_id, flow_id).await?;
    if stage.is_terminal || flow.stages.first().is_some_and(|first| first.id == id) {
        return Err(AppError::conflict(
            "the first or a terminal stage cannot be deleted",
        ));
    }
    if stage_has_cases(&state.db, auth.organisation_id, id).await? {
        return Err(AppError::conflict("stage has cases and cannot be deleted"));
    }
    let remaining: Vec<_> = flow
        .stages
        .iter()
        .filter(|item| item.id != id)
        .cloned()
        .collect();
    validate_stage_sequence(&remaining)?;

    sqlx::query("DELETE FROM intake_stages WHERE organisation_id = $1 AND id = $2")
        .bind(auth.organisation_id)
        .bind(id)
        .execute(&state.db)
        .await?;

    write_audit(
        &state.db,
        auth.organisation_id,
        auth.user_id,
        "intake.stage.delete",
        "intake_stage",
        Some(id),
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    post,
    path = "/api/v1/intake/flows/{id}/stages/reorder",
    tag = "intake",
    security(("bearer_auth" = [])),
    request_body = ReorderWrite,
    params(("id" = Uuid, Path, description = "Intake flow id")),
    responses(
        (status = 200, description = "Stages reordered", body = FlowDto),
        (status = 401, description = "Missing or invalid access token", body = ErrorBody),
        (status = 403, description = "Missing intake.configure capability", body = ErrorBody),
        (status = 404, description = "Flow not found", body = ErrorBody),
        (status = 422, description = "Validation failed", body = ErrorBody)
    )
)]
pub async fn reorder_stages(
    State(state): State<AppState>,
    auth: AuthContext,
    Path(flow_id): Path<Uuid>,
    Json(payload): Json<ReorderWrite>,
) -> Result<Json<FlowDto>, AppError> {
    auth.require(Capability::IntakeConfigure)?;
    let flow = load_flow(&state.db, auth.organisation_id, flow_id).await?;
    if payload.stage_ids.len() != flow.stages.len() {
        return Err(AppError::validation(
            "stage_ids must contain every stage in the flow exactly once",
        ));
    }
    let expected: HashSet<Uuid> = flow.stages.iter().map(|stage| stage.id).collect();
    let actual: HashSet<Uuid> = payload.stage_ids.iter().copied().collect();
    if expected != actual || actual.len() != payload.stage_ids.len() {
        return Err(AppError::validation(
            "stage_ids must contain every stage in the flow exactly once",
        ));
    }

    let mut reordered = flow.clone();
    reordered.stages.sort_by_key(|stage| {
        payload
            .stage_ids
            .iter()
            .position(|id| *id == stage.id)
            .unwrap_or(usize::MAX)
    });
    validate_stage_sequence(&reordered.stages)?;

    let mut tx = state.db.begin().await?;
    for (index, stage_id) in payload.stage_ids.iter().enumerate() {
        sqlx::query(
            "UPDATE intake_stages SET sort_order = $1
             WHERE organisation_id = $2 AND flow_id = $3 AND id = $4",
        )
        .bind(((index as i32) + 1) * 10)
        .bind(auth.organisation_id)
        .bind(flow_id)
        .bind(stage_id)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;

    write_audit(
        &state.db,
        auth.organisation_id,
        auth.user_id,
        "intake.stage.reorder",
        "intake_flow",
        Some(flow_id),
    )
    .await;
    Ok(Json(
        load_flow(&state.db, auth.organisation_id, flow_id).await?,
    ))
}

// ---------------------------------------------------------------------------
// Policy writes
// ---------------------------------------------------------------------------

#[utoipa::path(
    put,
    path = "/api/v1/intake/stages/{id}/actions",
    tag = "intake",
    security(("bearer_auth" = [])),
    request_body = ActionsWrite,
    params(("id" = Uuid, Path, description = "Intake stage id")),
    responses(
        (status = 200, description = "Actions replaced", body = FlowDto),
        (status = 401, description = "Missing or invalid access token", body = ErrorBody),
        (status = 403, description = "Missing intake.configure capability", body = ErrorBody),
        (status = 404, description = "Stage not found", body = ErrorBody),
        (status = 422, description = "Validation failed", body = ErrorBody)
    )
)]
pub async fn replace_actions(
    State(state): State<AppState>,
    auth: AuthContext,
    Path(id): Path<Uuid>,
    Json(payload): Json<ActionsWrite>,
) -> Result<Json<FlowDto>, AppError> {
    auth.require(Capability::IntakeConfigure)?;
    let (_, flow_id) = load_stage(&state.db, auth.organisation_id, id).await?;
    let allowed: HashSet<&str> = payload.allowed.iter().map(String::as_str).collect();
    for action in &payload.allowed {
        if !is_action_key(action) {
            return Err(AppError::validation(format!(
                "unknown action key: {action}"
            )));
        }
    }

    let mut tx = state.db.begin().await?;
    sqlx::query("DELETE FROM intake_stage_actions WHERE organisation_id = $1 AND stage_id = $2")
        .bind(auth.organisation_id)
        .bind(id)
        .execute(&mut *tx)
        .await?;
    for action in ACTION_KEYS {
        sqlx::query(
            "INSERT INTO intake_stage_actions
                (organisation_id, stage_id, action_key, allowed)
             VALUES ($1, $2, $3, $4)",
        )
        .bind(auth.organisation_id)
        .bind(id)
        .bind(action)
        .bind(allowed.contains(action))
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;

    write_audit(
        &state.db,
        auth.organisation_id,
        auth.user_id,
        "intake.stage.actions",
        "intake_stage",
        Some(id),
    )
    .await;
    Ok(Json(
        load_flow(&state.db, auth.organisation_id, flow_id).await?,
    ))
}

#[utoipa::path(
    put,
    path = "/api/v1/intake/stages/{id}/checklist",
    tag = "intake",
    security(("bearer_auth" = [])),
    request_body = ChecklistWrite,
    params(("id" = Uuid, Path, description = "Intake stage id")),
    responses(
        (status = 200, description = "Checklist replaced", body = FlowDto),
        (status = 401, description = "Missing or invalid access token", body = ErrorBody),
        (status = 403, description = "Missing intake.configure capability", body = ErrorBody),
        (status = 404, description = "Stage not found", body = ErrorBody),
        (status = 422, description = "Validation failed", body = ErrorBody)
    )
)]
pub async fn replace_checklist(
    State(state): State<AppState>,
    auth: AuthContext,
    Path(id): Path<Uuid>,
    Json(payload): Json<ChecklistWrite>,
) -> Result<Json<FlowDto>, AppError> {
    auth.require(Capability::IntakeConfigure)?;
    let (_, flow_id) = load_stage(&state.db, auth.organisation_id, id).await?;
    for item in &payload.items {
        validate_name(&item.label)?;
    }

    let mut tx = state.db.begin().await?;
    sqlx::query(
        "DELETE FROM intake_stage_checklist_items
         WHERE organisation_id = $1 AND stage_id = $2",
    )
    .bind(auth.organisation_id)
    .bind(id)
    .execute(&mut *tx)
    .await?;
    for (index, item) in payload.items.iter().enumerate() {
        sqlx::query(
            "INSERT INTO intake_stage_checklist_items
                (id, organisation_id, stage_id, label, description, is_required, sort_order)
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(Uuid::now_v7())
        .bind(auth.organisation_id)
        .bind(id)
        .bind(item.label.trim())
        .bind(
            item.description
                .as_deref()
                .map(str::trim)
                .filter(|text| !text.is_empty()),
        )
        .bind(item.is_required)
        .bind(((index as i32) + 1) * 10)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;

    write_audit(
        &state.db,
        auth.organisation_id,
        auth.user_id,
        "intake.stage.checklist",
        "intake_stage",
        Some(id),
    )
    .await;
    Ok(Json(
        load_flow(&state.db, auth.organisation_id, flow_id).await?,
    ))
}

#[utoipa::path(
    put,
    path = "/api/v1/intake/stages/{id}/assignees",
    tag = "intake",
    security(("bearer_auth" = [])),
    request_body = AssigneesWrite,
    params(("id" = Uuid, Path, description = "Intake stage id")),
    responses(
        (status = 200, description = "Assignment rules replaced", body = FlowDto),
        (status = 401, description = "Missing or invalid access token", body = ErrorBody),
        (status = 403, description = "Missing intake.configure capability", body = ErrorBody),
        (status = 404, description = "Stage not found", body = ErrorBody),
        (status = 422, description = "Validation failed", body = ErrorBody)
    )
)]
pub async fn replace_assignees(
    State(state): State<AppState>,
    auth: AuthContext,
    Path(id): Path<Uuid>,
    Json(payload): Json<AssigneesWrite>,
) -> Result<Json<FlowDto>, AppError> {
    auth.require(Capability::IntakeConfigure)?;
    let (_, flow_id) = load_stage(&state.db, auth.organisation_id, id).await?;
    let organisation_id = auth.organisation_id;

    for rule in &payload.assignees {
        if rule.user_id.is_some() == rule.permission_set_id.is_some() {
            return Err(AppError::validation(
                "each assignment rule needs exactly one user or permission set",
            ));
        }
        let strategy = rule.strategy.as_deref().unwrap_or("fixed");
        if !matches!(strategy, "fixed" | "first_available" | "round_robin") {
            return Err(AppError::validation("unknown assignment strategy"));
        }
        if strategy == "fixed" && rule.user_id.is_none() {
            return Err(AppError::validation("fixed strategy requires a user_id"));
        }
        if strategy != "fixed" && rule.permission_set_id.is_none() {
            return Err(AppError::validation(
                "pool strategies require a permission_set_id",
            ));
        }
        if let Some(user_id) = rule.user_id {
            ensure_member(&state.db, organisation_id, user_id).await?;
        }
        if let Some(set_id) = rule.permission_set_id {
            ensure_permission_set(&state.db, organisation_id, set_id).await?;
        }
    }

    let mut tx = state.db.begin().await?;
    sqlx::query(
        "DELETE FROM intake_stage_assignees
         WHERE organisation_id = $1 AND stage_id = $2",
    )
    .bind(organisation_id)
    .bind(id)
    .execute(&mut *tx)
    .await?;
    for (index, rule) in payload.assignees.iter().enumerate() {
        sqlx::query(
            "INSERT INTO intake_stage_assignees
                (id, organisation_id, stage_id, user_id, permission_set_id,
                 strategy, notify, sort_order)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        )
        .bind(Uuid::now_v7())
        .bind(organisation_id)
        .bind(id)
        .bind(rule.user_id)
        .bind(rule.permission_set_id)
        .bind(rule.strategy.as_deref().unwrap_or("fixed"))
        .bind(rule.notify)
        .bind(rule.sort_order.unwrap_or(((index as i32) + 1) * 10))
        .execute(&mut *tx)
        .await?;
    }
    sqlx::query(
        "DELETE FROM intake_assignment_cursors
         WHERE organisation_id = $1 AND stage_id = $2",
    )
    .bind(organisation_id)
    .bind(id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;

    write_audit(
        &state.db,
        organisation_id,
        auth.user_id,
        "intake.stage.assignees",
        "intake_stage",
        Some(id),
    )
    .await;
    Ok(Json(load_flow(&state.db, organisation_id, flow_id).await?))
}

// ---------------------------------------------------------------------------
// Assignee directory
// ---------------------------------------------------------------------------

#[utoipa::path(
    get,
    path = "/api/v1/intake/assignees",
    tag = "intake",
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Assignable users and permission sets", body = AssigneesDto),
        (status = 401, description = "Missing or invalid access token", body = ErrorBody),
        (status = 403, description = "Missing intake.read capability", body = ErrorBody)
    )
)]
pub async fn list_assignees(
    State(state): State<AppState>,
    auth: AuthContext,
) -> Result<Json<AssigneesDto>, AppError> {
    auth.require(Capability::IntakeRead)?;
    let users = sqlx::query_as::<_, UserRow>(
        "SELECT u.id, u.full_name, u.email
         FROM organisation_users m
         JOIN users u ON u.id = m.user_id
         WHERE m.organisation_id = $1 AND u.is_active
         ORDER BY u.full_name, u.email",
    )
    .bind(auth.organisation_id)
    .fetch_all(&state.db)
    .await?;
    let permission_sets = sqlx::query_as::<_, PermissionSetRow>(
        "SELECT id, name, persona_key
         FROM permission_sets
         WHERE organisation_id = $1
         ORDER BY sort_order, name",
    )
    .bind(auth.organisation_id)
    .fetch_all(&state.db)
    .await?;

    Ok(Json(AssigneesDto {
        users: users
            .into_iter()
            .map(|row| UserRefDto {
                id: row.id,
                full_name: row.full_name,
                email: row.email,
            })
            .collect(),
        permission_sets: permission_sets
            .into_iter()
            .map(|row| PermissionSetRefDto {
                id: row.id,
                name: row.name,
                persona_key: row.persona_key,
            })
            .collect(),
    }))
}

// ---------------------------------------------------------------------------
// Validation and helpers
// ---------------------------------------------------------------------------

async fn load_stage(
    db: &PgPool,
    organisation_id: Uuid,
    id: Uuid,
) -> Result<(StageRow, Uuid), AppError> {
    let row = sqlx::query_as::<_, StageRow>(
        "SELECT id, flow_id, key, name, stage_kind, color, sort_order, is_terminal,
                on_enter_contact_status, sla_hours, checklist_gate,
                visible_fields, required_fields
         FROM intake_stages
         WHERE organisation_id = $1 AND id = $2",
    )
    .bind(organisation_id)
    .bind(id)
    .fetch_optional(db)
    .await?
    .ok_or_else(|| AppError::not_found("intake stage not found"))?;
    let flow_id = row.flow_id;
    Ok((row, flow_id))
}

async fn stage_has_cases(
    db: &PgPool,
    organisation_id: Uuid,
    stage_id: Uuid,
) -> Result<bool, AppError> {
    Ok(sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(
             SELECT 1 FROM intake_cases
             WHERE organisation_id = $1 AND stage_id = $2
         )",
    )
    .bind(organisation_id)
    .bind(stage_id)
    .fetch_one(db)
    .await?)
}

async fn ensure_flow_exists(db: &PgPool, organisation_id: Uuid, id: Uuid) -> Result<(), AppError> {
    let exists = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM intake_flows WHERE organisation_id = $1 AND id = $2)",
    )
    .bind(organisation_id)
    .bind(id)
    .fetch_one(db)
    .await?;
    if exists {
        Ok(())
    } else {
        Err(AppError::not_found("intake flow not found"))
    }
}

async fn ensure_division_exists(
    db: &PgPool,
    organisation_id: Uuid,
    id: Uuid,
) -> Result<(), AppError> {
    let exists = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM divisions WHERE organisation_id = $1 AND id = $2)",
    )
    .bind(organisation_id)
    .bind(id)
    .fetch_one(db)
    .await?;
    if exists {
        Ok(())
    } else {
        Err(AppError::not_found("division not found"))
    }
}

async fn ensure_member(db: &PgPool, organisation_id: Uuid, user_id: Uuid) -> Result<(), AppError> {
    let exists = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM organisation_users WHERE organisation_id = $1 AND user_id = $2)",
    )
    .bind(organisation_id)
    .bind(user_id)
    .fetch_one(db)
    .await?;
    if exists {
        Ok(())
    } else {
        Err(AppError::validation(
            "user is not a member of this organisation",
        ))
    }
}

async fn ensure_permission_set(
    db: &PgPool,
    organisation_id: Uuid,
    set_id: Uuid,
) -> Result<(), AppError> {
    let exists = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM permission_sets WHERE organisation_id = $1 AND id = $2)",
    )
    .bind(organisation_id)
    .bind(set_id)
    .fetch_one(db)
    .await?;
    if exists {
        Ok(())
    } else {
        Err(AppError::validation("permission set not found"))
    }
}

async fn custom_field_keys(db: &PgPool, organisation_id: Uuid) -> Result<Vec<String>, AppError> {
    Ok(sqlx::query_scalar::<_, String>(
        "SELECT key FROM custom_field_definitions
         WHERE organisation_id = $1 AND entity_type = 'contact' AND is_active",
    )
    .bind(organisation_id)
    .fetch_all(db)
    .await?)
}

pub(crate) fn validate_key(key: &str) -> Result<(), AppError> {
    if key.len() < 2
        || key.len() > 64
        || !key
            .chars()
            .next()
            .is_some_and(|first| first.is_ascii_lowercase())
        || !key.chars().all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '_'
        })
    {
        return Err(AppError::validation(
            "key must be lower case letters, digits and underscores, starting with a letter",
        ));
    }
    Ok(())
}

pub(crate) fn validate_name(name: &str) -> Result<String, AppError> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 160 {
        return Err(AppError::validation(
            "name must be between 1 and 160 characters",
        ));
    }
    Ok(name.to_owned())
}

fn validate_lifecycle_nudge(value: Option<&str>) -> Result<(), AppError> {
    if let Some(value) = value {
        if !matches!(
            value,
            "enquiry" | "onboarding" | "active" | "intake_cancelled"
        ) {
            return Err(AppError::validation("invalid lifecycle nudge"));
        }
    }
    Ok(())
}

fn validate_sla(hours: Option<i32>) -> Result<(), AppError> {
    if hours.is_some_and(|hours| hours <= 0) {
        return Err(AppError::validation("sla_hours must be positive"));
    }
    Ok(())
}

pub(crate) fn validate_fields(
    visible: &[String],
    required: &[String],
    custom_keys: &[String],
) -> Result<(), AppError> {
    let mut seen = HashSet::new();
    for key in visible {
        if !seen.insert(key) {
            return Err(AppError::validation(format!("duplicate field key: {key}")));
        }
        if !field_key_is_valid(key, custom_keys) {
            return Err(AppError::validation(format!("unknown field key: {key}")));
        }
    }
    for key in required {
        if !visible.contains(key) {
            return Err(AppError::validation(format!(
                "required field is not visible: {key}"
            )));
        }
        if !field_key_is_valid(key, custom_keys) {
            return Err(AppError::validation(format!("unknown field key: {key}")));
        }
    }
    Ok(())
}

fn validate_stage_sequence(stages: &[StageDto]) -> Result<(), AppError> {
    if stages.is_empty() {
        return Err(AppError::validation("a flow needs at least one stage"));
    }
    let enquiry: Vec<_> = stages
        .iter()
        .filter(|stage| stage.kind == "enquiry")
        .collect();
    let onboarded = stages
        .iter()
        .filter(|stage| stage.kind == "onboarded")
        .count();
    let cancelled = stages
        .iter()
        .filter(|stage| stage.kind == "cancelled")
        .count();
    if enquiry.len() != 1 || onboarded != 1 || cancelled != 1 {
        return Err(AppError::validation(
            "a flow needs exactly one enquiry, one onboarded and one cancelled stage",
        ));
    }
    if stages.first().is_none_or(|stage| stage.kind != "enquiry") {
        return Err(AppError::validation("the first stage must be enquiry"));
    }
    let mut seen_terminal = false;
    for stage in stages {
        if seen_terminal && !stage.is_terminal {
            return Err(AppError::validation("no stage may follow a terminal stage"));
        }
        if stage.is_terminal {
            seen_terminal = true;
        }
    }
    Ok(())
}

fn validate_stage_sequence_candidate(
    flow: &FlowDto,
    key: &str,
    kind: &str,
) -> Result<(), AppError> {
    if flow.stages.iter().any(|stage| stage.key == key) {
        return Err(AppError::conflict("stage key already exists in this flow"));
    }
    let mut candidate = flow.clone();
    let new_stage = StageDto {
        id: Uuid::nil(),
        key: key.to_owned(),
        name: key.to_owned(),
        kind: kind.to_owned(),
        color: None,
        sort_order: 0,
        is_terminal: intake::is_terminal_kind(kind),
        on_enter_contact_status: None,
        sla_hours: None,
        checklist_gate: false,
        visible_fields: Vec::new(),
        required_fields: Vec::new(),
        actions: BTreeMap::new(),
        assignees: Vec::new(),
        checklist: Vec::new(),
    };
    let insertion = if new_stage.is_terminal {
        candidate.stages.len()
    } else {
        candidate
            .stages
            .iter()
            .position(|stage| stage.is_terminal)
            .unwrap_or(candidate.stages.len())
    };
    candidate.stages.insert(insertion, new_stage);
    validate_stage_sequence(&candidate.stages)
}

fn map_conflict(error: sqlx::Error) -> AppError {
    if let sqlx::Error::Database(database) = &error {
        if database.code().as_deref() == Some("23505") {
            return AppError::conflict("that key is already in use");
        }
    }
    AppError::from(error)
}

fn string_array(value: Value) -> Vec<String> {
    value
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

fn trim_optional(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}
