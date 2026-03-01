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
    SkipperHand,
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
                SignalType::SkipperHand => "skipper_hand",
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

impl SecurityCollector {
    /// Parse npm audit JSON output into candidates.
    ///
    /// Expects format: `{ "vulnerabilities": { "package-name": { "fixAvailable": bool, "severity": "high"|"critical", ... } } }`
    pub fn parse_npm_audit(output: &str) -> Result<Vec<Candidate>, String> {
        let audit_json: serde_json::Value =
            serde_json::from_str(output).map_err(|e| format!("Failed to parse npm audit: {}", e))?;

        let mut candidates = vec![];

        if let Some(vuln_obj) = audit_json.get("vulnerabilities").and_then(|v| v.as_object()) {
            for (package_name, details) in vuln_obj.iter() {
                let severity = details
                    .get("severity")
                    .and_then(|s| s.as_str())
                    .unwrap_or("medium");

                let is_fixable = details
                    .get("fixAvailable")
                    .and_then(|f| f.as_bool())
                    .unwrap_or(false);

                let (risk_score, urgency, impact) = match severity {
                    "critical" => (95, 1.0, 1.0),
                    "high" => (80, 0.9, 0.8),
                    "moderate" => (50, 0.5, 0.5),
                    "low" => (20, 0.2, 0.3),
                    _ => (40, 0.4, 0.4),
                };

                candidates.push(
                    Candidate::new(
                        SignalType::Security,
                        Category::SecurityPatch,
                        format!("Update vulnerable dependency: {}", package_name),
                        format!(
                            "Security {} vulnerability found in {} ({})",
                            severity, package_name,
                            if is_fixable { "fixable" } else { "no fix available" }
                        ),
                        format!("security-{}-{}", package_name, severity),
                    )
                    .with_risk_score(risk_score)
                    .with_urgency(urgency)
                    .with_impact(impact)
                    .with_confidence(if is_fixable { 0.95 } else { 0.7 })
                    .with_evidence(serde_json::json!({
                        "package": package_name,
                        "severity": severity,
                        "fixable": is_fixable,
                    })),
                );
            }
        }

        Ok(candidates)
    }
}

impl SignalCollector for SecurityCollector {
    fn name(&self) -> &str {
        "security"
    }

    fn collect(&self, _ctx: &RepoContext) -> Result<Vec<Candidate>, String> {
        // In a real implementation, this would run npm audit or cargo audit
        // For now, return empty to indicate no vulnerabilities
        Ok(vec![])
    }
}

/// Dependency update collector.
pub struct DependencyCollector;

impl DependencyCollector {
    /// Classify the bump type based on version difference.
    ///
    /// Examples: "1.2.3" -> "2.0.0" = Major, "1.2.3" -> "1.3.0" = Minor, "1.2.3" -> "1.2.4" = Patch
    pub fn classify_bump(from: &str, to: &str) -> &'static str {
        let from_parts: Vec<&str> = from.split('.').collect();
        let to_parts: Vec<&str> = to.split('.').collect();

        if from_parts.is_empty() || to_parts.is_empty() {
            return "unknown";
        }

        if from_parts.first() != to_parts.first() {
            return "major";
        }
        if from_parts.get(1) != to_parts.get(1) {
            return "minor";
        }
        if from_parts.get(2) != to_parts.get(2) {
            return "patch";
        }
        "none"
    }

    /// Parse outdated dependencies output into candidates.
    ///
    /// Expected format: `package-name: current=1.2.3, latest=1.2.4`
    pub fn parse_outdated(output: &str) -> Vec<Candidate> {
        let mut candidates = vec![];

        for line in output.lines() {
            if line.trim().is_empty() {
                continue;
            }

            // Simple parser: "package-name: current=X.Y.Z, latest=A.B.C"
            if let Some((package, versions)) = line.split_once(':') {
                let package_name = package.trim();
                let mut current = "";
                let mut latest = "";

                for part in versions.split(',') {
                    if part.contains("current=") {
                        current = part.split('=').nth(1).map(|s| s.trim()).unwrap_or("");
                    }
                    if part.contains("latest=") {
                        latest = part.split('=').nth(1).map(|s| s.trim()).unwrap_or("");
                    }
                }

                if !current.is_empty() && !latest.is_empty() {
                    let bump_type = Self::classify_bump(current, latest);

                    let (effort, urgency) = match bump_type {
                        "major" => (0.8, 0.5),  // Major bumps are risky
                        "minor" => (0.4, 0.6),  // Minor is low effort, moderate urgency
                        "patch" => (0.1, 0.8),  // Patch is easy and important
                        _ => (0.5, 0.5),
                    };

                    candidates.push(
                        Candidate::new(
                            SignalType::Dependency,
                            Category::DependencyUpdate,
                            format!("Update {} to {}", package_name, latest),
                            format!(
                                "Dependency {} has a {} update: {} -> {}",
                                package_name, bump_type, current, latest
                            ),
                            format!("dep-{}-{}", package_name, latest),
                        )
                        .with_urgency(urgency)
                        .with_effort(effort)
                        .with_confidence(0.9)
                        .with_risk_score(match bump_type {
                            "major" => 60,
                            "minor" => 20,
                            "patch" => 5,
                            _ => 30,
                        })
                        .with_evidence(serde_json::json!({
                            "package": package_name,
                            "current": current,
                            "latest": latest,
                            "type": bump_type,
                        })),
                    );
                }
            }
        }

        candidates
    }
}

impl SignalCollector for DependencyCollector {
    fn name(&self) -> &str {
        "dependency"
    }

    fn collect(&self, _ctx: &RepoContext) -> Result<Vec<Candidate>, String> {
        // In a real implementation, this would run npm outdated, cargo outdated, etc.
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

/// Skipper Hand signal collector.
pub struct HandSignalCollector;

impl SignalCollector for HandSignalCollector {
    fn name(&self) -> &str {
        "skipper_hand"
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
    fn test_security_collector_parse_npm_audit() {
        let npm_output = r#"{
  "vulnerabilities": {
    "lodash": {
      "severity": "high",
      "fixAvailable": true
    },
    "moment": {
      "severity": "critical",
      "fixAvailable": true
    }
  }
}"#;

        let candidates = SecurityCollector::parse_npm_audit(npm_output).unwrap();
        assert_eq!(candidates.len(), 2);

        let high_vuln = candidates.iter().find(|c| c.title.contains("lodash")).unwrap();
        assert_eq!(high_vuln.risk_score, 80);
        assert_eq!(high_vuln.urgency, 0.9);

        let critical_vuln = candidates.iter().find(|c| c.title.contains("moment")).unwrap();
        assert_eq!(critical_vuln.risk_score, 95);
        assert_eq!(critical_vuln.urgency, 1.0);
    }

    #[test]
    fn test_dependency_collector_classify_bump() {
        assert_eq!(DependencyCollector::classify_bump("1.2.3", "2.0.0"), "major");
        assert_eq!(DependencyCollector::classify_bump("1.2.3", "1.3.0"), "minor");
        assert_eq!(DependencyCollector::classify_bump("1.2.3", "1.2.4"), "patch");
        assert_eq!(DependencyCollector::classify_bump("1.2.3", "1.2.3"), "none");
    }

    #[test]
    fn test_dependency_collector_parse_outdated() {
        let output = "lodash: current=4.17.20, latest=4.17.21\nmoment: current=2.29.0, latest=2.29.4";
        let candidates = DependencyCollector::parse_outdated(output);
        assert_eq!(candidates.len(), 2);

        let patch_bump = candidates
            .iter()
            .find(|c| c.title.contains("moment"))
            .unwrap();
        assert_eq!(patch_bump.effort, 0.1);  // Patch is easy
        assert_eq!(patch_bump.urgency, 0.8); // Patch is urgent

        let patch_bump2 = candidates
            .iter()
            .find(|c| c.title.contains("lodash"))
            .unwrap();
        assert_eq!(patch_bump2.effort, 0.1);
        assert_eq!(patch_bump2.risk_score, 5);  // Patch has low risk
    }

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
        assert_eq!(SignalType::SkipperHand.to_string(), "skipper_hand");
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
        assert_eq!(collector.name(), "skipper_hand");
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
