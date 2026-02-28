//! Shipwright memory system with vector search and learning.

pub mod architecture;
pub mod learning;
pub mod patterns;

pub use architecture::ArchitectureRule;
pub use learning::{Outcome, ScoringWeights, ABTest, ABGroup};
pub use patterns::FailurePattern;

/// Shipwright memory store.
pub struct ShipwrightMemory {
    // In a real implementation, this would wrap openfang_memory::MemoryStore
    // For now, we use in-memory collections for testing
    failure_patterns: Vec<FailurePattern>,
    architecture_rules: Vec<ArchitectureRule>,
    outcomes: Vec<Outcome>,
}

impl ShipwrightMemory {
    /// Create a new memory store.
    pub fn new() -> Self {
        Self {
            failure_patterns: vec![],
            architecture_rules: vec![],
            outcomes: vec![],
        }
    }

    /// Store a failure pattern.
    pub fn store_failure(&mut self, pattern: FailurePattern) {
        self.failure_patterns.push(pattern);
    }

    /// Search for similar failures by error text.
    pub fn search_similar_failures(
        &self,
        error_text: &str,
        repo: &str,
        limit: usize,
    ) -> Vec<FailurePattern> {
        self.failure_patterns
            .iter()
            .filter(|p| p.repo == repo && p.error_signature.to_lowercase().contains(&error_text.to_lowercase()))
            .take(limit)
            .cloned()
            .collect()
    }

    /// Compose context from similar failures.
    pub fn compose_context(&self, error: &str, repo: &str) -> String {
        let patterns = self.search_similar_failures(error, repo, 3);
        if patterns.is_empty() {
            return "No similar failures found in memory.".to_string();
        }

        patterns
            .iter()
            .map(|p| format!(
                "Previously fixed similar error:\n  Cause: {}\n  Fix: {}",
                p.root_cause, p.fix_applied
            ))
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    /// Store an architecture rule.
    pub fn store_architecture(&mut self, rule: ArchitectureRule) {
        self.architecture_rules.push(rule);
    }

    /// Get architecture rule for a repo.
    pub fn get_architecture(&self, repo: &str) -> Option<ArchitectureRule> {
        self.architecture_rules.iter().find(|r| r.repo == repo).cloned()
    }

    /// Check for dependency violation.
    pub fn check_dependency_violation(&self, repo: &str, from: &str, to: &str) -> bool {
        if let Some(rule) = self.get_architecture(repo) {
            !rule.is_dependency_allowed(from, to)
        } else {
            false
        }
    }

    /// Get hotspot files for a repo.
    pub fn get_hotspots(&self, repo: &str, limit: usize) -> Vec<(String, u32)> {
        if let Some(rule) = self.get_architecture(repo) {
            rule.get_hotspots(limit)
        } else {
            vec![]
        }
    }

    /// Record an outcome.
    pub fn record_outcome(&mut self, outcome: Outcome) {
        self.outcomes.push(outcome);
    }

    /// Get outcomes for a signal source.
    pub fn get_outcomes_for_signal(&self, signal_source: &str) -> Vec<Outcome> {
        self.outcomes
            .iter()
            .filter(|o| o.signal_source == signal_source)
            .cloned()
            .collect()
    }
}

impl Default for ShipwrightMemory {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_memory_store_failure() {
        let mut memory = ShipwrightMemory::new();
        let pattern = FailurePattern::new(
            "repo".to_string(),
            "build".to_string(),
            "Error".to_string(),
            "undefined variable".to_string(),
            "cause".to_string(),
            "fix".to_string(),
        );
        memory.store_failure(pattern.clone());
        assert_eq!(memory.failure_patterns.len(), 1);
    }

    #[test]
    fn test_search_similar_failures() {
        let mut memory = ShipwrightMemory::new();
        let pattern = FailurePattern::new(
            "repo".to_string(),
            "build".to_string(),
            "Error".to_string(),
            "undefined property".to_string(),
            "cause".to_string(),
            "fix".to_string(),
        );
        memory.store_failure(pattern);

        let results = memory.search_similar_failures("undefined property", "repo", 10);
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_compose_context() {
        let mut memory = ShipwrightMemory::new();
        let pattern = FailurePattern::new(
            "repo".to_string(),
            "build".to_string(),
            "Error".to_string(),
            "undefined property".to_string(),
            "missing initialization".to_string(),
            "initialize before use".to_string(),
        );
        memory.store_failure(pattern);

        let context = memory.compose_context("undefined property", "repo");
        assert!(context.contains("Previously fixed similar error"));
    }

    #[test]
    fn test_empty_database_returns_empty() {
        let memory = ShipwrightMemory::new();
        let results = memory.search_similar_failures("anything", "repo", 10);
        assert!(results.is_empty());
    }

    #[test]
    fn test_store_architecture() {
        let mut memory = ShipwrightMemory::new();
        let rule = ArchitectureRule::new("repo".to_string());
        memory.store_architecture(rule);
        assert_eq!(memory.architecture_rules.len(), 1);
    }

    #[test]
    fn test_check_dependency_violation() {
        let mut memory = ShipwrightMemory::new();
        let rule = ArchitectureRule::new("repo".to_string()).with_dependency_rule(
            "api".to_string(),
            "db".to_string(),
            false,
        );
        memory.store_architecture(rule);

        assert!(memory.check_dependency_violation("repo", "api", "db"));
    }

    #[test]
    fn test_get_hotspots() {
        let mut memory = ShipwrightMemory::new();
        let mut rule = ArchitectureRule::new("repo".to_string());
        rule.add_hotspot("file1.rs".to_string(), 10);
        rule.add_hotspot("file2.rs".to_string(), 5);
        memory.store_architecture(rule);

        let hotspots = memory.get_hotspots("repo", 1);
        assert_eq!(hotspots.len(), 1);
        assert_eq!(hotspots[0].0, "file1.rs");
    }

    #[test]
    fn test_record_outcome() {
        let mut memory = ShipwrightMemory::new();
        let outcome = Outcome::new("cand-1".to_string(), 75.0, "security".to_string());
        memory.record_outcome(outcome);
        assert_eq!(memory.outcomes.len(), 1);
    }

    #[test]
    fn test_get_outcomes_for_signal() {
        let mut memory = ShipwrightMemory::new();
        let outcome1 = Outcome::new("c1".to_string(), 75.0, "security".to_string());
        let outcome2 = Outcome::new("c2".to_string(), 60.0, "dependency".to_string());
        memory.record_outcome(outcome1);
        memory.record_outcome(outcome2);

        let security_outcomes = memory.get_outcomes_for_signal("security");
        assert_eq!(security_outcomes.len(), 1);
    }
}
