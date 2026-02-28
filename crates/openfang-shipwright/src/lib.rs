//! OpenFang Shipwright — Software Engineering Hand
//!
//! Autonomous delivery pipelines: Issue → Plan → Build → Test → PR → Deploy.
//! Integrates with OpenFang kernel for scheduling, RBAC, metering,
//! and cross-Hand intelligence via Collector, Researcher, and Predictor.

pub mod config;
pub mod decision;
pub mod github;
pub mod intelligence;
pub mod memory;
pub mod pipeline;

pub use config::ShipwrightConfig;
pub use decision::DecisionEngine;
pub use github::GitHubClient;
pub use intelligence::IntelligenceEngine;
pub use memory::ShipwrightMemory;
pub use pipeline::Pipeline;
