//! Architecture rules and enforcement.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A dependency rule between layers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DependencyRule {
    pub from: String,
    pub to: String,
    pub allowed: bool,
}

/// Architecture rules for a repository.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchitectureRule {
    pub id: String,
    pub repo: String,
    pub layers: Vec<String>,
    pub dependency_rules: Vec<DependencyRule>,
    pub hotspots: HashMap<String, u32>,
    pub conventions: Vec<String>,
    pub created_at: String,
}

impl ArchitectureRule {
    /// Create a new architecture rule.
    pub fn new(repo: String) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            repo,
            layers: vec![],
            dependency_rules: vec![],
            hotspots: HashMap::new(),
            conventions: vec![],
            created_at: chrono::Utc::now().to_rfc3339(),
        }
    }

    /// Add a layer to the architecture.
    pub fn with_layer(mut self, layer: String) -> Self {
        self.layers.push(layer);
        self
    }

    /// Add a dependency rule.
    pub fn with_dependency_rule(mut self, from: String, to: String, allowed: bool) -> Self {
        self.dependency_rules.push(DependencyRule { from, to, allowed });
        self
    }

    /// Add a convention.
    pub fn with_convention(mut self, convention: String) -> Self {
        self.conventions.push(convention);
        self
    }

    /// Add a hotspot file.
    pub fn add_hotspot(&mut self, file: String, change_count: u32) {
        self.hotspots.insert(file, change_count);
    }

    /// Check if a dependency is allowed.
    pub fn is_dependency_allowed(&self, from: &str, to: &str) -> bool {
        // If no rules defined, allow all
        if self.dependency_rules.is_empty() {
            return true;
        }

        // Check explicit rules
        for rule in &self.dependency_rules {
            if rule.from == from && rule.to == to {
                return rule.allowed;
            }
        }

        // Default: deny if rules exist but no matching rule
        false
    }

    /// Get top N hotspot files.
    pub fn get_hotspots(&self, limit: usize) -> Vec<(String, u32)> {
        let mut hotspots: Vec<_> = self.hotspots.iter().map(|(k, v)| (k.clone(), *v)).collect();
        hotspots.sort_by(|a, b| b.1.cmp(&a.1));
        hotspots.into_iter().take(limit).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_architecture_rule_creation() {
        let rule = ArchitectureRule::new("myrepo".to_string());
        assert_eq!(rule.repo, "myrepo");
        assert!(rule.layers.is_empty());
    }

    #[test]
    fn test_add_layers() {
        let rule = ArchitectureRule::new("myrepo".to_string())
            .with_layer("api".to_string())
            .with_layer("db".to_string());
        assert_eq!(rule.layers.len(), 2);
        assert!(rule.layers.contains(&"api".to_string()));
    }

    #[test]
    fn test_dependency_allowed() {
        let rule = ArchitectureRule::new("myrepo".to_string()).with_dependency_rule(
            "api".to_string(),
            "db".to_string(),
            true,
        );
        assert!(rule.is_dependency_allowed("api", "db"));
        assert!(!rule.is_dependency_allowed("api", "cache"));
    }

    #[test]
    fn test_dependency_denied() {
        let rule = ArchitectureRule::new("myrepo".to_string()).with_dependency_rule(
            "api".to_string(),
            "db".to_string(),
            false,
        );
        assert!(!rule.is_dependency_allowed("api", "db"));
    }

    #[test]
    fn test_add_hotspots() {
        let mut rule = ArchitectureRule::new("myrepo".to_string());
        rule.add_hotspot("src/main.rs".to_string(), 15);
        rule.add_hotspot("src/lib.rs".to_string(), 8);
        rule.add_hotspot("src/utils.rs".to_string(), 25);

        let hotspots = rule.get_hotspots(2);
        assert_eq!(hotspots.len(), 2);
        assert_eq!(hotspots[0].0, "src/utils.rs");
        assert_eq!(hotspots[0].1, 25);
    }

    #[test]
    fn test_add_conventions() {
        let rule = ArchitectureRule::new("myrepo".to_string())
            .with_convention("use Result<T> for errors".to_string())
            .with_convention("handlers in handlers/ dir".to_string());
        assert_eq!(rule.conventions.len(), 2);
    }

    #[test]
    fn test_architecture_rule_serialize() {
        let rule = ArchitectureRule::new("myrepo".to_string())
            .with_layer("api".to_string())
            .with_layer("db".to_string());
        let json = serde_json::to_string(&rule).unwrap();
        assert!(json.contains("\"repo\":\"myrepo\""));
    }
}
