//! External organisation CRUD through the HTTP API, against a real database.
//!
//! Marked `#[ignore]`; run with a migrated database:
//!
//! ```sh
//! DATABASE_URL=postgres://... cargo test -p carelibre-server --test organisations -- --ignored
//! ```

use std::time::Duration;

use axum::{
    body::Body,
    http::{Method, Request, StatusCode},
    Router,
};
use carelibre_server::{
    api,
    auth::token::issue_access_token,
    config::{Config, Environment},
    state::AppState,
};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

const SECRET: &str = "test-secret-that-is-at-least-32-characters-long";

struct Fixture {
    pool: PgPool,
    router: Router,
    organisation: Uuid,
    user: Uuid,
    token: String,
}

impl Fixture {
    async fn new() -> Self {
        let url =
            std::env::var("DATABASE_URL").expect("DATABASE_URL must be set for organisation tests");
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(4)
            .connect(&url)
            .await
            .expect("connect to PostgreSQL");
        sqlx::migrate!("../../migrations")
            .run(&pool)
            .await
            .expect("apply migrations");

        let config = Config {
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            environment: Environment::Development,
            database_url: url,
            database_max_connections: 4,
            run_migrations: false,
            jwt_secret: SECRET.to_owned(),
            access_token_ttl: Duration::from_secs(900),
            refresh_token_ttl: Duration::from_secs(3600),
            cors_allowed_origins: Vec::new(),
        };
        let router = api::router(AppState::new(pool.clone(), config));

        let organisation = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO organisations (id, slug, name) VALUES ($1, $2, 'Organisations Test')",
        )
        .bind(organisation)
        .bind(format!("organisations-test-{}", organisation.simple()))
        .execute(&pool)
        .await
        .expect("insert organisation");

        let user = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO users (id, email, password_hash, full_name)
             VALUES ($1, $2, 'not-a-real-hash', 'Test Owner')",
        )
        .bind(user)
        .bind(format!("organisations-{user}@example.com"))
        .execute(&pool)
        .await
        .expect("insert user");
        sqlx::query(
            "INSERT INTO organisation_users (organisation_id, user_id, role, is_owner)
             VALUES ($1, $2, 'owner', true)",
        )
        .bind(organisation)
        .bind(user)
        .execute(&pool)
        .await
        .expect("insert owner membership");

        let token = issue_access_token(
            SECRET,
            user,
            organisation,
            "owner",
            Duration::from_secs(900),
        )
        .expect("issue token");

        Self {
            pool,
            router,
            organisation,
            user,
            token,
        }
    }

    async fn call(&self, method: Method, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
        let mut request = Request::builder()
            .method(method)
            .uri(uri)
            .header("authorization", format!("Bearer {}", self.token));
        let body = match body {
            Some(body) => {
                request = request.header("content-type", "application/json");
                Body::from(body.to_string())
            }
            None => Body::empty(),
        };
        let response = self
            .router
            .clone()
            .oneshot(request.body(body).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let json = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        (status, json)
    }

    async fn cleanup(self) {
        sqlx::query("DELETE FROM organisations WHERE id = $1")
            .bind(self.organisation)
            .execute(&self.pool)
            .await
            .expect("delete test organisation");
        sqlx::query("DELETE FROM users WHERE id = $1")
            .bind(self.user)
            .execute(&self.pool)
            .await
            .expect("delete test user");
    }
}

#[tokio::test]
#[ignore = "requires a migrated PostgreSQL database (DATABASE_URL)"]
async fn create_list_update_and_archive_an_organisation() {
    let fx = Fixture::new().await;

    let (status, created) = fx
        .call(
            Method::POST,
            "/api/v1/organisations",
            Some(json!({
                "name": "  Northside Care  ",
                "kind": "provider",
                "abn": "12 345 678 901",
                "email": "hello@northside.example",
                "phone": "03 9000 0000",
                "website": "https://northside.example",
                "notes": "Preferred provider",
                "is_active": true
            })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(created["name"], "Northside Care");
    assert_eq!(created["abn"], "12345678901");
    assert_eq!(created["relationship_count"], 0);
    let id = created["id"].as_str().unwrap().to_owned();

    let (status, listed) = fx
        .call(
            Method::GET,
            "/api/v1/organisations?search=northside&status=active",
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(listed["total"], 1);
    assert_eq!(listed["items"][0]["id"], id);

    let (status, updated) = fx
        .call(
            Method::PATCH,
            &format!("/api/v1/organisations/{id}"),
            Some(json!({
                "name": "Northside Care Group",
                "kind": "provider",
                "abn": null,
                "email": null,
                "phone": null,
                "website": null,
                "notes": null,
                "is_active": true
            })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{updated}");
    assert_eq!(updated["name"], "Northside Care Group");

    let (status, _) = fx
        .call(Method::DELETE, &format!("/api/v1/organisations/{id}"), None)
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, active) = fx
        .call(Method::GET, "/api/v1/organisations?status=active", None)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(active["total"], 0);

    let (status, all) = fx
        .call(Method::GET, "/api/v1/organisations?status=all", None)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(all["total"], 1);
    assert_eq!(all["items"][0]["is_active"], false);

    fx.cleanup().await;
}

#[tokio::test]
#[ignore = "requires a migrated PostgreSQL database (DATABASE_URL)"]
async fn duplicate_names_and_invalid_fields_are_rejected() {
    let fx = Fixture::new().await;

    let (status, _) = fx
        .call(
            Method::POST,
            "/api/v1/organisations",
            Some(json!({ "name": "Blue Hills", "is_active": true })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, conflict) = fx
        .call(
            Method::POST,
            "/api/v1/organisations",
            Some(json!({ "name": "blue hills", "is_active": true })),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{conflict}");

    let (status, _) = fx
        .call(
            Method::POST,
            "/api/v1/organisations",
            Some(json!({ "name": "   ", "is_active": true })),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

    let (status, _) = fx
        .call(
            Method::POST,
            "/api/v1/organisations",
            Some(json!({ "name": "Bad ABN", "abn": "123", "is_active": true })),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

    fx.cleanup().await;
}
