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
use chrono::{DateTime, Duration, NaiveDate, TimeZone, Utc};
use rand::{rngs::StdRng, Rng, SeedableRng};
use serde_json::{json, Value};
use sqlx::postgres::PgPoolOptions;
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

const SLUG: &str = "blue-dreams-care";
const EMAIL_DOMAIN: &str = "bluedreams.example";
const PASSWORD: &str = "BlueDreamsCare!2026";

const FIRST_NAMES: &[&str] = &[
    "Olivia",
    "Liam",
    "Amelia",
    "Noah",
    "Charlotte",
    "Oliver",
    "Mia",
    "Elijah",
    "Ava",
    "James",
    "Sophia",
    "William",
    "Isla",
    "Jack",
    "Grace",
    "Henry",
    "Chloe",
    "Thomas",
    "Ruby",
    "Ethan",
    "Zoe",
    "Lucas",
    "Willow",
    "Mason",
    "Harper",
    "Logan",
    "Ivy",
    "Samuel",
    "Matilda",
    "Hudson",
    "Sienna",
    "Archer",
    "Aria",
    "Levi",
    "Ella",
    "Hunter",
    "Amara",
    "Jasper",
    "Priya",
    "Aarav",
    "Mei",
    "Diego",
    "Fatima",
    "Omar",
    "Anika",
    "Sione",
    "Leilani",
    "Yusuf",
    "Nina",
    "Kai",
];

const LAST_NAMES: &[&str] = &[
    "Smith", "Nguyen", "Williams", "Brown", "Wilson", "Taylor", "Johnson", "White", "Martin",
    "Anderson", "Thompson", "Thomas", "Walker", "Harris", "Ryan", "Robinson", "Kelly", "Young",
    "King", "Wright", "Lee", "Patel", "Singh", "Chen", "Kim", "O'Brien", "Murphy", "Cooper",
    "Campbell", "Evans", "Turner", "Baker", "Hill", "Ward", "Morris", "Moore", "Clark", "Hall",
    "Green", "Adams", "Baker", "Nelson", "Carter", "Mitchell", "Roberts", "Phillips", "Campbell",
    "Parker", "Evans", "Edwards",
];

const SUBURBS: &[&str] = &[
    "Northcote",
    "Brunswick",
    "Preston",
    "Reservoir",
    "Thornbury",
    "Coburg",
    "Fairfield",
    "Heidelberg",
    "Epping",
    "Bundoora",
    "Greensborough",
    "Eltham",
    "Diamond Creek",
    "Watsonia",
    "Macleod",
    "Rosanna",
    "Ivanhoe",
    "Alphington",
    "Clifton Hill",
    "Fitzroy North",
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
    let cost_centres = insert_cost_centres(&mut tx, organisation, &divisions).await?;
    let services = insert_services(&mut tx, organisation, &divisions, &mut rng).await?;
    let custom_fields = insert_custom_fields(&mut tx, organisation).await?;

    let clients = insert_clients(&mut tx, organisation, &mut rng, &custom_fields).await?;
    let staff = insert_staff(&mut tx, organisation, &mut rng, &custom_fields).await?;
    let external = insert_external_organisations(&mut tx, organisation, &mut rng).await?;
    insert_client_networks(&mut tx, organisation, &clients, &external, &mut rng).await?;
    insert_rosters(
        &mut tx,
        organisation,
        &clients,
        &staff,
        &divisions,
        &mut rng,
    )
    .await?;

    insert_audit(&mut tx, organisation, clients.len(), staff.len()).await?;

    tx.commit().await?;

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

    println!("Blue Dreams Care seeded");
    println!("  organisation: {organisation} ({SLUG})");
    println!("  clients:      {}", clients.len());
    println!("  staff:        {}", staff.len());
    println!("  contacts:     {contact_count}");
    println!("  relationships:{relationship_count}");
    println!("  shifts:       {shift_count}");
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

async fn insert_clients(
    tx: &mut Transaction<'_, Postgres>,
    organisation: Uuid,
    rng: &mut StdRng,
    fields: &HashMap<String, Uuid>,
) -> anyhow::Result<Vec<Uuid>> {
    let mut clients = Vec::with_capacity(600);

    for index in 0..600 {
        let (given, family, display) = person_name(rng);
        let id = Uuid::now_v7();
        let suburb = SUBURBS[rng.gen_range(0..SUBURBS.len())];

        sqlx::query(
            "INSERT INTO contacts
                (id, organisation_id, display_name, given_name, family_name,
                 preferred_name, pronouns, date_of_birth, email, phone, notes, details)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
        )
        .bind(id)
        .bind(organisation)
        .bind(&display)
        .bind(&given)
        .bind(&family)
        .bind(if index % 7 == 0 { Some(given.as_str()) } else { None })
        .bind(if index % 5 == 0 { Some("they/them") } else { None })
        .bind(date_of_birth(rng, 1940, 2005))
        .bind(format!("{}.{}{}@{EMAIL_DOMAIN}", given.to_lowercase(), family.to_lowercase(), index))
        .bind(phone(rng))
        .bind(format!("Lives in {suburb}. NDIS participant (demo)."))
        .bind(json!({
            "contact_role": "client",
            "address": {
                "line1": format!("{} {} Street", rng.gen_range(1..200), STREETS[rng.gen_range(0..STREETS.len())]),
                "suburb": suburb,
                "state": "VIC",
                "postcode": "3070"
            },
            "funding": if index % 4 == 0 { "aged_care" } else { "ndis" }
        }))
        .execute(&mut **tx)
        .await?;

        set_custom_value(
            tx,
            organisation,
            fields["ndis_number"],
            id,
            json!(format!("{:09}", rng.gen_range(0..1_000_000_000u32))),
        )
        .await?;
        set_custom_value(
            tx,
            organisation,
            fields["plan_end_date"],
            id,
            json!((Utc::now().date_naive() + Duration::days(rng.gen_range(90..500))).to_string()),
        )
        .await?;
        let risk = ["low", "medium", "high"][rng.gen_range(0..3)];
        set_custom_value(tx, organisation, fields["risk_level"], id, json!(risk)).await?;

        clients.push(id);
    }

    Ok(clients)
}

async fn insert_staff(
    tx: &mut Transaction<'_, Postgres>,
    organisation: Uuid,
    rng: &mut StdRng,
    fields: &HashMap<String, Uuid>,
) -> anyhow::Result<Vec<Uuid>> {
    let mut staff = Vec::with_capacity(300);

    for index in 0..300 {
        let (given, family, display) = person_name(rng);
        let id = Uuid::now_v7();

        sqlx::query(
            "INSERT INTO contacts
                (id, organisation_id, display_name, given_name, family_name,
                 preferred_name, pronouns, date_of_birth, email, phone, notes, details)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
        )
        .bind(id)
        .bind(organisation)
        .bind(&display)
        .bind(&given)
        .bind(&family)
        .bind(if index % 4 == 0 {
            Some(given.as_str())
        } else {
            None
        })
        .bind(if index % 6 == 0 {
            Some("she/her")
        } else if index % 6 == 1 {
            Some("he/him")
        } else {
            None
        })
        .bind(date_of_birth(rng, 1970, 2003))
        .bind(format!(
            "staff.{}.{}{}@{EMAIL_DOMAIN}",
            given.to_lowercase(),
            family.to_lowercase(),
            index
        ))
        .bind(phone(rng))
        .bind("Support worker (demo).")
        .bind(json!({
            "contact_role": "worker",
            "employment": {
                "type": if index % 3 == 0 { "casual" } else { "permanent" },
                "start_date": "2024-01-15"
            }
        }))
        .execute(&mut **tx)
        .await?;

        sqlx::query(
            "INSERT INTO relationships
                (id, organisation_id, from_contact_id, to_organisation_id,
                 from_role, to_role)
             VALUES ($1, $2, $3, $4, 'employee', 'employer')",
        )
        .bind(Uuid::now_v7())
        .bind(organisation)
        .bind(id)
        .bind(organisation)
        .execute(&mut **tx)
        .await?;

        set_custom_value(
            tx,
            organisation,
            fields["employee_number"],
            id,
            json!(format!("EMP-{:04}", index + 1)),
        )
        .await?;

        staff.push(id);
    }

    Ok(staff)
}

#[derive(Default)]
struct Networking {
    support_coordinator: Vec<Uuid>,
    health: Vec<Uuid>,
    pharmacy: Vec<Uuid>,
    plan_manager: Vec<Uuid>,
    coordinator_contacts: Vec<Uuid>,
}

async fn insert_external_organisations(
    tx: &mut Transaction<'_, Postgres>,
    organisation: Uuid,
    rng: &mut StdRng,
) -> anyhow::Result<Networking> {
    let mut networking = Networking::default();

    for (name, kind) in EXTERNAL_ORGS {
        let id = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO organisations
                (id, tenant_id, name, kind, abn, email, phone, is_active)
             VALUES ($1, $2, $3, $4, $5, $6, $7, true)",
        )
        .bind(id)
        .bind(organisation)
        .bind(name)
        .bind(kind)
        .bind(abn(rng))
        .bind(format!(
            "info{}@{EMAIL_DOMAIN}",
            name.to_lowercase().replace([' ', '\''], "")
        ))
        .bind(phone(rng))
        .execute(&mut **tx)
        .await?;

        match *kind {
            "support_coordinator" => networking.support_coordinator.push(id),
            "health" => networking.health.push(id),
            "pharmacy" => networking.pharmacy.push(id),
            "plan_manager" => networking.plan_manager.push(id),
            _ => {}
        }

        for _ in 0..2 {
            let (given, family, display) = person_name(rng);
            let contact_id = Uuid::now_v7();
            sqlx::query(
                "INSERT INTO contacts
                    (id, organisation_id, display_name, given_name, family_name,
                     date_of_birth, email, phone, notes, details)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
            )
            .bind(contact_id)
            .bind(organisation)
            .bind(&display)
            .bind(&given)
            .bind(&family)
            .bind(date_of_birth(rng, 1970, 2000))
            .bind(format!(
                "contact.{}@{EMAIL_DOMAIN}",
                Uuid::now_v7().simple()
            ))
            .bind(phone(rng))
            .bind(format!("Contact at {name} (demo)."))
            .bind(json!({ "contact_role": "other", "subtype": "external", "organisation": name }))
            .execute(&mut **tx)
            .await?;

            sqlx::query(
                "INSERT INTO relationships
                    (id, organisation_id, from_contact_id, to_organisation_id,
                     from_role, to_role)
                 VALUES ($1, $2, $3, $4, 'representative', 'organisation')",
            )
            .bind(Uuid::now_v7())
            .bind(organisation)
            .bind(contact_id)
            .bind(id)
            .execute(&mut **tx)
            .await?;

            if *kind == "support_coordinator" {
                networking.coordinator_contacts.push(contact_id);
            }
        }
    }

    Ok(networking)
}

async fn insert_client_networks(
    tx: &mut Transaction<'_, Postgres>,
    organisation: Uuid,
    clients: &[Uuid],
    networking: &Networking,
    rng: &mut StdRng,
) -> anyhow::Result<()> {
    for (index, client) in clients.iter().enumerate() {
        // Family / guardian contact.
        let (given, family, display) = person_name(rng);
        let family_id = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO contacts
                (id, organisation_id, display_name, given_name, family_name,
                 date_of_birth, email, phone, notes, details)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
        )
        .bind(family_id)
        .bind(organisation)
        .bind(&display)
        .bind(&given)
        .bind(&family)
        .bind(date_of_birth(rng, 1945, 1995))
        .bind(format!(
            "network.{}{}@{EMAIL_DOMAIN}",
            index,
            Uuid::now_v7().simple()
        ))
        .bind(phone(rng))
        .bind("Family or guardian contact (demo).")
        .bind(json!({ "contact_role": "other", "subtype": "family" }))
        .execute(&mut **tx)
        .await?;

        let to_role = if index % 3 == 0 { "guardian" } else { "family" };
        add_contact_contact(
            tx,
            organisation,
            *client,
            family_id,
            "person_supported",
            to_role,
        )
        .await?;

        if index % 3 == 0 {
            let (g2, f2, d2) = person_name(rng);
            let emergency_id = Uuid::now_v7();
            sqlx::query(
                "INSERT INTO contacts
                    (id, organisation_id, display_name, given_name, family_name,
                     email, phone, notes, details)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
            )
            .bind(emergency_id)
            .bind(organisation)
            .bind(&d2)
            .bind(&g2)
            .bind(&f2)
            .bind(format!(
                "emergency.{}{}@{EMAIL_DOMAIN}",
                index,
                Uuid::now_v7().simple()
            ))
            .bind(phone(rng))
            .bind("Emergency contact (demo).")
            .bind(json!({ "contact_role": "other", "subtype": "emergency_contact" }))
            .execute(&mut **tx)
            .await?;
            add_contact_contact(
                tx,
                organisation,
                *client,
                emergency_id,
                "person_supported",
                "emergency_contact",
            )
            .await?;
        }

        // External supports.
        let coordinator =
            networking.support_coordinator[rng.gen_range(0..networking.support_coordinator.len())];
        add_contact_org(
            tx,
            organisation,
            *client,
            coordinator,
            "participant",
            "support_coordinator",
        )
        .await?;

        let doctor = networking.health[rng.gen_range(0..networking.health.len())];
        add_contact_org(
            tx,
            organisation,
            *client,
            doctor,
            "patient",
            "general_practitioner",
        )
        .await?;

        if index % 2 == 0 {
            let pharmacy = networking.pharmacy[rng.gen_range(0..networking.pharmacy.len())];
            add_contact_org(tx, organisation, *client, pharmacy, "customer", "pharmacy").await?;
        }

        if index % 5 < 2 {
            let plan_manager =
                networking.plan_manager[rng.gen_range(0..networking.plan_manager.len())];
            add_contact_org(
                tx,
                organisation,
                *client,
                plan_manager,
                "participant",
                "plan_manager",
            )
            .await?;
        }

        if index % 2 == 0 {
            let coordinator_contact = networking.coordinator_contacts
                [rng.gen_range(0..networking.coordinator_contacts.len())];
            add_contact_contact(
                tx,
                organisation,
                *client,
                coordinator_contact,
                "participant",
                "support_coordinator",
            )
            .await?;
        }
    }

    Ok(())
}

async fn insert_rosters(
    tx: &mut Transaction<'_, Postgres>,
    organisation: Uuid,
    clients: &[Uuid],
    staff: &[Uuid],
    divisions: &[Uuid],
    _rng: &mut StdRng,
) -> anyhow::Result<()> {
    let start = Utc::now().date_naive();
    let slots: [(u32, u32); 4] = [(7, 9), (9, 12), (13, 15), (15, 18)];
    let mut shift_index = 0usize;

    for day in 0..14 {
        let date = start + Duration::days(day);
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

            let shift_id = Uuid::now_v7();
            sqlx::query(
                "INSERT INTO shifts
                    (id, organisation_id, division_id, title, kind, starts_at, ends_at,
                     location, status)
                 VALUES ($1, $2, $3, $4, 'service_delivery', $5, $6, $7, 'published')",
            )
            .bind(shift_id)
            .bind(organisation)
            .bind(divisions[shift_index % divisions.len()])
            .bind(SERVICES[shift_index % SERVICES.len()].0)
            .bind(starts_at)
            .bind(ends_at)
            .bind(format!(
                "Client home, {}",
                SUBURBS[shift_index % SUBURBS.len()]
            ))
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
                 VALUES ($1, $2, $3, $4, $5, 'assigned')",
            )
            .bind(Uuid::now_v7())
            .bind(organisation)
            .bind(shift_id)
            .bind(session_id)
            .bind(client)
            .execute(&mut **tx)
            .await?;

            sqlx::query(
                "INSERT INTO worker_assignments
                    (id, organisation_id, shift_id, session_id, contact_id, status)
                 VALUES ($1, $2, $3, $4, $5, 'assigned')",
            )
            .bind(Uuid::now_v7())
            .bind(organisation)
            .bind(shift_id)
            .bind(session_id)
            .bind(worker)
            .execute(&mut **tx)
            .await?;

            shift_index += 1;
        }
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

async fn add_contact_contact(
    tx: &mut Transaction<'_, Postgres>,
    organisation: Uuid,
    from: Uuid,
    to: Uuid,
    from_role: &str,
    to_role: &str,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO relationships
            (id, organisation_id, from_contact_id, to_contact_id,
             from_role, to_role)
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(Uuid::now_v7())
    .bind(organisation)
    .bind(from)
    .bind(to)
    .bind(from_role)
    .bind(to_role)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn add_contact_org(
    tx: &mut Transaction<'_, Postgres>,
    organisation: Uuid,
    from: Uuid,
    to_org: Uuid,
    from_role: &str,
    to_role: &str,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO relationships
            (id, organisation_id, from_contact_id, to_organisation_id,
             from_role, to_role)
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(Uuid::now_v7())
    .bind(organisation)
    .bind(from)
    .bind(to_org)
    .bind(from_role)
    .bind(to_role)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

fn person_name(rng: &mut StdRng) -> (String, String, String) {
    let given = FIRST_NAMES[rng.gen_range(0..FIRST_NAMES.len())].to_owned();
    let family = LAST_NAMES[rng.gen_range(0..LAST_NAMES.len())].to_owned();
    let display = format!("{given} {family}");
    (given, family, display)
}

fn date_of_birth(rng: &mut StdRng, from_year: i32, to_year: i32) -> NaiveDate {
    let start = NaiveDate::from_ymd_opt(from_year, 1, 1).expect("valid date");
    let days = i64::from(to_year - from_year) * 365 + 364;
    start + Duration::days(rng.gen_range(0..days))
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
