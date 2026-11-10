//! Funding sources, client packages, quotes/service agreements and roster
//! generation.
//!
//! Claims submission is intentionally not here yet. The model records the
//! source's `claiming_model` and the agreement lines so a later claims slice can
//! branch without changing the core tables.

use chrono::{
    DateTime, Datelike, Duration, Local, LocalResult, NaiveDate, NaiveTime, TimeZone, Utc,
};
use serde::Serialize;
use serde_json::Value;
use sqlx::{FromRow, PgPool};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::error::AppError;

pub const SOURCE_KINDS: [&str; 5] = ["ndis", "support_at_home", "chsp", "private", "other"];

pub const CLAIMING_MODELS: [&str; 5] = [
    "ndis_price_guide",
    "support_at_home",
    "chsp",
    "fee_for_service",
    "manual",
];

pub const PACKAGE_STATUSES: [&str; 4] = ["draft", "active", "exhausted", "closed"];

pub const AGREEMENT_STATUSES: [&str; 6] = [
    "draft",
    "quoted",
    "accepted",
    "active",
    "completed",
    "cancelled",
];

pub const UNITS: [&str; 4] = ["hour", "session", "each", "day"];

pub const RECURRENCES: [&str; 4] = ["none", "daily", "weekly", "fortnightly"];

#[derive(Debug, Clone, FromRow)]
struct FundingSourceRow {
    id: Uuid,
    key: String,
    name: String,
    kind: String,
    claiming_model: String,
    description: Option<String>,
    configuration: sqlx::types::Json<Value>,
    is_active: bool,
    is_system: bool,
}

impl FundingSourceRow {
    fn into_source(self) -> FundingSource {
        FundingSource {
            id: self.id,
            key: self.key,
            name: self.name,
            kind: self.kind,
            claiming_model: self.claiming_model,
            description: self.description,
            configuration: self.configuration.0,
            is_active: self.is_active,
            is_system: self.is_system,
        }
    }
}

#[derive(Debug, Clone, FromRow)]
struct AgreementRow {
    id: Uuid,
    client_contact_id: Uuid,
    funding_package_id: Uuid,
    title: String,
    status: String,
    quote_date: Option<NaiveDate>,
    start_date: NaiveDate,
    end_date: Option<NaiveDate>,
    total_amount: f64,
    notes: Option<String>,
    finalised_at: Option<DateTime<Utc>>,
}

impl AgreementRow {
    fn into_agreement(self, lines: Vec<AgreementLine>) -> Agreement {
        Agreement {
            id: self.id,
            client_contact_id: self.client_contact_id,
            funding_package_id: self.funding_package_id,
            title: self.title,
            status: self.status,
            quote_date: self.quote_date,
            start_date: self.start_date,
            end_date: self.end_date,
            total_amount: self.total_amount,
            notes: self.notes,
            finalised_at: self.finalised_at,
            lines,
        }
    }
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct FundingSource {
    pub id: Uuid,
    pub key: String,
    pub name: String,
    /// `ndis`, `support_at_home`, `chsp`, `private` or `other`.
    pub kind: String,
    /// `ndis_price_guide`, `support_at_home`, `chsp`, `fee_for_service` or `manual`.
    pub claiming_model: String,
    pub description: Option<String>,
    pub configuration: Value,
    pub is_active: bool,
    pub is_system: bool,
}

#[derive(Debug, Clone, FromRow, Serialize, ToSchema)]
pub struct FundingPackage {
    pub id: Uuid,
    pub client_contact_id: Uuid,
    pub funding_source_id: Uuid,
    pub source_name: String,
    pub source_kind: String,
    pub name: String,
    pub reference: Option<String>,
    pub start_date: NaiveDate,
    pub end_date: Option<NaiveDate>,
    pub budget_amount: Option<f64>,
    pub status: String,
    pub notes: Option<String>,
    pub committed_amount: f64,
    pub remaining_amount: Option<f64>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct Agreement {
    pub id: Uuid,
    pub client_contact_id: Uuid,
    pub funding_package_id: Uuid,
    pub title: String,
    pub status: String,
    pub quote_date: Option<NaiveDate>,
    pub start_date: NaiveDate,
    pub end_date: Option<NaiveDate>,
    pub total_amount: f64,
    pub notes: Option<String>,
    pub finalised_at: Option<DateTime<Utc>>,
    pub lines: Vec<AgreementLine>,
}

#[derive(Debug, Clone, FromRow, Serialize, ToSchema)]
pub struct AgreementLine {
    pub id: Uuid,
    pub agreement_id: Uuid,
    pub service_id: Option<Uuid>,
    pub service_name: Option<String>,
    pub description: String,
    pub quantity: f64,
    pub unit: String,
    pub unit_rate: f64,
    pub amount: f64,
    pub recurrence: String,
    pub weekdays: Vec<i16>,
    pub start_time: Option<NaiveTime>,
    pub duration_minutes: Option<i32>,
    pub site_id: Option<Uuid>,
    pub site_name: Option<String>,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, FromRow, Serialize, ToSchema)]
pub struct ServiceRef {
    pub id: Uuid,
    pub name: String,
}

pub async fn load_service_refs(
    db: &PgPool,
    organisation_id: Uuid,
) -> Result<Vec<ServiceRef>, AppError> {
    Ok(sqlx::query_as::<_, ServiceRef>(
        "SELECT id, name FROM services
         WHERE organisation_id = $1 AND is_active
         ORDER BY name",
    )
    .bind(organisation_id)
    .fetch_all(db)
    .await?)
}

pub struct PackageWrite {
    pub funding_source_id: Uuid,
    pub name: String,
    pub reference: Option<String>,
    pub start_date: NaiveDate,
    pub end_date: Option<NaiveDate>,
    pub budget_amount: Option<f64>,
    pub notes: Option<String>,
}

pub struct AgreementWrite {
    pub funding_package_id: Uuid,
    pub title: String,
    pub status: Option<String>,
    pub quote_date: Option<NaiveDate>,
    pub start_date: NaiveDate,
    pub end_date: Option<NaiveDate>,
    pub notes: Option<String>,
}

pub struct LineWrite {
    pub service_id: Option<Uuid>,
    pub description: String,
    pub quantity: f64,
    pub unit: Option<String>,
    pub unit_rate: f64,
    pub recurrence: Option<String>,
    pub weekdays: Vec<i16>,
    pub start_time: Option<NaiveTime>,
    pub duration_minutes: Option<i32>,
    pub site_id: Option<Uuid>,
    pub notes: Option<String>,
}

pub async fn load_sources(
    db: &PgPool,
    organisation_id: Uuid,
) -> Result<Vec<FundingSource>, AppError> {
    Ok(sqlx::query_as::<_, FundingSourceRow>(
        "SELECT id, key, name, kind, claiming_model, description, configuration,
                is_active, is_system
         FROM funding_sources
         WHERE organisation_id = $1
         ORDER BY is_system DESC, name",
    )
    .bind(organisation_id)
    .fetch_all(db)
    .await?
    .into_iter()
    .map(FundingSourceRow::into_source)
    .collect())
}

pub async fn load_packages(
    db: &PgPool,
    organisation_id: Uuid,
    client_contact_id: Uuid,
) -> Result<Vec<FundingPackage>, AppError> {
    Ok(sqlx::query_as::<_, FundingPackage>(
        "SELECT p.id, p.client_contact_id, p.funding_source_id,
                s.name AS source_name, s.kind AS source_kind,
                p.name, p.reference, p.start_date, p.end_date,
                p.budget_amount::float8 AS budget_amount, p.status, p.notes,
                COALESCE(v.committed_amount, 0)::float8 AS committed_amount,
                v.remaining_amount::float8 AS remaining_amount
         FROM funding_packages p
         JOIN funding_sources s
           ON s.organisation_id = p.organisation_id AND s.id = p.funding_source_id
         LEFT JOIN funding_package_summary v
           ON v.organisation_id = p.organisation_id AND v.funding_package_id = p.id
         WHERE p.organisation_id = $1 AND p.client_contact_id = $2
         ORDER BY p.start_date DESC, p.name",
    )
    .bind(organisation_id)
    .bind(client_contact_id)
    .fetch_all(db)
    .await?)
}

pub async fn load_agreements(
    db: &PgPool,
    organisation_id: Uuid,
    client_contact_id: Uuid,
) -> Result<Vec<Agreement>, AppError> {
    let mut agreements = sqlx::query_as::<_, AgreementRow>(
        "SELECT id, client_contact_id, funding_package_id, title, status,
                quote_date, start_date, end_date, total_amount::float8 AS total_amount,
                notes, finalised_at
         FROM client_agreements
         WHERE organisation_id = $1 AND client_contact_id = $2
         ORDER BY start_date DESC, title",
    )
    .bind(organisation_id)
    .bind(client_contact_id)
    .fetch_all(db)
    .await?;

    let mut result = Vec::with_capacity(agreements.len());
    for agreement in agreements.drain(..) {
        let lines = load_lines(db, organisation_id, agreement.id).await?;
        result.push(agreement.into_agreement(lines));
    }
    Ok(result)
}

pub async fn load_lines(
    db: &PgPool,
    organisation_id: Uuid,
    agreement_id: Uuid,
) -> Result<Vec<AgreementLine>, AppError> {
    Ok(sqlx::query_as::<_, AgreementLine>(
        "SELECT l.id, l.agreement_id, l.service_id, sv.name AS service_name,
                l.description, l.quantity::float8 AS quantity, l.unit,
                l.unit_rate::float8 AS unit_rate, l.amount::float8 AS amount,
                l.recurrence, l.weekdays, l.start_time, l.duration_minutes,
                l.site_id, st.name AS site_name, l.notes
         FROM agreement_lines l
         LEFT JOIN services sv
           ON sv.organisation_id = l.organisation_id AND sv.id = l.service_id
         LEFT JOIN sites st
           ON st.organisation_id = l.organisation_id AND st.id = l.site_id
         WHERE l.organisation_id = $1 AND l.agreement_id = $2
         ORDER BY l.id",
    )
    .bind(organisation_id)
    .bind(agreement_id)
    .fetch_all(db)
    .await?)
}

pub async fn create_package(
    db: &PgPool,
    organisation_id: Uuid,
    client_contact_id: Uuid,
    write: PackageWrite,
) -> Result<FundingPackage, AppError> {
    ensure_client(db, organisation_id, client_contact_id).await?;
    ensure_source(db, organisation_id, write.funding_source_id).await?;
    validate_dates(write.start_date, write.end_date)?;
    let name = write.name.trim();
    if name.is_empty() || name.chars().count() > 160 {
        return Err(AppError::validation(
            "package name must be between 1 and 160 characters",
        ));
    }
    if let Some(budget) = write.budget_amount {
        if budget < 0.0 {
            return Err(AppError::validation("budget cannot be negative"));
        }
    }

    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO funding_packages
            (id, organisation_id, client_contact_id, funding_source_id, name,
             reference, start_date, end_date, budget_amount, status, notes)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,'active',$10)",
    )
    .bind(id)
    .bind(organisation_id)
    .bind(client_contact_id)
    .bind(write.funding_source_id)
    .bind(name)
    .bind(trimmed(write.reference.as_deref()))
    .bind(write.start_date)
    .bind(write.end_date)
    .bind(write.budget_amount)
    .bind(trimmed(write.notes.as_deref()))
    .execute(db)
    .await?;

    load_packages(db, organisation_id, client_contact_id)
        .await?
        .into_iter()
        .find(|package| package.id == id)
        .ok_or_else(|| AppError::not_found("funding package not found after creation"))
}

pub async fn set_package_status(
    db: &PgPool,
    organisation_id: Uuid,
    id: Uuid,
    status: &str,
) -> Result<(), AppError> {
    if !PACKAGE_STATUSES.contains(&status) {
        return Err(AppError::validation(
            "status must be draft, active, exhausted or closed",
        ));
    }
    let changed = sqlx::query(
        "UPDATE funding_packages SET status = $3
         WHERE organisation_id = $1 AND id = $2",
    )
    .bind(organisation_id)
    .bind(id)
    .bind(status)
    .execute(db)
    .await?
    .rows_affected();
    if changed == 0 {
        return Err(AppError::not_found("funding package not found"));
    }
    Ok(())
}

pub async fn create_agreement(
    db: &PgPool,
    organisation_id: Uuid,
    client_contact_id: Uuid,
    write: AgreementWrite,
) -> Result<Agreement, AppError> {
    ensure_client(db, organisation_id, client_contact_id).await?;
    ensure_package(db, organisation_id, write.funding_package_id).await?;
    validate_dates(write.start_date, write.end_date)?;
    let title = write.title.trim();
    if title.is_empty() || title.chars().count() > 200 {
        return Err(AppError::validation(
            "agreement title must be between 1 and 200 characters",
        ));
    }
    let status = write.status.as_deref().unwrap_or("quoted");
    if !AGREEMENT_STATUSES.contains(&status) {
        return Err(AppError::validation(
            "status must be draft, quoted, accepted, active, completed or cancelled",
        ));
    }

    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO client_agreements
            (id, organisation_id, client_contact_id, funding_package_id, title,
             status, quote_date, start_date, end_date, notes)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)",
    )
    .bind(id)
    .bind(organisation_id)
    .bind(client_contact_id)
    .bind(write.funding_package_id)
    .bind(title)
    .bind(status)
    .bind(write.quote_date)
    .bind(write.start_date)
    .bind(write.end_date)
    .bind(trimmed(write.notes.as_deref()))
    .execute(db)
    .await?;

    load_agreements(db, organisation_id, client_contact_id)
        .await?
        .into_iter()
        .find(|agreement| agreement.id == id)
        .ok_or_else(|| AppError::not_found("agreement not found after creation"))
}

pub async fn set_agreement_status(
    db: &PgPool,
    organisation_id: Uuid,
    id: Uuid,
    status: &str,
) -> Result<(), AppError> {
    if !AGREEMENT_STATUSES.contains(&status) {
        return Err(AppError::validation(
            "status must be draft, quoted, accepted, active, completed or cancelled",
        ));
    }
    if matches!(status, "active" | "completed" | "cancelled") {
        return Err(AppError::validation(
            "use the finalise action to activate an agreement",
        ));
    }
    let changed = sqlx::query(
        "UPDATE client_agreements SET status = $3
         WHERE organisation_id = $1 AND id = $2 AND finalised_at IS NULL",
    )
    .bind(organisation_id)
    .bind(id)
    .bind(status)
    .execute(db)
    .await?
    .rows_affected();
    if changed == 0 {
        return Err(AppError::not_found(
            "agreement not found or already finalised",
        ));
    }
    Ok(())
}

pub async fn add_line(
    db: &PgPool,
    organisation_id: Uuid,
    agreement_id: Uuid,
    write: LineWrite,
) -> Result<AgreementLine, AppError> {
    ensure_agreement(db, organisation_id, agreement_id).await?;
    let unit = write.unit.as_deref().unwrap_or("hour");
    if !UNITS.contains(&unit) {
        return Err(AppError::validation(
            "unit must be hour, session, each or day",
        ));
    }
    let recurrence = write.recurrence.as_deref().unwrap_or("none");
    if !RECURRENCES.contains(&recurrence) {
        return Err(AppError::validation(
            "recurrence must be none, daily, weekly or fortnightly",
        ));
    }
    if write.description.trim().is_empty() {
        return Err(AppError::validation("description is required"));
    }
    if write.description.trim().chars().count() > 300 {
        return Err(AppError::validation(
            "description must be 300 characters or fewer",
        ));
    }
    if write.quantity <= 0.0 {
        return Err(AppError::validation("quantity must be greater than zero"));
    }
    if write.unit_rate < 0.0 {
        return Err(AppError::validation("unit rate cannot be negative"));
    }
    let weekdays = normalise_weekdays(write.weekdays)?;
    if recurrence != "none" {
        if weekdays.is_empty() {
            return Err(AppError::validation(
                "choose at least one weekday for a recurring line",
            ));
        }
        if write.start_time.is_none() || write.duration_minutes.is_none() {
            return Err(AppError::validation(
                "recurring lines need a start time and duration",
            ));
        }
    }
    if let Some(service_id) = write.service_id {
        ensure_service(db, organisation_id, service_id).await?;
    }
    if let Some(site_id) = write.site_id {
        ensure_site(db, organisation_id, site_id).await?;
    }

    let amount = (write.quantity * write.unit_rate * 100.0).round() / 100.0;
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO agreement_lines
            (id, organisation_id, agreement_id, service_id, description,
             quantity, unit, unit_rate, amount, recurrence, weekdays,
             start_time, duration_minutes, site_id, notes)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15)",
    )
    .bind(id)
    .bind(organisation_id)
    .bind(agreement_id)
    .bind(write.service_id)
    .bind(write.description.trim())
    .bind(write.quantity)
    .bind(unit)
    .bind(write.unit_rate)
    .bind(amount)
    .bind(recurrence)
    .bind(&weekdays)
    .bind(write.start_time)
    .bind(write.duration_minutes)
    .bind(write.site_id)
    .bind(trimmed(write.notes.as_deref()))
    .execute(db)
    .await?;

    recompute_total(db, organisation_id, agreement_id).await?;

    load_lines(db, organisation_id, agreement_id)
        .await?
        .into_iter()
        .find(|line| line.id == id)
        .ok_or_else(|| AppError::not_found("agreement line not found after creation"))
}

pub async fn delete_line(
    db: &PgPool,
    organisation_id: Uuid,
    agreement_id: Uuid,
    line_id: Uuid,
) -> Result<(), AppError> {
    let removed = sqlx::query(
        "DELETE FROM agreement_lines
         WHERE organisation_id = $1 AND agreement_id = $2 AND id = $3",
    )
    .bind(organisation_id)
    .bind(agreement_id)
    .bind(line_id)
    .execute(db)
    .await?
    .rows_affected();
    if removed == 0 {
        return Err(AppError::not_found("agreement line not found"));
    }
    recompute_total(db, organisation_id, agreement_id).await?;
    Ok(())
}

/// Finalise a quoted/accepted agreement and generate draft shifts for its
/// scheduled lines. The operation is idempotent in the sense that it refuses to
/// run twice.
pub async fn finalise_agreement(
    db: &PgPool,
    organisation_id: Uuid,
    agreement_id: Uuid,
) -> Result<(Agreement, usize), AppError> {
    let mut tx = db.begin().await?;

    let row = sqlx::query_as::<_, AgreementRow>(
        "SELECT id, client_contact_id, funding_package_id, title, status,
                quote_date, start_date, end_date, total_amount::float8 AS total_amount,
                notes, finalised_at
         FROM client_agreements
         WHERE organisation_id = $1 AND id = $2
         FOR UPDATE",
    )
    .bind(organisation_id)
    .bind(agreement_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| AppError::not_found("agreement not found"))?;

    if row.finalised_at.is_some()
        || matches!(row.status.as_str(), "active" | "completed" | "cancelled")
    {
        return Err(AppError::conflict("agreement has already been finalised"));
    }
    if !matches!(row.status.as_str(), "quoted" | "accepted") {
        return Err(AppError::validation(
            "mark the quote as quoted or accepted before finalising",
        ));
    }

    let lines = load_lines_tx(&mut tx, organisation_id, agreement_id).await?;
    let recurring: Vec<&AgreementLine> = lines
        .iter()
        .filter(|line| line.recurrence != "none")
        .collect();
    if !recurring.is_empty() && row.end_date.is_none() {
        return Err(AppError::validation(
            "a recurring agreement needs an end date before it can be finalised",
        ));
    }

    let mut generated = 0usize;
    for line in recurring {
        let Some(start_time) = line.start_time else {
            continue;
        };
        let Some(duration_minutes) = line.duration_minutes else {
            continue;
        };
        let Some(end_date) = row.end_date else {
            continue;
        };

        let mut date = row.start_date;
        while date <= end_date {
            if occurrence_on(date, &line.recurrence, &line.weekdays, row.start_date) {
                let naive = date.and_time(start_time);
                let starts_at = match Local.from_local_datetime(&naive) {
                    LocalResult::Single(value) => value.with_timezone(&Utc),
                    LocalResult::Ambiguous(value, _) => value.with_timezone(&Utc),
                    LocalResult::None => naive.and_utc(),
                };
                let ends_at = starts_at + Duration::minutes(i64::from(duration_minutes));
                let shift_id = Uuid::now_v7();
                sqlx::query(
                    "INSERT INTO shifts
                        (id, organisation_id, service_id, site_id, title, kind,
                         starts_at, ends_at, status, notes, funding_package_id,
                         agreement_line_id)
                     VALUES ($1,$2,$3,$4,$5,'service_delivery',$6,$7,'draft',$8,$9,$10)",
                )
                .bind(shift_id)
                .bind(organisation_id)
                .bind(line.service_id)
                .bind(line.site_id)
                .bind(&row.title)
                .bind(starts_at)
                .bind(ends_at)
                .bind(format!("Generated from agreement {}", row.title))
                .bind(row.funding_package_id)
                .bind(line.id)
                .execute(&mut *tx)
                .await?;

                sqlx::query(
                    "INSERT INTO client_assignments
                        (id, organisation_id, shift_id, contact_id, status)
                     VALUES ($1,$2,$3,$4,'assigned')",
                )
                .bind(Uuid::now_v7())
                .bind(organisation_id)
                .bind(shift_id)
                .bind(row.client_contact_id)
                .execute(&mut *tx)
                .await?;

                generated += 1;
            }
            date += Duration::days(1);
        }
    }

    sqlx::query(
        "UPDATE client_agreements
         SET status = 'active', finalised_at = now()
         WHERE organisation_id = $1 AND id = $2",
    )
    .bind(organisation_id)
    .bind(agreement_id)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    let agreement = load_agreements(db, organisation_id, row.client_contact_id)
        .await?
        .into_iter()
        .find(|agreement| agreement.id == agreement_id)
        .ok_or_else(|| AppError::not_found("agreement not found after finalising"))?;
    Ok((agreement, generated))
}

fn occurrence_on(
    date: NaiveDate,
    recurrence: &str,
    weekdays: &[i16],
    period_start: NaiveDate,
) -> bool {
    if recurrence == "daily" {
        return true;
    }
    let weekday = date.weekday().number_from_monday() as i16;
    if !weekdays.contains(&weekday) {
        return false;
    }
    if recurrence == "fortnightly" {
        let days = (date - period_start).num_days();
        return (days / 7) % 2 == 0;
    }
    true
}

fn normalise_weekdays(values: Vec<i16>) -> Result<Vec<i16>, AppError> {
    let mut result = Vec::new();
    for value in values {
        if !(1..=7).contains(&value) {
            return Err(AppError::validation("weekdays must be 1 (Mon) to 7 (Sun)"));
        }
        if !result.contains(&value) {
            result.push(value);
        }
    }
    result.sort_unstable();
    Ok(result)
}

async fn load_lines_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    organisation_id: Uuid,
    agreement_id: Uuid,
) -> Result<Vec<AgreementLine>, AppError> {
    Ok(sqlx::query_as::<_, AgreementLine>(
        "SELECT l.id, l.agreement_id, l.service_id, sv.name AS service_name,
                l.description, l.quantity::float8 AS quantity, l.unit,
                l.unit_rate::float8 AS unit_rate, l.amount::float8 AS amount,
                l.recurrence, l.weekdays, l.start_time, l.duration_minutes,
                l.site_id, st.name AS site_name, l.notes
         FROM agreement_lines l
         LEFT JOIN services sv
           ON sv.organisation_id = l.organisation_id AND sv.id = l.service_id
         LEFT JOIN sites st
           ON st.organisation_id = l.organisation_id AND st.id = l.site_id
         WHERE l.organisation_id = $1 AND l.agreement_id = $2
         ORDER BY l.id",
    )
    .bind(organisation_id)
    .bind(agreement_id)
    .fetch_all(&mut **tx)
    .await?)
}

async fn recompute_total(
    db: &PgPool,
    organisation_id: Uuid,
    agreement_id: Uuid,
) -> Result<(), AppError> {
    sqlx::query(
        "UPDATE client_agreements
         SET total_amount = COALESCE((
             SELECT SUM(amount) FROM agreement_lines
             WHERE organisation_id = $1 AND agreement_id = $2
         ), 0)
         WHERE organisation_id = $1 AND id = $2",
    )
    .bind(organisation_id)
    .bind(agreement_id)
    .execute(db)
    .await?;
    Ok(())
}

async fn ensure_client(db: &PgPool, organisation_id: Uuid, id: Uuid) -> Result<(), AppError> {
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM contacts WHERE organisation_id = $1 AND id = $2)",
    )
    .bind(organisation_id)
    .bind(id)
    .fetch_one(db)
    .await?;
    if exists {
        Ok(())
    } else {
        Err(AppError::not_found("client not found"))
    }
}

async fn ensure_source(db: &PgPool, organisation_id: Uuid, id: Uuid) -> Result<(), AppError> {
    let active: Option<bool> = sqlx::query_scalar(
        "SELECT is_active FROM funding_sources WHERE organisation_id = $1 AND id = $2",
    )
    .bind(organisation_id)
    .bind(id)
    .fetch_optional(db)
    .await?;
    match active {
        Some(true) => Ok(()),
        Some(false) => Err(AppError::validation("that funding source has been retired")),
        None => Err(AppError::not_found("funding source not found")),
    }
}

async fn ensure_package(db: &PgPool, organisation_id: Uuid, id: Uuid) -> Result<(), AppError> {
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM funding_packages WHERE organisation_id = $1 AND id = $2)",
    )
    .bind(organisation_id)
    .bind(id)
    .fetch_one(db)
    .await?;
    if exists {
        Ok(())
    } else {
        Err(AppError::not_found("funding package not found"))
    }
}

async fn ensure_agreement(db: &PgPool, organisation_id: Uuid, id: Uuid) -> Result<(), AppError> {
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM client_agreements WHERE organisation_id = $1 AND id = $2)",
    )
    .bind(organisation_id)
    .bind(id)
    .fetch_one(db)
    .await?;
    if exists {
        Ok(())
    } else {
        Err(AppError::not_found("agreement not found"))
    }
}

async fn ensure_service(db: &PgPool, organisation_id: Uuid, id: Uuid) -> Result<(), AppError> {
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM services WHERE organisation_id = $1 AND id = $2)",
    )
    .bind(organisation_id)
    .bind(id)
    .fetch_one(db)
    .await?;
    if exists {
        Ok(())
    } else {
        Err(AppError::not_found("service not found"))
    }
}

async fn ensure_site(db: &PgPool, organisation_id: Uuid, id: Uuid) -> Result<(), AppError> {
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM sites WHERE organisation_id = $1 AND id = $2)",
    )
    .bind(organisation_id)
    .bind(id)
    .fetch_one(db)
    .await?;
    if exists {
        Ok(())
    } else {
        Err(AppError::not_found("site not found"))
    }
}

fn validate_dates(start: NaiveDate, end: Option<NaiveDate>) -> Result<(), AppError> {
    if let Some(end) = end {
        if end < start {
            return Err(AppError::validation("end date cannot be before start date"));
        }
    }
    Ok(())
}

fn trimmed(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}
