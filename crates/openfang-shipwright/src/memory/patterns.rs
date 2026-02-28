//! Failure pattern storage and semantic search.

use serde::{Deserialize, Serialize};

/// A failure pattern learned from a previous run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FailurePattern {
    pub id: String,
    pub repo: String,
    pub stage: String,
    pub error_class: String,
    pub error_signature: String,
    pub root_cause: String,
    pub fix_applied: String,
    pub fix_commit: Option<String>,
    pub success: bool,
    pub embedding: Vec<f32>,
    pub created_at: String,
}

impl FailurePattern {
    /// Create a new failure pattern.
    pub fn new(
        repo: String,
        stage: String,
        error_class: String,
        error_signature: String,
        root_cause: String,
        fix_applied: String,
    ) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            repo,
            stage,
            error_class,
            error_signature,
            root_cause,
            fix_applied,
            fix_commit: None,
            success: false,
            embedding: vec![],
            created_at: chrono::Utc::now().to_rfc3339(),
        }
    }

    /// Mark the pattern as successful.
    pub fn mark_successful(mut self) -> Self {
        self.success = true;
        self
    }

    /// Set the fix commit SHA.
    pub fn with_commit(mut self, commit: String) -> Self {
        self.fix_commit = Some(commit);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_failure_pattern_creation() {
        let pattern = FailurePattern::new(
            "myrepo".to_string(),
            "build".to_string(),
            "CompilationError".to_string(),
            "unused variable 'x'".to_string(),
            "Removed variable x".to_string(),
            "let x = 1; -> let _x = 1;".to_string(),
        );
        assert_eq!(pattern.repo, "myrepo");
        assert_eq!(pattern.stage, "build");
        assert!(!pattern.success);
        assert_eq!(pattern.fix_commit, None);
    }

    #[test]
    fn test_failure_pattern_mark_successful() {
        let pattern = FailurePattern::new(
            "myrepo".to_string(),
            "build".to_string(),
            "CompilationError".to_string(),
            "unused variable 'x'".to_string(),
            "Removed variable x".to_string(),
            "let x = 1; -> let _x = 1;".to_string(),
        )
        .mark_successful();
        assert!(pattern.success);
    }

    #[test]
    fn test_failure_pattern_with_commit() {
        let pattern = FailurePattern::new(
            "myrepo".to_string(),
            "build".to_string(),
            "CompilationError".to_string(),
            "unused variable 'x'".to_string(),
            "Removed variable x".to_string(),
            "let x = 1; -> let _x = 1;".to_string(),
        )
        .with_commit("abc123".to_string());
        assert_eq!(pattern.fix_commit, Some("abc123".to_string()));
    }

    #[test]
    fn test_failure_pattern_serialize() {
        let pattern = FailurePattern::new(
            "myrepo".to_string(),
            "build".to_string(),
            "CompilationError".to_string(),
            "unused variable 'x'".to_string(),
            "Removed variable x".to_string(),
            "let x = 1; -> let _x = 1;".to_string(),
        );
        let json = serde_json::to_string(&pattern).unwrap();
        assert!(json.contains("\"repo\":\"myrepo\""));
        assert!(json.contains("\"stage\":\"build\""));
    }

    #[test]
    fn test_failure_pattern_unique_id() {
        let pattern1 = FailurePattern::new(
            "myrepo".to_string(),
            "build".to_string(),
            "CompilationError".to_string(),
            "unused variable 'x'".to_string(),
            "Removed variable x".to_string(),
            "let x = 1; -> let _x = 1;".to_string(),
        );
        let pattern2 = FailurePattern::new(
            "myrepo".to_string(),
            "build".to_string(),
            "CompilationError".to_string(),
            "unused variable 'x'".to_string(),
            "Removed variable x".to_string(),
            "let x = 1; -> let _x = 1;".to_string(),
        );
        assert_ne!(pattern1.id, pattern2.id);
    }
}
