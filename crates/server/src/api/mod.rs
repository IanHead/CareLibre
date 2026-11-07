//! HTTP routing.

pub mod auth;
pub mod contacts;
pub mod health;
pub mod openapi;
pub mod relationships;
pub mod rostering;
pub mod status;

use axum::{
    http::{header, HeaderValue, Method},
    routing::{get, post},
    Router,
};
use tower_http::{cors::CorsLayer, trace::TraceLayer};

use crate::{config::Config, state::AppState};

/// Build the complete application router.
pub fn router(state: AppState) -> Router {
    let cors = cors_layer(&state.config);

    let app = Router::new()
        .route("/", get(status::index))
        .route("/healthz", get(health::liveness))
        .route("/readyz", get(health::readiness))
        .route(
            "/invite/{token}",
            get(crate::admin::invitations::accept_form).post(crate::admin::invitations::accept),
        )
        .nest("/admin", crate::admin::router())
        .nest("/api/v1", v1_routes())
        .with_state(state);

    app.merge(openapi::swagger())
        .layer(TraceLayer::new_for_http())
        .layer(cors)
}

fn v1_routes() -> Router<AppState> {
    Router::new()
        .route("/auth/register", post(auth::register))
        .route("/auth/login", post(auth::login))
        .route("/auth/refresh", post(auth::refresh))
        .route("/auth/logout", post(auth::logout))
        .route("/me", get(auth::me))
        .route("/contacts", get(contacts::list).post(contacts::create))
        .route("/contacts/{id}", get(contacts::get).patch(contacts::update))
        .route(
            "/relationships",
            get(relationships::list).post(relationships::create),
        )
        .route("/relationships/graph", get(relationships::graph))
        .route(
            "/relationships/{id}",
            axum::routing::delete(relationships::delete),
        )
        .route(
            "/sites",
            get(rostering::list_sites).post(rostering::create_site),
        )
        .route(
            "/shifts",
            get(rostering::list_shifts).post(rostering::create_shift),
        )
        .route(
            "/shifts/{id}",
            get(rostering::get_shift).patch(rostering::update_shift),
        )
        .route(
            "/shifts/{id}/assignments",
            post(rostering::create_assignment),
        )
        .route("/shifts/{id}/suitability", get(rostering::suitability))
        .route(
            "/assignments/{id}",
            axum::routing::patch(rostering::update_assignment).delete(rostering::delete_assignment),
        )
        .route(
            "/roster-constraints",
            get(rostering::list_constraints).post(rostering::create_constraint),
        )
        .route(
            "/roster-constraints/{id}",
            axum::routing::delete(rostering::delete_constraint),
        )
        .route("/custom-fields", get(contacts::list_custom_fields))
}

fn cors_layer(config: &Config) -> CorsLayer {
    let layer = CorsLayer::new()
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::PATCH,
            Method::DELETE,
            Method::OPTIONS,
        ])
        .allow_headers([
            header::AUTHORIZATION,
            header::CONTENT_TYPE,
            header::ACCEPT,
            header::HeaderName::from_static("x-request-id"),
        ]);

    if config.cors_allowed_origins.is_empty() {
        return layer;
    }

    if config
        .cors_allowed_origins
        .iter()
        .any(|origin| origin == "*")
    {
        return layer.allow_origin(tower_http::cors::Any);
    }

    let origins = config
        .cors_allowed_origins
        .iter()
        .filter_map(|origin| origin.parse::<HeaderValue>().ok())
        .collect::<Vec<_>>();

    layer.allow_origin(origins)
}
