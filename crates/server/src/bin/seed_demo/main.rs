//! Seed a realistic demo dataset for "Blue Dreams Care".
//!
//! Development only. Refuses to run without `--confirm`, and refuses when
//! `APP_ENV=production`.
//!
//! Run with:
//! ```sh
//! DATABASE_URL=postgres://... cargo run -p carelibre-server --bin seed-demo -- --confirm
//! ```
//!
//! The seed is idempotent for the demo tenant: it deletes and recreates the
//! `blue-dreams-care` organisation. Users with `@bluedreams.example` addresses
//! are replaced; nothing else in the database is touched.

use std::collections::HashMap;

use anyhow::{bail, Context};
use chrono::{DateTime, Duration, NaiveDate, NaiveTime, TimeZone, Utc};
use rand::{rngs::StdRng, Rng, SeedableRng};
use serde_json::{json, Value};
use sqlx::postgres::PgPoolOptions;
use sqlx::types::Json as SqlJson;
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use carelibre_server::funding;

mod names;
mod people;

use people::{People, Staff};

const SLUG: &str = "blue-dreams-care";
const EMAIL_DOMAIN: &str = "bluedreams.example";
const PASSWORD: &str = "BlueDreamsCare!2026";

/// Suburb, postcode and the index of the nearest GP practice in
/// `EXTERNAL_ORGS` (the `health` entries, in order).
const SUBURBS: &[(&str, &str, usize)] = &[
    ("Northcote", "3070", 0),
    ("Brunswick", "3056", 1),
    ("Preston", "3072", 2),
    ("Reservoir", "3073", 3),
    ("Thornbury", "3071", 4),
    ("Coburg", "3058", 1),
    ("Fairfield", "3078", 4),
    ("Heidelberg", "3084", 5),
    ("Epping", "3076", 6),
    ("Bundoora", "3083", 7),
    ("Greensborough", "3088", 7),
    ("Eltham", "3095", 5),
    ("Diamond Creek", "3089", 7),
    ("Watsonia", "3087", 7),
    ("Macleod", "3085", 5),
    ("Rosanna", "3084", 5),
    ("Ivanhoe", "3079", 5),
    ("Alphington", "3078", 0),
    ("Clifton Hill", "3068", 0),
    ("Fitzroy North", "3068", 0),
];

const STREETS: &[&str] = &[
    "High",
    "Station",
    "Plenty",
    "Bell",
    "Gower",
    "Davidson",
    "Gilbert",
    "Arthurton",
    "Separation",
    "Dundas",
    "Victoria",
    "Elizabeth",
    "Albert",
    "Church",
    "Union",
    "St Georges",
];

const SERVICES: &[(&str, &str, &str, f64)] = &[
    ("Personal care", "ndis", "01_011_0107_1_1", 68.0),
    ("Community access", "ndis", "01_012_0107_1_1", 62.0),
    ("Respite", "ndis", "01_013_0107_1_1", 75.0),
    (
        "Supported independent living",
        "ndis",
        "01_014_0107_1_1",
        82.0,
    ),
    ("Transport", "ndis", "01_015_0107_1_1", 55.0),
    ("Domestic assistance", "aged_care", "AC-DOM-01", 58.0),
    ("Personal care (aged)", "aged_care", "AC-PC-01", 66.0),
    ("Social support", "aged_care", "AC-SOC-01", 54.0),
    ("Nursing", "aged_care", "AC-NUR-01", 95.0),
    ("Allied health", "ndis", "01_016_0107_1_1", 120.0),
    ("Support coordination", "ndis", "01_017_0107_1_1", 100.0),
    ("Private care", "other", "PRIV-01", 70.0),
];

const EXTERNAL_ORGS: &[(&str, &str)] = &[
    ("Northside Medical Centre", "health"),
    ("Brunswick Family Practice", "health"),
    ("Preston Health Hub", "health"),
    ("Reservoir Medical Group", "health"),
    ("Thornbury Doctors", "health"),
    ("Heidelberg General Practice", "health"),
    ("Epping Community Health", "health"),
    ("Bundoora Health Clinic", "health"),
    ("Wattle Grove Pharmacy", "pharmacy"),
    ("Bluebell Chemist", "pharmacy"),
    ("Northcote Pharmacy Plus", "pharmacy"),
    ("Coburg Care Pharmacy", "pharmacy"),
    ("Care Connect Support Coordination", "support_coordinator"),
    ("Pathways Coordination", "support_coordinator"),
    ("Bright Futures Support Coordination", "support_coordinator"),
    ("Harbour Support Coordinators", "support_coordinator"),
    ("Horizon Plan Management", "plan_manager"),
    ("Clearwater Plan Managers", "plan_manager"),
    ("Blue Sky Plan Management", "plan_manager"),
    ("Access Plan Partners", "plan_manager"),
    ("Melbourne Allied Health", "allied_health"),
    ("Northside Physiotherapy", "allied_health"),
    ("Speech Works", "allied_health"),
    ("Occupational Therapy Collective", "allied_health"),
    ("Accessible Transit", "transport"),
    ("Community Wheels", "transport"),
    ("Mobility Works Equipment", "equipment"),
    ("Independent Living Supplies", "equipment"),
    ("SafeHome Modifications", "equipment"),
    ("Yarra Cleaning Services", "local_business"),
    ("Bluebell Cafe", "local_business"),
    ("Northcote Laundry", "local_business"),
    ("Green Thumb Garden Care", "local_business"),
    ("Preston Handyman Services", "local_business"),
    ("Advocacy Victoria", "advocacy"),
    ("Disability Rights Network", "advocacy"),
    ("Carer Support Victoria", "advocacy"),
    ("Northern Specialist Rooms", "medical_specialists"),
    ("Office of the Public Advocate", "government"),
    ("Interpreter Connect", "other"),
    ("Community Legal Help", "other"),
    ("Wellbeing Counselling", "other"),
];

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    if !std::env::args().any(|argument| argument == "--confirm") {
        bail!("refusing to seed without `--confirm`");
    }

    let environment = std::env::var("APP_ENV").unwrap_or_else(|_| "development".to_owned());
    if environment.eq_ignore_ascii_case("production") || environment.eq_ignore_ascii_case("prod") {
        bail!("refusing to seed demo data when APP_ENV=production");
    }

    let url = std::env::var("DATABASE_URL").context("DATABASE_URL is required")?;
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .context("connect to PostgreSQL")?;

    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .context("apply migrations")?;

    reset(&pool).await?;

    let mut rng = StdRng::seed_from_u64(20_261_107);
    let mut tx = pool.begin().await?;

    let organisation = insert_organisation(&mut tx, &mut rng).await?;
    let sets = load_personas(&mut tx, organisation).await?;
    create_users(&mut tx, organisation, &sets).await?;
    let divisions = insert_divisions(&mut tx, organisation).await?;
    let sites = insert_sites(&mut tx, organisation, &divisions).await?;
    let cost_centres = insert_cost_centres(&mut tx, organisation, &divisions).await?;
    let services = insert_services(&mut tx, organisation, &divisions, &mut rng).await?;
    let custom_fields = insert_custom_fields(&mut tx, organisation).await?;

    let (clients, staff, duplicate_names) = {
        let mut people = People::new(&mut tx, &mut rng, organisation);
        let networking = people.external_organisations().await?;
        let clients = people.clients(&networking, &custom_fields).await?;
        let staff = people.staff(&custom_fields).await?;
        people.flush().await?;
        (clients, staff, people.duplicate_names())
    };
    let Staff {
        ids: staff,
        conflicts,
    } = staff;
    insert_rosters(
        &mut tx,
        organisation,
        RosterData {
            clients: &clients,
            staff: &staff,
            divisions: &divisions,
            services: &services,
            sites: &sites,
        },
        &mut rng,
    )
    .await?;
    insert_roster_constraints(&mut tx, organisation, &clients, &staff, &services, &sites).await?;
    insert_conflict_constraints(&mut tx, organisation, &conflicts).await?;
    let (quality_values, client_requirements, organisation_requirements) =
        insert_worker_qualities(&mut tx, organisation, &clients, &staff).await?;
    let (funding_packages, funding_agreements, quoted_agreements) =
        insert_funding_examples(&mut tx, organisation, &clients, &services, &sites).await?;

    insert_audit(&mut tx, organisation, clients.len(), staff.len()).await?;

    tx.commit().await?;

    let mut generated_shifts = 0;
    for agreement in quoted_agreements.iter().take(2) {
        match funding::finalise_agreement(&pool, organisation, *agreement).await {
            Ok((_, generated)) => generated_shifts += generated,
            Err(err) => eprintln!("warning: could not finalise demo agreement: {err}"),
        }
    }

    let contact_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM contacts WHERE organisation_id = $1")
            .bind(organisation)
            .fetch_one(&pool)
            .await?;
    let relationship_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM relationships WHERE organisation_id = $1")
            .bind(organisation)
            .fetch_one(&pool)
            .await?;
    let shift_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM shifts WHERE organisation_id = $1")
            .bind(organisation)
            .fetch_one(&pool)
            .await?;
    let site_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM sites WHERE organisation_id = $1")
            .bind(organisation)
            .fetch_one(&pool)
            .await?;
    let constraint_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM roster_constraints WHERE organisation_id = $1")
            .bind(organisation)
            .fetch_one(&pool)
            .await?;

    println!("Blue Dreams Care seeded");
    println!("  organisation: {organisation} ({SLUG})");
    println!("  clients:      {}", clients.len());
    println!("  staff:        {}", staff.len());
    println!("  contacts:     {contact_count}");
    println!("  relationships:{relationship_count}");
    println!("  shared names: {duplicate_names} (coincidental duplicates)");
    println!("  shifts:       {shift_count}");
    println!("  sites:        {site_count}");
    println!("  constraints:  {constraint_count}");
    println!("  qualities:    {quality_values} worker values, {client_requirements} client requirements, {organisation_requirements} organisation requirements");
    println!("  funding:      {funding_packages} packages, {funding_agreements} agreements, {generated_shifts} generated shifts");
    println!("  divisions:    {}", divisions.len());
    println!("  cost centres: {}", cost_centres.len());
    println!("  services:     {}", services.len());
    println!("  users:");
    for email in [
        "owner@bluedreams.example",
        "admin@bluedreams.example",
        "rostering@bluedreams.example",
        "finance@bluedreams.example",
        "intake@bluedreams.example",
    ] {
        println!("    {email}");
    }
    println!("  password:     {PASSWORD}");

    Ok(())
}

async fn reset(pool: &PgPool) -> anyhow::Result<()> {
    if let Some(organisation) =
        sqlx::query_scalar::<_, Uuid>("SELECT id FROM organisations WHERE slug = $1")
            .bind(SLUG)
            .fetch_optional(pool)
            .await?
    {
        sqlx::query("DELETE FROM audit_log WHERE organisation_id = $1")
            .bind(organisation)
            .execute(pool)
            .await?;
        sqlx::query("DELETE FROM organisations WHERE id = $1")
            .bind(organisation)
            .execute(pool)
            .await?;
    }

    sqlx::query("DELETE FROM users WHERE lower(email) LIKE $1")
        .bind(format!("%@{EMAIL_DOMAIN}"))
        .execute(pool)
        .await?;

    Ok(())
}

async fn insert_organisation(
    tx: &mut Transaction<'_, Postgres>,
    rng: &mut StdRng,
) -> anyhow::Result<Uuid> {
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO organisations
            (id, slug, name, edition, kind, abn, email, phone, website, notes)
         VALUES ($1, $2, 'Blue Dreams Care', 'community', 'provider', $3, $4, $5, $6, $7)",
    )
    .bind(id)
    .bind(SLUG)
    .bind(abn(rng))
    .bind(format!("hello@{EMAIL_DOMAIN}"))
    .bind("03 9000 1234")
    .bind("https://bluedreams.example")
    .bind("Demo dataset: disability and aged-care provider in Melbourne's north.")
    .execute(&mut **tx)
    .await?;

    Ok(id)
}

async fn load_personas(
    tx: &mut Transaction<'_, Postgres>,
    organisation: Uuid,
) -> anyhow::Result<HashMap<String, Uuid>> {
    let rows = sqlx::query_as::<_, (String, Uuid)>(
        "SELECT persona_key, id FROM permission_sets
         WHERE organisation_id = $1 AND persona_key IS NOT NULL",
    )
    .bind(organisation)
    .fetch_all(&mut **tx)
    .await?;

    Ok(rows.into_iter().collect())
}

async fn create_users(
    tx: &mut Transaction<'_, Postgres>,
    organisation: Uuid,
    sets: &HashMap<String, Uuid>,
) -> anyhow::Result<()> {
    let password_hash =
        carelibre_server::auth::password::hash_password(PASSWORD.to_owned()).await?;

    for (email, name, persona, is_owner) in [
        ("owner@bluedreams.example", "Avery Chen", None, true),
        (
            "admin@bluedreams.example",
            "Priya Nair",
            Some("administrator"),
            false,
        ),
        (
            "rostering@bluedreams.example",
            "Mia Robinson",
            Some("rostering"),
            false,
        ),
        (
            "finance@bluedreams.example",
            "Noah Williams",
            Some("finance"),
            false,
        ),
        (
            "intake@bluedreams.example",
            "Amelia Brown",
            Some("intake"),
            false,
        ),
    ] {
        let user_id = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO users (id, email, password_hash, full_name)
             VALUES ($1, $2, $3, $4)",
        )
        .bind(user_id)
        .bind(email)
        .bind(&password_hash)
        .bind(name)
        .execute(&mut **tx)
        .await?;

        sqlx::query(
            "INSERT INTO organisation_users
                (organisation_id, user_id, role, is_owner)
             VALUES ($1, $2, $3, $4)",
        )
        .bind(organisation)
        .bind(user_id)
        .bind(if is_owner { "owner" } else { "staff" })
        .bind(is_owner)
        .execute(&mut **tx)
        .await?;

        if let Some(persona) = persona {
            let set_id = sets
                .get(persona)
                .with_context(|| format!("missing persona {persona}"))?;
            sqlx::query(
                "INSERT INTO organisation_user_permission_sets
                    (organisation_id, user_id, permission_set_id)
                 VALUES ($1, $2, $3)",
            )
            .bind(organisation)
            .bind(user_id)
            .bind(set_id)
            .execute(&mut **tx)
            .await?;
        }
    }

    Ok(())
}

async fn insert_divisions(
    tx: &mut Transaction<'_, Postgres>,
    organisation: Uuid,
) -> anyhow::Result<Vec<Uuid>> {
    let names = [
        ("North", "NORTH"),
        ("South", "SOUTH"),
        ("East", "EAST"),
        ("Respite and Community", "RESPITE"),
    ];

    let mut ids = Vec::new();
    for (name, code) in names {
        let id = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO divisions (id, organisation_id, name, code)
             VALUES ($1, $2, $3, $4)",
        )
        .bind(id)
        .bind(organisation)
        .bind(name)
        .bind(code)
        .execute(&mut **tx)
        .await?;
        ids.push(id);
    }
    Ok(ids)
}

async fn insert_sites(
    tx: &mut Transaction<'_, Postgres>,
    organisation: Uuid,
    divisions: &[Uuid],
) -> anyhow::Result<Vec<Uuid>> {
    let names = [
        ("Blue Dreams Day Program", 0usize),
        ("North Respite House", 0),
        ("South Community Hub", 1),
        ("East Office", 2),
    ];

    let mut ids = Vec::new();
    for (name, division_index) in names {
        let id = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO sites (id, organisation_id, division_id, name, address)
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(id)
        .bind(organisation)
        .bind(divisions.get(division_index).copied())
        .bind(name)
        .bind(format!(
            "{} demo site",
            SUBURBS[ids.len() % SUBURBS.len()].0
        ))
        .execute(&mut **tx)
        .await?;
        ids.push(id);
    }
    Ok(ids)
}

async fn insert_cost_centres(
    tx: &mut Transaction<'_, Postgres>,
    organisation: Uuid,
    divisions: &[Uuid],
) -> anyhow::Result<Vec<Uuid>> {
    let parent = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO cost_centres
            (id, organisation_id, code, name, description, external_system, external_code)
         VALUES ($1, $2, 'CC-100', 'Programs', 'All care programs', 'xero', 'PROG-100')",
    )
    .bind(parent)
    .bind(organisation)
    .execute(&mut **tx)
    .await?;

    let children = [
        ("CC-110", "North", "PROG-110"),
        ("CC-120", "South", "PROG-120"),
        ("CC-130", "East", "PROG-130"),
        ("CC-140", "Respite", "PROG-140"),
    ];

    let mut ids = vec![parent];
    for (code, name, external) in children {
        let id = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO cost_centres
                (id, organisation_id, parent_id, code, name, external_system, external_code)
             VALUES ($1, $2, $3, $4, $5, 'xero', $6)",
        )
        .bind(id)
        .bind(organisation)
        .bind(parent)
        .bind(code)
        .bind(name)
        .bind(external)
        .execute(&mut **tx)
        .await?;
        ids.push(id);
    }

    for (index, division) in divisions.iter().enumerate() {
        let cost_centre = ids.get(index + 1).copied().unwrap_or(parent);
        sqlx::query(
            "INSERT INTO cost_centre_assignments
                (id, organisation_id, cost_centre_id, entity_type, entity_id)
             VALUES ($1, $2, $3, 'division', $4)",
        )
        .bind(Uuid::now_v7())
        .bind(organisation)
        .bind(cost_centre)
        .bind(division)
        .execute(&mut **tx)
        .await?;
    }

    Ok(ids)
}

async fn insert_services(
    tx: &mut Transaction<'_, Postgres>,
    organisation: Uuid,
    divisions: &[Uuid],
    _rng: &mut StdRng,
) -> anyhow::Result<Vec<Uuid>> {
    let mut ids = Vec::new();

    for (index, (name, scheme, funding_code, base)) in SERVICES.iter().enumerate() {
        let service_id = Uuid::now_v7();
        let code = format!("SVC-{:03}", index + 1);
        sqlx::query(
            "INSERT INTO services
                (id, organisation_id, division_id, code, name, description,
                 funding_scheme, funding_code)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        )
        .bind(service_id)
        .bind(organisation)
        .bind(divisions[index % divisions.len()])
        .bind(&code)
        .bind(name)
        .bind(format!("{name} support"))
        .bind(scheme)
        .bind(funding_code)
        .execute(&mut **tx)
        .await?;

        for (rate_name, multiplier) in [("Weekday", 1.0), ("Weekend", 1.2)] {
            let rate_id = Uuid::now_v7();
            sqlx::query(
                "INSERT INTO rates
                    (id, organisation_id, service_id, name, unit_of_measure, description)
                 VALUES ($1, $2, $3, $4, 'hour', $5)",
            )
            .bind(rate_id)
            .bind(organisation)
            .bind(service_id)
            .bind(rate_name)
            .bind(format!("{rate_name} rate for {name}"))
            .execute(&mut **tx)
            .await?;

            for (from, to, factor) in [
                ("2026-07-01", Some("2027-07-01"), 1.0),
                ("2027-07-01", None, 1.04),
            ] {
                let amount = (base * multiplier * factor * 100.0).round() / 100.0;
                sqlx::query(
                    "INSERT INTO rate_versions
                        (id, organisation_id, rate_id, amount, effective_from, effective_to)
                     VALUES ($1, $2, $3, $4, $5::date, $6::date)",
                )
                .bind(Uuid::now_v7())
                .bind(organisation)
                .bind(rate_id)
                .bind(amount)
                .bind(from)
                .bind(to)
                .execute(&mut **tx)
                .await?;
            }
        }

        ids.push(service_id);
    }

    Ok(ids)
}

async fn insert_custom_fields(
    tx: &mut Transaction<'_, Postgres>,
    organisation: Uuid,
) -> anyhow::Result<HashMap<String, Uuid>> {
    let definitions = [
        (
            "ndis_number",
            "NDIS number",
            "text",
            Some("^[0-9]{9}$"),
            None,
        ),
        ("plan_end_date", "Plan end date", "date", None, None),
        (
            "risk_level",
            "Risk level",
            "select",
            None,
            Some(json!(["low", "medium", "high"])),
        ),
        ("employee_number", "Employee number", "text", None, None),
    ];

    let mut map = HashMap::new();
    for (key, label, data_type, pattern, options) in definitions {
        let id = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO custom_field_definitions
                (id, organisation_id, entity_type, key, label, data_type, pattern, options, sort_order)
             VALUES ($1, $2, 'contact', $3, $4, $5, $6, $7, 0)",
        )
        .bind(id)
        .bind(organisation)
        .bind(key)
        .bind(label)
        .bind(data_type)
        .bind(pattern)
        .bind(options)
        .execute(&mut **tx)
        .await?;
        map.insert(key.to_owned(), id);
    }

    Ok(map)
}

struct RosterData<'a> {
    clients: &'a [Uuid],
    staff: &'a [Uuid],
    divisions: &'a [Uuid],
    services: &'a [Uuid],
    sites: &'a [Uuid],
}

async fn insert_rosters(
    tx: &mut Transaction<'_, Postgres>,
    organisation: Uuid,
    data: RosterData<'_>,
    _rng: &mut StdRng,
) -> anyhow::Result<()> {
    let RosterData {
        clients,
        staff,
        divisions,
        services,
        sites,
    } = data;
    let today = Utc::now().date_naive();
    let start = today - Duration::days(7);
    let slots: [(u32, u32); 4] = [(7, 9), (9, 12), (13, 15), (15, 18)];
    let mut shift_index = 0usize;

    for day in 0..14 {
        let date = start + Duration::days(day);
        let past = date < today;
        for slot in 0..30 {
            let (start_hour, end_hour) = slots[slot % slots.len()];
            let Some(starts_at) = utc_at(date, start_hour) else {
                continue;
            };
            let Some(ends_at) = utc_at(date, end_hour) else {
                continue;
            };

            let client = clients[(day as usize * 37 + slot * 7) % clients.len()];
            let worker = staff[(day as usize * 23 + slot * 11) % staff.len()];
            let service = services[shift_index % services.len()];
            let site = sites[shift_index % sites.len()];
            let shift_status = if past { "completed" } else { "published" };
            let assignment_status = if past { "attended" } else { "assigned" };

            let shift_id = Uuid::now_v7();
            sqlx::query(
                "INSERT INTO shifts
                    (id, organisation_id, division_id, service_id, site_id, title, kind,
                     starts_at, ends_at, location, status)
                 VALUES ($1, $2, $3, $4, $5, $6, 'service_delivery', $7, $8, $9, $10)",
            )
            .bind(shift_id)
            .bind(organisation)
            .bind(divisions[shift_index % divisions.len()])
            .bind(service)
            .bind(site)
            .bind(SERVICES[shift_index % SERVICES.len()].0)
            .bind(starts_at)
            .bind(ends_at)
            .bind(format!(
                "Client home, {}",
                SUBURBS[shift_index % SUBURBS.len()].0
            ))
            .bind(shift_status)
            .execute(&mut **tx)
            .await?;

            let session_id = if shift_index % 5 == 0 {
                let Some(session_start) = utc_at(date, start_hour) else {
                    continue;
                };
                let Some(session_end) = utc_at(date, start_hour + 1) else {
                    continue;
                };
                let id = Uuid::now_v7();
                sqlx::query(
                    "INSERT INTO shift_sessions
                        (id, organisation_id, shift_id, title, starts_at, ends_at)
                     VALUES ($1, $2, $3, 'Session', $4, $5)",
                )
                .bind(id)
                .bind(organisation)
                .bind(shift_id)
                .bind(session_start)
                .bind(session_end)
                .execute(&mut **tx)
                .await?;
                Some(id)
            } else {
                None
            };

            sqlx::query(
                "INSERT INTO client_assignments
                    (id, organisation_id, shift_id, session_id, contact_id, status)
                 VALUES ($1, $2, $3, $4, $5, $6)",
            )
            .bind(Uuid::now_v7())
            .bind(organisation)
            .bind(shift_id)
            .bind(session_id)
            .bind(client)
            .bind(assignment_status)
            .execute(&mut **tx)
            .await?;

            let (outcome, rating): (Option<&str>, Option<i16>) = if past {
                if shift_index % 17 == 0 {
                    (Some("negative"), Some(2))
                } else if shift_index % 5 == 0 {
                    (Some("neutral"), Some(3))
                } else {
                    (
                        Some("positive"),
                        Some(if shift_index % 3 == 0 { 5 } else { 4 }),
                    )
                }
            } else {
                (None, None)
            };

            sqlx::query(
                "INSERT INTO worker_assignments
                    (id, organisation_id, shift_id, session_id, contact_id, status,
                     outcome, outcome_rating)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
            )
            .bind(Uuid::now_v7())
            .bind(organisation)
            .bind(shift_id)
            .bind(session_id)
            .bind(worker)
            .bind(assignment_status)
            .bind(outcome)
            .bind(rating)
            .execute(&mut **tx)
            .await?;

            // Some group shifts have a second worker, which creates
            // worker-to-worker history for suitability.
            if shift_index % 10 == 0 {
                let second = staff[(shift_index * 13 + 7) % staff.len()];
                if second != worker {
                    sqlx::query(
                        "INSERT INTO worker_assignments
                            (id, organisation_id, shift_id, session_id, contact_id, status,
                             outcome, outcome_rating)
                         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
                    )
                    .bind(Uuid::now_v7())
                    .bind(organisation)
                    .bind(shift_id)
                    .bind(session_id)
                    .bind(second)
                    .bind(assignment_status)
                    .bind(outcome)
                    .bind(rating)
                    .execute(&mut **tx)
                    .await?;
                }
            }

            shift_index += 1;
        }
    }

    Ok(())
}

async fn insert_worker_qualities(
    tx: &mut Transaction<'_, Postgres>,
    organisation: Uuid,
    clients: &[Uuid],
    staff: &[Uuid],
) -> anyhow::Result<(usize, usize, usize)> {
    let rows: Vec<(String, Uuid)> =
        sqlx::query_as("SELECT key, id FROM worker_qualities WHERE organisation_id = $1")
            .bind(organisation)
            .fetch_all(&mut **tx)
            .await?;
    let qualities: HashMap<String, Uuid> = rows.into_iter().collect();
    let quality = |key: &str| {
        qualities
            .get(key)
            .copied()
            .with_context(|| format!("the demo catalogue is missing {key}"))
    };

    // Intrinsic profile data, entered once on the contact. The mix gives the
    // requirement engine realistic matches, mismatches and unknowns.
    for (index, contact) in staff.iter().enumerate() {
        let gender = match index % 11 {
            3 => "non_binary",
            8 => "prefer_not_to_say",
            _ if index % 2 == 0 => "female",
            _ => "male",
        };
        let languages: Vec<String> = match index % 6 {
            0 => vec!["English".to_owned(), "Cantonese".to_owned()],
            1 => vec!["English".to_owned(), "Arabic".to_owned()],
            2 => vec!["English".to_owned(), "Vietnamese".to_owned()],
            3 => vec!["English".to_owned(), "Mandarin".to_owned()],
            _ => vec!["English".to_owned()],
        };
        let cultural_identity = match index % 9 {
            0 => Some("Anglo-Australian"),
            1 => Some("Chinese-Australian"),
            2 => Some("Vietnamese-Australian"),
            3 => Some("Aboriginal and Torres Strait Islander"),
            4 => Some("Greek-Australian"),
            _ => None,
        };
        sqlx::query(
            "UPDATE contacts
             SET gender = $3, languages = $4, cultural_identity = $5
             WHERE organisation_id = $1 AND id = $2",
        )
        .bind(organisation)
        .bind(contact)
        .bind(gender)
        .bind(&languages)
        .bind(cultural_identity)
        .execute(&mut **tx)
        .await?;
    }

    for (index, contact) in clients.iter().enumerate() {
        let languages: Vec<String> = match index % 7 {
            0 => vec!["English".to_owned(), "Cantonese".to_owned()],
            1 => vec!["English".to_owned(), "Italian".to_owned()],
            2 => vec!["English".to_owned(), "Vietnamese".to_owned()],
            3 => vec!["English".to_owned(), "Arabic".to_owned()],
            _ => vec!["English".to_owned()],
        };
        let cultural_identity = match index % 8 {
            0 => Some("Chinese-Australian"),
            1 => Some("Italian-Australian"),
            2 => Some("Vietnamese-Australian"),
            3 => Some("Aboriginal and Torres Strait Islander"),
            _ => None,
        };
        sqlx::query(
            "UPDATE contacts
             SET languages = $3, cultural_identity = $4
             WHERE organisation_id = $1 AND id = $2",
        )
        .bind(organisation)
        .bind(contact)
        .bind(&languages)
        .bind(cultural_identity)
        .execute(&mut **tx)
        .await?;
    }

    let today = Utc::now().date_naive();
    let mut value_count = 0;

    for (index, worker) in staff.iter().copied().enumerate() {
        // Organisation screening checks: most workers hold them, a few are
        // missing, expired or close to expiry so suitability shows every state.
        match index % 11 {
            4 => {}
            3 => {
                add_quality_value(
                    tx,
                    organisation,
                    worker,
                    quality("ndis_worker_screening")?,
                    Some(today - Duration::days(30)),
                    "NDIS clearance expired",
                )
                .await?;
                value_count += 1;
            }
            5 => {
                add_quality_value(
                    tx,
                    organisation,
                    worker,
                    quality("ndis_worker_screening")?,
                    Some(today + Duration::days(15)),
                    "NDIS clearance expiring",
                )
                .await?;
                value_count += 1;
            }
            _ => {
                add_quality_value(
                    tx,
                    organisation,
                    worker,
                    quality("ndis_worker_screening")?,
                    Some(today + Duration::days(365 + (index as i64 % 120))),
                    "NDIS clearance current",
                )
                .await?;
                value_count += 1;
            }
        }

        if index % 13 != 6 {
            add_quality_value(
                tx,
                organisation,
                worker,
                quality("police_check")?,
                Some(today + Duration::days(180 + (index as i64 % 90))),
                "Police check current",
            )
            .await?;
            value_count += 1;
        }

        if index % 17 != 8 {
            add_quality_value(
                tx,
                organisation,
                worker,
                quality("working_with_children_check")?,
                Some(today + Duration::days(730 + (index as i64 % 180))),
                "Working With Children Check current",
            )
            .await?;
            value_count += 1;
        }

        match index % 8 {
            0 => {
                add_quality_value(
                    tx,
                    organisation,
                    worker,
                    quality("first_aid")?,
                    Some(today - Duration::days(20)),
                    "First aid expired",
                )
                .await?;
                value_count += 1;
            }
            1 => {
                add_quality_value(
                    tx,
                    organisation,
                    worker,
                    quality("first_aid")?,
                    Some(today + Duration::days(20)),
                    "First aid expiring",
                )
                .await?;
                value_count += 1;
            }
            2 => {}
            _ => {
                add_quality_value(
                    tx,
                    organisation,
                    worker,
                    quality("first_aid")?,
                    Some(today + Duration::days(365 + (index as i64 % 200))),
                    "First aid current",
                )
                .await?;
                value_count += 1;
            }
        }

        if index % 3 != 2 {
            add_quality_value(
                tx,
                organisation,
                worker,
                quality("manual_handling")?,
                Some(today + Duration::days(365 + (index as i64 % 200))),
                "Manual handling current",
            )
            .await?;
            value_count += 1;
        }

        if index % 9 == 0 {
            add_quality_value(
                tx,
                organisation,
                worker,
                quality("peg_feeding")?,
                Some(today + Duration::days(365 + (index as i64 % 120))),
                "PEG feeding training current",
            )
            .await?;
            value_count += 1;
        }

        if index % 7 == 0 {
            add_quality_value(
                tx,
                organisation,
                worker,
                quality("medication_administration")?,
                Some(today + Duration::days(365 + (index as i64 % 120))),
                "Medication administration current",
            )
            .await?;
            value_count += 1;
        }

        if index % 4 == 0 {
            add_quality_value(
                tx,
                organisation,
                worker,
                quality("drivers_licence")?,
                Some(today + Duration::days(730 + (index as i64 % 365))),
                "Driver's licence current",
            )
            .await?;
            value_count += 1;
        }
    }

    let mut client_count = 0;
    for (index, client) in clients.iter().copied().enumerate() {
        match index {
            0 => {
                add_client_requirement(
                    tx,
                    organisation,
                    client,
                    quality("gender")?,
                    "equals",
                    Some(Value::String("female".to_owned())),
                    "warn",
                    "Personal care preference",
                )
                .await?;
                add_client_requirement(
                    tx,
                    organisation,
                    client,
                    quality("peg_feeding")?,
                    "exists",
                    None,
                    "warn",
                    "PEG fed; worker must be trained",
                )
                .await?;
                client_count += 2;
            }
            1 => {
                add_client_requirement(
                    tx,
                    organisation,
                    client,
                    quality("manual_handling")?,
                    "exists",
                    None,
                    "warn",
                    "Manual transfers",
                )
                .await?;
                client_count += 1;
            }
            2 => {
                add_client_requirement(
                    tx,
                    organisation,
                    client,
                    quality("languages")?,
                    "contains",
                    Some(Value::String("Cantonese".to_owned())),
                    "warn",
                    "Speaks Cantonese at home",
                )
                .await?;
                client_count += 1;
            }
            3 => {
                add_client_requirement(
                    tx,
                    organisation,
                    client,
                    quality("first_aid")?,
                    "exists",
                    None,
                    "block",
                    "High medical support needs",
                )
                .await?;
                client_count += 1;
            }
            4 => {
                add_client_requirement(
                    tx,
                    organisation,
                    client,
                    quality("peg_feeding")?,
                    "exists",
                    None,
                    "block",
                    "PEG fed; training is mandatory",
                )
                .await?;
                client_count += 1;
            }
            5 => {
                add_client_requirement(
                    tx,
                    organisation,
                    client,
                    quality("medication_administration")?,
                    "exists",
                    None,
                    "warn",
                    "Medication support",
                )
                .await?;
                client_count += 1;
            }
            6 => {
                add_client_requirement(
                    tx,
                    organisation,
                    client,
                    quality("gender")?,
                    "equals",
                    Some(Value::String("female".to_owned())),
                    "warn",
                    "Personal care preference",
                )
                .await?;
                client_count += 1;
            }
            7 => {
                add_client_requirement(
                    tx,
                    organisation,
                    client,
                    quality("drivers_licence")?,
                    "exists",
                    None,
                    "warn",
                    "Transport support",
                )
                .await?;
                client_count += 1;
            }
            8 => {
                add_client_requirement(
                    tx,
                    organisation,
                    client,
                    quality("cpr")?,
                    "exists",
                    None,
                    "warn",
                    "High intensity support",
                )
                .await?;
                client_count += 1;
            }
            9 => {
                add_client_requirement(
                    tx,
                    organisation,
                    client,
                    quality("working_with_children_check")?,
                    "exists",
                    None,
                    "warn",
                    "Child-safe support setting",
                )
                .await?;
                client_count += 1;
            }
            _ => {}
        }
    }

    let organisation_requirements = [
        (
            "ndis_worker_screening",
            "block",
            "NDIS practice standard: all workers must hold a current clearance",
        ),
        (
            "police_check",
            "warn",
            "Organisation policy: current police check",
        ),
        (
            "working_with_children_check",
            "warn",
            "Working with children and vulnerable people",
        ),
    ];
    for (key, severity, reason) in organisation_requirements {
        add_organisation_requirement(
            tx,
            organisation,
            quality(key)?,
            "exists",
            None,
            severity,
            reason,
        )
        .await?;
    }

    Ok((value_count, client_count, organisation_requirements.len()))
}

async fn add_quality_value(
    tx: &mut Transaction<'_, Postgres>,
    organisation: Uuid,
    worker: Uuid,
    quality: Uuid,
    expires_on: Option<NaiveDate>,
    evidence: &str,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO worker_quality_values
            (id, organisation_id, worker_contact_id, quality_id, value, expires_on, evidence)
         VALUES ($1, $2, $3, $4, 'true'::jsonb, $5, $6)",
    )
    .bind(Uuid::now_v7())
    .bind(organisation)
    .bind(worker)
    .bind(quality)
    .bind(expires_on)
    .bind(evidence)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn add_client_requirement(
    tx: &mut Transaction<'_, Postgres>,
    organisation: Uuid,
    client: Uuid,
    quality: Uuid,
    operator: &str,
    value: Option<Value>,
    severity: &str,
    reason: &str,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO client_quality_requirements
            (id, organisation_id, client_contact_id, quality_id, operator, value, severity, reason)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
    )
    .bind(Uuid::now_v7())
    .bind(organisation)
    .bind(client)
    .bind(quality)
    .bind(operator)
    .bind(value.map(SqlJson))
    .bind(severity)
    .bind(reason)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn add_organisation_requirement(
    tx: &mut Transaction<'_, Postgres>,
    organisation: Uuid,
    quality: Uuid,
    operator: &str,
    value: Option<Value>,
    severity: &str,
    reason: &str,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO organisation_quality_requirements
            (id, organisation_id, quality_id, operator, value, severity, reason)
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(Uuid::now_v7())
    .bind(organisation)
    .bind(quality)
    .bind(operator)
    .bind(value.map(SqlJson))
    .bind(severity)
    .bind(reason)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn insert_funding_examples(
    tx: &mut Transaction<'_, Postgres>,
    organisation: Uuid,
    clients: &[Uuid],
    services: &[Uuid],
    sites: &[Uuid],
) -> anyhow::Result<(usize, usize, Vec<Uuid>)> {
    let rows: Vec<(String, Uuid)> =
        sqlx::query_as("SELECT key, id FROM funding_sources WHERE organisation_id = $1")
            .bind(organisation)
            .fetch_all(&mut **tx)
            .await?;
    let sources: HashMap<String, Uuid> = rows.into_iter().collect();
    let source = |key: &str| {
        sources
            .get(key)
            .copied()
            .with_context(|| format!("the demo funding sources are missing {key}"))
    };

    let today = Utc::now().date_naive();
    let mut package_count = 0;
    let mut agreement_count = 0;
    let mut quoted = Vec::new();

    for (index, client) in clients.iter().copied().enumerate() {
        let (source_key, budget) = match index {
            0 => ("ndis", 50_000.0),
            1 => ("support_at_home", 15_000.0),
            2 => ("chsp", 8_000.0),
            3 => ("private", 5_000.0),
            4 => ("tac", 20_000.0),
            _ if index % 7 == 0 => ("ndis", 30_000.0),
            _ => continue,
        };

        let package_id = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO funding_packages
                (id, organisation_id, client_contact_id, funding_source_id, name,
                 reference, start_date, end_date, budget_amount, status, notes)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, 'active', $10)",
        )
        .bind(package_id)
        .bind(organisation)
        .bind(client)
        .bind(source(source_key)?)
        .bind(format!("{source_key} package"))
        .bind(format!("DEMO-{source_key}-{index:04}"))
        .bind(today - Duration::days(30))
        .bind(today + Duration::days(335))
        .bind(budget)
        .bind("Seeded example funding package")
        .execute(&mut **tx)
        .await?;
        package_count += 1;

        let agreement_id = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO client_agreements
                (id, organisation_id, client_contact_id, funding_package_id, title,
                 status, quote_date, start_date, end_date, total_amount, notes)
             VALUES ($1, $2, $3, $4, $5, 'quoted', $6, $7, $8, 322, $9)",
        )
        .bind(agreement_id)
        .bind(organisation)
        .bind(client)
        .bind(package_id)
        .bind("Weekly personal care and community access")
        .bind(today)
        .bind(today)
        .bind(today + Duration::days(90))
        .bind("Seeded example quote; finalise it to generate draft shifts")
        .execute(&mut **tx)
        .await?;
        agreement_count += 1;

        let service = services.get(index % services.len()).copied();
        let site = sites.get(index % sites.len()).copied();
        sqlx::query(
            "INSERT INTO agreement_lines
                (id, organisation_id, agreement_id, service_id, site_id, description,
                 quantity, unit, unit_rate, amount, recurrence, weekdays,
                 start_time, duration_minutes)
             VALUES ($1, $2, $3, $4, $5, 'Personal care', 2, 'hour', 68, 136,
                     'weekly', ARRAY[1,3]::smallint[], $6, 120)",
        )
        .bind(Uuid::now_v7())
        .bind(organisation)
        .bind(agreement_id)
        .bind(service)
        .bind(site)
        .bind(NaiveTime::from_hms_opt(9, 0, 0).expect("valid time"))
        .execute(&mut **tx)
        .await?;

        sqlx::query(
            "INSERT INTO agreement_lines
                (id, organisation_id, agreement_id, service_id, site_id, description,
                 quantity, unit, unit_rate, amount, recurrence, weekdays,
                 start_time, duration_minutes)
             VALUES ($1, $2, $3, $4, $5, 'Community access', 3, 'hour', 62, 186,
                     'weekly', ARRAY[5]::smallint[], $6, 180)",
        )
        .bind(Uuid::now_v7())
        .bind(organisation)
        .bind(agreement_id)
        .bind(service)
        .bind(site)
        .bind(NaiveTime::from_hms_opt(13, 0, 0).expect("valid time"))
        .execute(&mut **tx)
        .await?;

        quoted.push(agreement_id);
    }

    Ok((package_count, agreement_count, quoted))
}

async fn insert_roster_constraints(
    tx: &mut Transaction<'_, Postgres>,
    organisation: Uuid,
    clients: &[Uuid],
    staff: &[Uuid],
    services: &[Uuid],
    sites: &[Uuid],
) -> anyhow::Result<()> {
    if clients.len() < 2 || staff.len() < 4 || services.is_empty() || sites.is_empty() {
        return Ok(());
    }

    struct ConstraintSeed {
        worker: Uuid,
        target_contact: Option<Uuid>,
        target_site: Option<Uuid>,
        target_service: Option<Uuid>,
        target_division: Option<Uuid>,
        kind: &'static str,
        strength: i16,
        reason: &'static str,
        source: &'static str,
    }

    let rows = [
        ConstraintSeed {
            worker: staff[0],
            target_contact: Some(clients[0]),
            target_site: None,
            target_service: None,
            target_division: None,
            kind: "preferred",
            strength: 4,
            reason: "Continuity requested by family",
            source: "client_request",
        },
        ConstraintSeed {
            worker: staff[1],
            target_contact: Some(clients[1]),
            target_site: None,
            target_service: None,
            target_division: None,
            kind: "avoid",
            strength: 3,
            reason: "Client prefers a different worker",
            source: "client_request",
        },
        ConstraintSeed {
            worker: staff[2],
            target_contact: Some(staff[3]),
            target_site: None,
            target_service: None,
            target_division: None,
            kind: "blocked",
            strength: 5,
            reason: "Do not roster these workers together",
            source: "hr",
        },
        ConstraintSeed {
            worker: staff[4 % staff.len()],
            target_contact: None,
            target_site: Some(sites[0]),
            target_service: None,
            target_division: None,
            kind: "preferred",
            strength: 2,
            reason: "Lives close to this site",
            source: "manual",
        },
        ConstraintSeed {
            worker: staff[5 % staff.len()],
            target_contact: None,
            target_site: None,
            target_service: Some(services[0]),
            target_division: None,
            kind: "preferred",
            strength: 2,
            reason: "Regular program lead",
            source: "manual",
        },
    ];

    for seed in rows {
        sqlx::query(
            "INSERT INTO roster_constraints
                (id, organisation_id, worker_contact_id, target_contact_id, target_site_id,
                 target_service_id, target_division_id, kind, strength, reason, source)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)",
        )
        .bind(Uuid::now_v7())
        .bind(organisation)
        .bind(seed.worker)
        .bind(seed.target_contact)
        .bind(seed.target_site)
        .bind(seed.target_service)
        .bind(seed.target_division)
        .bind(seed.kind)
        .bind(seed.strength)
        .bind(seed.reason)
        .bind(seed.source)
        .execute(&mut **tx)
        .await?;
    }

    Ok(())
}

/// Workers related to a client are blocked from rostering to them.
async fn insert_conflict_constraints(
    tx: &mut Transaction<'_, Postgres>,
    organisation: Uuid,
    conflicts: &[(Uuid, Uuid)],
) -> anyhow::Result<()> {
    for (worker, client) in conflicts {
        sqlx::query(
            "INSERT INTO roster_constraints
                (id, organisation_id, worker_contact_id, target_contact_id, kind, strength,
                 reason, source)
             VALUES ($1, $2, $3, $4, 'blocked', 5, $5, 'hr')",
        )
        .bind(Uuid::now_v7())
        .bind(organisation)
        .bind(worker)
        .bind(client)
        .bind("Family member: declared conflict of interest")
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

async fn insert_audit(
    tx: &mut Transaction<'_, Postgres>,
    organisation: Uuid,
    clients: usize,
    staff: usize,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO audit_log (organisation_id, action, entity_type, metadata)
         VALUES ($1, 'demo.seeded', 'organisation', $2)",
    )
    .bind(organisation)
    .bind(json!({ "clients": clients, "staff": staff }))
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn set_custom_value(
    tx: &mut Transaction<'_, Postgres>,
    organisation: Uuid,
    definition_id: Uuid,
    contact_id: Uuid,
    value: Value,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO custom_field_values
            (id, organisation_id, definition_id, entity_type, entity_id, value)
         VALUES ($1, $2, $3, 'contact', $4, $5)",
    )
    .bind(Uuid::now_v7())
    .bind(organisation)
    .bind(definition_id)
    .bind(contact_id)
    .bind(value)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

fn phone(rng: &mut StdRng) -> String {
    format!(
        "04{:02} {:03} {:03}",
        rng.gen_range(0..100),
        rng.gen_range(0..1000),
        rng.gen_range(0..1000)
    )
}

fn abn(rng: &mut StdRng) -> String {
    format!("{:011}", rng.gen_range(10_000_000_000u64..99_999_999_999))
}

fn utc_at(date: NaiveDate, hour: u32) -> Option<DateTime<Utc>> {
    let naive = date.and_hms_opt(hour, 0, 0)?;
    Some(Utc.from_utc_datetime(&naive))
}
