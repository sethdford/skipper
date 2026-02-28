//! Fleet: multi-repo daemon orchestration with auto-scaling and patrol.
//!
//! Manages:
//! - Issue polling and triage scoring
//! - Worker pool allocation and auto-scaling
//! - Job dispatch to available workers
//! - Periodic patrol checks (security, dependencies, coverage, DORA)
//! - Feed patrol results into decision engine

pub mod daemon;
pub mod dispatch;
pub mod patrol;

pub use daemon::{PollResult, TriagePriority};
pub use dispatch::{Dispatcher, JobClaim, WorkerPool};
pub use patrol::PatrolFinding;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Fleet-wide metrics snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FleetStatus {
    pub active_pipelines: u32,
    pub queued_issues: u32,
    pub allocated_workers: u32,
    pub available_workers: u32,
    pub total_cost_usd: f64,
    pub repos: HashMap<String, RepoStatus>,
}

/// Per-repository status.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoStatus {
    pub repo: String,
    pub active_pipelines: u32,
    pub queued_issues: u32,
    pub workers_allocated: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fleet_status_default() {
        let status = FleetStatus {
            active_pipelines: 0,
            queued_issues: 0,
            allocated_workers: 0,
            available_workers: 8,
            total_cost_usd: 0.0,
            repos: HashMap::new(),
        };
        assert_eq!(status.available_workers, 8);
        assert_eq!(status.repos.len(), 0);
    }

    #[test]
    fn test_repo_status_new() {
        let status = RepoStatus {
            repo: "owner/repo".to_string(),
            active_pipelines: 1,
            queued_issues: 3,
            workers_allocated: 2,
        };
        assert_eq!(status.repo, "owner/repo");
        assert_eq!(status.active_pipelines, 1);
    }
}
