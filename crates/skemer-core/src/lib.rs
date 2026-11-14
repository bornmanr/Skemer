//! Skemer core: models, settings, plan helpers, and source traits.
//!
//! This crate deliberately has no Tauri, GTK, or D-Bus dependencies. It is
//! shared by the shell, the privileged helper, and the CLI.

pub mod models;
pub mod paths;
pub mod plan;
pub mod settings;
pub mod sources;

pub use models::{ApplyOutcome, ApplyRequest, HoldDecision, UpdateItem, UpdatePlan, UpdateSource};
pub use settings::Settings;
