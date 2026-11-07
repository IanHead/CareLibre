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
            (id, organisation_id, from_contact_id, to_contact_id, relationship_type)
         VALUES ($1, $2, $3, $4, 'broker')",
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
            (id, organisation_id, from_organisation_id, to_organisation_id, relationship_type)
         VALUES ($1, $2, $3, $4, 'referral_partner')",
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
            (id, organisation_id, from_organisation_id, to_organisation_id, relationship_type)
         VALUES ($1, $2, $3, $4, 'referral_partner')",
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
