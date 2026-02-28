//! OpenFang Shipwright — Software Engineering Hand
//!
//! Autonomous delivery pipelines: Issue → Plan → Build → Test → PR → Deploy.
//! Integrates with OpenFang kernel for scheduling, RBAC, metering,
//! and cross-Hand intelligence via Collector, Researcher, and Predictor.

pub mod config;

pub use config::ShipwrightConfig;
