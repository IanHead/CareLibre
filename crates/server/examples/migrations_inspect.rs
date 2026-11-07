use sqlx::postgres::PgPoolOptions;
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    let url = std::env::var("DATABASE_URL")?;
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(&url)
        .await?;
    let rows: Vec<(i64, String, bool)> = sqlx::query_as(
        "SELECT version, description, success FROM _sqlx_migrations ORDER BY version",
    )
    .fetch_all(&pool)
    .await?;
    for row in rows {
        println!("{row:?}");
    }
    let orgs: Vec<(uuid::Uuid, String)> =
        sqlx::query_as("SELECT id, slug FROM organisations ORDER BY slug")
            .fetch_all(&pool)
            .await?;
    println!("ORGS {orgs:?}");
    Ok(())
}
