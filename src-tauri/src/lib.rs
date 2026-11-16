//! llm-switch — local gateway in front of many LLM backends.
//!
//! Module map (see docs/DEVELOPMENT-PLAN.md):
//! - [`config`]  config file, defaults, validation
//! - [`engine`]  per-engine presets: capabilities, dashboard, KV telemetry
//! - [`store`]   sqlite schema + access layer
//! - [`gateway`] axum server: client protocols, routing, metrics, logs, cache
//!
//! M0 scope: control plane only. Forwarding arrives in M1.

pub mod config;
pub mod engine;
pub mod gateway;
pub mod redact;
pub mod services;
pub mod store;
pub mod takeover;
pub mod types;

#[cfg(feature = "gui")]
pub mod commands;
#[cfg(feature = "gui")]
pub mod gui;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
