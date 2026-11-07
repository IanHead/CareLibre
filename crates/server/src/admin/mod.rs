//! Server-rendered administration area.
//!
//! Auth model: the signed-in browser holds an HttpOnly, SameSite=Lax cookie
//! containing an opaque session token (the same kind used for API refresh
//! tokens). Every request resolves it against `auth_sessions` and requires an
//! `owner` or `admin` membership.

pub mod auth;
pub mod erd;
pub mod html;
pub mod pages;
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
        .route("/members", get(pages::members))
        .route("/sessions", get(pages::sessions))
        .route("/schema", get(schema::page))
}
