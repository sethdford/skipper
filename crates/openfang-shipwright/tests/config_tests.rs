//! Configuration validation tests.

use openfang_shipwright::config::{
    ShipwrightConfig, PipelineTemplateName, FleetConfig, DecisionConfig,
    IntelligenceConfig, GitHubConfig, RepoConfig,
};
use std::path::PathBuf;

#[test]
fn config_default_values() {
    let config = ShipwrightConfig::default();

    assert!(!config.enabled);
    assert_eq!(config.default_template, PipelineTemplateName::Standard);
    assert_eq!(config.repos.len(), 0);
}

#[test]
fn fleet_config_default() {
    let config = FleetConfig::default();

    assert_eq!(config.poll_interval_seconds, 60);
    assert!(!config.auto_scale);
    assert_eq!(config.max_workers, 8);
}

#[test]
fn decision_config_default() {
    let config = DecisionConfig::default();

    assert!(!config.enabled);
    assert_eq!(config.cycle_interval_seconds, 1800);
    assert_eq!(config.max_issues_per_day, 15);
}

#[test]
fn intelligence_config_default() {
    let config = IntelligenceConfig::default();

    assert_eq!(config.cache_ttl_seconds, 3600);
    assert!(config.prediction_enabled);
}

#[test]
fn github_config_default() {
    let config = GitHubConfig::default();

    assert_eq!(config.watch_labels.len(), 2);
    assert!(!config.auto_merge);
}

#[test]
fn repo_config_creation() {
    let repo = RepoConfig {
        path: PathBuf::from("/path/to/repo"),
        owner: "myorg".to_string(),
        repo: "myrepo".to_string(),
        template: PipelineTemplateName::Standard,
        max_parallel: 3,
        auto_merge: true,
    };

    assert_eq!(repo.owner, "myorg");
    assert_eq!(repo.repo, "myrepo");
    assert_eq!(repo.max_parallel, 3);
}

