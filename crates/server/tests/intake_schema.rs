//! Schema-level intake tests.
//!
//! These require a migrated PostgreSQL database and are marked `#[ignore]` so
//! the default `cargo test` stays hermetic. Run them with:
//!
//! ```sh
//! DATABASE_URL=postgres://... cargo test --workspace -- --ignored
//! ```

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
        .bind(format!("intake-test-{}", &id.simple().to_string()[..12]))
        .bind("Intake Schema Test")
        .execute(&mut **tx)
        .await
        .expect("insert organisation");
    id
}

#[tokio::test]
#[ignore = "requires a migrated PostgreSQL database (DATABASE_URL)"]
async fn seeds_default_flows_stages_and_policies() {
    let pool = pool().await;
    let mut tx = pool.begin().await.unwrap();
    let organisation = create_organisation(&mut tx).await;

    let flows: i64 =
        sqlx::query_scalar("SELECT count(*) FROM intake_flows WHERE organisation_id = $1")
            .bind(organisation)
            .fetch_one(&mut *tx)
            .await
            .unwrap();
    assert_eq!(flows, 9);

    let stages_per_flow: Vec<i64> = sqlx::query_scalar(
        "SELECT count(*) FROM intake_stages
         WHERE organisation_id = $1 GROUP BY flow_id",
    )
    .bind(organisation)
    .fetch_all(&mut *tx)
    .await
    .unwrap();
    assert_eq!(stages_per_flow.len(), 9);
    assert!(stages_per_flow.iter().all(|count| *count == 6));

    let actions: i64 =
        sqlx::query_scalar("SELECT count(*) FROM intake_stage_actions WHERE organisation_id = $1")
            .bind(organisation)
            .fetch_one(&mut *tx)
            .await
            .unwrap();
    assert_eq!(actions, 9 * 6 * 7);

    let enquiry_first: bool = sqlx::query_scalar(
        "SELECT bool_and(first_kind = 'enquiry') FROM (
             SELECT DISTINCT ON (flow_id) flow_id, stage_kind AS first_kind
             FROM intake_stages
             WHERE organisation_id = $1
             ORDER BY flow_id, sort_order, id
         ) first_stages",
    )
    .bind(organisation)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    assert!(enquiry_first);

    // External organisations and branches do not own a second copy of the
    // tenant's flows.
    let external = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO organisations (id, tenant_id, name, kind)
         VALUES ($1, $2, 'External Provider', 'provider')",
    )
    .bind(external)
    .bind(organisation)
    .execute(&mut *tx)
    .await
    .unwrap();
    let external_flows: i64 =
        sqlx::query_scalar("SELECT count(*) FROM intake_flows WHERE organisation_id = $1")
            .bind(external)
            .fetch_one(&mut *tx)
            .await
            .unwrap();
    assert_eq!(external_flows, 0);

    tx.rollback().await.unwrap();
}

#[tokio::test]
#[ignore = "requires a migrated PostgreSQL database (DATABASE_URL)"]
async fn intake_capabilities_are_seeded_on_personas() {
    let pool = pool().await;
    let mut tx = pool.begin().await.unwrap();
    let organisation = create_organisation(&mut tx).await;

    let intake_capabilities: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM permission_set_capabilities pc
         JOIN permission_sets ps ON ps.id = pc.permission_set_id
         WHERE ps.organisation_id = $1 AND ps.persona_key = 'intake'
           AND pc.capability LIKE 'intake.%'",
    )
    .bind(organisation)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    assert_eq!(intake_capabilities, 3);

    let admin_capabilities: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM permission_set_capabilities pc
         JOIN permission_sets ps ON ps.id = pc.permission_set_id
         WHERE ps.organisation_id = $1 AND ps.persona_key = 'administrator'
           AND pc.capability LIKE 'intake.%'",
    )
    .bind(organisation)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    assert_eq!(admin_capabilities, 4);

    tx.rollback().await.unwrap();
}

#[tokio::test]
#[ignore = "requires a migrated PostgreSQL database (DATABASE_URL)"]
async fn a_contact_can_have_only_one_open_case_per_flow() {
    let pool = pool().await;
    let mut tx = pool.begin().await.unwrap();
    let organisation = create_organisation(&mut tx).await;

    let contact = Uuid::now_v7();
    sqlx::query("INSERT INTO contacts (id, organisation_id, display_name) VALUES ($1, $2, $3)")
        .bind(contact)
        .bind(organisation)
        .bind("Mia Chen")
        .execute(&mut *tx)
        .await
        .unwrap();

    let (flow_id, stage_id): (Uuid, Uuid) = sqlx::query_as(
        "SELECT f.id, s.id
         FROM intake_flows f
         JOIN intake_stages s ON s.flow_id = f.id AND s.organisation_id = f.organisation_id
         WHERE f.organisation_id = $1 AND f.key = 'client_standard'
         ORDER BY s.sort_order, s.id
         LIMIT 1",
    )
    .bind(organisation)
    .fetch_one(&mut *tx)
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO intake_cases
            (id, organisation_id, flow_id, stage_id, contact_id, subject_kind, display_name)
         VALUES ($1, $2, $3, $4, $5, 'client', 'Mia Chen')",
    )
    .bind(Uuid::now_v7())
    .bind(organisation)
    .bind(flow_id)
    .bind(stage_id)
    .bind(contact)
    .execute(&mut *tx)
    .await
    .expect("first case inserted");

    let duplicate = sqlx::query(
        "INSERT INTO intake_cases
            (id, organisation_id, flow_id, stage_id, contact_id, subject_kind, display_name)
         VALUES ($1, $2, $3, $4, $5, 'client', 'Mia Chen')",
    )
    .bind(Uuid::now_v7())
    .bind(organisation)
    .bind(flow_id)
    .bind(stage_id)
    .bind(contact)
    .execute(&mut *tx)
    .await;

    assert!(duplicate.is_err(), "a second open case must be rejected");

    tx.rollback().await.unwrap();
}
