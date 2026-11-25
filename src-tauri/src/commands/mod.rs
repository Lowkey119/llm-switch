//! Tauri IPC command modules (compiled only with `--features gui`).

pub mod agents;
pub mod backend;
pub mod cache;
pub mod config_archive;
pub mod config_io;
pub mod ext_install;
pub mod float;
pub mod halctl;
pub mod logs;
pub mod metrics;
pub mod panel;
pub mod params;
pub mod prompt;
pub mod remote;
pub mod routing;
pub mod skill;
pub mod sysmon;
pub mod takeover;
pub mod task;
pub mod ui;
pub mod update;
pub mod usage;
