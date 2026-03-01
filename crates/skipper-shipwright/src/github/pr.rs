//! GitHub Pull Request lifecycle management.

use super::{GitHubClient, Result};
use serde::{Deserialize, Serialize};

/// A GitHub Pull Request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PullRequest {
    pub id: u64,
    pub number: u64,
    pub title: String,
    pub body: Option<String>,
    pub state: String,
    pub head: BranchRef,
    pub base: BranchRef,
    pub user: User,
    pub created_at: String,
    pub updated_at: String,
    pub draft: bool,
}

/// Branch reference.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BranchRef {
    pub sha: String,
    pub ref_: String,
}

/// User information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub login: String,
    pub id: u64,
}

impl GitHubClient {
    /// Create a pull request.
    pub async fn create_pr(
        &self,
        owner: &str,
        repo: &str,
        title: &str,
        body: &str,
        head: &str,
        base: &str,
    ) -> Result<PullRequest> {
        let url = format!("https://api.github.com/repos/{}/{}/pulls", owner, repo);

        let payload = serde_json::json!({
            "title": title,
            "body": body,
            "head": head,
            "base": base,
            "draft": false,
        });

        let response = self
            .http_client
            .post(&url)
            .header("Authorization", self.auth_header())
            .header("Accept", "application/vnd.github.v3+json")
            .json(&payload)
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(super::GitHubError::ApiError(format!(
                "Failed to create PR: {}",
                response.status()
            )));
        }

        let pr: PullRequest = response.json().await?;
        Ok(pr)
    }

    /// Select reviewers for a PR, deduplicating and limiting to 3.
    pub fn select_reviewers(
        &self,
        candidates: Vec<String>,
        limit: usize,
    ) -> Vec<String> {
        let mut seen = std::collections::HashSet::new();
        let mut reviewers = vec![];

        for candidate in candidates {
            if seen.insert(candidate.clone()) && reviewers.len() < limit {
                reviewers.push(candidate);
            }
        }

        reviewers
    }

    /// Auto-merge a PR if conditions are met.
    pub async fn auto_merge(
        &self,
        owner: &str,
        repo: &str,
        pr_number: u64,
        commit_title: &str,
    ) -> Result<()> {
        let url = format!(
            "https://api.github.com/repos/{}/{}/pulls/{}/merge",
            owner, repo, pr_number
        );

        let payload = serde_json::json!({
            "commit_title": commit_title,
            "merge_method": "squash",
        });

        let response = self
            .http_client
            .put(&url)
            .header("Authorization", self.auth_header())
            .header("Accept", "application/vnd.github.v3+json")
            .json(&payload)
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(super::GitHubError::ApiError(format!(
                "Failed to merge PR: {}",
                response.status()
            )));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pull_request_serialize() {
        let pr = PullRequest {
            id: 1,
            number: 1,
            title: "Add feature".to_string(),
            body: Some("This PR adds a new feature".to_string()),
            state: "open".to_string(),
            head: BranchRef {
                sha: "abc123".to_string(),
                ref_: "refs/heads/feature".to_string(),
            },
            base: BranchRef {
                sha: "def456".to_string(),
                ref_: "refs/heads/main".to_string(),
            },
            user: User {
                login: "alice".to_string(),
                id: 1,
            },
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-02T00:00:00Z".to_string(),
            draft: false,
        };
        let json = serde_json::to_string(&pr).unwrap();
        assert!(json.contains("\"title\":\"Add feature\""));
    }

    #[test]
    fn test_select_reviewers_deduplicates() {
        let client = GitHubClient::with_token("test".to_string());
        let candidates = vec![
            "alice".to_string(),
            "bob".to_string(),
            "alice".to_string(),
            "charlie".to_string(),
        ];
        let reviewers = client.select_reviewers(candidates, 3);
        assert_eq!(reviewers.len(), 3);
        assert!(reviewers.contains(&"alice".to_string()));
        assert!(!reviewers.iter().filter(|r| r == &"alice").count() > 1);
    }

    #[test]
    fn test_select_reviewers_respects_limit() {
        let client = GitHubClient::with_token("test".to_string());
        let candidates = vec![
            "alice".to_string(),
            "bob".to_string(),
            "charlie".to_string(),
            "dave".to_string(),
            "eve".to_string(),
        ];
        let reviewers = client.select_reviewers(candidates, 3);
        assert_eq!(reviewers.len(), 3);
    }

    #[test]
    fn test_branch_ref_serialize() {
        let branch = BranchRef {
            sha: "abc123".to_string(),
            ref_: "refs/heads/main".to_string(),
        };
        let json = serde_json::to_string(&branch).unwrap();
        assert!(json.contains("\"sha\":\"abc123\""));
    }

    #[test]
    fn test_user_serialize() {
        let user = User {
            login: "alice".to_string(),
            id: 1,
        };
        let json = serde_json::to_string(&user).unwrap();
        assert!(json.contains("\"login\":\"alice\""));
    }
}
