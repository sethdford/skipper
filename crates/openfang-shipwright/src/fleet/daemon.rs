//! Daemon poll cycle: issue triage, dispatch, queue management.
//!
//! Polls GitHub for labeled issues, triages by priority, dispatches to fleet
//! respecting worker limits, and tracks status.

use std::collections::HashMap;

/// Issue priority for triage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum TriagePriority {
    P0,
    P1,
    P2,
    Unlabeled,
}

/// Mock GitHub issue for testing.
#[derive(Debug, Clone)]
pub struct Issue {
    pub id: String,
    pub title: String,
    pub labels: Vec<String>,
    pub priority: TriagePriority,
}

impl Issue {
    /// Create a new issue.
    pub fn new(id: String, title: String) -> Self {
        Self {
            id,
            title,
            labels: vec![],
            priority: TriagePriority::Unlabeled,
        }
    }

    /// Add a label and update priority.
    pub fn with_label(mut self, label: String) -> Self {
        if label == "p0" {
            self.priority = TriagePriority::P0;
        } else if label == "p1" {
            self.priority = TriagePriority::P1;
        } else if label == "p2" {
            self.priority = TriagePriority::P2;
        }
        self.labels.push(label);
        self
    }
}

/// Poll cycle result.
#[derive(Debug, Clone)]
pub struct PollResult {
    pub dispatched: Vec<String>,
    pub queued_remaining: u32,
    pub skipped: u32,
}

impl Default for PollResult {
    fn default() -> Self {
        Self {
            dispatched: vec![],
            queued_remaining: 0,
            skipped: 0,
        }
    }
}

/// Async poll cycle: dispatch issues respecting limits.
pub async fn poll_cycle(
    issues: Vec<Issue>,
    max_parallel: u32,
    available_workers: u32,
    claimed_ids: &[String],
) -> PollResult {
    let mut result = PollResult::default();
    let claimed_set: std::collections::HashSet<_> = claimed_ids.iter().cloned().collect();

    // Filter and sort
    let mut available: Vec<_> = issues
        .into_iter()
        .filter(|i| !claimed_set.contains(&i.id))
        .collect();

    // Skip proposed issues
    let mut proposed_count = 0;
    available.retain(|issue| {
        if issue.labels.iter().any(|l| l == "shipwright:proposed") {
            proposed_count += 1;
            false
        } else {
            true
        }
    });

    // Sort by priority
    available.sort_by_key(|i| std::cmp::Reverse(i.priority));

    // Dispatch up to max_parallel, respecting worker availability
    let dispatch_limit = max_parallel.min(available_workers);
    for issue in available.iter().take(dispatch_limit as usize) {
        result.dispatched.push(issue.id.clone());
    }

    result.queued_remaining = (available.len() as u32).saturating_sub(dispatch_limit);
    result.skipped = proposed_count;
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_issue_new() {
        let issue = Issue::new("123".to_string(), "Fix bug".to_string());
        assert_eq!(issue.id, "123");
        assert_eq!(issue.title, "Fix bug");
        assert_eq!(issue.priority, TriagePriority::Unlabeled);
    }

    #[test]
    fn test_issue_with_label() {
        let issue = Issue::new("123".to_string(), "Fix bug".to_string())
            .with_label("p0".to_string());
        assert_eq!(issue.priority, TriagePriority::P0);
        assert!(issue.labels.contains(&"p0".to_string()));
    }

    #[tokio::test]
    async fn test_poll_cycle_basic() {
        let issues = vec![
            Issue::new("1".to_string(), "Issue 1".to_string()).with_label("p0".to_string()),
            Issue::new("2".to_string(), "Issue 2".to_string()).with_label("p1".to_string()),
            Issue::new("3".to_string(), "Issue 3".to_string()),
        ];

        let result = poll_cycle(issues, 2, 2, &[]).await;
        assert_eq!(result.dispatched.len(), 2);
        assert_eq!(result.queued_remaining, 1);
        // P0 and P1 should be dispatched first
        assert!(result.dispatched.contains(&"1".to_string()));
        assert!(result.dispatched.contains(&"2".to_string()));
    }

    #[tokio::test]
    async fn test_poll_cycle_respects_max_parallel() {
        let issues = vec![
            Issue::new("1".to_string(), "Issue 1".to_string()),
            Issue::new("2".to_string(), "Issue 2".to_string()),
            Issue::new("3".to_string(), "Issue 3".to_string()),
        ];

        let result = poll_cycle(issues, 2, 5, &[]).await;
        assert_eq!(result.dispatched.len(), 2);
        assert_eq!(result.queued_remaining, 1);
    }

    #[tokio::test]
    async fn test_poll_cycle_skips_claimed() {
        let issues = vec![
            Issue::new("1".to_string(), "Issue 1".to_string()),
            Issue::new("2".to_string(), "Issue 2".to_string()),
        ];

        let claimed = vec!["1".to_string()];
        let result = poll_cycle(issues, 2, 5, &claimed).await;
        assert_eq!(result.dispatched.len(), 1);
        assert!(result.dispatched.contains(&"2".to_string()));
    }

    #[tokio::test]
    async fn test_poll_cycle_respects_worker_capacity() {
        let issues = vec![
            Issue::new("1".to_string(), "Issue 1".to_string()),
            Issue::new("2".to_string(), "Issue 2".to_string()),
            Issue::new("3".to_string(), "Issue 3".to_string()),
        ];

        let result = poll_cycle(issues, 10, 1, &[]).await;
        assert_eq!(result.dispatched.len(), 1);
    }

    #[tokio::test]
    async fn test_poll_cycle_skips_proposed() {
        let issues = vec![
            Issue::new("1".to_string(), "Issue 1".to_string())
                .with_label("shipwright:proposed".to_string()),
            Issue::new("2".to_string(), "Issue 2".to_string()),
        ];

        let result = poll_cycle(issues, 2, 5, &[]).await;
        assert_eq!(result.dispatched.len(), 1);
        assert!(result.dispatched.contains(&"2".to_string()));
        assert_eq!(result.skipped, 1);
    }

    #[tokio::test]
    async fn test_poll_cycle_empty() {
        let result = poll_cycle(vec![], 2, 5, &[]).await;
        assert_eq!(result.dispatched.len(), 0);
        assert_eq!(result.queued_remaining, 0);
    }
}
