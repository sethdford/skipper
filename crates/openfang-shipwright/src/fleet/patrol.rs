//! Patrol: periodic checks that feed the decision engine with candidates.
//!
//! Detects security vulnerabilities, outdated dependencies, coverage regression,
//! and DORA metric degradation, feeding results back to decision engine as candidates.

use crate::decision::signals::Candidate;

/// Patrol finding type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatrolFinding {
    SecurityVulnerability,
    OutdatedDependency,
    CoverageRegression,
    DoraRegression,
}

/// Patrol check result.
#[derive(Debug, Clone)]
pub struct PatrolResult {
    pub repo: String,
    pub finding_type: PatrolFinding,
    pub severity: u8,
    pub description: String,
}

/// Patrol executor.
pub struct Patrol;

impl Patrol {
    /// Run all patrol checks on a repo.
    pub async fn run(repo: &str) -> Vec<PatrolResult> {
        let _repo = repo;
        let mut results = Vec::new();

        // Mock implementation: would call real scanners
        // - npm audit for Node
        // - cargo audit for Rust
        // - coverage reports
        // - GitHub deployment history for DORA

        results
    }

    /// Check for security vulnerabilities.
    pub async fn check_security(_repo: &str) -> Vec<PatrolResult> {
        // In real implementation: run npm audit, cargo audit, etc.
        vec![]
    }

    /// Check for outdated dependencies.
    pub async fn check_dependencies(_repo: &str) -> Vec<PatrolResult> {
        // In real implementation: run npm outdated, cargo update --dry-run, etc.
        vec![]
    }

    /// Check for coverage regression.
    pub async fn check_coverage(_repo: &str, _threshold: f64) -> Vec<PatrolResult> {
        // In real implementation: read coverage reports and compare to baseline
        vec![]
    }

    /// Check for DORA metric regression.
    pub async fn check_dora(_repo: &str) -> Vec<PatrolResult> {
        // In real implementation: fetch from database and compare trend
        vec![]
    }

    /// Convert patrol result to decision candidate.
    pub fn to_candidate(result: &PatrolResult) -> Candidate {
        let (signal_type, category) = match result.finding_type {
            PatrolFinding::SecurityVulnerability => {
                (crate::decision::signals::SignalType::Security,
                 crate::decision::signals::Category::SecurityPatch)
            }
            PatrolFinding::OutdatedDependency => {
                (crate::decision::signals::SignalType::Dependency,
                 crate::decision::signals::Category::DependencyUpdate)
            }
            PatrolFinding::CoverageRegression => {
                (crate::decision::signals::SignalType::Coverage,
                 crate::decision::signals::Category::BugFix)
            }
            PatrolFinding::DoraRegression => {
                (crate::decision::signals::SignalType::Dora,
                 crate::decision::signals::Category::Performance)
            }
        };

        Candidate::new(
            signal_type,
            category,
            result.description.clone(),
            format!("Patrol finding: {}", result.description),
            format!("patrol-{}-{}", result.repo, result.finding_type as u8),
        )
        .with_risk_score(result.severity)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_patrol_run_empty() {
        let results = Patrol::run("test-repo").await;
        // Mock returns empty; real implementation would have findings
        assert_eq!(results.len(), 0);
    }

    #[tokio::test]
    async fn test_patrol_check_security_empty() {
        let results = Patrol::check_security("test-repo").await;
        assert_eq!(results.len(), 0);
    }

    #[tokio::test]
    async fn test_patrol_check_dependencies_empty() {
        let results = Patrol::check_dependencies("test-repo").await;
        assert_eq!(results.len(), 0);
    }

    #[tokio::test]
    async fn test_patrol_check_coverage_empty() {
        let results = Patrol::check_coverage("test-repo", 80.0).await;
        assert_eq!(results.len(), 0);
    }

    #[tokio::test]
    async fn test_patrol_check_dora_empty() {
        let results = Patrol::check_dora("test-repo").await;
        assert_eq!(results.len(), 0);
    }

    #[test]
    fn test_to_candidate_security() {
        let result = PatrolResult {
            repo: "test-repo".to_string(),
            finding_type: PatrolFinding::SecurityVulnerability,
            severity: 90,
            description: "Critical CVE".to_string(),
        };

        let candidate = Patrol::to_candidate(&result);
        assert_eq!(candidate.description, "Patrol finding: Critical CVE");
        assert_eq!(candidate.risk_score, 90);
    }

    #[test]
    fn test_to_candidate_dependency() {
        let result = PatrolResult {
            repo: "test-repo".to_string(),
            finding_type: PatrolFinding::OutdatedDependency,
            severity: 30,
            description: "lodash 4.17.15 outdated".to_string(),
        };

        let candidate = Patrol::to_candidate(&result);
        assert_eq!(candidate.risk_score, 30);
    }

    #[test]
    fn test_patrol_result_new() {
        let result = PatrolResult {
            repo: "owner/repo".to_string(),
            finding_type: PatrolFinding::CoverageRegression,
            severity: 50,
            description: "Coverage dropped from 85% to 79%".to_string(),
        };

        assert_eq!(result.repo, "owner/repo");
        assert_eq!(result.finding_type, PatrolFinding::CoverageRegression);
        assert_eq!(result.severity, 50);
    }
}
