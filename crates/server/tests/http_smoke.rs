use std::time::Duration;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use carelibre_server::{
    api,
    config::{Config, Environment},
    state::AppState,
};
use http_body_util::BodyExt;
use sqlx::postgres::PgPoolOptions;
use tower::ServiceExt;

/// Build an app whose pool is lazy: these tests never touch PostgreSQL.
fn test_state() -> AppState {
    let config = Config {
        bind_addr: "127.0.0.1:0".parse().expect("valid socket address"),
        environment: Environment::Development,
        database_url: "postgres://carelibre:carelibre@localhost:5432/carelibre".to_owned(),
        database_max_connections: 1,
        run_migrations: false,
        jwt_secret: "test-secret-that-is-at-least-32-characters-long".to_owned(),
        access_token_ttl: Duration::from_secs(900),
        refresh_token_ttl: Duration::from_secs(3600),
        cors_allowed_origins: Vec::new(),
    };

    let pool = PgPoolOptions::new()
        // These tests never have a database; fail fast instead of waiting for
        // the default 30-second acquire timeout on public pages that query.
        .acquire_timeout(Duration::from_secs(1))
        .connect_lazy(&config.database_url)
        .expect("lazy pool should parse");

    AppState::new(pool, config)
}

async fn get(uri: &str) -> axum::response::Response {
    api::router(test_state())
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap()
}

#[tokio::test]
async fn liveness_is_public() {
    let response = get("/healthz").await;
    assert_eq!(response.status(), StatusCode::OK);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["status"], "ok");
    assert_eq!(json["version"], env!("CARGO_PKG_VERSION"));
}

#[tokio::test]
async fn me_requires_a_bearer_token() {
    let response = get("/api/v1/me").await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["error"]["code"], "unauthorized");
}

#[tokio::test]
async fn me_rejects_a_garbage_token() {
    let response = api::router(test_state())
        .oneshot(
            Request::builder()
                .uri("/api/v1/me")
                .header("authorization", "Bearer not-a-real-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn openapi_document_is_served() {
    let response = get("/api-docs/openapi.json").await;
    assert_eq!(response.status(), StatusCode::OK);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["info"]["title"], "CareLibre API");
    assert!(json["paths"]["/api/v1/auth/login"].is_object());
}

#[tokio::test]
async fn unknown_routes_404() {
    let response = get("/api/v1/does-not-exist").await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn root_serves_the_status_page() {
    let response = get("/").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get("cache-control")
            .and_then(|value| value.to_str().ok()),
        Some("no-store")
    );

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let html = String::from_utf8(body.to_vec()).unwrap();

    assert!(html.contains("CareLibre"));
    assert!(html.contains("Connect a client"));
    assert!(html.contains("/api/v1"));
    assert!(html.contains("/api-docs/openapi.json"));
}

#[tokio::test]
async fn admin_requires_authentication() {
    let response = get("/admin").await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        response
            .headers()
            .get("location")
            .and_then(|value| value.to_str().ok()),
        Some("/admin/login")
    );
}

#[tokio::test]
async fn admin_schema_requires_authentication() {
    let response = get("/admin/schema").await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
}

#[tokio::test]
async fn admin_custom_fields_require_authentication() {
    let response = get("/admin/custom-fields").await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
}

#[tokio::test]
async fn admin_onboarding_requires_authentication() {
    let response = get("/admin/onboarding").await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
}

#[tokio::test]
async fn admin_permission_sets_require_authentication() {
    let response = get("/admin/permission-sets").await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
}

#[tokio::test]
async fn admin_login_page_is_public() {
    let response = get("/admin/login").await;
    assert_eq!(response.status(), StatusCode::OK);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let html = String::from_utf8(body.to_vec()).unwrap();
    assert!(html.contains("action=\"/admin/login\""));
    assert!(html.contains("name=\"email\""));
}

#[tokio::test]
async fn admin_invitations_require_authentication() {
    let response = get("/admin/invitations").await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
}

#[tokio::test]
async fn invite_page_is_public_and_neutral_for_bad_tokens() {
    let response = get("/invite/not-a-real-token").await;
    assert_eq!(response.status(), StatusCode::OK);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let html = String::from_utf8(body.to_vec()).unwrap();
    assert!(html.contains("Invitation not valid"));
}
