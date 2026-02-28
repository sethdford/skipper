//! Signal collectors for the decision engine.
//!
//! Collectors gather signals from various sources (security, dependencies, coverage, etc.)
//! and produce candidates for scoring and tier assignment.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A signal source type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SignalType {
    Security,
    Dependency,
    Coverage,
    DeadCode,
    Performance,
    Architecture,
    Dora,
    Documentation,
    Failure,
    OpenFangHand,
}

impl std::fmt::Display for SignalType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                SignalType::Security => "security",
                SignalType::Dependency => "dependency",
                SignalType::Coverage => "coverage",
                SignalType::DeadCode => "dead_code",
                SignalType::Performance => "performance",
                SignalType::Architecture => "architecture",
                SignalType::Dora => "dora",
                SignalType::Documentation => "documentation",
                SignalType::Failure => "failure",
                SignalType::OpenFangHand => "openfang_hand",
            }
        )
    }
}

/// A category for prioritization.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Category {
    SecurityPatch,
    DependencyUpdate,
    BugFix,
    Performance,
    Refactoring,
    Feature,
}

impl std::fmt::Display for Category {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                Category::SecurityPatch => "security_patch",
                Category::DependencyUpdate => "dependency_update",
                Category::BugFix => "bug_fix",
                Category::Performance => "performance",
                Category::Refactoring => "refactoring",
                Category::Feature => "feature",
            }
        )
    }
}

/// A candidate for automation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Candidate {
    pub id: String,
    pub signal: SignalType,
    pub category: Category,
    pub title: String,
    pub description: String,
    pub evidence: serde_json::Value,
    pub risk_score: u8,
    pub confidence: f64,
    pub dedup_key: String,
    pub impact: f64,
    pub urgency: f64,
    pub effort: f64,
}

impl Candidate {
    /// Create a new candidate.
    pub fn new(
        signal: SignalType,
        category: Category,
        title: String,
        description: String,
        dedup_key: String,
    ) -> Self {
        let id = uuid::Uuid::new_v4().to_string();
        Self {
            id,
            signal,
            category,
            title,
            description,
            evidence: serde_json::Value::Null,
            risk_score: 50,
            confidence: 0.8,
            dedup_key,
            impact: 0.5,
            urgency: 0.5,
            effort: 0.5,
        }
    }

    /// Set evidence data.
    pub fn with_evidence(mut self, evidence: serde_json::Value) -> Self {
        self.evidence = evidence;
        self
    }

    /// Set risk score (0-100), clamped to valid range.
    pub fn with_risk_score(mut self, score: u8) -> Self {
        self.risk_score = score.clamp(0, 100);
        self
    }

    /// Set confidence (0.0-1.0).
    pub fn with_confidence(mut self, conf: f64) -> Self {
        self.confidence = conf.clamp(0.0, 1.0);
        self
    }

    /// Set impact (0.0-1.0).
    pub fn with_impact(mut self, impact: f64) -> Self {
        self.impact = impact.clamp(0.0, 1.0);
        self
    }

    /// Set urgency (0.0-1.0).
    pub fn with_urgency(mut self, urgency: f64) -> Self {
        self.urgency = urgency.clamp(0.0, 1.0);
        self
    }

    /// Set effort (0.0-1.0).
    pub fn with_effort(mut self, effort: f64) -> Self {
        self.effort = effort.clamp(0.0, 1.0);
        self
    }
}

/// Repository context for signal collection.
#[derive(Debug, Clone)]
pub struct RepoContext {
    pub repo: String,
    pub owner: String,
    pub path: String,
    pub hotspots: Vec<String>,
    pub dependencies: HashMap<String, String>,
}

impl RepoContext {
    /// Create a new repo context.
    pub fn new(repo: String, owner: String, path: String) -> Self {
        Self {
            repo,
            owner,
            path,
            hotspots: vec![],
            dependencies: HashMap::new(),
        }
    }
}

/// A signal collector trait.
pub trait SignalCollector: Send + Sync {
    /// Get the collector's name.
    fn name(&self) -> &str;

    /// Collect candidates from this source.
    fn collect(&self, ctx: &RepoContext) -> Result<Vec<Candidate>, String>;
}

/// Security vulnerability collector.
pub struct SecurityCollector;

impl SignalCollector for SecurityCollector {
    fn name(&self) -> &str {
        "security"
    }

    fn collect(&self, _ctx: &RepoContext) -> Result<Vec<Candidate>, String> {
        // In a real implementation, this would run npm audit, cargo audit, etc.
        Ok(vec![])
    }
}

/// Dependency update collector.
pub struct DependencyCollector;

impl SignalCollector for DependencyCollector {
    fn name(&self) -> &str {
        "dependency"
    }

    fn collect(&self, _ctx: &RepoContext) -> Result<Vec<Candidate>, String> {
        // In a real implementation, this would check for outdated dependencies
        Ok(vec![])
    }
}

/// Code coverage collector.
pub struct CoverageCollector {
    pub threshold_percent: f64,
}

impl SignalCollector for CoverageCollector {
    fn name(&self) -> &str {
        "coverage"
    }

    fn collect(&self, _ctx: &RepoContext) -> Result<Vec<Candidate>, String> {
        // In a real implementation, this would read coverage reports
        Ok(vec![])
    }
}

/// Dead code collector.
pub struct DeadCodeCollector;

impl SignalCollector for DeadCodeCollector {
    fn name(&self) -> &str {
        "dead_code"
    }

    fn collect(&self, _ctx: &RepoContext) -> Result<Vec<Candidate>, String> {
        Ok(vec![])
    }
}

/// Performance regression collector.
pub struct PerformanceCollector {
    pub threshold_percent: f64,
}

impl SignalCollector for PerformanceCollector {
    fn name(&self) -> &str {
        "performance"
    }

    fn collect(&self, _ctx: &RepoContext) -> Result<Vec<Candidate>, String> {
        Ok(vec![])
    }
}

/// Architecture violation collector.
pub struct ArchitectureCollector;

impl SignalCollector for ArchitectureCollector {
    fn name(&self) -> &str {
        "architecture"
    }

    fn collect(&self, _ctx: &RepoContext) -> Result<Vec<Candidate>, String> {
        Ok(vec![])
    }
}

/// DORA metric degradation collector.
pub struct DoraCollector;

impl SignalCollector for DoraCollector {
    fn name(&self) -> &str {
        "dora"
    }

    fn collect(&self, _ctx: &RepoContext) -> Result<Vec<Candidate>, String> {
        Ok(vec![])
    }
}

/// Documentation collector.
pub struct DocumentationCollector;

impl SignalCollector for DocumentationCollector {
    fn name(&self) -> &str {
        "documentation"
    }

    fn collect(&self, _ctx: &RepoContext) -> Result<Vec<Candidate>, String> {
        Ok(vec![])
    }
}

/// Failure pattern collector.
pub struct FailureCollector;

impl SignalCollector for FailureCollector {
    fn name(&self) -> &str {
        "failure"
    }

    fn collect(&self, _ctx: &RepoContext) -> Result<Vec<Candidate>, String> {
        Ok(vec![])
    }
}

/// OpenFang Hand signal collector.
pub struct HandSignalCollector;

impl SignalCollector for HandSignalCollector {
    fn name(&self) -> &str {
        "openfang_hand"
    }

    fn collect(&self, _ctx: &RepoContext) -> Result<Vec<Candidate>, String> {
        // In a real implementation, this would query Collector, Researcher, Predictor hands
        Ok(vec![])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_candidate_new() {
        let candidate = Candidate::new(
            SignalType::Security,
            Category::SecurityPatch,
            "CVE-2024-1234".to_string(),
            "Critical vulnerability in dependency".to_string(),
            "cve-2024-1234".to_string(),
        );
        assert_eq!(candidate.signal, SignalType::Security);
        assert_eq!(candidate.category, Category::SecurityPatch);
        assert_eq!(candidate.risk_score, 50);
        assert_eq!(candidate.confidence, 0.8);
    }

    #[test]
    fn test_candidate_with_builders() {
        let candidate = Candidate::new(
            SignalType::Security,
            Category::SecurityPatch,
            "Critical CVE".to_string(),
            "description".to_string(),
            "cve-key".to_string(),
        )
        .with_risk_score(95)
        .with_confidence(0.99)
        .with_impact(0.95)
        .with_urgency(0.90)
        .with_effort(0.3);

        assert_eq!(candidate.risk_score, 95);
        assert_eq!(candidate.confidence, 0.99);
        assert_eq!(candidate.impact, 0.95);
        assert_eq!(candidate.urgency, 0.90);
        assert_eq!(candidate.effort, 0.3);
    }

    #[test]
    fn test_confidence_clamped() {
        let candidate = Candidate::new(
            SignalType::Security,
            Category::SecurityPatch,
            "test".to_string(),
            "test".to_string(),
            "test".to_string(),
        )
        .with_confidence(1.5);

        assert_eq!(candidate.confidence, 1.0);
    }

    #[test]
    fn test_confidence_negative_clamped() {
        let candidate = Candidate::new(
            SignalType::Security,
            Category::SecurityPatch,
            "test".to_string(),
            "test".to_string(),
            "test".to_string(),
        )
        .with_confidence(-0.5);

        assert_eq!(candidate.confidence, 0.0);
    }

    #[test]
    fn test_med003_risk_score_clamped_over_100() {
        // MED-003: risk_score should clamp to 0-100 range
        let candidate = Candidate::new(
            SignalType::Security,
            Category::SecurityPatch,
            "test".to_string(),
            "test".to_string(),
            "test".to_string(),
        )
        .with_risk_score(255);

        assert_eq!(
            candidate.risk_score, 100,
            "Risk score > 100 should be clamped to 100"
        );
    }

    #[test]
    fn test_med003_risk_score_clamped_under_0() {
        // MED-003: risk_score should clamp to 0-100 range
        // Note: u8 can't be negative, but we test the clamping logic is in place
        let candidate = Candidate::new(
            SignalType::Security,
            Category::SecurityPatch,
            "test".to_string(),
            "test".to_string(),
            "test".to_string(),
        )
        .with_risk_score(0);

        assert_eq!(candidate.risk_score, 0, "Risk score 0 should be accepted");
    }

    #[test]
    fn test_signal_type_display() {
        assert_eq!(SignalType::Security.to_string(), "security");
        assert_eq!(SignalType::Dependency.to_string(), "dependency");
        assert_eq!(SignalType::OpenFangHand.to_string(), "openfang_hand");
    }

    #[test]
    fn test_category_display() {
        assert_eq!(Category::SecurityPatch.to_string(), "security_patch");
        assert_eq!(Category::Feature.to_string(), "feature");
    }

    #[test]
    fn test_security_collector() {
        let collector = SecurityCollector;
        assert_eq!(collector.name(), "security");
        let ctx = RepoContext::new("repo".to_string(), "owner".to_string(), "/path".to_string());
        let candidates = collector.collect(&ctx).unwrap();
        assert_eq!(candidates.len(), 0);
    }

    #[test]
    fn test_dependency_collector() {
        let collector = DependencyCollector;
        assert_eq!(collector.name(), "dependency");
    }

    #[test]
    fn test_coverage_collector() {
        let collector = CoverageCollector {
            threshold_percent: 80.0,
        };
        assert_eq!(collector.name(), "coverage");
        assert_eq!(collector.threshold_percent, 80.0);
    }

    #[test]
    fn test_dead_code_collector() {
        let collector = DeadCodeCollector;
        assert_eq!(collector.name(), "dead_code");
    }

    #[test]
    fn test_performance_collector() {
        let collector = PerformanceCollector {
            threshold_percent: 10.0,
        };
        assert_eq!(collector.name(), "performance");
    }

    #[test]
    fn test_architecture_collector() {
        let collector = ArchitectureCollector;
        assert_eq!(collector.name(), "architecture");
    }

    #[test]
    fn test_dora_collector() {
        let collector = DoraCollector;
        assert_eq!(collector.name(), "dora");
    }

    #[test]
    fn test_documentation_collector() {
        let collector = DocumentationCollector;
        assert_eq!(collector.name(), "documentation");
    }

    #[test]
    fn test_failure_collector() {
        let collector = FailureCollector;
        assert_eq!(collector.name(), "failure");
    }

    #[test]
    fn test_hand_signal_collector() {
        let collector = HandSignalCollector;
        assert_eq!(collector.name(), "openfang_hand");
    }

    #[test]
    fn test_repo_context() {
        let ctx = RepoContext::new("repo".to_string(), "owner".to_string(), "/path".to_string());
        assert_eq!(ctx.repo, "repo");
        assert_eq!(ctx.owner, "owner");
        assert_eq!(ctx.path, "/path");
        assert!(ctx.hotspots.is_empty());
        assert!(ctx.dependencies.is_empty());
    }
}
