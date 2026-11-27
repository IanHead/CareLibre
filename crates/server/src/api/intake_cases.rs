//! Intake cases, the board, transitions, contact matching and lifecycle.

use std::collections::{BTreeMap, HashMap, HashSet};

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use chrono::{DateTime, Duration, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sqlx::types::Json as SqlJson;
use sqlx::{FromRow, PgPool, Postgres, QueryBuilder, Transaction};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::{
    api::intake::{descendant_divisions, load_flows, FlowDto, StageDto, UserRefDto},
    auth::extractor::AuthContext,
    error::{AppError, ErrorBody},
    intake::{self, cases as case_service, ACTION_KEYS},
    permissions::Capability,
    state::AppState,
    worker_qualities::{normalise_gender, normalise_languages},
};

// ---------------------------------------------------------------------------
// DTOs
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct FlowRefDto {
    pub id: Uuid,
    pub key: String,
    pub name: String,
    pub subject_kind: String,
    pub variant: Option<String>,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct StageRefDto {
    pub id: Uuid,
    pub key: String,
    pub name: String,
    pub kind: String,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct DivisionRefDto {
    pub id: Uuid,
    pub name: String,
    pub parent_id: Option<Uuid>,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct ContactSummaryDto {
    pub id: Uuid,
    pub display_name: String,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub date_of_birth: Option<NaiveDate>,
    pub lifecycle_status: String,
    pub role: String,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct ChecklistProgressDto {
    pub done: i64,
    pub total: i64,
    pub required_done: i64,
    pub required_total: i64,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct CaseSummaryDto {
    pub id: Uuid,
    pub display_name: String,
    pub contact_id: Option<Uuid>,
    pub flow: FlowRefDto,
    pub stage_id: Uuid,
    pub stage: StageRefDto,
    pub lifecycle_status: Option<String>,
    pub division: Option<DivisionRefDto>,
    pub assignee: Option<UserRefDto>,
    pub priority: i16,
    pub source: String,
    pub waitlisted: bool,
    pub checklist: ChecklistProgressDto,
    pub stage_entered_at: DateTime<Utc>,
    pub stage_due_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
    pub version: i32,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct ReferralDto {
    pub source: String,
    pub referrer_contact_id: Option<Uuid>,
    pub referrer_organisation_id: Option<Uuid>,
    pub reference: Option<String>,
    pub received_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct CaseDetailCaseDto {
    pub id: Uuid,
    pub display_name: String,
    pub contact_id: Option<Uuid>,
    pub flow: FlowRefDto,
    pub stage_id: Uuid,
    pub stage: StageRefDto,
    pub lifecycle_status: Option<String>,
    pub division: Option<DivisionRefDto>,
    pub assignee: Option<UserRefDto>,
    pub priority: i16,
    pub source: String,
    pub waitlisted: bool,
    pub checklist: ChecklistProgressDto,
    pub stage_entered_at: DateTime<Utc>,
    pub stage_due_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
    pub version: i32,
    pub data: Value,
    pub referral: ReferralDto,
    pub created_by: Option<UserRefDto>,
    pub created_at: DateTime<Utc>,
    pub onboarded_at: Option<DateTime<Utc>>,
    pub closed_at: Option<DateTime<Utc>>,
    pub cancellation_reason: Option<String>,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct CaseChecklistItemDto {
    pub id: Uuid,
    pub label: String,
    pub description: Option<String>,
    pub is_required: bool,
    pub completed_at: Option<DateTime<Utc>>,
    pub completed_by: Option<UserRefDto>,
    pub note: Option<String>,
    pub sort_order: i32,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct ActionPolicyDto {
    pub allowed: bool,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct WaitlistEntryDto {
    pub id: Uuid,
    pub service_id: Option<Uuid>,
    pub service_name: Option<String>,
    pub division_id: Option<Uuid>,
    pub division_name: Option<String>,
    pub priority: i16,
    pub requested_start: Option<NaiveDate>,
    pub status: String,
    pub offered_at: Option<DateTime<Utc>>,
    pub offer_expires_at: Option<DateTime<Utc>>,
    pub notes: Option<String>,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct CaseDetailDto {
    pub case: CaseDetailCaseDto,
    pub contact: Option<ContactSummaryDto>,
    pub flow: FlowDto,
    pub stage: StageDto,
    pub visible_fields: Vec<String>,
    pub required_fields: Vec<String>,
    pub checklist: Vec<CaseChecklistItemDto>,
    pub allowed_transitions: Vec<StageRefDto>,
    pub action_policy: BTreeMap<String, ActionPolicyDto>,
    pub waitlist: Vec<WaitlistEntryDto>,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct CaseCreateResponse {
    pub case: CaseDetailDto,
    pub contact: Option<ContactSummaryDto>,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct BoardColumnDto {
    pub stage: StageRefDto,
    pub count: i64,
    pub cards: Vec<CaseSummaryDto>,
    pub next_cursor: Option<String>,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct BoardDto {
    pub flow: FlowDto,
    pub columns: Vec<BoardColumnDto>,
    pub generated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct CaseListDto {
    pub items: Vec<CaseSummaryDto>,
    pub next_cursor: Option<String>,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct OpenCaseRefDto {
    pub id: Uuid,
    pub flow_id: Uuid,
    pub flow_name: String,
    pub stage_name: String,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct ContactMatchDto {
    pub id: Uuid,
    pub display_name: String,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub date_of_birth: Option<NaiveDate>,
    pub lifecycle_status: String,
    #[serde(rename = "match")]
    pub match_kind: String,
    pub open_case: Option<OpenCaseRefDto>,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct ContactMatchResponse {
    pub candidates: Vec<ContactMatchDto>,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct TimelineEventDto {
    pub id: i64,
    pub event_type: String,
    pub actor: Option<UserRefDto>,
    pub from_stage: Option<StageRefDto>,
    pub to_stage: Option<StageRefDto>,
    pub metadata: Value,
    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct TimelineResponse {
    pub items: Vec<TimelineEventDto>,
    pub next: Option<i64>,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct ChecklistToggleResponse {
    pub item: CaseChecklistItemDto,
    pub checklist: ChecklistProgressDto,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct ContactStatusHistoryDto {
    pub id: Uuid,
    pub from_status: Option<String>,
    pub to_status: String,
    pub reason: Option<String>,
    pub actor: Option<UserRefDto>,
    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct ContactStatusResponse {
    pub contact: ContactSummaryDto,
    pub history: ContactStatusHistoryDto,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct ContactStatusHistoryResponse {
    pub items: Vec<ContactStatusHistoryDto>,
    pub next_cursor: Option<String>,
}

#[derive(Clone, Debug, Deserialize, ToSchema)]
pub struct ContactCreate {
    pub given_name: Option<String>,
    pub family_name: Option<String>,
    pub preferred_name: Option<String>,
    pub pronouns: Option<String>,
    pub date_of_birth: Option<NaiveDate>,
    pub gender: Option<String>,
    #[serde(default)]
    pub languages: Vec<String>,
    pub cultural_identity: Option<String>,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub notes: Option<String>,
    pub details: Option<Value>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CaseCreate {
    pub client_request_id: Option<String>,
    pub legacy_draft_id: Option<String>,
    pub flow_id: Uuid,
    pub contact_id: Option<Uuid>,
    pub external_organisation_id: Option<Uuid>,
    pub contact: Option<ContactCreate>,
    pub display_name: Option<String>,
    pub source: Option<String>,
    pub referrer_contact_id: Option<Uuid>,
    pub referrer_organisation_id: Option<Uuid>,
    pub referral_reference: Option<String>,
    pub referral_received_at: Option<DateTime<Utc>>,
    pub division_id: Option<Uuid>,
    pub priority: Option<i16>,
    #[serde(default = "empty_object")]
    pub data: Value,
    #[serde(default)]
    pub allow_duplicate_contact: bool,
    pub duplicate_reason: Option<String>,
}

fn empty_object() -> Value {
    Value::Object(Map::new())
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CaseUpdate {
    pub version: i32,
    pub display_name: Option<String>,
    pub data: Option<Value>,
    pub assignee_user_id: Option<Uuid>,
    #[serde(default)]
    pub unassign: bool,
    pub division_id: Option<Uuid>,
    pub priority: Option<i16>,
    #[serde(default)]
    pub force: bool,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct TransitionRequest {
    pub to_stage_id: Uuid,
    pub version: i32,
    pub reason: Option<String>,
    #[serde(default, rename = "override")]
    pub override_validation: bool,
    pub override_reason: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ChecklistToggle {
    #[serde(default)]
    pub completed: bool,
    pub note: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CancelRequest {
    pub version: i32,
    pub reason: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ArchiveRequest {
    pub version: i32,
    pub reason: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ContactStatusWrite {
    pub status: String,
    pub reason: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct ContactMatchParams {
    pub email: Option<String>,
    pub phone: Option<String>,
    pub name: Option<String>,
    pub date_of_birth: Option<NaiveDate>,
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct BoardParams {
    pub flow_id: Uuid,
    pub division_id: Option<Uuid>,
    pub include_descendants: Option<bool>,
    pub assignee_user_id: Option<String>,
    pub lifecycle_status: Option<String>,
    pub search: Option<String>,
    pub include_closed: Option<bool>,
    pub limit_per_stage: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct CaseListParams {
    pub flow_id: Uuid,
    pub stage_id: Option<Uuid>,
    pub division_id: Option<Uuid>,
    pub include_descendants: Option<bool>,
    pub assignee_user_id: Option<String>,
    pub lifecycle_status: Option<String>,
    pub search: Option<String>,
    pub include_closed: Option<bool>,
    pub cursor: Option<String>,
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct TimelineParams {
    pub limit: Option<u32>,
    pub before: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct HistoryParams {
    pub limit: Option<u32>,
}

// ---------------------------------------------------------------------------
// Rows
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, FromRow)]
struct CaseRow {
    id: Uuid,
    display_name: String,
    contact_id: Option<Uuid>,
    flow_id: Uuid,
    flow_key: String,
    flow_name: String,
    subject_kind: String,
    variant: Option<String>,
    stage_id: Uuid,
    stage_key: String,
    stage_name: String,
    stage_kind: String,
    stage_is_terminal: bool,
    lifecycle_status: Option<String>,
    contact_display_name: Option<String>,
    contact_email: Option<String>,
    contact_phone: Option<String>,
    contact_date_of_birth: Option<NaiveDate>,
    contact_role: Option<String>,
    division_id: Option<Uuid>,
    division_name: Option<String>,
    division_parent_id: Option<Uuid>,
    assignee_user_id: Option<Uuid>,
    assignee_name: Option<String>,
    assignee_email: Option<String>,
    priority: i16,
    source: String,
    waitlisted: bool,
    checklist_total: i64,
    checklist_done: i64,
    checklist_required_total: i64,
    checklist_required_done: i64,
    stage_entered_at: DateTime<Utc>,
    stage_due_at: Option<DateTime<Utc>>,
    updated_at: DateTime<Utc>,
    version: i32,
    data: SqlJson<Value>,
    referrer_contact_id: Option<Uuid>,
    referrer_organisation_id: Option<Uuid>,
    referral_reference: Option<String>,
    referral_received_at: Option<DateTime<Utc>>,
    created_by: Option<Uuid>,
    created_by_name: Option<String>,
    created_by_email: Option<String>,
    created_at: DateTime<Utc>,
    onboarded_at: Option<DateTime<Utc>>,
    closed_at: Option<DateTime<Utc>>,
    cancellation_reason: Option<String>,
}

#[derive(Debug, FromRow)]
struct ContactRow {
    id: Uuid,
    display_name: String,
    given_name: Option<String>,
    family_name: Option<String>,
    email: Option<String>,
    phone: Option<String>,
    date_of_birth: Option<NaiveDate>,
    lifecycle_status: String,
    role: String,
}

#[derive(Debug, FromRow)]
struct ContactMatchRow {
    id: Uuid,
    display_name: String,
    email: Option<String>,
    phone: Option<String>,
    date_of_birth: Option<NaiveDate>,
    lifecycle_status: String,
}

#[derive(Debug, FromRow)]
struct OpenCaseRow {
    contact_id: Uuid,
    case_id: Uuid,
    flow_id: Uuid,
    flow_name: String,
    stage_name: String,
}

#[derive(Debug, FromRow)]
struct ChecklistItemRow {
    id: Uuid,
    label: String,
    description: Option<String>,
    is_required: bool,
    completed_at: Option<DateTime<Utc>>,
    completed_by: Option<Uuid>,
    completed_by_name: Option<String>,
    completed_by_email: Option<String>,
    note: Option<String>,
    sort_order: i32,
}

#[derive(Debug, FromRow)]
struct WaitlistRow {
    id: Uuid,
    service_id: Option<Uuid>,
    service_name: Option<String>,
    division_id: Option<Uuid>,
    division_name: Option<String>,
    priority: i16,
    requested_start: Option<NaiveDate>,
    status: String,
    offered_at: Option<DateTime<Utc>>,
    offer_expires_at: Option<DateTime<Utc>>,
    notes: Option<String>,
}

#[derive(Debug, FromRow)]
struct TimelineRow {
    id: i64,
    event_type: String,
    actor_id: Option<Uuid>,
    actor_name: Option<String>,
    actor_email: Option<String>,
    from_stage_id: Option<Uuid>,
    from_stage_key: Option<String>,
    from_stage_name: Option<String>,
    from_stage_kind: Option<String>,
    to_stage_id: Option<Uuid>,
    to_stage_key: Option<String>,
    to_stage_name: Option<String>,
    to_stage_kind: Option<String>,
    metadata: SqlJson<Value>,
    created_at: DateTime<Utc>,
}

#[derive(Debug, FromRow)]
struct StatusHistoryRow {
    id: Uuid,
    from_status: Option<String>,
    to_status: String,
    reason: Option<String>,
    actor_id: Option<Uuid>,
    actor_name: Option<String>,
    actor_email: Option<String>,
    created_at: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// Case SQL
// ---------------------------------------------------------------------------

const CASE_FROM: &str = r#"
FROM intake_cases c
JOIN intake_flows f
  ON f.organisation_id = c.organisation_id AND f.id = c.flow_id
JOIN intake_stages s
  ON s.organisation_id = c.organisation_id AND s.id = c.stage_id
LEFT JOIN contacts ct
  ON ct.organisation_id = c.organisation_id AND ct.id = c.contact_id
LEFT JOIN contact_care_roles r ON r.contact_id = c.contact_id
LEFT JOIN divisions d
  ON d.organisation_id = c.organisation_id AND d.id = c.division_id
LEFT JOIN users u ON u.id = c.assignee_user_id
LEFT JOIN users cb ON cb.id = c.created_by
"#;

const CASE_SELECT: &str = r#"
SELECT c.id, c.display_name, c.contact_id, c.flow_id,
       f.key AS flow_key, f.name AS flow_name, f.subject_kind, f.variant,
       c.stage_id, s.key AS stage_key, s.name AS stage_name,
       s.stage_kind, s.is_terminal AS stage_is_terminal,
       ct.lifecycle_status, ct.display_name AS contact_display_name,
       ct.email AS contact_email, ct.phone AS contact_phone,
       ct.date_of_birth AS contact_date_of_birth,
       CASE WHEN c.contact_id IS NULL THEN NULL
            WHEN r.is_client AND r.is_worker THEN 'dual'
            WHEN r.is_client THEN 'client'
            WHEN r.is_worker THEN 'worker'
            ELSE 'other' END AS contact_role,
       c.division_id, d.name AS division_name, d.parent_id AS division_parent_id,
       c.assignee_user_id, u.full_name AS assignee_name, u.email AS assignee_email,
       c.priority, c.source,
       EXISTS(
           SELECT 1 FROM intake_waitlist_entries w
           WHERE w.organisation_id = c.organisation_id AND w.case_id = c.id
             AND w.status IN ('waiting', 'offered')
       ) AS waitlisted,
       (SELECT count(*) FROM intake_case_checklist_items ci
        WHERE ci.organisation_id = c.organisation_id AND ci.case_id = c.id
          AND ci.stage_id = c.stage_id) AS checklist_total,
       (SELECT count(*) FROM intake_case_checklist_items ci
        WHERE ci.organisation_id = c.organisation_id AND ci.case_id = c.id
          AND ci.stage_id = c.stage_id AND ci.completed_at IS NOT NULL) AS checklist_done,
       (SELECT count(*) FROM intake_case_checklist_items ci
        WHERE ci.organisation_id = c.organisation_id AND ci.case_id = c.id
          AND ci.stage_id = c.stage_id AND ci.is_required) AS checklist_required_total,
       (SELECT count(*) FROM intake_case_checklist_items ci
        WHERE ci.organisation_id = c.organisation_id AND ci.case_id = c.id
          AND ci.stage_id = c.stage_id AND ci.is_required
          AND ci.completed_at IS NOT NULL) AS checklist_required_done,
       c.stage_entered_at, c.stage_due_at, c.updated_at, c.version, c.data,
       c.referrer_contact_id, c.referrer_organisation_id, c.referral_reference,
       c.referral_received_at, c.created_by,
       cb.full_name AS created_by_name, cb.email AS created_by_email,
       c.created_at, c.onboarded_at, c.closed_at, c.cancellation_reason
"#;

#[derive(Clone, Copy, Debug)]
enum AssigneeFilter {
    Any,
    Me(Uuid),
    Unassigned,
    User(Uuid),
}

#[derive(Clone, Debug)]
struct CaseFilter {
    organisation_id: Uuid,
    flow_id: Uuid,
    stage_id: Option<Uuid>,
    division_ids: Option<Vec<Uuid>>,
    assignee: AssigneeFilter,
    lifecycle_statuses: Option<Vec<String>>,
    search: Option<String>,
    include_closed: bool,
}

fn push_case_filters(query: &mut QueryBuilder<'_, Postgres>, filter: &CaseFilter) {
    query.push(" WHERE c.organisation_id = ");
    query.push_bind(filter.organisation_id);
    query.push(" AND c.flow_id = ");
    query.push_bind(filter.flow_id);
    if let Some(stage_id) = filter.stage_id {
        query.push(" AND c.stage_id = ");
        query.push_bind(stage_id);
    }
    if !filter.include_closed {
        query.push(" AND c.closed_at IS NULL");
    }
    if let Some(division_ids) = &filter.division_ids {
        query.push(" AND c.division_id = ANY(");
        query.push_bind(division_ids.clone());
        query.push(")");
    }
    match filter.assignee {
        AssigneeFilter::Any => {}
        AssigneeFilter::Me(user_id) => {
            query.push(" AND c.assignee_user_id = ");
            query.push_bind(user_id);
        }
        AssigneeFilter::Unassigned => {
            query.push(" AND c.assignee_user_id IS NULL");
        }
        AssigneeFilter::User(user_id) => {
            query.push(" AND c.assignee_user_id = ");
            query.push_bind(user_id);
        }
    }
    if let Some(statuses) = &filter.lifecycle_statuses {
        query.push(" AND ct.lifecycle_status = ANY(");
        query.push_bind(statuses.clone());
        query.push(")");
    }
    if let Some(search) = &filter.search {
        let pattern = format!("%{search}%");
        query.push(" AND (c.display_name ILIKE ");
        query.push_bind(pattern.clone());
        query.push(" OR ct.display_name ILIKE ");
        query.push_bind(pattern.clone());
        query.push(" OR ct.email ILIKE ");
        query.push_bind(pattern.clone());
        query.push(" OR ct.phone ILIKE ");
        query.push_bind(pattern);
        query.push(")");
    }
}

async fn count_cases(
    db: &PgPool,
    filter: &CaseFilter,
    cursor: Option<&CaseCursor>,
) -> Result<i64, AppError> {
    let mut query = QueryBuilder::<Postgres>::new("SELECT count(*) ");
    query.push(CASE_FROM);
    push_case_filters(&mut query, filter);
    push_cursor_filter(&mut query, cursor);
    Ok(query.build_query_scalar().fetch_one(db).await?)
}

async fn load_case_rows(
    db: &PgPool,
    filter: &CaseFilter,
    limit: u32,
    cursor: Option<&CaseCursor>,
) -> Result<Vec<CaseRow>, AppError> {
    let mut query = QueryBuilder::<Postgres>::new(CASE_SELECT);
    query.push(CASE_FROM);
    push_case_filters(&mut query, filter);
    push_cursor_filter(&mut query, cursor);
    query.push(" ORDER BY c.priority DESC, c.stage_entered_at ASC, c.id ASC LIMIT ");
    query.push_bind(i64::from(limit));
    Ok(query.build_query_as::<CaseRow>().fetch_all(db).await?)
}

#[derive(Clone, Debug)]
struct CaseCursor {
    priority: i16,
    entered_at: DateTime<Utc>,
    id: Uuid,
}

impl CaseCursor {
    fn from_row(row: &CaseRow) -> Self {
        Self {
            priority: row.priority,
            entered_at: row.stage_entered_at,
            id: row.id,
        }
    }

    fn encode(&self) -> String {
        format!(
            "{}|{}|{}",
            self.priority,
            self.entered_at.timestamp_micros(),
            self.id
        )
    }

    fn decode(value: &str) -> Result<Self, AppError> {
        let mut parts = value.splitn(3, '|');
        let priority = parts
            .next()
            .and_then(|value| value.parse::<i16>().ok())
            .ok_or_else(|| AppError::validation("invalid cursor"))?;
        let micros = parts
            .next()
            .and_then(|value| value.parse::<i64>().ok())
            .ok_or_else(|| AppError::validation("invalid cursor"))?;
        let id = parts
            .next()
            .and_then(|value| Uuid::parse_str(value).ok())
            .ok_or_else(|| AppError::validation("invalid cursor"))?;
        let entered_at = DateTime::<Utc>::from_timestamp_micros(micros)
            .ok_or_else(|| AppError::validation("invalid cursor"))?;
        Ok(Self {
            priority,
            entered_at,
            id,
        })
    }
}

fn push_cursor_filter(query: &mut QueryBuilder<'_, Postgres>, cursor: Option<&CaseCursor>) {
    if let Some(cursor) = cursor {
        query.push(" AND (c.priority < ");
        query.push_bind(cursor.priority);
        query.push(" OR (c.priority = ");
        query.push_bind(cursor.priority);
        query.push(" AND (c.stage_entered_at > ");
        query.push_bind(cursor.entered_at);
        query.push(" OR (c.stage_entered_at = ");
        query.push_bind(cursor.entered_at);
        query.push(" AND c.id > ");
        query.push_bind(cursor.id);
        query.push("))))");
    }
}

// ---------------------------------------------------------------------------
// Conversions
// ---------------------------------------------------------------------------

fn flow_ref(row: &CaseRow) -> FlowRefDto {
    FlowRefDto {
        id: row.flow_id,
        key: row.flow_key.clone(),
        name: row.flow_name.clone(),
        subject_kind: row.subject_kind.clone(),
        variant: row.variant.clone(),
    }
}

fn stage_ref(row: &CaseRow) -> StageRefDto {
    StageRefDto {
        id: row.stage_id,
        key: row.stage_key.clone(),
        name: row.stage_name.clone(),
        kind: row.stage_kind.clone(),
    }
}

fn checklist_progress(row: &CaseRow) -> ChecklistProgressDto {
    ChecklistProgressDto {
        done: row.checklist_done,
        total: row.checklist_total,
        required_done: row.checklist_required_done,
        required_total: row.checklist_required_total,
    }
}

fn assignee(row: &CaseRow) -> Option<UserRefDto> {
    Some(UserRefDto {
        id: row.assignee_user_id?,
        full_name: row.assignee_name.clone()?,
        email: row.assignee_email.clone()?,
    })
}

fn division(row: &CaseRow) -> Option<DivisionRefDto> {
    Some(DivisionRefDto {
        id: row.division_id?,
        name: row.division_name.clone()?,
        parent_id: row.division_parent_id,
    })
}

fn summary(row: &CaseRow) -> CaseSummaryDto {
    CaseSummaryDto {
        id: row.id,
        display_name: row.display_name.clone(),
        contact_id: row.contact_id,
        flow: flow_ref(row),
        stage_id: row.stage_id,
        stage: stage_ref(row),
        lifecycle_status: row.lifecycle_status.clone(),
        division: division(row),
        assignee: assignee(row),
        priority: row.priority,
        source: row.source.clone(),
        waitlisted: row.waitlisted,
        checklist: checklist_progress(row),
        stage_entered_at: row.stage_entered_at,
        stage_due_at: row.stage_due_at,
        updated_at: row.updated_at,
        version: row.version,
    }
}

fn contact_summary(row: &CaseRow) -> Option<ContactSummaryDto> {
    Some(ContactSummaryDto {
        id: row.contact_id?,
        display_name: row.contact_display_name.clone()?,
        email: row.contact_email.clone(),
        phone: row.contact_phone.clone(),
        date_of_birth: row.contact_date_of_birth,
        lifecycle_status: row.lifecycle_status.clone()?,
        role: row
            .contact_role
            .clone()
            .unwrap_or_else(|| "other".to_owned()),
    })
}

fn created_by(row: &CaseRow) -> Option<UserRefDto> {
    Some(UserRefDto {
        id: row.created_by?,
        full_name: row.created_by_name.clone()?,
        email: row.created_by_email.clone()?,
    })
}

fn detail_case(row: &CaseRow, data: Value) -> CaseDetailCaseDto {
    CaseDetailCaseDto {
        id: row.id,
        display_name: row.display_name.clone(),
        contact_id: row.contact_id,
        flow: flow_ref(row),
        stage_id: row.stage_id,
        stage: stage_ref(row),
        lifecycle_status: row.lifecycle_status.clone(),
        division: division(row),
        assignee: assignee(row),
        priority: row.priority,
        source: row.source.clone(),
        waitlisted: row.waitlisted,
        checklist: checklist_progress(row),
        stage_entered_at: row.stage_entered_at,
        stage_due_at: row.stage_due_at,
        updated_at: row.updated_at,
        version: row.version,
        data,
        referral: ReferralDto {
            source: row.source.clone(),
            referrer_contact_id: row.referrer_contact_id,
            referrer_organisation_id: row.referrer_organisation_id,
            reference: row.referral_reference.clone(),
            received_at: row.referral_received_at,
        },
        created_by: created_by(row),
        created_at: row.created_at,
        onboarded_at: row.onboarded_at,
        closed_at: row.closed_at,
        cancellation_reason: row.cancellation_reason.clone(),
    }
}

fn stage_to_ref(stage: &StageDto) -> StageRefDto {
    StageRefDto {
        id: stage.id,
        key: stage.key.clone(),
        name: stage.name.clone(),
        kind: stage.kind.clone(),
    }
}

fn checklist_dto(row: ChecklistItemRow) -> CaseChecklistItemDto {
    let completed_by = match (
        row.completed_by,
        row.completed_by_name.clone(),
        row.completed_by_email.clone(),
    ) {
        (Some(id), Some(full_name), Some(email)) => Some(UserRefDto {
            id,
            full_name,
            email,
        }),
        _ => None,
    };
    CaseChecklistItemDto {
        id: row.id,
        label: row.label,
        description: row.description,
        is_required: row.is_required,
        completed_at: row.completed_at,
        completed_by,
        note: row.note,
        sort_order: row.sort_order,
    }
}

fn timeline_dto(row: TimelineRow) -> TimelineEventDto {
    let actor = match (row.actor_id, row.actor_name, row.actor_email) {
        (Some(id), Some(full_name), Some(email)) => Some(UserRefDto {
            id,
            full_name,
            email,
        }),
        _ => None,
    };
    let from_stage = match (
        row.from_stage_id,
        row.from_stage_key,
        row.from_stage_name,
        row.from_stage_kind,
    ) {
        (Some(id), Some(key), Some(name), Some(kind)) => Some(StageRefDto {
            id,
            key,
            name,
            kind,
        }),
        _ => None,
    };
    let to_stage = match (
        row.to_stage_id,
        row.to_stage_key,
        row.to_stage_name,
        row.to_stage_kind,
    ) {
        (Some(id), Some(key), Some(name), Some(kind)) => Some(StageRefDto {
            id,
            key,
            name,
            kind,
        }),
        _ => None,
    };
    TimelineEventDto {
        id: row.id,
        event_type: row.event_type,
        actor,
        from_stage,
        to_stage,
        metadata: row.metadata.0,
        created_at: row.created_at,
    }
}

fn status_history_dto(row: StatusHistoryRow) -> ContactStatusHistoryDto {
    let actor = match (row.actor_id, row.actor_name, row.actor_email) {
        (Some(id), Some(full_name), Some(email)) => Some(UserRefDto {
            id,
            full_name,
            email,
        }),
        _ => None,
    };
    ContactStatusHistoryDto {
        id: row.id,
        from_status: row.from_status,
        to_status: row.to_status,
        reason: row.reason,
        actor,
        created_at: row.created_at,
    }
}

// ---------------------------------------------------------------------------
// Reads
// ---------------------------------------------------------------------------

async fn load_case_row(db: &PgPool, organisation_id: Uuid, id: Uuid) -> Result<CaseRow, AppError> {
    let mut query = QueryBuilder::<Postgres>::new(CASE_SELECT);
    query.push(CASE_FROM);
    query.push(" WHERE c.organisation_id = ");
    query.push_bind(organisation_id);
    query.push(" AND c.id = ");
    query.push_bind(id);
    query
        .build_query_as::<CaseRow>()
        .fetch_optional(db)
        .await?
        .ok_or_else(|| AppError::not_found("intake case not found"))
}

async fn load_case_detail(
    db: &PgPool,
    organisation_id: Uuid,
    row: &CaseRow,
) -> Result<CaseDetailDto, AppError> {
    let flows = load_flows(db, organisation_id, true).await?;
    let flow = flows
        .into_iter()
        .find(|flow| flow.id == row.flow_id)
        .ok_or_else(|| AppError::not_found("intake flow not found"))?;
    let stage = flow
        .stages
        .iter()
        .find(|stage| stage.id == row.stage_id)
        .cloned()
        .ok_or_else(|| AppError::not_found("intake stage not found"))?;
    let data =
        case_service::merge_case_data(db, organisation_id, row.id, row.data.0.clone()).await?;

    let checklist_rows = sqlx::query_as::<_, ChecklistItemRow>(
        "SELECT ci.id, ci.label, ci.description, ci.is_required, ci.completed_at,
                ci.completed_by, u.full_name AS completed_by_name,
                u.email AS completed_by_email, ci.note, ci.sort_order
         FROM intake_case_checklist_items ci
         LEFT JOIN users u ON u.id = ci.completed_by
         WHERE ci.organisation_id = $1 AND ci.case_id = $2 AND ci.stage_id = $3
         ORDER BY ci.sort_order, ci.id",
    )
    .bind(organisation_id)
    .bind(row.id)
    .bind(row.stage_id)
    .fetch_all(db)
    .await?;
    let checklist = checklist_rows
        .into_iter()
        .map(|row| {
            let completed_by = match (
                row.completed_by,
                row.completed_by_name.clone(),
                row.completed_by_email.clone(),
            ) {
                (Some(id), Some(full_name), Some(email)) => Some(UserRefDto {
                    id,
                    full_name,
                    email,
                }),
                _ => None,
            };
            CaseChecklistItemDto {
                id: row.id,
                label: row.label,
                description: row.description,
                is_required: row.is_required,
                completed_at: row.completed_at,
                completed_by,
                note: row.note,
                sort_order: row.sort_order,
            }
        })
        .collect();

    let waitlist_rows = sqlx::query_as::<_, WaitlistRow>(
        "SELECT w.id, w.service_id, sv.name AS service_name, w.division_id,
                d.name AS division_name, w.priority, w.requested_start, w.status,
                w.offered_at, w.offer_expires_at, w.notes
         FROM intake_waitlist_entries w
         LEFT JOIN services sv
           ON sv.organisation_id = w.organisation_id AND sv.id = w.service_id
         LEFT JOIN divisions d
           ON d.organisation_id = w.organisation_id AND d.id = w.division_id
         WHERE w.organisation_id = $1 AND w.case_id = $2
         ORDER BY w.priority DESC, w.created_at, w.id",
    )
    .bind(organisation_id)
    .bind(row.id)
    .fetch_all(db)
    .await?;

    let action_policy = ACTION_KEYS
        .iter()
        .map(|key| {
            let allowed = stage.actions.get(*key).copied().unwrap_or(false);
            (
                (*key).to_owned(),
                ActionPolicyDto {
                    allowed,
                    reason: (!allowed).then(|| {
                        format!("This action is not available at the {} stage.", stage.name)
                    }),
                },
            )
        })
        .collect();

    let allowed_transitions = if stage.is_terminal {
        Vec::new()
    } else {
        flow.stages
            .iter()
            .filter(|candidate| candidate.id != stage.id)
            .map(stage_to_ref)
            .collect()
    };

    Ok(CaseDetailDto {
        case: detail_case(row, data),
        contact: contact_summary(row),
        flow,
        stage: stage.clone(),
        visible_fields: stage.visible_fields.clone(),
        required_fields: stage.required_fields.clone(),
        checklist,
        allowed_transitions,
        action_policy,
        waitlist: waitlist_rows
            .into_iter()
            .map(|row| WaitlistEntryDto {
                id: row.id,
                service_id: row.service_id,
                service_name: row.service_name,
                division_id: row.division_id,
                division_name: row.division_name,
                priority: row.priority,
                requested_start: row.requested_start,
                status: row.status,
                offered_at: row.offered_at,
                offer_expires_at: row.offer_expires_at,
                notes: row.notes,
            })
            .collect(),
    })
}

#[utoipa::path(
    get,
    path = "/api/v1/intake/cases/{id}",
    tag = "intake",
    security(("bearer_auth" = [])),
    params(("id" = Uuid, Path, description = "Intake case id")),
    responses(
        (status = 200, description = "Case detail", body = CaseDetailDto),
        (status = 401, description = "Missing or invalid access token", body = ErrorBody),
        (status = 403, description = "Missing intake.read capability", body = ErrorBody),
        (status = 404, description = "Case not found", body = ErrorBody)
    )
)]
pub async fn get_case(
    State(state): State<AppState>,
    auth: AuthContext,
    Path(id): Path<Uuid>,
) -> Result<Json<CaseDetailDto>, AppError> {
    auth.require(Capability::IntakeRead)?;
    let row = load_case_row(&state.db, auth.organisation_id, id).await?;
    Ok(Json(
        load_case_detail(&state.db, auth.organisation_id, &row).await?,
    ))
}

// ---------------------------------------------------------------------------
// Board and list
// ---------------------------------------------------------------------------

#[utoipa::path(
    get,
    path = "/api/v1/intake/board",
    tag = "intake",
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Board columns and cards", body = BoardDto),
        (status = 401, description = "Missing or invalid access token", body = ErrorBody),
        (status = 403, description = "Missing intake.read capability", body = ErrorBody),
        (status = 404, description = "Flow or division not found", body = ErrorBody),
        (status = 422, description = "Invalid filter", body = ErrorBody)
    )
)]
pub async fn board(
    State(state): State<AppState>,
    auth: AuthContext,
    Query(params): Query<BoardParams>,
) -> Result<Json<BoardDto>, AppError> {
    auth.require(Capability::IntakeRead)?;
    let flows = load_flows(&state.db, auth.organisation_id, true).await?;
    let flow = flows
        .into_iter()
        .find(|flow| flow.id == params.flow_id)
        .ok_or_else(|| AppError::not_found("intake flow not found"))?;
    let filter = build_filter(
        &state,
        &auth,
        params.flow_id,
        None,
        params.division_id,
        params.include_descendants,
        params.assignee_user_id.as_deref(),
        params.lifecycle_status.as_deref(),
        params.search.as_deref(),
        params.include_closed.unwrap_or(false),
    )
    .await?;
    let limit = params.limit_per_stage.unwrap_or(30).clamp(1, 100);

    let mut columns = Vec::with_capacity(flow.stages.len());
    for stage in &flow.stages {
        let mut stage_filter = filter.clone();
        stage_filter.stage_id = Some(stage.id);
        let count = count_cases(&state.db, &stage_filter, None).await?;
        let rows = load_case_rows(&state.db, &stage_filter, limit, None).await?;
        let next_cursor = (rows.len() as u32 == limit)
            .then(|| rows.last().map(|row| CaseCursor::from_row(row).encode()))
            .flatten();
        columns.push(BoardColumnDto {
            stage: stage_to_ref(stage),
            count,
            cards: rows.iter().map(summary).collect(),
            next_cursor,
        });
    }

    Ok(Json(BoardDto {
        flow,
        columns,
        generated_at: Utc::now(),
    }))
}

#[utoipa::path(
    get,
    path = "/api/v1/intake/cases",
    tag = "intake",
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Paged intake cases", body = CaseListDto),
        (status = 401, description = "Missing or invalid access token", body = ErrorBody),
        (status = 403, description = "Missing intake.read capability", body = ErrorBody),
        (status = 404, description = "Division not found", body = ErrorBody),
        (status = 422, description = "Invalid filter or cursor", body = ErrorBody)
    )
)]
pub async fn list_cases(
    State(state): State<AppState>,
    auth: AuthContext,
    Query(params): Query<CaseListParams>,
) -> Result<Json<CaseListDto>, AppError> {
    auth.require(Capability::IntakeRead)?;
    let filter = build_filter(
        &state,
        &auth,
        params.flow_id,
        params.stage_id,
        params.division_id,
        params.include_descendants,
        params.assignee_user_id.as_deref(),
        params.lifecycle_status.as_deref(),
        params.search.as_deref(),
        params.include_closed.unwrap_or(false),
    )
    .await?;
    let limit = params.limit.unwrap_or(50).clamp(1, 200);
    let cursor = params
        .cursor
        .as_deref()
        .map(CaseCursor::decode)
        .transpose()?;
    let rows = load_case_rows(&state.db, &filter, limit, cursor.as_ref()).await?;
    let next_cursor = (rows.len() as u32 == limit)
        .then(|| rows.last().map(|row| CaseCursor::from_row(row).encode()))
        .flatten();
    Ok(Json(CaseListDto {
        items: rows.iter().map(summary).collect(),
        next_cursor,
    }))
}

#[allow(clippy::too_many_arguments)]
async fn build_filter(
    state: &AppState,
    auth: &AuthContext,
    flow_id: Uuid,
    stage_id: Option<Uuid>,
    division_id: Option<Uuid>,
    include_descendants: Option<bool>,
    assignee: Option<&str>,
    lifecycle: Option<&str>,
    search: Option<&str>,
    include_closed: bool,
) -> Result<CaseFilter, AppError> {
    let division_ids = match division_id {
        Some(division_id) => Some(
            descendant_divisions(
                &state.db,
                auth.organisation_id,
                division_id,
                include_descendants.unwrap_or(true),
            )
            .await?
            .into_iter()
            .collect(),
        ),
        None => None,
    };

    let assignee = match assignee.map(str::trim).filter(|value| !value.is_empty()) {
        None => AssigneeFilter::Any,
        Some("me") => AssigneeFilter::Me(auth.user_id),
        Some("unassigned") => AssigneeFilter::Unassigned,
        Some(value) => AssigneeFilter::User(Uuid::parse_str(value).map_err(|_| {
            AppError::validation("assignee_user_id must be a uuid, me or unassigned")
        })?),
    };

    let lifecycle_statuses = lifecycle
        .map(|value| {
            value
                .split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .filter(|values| !values.is_empty());
    if let Some(values) = &lifecycle_statuses {
        for value in values {
            if !intake::is_lifecycle_status(value) {
                return Err(AppError::validation(format!(
                    "unknown lifecycle status: {value}"
                )));
            }
        }
    }

    Ok(CaseFilter {
        organisation_id: auth.organisation_id,
        flow_id,
        stage_id,
        division_ids,
        assignee,
        lifecycle_statuses,
        search: search
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned),
        include_closed,
    })
}

// ---------------------------------------------------------------------------
// Contact matching and case creation
// ---------------------------------------------------------------------------

#[utoipa::path(
    get,
    path = "/api/v1/intake/contact-matches",
    tag = "intake",
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Candidate contacts", body = ContactMatchResponse),
        (status = 401, description = "Missing or invalid access token", body = ErrorBody),
        (status = 403, description = "Missing intake.read capability", body = ErrorBody),
        (status = 422, description = "No search terms supplied", body = ErrorBody)
    )
)]
pub async fn contact_matches(
    State(state): State<AppState>,
    auth: AuthContext,
    Query(params): Query<ContactMatchParams>,
) -> Result<Json<ContactMatchResponse>, AppError> {
    auth.require(Capability::IntakeRead)?;
    let email = params.email.as_deref().map(str::trim).unwrap_or("");
    let phone = params.phone.as_deref().map(str::trim).unwrap_or("");
    let name = params.name.as_deref().map(str::trim).unwrap_or("");
    if email.is_empty() && phone.is_empty() && name.is_empty() {
        return Err(AppError::validation(
            "supply at least one of email, phone or name",
        ));
    }
    let limit = params.limit.unwrap_or(10).clamp(1, 25);
    let candidates = find_contact_matches(
        &state.db,
        auth.organisation_id,
        email,
        phone,
        name,
        params.date_of_birth,
        i64::from(limit),
    )
    .await?;
    Ok(Json(ContactMatchResponse { candidates }))
}

async fn find_contact_matches(
    db: &PgPool,
    organisation_id: Uuid,
    email: &str,
    phone: &str,
    name: &str,
    date_of_birth: Option<NaiveDate>,
    limit: i64,
) -> Result<Vec<ContactMatchDto>, AppError> {
    let email_norm = email.trim().to_ascii_lowercase();
    let phone_norm: String = phone.chars().filter(char::is_ascii_digit).collect();
    let name_norm = name.trim().to_ascii_lowercase();

    let rows = sqlx::query_as::<_, ContactMatchRow>(
        "SELECT id, display_name, email, phone, date_of_birth, lifecycle_status
         FROM contacts
         WHERE organisation_id = $1
           AND (
               ($2 <> '' AND lower(coalesce(email, '')) = $2)
               OR ($3 <> '' AND regexp_replace(coalesce(phone, ''), '[^0-9]', '', 'g') = $3)
               OR ($4 <> '' AND (
                   lower(display_name) LIKE '%' || $4 || '%'
                   OR lower(coalesce(given_name, '') || ' ' || coalesce(family_name, ''))
                      LIKE '%' || $4 || '%'
               ))
           )
           AND ($5::date IS NULL OR date_of_birth = $5)
         ORDER BY display_name, id
         LIMIT $6",
    )
    .bind(organisation_id)
    .bind(&email_norm)
    .bind(&phone_norm)
    .bind(&name_norm)
    .bind(date_of_birth)
    .bind(limit)
    .fetch_all(db)
    .await?;

    if rows.is_empty() {
        return Ok(Vec::new());
    }

    let ids: Vec<Uuid> = rows.iter().map(|row| row.id).collect();
    let open_cases = sqlx::query_as::<_, OpenCaseRow>(
        "SELECT c.contact_id, c.id AS case_id, c.flow_id, f.name AS flow_name,
                s.name AS stage_name
         FROM intake_cases c
         JOIN intake_flows f
           ON f.organisation_id = c.organisation_id AND f.id = c.flow_id
         JOIN intake_stages s
           ON s.organisation_id = c.organisation_id AND s.id = c.stage_id
         WHERE c.organisation_id = $1 AND c.contact_id = ANY($2)
           AND c.closed_at IS NULL AND c.onboarded_at IS NULL
         ORDER BY c.updated_at DESC, c.id",
    )
    .bind(organisation_id)
    .bind(&ids)
    .fetch_all(db)
    .await?;
    let mut open_by_contact = HashMap::new();
    for row in open_cases {
        open_by_contact
            .entry(row.contact_id)
            .or_insert(OpenCaseRefDto {
                id: row.case_id,
                flow_id: row.flow_id,
                flow_name: row.flow_name,
                stage_name: row.stage_name,
            });
    }

    Ok(rows
        .into_iter()
        .map(|row| {
            let match_kind = if !email_norm.is_empty()
                && row
                    .email
                    .as_deref()
                    .is_some_and(|value| value.trim().eq_ignore_ascii_case(&email_norm))
            {
                "email"
            } else if !phone_norm.is_empty()
                && row.phone.as_deref().is_some_and(|value| {
                    value
                        .chars()
                        .filter(char::is_ascii_digit)
                        .collect::<String>()
                        == phone_norm
                })
            {
                "phone"
            } else if date_of_birth.is_some() {
                "name_dob"
            } else {
                "name"
            };
            ContactMatchDto {
                id: row.id,
                display_name: row.display_name,
                email: row.email,
                phone: row.phone,
                date_of_birth: row.date_of_birth,
                lifecycle_status: row.lifecycle_status,
                match_kind: match_kind.to_owned(),
                open_case: open_by_contact.remove(&row.id),
            }
        })
        .collect())
}

#[utoipa::path(
    post,
    path = "/api/v1/intake/cases",
    tag = "intake",
    security(("bearer_auth" = [])),
    request_body = CaseCreate,
    responses(
        (status = 201, description = "Case created", body = CaseCreateResponse),
        (status = 200, description = "Existing case returned for an idempotent retry", body = CaseCreateResponse),
        (status = 401, description = "Missing or invalid access token", body = ErrorBody),
        (status = 403, description = "Missing intake.write or contacts.write capability", body = ErrorBody),
        (status = 404, description = "Flow, contact or organisation not found", body = ErrorBody),
        (status = 409, description = "Duplicate contact or existing open case", body = ErrorBody),
        (status = 422, description = "Validation failed", body = ErrorBody)
    )
)]
pub async fn create_case(
    State(state): State<AppState>,
    auth: AuthContext,
    Json(payload): Json<CaseCreate>,
) -> Result<(StatusCode, Json<CaseCreateResponse>), AppError> {
    auth.require(Capability::IntakeWrite)?;
    if let Some(request_id) = payload.client_request_id.as_deref() {
        if let Some(existing) =
            find_case_by_client_request(&state.db, auth.organisation_id, request_id).await?
        {
            let row = load_case_row(&state.db, auth.organisation_id, existing).await?;
            let contact = contact_summary(&row);
            return Ok((
                StatusCode::OK,
                Json(CaseCreateResponse {
                    case: load_case_detail(&state.db, auth.organisation_id, &row).await?,
                    contact,
                }),
            ));
        }
    }
    if let Some(legacy_id) = payload.legacy_draft_id.as_deref() {
        if let Some(existing) =
            find_case_by_legacy_draft(&state.db, auth.organisation_id, legacy_id).await?
        {
            let row = load_case_row(&state.db, auth.organisation_id, existing).await?;
            let contact = contact_summary(&row);
            return Ok((
                StatusCode::OK,
                Json(CaseCreateResponse {
                    case: load_case_detail(&state.db, auth.organisation_id, &row).await?,
                    contact,
                }),
            ));
        }
    }

    let flows = load_flows(&state.db, auth.organisation_id, true).await?;
    let flow = flows
        .into_iter()
        .find(|flow| flow.id == payload.flow_id)
        .ok_or_else(|| AppError::not_found("intake flow not found"))?;
    if !flow.is_active {
        return Err(AppError::validation("that intake flow is not active"));
    }
    let first_stage = flow
        .stages
        .first()
        .cloned()
        .ok_or_else(|| AppError::validation("the intake flow has no stages"))?;
    if first_stage.kind != "enquiry" {
        return Err(AppError::validation(
            "the first intake stage must be enquiry",
        ));
    }

    let source = payload.source.as_deref().unwrap_or("enquiry");
    if !matches!(source, "enquiry" | "referral" | "internal" | "waitlist") {
        return Err(AppError::validation("unknown intake source"));
    }
    if payload.referrer_contact_id.is_some() && payload.referrer_organisation_id.is_some() {
        return Err(AppError::validation(
            "supply at most one referrer contact or organisation",
        ));
    }
    if let Some(division_id) = payload.division_id {
        ensure_division(&state.db, auth.organisation_id, division_id).await?;
    }

    let definitions = case_service::custom_definitions(&state.db, auth.organisation_id).await?;

    if flow.subject_kind == "organisation" {
        if payload.contact_id.is_some() || payload.contact.is_some() {
            return Err(AppError::validation(
                "organisation flows cannot link a person contact",
            ));
        }
    } else if payload.external_organisation_id.is_some() {
        return Err(AppError::validation(
            "person flows cannot link an external organisation",
        ));
    }

    let (existing_contact, new_contact) = match (payload.contact_id, flow.subject_kind.as_str()) {
        (Some(contact_id), _) => (
            Some(load_contact_row(&state.db, auth.organisation_id, contact_id).await?),
            None,
        ),
        (None, "organisation") => (None, None),
        (None, _) => {
            let input = payload.contact.as_ref().cloned().ok_or_else(|| {
                AppError::validation("a contact payload is required for a person intake")
            })?;
            if !payload.allow_duplicate_contact {
                let candidates = find_contact_matches(
                    &state.db,
                    auth.organisation_id,
                    input.email.as_deref().unwrap_or(""),
                    input.phone.as_deref().unwrap_or(""),
                    &display_name_from_input(&input),
                    input.date_of_birth,
                    10,
                )
                .await?;
                if !candidates.is_empty() {
                    return Err(AppError::conflict_with_details(
                        "An existing contact may match this person.",
                        json!({
                            "reason": "contact_match",
                            "candidates": candidates,
                        }),
                    ));
                }
            } else if payload
                .duplicate_reason
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .is_none()
            {
                return Err(AppError::validation(
                    "duplicate_reason is required when forcing a duplicate contact",
                ));
            }
            (None, Some(input))
        }
    };

    let external_organisation = if flow.subject_kind == "organisation" {
        let external_id = payload.external_organisation_id.ok_or_else(|| {
            AppError::validation("external_organisation_id is required for an organisation intake")
        })?;
        Some(load_external_organisation(&state.db, auth.organisation_id, external_id).await?)
    } else {
        None
    };

    let open_case = find_open_case(
        &state.db,
        auth.organisation_id,
        flow.id,
        existing_contact.as_ref().map(|contact| contact.id),
        external_organisation
            .as_ref()
            .map(|organisation| organisation.id),
    )
    .await?;
    if let Some(open) = open_case {
        return Err(AppError::conflict_with_details(
            "This person already has an open intake for this flow.",
            json!({
                "reason": "open_case_exists",
                "case_id": open.0,
                "flow_id": flow.id,
                "stage_name": open.1,
            }),
        ));
    }

    let mut tx = state.db.begin().await?;
    let contact = if let Some(input) = new_contact {
        Some(insert_contact(&mut tx, &auth, &input).await?)
    } else {
        existing_contact
    };
    if let Some(contact) = &contact {
        if matches!(
            contact.lifecycle_status.as_str(),
            "intake_cancelled" | "offboarded"
        ) {
            case_service::set_contact_status(
                &mut tx,
                auth.organisation_id,
                contact.id,
                "enquiry",
                Some("New intake case"),
                auth.user_id,
            )
            .await?;
        }
    }

    let display_name = payload
        .display_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .or_else(|| contact.as_ref().map(|contact| contact.display_name.clone()))
        .or_else(|| {
            external_organisation
                .as_ref()
                .map(|organisation| organisation.name.clone())
        })
        .ok_or_else(|| AppError::validation("a display name is required"))?;
    if display_name.chars().count() > 200 {
        return Err(AppError::validation(
            "display_name must be 200 characters or fewer",
        ));
    }

    let base = if let Some(contact) = &contact {
        case_service::contact_identity_data(
            contact.given_name.as_deref(),
            contact.family_name.as_deref(),
            contact.email.as_deref(),
            contact.phone.as_deref(),
            contact.date_of_birth,
        )
    } else {
        let mut map = Map::new();
        if let Some(organisation) = &external_organisation {
            map.insert(
                "organisation_name".to_owned(),
                Value::String(organisation.name.clone()),
            );
        }
        Value::Object(map)
    };
    let extra_data = payload
        .data
        .as_object()
        .ok_or_else(|| AppError::validation("data must be a JSON object"))?;
    let merged = case_service::merge_case_maps(base, extra_data);
    let (builtin, custom) = case_service::split_case_data(&definitions, &merged)?;

    let case_id = Uuid::now_v7();
    let priority = payload.priority.unwrap_or(0);
    sqlx::query(
        "INSERT INTO intake_cases
            (id, organisation_id, flow_id, stage_id, contact_id,
             external_organisation_id, subject_kind, display_name, data, source,
             referrer_contact_id, referrer_organisation_id, referral_reference,
             referral_received_at, division_id, priority, created_by,
             client_request_id, legacy_draft_id)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10,
                 $11, $12, $13, $14, $15, $16, $17, $18, $19)",
    )
    .bind(case_id)
    .bind(auth.organisation_id)
    .bind(flow.id)
    .bind(first_stage.id)
    .bind(contact.as_ref().map(|contact| contact.id))
    .bind(
        external_organisation
            .as_ref()
            .map(|organisation| organisation.id),
    )
    .bind(&flow.subject_kind)
    .bind(&display_name)
    .bind(&builtin)
    .bind(source)
    .bind(payload.referrer_contact_id)
    .bind(payload.referrer_organisation_id)
    .bind(trim_optional(payload.referral_reference.as_deref()))
    .bind(payload.referral_received_at)
    .bind(payload.division_id)
    .bind(priority)
    .bind(auth.user_id)
    .bind(trim_optional(payload.client_request_id.as_deref()))
    .bind(trim_optional(payload.legacy_draft_id.as_deref()))
    .execute(&mut *tx)
    .await?;

    case_service::write_case_data(&mut tx, auth.organisation_id, case_id, &builtin, &custom)
        .await?;
    case_service::materialise_checklist(&mut tx, auth.organisation_id, case_id, first_stage.id)
        .await?;

    let event_id = insert_event(
        &mut tx,
        auth.organisation_id,
        case_id,
        "created",
        auth.user_id,
        None,
        Some(first_stage.id),
        json!({ "legacy_draft_id": payload.legacy_draft_id }),
    )
    .await?;

    let assignee = case_service::resolve_assignment(
        &mut tx,
        auth.organisation_id,
        case_id,
        &display_name,
        first_stage.id,
        &first_stage.name,
        auth.user_id,
        event_id,
    )
    .await?;
    if let Some(assignee) = assignee {
        sqlx::query(
            "UPDATE intake_cases SET assignee_user_id = $1
             WHERE organisation_id = $2 AND id = $3",
        )
        .bind(assignee)
        .bind(auth.organisation_id)
        .bind(case_id)
        .execute(&mut *tx)
        .await?;
    }

    write_audit(
        &mut tx,
        auth.organisation_id,
        auth.user_id,
        "intake.case.created",
        case_id,
        json!({
            "flow_id": flow.id,
            "stage_id": first_stage.id,
            "contact_id": contact.as_ref().map(|contact| contact.id),
        }),
    )
    .await?;
    tx.commit().await?;

    let row = load_case_row(&state.db, auth.organisation_id, case_id).await?;
    let contact = contact_summary(&row);
    Ok((
        StatusCode::CREATED,
        Json(CaseCreateResponse {
            case: load_case_detail(&state.db, auth.organisation_id, &row).await?,
            contact,
        }),
    ))
}

// ---------------------------------------------------------------------------
// Writes
// ---------------------------------------------------------------------------

#[utoipa::path(
    patch,
    path = "/api/v1/intake/cases/{id}",
    tag = "intake",
    security(("bearer_auth" = [])),
    request_body = CaseUpdate,
    params(("id" = Uuid, Path, description = "Intake case id")),
    responses(
        (status = 200, description = "Case updated", body = CaseDetailDto),
        (status = 401, description = "Missing or invalid access token", body = ErrorBody),
        (status = 403, description = "Missing capability", body = ErrorBody),
        (status = 404, description = "Case not found", body = ErrorBody),
        (status = 409, description = "Version conflict", body = ErrorBody),
        (status = 422, description = "Validation failed", body = ErrorBody)
    )
)]
pub async fn update_case(
    State(state): State<AppState>,
    auth: AuthContext,
    Path(id): Path<Uuid>,
    Json(payload): Json<CaseUpdate>,
) -> Result<Json<CaseDetailDto>, AppError> {
    auth.require(Capability::IntakeWrite)?;
    let row = load_case_row(&state.db, auth.organisation_id, id).await?;
    if payload.version != row.version {
        return Err(version_conflict(row.version, row.id));
    }
    let flows = load_flows(&state.db, auth.organisation_id, true).await?;
    let flow = flows
        .into_iter()
        .find(|flow| flow.id == row.flow_id)
        .ok_or_else(|| AppError::not_found("intake flow not found"))?;
    let stage = flow
        .stages
        .iter()
        .find(|stage| stage.id == row.stage_id)
        .ok_or_else(|| AppError::not_found("intake stage not found"))?;

    if let Some(division_id) = payload.division_id {
        ensure_division(&state.db, auth.organisation_id, division_id).await?;
    }
    if payload.assignee_user_id.is_some() || payload.unassign {
        auth.require(Capability::IntakeAssign)?;
        if let Some(user_id) = payload.assignee_user_id {
            ensure_active_member(&state.db, auth.organisation_id, user_id).await?;
        }
    }

    let definitions = case_service::custom_definitions(&state.db, auth.organisation_id).await?;
    let mut current =
        case_service::merge_case_data(&state.db, auth.organisation_id, id, row.data.0.clone())
            .await?;
    if let Some(patch) = &payload.data {
        let patch = patch
            .as_object()
            .ok_or_else(|| AppError::validation("data must be a JSON object"))?;
        if !payload.force {
            let visible: HashSet<&str> = stage.visible_fields.iter().map(String::as_str).collect();
            for key in patch.keys() {
                if !visible.contains(key.as_str()) {
                    return Err(AppError::validation(format!(
                        "field {key} is not visible at this stage"
                    )));
                }
            }
        } else {
            auth.require(Capability::IntakeConfigure)?;
        }
        let mut object = current
            .as_object_mut()
            .cloned()
            .ok_or_else(|| AppError::validation("case data must be an object"))?;
        for (key, value) in patch {
            if value.is_null() {
                object.remove(key);
            } else {
                object.insert(key.clone(), value.clone());
            }
        }
        current = Value::Object(object);
    }
    let merged_map = current.as_object().cloned().unwrap_or_default();
    let (builtin, custom) = case_service::split_case_data(&definitions, &merged_map)?;

    let new_display_name = match payload.display_name.as_deref() {
        Some(name) => {
            let name = name.trim();
            if name.is_empty() || name.chars().count() > 200 {
                return Err(AppError::validation(
                    "display_name must be between 1 and 200 characters",
                ));
            }
            Some(name.to_owned())
        }
        None => None,
    };

    let mut tx = state.db.begin().await?;
    case_service::write_case_data(&mut tx, auth.organisation_id, id, &builtin, &custom).await?;
    sqlx::query(
        "UPDATE intake_cases SET
            display_name = COALESCE($1, display_name),
            division_id = COALESCE($2, division_id),
            priority = COALESCE($3, priority),
            version = version + 1
         WHERE organisation_id = $4 AND id = $5",
    )
    .bind(new_display_name)
    .bind(payload.division_id)
    .bind(payload.priority)
    .bind(auth.organisation_id)
    .bind(id)
    .execute(&mut *tx)
    .await?;

    if payload.assignee_user_id.is_some() || payload.unassign {
        let event_id = insert_event(
            &mut tx,
            auth.organisation_id,
            id,
            "assigned",
            auth.user_id,
            None,
            None,
            json!({ "unassign": payload.unassign }),
        )
        .await?;
        let assignee = if payload.unassign {
            None
        } else {
            payload.assignee_user_id
        };
        sqlx::query(
            "UPDATE intake_cases SET assignee_user_id = $1
             WHERE organisation_id = $2 AND id = $3",
        )
        .bind(assignee)
        .bind(auth.organisation_id)
        .bind(id)
        .execute(&mut *tx)
        .await?;
        if let Some(assignee) = assignee {
            if assignee != auth.user_id {
                case_service::notify_user(
                    &mut tx,
                    auth.organisation_id,
                    assignee,
                    id,
                    &format!("case:{id}:assigned:event:{event_id}"),
                    &row.display_name,
                    &row.stage_name,
                )
                .await?;
            }
        }
    }

    insert_event(
        &mut tx,
        auth.organisation_id,
        id,
        "updated",
        auth.user_id,
        None,
        None,
        json!({
            "fields": payload
                .data
                .as_ref()
                .and_then(Value::as_object)
                .map(|data| data.keys().cloned().collect::<Vec<_>>())
        }),
    )
    .await?;
    write_audit(
        &mut tx,
        auth.organisation_id,
        auth.user_id,
        "intake.case.updated",
        id,
        json!({ "force": payload.force }),
    )
    .await?;
    tx.commit().await?;

    let row = load_case_row(&state.db, auth.organisation_id, id).await?;
    Ok(Json(
        load_case_detail(&state.db, auth.organisation_id, &row).await?,
    ))
}

#[utoipa::path(
    post,
    path = "/api/v1/intake/cases/{id}/transition",
    tag = "intake",
    security(("bearer_auth" = [])),
    request_body = TransitionRequest,
    params(("id" = Uuid, Path, description = "Intake case id")),
    responses(
        (status = 200, description = "Case transitioned", body = CaseDetailDto),
        (status = 401, description = "Missing or invalid access token", body = ErrorBody),
        (status = 403, description = "Missing capability", body = ErrorBody),
        (status = 404, description = "Case or stage not found", body = ErrorBody),
        (status = 409, description = "Version conflict", body = ErrorBody),
        (status = 422, description = "Transition blocked", body = ErrorBody)
    )
)]
pub async fn transition_case(
    State(state): State<AppState>,
    auth: AuthContext,
    Path(id): Path<Uuid>,
    Json(payload): Json<TransitionRequest>,
) -> Result<Json<CaseDetailDto>, AppError> {
    auth.require(Capability::IntakeWrite)?;
    let row = load_case_row(&state.db, auth.organisation_id, id).await?;
    if payload.version != row.version {
        return Err(version_conflict(row.version, row.id));
    }
    if row.stage_is_terminal {
        return Err(invalid_transition(
            "a terminal case cannot transition; reopen it instead",
        ));
    }
    if payload.to_stage_id == row.stage_id {
        return Err(invalid_transition("the case is already in that stage"));
    }
    let flows = load_flows(&state.db, auth.organisation_id, true).await?;
    let flow = flows
        .into_iter()
        .find(|flow| flow.id == row.flow_id)
        .ok_or_else(|| AppError::not_found("intake flow not found"))?;
    let target = flow
        .stages
        .iter()
        .find(|stage| stage.id == payload.to_stage_id)
        .cloned()
        .ok_or_else(|| AppError::not_found("target intake stage not found"))?;
    let current = flow
        .stages
        .iter()
        .find(|stage| stage.id == row.stage_id)
        .cloned()
        .ok_or_else(|| AppError::not_found("intake stage not found"))?;

    if target.kind == "cancelled"
        && payload
            .reason
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .is_none()
    {
        return Err(AppError::validation("a cancellation reason is required"));
    }

    let merged =
        case_service::merge_case_data(&state.db, auth.organisation_id, id, row.data.0.clone())
            .await?;
    let values = case_service::values_by_key(&merged);
    let missing_fields: Vec<String> = current
        .required_fields
        .iter()
        .filter(|key| {
            values
                .get(*key)
                .is_none_or(|value| !case_service::value_present(value))
        })
        .cloned()
        .collect();

    let mut incomplete_checklist = Vec::new();
    if current.checklist_gate {
        let mut tx = state.db.begin().await?;
        incomplete_checklist = case_service::incomplete_required_checklist(
            &mut tx,
            auth.organisation_id,
            id,
            current.id,
        )
        .await?
        .into_iter()
        .map(|(id, label)| json!({ "id": id, "label": label, "is_required": true }))
        .collect();
        tx.rollback().await?;
    }

    let override_validation = payload.override_validation;
    if override_validation {
        auth.require(Capability::IntakeConfigure)?;
        if payload
            .override_reason
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .is_none()
        {
            return Err(AppError::validation(
                "override_reason is required when overriding validation",
            ));
        }
    } else if !missing_fields.is_empty() || !incomplete_checklist.is_empty() {
        return Err(AppError::validation_with_details(
            "Complete the required checks before moving on.",
            json!({
                "reason": "intake_transition_blocked",
                "missing_fields": missing_fields,
                "incomplete_checklist": incomplete_checklist,
            }),
        ));
    }

    if target.kind == "onboarded" && row.contact_id.is_some() && row.subject_kind != "other_contact"
    {
        auth.require(Capability::RelationshipsWrite)?;
    }

    let mut tx = state.db.begin().await?;
    let now = Utc::now();
    let due_at = target
        .sla_hours
        .map(|hours| now + Duration::hours(i64::from(hours)));
    sqlx::query(
        "UPDATE intake_cases SET
            stage_id = $1, stage_entered_at = $2, stage_due_at = $3,
            version = version + 1
         WHERE organisation_id = $4 AND id = $5",
    )
    .bind(target.id)
    .bind(now)
    .bind(due_at)
    .bind(auth.organisation_id)
    .bind(id)
    .execute(&mut *tx)
    .await?;

    case_service::materialise_checklist(&mut tx, auth.organisation_id, id, target.id).await?;
    let event_id = insert_event(
        &mut tx,
        auth.organisation_id,
        id,
        "transitioned",
        auth.user_id,
        Some(current.id),
        Some(target.id),
        json!({
            "reason": payload.reason,
            "override": override_validation,
            "override_reason": payload.override_reason,
        }),
    )
    .await?;
    let assignee = case_service::resolve_assignment(
        &mut tx,
        auth.organisation_id,
        id,
        &row.display_name,
        target.id,
        &target.name,
        auth.user_id,
        event_id,
    )
    .await?;
    if let Some(assignee) = assignee {
        sqlx::query(
            "UPDATE intake_cases SET assignee_user_id = $1
             WHERE organisation_id = $2 AND id = $3",
        )
        .bind(assignee)
        .bind(auth.organisation_id)
        .bind(id)
        .execute(&mut *tx)
        .await?;
    }

    if let Some(contact_id) = row.contact_id {
        if let Some(nudge) = target.on_enter_contact_status.as_deref() {
            case_service::apply_lifecycle_nudge(
                &mut tx,
                auth.organisation_id,
                contact_id,
                nudge,
                auth.user_id,
                &target.name,
            )
            .await?;
        }
    }

    if target.kind == "onboarded" {
        sqlx::query(
            "UPDATE intake_cases SET onboarded_at = $1
             WHERE organisation_id = $2 AND id = $3",
        )
        .bind(now)
        .bind(auth.organisation_id)
        .bind(id)
        .execute(&mut *tx)
        .await?;
        if let Some(contact_id) = row.contact_id {
            case_service::link_role(&mut tx, auth.organisation_id, contact_id, &row.subject_kind)
                .await?;
            case_service::copy_case_to_contact(&mut tx, auth.organisation_id, id, contact_id)
                .await?;
        }
        insert_event(
            &mut tx,
            auth.organisation_id,
            id,
            "onboarded",
            auth.user_id,
            None,
            Some(target.id),
            json!({}),
        )
        .await?;
    }

    if target.kind == "cancelled" {
        sqlx::query(
            "UPDATE intake_cases SET closed_at = $1, cancellation_reason = $2
             WHERE organisation_id = $3 AND id = $4",
        )
        .bind(now)
        .bind(trim_optional(payload.reason.as_deref()))
        .bind(auth.organisation_id)
        .bind(id)
        .execute(&mut *tx)
        .await?;
        if let Some(contact_id) = row.contact_id {
            case_service::apply_lifecycle_nudge(
                &mut tx,
                auth.organisation_id,
                contact_id,
                "intake_cancelled",
                auth.user_id,
                &target.name,
            )
            .await?;
        }
        insert_event(
            &mut tx,
            auth.organisation_id,
            id,
            "cancelled",
            auth.user_id,
            None,
            Some(target.id),
            json!({ "reason": payload.reason }),
        )
        .await?;
    }

    write_audit(
        &mut tx,
        auth.organisation_id,
        auth.user_id,
        "intake.case.transitioned",
        id,
        json!({
            "from": current.id,
            "to": target.id,
            "override": override_validation,
        }),
    )
    .await?;
    tx.commit().await?;

    let row = load_case_row(&state.db, auth.organisation_id, id).await?;
    Ok(Json(
        load_case_detail(&state.db, auth.organisation_id, &row).await?,
    ))
}

#[utoipa::path(
    post,
    path = "/api/v1/intake/cases/{id}/checklist/{item_id}",
    tag = "intake",
    security(("bearer_auth" = [])),
    request_body = ChecklistToggle,
    params(
        ("id" = Uuid, Path, description = "Intake case id"),
        ("item_id" = Uuid, Path, description = "Checklist item id")
    ),
    responses(
        (status = 200, description = "Checklist item updated", body = ChecklistToggleResponse),
        (status = 401, description = "Missing or invalid access token", body = ErrorBody),
        (status = 403, description = "Missing intake.write capability", body = ErrorBody),
        (status = 404, description = "Case or item not found", body = ErrorBody)
    )
)]
pub async fn toggle_checklist_item(
    State(state): State<AppState>,
    auth: AuthContext,
    Path((case_id, item_id)): Path<(Uuid, Uuid)>,
    Json(payload): Json<ChecklistToggle>,
) -> Result<Json<ChecklistToggleResponse>, AppError> {
    auth.require(Capability::IntakeWrite)?;
    let row = load_case_row(&state.db, auth.organisation_id, case_id).await?;
    let exists = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(
             SELECT 1 FROM intake_case_checklist_items
             WHERE organisation_id = $1 AND id = $2 AND case_id = $3 AND stage_id = $4
         )",
    )
    .bind(auth.organisation_id)
    .bind(item_id)
    .bind(case_id)
    .bind(row.stage_id)
    .fetch_one(&state.db)
    .await?;
    if !exists {
        return Err(AppError::not_found("checklist item not found"));
    }

    let mut tx = state.db.begin().await?;
    if payload.completed {
        sqlx::query(
            "UPDATE intake_case_checklist_items
             SET completed_at = now(), completed_by = $1, note = $2
             WHERE organisation_id = $3 AND id = $4",
        )
        .bind(auth.user_id)
        .bind(trim_optional(payload.note.as_deref()))
        .bind(auth.organisation_id)
        .bind(item_id)
        .execute(&mut *tx)
        .await?;
        insert_event(
            &mut tx,
            auth.organisation_id,
            case_id,
            "checklist_completed",
            auth.user_id,
            None,
            None,
            json!({ "item_id": item_id }),
        )
        .await?;
    } else {
        sqlx::query(
            "UPDATE intake_case_checklist_items
             SET completed_at = NULL, completed_by = NULL, note = $1
             WHERE organisation_id = $2 AND id = $3",
        )
        .bind(trim_optional(payload.note.as_deref()))
        .bind(auth.organisation_id)
        .bind(item_id)
        .execute(&mut *tx)
        .await?;
        insert_event(
            &mut tx,
            auth.organisation_id,
            case_id,
            "checklist_reopened",
            auth.user_id,
            None,
            None,
            json!({ "item_id": item_id }),
        )
        .await?;
    }
    let progress =
        case_service::checklist_progress(&mut tx, auth.organisation_id, case_id, row.stage_id)
            .await?;
    write_audit(
        &mut tx,
        auth.organisation_id,
        auth.user_id,
        "intake.checklist.updated",
        case_id,
        json!({ "item_id": item_id, "completed": payload.completed }),
    )
    .await?;
    tx.commit().await?;

    let item = sqlx::query_as::<_, ChecklistItemRow>(
        "SELECT ci.id, ci.label, ci.description, ci.is_required, ci.completed_at,
                ci.completed_by, u.full_name AS completed_by_name,
                u.email AS completed_by_email, ci.note, ci.sort_order
         FROM intake_case_checklist_items ci
         LEFT JOIN users u ON u.id = ci.completed_by
         WHERE ci.organisation_id = $1 AND ci.id = $2",
    )
    .bind(auth.organisation_id)
    .bind(item_id)
    .fetch_one(&state.db)
    .await?;
    Ok(Json(ChecklistToggleResponse {
        item: checklist_dto(item),
        checklist: ChecklistProgressDto {
            done: progress.0,
            total: progress.1,
            required_done: progress.2,
            required_total: progress.3,
        },
    }))
}

// ---------------------------------------------------------------------------
// Cancel, archive and timeline
// ---------------------------------------------------------------------------

#[utoipa::path(
    post,
    path = "/api/v1/intake/cases/{id}/cancel",
    tag = "intake",
    security(("bearer_auth" = [])),
    request_body = CancelRequest,
    params(("id" = Uuid, Path, description = "Intake case id")),
    responses(
        (status = 200, description = "Case cancelled", body = CaseDetailDto),
        (status = 401, description = "Missing or invalid access token", body = ErrorBody),
        (status = 403, description = "Missing intake.write capability", body = ErrorBody),
        (status = 404, description = "Case not found", body = ErrorBody),
        (status = 409, description = "Version conflict", body = ErrorBody),
        (status = 422, description = "Validation failed", body = ErrorBody)
    )
)]
pub async fn cancel_case(
    State(state): State<AppState>,
    auth: AuthContext,
    Path(id): Path<Uuid>,
    Json(payload): Json<CancelRequest>,
) -> Result<Json<CaseDetailDto>, AppError> {
    auth.require(Capability::IntakeWrite)?;
    let row = load_case_row(&state.db, auth.organisation_id, id).await?;
    if payload.version != row.version {
        return Err(version_conflict(row.version, row.id));
    }
    if row.stage_is_terminal {
        return Err(invalid_transition(
            "a terminal case cannot be cancelled; archive it instead",
        ));
    }
    let reason = payload.reason.trim();
    if reason.is_empty() {
        return Err(AppError::validation("a cancellation reason is required"));
    }

    let mut tx = state.db.begin().await?;
    sqlx::query(
        "UPDATE intake_cases SET closed_at = now(), cancellation_reason = $1,
            version = version + 1
         WHERE organisation_id = $2 AND id = $3",
    )
    .bind(reason)
    .bind(auth.organisation_id)
    .bind(id)
    .execute(&mut *tx)
    .await?;
    insert_event(
        &mut tx,
        auth.organisation_id,
        id,
        "cancelled",
        auth.user_id,
        None,
        Some(row.stage_id),
        json!({ "reason": reason }),
    )
    .await?;
    if let Some(contact_id) = row.contact_id {
        case_service::apply_lifecycle_nudge(
            &mut tx,
            auth.organisation_id,
            contact_id,
            "intake_cancelled",
            auth.user_id,
            &row.stage_name,
        )
        .await?;
    }
    write_audit(
        &mut tx,
        auth.organisation_id,
        auth.user_id,
        "intake.case.cancelled",
        id,
        json!({ "reason": reason }),
    )
    .await?;
    tx.commit().await?;

    let row = load_case_row(&state.db, auth.organisation_id, id).await?;
    Ok(Json(
        load_case_detail(&state.db, auth.organisation_id, &row).await?,
    ))
}

#[utoipa::path(
    post,
    path = "/api/v1/intake/cases/{id}/archive",
    tag = "intake",
    security(("bearer_auth" = [])),
    request_body = ArchiveRequest,
    params(("id" = Uuid, Path, description = "Intake case id")),
    responses(
        (status = 200, description = "Case archived", body = CaseDetailDto),
        (status = 401, description = "Missing or invalid access token", body = ErrorBody),
        (status = 403, description = "Missing intake.write capability", body = ErrorBody),
        (status = 404, description = "Case not found", body = ErrorBody),
        (status = 409, description = "Version conflict", body = ErrorBody)
    )
)]
pub async fn archive_case(
    State(state): State<AppState>,
    auth: AuthContext,
    Path(id): Path<Uuid>,
    Json(payload): Json<ArchiveRequest>,
) -> Result<Json<CaseDetailDto>, AppError> {
    auth.require(Capability::IntakeWrite)?;
    let row = load_case_row(&state.db, auth.organisation_id, id).await?;
    if payload.version != row.version {
        return Err(version_conflict(row.version, row.id));
    }
    let mut tx = state.db.begin().await?;
    sqlx::query(
        "UPDATE intake_cases SET closed_at = now(), version = version + 1
         WHERE organisation_id = $1 AND id = $2",
    )
    .bind(auth.organisation_id)
    .bind(id)
    .execute(&mut *tx)
    .await?;
    insert_event(
        &mut tx,
        auth.organisation_id,
        id,
        "archived",
        auth.user_id,
        None,
        Some(row.stage_id),
        json!({ "reason": payload.reason }),
    )
    .await?;
    write_audit(
        &mut tx,
        auth.organisation_id,
        auth.user_id,
        "intake.case.archived",
        id,
        json!({ "reason": payload.reason }),
    )
    .await?;
    tx.commit().await?;

    let row = load_case_row(&state.db, auth.organisation_id, id).await?;
    Ok(Json(
        load_case_detail(&state.db, auth.organisation_id, &row).await?,
    ))
}

#[utoipa::path(
    get,
    path = "/api/v1/intake/cases/{id}/timeline",
    tag = "intake",
    security(("bearer_auth" = [])),
    params(("id" = Uuid, Path, description = "Intake case id")),
    responses(
        (status = 200, description = "Case timeline", body = TimelineResponse),
        (status = 401, description = "Missing or invalid access token", body = ErrorBody),
        (status = 403, description = "Missing intake.read capability", body = ErrorBody),
        (status = 404, description = "Case not found", body = ErrorBody)
    )
)]
pub async fn timeline(
    State(state): State<AppState>,
    auth: AuthContext,
    Path(id): Path<Uuid>,
    Query(params): Query<TimelineParams>,
) -> Result<Json<TimelineResponse>, AppError> {
    auth.require(Capability::IntakeRead)?;
    load_case_row(&state.db, auth.organisation_id, id).await?;
    let limit = params.limit.unwrap_or(50).clamp(1, 200);
    let rows = sqlx::query_as::<_, TimelineRow>(
        "SELECT e.id, e.event_type, e.actor_user_id AS actor_id,
                u.full_name AS actor_name, u.email AS actor_email,
                e.from_stage_id, fs.key AS from_stage_key, fs.name AS from_stage_name,
                fs.stage_kind AS from_stage_kind,
                e.to_stage_id, ts.key AS to_stage_key, ts.name AS to_stage_name,
                ts.stage_kind AS to_stage_kind,
                e.metadata, e.created_at
         FROM intake_case_events e
         LEFT JOIN users u ON u.id = e.actor_user_id
         LEFT JOIN intake_stages fs
           ON fs.organisation_id = e.organisation_id AND fs.id = e.from_stage_id
         LEFT JOIN intake_stages ts
           ON ts.organisation_id = e.organisation_id AND ts.id = e.to_stage_id
         WHERE e.organisation_id = $1 AND e.case_id = $2
           AND ($3::bigint IS NULL OR e.id < $3)
         ORDER BY e.created_at DESC, e.id DESC
         LIMIT $4",
    )
    .bind(auth.organisation_id)
    .bind(id)
    .bind(params.before)
    .bind(i64::from(limit))
    .fetch_all(&state.db)
    .await?;
    let next = (rows.len() as u32 == limit)
        .then(|| rows.last().map(|row| row.id))
        .flatten();
    Ok(Json(TimelineResponse {
        items: rows.into_iter().map(timeline_dto).collect(),
        next,
    }))
}

// ---------------------------------------------------------------------------
// Contact lifecycle
// ---------------------------------------------------------------------------

#[utoipa::path(
    get,
    path = "/api/v1/contacts/{id}/status-history",
    tag = "intake",
    security(("bearer_auth" = [])),
    params(("id" = Uuid, Path, description = "Contact id")),
    responses(
        (status = 200, description = "Lifecycle history", body = ContactStatusHistoryResponse),
        (status = 401, description = "Missing or invalid access token", body = ErrorBody),
        (status = 403, description = "Missing contacts.read capability", body = ErrorBody),
        (status = 404, description = "Contact not found", body = ErrorBody)
    )
)]
pub async fn contact_status_history(
    State(state): State<AppState>,
    auth: AuthContext,
    Path(id): Path<Uuid>,
    Query(params): Query<HistoryParams>,
) -> Result<Json<ContactStatusHistoryResponse>, AppError> {
    auth.require(Capability::ContactsRead)?;
    let rows = sqlx::query_as::<_, StatusHistoryRow>(
        "SELECT h.id, h.from_status, h.to_status, h.reason,
                h.actor_user_id AS actor_id, u.full_name AS actor_name,
                u.email AS actor_email, h.created_at
         FROM contact_status_history h
         LEFT JOIN users u ON u.id = h.actor_user_id
         WHERE h.organisation_id = $1 AND h.contact_id = $2
         ORDER BY h.created_at DESC, h.id DESC
         LIMIT $3",
    )
    .bind(auth.organisation_id)
    .bind(id)
    .bind(i64::from(params.limit.unwrap_or(100).clamp(1, 200)))
    .fetch_all(&state.db)
    .await?;
    if rows.is_empty() {
        let exists = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM contacts WHERE organisation_id = $1 AND id = $2)",
        )
        .bind(auth.organisation_id)
        .bind(id)
        .fetch_one(&state.db)
        .await?;
        if !exists {
            return Err(AppError::not_found("contact not found"));
        }
    }
    Ok(Json(ContactStatusHistoryResponse {
        items: rows.into_iter().map(status_history_dto).collect(),
        next_cursor: None,
    }))
}

#[utoipa::path(
    patch,
    path = "/api/v1/contacts/{id}/status",
    tag = "intake",
    security(("bearer_auth" = [])),
    request_body = ContactStatusWrite,
    params(("id" = Uuid, Path, description = "Contact id")),
    responses(
        (status = 200, description = "Contact status updated", body = ContactStatusResponse),
        (status = 401, description = "Missing or invalid access token", body = ErrorBody),
        (status = 403, description = "Missing contacts.write capability", body = ErrorBody),
        (status = 404, description = "Contact not found", body = ErrorBody),
        (status = 422, description = "Invalid transition", body = ErrorBody)
    )
)]
pub async fn update_contact_status(
    State(state): State<AppState>,
    auth: AuthContext,
    Path(id): Path<Uuid>,
    Json(payload): Json<ContactStatusWrite>,
) -> Result<Json<ContactStatusResponse>, AppError> {
    auth.require(Capability::ContactsWrite)?;
    if !intake::is_lifecycle_status(&payload.status) {
        return Err(AppError::validation("unknown lifecycle status"));
    }
    let reason = payload
        .reason
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if matches!(
        payload.status.as_str(),
        "suspended" | "offboarding" | "offboarded" | "intake_cancelled"
    ) && reason.is_none()
    {
        return Err(AppError::validation(
            "a reason is required for this lifecycle change",
        ));
    }
    let current = sqlx::query_scalar::<_, String>(
        "SELECT lifecycle_status FROM contacts WHERE organisation_id = $1 AND id = $2",
    )
    .bind(auth.organisation_id)
    .bind(id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::not_found("contact not found"))?;
    if current != payload.status
        && !case_service::allowed_lifecycle_transition(&current, &payload.status)
    {
        return Err(AppError::validation_with_details(
            "That lifecycle transition is not allowed.",
            json!({
                "reason": "lifecycle_transition_invalid",
                "from": current,
                "to": payload.status,
            }),
        ));
    }
    let mut tx = state.db.begin().await?;
    case_service::set_contact_status(
        &mut tx,
        auth.organisation_id,
        id,
        &payload.status,
        reason,
        auth.user_id,
    )
    .await?;
    write_audit(
        &mut tx,
        auth.organisation_id,
        auth.user_id,
        "contact.lifecycle.updated",
        id,
        json!({ "from": current, "to": payload.status, "reason": reason }),
    )
    .await?;
    tx.commit().await?;

    let contact = load_contact_summary(&state.db, auth.organisation_id, id).await?;
    let history = sqlx::query_as::<_, StatusHistoryRow>(
        "SELECT h.id, h.from_status, h.to_status, h.reason,
                h.actor_user_id AS actor_id, u.full_name AS actor_name,
                u.email AS actor_email, h.created_at
         FROM contact_status_history h
         LEFT JOIN users u ON u.id = h.actor_user_id
         WHERE h.organisation_id = $1 AND h.contact_id = $2
         ORDER BY h.created_at DESC, h.id DESC LIMIT 1",
    )
    .bind(auth.organisation_id)
    .bind(id)
    .fetch_one(&state.db)
    .await?;
    Ok(Json(ContactStatusResponse {
        contact,
        history: status_history_dto(history),
    }))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

struct ExternalOrganisation {
    id: Uuid,
    name: String,
}

async fn find_case_by_client_request(
    db: &PgPool,
    organisation_id: Uuid,
    request_id: &str,
) -> Result<Option<Uuid>, AppError> {
    Ok(sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM intake_cases
         WHERE organisation_id = $1 AND client_request_id = $2",
    )
    .bind(organisation_id)
    .bind(request_id)
    .fetch_optional(db)
    .await?)
}

async fn find_case_by_legacy_draft(
    db: &PgPool,
    organisation_id: Uuid,
    legacy_id: &str,
) -> Result<Option<Uuid>, AppError> {
    Ok(sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM intake_cases
         WHERE organisation_id = $1 AND legacy_draft_id = $2",
    )
    .bind(organisation_id)
    .bind(legacy_id)
    .fetch_optional(db)
    .await?)
}

async fn find_open_case(
    db: &PgPool,
    organisation_id: Uuid,
    flow_id: Uuid,
    contact_id: Option<Uuid>,
    external_organisation_id: Option<Uuid>,
) -> Result<Option<(Uuid, String)>, AppError> {
    let row = sqlx::query_as::<_, (Uuid, String)>(
        "SELECT c.id, s.name
         FROM intake_cases c
         JOIN intake_stages s
           ON s.organisation_id = c.organisation_id AND s.id = c.stage_id
         WHERE c.organisation_id = $1 AND c.flow_id = $2
           AND (($3::uuid IS NOT NULL AND c.contact_id = $3)
                OR ($4::uuid IS NOT NULL AND c.external_organisation_id = $4))
           AND c.closed_at IS NULL AND c.onboarded_at IS NULL
         ORDER BY c.updated_at DESC, c.id LIMIT 1",
    )
    .bind(organisation_id)
    .bind(flow_id)
    .bind(contact_id)
    .bind(external_organisation_id)
    .fetch_optional(db)
    .await?;
    Ok(row)
}

async fn load_contact_row(
    db: &PgPool,
    organisation_id: Uuid,
    id: Uuid,
) -> Result<ContactRow, AppError> {
    sqlx::query_as::<_, ContactRow>(
        "SELECT c.id, c.display_name, c.given_name, c.family_name, c.email,
                c.phone, c.date_of_birth, c.lifecycle_status,
                CASE WHEN r.is_client AND r.is_worker THEN 'dual'
                     WHEN r.is_client THEN 'client'
                     WHEN r.is_worker THEN 'worker'
                     ELSE 'other' END AS role
         FROM contacts c
         LEFT JOIN contact_care_roles r ON r.contact_id = c.id
         WHERE c.organisation_id = $1 AND c.id = $2",
    )
    .bind(organisation_id)
    .bind(id)
    .fetch_optional(db)
    .await?
    .ok_or_else(|| AppError::not_found("contact not found"))
}

async fn load_contact_summary(
    db: &PgPool,
    organisation_id: Uuid,
    id: Uuid,
) -> Result<ContactSummaryDto, AppError> {
    let row = load_contact_row(db, organisation_id, id).await?;
    Ok(ContactSummaryDto {
        id: row.id,
        display_name: row.display_name,
        email: row.email,
        phone: row.phone,
        date_of_birth: row.date_of_birth,
        lifecycle_status: row.lifecycle_status,
        role: row.role,
    })
}

async fn insert_contact(
    tx: &mut Transaction<'_, Postgres>,
    auth: &AuthContext,
    input: &ContactCreate,
) -> Result<ContactRow, AppError> {
    auth.require(Capability::ContactsWrite)?;
    let display_name = display_name_from_input(input);
    if display_name.trim().is_empty() {
        return Err(AppError::validation("a contact name is required"));
    }
    if display_name.chars().count() > 160 {
        return Err(AppError::validation(
            "contact display_name must be 160 characters or fewer",
        ));
    }
    let gender = input
        .gender
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| {
            normalise_gender(value)
                .map(str::to_owned)
                .ok_or_else(|| AppError::validation("invalid gender"))
        })
        .transpose()?;
    let languages = normalise_languages(input.languages.clone());
    let mut details = input.details.clone().unwrap_or_else(|| json!({}));
    let Some(fields) = details.as_object_mut() else {
        return Err(AppError::validation("details must be a JSON object"));
    };
    fields.remove("contact_role");
    fields.remove("gender");
    fields.remove("languages");
    fields.remove("cultural_identity");

    let given_name = trim_optional(input.given_name.as_deref());
    let family_name = trim_optional(input.family_name.as_deref());
    let email = trim_optional(input.email.as_deref());
    let phone = trim_optional(input.phone.as_deref());
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO contacts
            (id, organisation_id, display_name, given_name, family_name, preferred_name,
             pronouns, date_of_birth, gender, languages, cultural_identity,
             email, phone, notes, details, lifecycle_status)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, 'enquiry')",
    )
    .bind(id)
    .bind(auth.organisation_id)
    .bind(&display_name)
    .bind(&given_name)
    .bind(&family_name)
    .bind(trim_optional(input.preferred_name.as_deref()))
    .bind(trim_optional(input.pronouns.as_deref()))
    .bind(input.date_of_birth)
    .bind(gender)
    .bind(&languages)
    .bind(trim_optional(input.cultural_identity.as_deref()))
    .bind(&email)
    .bind(&phone)
    .bind(trim_optional(input.notes.as_deref()))
    .bind(SqlJson(details))
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        "INSERT INTO contact_status_history
            (id, organisation_id, contact_id, from_status, to_status, reason, actor_user_id)
         VALUES ($1, $2, $3, NULL, 'enquiry', 'Intake enquiry created', $4)",
    )
    .bind(Uuid::now_v7())
    .bind(auth.organisation_id)
    .bind(id)
    .bind(auth.user_id)
    .execute(&mut **tx)
    .await?;

    Ok(ContactRow {
        id,
        display_name,
        given_name,
        family_name,
        email,
        phone,
        date_of_birth: input.date_of_birth,
        lifecycle_status: "enquiry".to_owned(),
        role: "other".to_owned(),
    })
}

fn display_name_from_input(input: &ContactCreate) -> String {
    let given = input
        .given_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let family = input
        .family_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    match (given, family) {
        (Some(given), Some(family)) => format!("{given} {family}"),
        (Some(given), None) => given.to_owned(),
        (None, Some(family)) => family.to_owned(),
        (None, None) => input
            .preferred_name
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("")
            .to_owned(),
    }
}

async fn load_external_organisation(
    db: &PgPool,
    organisation_id: Uuid,
    id: Uuid,
) -> Result<ExternalOrganisation, AppError> {
    sqlx::query_as::<_, (Uuid, String)>(
        "SELECT id, name FROM organisations
         WHERE tenant_id = $1 AND id = $2",
    )
    .bind(organisation_id)
    .bind(id)
    .fetch_optional(db)
    .await?
    .map(|(id, name)| ExternalOrganisation { id, name })
    .ok_or_else(|| AppError::not_found("organisation not found"))
}

async fn ensure_division(db: &PgPool, organisation_id: Uuid, id: Uuid) -> Result<(), AppError> {
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

async fn ensure_active_member(
    db: &PgPool,
    organisation_id: Uuid,
    user_id: Uuid,
) -> Result<(), AppError> {
    let exists = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(
             SELECT 1 FROM organisation_users m
             JOIN users u ON u.id = m.user_id
             WHERE m.organisation_id = $1 AND m.user_id = $2 AND u.is_active
         )",
    )
    .bind(organisation_id)
    .bind(user_id)
    .fetch_one(db)
    .await?;
    if exists {
        Ok(())
    } else {
        Err(AppError::validation(
            "assignee is not an active member of this organisation",
        ))
    }
}

#[allow(clippy::too_many_arguments)]
async fn insert_event(
    tx: &mut Transaction<'_, Postgres>,
    organisation_id: Uuid,
    case_id: Uuid,
    event_type: &str,
    actor_user_id: Uuid,
    from_stage_id: Option<Uuid>,
    to_stage_id: Option<Uuid>,
    metadata: Value,
) -> Result<i64, AppError> {
    Ok(sqlx::query_scalar::<_, i64>(
        "INSERT INTO intake_case_events
            (organisation_id, case_id, event_type, actor_user_id,
             from_stage_id, to_stage_id, metadata)
         VALUES ($1, $2, $3, $4, $5, $6, $7)
         RETURNING id",
    )
    .bind(organisation_id)
    .bind(case_id)
    .bind(event_type)
    .bind(actor_user_id)
    .bind(from_stage_id)
    .bind(to_stage_id)
    .bind(metadata)
    .fetch_one(&mut **tx)
    .await?)
}

async fn write_audit(
    tx: &mut Transaction<'_, Postgres>,
    organisation_id: Uuid,
    actor_user_id: Uuid,
    action: &str,
    case_id: Uuid,
    metadata: Value,
) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO audit_log
            (organisation_id, actor_user_id, action, entity_type, entity_id, metadata)
         VALUES ($1, $2, $3, 'intake_case', $4, $5)",
    )
    .bind(organisation_id)
    .bind(actor_user_id)
    .bind(action)
    .bind(case_id)
    .bind(metadata)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

fn version_conflict(current_version: i32, case_id: Uuid) -> AppError {
    AppError::conflict_with_details(
        "This case was changed by someone else. Reload and try again.",
        json!({ "reason": "version_conflict", "current_version": current_version, "case_id": case_id }),
    )
}

fn invalid_transition(message: &str) -> AppError {
    AppError::validation_with_details(message, json!({ "reason": "invalid_transition" }))
}

fn trim_optional(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}
