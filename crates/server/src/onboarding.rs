//! Onboarding task catalogue and progress.
//!
//! Definitions are code; only manual decisions are persisted. Automatic tasks
//! are detected from real data so a false tick is impossible.

use std::collections::HashMap;

use sqlx::PgPool;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Organisation,
    Security,
    Structure,
    Catalogue,
    People,
    Rostering,
    Integrations,
    GoLive,
}

impl Stage {
    pub const ALL: [Self; 8] = [
        Self::Organisation,
        Self::Security,
        Self::Structure,
        Self::Catalogue,
        Self::People,
        Self::Rostering,
        Self::Integrations,
        Self::GoLive,
    ];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Organisation => "Organisation",
            Self::Security => "Security and access",
            Self::Structure => "Structure",
            Self::Catalogue => "Catalogue",
            Self::People => "People",
            Self::Rostering => "Rostering pilot",
            Self::Integrations => "Integrations and communications",
            Self::GoLive => "Go live",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Detection {
    Manual,
    SecondAdmin,
    Division,
    CostCentre,
    Service,
    RateVersion,
    Contacts,
    Relationships,
    ImportCommitted,
    Shift,
    Assignment,
}

#[derive(Debug)]
pub struct Task {
    pub key: &'static str,
    pub stage: Stage,
    pub title: &'static str,
    pub why: &'static str,
    pub required: bool,
    pub detection: Detection,
    pub estimate: &'static str,
    pub href: Option<&'static str>,
    pub roles: &'static str,
}

macro_rules! task {
    ($key:literal, $stage:ident, $title:literal, $why:literal, $required:literal, $detection:ident, $estimate:literal, $href:expr, $roles:literal) => {
        Task {
            key: $key,
            stage: Stage::$stage,
            title: $title,
            why: $why,
            required: $required,
            detection: Detection::$detection,
            estimate: $estimate,
            href: $href,
            roles: $roles,
        }
    };
}

pub const TASKS: &[Task] = &[
    task!(
        "org.profile",
        Organisation,
        "Review organisation details",
        "Name, ABN, contact details and website feed documents and invoices.",
        true,
        Manual,
        "2 min",
        None,
        "owner"
    ),
    task!(
        "org.timezone",
        Organisation,
        "Confirm timezone and locale",
        "Rosters and effective dates depend on the right timezone.",
        true,
        Manual,
        "1 min",
        None,
        "owner"
    ),
    task!(
        "access.second_admin",
        Security,
        "Add a second owner or administrator",
        "Prevents a single-account lockout.",
        true,
        SecondAdmin,
        "5 min",
        Some("/admin/invitations/new"),
        "owner"
    ),
    task!(
        "access.roles",
        Security,
        "Review personas and assign permission sets",
        "Give each person the smallest set that does the job.",
        true,
        Manual,
        "5 min",
        Some("/admin/permission-sets"),
        "owner"
    ),
    task!(
        "security.production",
        Security,
        "Confirm production settings",
        "Production mode, a unique JWT secret and TLS in front of the server.",
        true,
        Manual,
        "10 min",
        None,
        "owner"
    ),
    task!(
        "security.backups",
        Security,
        "Verify database backups",
        "A schedule is not a backup until a restore has been tested.",
        true,
        Manual,
        "30 min",
        None,
        "owner"
    ),
    task!(
        "structure.division",
        Structure,
        "Create at least one division",
        "Divisions own services and carry cost centres.",
        true,
        Division,
        "10 min",
        Some("/admin/import"),
        "owner/admin"
    ),
    task!(
        "structure.cost_centres",
        Structure,
        "Add cost centres",
        "Optional unless you reconcile to an accounting system.",
        false,
        CostCentre,
        "15 min",
        Some("/admin/import"),
        "owner/admin"
    ),
    task!(
        "catalogue.service",
        Catalogue,
        "Add a service mapped to a funding code",
        "NDIS or aged-care funding is attached to services.",
        true,
        Service,
        "10 min",
        Some("/admin/import"),
        "owner/admin"
    ),
    task!(
        "catalogue.rate",
        Catalogue,
        "Add a rate with an effective version",
        "Rates are effective-dated; overlap is rejected.",
        true,
        RateVersion,
        "10 min",
        Some("/admin/import"),
        "owner/admin"
    ),
    task!(
        "catalogue.custom_fields",
        Catalogue,
        "Review custom fields",
        "Add only fields your organisation will maintain.",
        false,
        Manual,
        "5 min",
        Some("/admin/custom-fields"),
        "owner/admin"
    ),
    task!(
        "people.contacts",
        People,
        "Add or import clients and workers",
        "Contacts are untyped; roles come from relationships and assignments.",
        true,
        Contacts,
        "30 min",
        Some("/admin/import"),
        "owner/admin/manager"
    ),
    task!(
        "people.relationships",
        People,
        "Link contacts and organisations",
        "Guardians, brokers, doctors and support coordinators.",
        true,
        Relationships,
        "30 min",
        Some("/admin/import"),
        "owner/admin/manager"
    ),
    task!(
        "people.import",
        People,
        "Import from a CSV export",
        "Fastest path when moving from another system.",
        false,
        ImportCommitted,
        "30 min",
        Some("/admin/import"),
        "owner/admin"
    ),
    task!(
        "roster.shift",
        Rostering,
        "Create the first shift",
        "Try the roster with real clients and workers.",
        true,
        Shift,
        "15 min",
        None,
        "manager"
    ),
    task!(
        "roster.assignments",
        Rostering,
        "Assign a client and a worker",
        "Confirms the roster links people correctly.",
        true,
        Assignment,
        "10 min",
        None,
        "manager"
    ),
    task!(
        "roster.completed",
        Rostering,
        "Run and complete one pilot shift",
        "The real proof that configuration works end to end.",
        true,
        Manual,
        "1 day",
        None,
        "manager"
    ),
    task!(
        "integrations.accounting",
        Integrations,
        "Connect Xero or QuickBooks",
        "Not built yet; plan the mapping when available.",
        false,
        Manual,
        "—",
        None,
        "owner"
    ),
    task!(
        "comms.sms",
        Integrations,
        "Configure SMS or email",
        "Not built yet; plan templates when available.",
        false,
        Manual,
        "—",
        None,
        "owner"
    ),
    task!(
        "readiness.audit",
        GoLive,
        "Review users, sessions and audit log",
        "Confirm only intended accounts have access.",
        true,
        Manual,
        "10 min",
        Some("/admin/sessions"),
        "owner"
    ),
    task!(
        "readiness.signoff",
        GoLive,
        "Mark the organisation live",
        "Requires every required task above.",
        true,
        Manual,
        "1 min",
        Some("/admin/onboarding"),
        "owner"
    ),
];

#[derive(Debug, Clone)]
pub struct TaskState {
    pub status: String,
    pub reason: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Summary {
    pub required_done: usize,
    pub required_total: usize,
    pub required_skipped: usize,
    pub optional_done: usize,
    pub optional_total: usize,
    pub percent: u8,
    pub next: Option<&'static Task>,
    pub complete: bool,
}

pub async fn manual_states(
    pool: &PgPool,
    organisation: Uuid,
) -> Result<HashMap<String, TaskState>, sqlx::Error> {
    let rows = sqlx::query_as::<_, (String, String, Option<String>)>(
        "SELECT task_key, status, reason FROM onboarding_task_states
         WHERE organisation_id = $1",
    )
    .bind(organisation)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|(key, status, reason)| (key, TaskState { status, reason }))
        .collect())
}

pub async fn is_detected(
    pool: &PgPool,
    organisation: Uuid,
    detection: Detection,
) -> Result<bool, sqlx::Error> {
    match detection {
        Detection::Manual => Ok(false),
        Detection::SecondAdmin => {
            sqlx::query_scalar::<_, bool>(
                "SELECT (
                    SELECT count(*)
                    FROM organisation_users ou
                    JOIN users u ON u.id = ou.user_id
                    WHERE ou.organisation_id = $1
                      AND u.is_active
                      AND (
                          ou.is_owner
                          OR EXISTS (
                              SELECT 1
                              FROM organisation_user_permission_sets oup
                              JOIN permission_sets ps ON ps.id = oup.permission_set_id
                              WHERE oup.organisation_id = ou.organisation_id
                                AND oup.user_id = ou.user_id
                                AND ps.persona_key = 'administrator'
                          )
                      )
                 ) >= 2",
            )
            .bind(organisation)
            .fetch_one(pool)
            .await
        }
        Detection::Division => {
            exists(pool, "SELECT EXISTS(SELECT 1 FROM divisions WHERE organisation_id=$1)", organisation).await
        }
        Detection::CostCentre => {
            exists(pool, "SELECT EXISTS(SELECT 1 FROM cost_centres WHERE organisation_id=$1)", organisation).await
        }
        Detection::Service => {
            exists(pool, "SELECT EXISTS(SELECT 1 FROM services WHERE organisation_id=$1)", organisation).await
        }
        Detection::RateVersion => {
            exists(pool, "SELECT EXISTS(SELECT 1 FROM rate_versions WHERE organisation_id=$1)", organisation).await
        }
        Detection::Contacts => {
            exists(pool, "SELECT EXISTS(SELECT 1 FROM contacts WHERE organisation_id=$1)", organisation).await
        }
        Detection::Relationships => {
            exists(pool, "SELECT EXISTS(SELECT 1 FROM relationships WHERE organisation_id=$1)", organisation).await
        }
        Detection::ImportCommitted => exists(
            pool,
            "SELECT EXISTS(SELECT 1 FROM import_batches WHERE organisation_id=$1 AND status='committed')",
            organisation,
        )
        .await,
        Detection::Shift => {
            exists(pool, "SELECT EXISTS(SELECT 1 FROM shifts WHERE organisation_id=$1)", organisation).await
        }
        Detection::Assignment => exists(
            pool,
            "SELECT EXISTS(SELECT 1 FROM client_assignments WHERE organisation_id=$1)
                 OR EXISTS(SELECT 1 FROM worker_assignments WHERE organisation_id=$1)",
            organisation,
        )
        .await,
    }
}

async fn exists(pool: &PgPool, sql: &str, organisation: Uuid) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar(sql)
        .bind(organisation)
        .fetch_one(pool)
        .await
}

pub async fn summary(pool: &PgPool, organisation: Uuid) -> Result<Summary, sqlx::Error> {
    sqlx::query(
        "INSERT INTO organisation_onboarding (organisation_id) VALUES ($1)
         ON CONFLICT DO NOTHING",
    )
    .bind(organisation)
    .execute(pool)
    .await?;

    let states = manual_states(pool, organisation).await?;

    let mut required_done = 0;
    let mut required_total = 0;
    let mut required_skipped = 0;
    let mut optional_done = 0;
    let mut optional_total = 0;
    let mut next = None;

    for task in TASKS {
        let state = states.get(task.key);
        let done = match task.detection {
            Detection::Manual => state.is_some_and(|state| state.status == "done"),
            detection => is_detected(pool, organisation, detection).await?,
        };
        let skipped = state.is_some_and(|state| state.status == "skipped");

        if task.required {
            required_total += 1;
            if done {
                required_done += 1;
            }
            if skipped && !done {
                required_skipped += 1;
            }
            if !done && !skipped && next.is_none() {
                next = Some(task);
            }
        } else {
            optional_total += 1;
            if done {
                optional_done += 1;
            }
        }
    }

    let percent = u64::try_from(required_done)
        .ok()
        .and_then(|value| value.checked_mul(100))
        .and_then(|value| value.checked_div(u64::try_from(required_total).ok()?))
        .and_then(|value| u8::try_from(value).ok())
        .unwrap_or(0);

    Ok(Summary {
        required_done,
        required_total,
        required_skipped,
        optional_done,
        optional_total,
        percent,
        next,
        complete: required_done == required_total,
    })
}

pub fn task(key: &str) -> Option<&'static Task> {
    TASKS.iter().find(|task| task.key == key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_unique() {
        let mut seen = std::collections::HashSet::new();
        for task in TASKS {
            assert!(seen.insert(task.key), "duplicate key {}", task.key);
        }
    }

    #[test]
    fn every_stage_has_tasks() {
        for stage in Stage::ALL {
            assert!(TASKS.iter().any(|task| task.stage == stage));
        }
    }

    #[test]
    fn signoff_is_required() {
        assert!(task("readiness.signoff").is_some_and(|task| task.required));
    }
}
