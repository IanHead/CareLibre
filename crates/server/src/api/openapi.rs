//! OpenAPI document and Swagger UI wiring.

use utoipa::openapi::security::{HttpAuthScheme, HttpBuilder, SecurityScheme};
use utoipa::{Modify, OpenApi};

#[derive(OpenApi)]
#[openapi(
    info(
        title = "CareLibre API",
        version = env!("CARGO_PKG_VERSION"),
        description = "REST API for the CareLibre care management platform.\n\n\
                       All endpoints are versioned under `/api/v1`. Authenticate with a bearer \
                       token obtained from `/api/v1/auth/login`.",
        license(name = "AGPL-3.0-or-later")
    ),
    paths(
        crate::api::health::liveness,
        crate::api::health::readiness,
        crate::api::auth::register,
        crate::api::auth::login,
        crate::api::auth::refresh,
        crate::api::auth::logout,
        crate::api::auth::me,
        crate::api::relationships::list,
        crate::api::relationships::graph,
        crate::api::relationships::create,
        crate::api::relationships::delete,
        crate::api::rostering::list_sites,
        crate::api::rostering::create_site,
        crate::api::rostering::list_shifts,
        crate::api::rostering::create_shift,
        crate::api::rostering::get_shift,
        crate::api::rostering::update_shift,
        crate::api::rostering::create_assignment,
        crate::api::rostering::update_assignment,
        crate::api::rostering::delete_assignment,
        crate::api::rostering::list_constraints,
        crate::api::rostering::create_constraint,
        crate::api::rostering::delete_constraint,
        crate::api::rostering::suitability,
    ),
    components(schemas(
        crate::error::ErrorBody,
        crate::error::ErrorDetail,
        crate::api::health::Liveness,
        crate::api::health::Readiness,
        crate::api::auth::RegisterRequest,
        crate::api::auth::LoginRequest,
        crate::api::auth::RefreshRequest,
        crate::api::auth::LogoutRequest,
        crate::api::auth::AuthResponse,
        crate::api::auth::MeResponse,
        crate::api::auth::UserProfile,
        crate::api::auth::OrganisationProfile,
        crate::auth::role::Role,
        crate::relationships::PartyKind,
        crate::relationships::RelationshipEdge,
        crate::relationships::GraphNode,
        crate::relationships::RelationshipGraph,
        crate::api::relationships::RelationshipList,
        crate::api::relationships::RelationshipWrite,
        crate::api::rostering::SiteDto,
        crate::api::rostering::SiteWrite,
        crate::api::rostering::ShiftDto,
        crate::api::rostering::ShiftList,
        crate::api::rostering::ShiftWrite,
        crate::api::rostering::ShiftUpdate,
        crate::api::rostering::ShiftDetail,
        crate::api::rostering::AssignmentDto,
        crate::api::rostering::AssignmentWrite,
        crate::api::rostering::AssignmentUpdate,
        crate::api::rostering::SuitabilityResponse,
        crate::api::rostering::SuitabilityCandidate,
        crate::api::rostering::ContextRef,
        crate::api::rostering::PartyRef,
        crate::api::rostering::RosterConstraintDto,
        crate::api::rostering::RosterConstraintWrite,
    )),
    tags(
        (name = "system", description = "Liveness and readiness probes"),
        (name = "auth", description = "Authentication, sessions and identity"),
        (name = "relationships", description = "Two-sided links between people and organisations"),
        (name = "rostering", description = "Shifts, assignments, constraints and worker suitability")
    ),
    modifiers(&SecurityAddon)
)]
pub struct ApiDoc;

struct SecurityAddon;

impl Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        let components = openapi.components.get_or_insert_with(Default::default);
        components.add_security_scheme(
            "bearer_auth",
            SecurityScheme::Http(
                HttpBuilder::new()
                    .scheme(HttpAuthScheme::Bearer)
                    .bearer_format("JWT")
                    .build(),
            ),
        );
    }
}

/// Swagger UI at `/docs`, spec at `/api-docs/openapi.json`.
#[must_use]
pub fn swagger() -> utoipa_swagger_ui::SwaggerUi {
    utoipa_swagger_ui::SwaggerUi::new("/docs").url("/api-docs/openapi.json", ApiDoc::openapi())
}
