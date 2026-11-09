//! The catalogue of allowed relationship role pairs.
//!
//! Roles are not free text: every relationship uses a pair from its
//! organisation's catalogue (`relationship_types`), in either direction, so
//! "brother" and "brotherr" or "doctor" and "GP" cannot drift apart. Pairs are
//! stored as snake_case role keys; clients show them as words.

use serde::Serialize;
use sqlx::{FromRow, PgExecutor, PgPool};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::error::AppError;

/// Groups used to organise the catalogue in pickers.
pub const CATEGORIES: [(&str, &str); 6] = [
    ("care", "Care and support"),
    ("family", "Family"),
    ("health", "Health"),
    ("services", "Services"),
    ("work", "Work"),
    ("other", "Other"),
];

/// One allowed pair of roles.
#[derive(Debug, Clone, Serialize, FromRow, ToSchema)]
pub struct RelationshipType {
    pub id: Uuid,
    pub from_role: String,
    pub to_role: String,
    /// One of `care`, `family`, `health`, `services`, `work`, `other`.
    pub category: String,
    /// Retired types stay valid on existing relationships but cannot be used
    /// for new ones.
    pub is_active: bool,
    /// Relationships (not removed) currently using this pair.
    pub in_use: i64,
}

/// Turn what someone typed ("Support Coordinator") into a role key
/// (`support_coordinator`). `None` when nothing usable is left.
#[must_use]
pub fn normalise_role(input: &str) -> Option<String> {
    let key = input
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_ascii_lowercase)
        .collect::<Vec<_>>()
        .join("_");
    let starts_with_letter = key.chars().next().is_some_and(|c| c.is_ascii_lowercase());
    (starts_with_letter && key.len() <= 64).then_some(key)
}

/// The organisation's catalogue, grouped by category then role.
pub async fn load(
    db: &PgPool,
    organisation_id: Uuid,
) -> Result<Vec<RelationshipType>, sqlx::Error> {
    sqlx::query_as::<_, RelationshipType>(
        "SELECT t.id, t.from_role, t.to_role, t.category, t.is_active,
                (SELECT count(*) FROM relationships r
                 WHERE r.organisation_id = t.organisation_id AND r.removed_at IS NULL
                   AND ((r.from_role = t.from_role AND r.to_role = t.to_role)
                        OR (r.from_role = t.to_role AND r.to_role = t.from_role))) AS in_use
         FROM relationship_types t
         WHERE t.organisation_id = $1
         ORDER BY CASE t.category
                      WHEN 'care' THEN 1 WHEN 'family' THEN 2 WHEN 'health' THEN 3
                      WHEN 'services' THEN 4 WHEN 'work' THEN 5 ELSE 6 END,
                  t.from_role, t.to_role",
    )
    .bind(organisation_id)
    .fetch_all(db)
    .await
}

/// Fail unless `from_role`/`to_role` is an active pair, in either direction.
pub async fn ensure_allowed<'e>(
    executor: impl PgExecutor<'e>,
    organisation_id: Uuid,
    from_role: &str,
    to_role: &str,
) -> Result<(), AppError> {
    let active: Option<bool> = sqlx::query_scalar(
        "SELECT is_active FROM relationship_types
         WHERE organisation_id = $1
           AND ((from_role = $2 AND to_role = $3) OR (from_role = $3 AND to_role = $2))",
    )
    .bind(organisation_id)
    .bind(from_role)
    .bind(to_role)
    .fetch_optional(executor)
    .await?;

    match active {
        Some(true) => Ok(()),
        Some(false) => Err(AppError::validation(format!(
            "the relationship type {from_role}/{to_role} has been retired"
        ))),
        None => Err(AppError::validation(format!(
            "{from_role}/{to_role} is not a relationship type in this organisation; \
             choose one from GET /api/v1/relationship-types or ask an administrator to add it"
        ))),
    }
}

/// Add a pair. Roles are normalised; a pair already present either way round
/// is a conflict.
pub async fn create(
    db: &PgPool,
    organisation_id: Uuid,
    from_role: &str,
    to_role: &str,
    category: &str,
) -> Result<RelationshipType, AppError> {
    let from_role = normalise_role(from_role)
        .ok_or_else(|| AppError::validation("from_role needs letters, starting with one"))?;
    let to_role = normalise_role(to_role)
        .ok_or_else(|| AppError::validation("to_role needs letters, starting with one"))?;
    if !CATEGORIES.iter().any(|(key, _)| *key == category) {
        return Err(AppError::validation(
            "category must be one of care, family, health, services, work or other",
        ));
    }

    let id = Uuid::now_v7();
    let inserted = sqlx::query(
        "INSERT INTO relationship_types (id, organisation_id, from_role, to_role, category)
         VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT DO NOTHING",
    )
    .bind(id)
    .bind(organisation_id)
    .bind(&from_role)
    .bind(&to_role)
    .bind(category)
    .execute(db)
    .await?
    .rows_affected();
    if inserted == 0 {
        return Err(AppError::conflict(format!(
            "{from_role}/{to_role} is already a relationship type"
        )));
    }

    Ok(RelationshipType {
        id,
        from_role,
        to_role,
        category: category.to_owned(),
        is_active: true,
        in_use: 0,
    })
}

/// Retire (`false`) or restore (`true`) a pair.
pub async fn set_active(
    db: &PgPool,
    organisation_id: Uuid,
    id: Uuid,
    active: bool,
) -> Result<(), AppError> {
    let changed = sqlx::query(
        "UPDATE relationship_types SET is_active = $3 WHERE organisation_id = $1 AND id = $2",
    )
    .bind(organisation_id)
    .bind(id)
    .bind(active)
    .execute(db)
    .await?
    .rows_affected();
    if changed == 0 {
        return Err(AppError::not_found("relationship type not found"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::normalise_role;

    #[test]
    fn typed_roles_become_keys() {
        assert_eq!(
            normalise_role("Support Coordinator").as_deref(),
            Some("support_coordinator")
        );
        assert_eq!(
            normalise_role("  GP / clinic ").as_deref(),
            Some("gp_clinic")
        );
        assert_eq!(
            normalise_role("step-father").as_deref(),
            Some("step_father")
        );
        assert_eq!(
            normalise_role("2nd cousin"),
            None,
            "must start with a letter"
        );
        assert_eq!(normalise_role(" - "), None);
    }
}
