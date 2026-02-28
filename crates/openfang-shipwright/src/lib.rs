//! OpenFang Shipwright — Software Engineering Hand
//!
//! Autonomous delivery pipelines: Issue → Plan → Build → Test → PR → Deploy.
//! Integrates with OpenFang kernel for scheduling, RBAC, metering,
//! and cross-Hand intelligence via Collector, Researcher, and Predictor.

pub mod config;
pub mod github;
pub mod memory;

pub use config::ShipwrightConfig;
pub use github::GitHubClient;
pub use memory::ShipwrightMemory;
