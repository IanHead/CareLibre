//! Schema-level integration tests.
//!
//! These require a migrated PostgreSQL database and are marked `#[ignore]` so
//! the default `cargo test` stays hermetic. Run them with:
//!
//! ```sh
//! DATABASE_URL=postgres://... cargo test --workspace -- --ignored
//! ```
//!
//! Every test runs inside a transaction that is rolled back, so the database is
//! left untouched.

use serde_json::json;
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

async fn pool() -> PgPool {
    let url = std::env::var("DATABASE_URL")
        .expect("DATABASE_URL must be set for schema integration tests");
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .expect("connect to PostgreSQL");

    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("apply migrations");

    pool
}

async fn create_organisation(tx: &mut Transaction<'_, Postgres>) -> Uuid {
    let id = Uuid::now_v7();
    sqlx::query("INSERT INTO organisations (id, slug, name) VALUES ($1, $2, $3)")
        .bind(id)
        .bind(format!("test-{}", &id.simple().to_string()[..12]))
        .bind("Schema Test Org")
        .execute(&mut **tx)
        .await
        .expect("insert organisation");
    id
}

async fn create_contact(tx: &mut Transaction<'_, Postgres>, organisation: Uuid) -> Uuid {
    let id = Uuid::now_v7();
    sqlx::query("INSERT INTO contacts (id, organisation_id, display_name) VALUES ($1, $2, $3)")
        .bind(id)
        .bind(organisation)
        .bind("Test Contact")
        .execute(&mut **tx)
        .await
        .expect("insert contact");
    id
}

async fn create_shift(
    tx: &mut Transaction<'_, Postgres>,
    organisation: Uuid,
    division: Option<Uuid>,
) -> Uuid {
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO shifts (id, organisation_id, division_id, starts_at, ends_at)
         VALUES ($1, $2, $3, now(), now() + interval '1 hour')",
    )
    .bind(id)
    .bind(organisation)
    .bind(division)
    .execute(&mut **tx)
    .await
    .expect("insert shift");
    id
}

#[tokio::test]
#[ignore = "requires a migrated PostgreSQL database (DATABASE_URL)"]
async fn cost_centre_inheritance_follows_configured_rules() {
    let pool = pool().await;
    let mut tx = pool.begin().await.unwrap();

    let organisation = create_organisation(&mut tx).await;
    let parent_cost_centre = Uuid::now_v7();
    let child_cost_centre = Uuid::now_v7();

    sqlx::query(
        "INSERT INTO cost_centres (id, organisation_id, code, name) VALUES ($1, $2, 'CC-100', 'Programs')",
    )
    .bind(parent_cost_centre)
    .bind(organisation)
    .execute(&mut *tx)
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO cost_centres (id, organisation_id, parent_id, code, name)
         VALUES ($1, $2, $3, 'CC-110', 'Respite')",
    )
    .bind(child_cost_centre)
    .bind(organisation)
    .bind(parent_cost_centre)
    .execute(&mut *tx)
    .await
    .unwrap();

    let division = Uuid::now_v7();
    sqlx::query("INSERT INTO divisions (id, organisation_id, name) VALUES ($1, $2, 'North')")
        .bind(division)
        .bind(organisation)
        .execute(&mut *tx)
        .await
        .unwrap();

    // The division carries the parent cost centre.
    sqlx::query(
        "INSERT INTO cost_centre_assignments (id, organisation_id, cost_centre_id, entity_type, entity_id)
         VALUES ($1, $2, $3, 'division', $4)",
    )
    .bind(Uuid::now_v7())
    .bind(organisation)
    .bind(parent_cost_centre)
    .bind(division)
    .execute(&mut *tx)
    .await
    .unwrap();

    let shift = create_shift(&mut tx, organisation, Some(division)).await;

    sqlx::query(
        "INSERT INTO record_parents (organisation_id, child_type, child_id, parent_type, parent_id)
         VALUES ($1, 'shift', $2, 'division', $3)",
    )
    .bind(organisation)
    .bind(shift)
    .bind(division)
    .execute(&mut *tx)
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO cost_centre_inheritance_rules (organisation_id, entity_type, inherits_from, priority)
         VALUES ($1, 'shift', 'division', 1)",
    )
    .bind(organisation)
    .execute(&mut *tx)
    .await
    .unwrap();

    let inherited: Option<Uuid> =
        sqlx::query_scalar("SELECT carelibre_resolve_cost_centre($1, 'shift', $2)")
            .bind(organisation)
            .bind(shift)
            .fetch_one(&mut *tx)
            .await
            .unwrap();

    assert_eq!(
        inherited,
        Some(parent_cost_centre),
        "shift should inherit the division's cost centre"
    );

    // A direct assignment always wins over inheritance.
    sqlx::query(
        "INSERT INTO cost_centre_assignments (id, organisation_id, cost_centre_id, entity_type, entity_id)
         VALUES ($1, $2, $3, 'shift', $4)",
    )
    .bind(Uuid::now_v7())
    .bind(organisation)
    .bind(child_cost_centre)
    .bind(shift)
    .execute(&mut *tx)
    .await
    .unwrap();

    let direct: Option<Uuid> =
        sqlx::query_scalar("SELECT carelibre_resolve_cost_centre($1, 'shift', $2)")
            .bind(organisation)
            .bind(shift)
            .fetch_one(&mut *tx)
            .await
            .unwrap();

    assert_eq!(direct, Some(child_cost_centre));

    tx.rollback().await.unwrap();
}

#[tokio::test]
#[ignore = "requires a migrated PostgreSQL database (DATABASE_URL)"]
async fn rate_versions_cannot_overlap() {
    let pool = pool().await;
    let mut tx = pool.begin().await.unwrap();

    let organisation = create_organisation(&mut tx).await;

    let service = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO services (id, organisation_id, code, name, funding_scheme)
         VALUES ($1, $2, 'SVC-1', 'Personal care', 'ndis')",
    )
    .bind(service)
    .bind(organisation)
    .execute(&mut *tx)
    .await
    .unwrap();

    let rate = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO rates (id, organisation_id, service_id, name, unit_of_measure)
         VALUES ($1, $2, $3, 'Standard', 'hour')",
    )
    .bind(rate)
    .bind(organisation)
    .bind(service)
    .execute(&mut *tx)
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO rate_versions (id, organisation_id, rate_id, amount, effective_from, effective_to)
         VALUES ($1, $2, $3, 98.65, '2026-07-01', '2027-07-01')",
    )
    .bind(Uuid::now_v7())
    .bind(organisation)
    .bind(rate)
    .execute(&mut *tx)
    .await
    .unwrap();

    // Overlapping range must be rejected by the trigger.
    sqlx::query("SAVEPOINT rate_overlap")
        .execute(&mut *tx)
        .await
        .unwrap();
    let overlapping = sqlx::query(
        "INSERT INTO rate_versions (id, organisation_id, rate_id, amount, effective_from, effective_to)
         VALUES ($1, $2, $3, 105.00, '2027-01-01', '2028-01-01')",
    )
    .bind(Uuid::now_v7())
    .bind(organisation)
    .bind(rate)
    .execute(&mut *tx)
    .await;
    assert!(overlapping.is_err(), "overlapping rate version must fail");
    sqlx::query("ROLLBACK TO SAVEPOINT rate_overlap")
        .execute(&mut *tx)
        .await
        .unwrap();

    // Adjacent range (half-open intervals) is fine.
    sqlx::query(
        "INSERT INTO rate_versions (id, organisation_id, rate_id, amount, effective_from, effective_to)
         VALUES ($1, $2, $3, 105.00, '2027-07-01', '2028-07-01')",
    )
    .bind(Uuid::now_v7())
    .bind(organisation)
    .bind(rate)
    .execute(&mut *tx)
    .await
    .expect("adjacent rate version should be accepted");

    tx.rollback().await.unwrap();
}

#[tokio::test]
#[ignore = "requires a migrated PostgreSQL database (DATABASE_URL)"]
async fn relationships_cannot_cross_tenants() {
    let pool = pool().await;
    let mut tx = pool.begin().await.unwrap();

    let organisation_a = create_organisation(&mut tx).await;
    let organisation_b = create_organisation(&mut tx).await;

    let contact_a = create_contact(&mut tx, organisation_a).await;
    let contact_b = create_contact(&mut tx, organisation_b).await;

    // Contact from A, relationship owned by B: must be rejected by the
    // composite (organisation_id, contact_id) foreign key.
    let cross_tenant = sqlx::query(
        "INSERT INTO relationships
            (id, organisation_id, from_contact_id, to_contact_id, from_role, to_role)
         VALUES ($1, $2, $3, $4, 'broker', 'client')",
    )
    .bind(Uuid::now_v7())
    .bind(organisation_b)
    .bind(contact_a)
    .bind(contact_b)
    .execute(&mut *tx)
    .await;

    assert!(
        cross_tenant.is_err(),
        "relationship must not reference a contact from another tenant"
    );

    tx.rollback().await.unwrap();
}

#[tokio::test]
#[ignore = "requires a migrated PostgreSQL database (DATABASE_URL)"]
async fn organisations_are_unified_and_linked_by_relationships() {
    let pool = pool().await;
    let mut tx = pool.begin().await.unwrap();

    let tenant = create_organisation(&mut tx).await;

    // An external counterparty is just another row in `organisations`, scoped
    // to the tenant, and needs no slug of its own.
    let partner = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO organisations (id, tenant_id, name, kind)
         VALUES ($1, $2, 'Broker Co', 'broker')",
    )
    .bind(partner)
    .bind(tenant)
    .execute(&mut *tx)
    .await
    .expect("external organisation is a row in organisations");

    let tenant_tenant: Uuid =
        sqlx::query_scalar("SELECT tenant_id FROM organisations WHERE id = $1")
            .bind(tenant)
            .fetch_one(&mut *tx)
            .await
            .unwrap();
    assert_eq!(tenant_tenant, tenant, "tenant root points at itself");

    // Internal (tenant) and external organisations are linked by relationships.
    sqlx::query(
        "INSERT INTO relationships
            (id, organisation_id, from_organisation_id, to_organisation_id, from_role, to_role)
         VALUES ($1, $2, $3, $4, 'referral_partner', 'referred_by')",
    )
    .bind(Uuid::now_v7())
    .bind(tenant)
    .bind(tenant)
    .bind(partner)
    .execute(&mut *tx)
    .await
    .expect("relationship between internal and external organisations");

    // An organisation from another tenant cannot be an endpoint.
    let other_tenant = create_organisation(&mut tx).await;

    sqlx::query("SAVEPOINT foreign_org")
        .execute(&mut *tx)
        .await
        .unwrap();
    let cross_tenant = sqlx::query(
        "INSERT INTO relationships
            (id, organisation_id, from_organisation_id, to_organisation_id, from_role, to_role)
         VALUES ($1, $2, $3, $4, 'referral_partner', 'referred_by')",
    )
    .bind(Uuid::now_v7())
    .bind(tenant)
    .bind(tenant)
    .bind(other_tenant)
    .execute(&mut *tx)
    .await;
    assert!(
        cross_tenant.is_err(),
        "relationship must not reference an organisation from another tenant"
    );
    sqlx::query("ROLLBACK TO SAVEPOINT foreign_org")
        .execute(&mut *tx)
        .await
        .unwrap();

    tx.rollback().await.unwrap();
}

async fn care_roles(tx: &mut Transaction<'_, Postgres>, contact: Uuid) -> (bool, bool) {
    sqlx::query_as("SELECT is_client, is_worker FROM contact_care_roles WHERE contact_id = $1")
        .bind(contact)
        .fetch_one(&mut **tx)
        .await
        .expect("care roles")
}

#[tokio::test]
#[ignore = "requires a migrated PostgreSQL database (DATABASE_URL)"]
async fn clients_and_workers_come_from_relationships_with_the_organisation() {
    let pool = pool().await;
    let mut tx = pool.begin().await.unwrap();

    let tenant = create_organisation(&mut tx).await;
    let client = create_contact(&mut tx, tenant).await;
    let worker = create_contact(&mut tx, tenant).await;
    let family = create_contact(&mut tx, tenant).await;
    let former = create_contact(&mut tx, tenant).await;

    assert_eq!(care_roles(&mut tx, client).await, (false, false));

    // Contact -> organisation: the contact receives care from the organisation.
    sqlx::query(
        "INSERT INTO relationships
            (id, organisation_id, from_contact_id, to_organisation_id, from_role, to_role)
         VALUES ($1, $2, $3, $2, 'care_recipient', 'provider')",
    )
    .bind(Uuid::now_v7())
    .bind(tenant)
    .bind(client)
    .execute(&mut *tx)
    .await
    .unwrap();

    // Organisation -> contact works too, because relationships are two-sided.
    sqlx::query(
        "INSERT INTO relationships
            (id, organisation_id, from_organisation_id, to_contact_id, from_role, to_role)
         VALUES ($1, $2, $2, $3, 'employer', 'employee')",
    )
    .bind(Uuid::now_v7())
    .bind(tenant)
    .bind(worker)
    .execute(&mut *tx)
    .await
    .unwrap();

    // A care relationship with another contact does not make a client.
    sqlx::query(
        "INSERT INTO relationships
            (id, organisation_id, from_contact_id, to_contact_id, from_role, to_role)
         VALUES ($1, $2, $3, $4, 'care_recipient', 'provider')",
    )
    .bind(Uuid::now_v7())
    .bind(tenant)
    .bind(family)
    .bind(client)
    .execute(&mut *tx)
    .await
    .unwrap();

    // An ended relationship no longer counts.
    sqlx::query(
        "INSERT INTO relationships
            (id, organisation_id, from_contact_id, to_organisation_id, from_role, to_role,
             start_date, end_date, end_reason)
         VALUES ($1, $2, $3, $2, 'care_recipient', 'provider',
                 CURRENT_DATE - 30, CURRENT_DATE - 1, 'plan_ended')",
    )
    .bind(Uuid::now_v7())
    .bind(tenant)
    .bind(former)
    .execute(&mut *tx)
    .await
    .unwrap();

    assert_eq!(care_roles(&mut tx, client).await, (true, false));
    assert_eq!(care_roles(&mut tx, worker).await, (false, true));
    assert_eq!(care_roles(&mut tx, family).await, (false, false));
    assert_eq!(care_roles(&mut tx, former).await, (false, false));

    tx.rollback().await.unwrap();
}

#[tokio::test]
#[ignore = "requires a migrated PostgreSQL database (DATABASE_URL)"]
async fn assignments_are_consistent_and_contacts_are_fluid() {
    let pool = pool().await;
    let mut tx = pool.begin().await.unwrap();

    let organisation = create_organisation(&mut tx).await;
    let shift_one = create_shift(&mut tx, organisation, None).await;
    let shift_two = create_shift(&mut tx, organisation, None).await;

    let session_one = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO shift_sessions (id, organisation_id, shift_id, starts_at, ends_at)
         VALUES ($1, $2, $3, now(), now() + interval '30 minutes')",
    )
    .bind(session_one)
    .bind(organisation)
    .bind(shift_one)
    .execute(&mut *tx)
    .await
    .unwrap();

    let person = create_contact(&mut tx, organisation).await;

    // A session that belongs to a different shift must be rejected.
    sqlx::query("SAVEPOINT mismatch")
        .execute(&mut *tx)
        .await
        .unwrap();
    let mismatched = sqlx::query(
        "INSERT INTO client_assignments (id, organisation_id, shift_id, session_id, contact_id)
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(Uuid::now_v7())
    .bind(organisation)
    .bind(shift_two)
    .bind(session_one)
    .bind(person)
    .execute(&mut *tx)
    .await;
    assert!(
        mismatched.is_err(),
        "assignment session must belong to the same shift"
    );
    sqlx::query("ROLLBACK TO SAVEPOINT mismatch")
        .execute(&mut *tx)
        .await
        .unwrap();

    // Correct pairing works.
    sqlx::query(
        "INSERT INTO client_assignments (id, organisation_id, shift_id, session_id, contact_id)
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(Uuid::now_v7())
    .bind(organisation)
    .bind(shift_one)
    .bind(session_one)
    .bind(person)
    .execute(&mut *tx)
    .await
    .expect("assignment to matching session");

    // The same contact can also be a worker: contacts are untyped.
    sqlx::query(
        "INSERT INTO worker_assignments (id, organisation_id, shift_id, contact_id)
         VALUES ($1, $2, $3, $4)",
    )
    .bind(Uuid::now_v7())
    .bind(organisation)
    .bind(shift_two)
    .bind(person)
    .execute(&mut *tx)
    .await
    .expect("same contact may be assigned as a worker");

    // Duplicate assignment of the same contact to the same shift is rejected.
    sqlx::query(
        "INSERT INTO client_assignments (id, organisation_id, shift_id, contact_id)
         VALUES ($1, $2, $3, $4)",
    )
    .bind(Uuid::now_v7())
    .bind(organisation)
    .bind(shift_one)
    .bind(person)
    .execute(&mut *tx)
    .await
    .expect("first direct shift assignment");

    sqlx::query("SAVEPOINT duplicate")
        .execute(&mut *tx)
        .await
        .unwrap();
    let duplicate = sqlx::query(
        "INSERT INTO client_assignments (id, organisation_id, shift_id, contact_id)
         VALUES ($1, $2, $3, $4)",
    )
    .bind(Uuid::now_v7())
    .bind(organisation)
    .bind(shift_one)
    .bind(person)
    .execute(&mut *tx)
    .await;
    assert!(duplicate.is_err(), "duplicate shift assignment must fail");
    sqlx::query("ROLLBACK TO SAVEPOINT duplicate")
        .execute(&mut *tx)
        .await
        .unwrap();

    tx.rollback().await.unwrap();
}

async fn create_custom_field(
    tx: &mut Transaction<'_, Postgres>,
    organisation: Uuid,
    entity_type: &str,
    key: &str,
    data_type: &str,
    options: Option<serde_json::Value>,
) -> Uuid {
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO custom_field_definitions
            (id, organisation_id, entity_type, key, label, data_type, options)
         VALUES ($1, $2, $3, $4, 'Test field', $5, $6)",
    )
    .bind(id)
    .bind(organisation)
    .bind(entity_type)
    .bind(key)
    .bind(data_type)
    .bind(options)
    .execute(&mut **tx)
    .await
    .expect("insert custom field definition");
    id
}

#[tokio::test]
#[ignore = "requires a migrated PostgreSQL database (DATABASE_URL)"]
async fn custom_field_values_are_type_checked_and_shape_is_locked() {
    let pool = pool().await;
    let mut tx = pool.begin().await.unwrap();

    let organisation = create_organisation(&mut tx).await;
    let definition = create_custom_field(
        &mut tx,
        organisation,
        "contact",
        "ndis_number",
        "text",
        None,
    )
    .await;
    let entity = Uuid::now_v7();

    sqlx::query(
        "INSERT INTO custom_field_values
            (id, organisation_id, definition_id, entity_type, entity_id, value)
         VALUES ($1, $2, $3, 'contact', $4, $5)",
    )
    .bind(Uuid::now_v7())
    .bind(organisation)
    .bind(definition)
    .bind(entity)
    .bind(json!("123456789"))
    .execute(&mut *tx)
    .await
    .expect("text value accepted");

    // A number is wrong for a text field.
    sqlx::query("SAVEPOINT wrong_type")
        .execute(&mut *tx)
        .await
        .unwrap();
    let wrong_type = sqlx::query(
        "INSERT INTO custom_field_values
            (id, organisation_id, definition_id, entity_type, entity_id, value)
         VALUES ($1, $2, $3, 'contact', $4, $5)",
    )
    .bind(Uuid::now_v7())
    .bind(organisation)
    .bind(definition)
    .bind(entity)
    .bind(json!(123))
    .execute(&mut *tx)
    .await;
    assert!(wrong_type.is_err(), "number must be rejected for text");
    sqlx::query("ROLLBACK TO SAVEPOINT wrong_type")
        .execute(&mut *tx)
        .await
        .unwrap();

    // Data type cannot change while values exist.
    sqlx::query("SAVEPOINT shape_change")
        .execute(&mut *tx)
        .await
        .unwrap();
    let shape_change =
        sqlx::query("UPDATE custom_field_definitions SET data_type = 'integer' WHERE id = $1")
            .bind(definition)
            .execute(&mut *tx)
            .await;
    assert!(
        shape_change.is_err(),
        "data type must be immutable once values exist"
    );
    sqlx::query("ROLLBACK TO SAVEPOINT shape_change")
        .execute(&mut *tx)
        .await
        .unwrap();

    // Deleting the definition removes its values.
    sqlx::query("DELETE FROM custom_field_definitions WHERE id = $1")
        .bind(definition)
        .execute(&mut *tx)
        .await
        .unwrap();
    let remaining: i64 =
        sqlx::query_scalar("SELECT count(*) FROM custom_field_values WHERE definition_id = $1")
            .bind(definition)
            .fetch_one(&mut *tx)
            .await
            .unwrap();
    assert_eq!(remaining, 0, "values cascade with the definition");

    tx.rollback().await.unwrap();
}

#[tokio::test]
#[ignore = "requires a migrated PostgreSQL database (DATABASE_URL)"]
async fn custom_field_select_enforces_options_and_entity_type() {
    let pool = pool().await;
    let mut tx = pool.begin().await.unwrap();

    let organisation = create_organisation(&mut tx).await;

    // Choice fields must declare options.
    sqlx::query("SAVEPOINT no_options")
        .execute(&mut *tx)
        .await
        .unwrap();
    let missing_options = create_custom_field_try(
        &mut tx,
        organisation,
        "contact",
        "contact_preference",
        "select",
        None,
    )
    .await;
    assert!(missing_options.is_err(), "select requires options");
    sqlx::query("ROLLBACK TO SAVEPOINT no_options")
        .execute(&mut *tx)
        .await
        .unwrap();

    let definition = create_custom_field(
        &mut tx,
        organisation,
        "contact",
        "contact_preference",
        "select",
        Some(json!(["phone", "email"])),
    )
    .await;

    let entity = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO custom_field_values
            (id, organisation_id, definition_id, entity_type, entity_id, value)
         VALUES ($1, $2, $3, 'contact', $4, $5)",
    )
    .bind(Uuid::now_v7())
    .bind(organisation)
    .bind(definition)
    .bind(entity)
    .bind(json!("phone"))
    .execute(&mut *tx)
    .await
    .expect("configured option accepted");

    sqlx::query("SAVEPOINT bad_option")
        .execute(&mut *tx)
        .await
        .unwrap();
    let bad_option = sqlx::query(
        "INSERT INTO custom_field_values
            (id, organisation_id, definition_id, entity_type, entity_id, value)
         VALUES ($1, $2, $3, 'contact', $4, $5)",
    )
    .bind(Uuid::now_v7())
    .bind(organisation)
    .bind(definition)
    .bind(Uuid::now_v7())
    .bind(json!("carrier_pigeon"))
    .execute(&mut *tx)
    .await;
    assert!(bad_option.is_err(), "value outside options must fail");
    sqlx::query("ROLLBACK TO SAVEPOINT bad_option")
        .execute(&mut *tx)
        .await
        .unwrap();

    // A field defined for contacts cannot be attached to a shift.
    sqlx::query("SAVEPOINT wrong_entity")
        .execute(&mut *tx)
        .await
        .unwrap();
    let wrong_entity = sqlx::query(
        "INSERT INTO custom_field_values
            (id, organisation_id, definition_id, entity_type, entity_id, value)
         VALUES ($1, $2, $3, 'shift', $4, $5)",
    )
    .bind(Uuid::now_v7())
    .bind(organisation)
    .bind(definition)
    .bind(Uuid::now_v7())
    .bind(json!("phone"))
    .execute(&mut *tx)
    .await;
    assert!(
        wrong_entity.is_err(),
        "entity type must match the definition"
    );
    sqlx::query("ROLLBACK TO SAVEPOINT wrong_entity")
        .execute(&mut *tx)
        .await
        .unwrap();

    tx.rollback().await.unwrap();
}

async fn create_custom_field_try(
    tx: &mut Transaction<'_, Postgres>,
    organisation: Uuid,
    entity_type: &str,
    key: &str,
    data_type: &str,
    options: Option<serde_json::Value>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO custom_field_definitions
            (id, organisation_id, entity_type, key, label, data_type, options)
         VALUES ($1, $2, $3, $4, 'Test field', $5, $6)",
    )
    .bind(Uuid::now_v7())
    .bind(organisation)
    .bind(entity_type)
    .bind(key)
    .bind(data_type)
    .bind(options)
    .execute(&mut **tx)
    .await
    .map(|_| ())
}

async fn create_user(
    tx: &mut Transaction<'_, Postgres>,
    organisation: Uuid,
    email: &str,
    is_owner: bool,
) -> Uuid {
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO users (id, email, password_hash, full_name)
         VALUES ($1, $2, 'not-a-real-hash', 'Test User')",
    )
    .bind(id)
    .bind(format!("{email}-{id}@example.com"))
    .execute(&mut **tx)
    .await
    .expect("insert user");

    sqlx::query(
        "INSERT INTO organisation_users (organisation_id, user_id, role, is_owner)
         VALUES ($1, $2, $3, $4)",
    )
    .bind(organisation)
    .bind(id)
    .bind(if is_owner { "owner" } else { "staff" })
    .bind(is_owner)
    .execute(&mut **tx)
    .await
    .expect("insert organisation user");

    id
}

async fn set_id(tx: &mut Transaction<'_, Postgres>, organisation: Uuid, persona: &str) -> Uuid {
    sqlx::query_scalar(
        "SELECT id FROM permission_sets WHERE organisation_id = $1 AND persona_key = $2",
    )
    .bind(organisation)
    .bind(persona)
    .fetch_one(&mut **tx)
    .await
    .expect("persona exists")
}

#[tokio::test]
#[ignore = "requires a migrated PostgreSQL database (DATABASE_URL)"]
async fn permission_sets_are_seeded_for_new_organisations() {
    let pool = pool().await;
    let mut tx = pool.begin().await.unwrap();

    let organisation = create_organisation(&mut tx).await;

    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM permission_sets WHERE organisation_id = $1")
            .bind(organisation)
            .fetch_one(&mut *tx)
            .await
            .unwrap();
    assert_eq!(count, 7, "seven OOTB personas are seeded");

    for persona in [
        "rostering",
        "intake",
        "human_resources",
        "senior_management",
        "house_manager",
        "finance",
        "administrator",
    ] {
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM permission_sets WHERE organisation_id = $1 AND persona_key = $2)",
        )
        .bind(organisation)
        .bind(persona)
        .fetch_one(&mut *tx)
        .await
        .unwrap();
        assert!(exists, "persona {persona} is seeded");
    }

    let (editable, capabilities): (bool, i64) = sqlx::query_as(
        "SELECT ps.is_editable,
                (SELECT count(*) FROM permission_set_capabilities c
                 WHERE c.permission_set_id = ps.id)
         FROM permission_sets ps
         WHERE ps.organisation_id = $1 AND ps.persona_key = 'administrator'",
    )
    .bind(organisation)
    .fetch_one(&mut *tx)
    .await
    .unwrap();

    let total_capabilities: i64 = sqlx::query_scalar("SELECT count(*) FROM capabilities")
        .fetch_one(&mut *tx)
        .await
        .unwrap();

    assert!(!editable, "Administrator is immutable");
    assert_eq!(capabilities, total_capabilities);

    tx.rollback().await.unwrap();
}

#[tokio::test]
#[ignore = "requires a migrated PostgreSQL database (DATABASE_URL)"]
async fn permission_sets_are_guarded_and_additive() {
    let pool = pool().await;
    let mut tx = pool.begin().await.unwrap();

    let organisation = create_organisation(&mut tx).await;
    let owner = create_user(&mut tx, organisation, "owner", true).await;
    let worker = create_user(&mut tx, organisation, "worker", false).await;

    // The last owner cannot be demoted.
    sqlx::query("SAVEPOINT demote_owner")
        .execute(&mut *tx)
        .await
        .unwrap();
    let demote = sqlx::query("UPDATE organisation_users SET is_owner = false WHERE user_id = $1")
        .bind(owner)
        .execute(&mut *tx)
        .await;
    assert!(demote.is_err(), "last owner cannot be demoted");
    sqlx::query("ROLLBACK TO SAVEPOINT demote_owner")
        .execute(&mut *tx)
        .await
        .unwrap();

    // Administrator cannot be renamed or given/removed capabilities.
    let administrator = set_id(&mut tx, organisation, "administrator").await;

    sqlx::query("SAVEPOINT rename_admin")
        .execute(&mut *tx)
        .await
        .unwrap();
    let rename = sqlx::query("UPDATE permission_sets SET name = 'Everything' WHERE id = $1")
        .bind(administrator)
        .execute(&mut *tx)
        .await;
    assert!(rename.is_err(), "Administrator cannot be renamed");
    sqlx::query("ROLLBACK TO SAVEPOINT rename_admin")
        .execute(&mut *tx)
        .await
        .unwrap();

    sqlx::query("SAVEPOINT drop_admin_cap")
        .execute(&mut *tx)
        .await
        .unwrap();
    let drop_cap = sqlx::query(
        "DELETE FROM permission_set_capabilities
         WHERE permission_set_id = $1 AND capability = 'contacts.read'",
    )
    .bind(administrator)
    .execute(&mut *tx)
    .await;
    assert!(
        drop_cap.is_err(),
        "Administrator capabilities are immutable"
    );
    sqlx::query("ROLLBACK TO SAVEPOINT drop_admin_cap")
        .execute(&mut *tx)
        .await
        .unwrap();

    // Additive assignment: Rostering + Finance yields more than either alone.
    let rostering = set_id(&mut tx, organisation, "rostering").await;
    let finance = set_id(&mut tx, organisation, "finance").await;

    for set in [rostering, finance] {
        sqlx::query(
            "INSERT INTO organisation_user_permission_sets
                (organisation_id, user_id, permission_set_id)
             VALUES ($1, $2, $3)",
        )
        .bind(organisation)
        .bind(worker)
        .bind(set)
        .execute(&mut *tx)
        .await
        .unwrap();
    }

    let rostering_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM permission_set_capabilities WHERE permission_set_id = $1",
    )
    .bind(rostering)
    .fetch_one(&mut *tx)
    .await
    .unwrap();

    let union_count: i64 = sqlx::query_scalar(
        "SELECT count(DISTINCT c.capability)
         FROM organisation_user_permission_sets oup
         JOIN permission_set_capabilities c ON c.permission_set_id = oup.permission_set_id
         WHERE oup.organisation_id = $1 AND oup.user_id = $2",
    )
    .bind(organisation)
    .bind(worker)
    .fetch_one(&mut *tx)
    .await
    .unwrap();

    assert!(
        union_count > rostering_count,
        "adding Finance must add capabilities"
    );

    tx.rollback().await.unwrap();
}

#[tokio::test]
#[ignore = "requires a migrated PostgreSQL database (DATABASE_URL)"]
async fn permission_sets_cannot_cross_tenants() {
    let pool = pool().await;
    let mut tx = pool.begin().await.unwrap();

    let organisation_a = create_organisation(&mut tx).await;
    let organisation_b = create_organisation(&mut tx).await;

    let user_a = create_user(&mut tx, organisation_a, "cross", false).await;
    let set_b = set_id(&mut tx, organisation_b, "rostering").await;

    let cross = sqlx::query(
        "INSERT INTO organisation_user_permission_sets
            (organisation_id, user_id, permission_set_id)
         VALUES ($1, $2, $3)",
    )
    .bind(organisation_a)
    .bind(user_a)
    .bind(set_b)
    .execute(&mut *tx)
    .await;

    assert!(cross.is_err(), "permission sets are tenant-scoped");

    tx.rollback().await.unwrap();
}

#[tokio::test]
#[ignore = "requires a migrated PostgreSQL database (DATABASE_URL)"]
async fn invitations_are_single_use_and_tenant_scoped() {
    let pool = pool().await;
    let mut tx = pool.begin().await.unwrap();

    let organisation = create_organisation(&mut tx).await;
    let owner = create_user(&mut tx, organisation, "inviter", true).await;
    let set = set_id(&mut tx, organisation, "house_manager").await;

    let first = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO invitations
            (id, organisation_id, email, token_hash, expires_at, created_by)
         VALUES ($1, $2, 'invitee@example.com', 'hash-one', now() + interval '7 days', $3)",
    )
    .bind(first)
    .bind(organisation)
    .bind(owner)
    .execute(&mut *tx)
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO invitation_permission_sets
            (organisation_id, invitation_id, permission_set_id)
         VALUES ($1, $2, $3)",
    )
    .bind(organisation)
    .bind(first)
    .bind(set)
    .execute(&mut *tx)
    .await
    .unwrap();

    // Second pending invitation for the same email is rejected.
    sqlx::query("SAVEPOINT duplicate_invite")
        .execute(&mut *tx)
        .await
        .unwrap();
    let duplicate = sqlx::query(
        "INSERT INTO invitations
            (id, organisation_id, email, token_hash, expires_at)
         VALUES ($1, $2, 'INVITEE@example.com', 'hash-two', now() + interval '7 days')",
    )
    .bind(Uuid::now_v7())
    .bind(organisation)
    .execute(&mut *tx)
    .await;
    assert!(duplicate.is_err(), "one pending invitation per email");
    sqlx::query("ROLLBACK TO SAVEPOINT duplicate_invite")
        .execute(&mut *tx)
        .await
        .unwrap();

    // Once accepted, the email can be invited again.
    sqlx::query("UPDATE invitations SET accepted_at = now(), accepted_user_id = $2 WHERE id = $1")
        .bind(first)
        .bind(owner)
        .execute(&mut *tx)
        .await
        .unwrap();

    sqlx::query(
        "INSERT INTO invitations
            (id, organisation_id, email, token_hash, expires_at)
         VALUES ($1, $2, 'invitee@example.com', 'hash-three', now() + interval '7 days')",
    )
    .bind(Uuid::now_v7())
    .bind(organisation)
    .execute(&mut *tx)
    .await
    .expect("accepted invitation frees the email");

    // Invitation sets cannot cross tenants.
    let other_organisation = create_organisation(&mut tx).await;
    let other_set = set_id(&mut tx, other_organisation, "house_manager").await;
    let third = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO invitations
            (id, organisation_id, email, token_hash, expires_at)
         VALUES ($1, $2, 'third@example.com', 'hash-four', now() + interval '7 days')",
    )
    .bind(third)
    .bind(organisation)
    .execute(&mut *tx)
    .await
    .unwrap();

    sqlx::query("SAVEPOINT cross_tenant_set")
        .execute(&mut *tx)
        .await
        .unwrap();
    let cross = sqlx::query(
        "INSERT INTO invitation_permission_sets
            (organisation_id, invitation_id, permission_set_id)
         VALUES ($1, $2, $3)",
    )
    .bind(organisation)
    .bind(third)
    .bind(other_set)
    .execute(&mut *tx)
    .await;
    assert!(cross.is_err(), "invitation sets are tenant-scoped");
    sqlx::query("ROLLBACK TO SAVEPOINT cross_tenant_set")
        .execute(&mut *tx)
        .await
        .unwrap();

    tx.rollback().await.unwrap();
}

#[tokio::test]
#[ignore = "requires a migrated PostgreSQL database (DATABASE_URL)"]
async fn roster_constraints_are_validated_and_tenant_scoped() {
    let pool = pool().await;
    let mut tx = pool.begin().await.unwrap();

    let organisation = create_organisation(&mut tx).await;
    let other_organisation = create_organisation(&mut tx).await;
    let worker = create_contact(&mut tx, organisation).await;
    let target = create_contact(&mut tx, organisation).await;
    let foreign = create_contact(&mut tx, other_organisation).await;

    sqlx::query("SAVEPOINT worker_self_constraint")
        .execute(&mut *tx)
        .await
        .unwrap();
    let self_constraint = sqlx::query(
        "INSERT INTO roster_constraints
            (id, organisation_id, worker_contact_id, target_contact_id, kind)
         VALUES ($1, $2, $3, $3, 'blocked')",
    )
    .bind(Uuid::now_v7())
    .bind(organisation)
    .bind(worker)
    .execute(&mut *tx)
    .await;
    assert!(
        self_constraint.is_err(),
        "a worker cannot constrain themselves"
    );
    sqlx::query("ROLLBACK TO SAVEPOINT worker_self_constraint")
        .execute(&mut *tx)
        .await
        .unwrap();

    sqlx::query("SAVEPOINT cross_tenant_target")
        .execute(&mut *tx)
        .await
        .unwrap();
    let cross_tenant = sqlx::query(
        "INSERT INTO roster_constraints
            (id, organisation_id, worker_contact_id, target_contact_id, kind)
         VALUES ($1, $2, $3, $4, 'blocked')",
    )
    .bind(Uuid::now_v7())
    .bind(organisation)
    .bind(worker)
    .bind(foreign)
    .execute(&mut *tx)
    .await;
    assert!(
        cross_tenant.is_err(),
        "constraint targets must belong to the same tenant"
    );
    sqlx::query("ROLLBACK TO SAVEPOINT cross_tenant_target")
        .execute(&mut *tx)
        .await
        .unwrap();

    sqlx::query(
        "INSERT INTO roster_constraints
            (id, organisation_id, worker_contact_id, target_contact_id, kind, strength)
         VALUES ($1, $2, $3, $4, 'blocked', 5)",
    )
    .bind(Uuid::now_v7())
    .bind(organisation)
    .bind(worker)
    .bind(target)
    .execute(&mut *tx)
    .await
    .expect("valid worker-to-worker constraint");

    tx.rollback().await.unwrap();
}

#[tokio::test]
#[ignore = "requires a migrated PostgreSQL database (DATABASE_URL)"]
async fn worker_client_history_aggregates_assignment_outcomes() {
    let pool = pool().await;
    let mut tx = pool.begin().await.unwrap();

    let organisation = create_organisation(&mut tx).await;
    let worker = create_contact(&mut tx, organisation).await;
    let client = create_contact(&mut tx, organisation).await;
    let shift = create_shift(&mut tx, organisation, None).await;

    sqlx::query(
        "INSERT INTO client_assignments
            (id, organisation_id, shift_id, contact_id, status)
         VALUES ($1, $2, $3, $4, 'attended')",
    )
    .bind(Uuid::now_v7())
    .bind(organisation)
    .bind(shift)
    .bind(client)
    .execute(&mut *tx)
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO worker_assignments
            (id, organisation_id, shift_id, contact_id, status, outcome, outcome_rating)
         VALUES ($1, $2, $3, $4, 'attended', 'positive', 5)",
    )
    .bind(Uuid::now_v7())
    .bind(organisation)
    .bind(shift)
    .bind(worker)
    .execute(&mut *tx)
    .await
    .unwrap();

    let (attended, rating, positive): (i64, Option<f64>, i64) = sqlx::query_as(
        "SELECT attended_shifts, average_rating, positive_outcomes
         FROM worker_client_history
         WHERE organisation_id=$1 AND worker_contact_id=$2 AND client_contact_id=$3",
    )
    .bind(organisation)
    .bind(worker)
    .bind(client)
    .fetch_one(&mut *tx)
    .await
    .expect("history row");

    assert_eq!(attended, 1);
    assert_eq!(rating, Some(5.0));
    assert_eq!(positive, 1);

    tx.rollback().await.unwrap();
}

#[tokio::test]
#[ignore = "requires a migrated PostgreSQL database (DATABASE_URL)"]
async fn branding_is_restricted_to_tenant_roots() {
    let pool = pool().await;
    let mut tx = pool.begin().await.unwrap();

    let tenant = create_organisation(&mut tx).await;
    let partner = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO organisations (id, tenant_id, name, kind)
         VALUES ($1, $2, 'Partner Org', 'provider')",
    )
    .bind(partner)
    .bind(tenant)
    .execute(&mut *tx)
    .await
    .expect("external organisation");

    let png = [0x89u8, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    let sha = "a".repeat(64);

    sqlx::query("SAVEPOINT partner_branding")
        .execute(&mut *tx)
        .await
        .unwrap();
    let partner_insert = sqlx::query(
        "INSERT INTO organisation_branding
            (organisation_id, logo, logo_content_type, logo_sha256)
         VALUES ($1, $2, 'image/png', $3)",
    )
    .bind(partner)
    .bind(&png[..])
    .bind(&sha)
    .execute(&mut *tx)
    .await;
    assert!(
        partner_insert.is_err(),
        "counterparties cannot carry tenant branding"
    );
    sqlx::query("ROLLBACK TO SAVEPOINT partner_branding")
        .execute(&mut *tx)
        .await
        .unwrap();

    sqlx::query(
        "INSERT INTO organisation_branding
            (organisation_id, logo, logo_content_type, logo_sha256)
         VALUES ($1, $2, 'image/png', $3)",
    )
    .bind(tenant)
    .bind(&png[..])
    .bind(&sha)
    .execute(&mut *tx)
    .await
    .expect("tenant root may carry branding");

    tx.rollback().await.unwrap();
}
