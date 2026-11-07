#![allow(clippy::type_complexity)]

use sqlx::postgres::PgPoolOptions;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    let url = std::env::var("DATABASE_URL")?;
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await?;

    let orgs: Vec<(uuid::Uuid, String, String)> =
        sqlx::query_as("SELECT id, name, slug FROM organisations ORDER BY name")
            .fetch_all(&pool)
            .await?;
    println!("ORGS: {orgs:?}");
    let users: Vec<(String, String)> =
        sqlx::query_as("SELECT email, full_name FROM users ORDER BY email")
            .fetch_all(&pool)
            .await?;
    println!("USERS: {users:?}");
    let counts: (i64,) = sqlx::query_as("SELECT count(*) FROM contacts")
        .fetch_one(&pool)
        .await?;
    println!("CONTACTS: {}", counts.0);
    let contacts: Vec<(uuid::Uuid, uuid::Uuid, String, Option<String>, Option<String>, Option<String>)> = sqlx::query_as("SELECT id, organisation_id, display_name, email, phone, notes FROM contacts ORDER BY display_name LIMIT 10").fetch_all(&pool).await?;
    println!("SAMPLE CONTACTS: {contacts:?}");
    let fields: Vec<(uuid::Uuid, uuid::Uuid, String, String, String, Option<serde_json::Value>, i32)> = sqlx::query_as("SELECT id, organisation_id, key, label, data_type, options, sort_order FROM custom_field_definitions WHERE entity_type='contact' ORDER BY organisation_id, sort_order").fetch_all(&pool).await?;
    println!("CONTACT CUSTOM FIELDS: {fields:?}");
    let values: (i64,) =
        sqlx::query_as("SELECT count(*) FROM custom_field_values WHERE entity_type='contact'")
            .fetch_one(&pool)
            .await?;
    println!("CONTACT CUSTOM VALUES: {}", values.0);
    Ok(())
}
