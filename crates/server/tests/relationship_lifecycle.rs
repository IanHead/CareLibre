//! Relationship lifecycle through the HTTP API, against a real database.
//!
//! Marked `#[ignore]`; run with a migrated database:
//!
//! ```sh
//! DATABASE_URL=postgres://... cargo test --workspace -- --ignored
//! ```
//!
//! The test creates its own organisation and users, and deletes them at the
//! end, so the database is left as it was.

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
use chrono::{Duration as Days, Utc};
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
    users: Vec<Uuid>,
}

impl Fixture {
    async fn new() -> Self {
        let url = std::env::var("DATABASE_URL")
            .expect("DATABASE_URL must be set for relationship lifecycle tests");
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

        // Clear anything an earlier, interrupted run left behind.
        sqlx::query(
            "DELETE FROM organisations WHERE slug LIKE 'lifecycle-%' AND name = 'Lifecycle Test'",
        )
        .execute(&pool)
        .await
        .expect("remove leftover test organisations");
        sqlx::query(
            "DELETE FROM users u
             WHERE u.full_name LIKE 'Test %' AND u.email LIKE '%@example.com'
               AND u.password_hash = 'not-a-real-hash'
               AND NOT EXISTS (SELECT 1 FROM organisation_users ou WHERE ou.user_id = u.id)",
        )
        .execute(&pool)
        .await
        .expect("remove leftover test users");

        let organisation = Uuid::now_v7();
        sqlx::query("INSERT INTO organisations (id, slug, name) VALUES ($1, $2, 'Lifecycle Test')")
            .bind(organisation)
            .bind(format!(
                "lifecycle-{}",
                &organisation.simple().to_string()[..12]
            ))
            .execute(&pool)
            .await
            .expect("insert organisation");

        Self {
            pool,
            router,
            organisation,
            users: Vec::new(),
        }
    }

    /// A non-owner member holding one persona's permission set.
    async fn member(&mut self, persona: &str) -> String {
        let id = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO users (id, email, password_hash, full_name)
             VALUES ($1, $2, 'not-a-real-hash', $3)",
        )
        .bind(id)
        .bind(format!("{persona}-{id}@example.com"))
        .bind(format!("Test {persona}"))
        .execute(&self.pool)
        .await
        .expect("insert user");
        sqlx::query(
            "INSERT INTO organisation_users (organisation_id, user_id, role, is_owner)
             VALUES ($1, $2, 'staff', false)",
        )
        .bind(self.organisation)
        .bind(id)
        .execute(&self.pool)
        .await
        .expect("insert member");
        sqlx::query(
            "INSERT INTO organisation_user_permission_sets
                (organisation_id, user_id, permission_set_id)
             SELECT $1, $2, id FROM permission_sets
             WHERE organisation_id = $1 AND persona_key = $3",
        )
        .bind(self.organisation)
        .bind(id)
        .bind(persona)
        .execute(&self.pool)
        .await
        .expect("assign permission set");
        self.users.push(id);

        issue_access_token(
            SECRET,
            id,
            self.organisation,
            "staff",
            Duration::from_secs(900),
        )
        .expect("token")
    }

    async fn contact(&self, name: &str) -> Uuid {
        let id = Uuid::now_v7();
        sqlx::query("INSERT INTO contacts (id, organisation_id, display_name) VALUES ($1, $2, $3)")
            .bind(id)
            .bind(self.organisation)
            .bind(name)
            .execute(&self.pool)
            .await
            .expect("insert contact");
        id
    }

    async fn call(
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

    async fn cleanup(self) {
        sqlx::query("DELETE FROM organisations WHERE id = $1")
            .bind(self.organisation)
            .execute(&self.pool)
            .await
            .expect("delete test organisation and its history");
        for user in self.users {
            sqlx::query("DELETE FROM users WHERE id = $1")
                .bind(user)
                .execute(&self.pool)
                .await
                .expect("delete test user");
        }
    }
}

fn actions(history: &Value) -> Vec<String> {
    history["items"]
        .as_array()
        .unwrap_or_else(|| panic!("not a history: {history}"))
        .iter()
        .map(|event| event["action"].as_str().unwrap().to_owned())
        .collect()
}

#[tokio::test]
#[ignore = "requires a migrated PostgreSQL database (DATABASE_URL)"]
async fn relationships_change_over_time_with_history_and_permissions() {
    let mut fx = Fixture::new().await;
    let admin = fx.member("administrator").await;
    let intake = fx.member("intake").await;
    let manager = fx.member("senior_management").await;

    let client = fx.contact("Client").await;
    let first_coordinator = fx.contact("First Coordinator").await;
    let second_coordinator = fx.contact("Second Coordinator").await;
    let worker = fx.contact("Worker").await;
    let org = fx.organisation;
    let today = Utc::now().date_naive();

    // Add.
    let (status, original) = fx
        .call(
            &intake,
            Method::POST,
            "/api/v1/relationships",
            Some(json!({
                "from_kind": "contact", "from_id": client, "from_role": "participant",
                "to_kind": "contact", "to_id": first_coordinator, "to_role": "support_coordinator",
                "start_date": today - Days::days(60), "end_date": null, "end_reason": null,
                "notes": null
            })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{original}");
    assert_eq!(original["status"], "current");
    let original_id = original["id"].as_str().unwrap().to_owned();

    // An end date always needs a reason.
    let (status, _) = fx
        .call(
            &intake,
            Method::POST,
            "/api/v1/relationships",
            Some(json!({
                "from_kind": "contact", "from_id": client, "from_role": "patient",
                "to_kind": "contact", "to_id": second_coordinator, "to_role": "doctor",
                "start_date": null, "end_date": today, "end_reason": null, "notes": null
            })),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

    // Transfer to a new coordinator from today.
    let (status, succession) = fx
        .call(
            &intake,
            Method::POST,
            &format!("/api/v1/relationships/{original_id}/transfer"),
            Some(json!({
                "effective_date": today, "replace": "to", "new_kind": "contact",
                "new_id": second_coordinator, "reason": "changed_provider",
                "note": "Family chose a new coordinator."
            })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{succession}");
    assert_eq!(succession["ended"]["status"], "ended");
    assert_eq!(succession["ended"]["end_reason"], "changed_provider");
    assert_eq!(succession["started"]["to_id"], json!(second_coordinator));
    assert_eq!(succession["started"]["previous_id"], json!(original_id));
    let current_id = succession["started"]["id"].as_str().unwrap().to_owned();

    // Ended relationships stay listed; the network only shows current ones.
    let (_, list) = fx
        .call(
            &intake,
            Method::GET,
            &format!("/api/v1/relationships?contact_id={client}"),
            None,
        )
        .await;
    assert_eq!(list["total"], 2);
    let (_, graph) = fx
        .call(&intake, Method::GET, "/api/v1/relationships/graph", None)
        .await;
    assert_eq!(graph["edges"].as_array().unwrap().len(), 1);

    // End in the future after an incident; the note is for managers only.
    let (status, ended) = fx
        .call(
            &intake,
            Method::POST,
            &format!("/api/v1/relationships/{current_id}/end"),
            Some(json!({
                "end_date": today + Days::days(14), "reason": "incident",
                "note": "Incident IR-1042."
            })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{ended}");
    assert_eq!(
        ended["status"], "current",
        "still current until the end date"
    );

    let (status, _) = fx
        .call(
            &intake,
            Method::POST,
            &format!("/api/v1/relationships/{current_id}/end"),
            Some(json!({ "end_date": today, "reason": "other", "note": null })),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "already has an end date");

    let history_uri = format!("/api/v1/relationships/{current_id}/history");
    let (_, history) = fx.call(&intake, Method::GET, &history_uri, None).await;
    assert_eq!(
        actions(&history),
        ["added", "transferred", "added", "ended"],
        "the chain includes the relationship it replaced"
    );
    let incident = &history["items"][3];
    assert_eq!(incident["note"], Value::Null);
    assert_eq!(incident["note_hidden"], true);
    assert_eq!(history["items"][2]["reason"], "changed_provider");
    assert_eq!(history["items"][1]["actor_name"], "Test intake");

    let (_, history) = fx.call(&manager, Method::GET, &history_uri, None).await;
    assert_eq!(history["items"][3]["note"], "Incident IR-1042.");

    // Correct a mistake; an unchanged correction is rejected.
    let mut corrected = json!({
        "from_kind": "contact", "from_id": client, "from_role": "participant",
        "to_kind": "contact", "to_id": second_coordinator, "to_role": "support_coordinator",
        "start_date": today, "end_date": today + Days::days(14), "end_reason": "incident",
        "notes": "Primary coordinator.",
        "note": "Notes were missing."
    });
    let (status, body) = fx
        .call(
            &intake,
            Method::PATCH,
            &format!("/api/v1/relationships/{current_id}"),
            Some(corrected.clone()),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["notes"], "Primary coordinator.");
    corrected["note"] = json!("Again.");
    let (status, _) = fx
        .call(
            &intake,
            Method::PATCH,
            &format!("/api/v1/relationships/{current_id}"),
            Some(corrected),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "nothing changed");

    // Remove needs relationships.remove and a reason.
    let remove_uri = format!("/api/v1/relationships/{current_id}/remove");
    let (status, _) = fx
        .call(
            &intake,
            Method::POST,
            &remove_uri,
            Some(json!({ "reason": "Wrong" })),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = fx
        .call(
            &admin,
            Method::POST,
            &remove_uri,
            Some(json!({ "reason": "  " })),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let (status, _) = fx
        .call(
            &admin,
            Method::POST,
            &remove_uri,
            Some(json!({ "reason": "Entered against the wrong client." })),
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (_, list) = fx
        .call(
            &intake,
            Method::GET,
            &format!("/api/v1/relationships?contact_id={client}"),
            None,
        )
        .await;
    assert_eq!(list["total"], 1, "removed relationships are hidden");
    let (_, history) = fx.call(&admin, Method::GET, &history_uri, None).await;
    assert_eq!(
        actions(&history).last().unwrap(),
        "removed",
        "but kept in history"
    );

    // Offboarding a worker takes them out of the worker list.
    let (status, employment) = fx
        .call(
            &intake,
            Method::POST,
            "/api/v1/relationships",
            Some(json!({
                "from_kind": "contact", "from_id": worker, "from_role": "employee",
                "to_kind": "organisation", "to_id": org, "to_role": "employer",
                "start_date": today - Days::days(400), "end_date": null, "end_reason": null,
                "notes": null
            })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{employment}");
    let (_, workers) = fx
        .call(&intake, Method::GET, "/api/v1/contacts?role=worker", None)
        .await;
    assert_eq!(workers["total"], 1);

    let (status, _) = fx
        .call(
            &intake,
            Method::POST,
            &format!(
                "/api/v1/relationships/{}/end",
                employment["id"].as_str().unwrap()
            ),
            Some(json!({
                "end_date": today - Days::days(1), "reason": "offboarded", "note": null
            })),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let (_, workers) = fx
        .call(&intake, Method::GET, "/api/v1/contacts?role=worker", None)
        .await;
    assert_eq!(workers["total"], 0);

    // Every change reached the audit log, and history cannot be rewritten.
    let audited: Vec<String> = sqlx::query_scalar(
        "SELECT action FROM audit_log
         WHERE organisation_id = $1 AND entity_type = 'relationship'
         ORDER BY id",
    )
    .bind(org)
    .fetch_all(&fx.pool)
    .await
    .unwrap();
    assert_eq!(
        audited,
        [
            "relationship.added",
            "relationship.transferred",
            "relationship.ended",
            "relationship.corrected",
            "relationship.removed",
            "relationship.added",
            "relationship.ended",
        ]
    );
    let rewrite =
        sqlx::query("UPDATE relationship_events SET note = 'x' WHERE organisation_id = $1")
            .bind(org)
            .execute(&fx.pool)
            .await;
    assert!(rewrite.is_err(), "history is append-only");

    fx.cleanup().await;
}

#[tokio::test]
#[ignore = "requires a migrated PostgreSQL database (DATABASE_URL)"]
async fn removal_and_incident_notes_have_sensible_defaults() {
    let fx = Fixture::new().await;

    let holders: Vec<(String, String)> = sqlx::query_as(
        "SELECT ps.persona_key, c.capability
         FROM permission_set_capabilities c
         JOIN permission_sets ps ON ps.id = c.permission_set_id
         WHERE ps.organisation_id = $1
           AND c.capability IN ('relationships.remove', 'relationships.incident_notes')
         ORDER BY 1, 2",
    )
    .bind(fx.organisation)
    .fetch_all(&fx.pool)
    .await
    .unwrap();

    let expected = [
        ("administrator", "relationships.incident_notes"),
        ("administrator", "relationships.remove"),
        ("house_manager", "relationships.incident_notes"),
        ("senior_management", "relationships.incident_notes"),
    ];
    let holders: Vec<(&str, &str)> = holders
        .iter()
        .map(|(persona, capability)| (persona.as_str(), capability.as_str()))
        .collect();
    assert_eq!(holders, expected);

    fx.cleanup().await;
}
