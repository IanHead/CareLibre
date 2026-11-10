//! HTTP routing.

pub mod auth;
pub mod contacts;
pub mod funding;
pub mod health;
pub mod openapi;
pub mod organisation;
pub mod relationship_types;
pub mod relationships;
pub mod rostering;
pub mod status;
pub mod worker_qualities;

use axum::{
    extract::DefaultBodyLimit,
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
        .route(
            "/organisation",
            get(organisation::get_profile).patch(organisation::update_profile),
        )
        .route(
            "/organisation/logo",
            get(organisation::get_logo)
                .put(organisation::upload_logo)
                .delete(organisation::delete_logo_handler)
                .layer(DefaultBodyLimit::max(
                    crate::organisation::MAX_LOGO_BYTES + 64 * 1024,
                )),
        )
        .route("/contacts", get(contacts::list).post(contacts::create))
        .route("/contacts/{id}", get(contacts::get).patch(contacts::update))
        .route(
            "/contacts/{id}/qualities",
            get(worker_qualities::list_values).post(worker_qualities::set_value),
        )
        .route(
            "/contacts/{id}/qualities/{quality_id}",
            axum::routing::delete(worker_qualities::delete_value),
        )
        .route(
            "/contacts/{id}/requirements",
            get(worker_qualities::list_requirements).post(worker_qualities::set_requirement),
        )
        .route(
            "/contacts/{id}/requirements/{requirement_id}",
            axum::routing::delete(worker_qualities::delete_requirement),
        )
        .route(
            "/worker-qualities",
            get(worker_qualities::list).post(worker_qualities::create),
        )
        .route(
            "/worker-qualities/{id}",
            axum::routing::patch(worker_qualities::update),
        )
        .route(
            "/funding-sources",
            get(funding::list_sources).post(funding::create_source),
        )
        .route(
            "/funding-sources/{id}",
            axum::routing::patch(funding::update_source),
        )
        .route(
            "/contacts/{id}/funding-packages",
            get(funding::list_packages).post(funding::create_package),
        )
        .route(
            "/funding-packages/{id}",
            axum::routing::patch(funding::update_package_status),
        )
        .route(
            "/contacts/{id}/agreements",
            get(funding::list_agreements).post(funding::create_agreement),
        )
        .route(
            "/agreements/{id}",
            axum::routing::patch(funding::update_agreement_status),
        )
        .route("/agreements/{id}/lines", post(funding::add_line))
        .route(
            "/agreements/{id}/lines/{line_id}",
            axum::routing::delete(funding::delete_line),
        )
        .route("/agreements/{id}/finalise", post(funding::finalise))
        .route(
            "/relationships",
            get(relationships::list).post(relationships::create),
        )
        .route("/relationships/graph", get(relationships::graph))
        .route(
            "/relationship-types",
            get(relationship_types::list).post(relationship_types::create),
        )
        .route(
            "/relationship-types/{id}",
            axum::routing::patch(relationship_types::update),
        )
        .route(
            "/relationships/{id}",
            axum::routing::patch(relationships::correct),
        )
        .route("/relationships/{id}/history", get(relationships::history))
        .route("/relationships/{id}/end", post(relationships::end))
        .route(
            "/relationships/{id}/transfer",
            post(relationships::transfer),
        )
        .route(
            "/relationships/{id}/change-role",
            post(relationships::change_role),
        )
        .route("/relationships/{id}/remove", post(relationships::remove))
        .route(
            "/sites",
            get(rostering::list_sites).post(rostering::create_site),
        )
        .route(
            "/shifts",
            get(rostering::list_shifts).post(rostering::create_shift),
        )
        .route("/shifts/batch", post(rostering::create_shifts))
        .route(
            "/shifts/{id}",
            get(rostering::get_shift)
                .patch(rostering::update_shift)
                .delete(rostering::delete_shift),
        )
        .route("/shifts/{id}/copy", post(rostering::copy_shift))
        .route(
            "/shifts/{id}/assignments",
            post(rostering::create_assignment),
        )
        .route("/divisions", get(rostering::list_divisions))
        .route("/services", get(rostering::list_services))
        .route("/cost-centres", get(rostering::list_cost_centres))
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
