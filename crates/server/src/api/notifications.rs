//! In-app notifications for assignees and watchers.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::{
    auth::extractor::AuthContext,
    error::{AppError, ErrorBody},
    state::AppState,
};

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct NotificationDto {
    pub id: i64,
    pub kind: String,
    pub title: String,
    pub body: Option<String>,
    pub entity_type: Option<String>,
    pub entity_id: Option<Uuid>,
    pub read_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct NotificationListDto {
    pub items: Vec<NotificationDto>,
    pub unread_count: i64,
    pub next_cursor: Option<i64>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct UnreadCountDto {
    pub unread_count: i64,
}

#[derive(Debug, Default, Deserialize)]
pub struct ListParams {
    pub unread: Option<bool>,
    pub limit: Option<u32>,
    pub before: Option<i64>,
}

#[derive(Debug, FromRow)]
struct NotificationRow {
    id: i64,
    kind: String,
    title: String,
    body: Option<String>,
    entity_type: Option<String>,
    entity_id: Option<Uuid>,
    read_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
}

#[utoipa::path(
    get,
    path = "/api/v1/notifications",
    tag = "notifications",
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Notifications for the signed-in user", body = NotificationListDto),
        (status = 401, description = "Missing or invalid access token", body = ErrorBody)
    )
)]
pub async fn list(
    State(state): State<AppState>,
    auth: AuthContext,
    Query(params): Query<ListParams>,
) -> Result<Json<NotificationListDto>, AppError> {
    let limit = params.limit.unwrap_or(30).clamp(1, 100);
    let rows = sqlx::query_as::<_, NotificationRow>(
        "SELECT id, kind, title, body, entity_type, entity_id, read_at, created_at
         FROM notifications
         WHERE organisation_id = $1 AND user_id = $2
           AND ($3::boolean IS NULL OR ($3 AND read_at IS NULL))
           AND ($4::bigint IS NULL OR id < $4)
         ORDER BY created_at DESC, id DESC
         LIMIT $5",
    )
    .bind(auth.organisation_id)
    .bind(auth.user_id)
    .bind(params.unread)
    .bind(params.before)
    .bind(i64::from(limit))
    .fetch_all(&state.db)
    .await?;

    let unread_count = sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM notifications
         WHERE organisation_id = $1 AND user_id = $2 AND read_at IS NULL",
    )
    .bind(auth.organisation_id)
    .bind(auth.user_id)
    .fetch_one(&state.db)
    .await?;

    let next_cursor = (rows.len() as u32 == limit)
        .then(|| rows.last().map(|row| row.id))
        .flatten();

    Ok(Json(NotificationListDto {
        items: rows.into_iter().map(notification_dto).collect(),
        unread_count,
        next_cursor,
    }))
}

#[utoipa::path(
    post,
    path = "/api/v1/notifications/{id}/read",
    tag = "notifications",
    security(("bearer_auth" = [])),
    params(("id" = i64, Path, description = "Notification id")),
    responses(
        (status = 204, description = "Notification marked read"),
        (status = 401, description = "Missing or invalid access token", body = ErrorBody),
        (status = 404, description = "Notification not found", body = ErrorBody)
    )
)]
pub async fn mark_read(
    State(state): State<AppState>,
    auth: AuthContext,
    Path(id): Path<i64>,
) -> Result<StatusCode, AppError> {
    let changed = sqlx::query(
        "UPDATE notifications SET read_at = now()
         WHERE organisation_id = $1 AND user_id = $2 AND id = $3 AND read_at IS NULL",
    )
    .bind(auth.organisation_id)
    .bind(auth.user_id)
    .bind(id)
    .execute(&state.db)
    .await?
    .rows_affected();

    if changed == 0 {
        let exists = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(
                 SELECT 1 FROM notifications
                 WHERE organisation_id = $1 AND user_id = $2 AND id = $3
             )",
        )
        .bind(auth.organisation_id)
        .bind(auth.user_id)
        .bind(id)
        .fetch_one(&state.db)
        .await?;
        if !exists {
            return Err(AppError::not_found("notification not found"));
        }
    }

    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    post,
    path = "/api/v1/notifications/read-all",
    tag = "notifications",
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "All notifications marked read", body = UnreadCountDto),
        (status = 401, description = "Missing or invalid access token", body = ErrorBody)
    )
)]
pub async fn mark_all_read(
    State(state): State<AppState>,
    auth: AuthContext,
) -> Result<Json<UnreadCountDto>, AppError> {
    sqlx::query(
        "UPDATE notifications SET read_at = now()
         WHERE organisation_id = $1 AND user_id = $2 AND read_at IS NULL",
    )
    .bind(auth.organisation_id)
    .bind(auth.user_id)
    .execute(&state.db)
    .await?;
    Ok(Json(UnreadCountDto { unread_count: 0 }))
}

fn notification_dto(row: NotificationRow) -> NotificationDto {
    NotificationDto {
        id: row.id,
        kind: row.kind,
        title: row.title,
        body: row.body,
        entity_type: row.entity_type,
        entity_id: row.entity_id,
        read_at: row.read_at,
        created_at: row.created_at,
    }
}
