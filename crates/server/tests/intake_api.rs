//! Intake case API tests against a real PostgreSQL database.
//!
//! Marked `#[ignore]`; run with a migrated database:
//!
//! ```sh
//! DATABASE_URL=postgres://... cargo test -p carelibre-server --test intake_api -- --ignored
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
            std::env::var("DATABASE_URL").expect("DATABASE_URL must be set for intake API tests");
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
            demo_seed_enabled: false,
        };
        let router = api::router(AppState::new(pool.clone(), config));

        let organisation = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO organisations (id, slug, name) VALUES ($1, $2, 'Intake API Test')",
        )
        .bind(organisation)
        .bind(format!("intake-api-{}", organisation.simple()))
        .execute(&pool)
        .await
        .expect("insert organisation");

        let user = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO users (id, email, password_hash, full_name)
             VALUES ($1, $2, 'not-a-real-hash', 'Test Owner')",
        )
        .bind(user)
        .bind(format!("intake-{user}@example.com"))
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
        self.call_as(&self.token, method, uri, body).await
    }

    async fn call_as(
        &self,
        token: &str,
        method: Method,
        uri: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let mut request = Request::builder()
            .method(method)
            .uri(uri)
            .header("authorization", format!("Bearer {token}"));
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

    async fn flow(&self, key: &str) -> Value {
        let (status, body) = self.call(Method::GET, "/api/v1/intake/flows", None).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|flow| flow["key"] == key)
            .cloned()
            .expect("flow exists")
    }

    fn stage_id(flow: &Value, kind: &str) -> String {
        flow["stages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|stage| stage["kind"] == kind)
            .and_then(|stage| stage["id"].as_str())
            .expect("stage id")
            .to_owned()
    }

    async fn create_case(
        &self,
        flow_id: &str,
        given: &str,
        family: &str,
        email: &str,
    ) -> (StatusCode, Value) {
        self.call(
            Method::POST,
            "/api/v1/intake/cases",
            Some(json!({
                "flow_id": flow_id,
                "contact": {
                    "given_name": given,
                    "family_name": family,
                    "email": email,
                    "phone": "0400 000 000",
                    "date_of_birth": "1990-04-12"
                },
                "data": { "notes": "Test enquiry" }
            })),
        )
        .await
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
async fn create_board_detail_and_onboard_a_case() {
    let fx = Fixture::new().await;
    let flow = fx.flow("client_standard").await;
    let flow_id = flow["id"].as_str().unwrap();
    let onboarded = Fixture::stage_id(&flow, "onboarded");

    let (status, created) = fx
        .create_case(flow_id, "Mia", "Chen", "mia@example.com")
        .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let case_id = created["case"]["case"]["id"].as_str().unwrap();
    let contact_id = created["contact"]["id"].as_str().unwrap();
    assert_eq!(created["contact"]["lifecycle_status"], "enquiry");
    assert_eq!(created["case"]["case"]["stage"]["kind"], "enquiry");
    assert_eq!(created["case"]["case"]["data"]["first_name"], "Mia");
    assert!(!created["case"]["checklist"].as_array().unwrap().is_empty());

    let (status, board) = fx
        .call(
            Method::GET,
            &format!("/api/v1/intake/board?flow_id={flow_id}"),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{board}");
    let enquiry = board["columns"]
        .as_array()
        .unwrap()
        .iter()
        .find(|column| column["stage"]["kind"] == "enquiry")
        .unwrap();
    assert_eq!(enquiry["count"], 1);
    assert_eq!(enquiry["cards"][0]["id"], case_id);

    let (status, detail) = fx
        .call(
            Method::GET,
            &format!("/api/v1/intake/cases/{case_id}"),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{detail}");
    assert_eq!(detail["case"]["stage"]["kind"], "enquiry");
    assert_eq!(
        detail["required_fields"],
        json!(["first_name", "family_name"])
    );

    // Duplicate contact matching.
    let (status, duplicate) = fx
        .call(
            Method::POST,
            "/api/v1/intake/cases",
            Some(json!({
                "flow_id": flow_id,
                "contact": { "given_name": "Mia", "family_name": "Chen", "email": "mia@example.com" }
            })),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{duplicate}");
    assert_eq!(duplicate["error"]["details"]["reason"], "contact_match");

    // Existing open case.
    let (status, open) = fx
        .call(
            Method::POST,
            "/api/v1/intake/cases",
            Some(json!({ "flow_id": flow_id, "contact_id": contact_id })),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{open}");
    assert_eq!(open["error"]["details"]["reason"], "open_case_exists");

    let (status, transitioned) = fx
        .call(
            Method::POST,
            &format!("/api/v1/intake/cases/{case_id}/transition"),
            Some(json!({ "to_stage_id": onboarded, "version": 1 })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{transitioned}");
    assert_eq!(transitioned["case"]["stage"]["kind"], "onboarded");
    assert!(transitioned["case"]["onboarded_at"].is_string());
    assert_eq!(transitioned["case"]["lifecycle_status"], "active");

    let (status, contact) = fx
        .call(Method::GET, &format!("/api/v1/contacts/{contact_id}"), None)
        .await;
    assert_eq!(status, StatusCode::OK, "{contact}");
    assert_eq!(contact["contact"]["role"], "client");
    assert_eq!(contact["contact"]["lifecycle_status"], "active");

    // Onboarded cases do not block a new attempt for the same flow.
    let (status, second) = fx
        .call(
            Method::POST,
            "/api/v1/intake/cases",
            Some(json!({ "flow_id": flow_id, "contact_id": contact_id })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{second}");
    assert_ne!(second["case"]["case"]["id"], case_id);

    fx.cleanup().await;
}

#[tokio::test]
#[ignore = "requires a migrated PostgreSQL database (DATABASE_URL)"]
async fn required_fields_and_checklist_gate_transitions() {
    let fx = Fixture::new().await;
    let flow = fx.flow("client_standard").await;
    let flow_id = flow["id"].as_str().unwrap();
    let eligibility = Fixture::stage_id(&flow, "eligibility");
    let risk = Fixture::stage_id(&flow, "risk_assessment");

    let (_, created) = fx
        .create_case(flow_id, "Sam", "Lee", "sam@example.com")
        .await;
    let case_id = created["case"]["case"]["id"].as_str().unwrap();

    let (status, at_eligibility) = fx
        .call(
            Method::POST,
            &format!("/api/v1/intake/cases/{case_id}/transition"),
            Some(json!({ "to_stage_id": eligibility, "version": 1 })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{at_eligibility}");
    let version = at_eligibility["case"]["version"].as_i64().unwrap();

    let (status, blocked) = fx
        .call(
            Method::POST,
            &format!("/api/v1/intake/cases/{case_id}/transition"),
            Some(json!({ "to_stage_id": risk, "version": version })),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{blocked}");
    assert_eq!(
        blocked["error"]["details"]["reason"],
        "intake_transition_blocked"
    );
    assert!(blocked["error"]["details"]["missing_fields"]
        .as_array()
        .unwrap()
        .iter()
        .any(|value| value == "consent_services"));
    assert!(!blocked["error"]["details"]["incomplete_checklist"]
        .as_array()
        .unwrap()
        .is_empty());

    let (status, patched) = fx
        .call(
            Method::PATCH,
            &format!("/api/v1/intake/cases/{case_id}"),
            Some(json!({
                "version": version,
                "data": { "consent_services": true }
            })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{patched}");
    let version = patched["case"]["version"].as_i64().unwrap();

    let (status, detail) = fx
        .call(
            Method::GET,
            &format!("/api/v1/intake/cases/{case_id}"),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    for item in detail["checklist"].as_array().unwrap() {
        if item["is_required"] == true {
            let item_id = item["id"].as_str().unwrap();
            let (status, toggled) = fx
                .call(
                    Method::POST,
                    &format!("/api/v1/intake/cases/{case_id}/checklist/{item_id}"),
                    Some(json!({ "completed": true })),
                )
                .await;
            assert_eq!(status, StatusCode::OK, "{toggled}");
        }
    }

    let (status, moved) = fx
        .call(
            Method::POST,
            &format!("/api/v1/intake/cases/{case_id}/transition"),
            Some(json!({ "to_stage_id": risk, "version": version })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{moved}");
    assert_eq!(moved["case"]["stage"]["kind"], "risk_assessment");

    fx.cleanup().await;
}

#[tokio::test]
#[ignore = "requires a migrated PostgreSQL database (DATABASE_URL)"]
async fn case_creation_is_idempotent_and_can_be_cancelled() {
    let fx = Fixture::new().await;
    let flow = fx.flow("client_standard").await;
    let flow_id = flow["id"].as_str().unwrap();

    let payload = json!({
        "client_request_id": "test-request-1",
        "flow_id": flow_id,
        "contact": { "given_name": "Jo", "family_name": "Patel", "email": "jo@example.com" }
    });
    let (status, first) = fx
        .call(Method::POST, "/api/v1/intake/cases", Some(payload.clone()))
        .await;
    assert_eq!(status, StatusCode::CREATED, "{first}");
    let case_id = first["case"]["case"]["id"].as_str().unwrap().to_owned();

    let (status, retry) = fx
        .call(Method::POST, "/api/v1/intake/cases", Some(payload))
        .await;
    assert_eq!(status, StatusCode::OK, "{retry}");
    assert_eq!(retry["case"]["case"]["id"], case_id);

    let (status, cancelled) = fx
        .call(
            Method::POST,
            &format!("/api/v1/intake/cases/{case_id}/cancel"),
            Some(json!({ "version": 1, "reason": "Chose another provider" })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{cancelled}");
    assert!(cancelled["case"]["closed_at"].is_string());
    assert_eq!(
        cancelled["case"]["cancellation_reason"],
        "Chose another provider"
    );

    fx.cleanup().await;
}

#[tokio::test]
#[ignore = "requires a migrated PostgreSQL database (DATABASE_URL)"]
async fn contact_lifecycle_endpoint_records_history() {
    let fx = Fixture::new().await;
    let flow = fx.flow("client_standard").await;
    let flow_id = flow["id"].as_str().unwrap();
    let onboarded = Fixture::stage_id(&flow, "onboarded");

    let (_, created) = fx
        .create_case(flow_id, "Ana", "Ruiz", "ana@example.com")
        .await;
    let case_id = created["case"]["case"]["id"].as_str().unwrap().to_owned();
    let contact_id = created["contact"]["id"].as_str().unwrap().to_owned();

    let (status, _) = fx
        .call(
            Method::POST,
            &format!("/api/v1/intake/cases/{case_id}/transition"),
            Some(json!({ "to_stage_id": onboarded, "version": 1 })),
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    let (status, suspended) = fx
        .call(
            Method::PATCH,
            &format!("/api/v1/contacts/{contact_id}/status"),
            Some(json!({ "status": "suspended", "reason": "Hospital admission" })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{suspended}");
    assert_eq!(suspended["contact"]["lifecycle_status"], "suspended");

    let (status, history) = fx
        .call(
            Method::GET,
            &format!("/api/v1/contacts/{contact_id}/status-history"),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{history}");
    assert_eq!(history["items"][0]["from_status"], "active");
    assert_eq!(history["items"][0]["to_status"], "suspended");

    let (status, invalid) = fx
        .call(
            Method::PATCH,
            &format!("/api/v1/contacts/{contact_id}/status"),
            Some(json!({ "status": "enquiry" })),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{invalid}");
    assert_eq!(
        invalid["error"]["details"]["reason"],
        "lifecycle_transition_invalid"
    );

    fx.cleanup().await;
}

#[tokio::test]
#[ignore = "requires a migrated PostgreSQL database (DATABASE_URL)"]
async fn assignment_notifies_another_user() {
    let fx = Fixture::new().await;
    let flow = fx.flow("client_standard").await;
    let flow_id = flow["id"].as_str().unwrap();
    let eligibility = Fixture::stage_id(&flow, "eligibility");

    let second_user = Uuid::now_v7();
    let second_email = format!("intake-assignee-{second_user}@example.com");
    sqlx::query(
        "INSERT INTO users (id, email, password_hash, full_name)
         VALUES ($1, $2, 'not-a-real-hash', 'Stage Owner')",
    )
    .bind(second_user)
    .bind(&second_email)
    .execute(&fx.pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO organisation_users (organisation_id, user_id, role)
         VALUES ($1, $2, 'staff')",
    )
    .bind(fx.organisation)
    .bind(second_user)
    .execute(&fx.pool)
    .await
    .unwrap();
    let second_token = issue_access_token(
        SECRET,
        second_user,
        fx.organisation,
        "staff",
        Duration::from_secs(900),
    )
    .unwrap();

    let (status, configured) = fx
        .call(
            Method::PUT,
            &format!("/api/v1/intake/stages/{eligibility}/assignees"),
            Some(json!({
                "assignees": [{
                    "user_id": second_user,
                    "strategy": "fixed",
                    "notify": true
                }]
            })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{configured}");

    let (_, created) = fx
        .create_case(flow_id, "Jo", "Kim", "jo.kim@example.com")
        .await;
    let case_id = created["case"]["case"]["id"].as_str().unwrap();

    let (status, transitioned) = fx
        .call(
            Method::POST,
            &format!("/api/v1/intake/cases/{case_id}/transition"),
            Some(json!({ "to_stage_id": eligibility, "version": 1 })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{transitioned}");
    let expected_assignee = second_user.to_string();
    assert_eq!(
        transitioned["case"]["assignee"]["id"].as_str(),
        Some(expected_assignee.as_str())
    );

    let (status, notifications) = fx
        .call_as(
            &second_token,
            Method::GET,
            "/api/v1/notifications?unread=true",
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{notifications}");
    assert!(notifications["unread_count"].as_i64().unwrap() >= 1);
    let notification_id = notifications["items"][0]["id"].as_i64().unwrap();

    let (status, _) = fx
        .call_as(
            &second_token,
            Method::POST,
            &format!("/api/v1/notifications/{notification_id}/read"),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (_, after) = fx
        .call_as(
            &second_token,
            Method::GET,
            "/api/v1/notifications?unread=true",
            None,
        )
        .await;
    assert_eq!(after["unread_count"], 0);

    sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(second_user)
        .execute(&fx.pool)
        .await
        .unwrap();
    fx.cleanup().await;
}

#[tokio::test]
#[ignore = "requires a migrated PostgreSQL database (DATABASE_URL)"]
async fn division_filter_includes_child_subdivisions() {
    let fx = Fixture::new().await;
    let flow = fx.flow("client_standard").await;
    let flow_id = flow["id"].as_str().unwrap();

    let parent = Uuid::now_v7();
    let child = Uuid::now_v7();
    sqlx::query("INSERT INTO divisions (id, organisation_id, name) VALUES ($1, $2, 'North')")
        .bind(parent)
        .bind(fx.organisation)
        .execute(&fx.pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO divisions (id, organisation_id, parent_id, name)
         VALUES ($1, $2, $3, 'North Coast')",
    )
    .bind(child)
    .bind(fx.organisation)
    .bind(parent)
    .execute(&fx.pool)
    .await
    .unwrap();

    let (status, created) = fx
        .call(
            Method::POST,
            "/api/v1/intake/cases",
            Some(json!({
                "flow_id": flow_id,
                "division_id": child,
                "contact": { "given_name": "Lee", "family_name": "Park", "email": "lee@example.com" }
            })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");

    let (_, with_children) = fx
        .call(
            Method::GET,
            &format!(
                "/api/v1/intake/board?flow_id={flow_id}&division_id={parent}&include_descendants=true"
            ),
            None,
        )
        .await;
    let count: i64 = with_children["columns"]
        .as_array()
        .unwrap()
        .iter()
        .map(|column| column["count"].as_i64().unwrap())
        .sum();
    assert_eq!(count, 1);

    let (_, exact) = fx
        .call(
            Method::GET,
            &format!(
                "/api/v1/intake/board?flow_id={flow_id}&division_id={parent}&include_descendants=false"
            ),
            None,
        )
        .await;
    let count: i64 = exact["columns"]
        .as_array()
        .unwrap()
        .iter()
        .map(|column| column["count"].as_i64().unwrap())
        .sum();
    assert_eq!(count, 0);

    fx.cleanup().await;
}
