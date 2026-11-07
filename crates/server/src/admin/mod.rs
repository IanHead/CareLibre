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
pub mod html;
pub mod pages;
pub mod permission_sets;
pub mod schema;

use axum::{
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
}
