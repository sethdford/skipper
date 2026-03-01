//! GitHub Deployments API integration.

use serde::{Deserialize, Serialize};

/// Deployment status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeploymentStatus {
    Pending,
    InProgress,
    Success,
    Failure,
    Inactive,
    Error,
}

/// A GitHub Deployment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Deployment {
    pub id: u64,
    pub url: String,
    pub sha: String,
    pub ref_: String,
    pub environment: String,
    pub status: DeploymentStatus,
    pub created_at: String,
    pub updated_at: String,
}

/// Deployment status update.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeploymentStatusUpdate {
    pub state: DeploymentStatus,
    pub description: String,
    pub environment_url: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deployment_status_serialize() {
        let status = DeploymentStatus::Success;
        let json = serde_json::to_string(&status).unwrap();
        assert_eq!(json, "\"success\"");
    }

    #[test]
    fn test_deployment_serialize() {
        let deployment = Deployment {
            id: 1,
            url: "https://api.github.com/repos/owner/repo/deployments/1".to_string(),
            sha: "abc123".to_string(),
            ref_: "refs/heads/main".to_string(),
            environment: "production".to_string(),
            status: DeploymentStatus::Success,
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:05:00Z".to_string(),
        };
        let json = serde_json::to_string(&deployment).unwrap();
        assert!(json.contains("\"environment\":\"production\""));
    }

    #[test]
    fn test_deployment_status_update_serialize() {
        let update = DeploymentStatusUpdate {
            state: DeploymentStatus::Success,
            description: "Deployment succeeded".to_string(),
            environment_url: Some("https://example.com".to_string()),
        };
        let json = serde_json::to_string(&update).unwrap();
        assert!(json.contains("\"description\":\"Deployment succeeded\""));
    }
}
