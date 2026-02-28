//! Shipwright configuration types.
//!
//! Parsed from `[shipwright]` section in `openfang.toml`.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Top-level Shipwright configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ShipwrightConfig {
    pub enabled: bool,
    pub default_template: PipelineTemplateName,
    pub fleet: FleetConfig,
    pub decision: DecisionConfig,
    pub intelligence: IntelligenceConfig,
    pub github: GitHubConfig,
    pub repos: Vec<RepoConfig>,
}

impl Default for ShipwrightConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            default_template: PipelineTemplateName::Standard,
            fleet: FleetConfig::default(),
            decision: DecisionConfig::default(),
            intelligence: IntelligenceConfig::default(),
            github: GitHubConfig::default(),
            repos: vec![],
        }
    }
}

/// Pipeline template selection.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum PipelineTemplateName {
    Fast,
    #[default]
    Standard,
    Full,
    Hotfix,
    Autonomous,
    CostAware,
}

/// Fleet/daemon worker configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct FleetConfig {
    pub poll_interval_seconds: u64,
    pub auto_scale: bool,
    pub max_workers: u32,
    pub min_workers: u32,
    pub worker_mem_gb: u32,
    pub cost_per_job_usd: f64,
}

impl Default for FleetConfig {
    fn default() -> Self {
        Self {
            poll_interval_seconds: 60,
            auto_scale: false,
            max_workers: 8,
            min_workers: 1,
            worker_mem_gb: 4,
            cost_per_job_usd: 5.0,
        }
    }
}

/// Decision engine configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DecisionConfig {
    pub enabled: bool,
    pub cycle_interval_seconds: u64,
    pub max_issues_per_day: u32,
    pub max_cost_per_day_usd: f64,
    pub cooldown_seconds: u64,
    pub halt_after_failures: u32,
    pub outcome_learning: bool,
}

impl Default for DecisionConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            cycle_interval_seconds: 1800,
            max_issues_per_day: 15,
            max_cost_per_day_usd: 25.0,
            cooldown_seconds: 300,
            halt_after_failures: 3,
            outcome_learning: true,
        }
    }
}

/// Intelligence layer configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct IntelligenceConfig {
    pub cache_ttl_seconds: u64,
    pub prediction_enabled: bool,
    pub adversarial_enabled: bool,
    pub architecture_enabled: bool,
}

impl Default for IntelligenceConfig {
    fn default() -> Self {
        Self {
            cache_ttl_seconds: 3600,
            prediction_enabled: true,
            adversarial_enabled: false,
            architecture_enabled: false,
        }
    }
}

/// GitHub integration configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GitHubConfig {
    pub watch_labels: Vec<String>,
    pub auto_merge: bool,
    pub check_runs_enabled: bool,
    pub deployment_tracking: bool,
}

impl Default for GitHubConfig {
    fn default() -> Self {
        Self {
            watch_labels: vec!["shipwright".into(), "ready-to-build".into()],
            auto_merge: false,
            check_runs_enabled: true,
            deployment_tracking: true,
        }
    }
}

/// Per-repository configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoConfig {
    pub path: PathBuf,
    pub owner: String,
    pub repo: String,
    #[serde(default = "default_template")]
    pub template: PipelineTemplateName,
    #[serde(default = "default_max_parallel")]
    pub max_parallel: u32,
    #[serde(default)]
    pub auto_merge: bool,
}

fn default_template() -> PipelineTemplateName {
    PipelineTemplateName::Standard
}

fn default_max_parallel() -> u32 {
    2
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config_serializes() {
        let config = ShipwrightConfig::default();
        let toml_str = toml::to_string_pretty(&config).unwrap();
        assert!(toml_str.contains("enabled = false"));
    }

    #[test]
    fn test_config_round_trip() {
        let config = ShipwrightConfig {
            enabled: true,
            default_template: PipelineTemplateName::Autonomous,
            repos: vec![RepoConfig {
                path: "/tmp/test".into(),
                owner: "myorg".into(),
                repo: "myrepo".into(),
                template: PipelineTemplateName::Full,
                max_parallel: 3,
                auto_merge: true,
            }],
            ..Default::default()
        };
        let toml_str = toml::to_string_pretty(&config).unwrap();
        let parsed: ShipwrightConfig = toml::from_str(&toml_str).unwrap();
        assert!(parsed.enabled);
        assert_eq!(parsed.repos.len(), 1);
        assert_eq!(parsed.repos[0].max_parallel, 3);
    }

    #[test]
    fn test_defaults_are_sane() {
        let config = ShipwrightConfig::default();
        assert!(!config.enabled);
        assert_eq!(config.fleet.max_workers, 8);
        assert_eq!(config.decision.max_issues_per_day, 15);
        assert_eq!(config.intelligence.cache_ttl_seconds, 3600);
        assert_eq!(
            config.github.watch_labels,
            vec!["shipwright", "ready-to-build"]
        );
    }

    #[test]
    fn test_partial_toml_uses_defaults() {
        let toml_str = r#"
            enabled = true
            [fleet]
            max_workers = 4
        "#;
        let config: ShipwrightConfig = toml::from_str(toml_str).unwrap();
        assert!(config.enabled);
        assert_eq!(config.fleet.max_workers, 4);
        assert_eq!(config.fleet.poll_interval_seconds, 60); // default
        assert!(!config.decision.enabled); // default
    }
}
