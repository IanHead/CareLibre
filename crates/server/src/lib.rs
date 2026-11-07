//! CareLibre server library.
//!
//! The binary in `main.rs` is a thin wrapper around [`api::router`]; everything
//! else lives here so it can be exercised from integration tests.

pub mod admin;
pub mod api;
pub mod auth;
pub mod config;
pub mod custom_fields;
pub mod db;
pub mod error;
pub mod import;
pub mod onboarding;
pub mod organisation;
pub mod permissions;
pub mod relationships;
pub mod rostering;
pub mod state;
pub mod telemetry;
pub mod util;
