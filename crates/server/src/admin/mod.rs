//! Server-rendered administration area.
//!
//! Auth model: the signed-in browser holds an HttpOnly, SameSite=Lax cookie
//! containing an opaque session token (the same kind used for API refresh
//! tokens). Every request resolves it against `auth_sessions` and requires an
//! `owner` or `admin` membership.

pub mod auth;
pub mod csrf;
pub mod custom_fields;
pub mod erd;
pub mod funding_sources;
pub mod html;
pub mod import_ui;
pub mod invitations;
pub mod network;
pub mod onboarding;
pub mod organisation;
pub mod organisation_requirements;
pub mod pages;
pub mod permission_sets;
pub mod relationship_types;
pub mod schema;

use axum::{
    extract::DefaultBodyLimit,
    routing::{get, post},
    Router,
};

use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(pages::dashboard))
        .route("/login", get(auth::login_form).post(auth::login))
        .route("/logout", post(auth::logout))
        .route("/users", get(pages::users))
        .route("/sessions", get(pages::sessions))
        .route(
            "/organisation",
            get(organisation::page).post(organisation::update_name),
        )
        .route(
            "/organisation/logo",
            get(organisation::logo)
                .post(organisation::upload_logo)
                .layer(DefaultBodyLimit::max(
                    crate::organisation::MAX_LOGO_BYTES + 64 * 1024,
                )),
        )
        .route("/organisation/logo/delete", post(organisation::delete_logo))
        .route(
            "/organisation/requirements",
            post(organisation_requirements::add),
        )
        .route(
            "/organisation/requirements/{id}/delete",
            post(organisation_requirements::delete),
        )
        .route(
            "/funding-sources",
            get(funding_sources::list).post(funding_sources::add),
        )
        .route(
            "/funding-sources/{id}/active",
            post(funding_sources::set_active),
        )
        .route("/relationships", get(network::page))
        .route(
            "/relationship-types",
            get(relationship_types::list).post(relationship_types::add),
        )
        .route(
            "/relationship-types/{id}/active",
            post(relationship_types::set_active),
        )
        .route("/schema", get(schema::page))
        .route(
            "/custom-fields",
            get(custom_fields::list).post(custom_fields::create),
        )
        .route("/custom-fields/new", get(custom_fields::new_form))
        .route("/custom-fields/{id}/edit", get(custom_fields::edit_form))
        .route("/custom-fields/{id}", post(custom_fields::update))
        .route("/custom-fields/{id}/archive", post(custom_fields::archive))
        .route("/custom-fields/{id}/delete", post(custom_fields::delete))
        .route(
            "/permission-sets",
            get(permission_sets::list).post(permission_sets::create),
        )
        .route("/permission-sets/new", get(permission_sets::new_form))
        .route(
            "/permission-sets/{id}/edit",
            get(permission_sets::edit_form),
        )
        .route("/permission-sets/{id}", post(permission_sets::update))
        .route(
            "/permission-sets/{id}/delete",
            post(permission_sets::delete),
        )
        .route(
            "/users/{user_id}/access",
            get(pages::user_access).post(pages::update_user_access),
        )
        .route(
            "/invitations",
            get(invitations::list).post(invitations::create),
        )
        .route("/invitations/new", get(invitations::new_form))
        .route("/invitations/{id}/revoke", post(invitations::revoke))
        .route("/onboarding", get(onboarding::index))
        .route("/onboarding/{key}/done", post(onboarding::done))
        .route("/onboarding/{key}/skip", post(onboarding::skip))
        .route("/onboarding/signoff", post(onboarding::signoff))
        .route("/onboarding/mode", post(onboarding::mode))
        .route("/import", get(import_ui::list))
        .route("/import/templates/{object}", get(import_ui::template))
        .route("/import/spec/{object}", get(import_ui::spec))
        .route(
            "/import/upload/{object}",
            post(import_ui::upload).layer(DefaultBodyLimit::max(crate::import::MAX_FILE_BYTES)),
        )
        .route("/import/batch/{batch_id}", get(import_ui::review))
        .route("/import/batch/{batch_id}/commit", post(import_ui::commit))
        .route("/import/batch/{batch_id}/cancel", post(import_ui::cancel))
        .route(
            "/import/batch/{batch_id}/errors.csv",
            get(import_ui::errors_csv),
        )
}
