//! Bridge to Shipwright's fleet and daemon state on disk.
//!
//! Reads fleet configuration, costs, and daemon state from JSON files
//! in ~/.shipwright/ and aggregates them into a fleet status object.

use std::path::PathBuf;
use serde_json::{json, Value};

/// Bridge to Shipwright's fleet state on disk.
pub struct FleetBridge {
    /// Path to fleet-config.json
    fleet_config_path: PathBuf,
    /// Path to costs.json
    costs_path: PathBuf,
    /// Path to daemon state
    daemon_state_path: PathBuf,
}

impl FleetBridge {
    /// Create a new fleet bridge with defaults.
    ///
    /// Defaults:
    /// - fleet-config.json: ~/.shipwright/fleet-config.json
    /// - costs.json: ~/.shipwright/costs.json
    /// - daemon-state.json: ~/.shipwright/daemon-state.json
    pub fn new() -> Self {
        let shipwright_home = if let Ok(home) = std::env::var("HOME") {
            PathBuf::from(home).join(".shipwright")
        } else {
            PathBuf::from("/tmp/.shipwright")
        };

        Self {
            fleet_config_path: shipwright_home.join("fleet-config.json"),
            costs_path: shipwright_home.join("costs.json"),
            daemon_state_path: shipwright_home.join("daemon-state.json"),
        }
    }

    /// Create a fleet bridge with custom paths.
    pub fn with_paths<P: AsRef<std::path::Path>>(
        fleet_config: P,
        costs: P,
        daemon_state: P,
    ) -> Self {
        Self {
            fleet_config_path: fleet_config.as_ref().to_path_buf(),
            costs_path: costs.as_ref().to_path_buf(),
            daemon_state_path: daemon_state.as_ref().to_path_buf(),
        }
    }

    /// Get the current fleet status.
    ///
    /// Reads fleet-config.json, costs.json, and daemon-state.json and aggregates
    /// into a single status object. Returns sensible defaults if files are missing.
    pub async fn get_fleet_status(&self) -> Result<Value, String> {
        let fleet_config = self.read_fleet_config().await;
        let costs = self.read_costs().await;
        let daemon_state = self.read_daemon_state().await;

        let status = json!({
            "fleet_config": fleet_config,
            "costs": costs,
            "daemon_state": daemon_state,
            "timestamp": chrono::Utc::now().to_rfc3339(),
        });

        Ok(status)
    }

    /// Read and parse fleet-config.json.
    async fn read_fleet_config(&self) -> Value {
        match tokio::fs::read_to_string(&self.fleet_config_path).await {
            Ok(content) => match serde_json::from_str(&content) {
                Ok(value) => value,
                Err(_) => json!({"error": "Failed to parse fleet-config.json"}),
            },
            Err(_) => json!({"error": "fleet-config.json not found", "repos": []}),
        }
    }

    /// Read and parse costs.json.
    async fn read_costs(&self) -> Value {
        match tokio::fs::read_to_string(&self.costs_path).await {
            Ok(content) => match serde_json::from_str(&content) {
                Ok(value) => value,
                Err(_) => json!({"error": "Failed to parse costs.json"}),
            },
            Err(_) => json!({"total_cost_usd": 0.0, "costs_by_repo": {}}),
        }
    }

    /// Read and parse daemon-state.json.
    async fn read_daemon_state(&self) -> Value {
        match tokio::fs::read_to_string(&self.daemon_state_path).await {
            Ok(content) => match serde_json::from_str(&content) {
                Ok(value) => value,
                Err(_) => json!({"error": "Failed to parse daemon-state.json"}),
            },
            Err(_) => json!({
                "is_running": false,
                "active_pipelines": 0,
                "queued_issues": 0,
            }),
        }
    }
}

impl Default for FleetBridge {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_fleet_bridge_new() {
        let bridge = FleetBridge::new();
        assert!(bridge.fleet_config_path.to_string_lossy().contains(".shipwright"));
        assert!(bridge.costs_path.to_string_lossy().contains(".shipwright"));
        assert!(bridge.daemon_state_path.to_string_lossy().contains(".shipwright"));
    }

    #[test]
    fn test_fleet_bridge_with_paths() {
        let temp = TempDir::new().unwrap();
        let temp_path = temp.path();
        let config_path = temp_path.join("config.json");
        let costs_path = temp_path.join("costs.json");
        let state_path = temp_path.join("state.json");

        let bridge = FleetBridge::with_paths(&config_path, &costs_path, &state_path);
        assert_eq!(bridge.fleet_config_path, config_path);
        assert_eq!(bridge.costs_path, costs_path);
        assert_eq!(bridge.daemon_state_path, state_path);
    }

    #[tokio::test]
    async fn test_fleet_status_missing_files() {
        let temp = TempDir::new().unwrap();
        let temp_path = temp.path();
        let bridge = FleetBridge::with_paths(
            temp_path.join("config.json"),
            temp_path.join("costs.json"),
            temp_path.join("state.json"),
        );

        let result = bridge.get_fleet_status().await;
        assert!(result.is_ok());

        let status = result.unwrap();
        assert!(status.get("timestamp").is_some());
        assert!(status.get("fleet_config").is_some());
        assert!(status.get("costs").is_some());
        assert!(status.get("daemon_state").is_some());
    }

    #[tokio::test]
    async fn test_read_fleet_config_not_found() {
        let temp = TempDir::new().unwrap();
        let bridge = FleetBridge::with_paths(
            &temp.path().join("nonexistent.json"),
            &temp.path().join("costs.json"),
            &temp.path().join("state.json"),
        );

        let result = bridge.read_fleet_config().await;
        assert!(result.get("error").is_some());
    }

    #[tokio::test]
    async fn test_read_fleet_config_valid() {
        let temp = TempDir::new().unwrap();
        let config_path = temp.path().join("config.json");

        let config_json = json!({
            "repos": [
                {"name": "repo1", "workers": 2},
            ]
        });

        tokio::fs::write(&config_path, config_json.to_string())
            .await
            .unwrap();

        let bridge = FleetBridge::with_paths(
            &config_path,
            &temp.path().join("costs.json"),
            &temp.path().join("state.json"),
        );

        let result = bridge.read_fleet_config().await;
        assert!(result.get("repos").is_some());
        assert_eq!(result["repos"][0]["name"], "repo1");
    }

    #[tokio::test]
    async fn test_read_costs_valid() {
        let temp = TempDir::new().unwrap();
        let costs_path = temp.path().join("costs.json");

        let costs_json = json!({
            "total_cost_usd": 42.50,
            "costs_by_repo": {
                "repo1": 25.0,
                "repo2": 17.5,
            }
        });

        tokio::fs::write(&costs_path, costs_json.to_string())
            .await
            .unwrap();

        let bridge = FleetBridge::with_paths(
            &temp.path().join("config.json"),
            &costs_path,
            &temp.path().join("state.json"),
        );

        let result = bridge.read_costs().await;
        assert_eq!(result["total_cost_usd"], 42.5);
        assert_eq!(result["costs_by_repo"]["repo1"], 25.0);
    }

    #[tokio::test]
    async fn test_read_daemon_state_not_found() {
        let temp = TempDir::new().unwrap();
        let bridge = FleetBridge::with_paths(
            &temp.path().join("config.json"),
            &temp.path().join("costs.json"),
            &temp.path().join("nonexistent.json"),
        );

        let result = bridge.read_daemon_state().await;
        assert_eq!(result["is_running"], false);
        assert_eq!(result["active_pipelines"], 0);
    }

    #[tokio::test]
    async fn test_read_daemon_state_valid() {
        let temp = TempDir::new().unwrap();
        let state_path = temp.path().join("state.json");

        let state_json = json!({
            "is_running": true,
            "active_pipelines": 5,
            "queued_issues": 12,
        });

        tokio::fs::write(&state_path, state_json.to_string())
            .await
            .unwrap();

        let bridge = FleetBridge::with_paths(
            &temp.path().join("config.json"),
            &temp.path().join("costs.json"),
            &state_path,
        );

        let result = bridge.read_daemon_state().await;
        assert_eq!(result["is_running"], true);
        assert_eq!(result["active_pipelines"], 5);
        assert_eq!(result["queued_issues"], 12);
    }

    #[tokio::test]
    async fn test_get_fleet_status_aggregates() {
        let temp = TempDir::new().unwrap();
        let config_path = temp.path().join("config.json");
        let costs_path = temp.path().join("costs.json");
        let state_path = temp.path().join("state.json");

        // Write valid files
        tokio::fs::write(&config_path, json!({"repos": []}).to_string())
            .await
            .unwrap();
        tokio::fs::write(&costs_path, json!({"total_cost_usd": 0.0}).to_string())
            .await
            .unwrap();
        tokio::fs::write(&state_path, json!({"is_running": true}).to_string())
            .await
            .unwrap();

        let bridge = FleetBridge::with_paths(&config_path, &costs_path, &state_path);
        let result = bridge.get_fleet_status().await;

        assert!(result.is_ok());
        let status = result.unwrap();
        assert!(status.get("fleet_config").is_some());
        assert!(status.get("costs").is_some());
        assert!(status.get("daemon_state").is_some());
        assert!(status.get("timestamp").is_some());
    }
}
