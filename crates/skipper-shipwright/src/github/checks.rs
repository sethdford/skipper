//! GitHub Checks API integration.

use serde::{Deserialize, Serialize};

/// Check run status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckRunStatus {
    Queued,
    InProgress,
    Completed,
}

/// Check run conclusion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckRunConclusion {
    Success,
    Failure,
    Neutral,
    Cancelled,
    TimedOut,
    ActionRequired,
}

/// A GitHub Check Run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckRun {
    pub id: u64,
    pub name: String,
    pub head_sha: String,
    pub status: CheckRunStatus,
    pub conclusion: Option<CheckRunConclusion>,
    pub output: CheckRunOutput,
}

/// Check run output details.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckRunOutput {
    pub title: String,
    pub summary: String,
    pub annotations: Vec<CheckAnnotation>,
}

/// A check annotation (error/warning line).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckAnnotation {
    pub path: String,
    pub start_line: u32,
    pub end_line: u32,
    pub annotation_level: String,
    pub message: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_check_run_status_serialize() {
        let status = CheckRunStatus::InProgress;
        let json = serde_json::to_string(&status).unwrap();
        assert_eq!(json, "\"in_progress\"");
    }

    #[test]
    fn test_check_run_conclusion_serialize() {
        let conclusion = CheckRunConclusion::Success;
        let json = serde_json::to_string(&conclusion).unwrap();
        assert_eq!(json, "\"success\"");
    }

    #[test]
    fn test_check_run_serialize() {
        let check = CheckRun {
            id: 1,
            name: "Build".to_string(),
            head_sha: "abc123".to_string(),
            status: CheckRunStatus::Completed,
            conclusion: Some(CheckRunConclusion::Success),
            output: CheckRunOutput {
                title: "Build succeeded".to_string(),
                summary: "All checks passed".to_string(),
                annotations: vec![],
            },
        };
        let json = serde_json::to_string(&check).unwrap();
        assert!(json.contains("\"name\":\"Build\""));
    }

    #[test]
    fn test_check_annotation_serialize() {
        let annotation = CheckAnnotation {
            path: "src/main.rs".to_string(),
            start_line: 10,
            end_line: 15,
            annotation_level: "error".to_string(),
            message: "Undefined variable".to_string(),
        };
        let json = serde_json::to_string(&annotation).unwrap();
        assert!(json.contains("\"path\":\"src/main.rs\""));
    }
}
