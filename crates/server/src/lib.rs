//! CareLibre server library.
//!
//! The binary in `main.rs` is a thin wrapper around [`api::router`]; everything
//! else lives here so it can be exercised from integration tests.

pub mod api;
pub mod auth;
pub mod config;
pub mod db;
pub mod error;
pub mod state;
pub mod telemetry;
