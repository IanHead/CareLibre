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
    )),
    tags(
        (name = "system", description = "Liveness and readiness probes"),
        (name = "auth", description = "Authentication, sessions and identity")
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
