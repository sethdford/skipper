//! GitHub API client with GraphQL, Checks, Deployments, and PR lifecycle support.

use dashmap::DashMap;
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use thiserror::Error;

pub mod checks;
pub mod deployments;
pub mod graphql;
pub mod pr;

pub use checks::{CheckRun, CheckRunStatus};
pub use deployments::{Deployment, DeploymentStatus};
pub use graphql::GraphQLQuery;
pub use pr::PullRequest;

/// GitHub API client error type.
#[derive(Error, Debug)]
pub enum GitHubError {
    #[error("HTTP request failed: {0}")]
    RequestFailed(#[from] reqwest::Error),

    #[error("Invalid GitHub token")]
    InvalidToken,

    #[error("Rate limit exceeded")]
    RateLimited,

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("API error: {0}")]
    ApiError(String),

    #[error("JSON serialization error: {0}")]
    JsonError(#[from] serde_json::Error),

    #[error("Cache error: {0}")]
    CacheError(String),
}

pub type Result<T> = std::result::Result<T, GitHubError>;

/// A cached value with TTL.
#[derive(Debug, Clone)]
struct CacheEntry<T> {
    value: T,
    inserted_at: SystemTime,
}

impl<T> CacheEntry<T> {
    fn is_expired(&self, ttl: Duration) -> bool {
        self.inserted_at.elapsed().unwrap_or(ttl) >= ttl
    }
}

/// GitHub issue summary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Issue {
    pub id: u64,
    pub number: u64,
    pub title: String,
    pub body: Option<String>,
    pub labels: Vec<String>,
    pub state: String,
    pub created_at: String,
    pub updated_at: String,
}

/// GitHub client for REST and GraphQL API interactions.
#[derive(Debug)]
pub struct GitHubClient {
    http_client: Client,
    token: String,
    cache: Arc<DashMap<String, CacheEntry<serde_json::Value>>>,
}

impl GitHubClient {
    /// Create a new GitHub client from environment GITHUB_TOKEN.
    pub fn new() -> Result<Self> {
        let token = std::env::var("GITHUB_TOKEN")
            .map_err(|_| GitHubError::InvalidToken)?;
        Ok(Self::with_token(token))
    }

    /// Create a new GitHub client with explicit token.
    pub fn with_token(token: String) -> Self {
        Self {
            http_client: Client::new(),
            token,
            cache: Arc::new(DashMap::new()),
        }
    }

    /// Get authorization header value.
    fn auth_header(&self) -> String {
        format!("token {}", self.token)
    }

    /// Check if a response indicates rate limiting.
    fn check_rate_limit(status: StatusCode, body_hint: &str) -> Option<GitHubError> {
        if status == StatusCode::TOO_MANY_REQUESTS {
            return Some(GitHubError::RateLimited);
        }
        if status == StatusCode::FORBIDDEN && body_hint.to_lowercase().contains("rate limit") {
            return Some(GitHubError::RateLimited);
        }
        None
    }

    /// List issues for a repository with optional label filter.
    pub async fn list_issues(
        &self,
        owner: &str,
        repo: &str,
        labels: Option<&[String]>,
    ) -> Result<Vec<Issue>> {
        let mut url = format!(
            "https://api.github.com/repos/{}/{}/issues?state=open&per_page=100",
            owner, repo
        );

        if let Some(labels) = labels {
            if !labels.is_empty() {
                url.push_str("&labels=");
                url.push_str(&labels.join(","));
            }
        }

        let response = self
            .http_client
            .get(&url)
            .header("Authorization", self.auth_header())
            .header("Accept", "application/vnd.github.v3+json")
            .send()
            .await?;

        if response.status() == StatusCode::UNAUTHORIZED {
            return Err(GitHubError::InvalidToken);
        }

        if let Some(rate_limit_err) = Self::check_rate_limit(response.status(), "") {
            return Err(rate_limit_err);
        }

        if !response.status().is_success() {
            return Err(GitHubError::ApiError(format!(
                "Failed to list issues: {}",
                response.status()
            )));
        }

        let issues: Vec<Issue> = response.json().await?;
        Ok(issues)
    }

    /// Create an issue in a repository.
    pub async fn create_issue(
        &self,
        owner: &str,
        repo: &str,
        title: &str,
        body: &str,
        labels: Option<&[String]>,
    ) -> Result<Issue> {
        let url = format!("https://api.github.com/repos/{}/{}/issues", owner, repo);

        let mut payload = serde_json::json!({
            "title": title,
            "body": body,
        });

        if let Some(labels) = labels {
            payload["labels"] = serde_json::Value::Array(
                labels.iter().map(|l| serde_json::Value::String(l.clone())).collect(),
            );
        }

        let response = self
            .http_client
            .post(&url)
            .header("Authorization", self.auth_header())
            .header("Accept", "application/vnd.github.v3+json")
            .json(&payload)
            .send()
            .await?;

        if response.status() == StatusCode::UNAUTHORIZED {
            return Err(GitHubError::InvalidToken);
        }

        if let Some(rate_limit_err) = Self::check_rate_limit(response.status(), "") {
            return Err(rate_limit_err);
        }

        if !response.status().is_success() {
            return Err(GitHubError::ApiError(format!(
                "Failed to create issue: {}",
                response.status()
            )));
        }

        let issue: Issue = response.json().await?;
        Ok(issue)
    }

    /// Add a label to an issue.
    pub async fn add_label(
        &self,
        owner: &str,
        repo: &str,
        issue_number: u64,
        label: &str,
    ) -> Result<()> {
        let url = format!(
            "https://api.github.com/repos/{}/{}/issues/{}/labels",
            owner, repo, issue_number
        );

        let payload = serde_json::json!([label]);

        let response = self
            .http_client
            .post(&url)
            .header("Authorization", self.auth_header())
            .header("Accept", "application/vnd.github.v3+json")
            .json(&payload)
            .send()
            .await?;

        if response.status() == StatusCode::UNAUTHORIZED {
            return Err(GitHubError::InvalidToken);
        }

        if let Some(rate_limit_err) = Self::check_rate_limit(response.status(), "") {
            return Err(rate_limit_err);
        }

        if !response.status().is_success() {
            return Err(GitHubError::ApiError(format!(
                "Failed to add label: {}",
                response.status()
            )));
        }

        Ok(())
    }

    /// Clear the cache.
    pub fn clear_cache(&self) {
        self.cache.clear();
    }

    /// Get the number of cached entries.
    pub fn cache_size(&self) -> usize {
        self.cache.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_github_client_invalid_token_error() {
        // Test the error variant directly without manipulating env
        let err = GitHubError::InvalidToken;
        assert!(matches!(err, GitHubError::InvalidToken));
        assert!(err.to_string().contains("Invalid"));
    }

    #[test]
    fn test_github_client_with_token() {
        let client = GitHubClient::with_token("test-token".to_string());
        assert_eq!(client.auth_header(), "token test-token");
    }

    #[test]
    fn test_cache_entry_not_expired() {
        let entry = CacheEntry {
            value: serde_json::json!({"test": true}),
            inserted_at: SystemTime::now(),
        };
        assert!(!entry.is_expired(Duration::from_secs(60)));
    }

    #[test]
    fn test_cache_entry_expired() {
        let past = SystemTime::now() - Duration::from_secs(120);
        let entry = CacheEntry {
            value: serde_json::json!({"test": true}),
            inserted_at: past,
        };
        assert!(entry.is_expired(Duration::from_secs(60)));
    }

    #[test]
    fn test_issue_serialize() {
        let issue = Issue {
            id: 1,
            number: 1,
            title: "Test issue".to_string(),
            body: Some("Test body".to_string()),
            labels: vec!["bug".to_string()],
            state: "open".to_string(),
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-02T00:00:00Z".to_string(),
        };
        let json = serde_json::to_string(&issue).unwrap();
        assert!(json.contains("\"title\":\"Test issue\""));
    }

    #[test]
    fn test_client_cache_operations() {
        let client = GitHubClient::with_token("test".to_string());
        assert_eq!(client.cache_size(), 0);
        client.cache.insert(
            "key1".to_string(),
            CacheEntry {
                value: serde_json::json!({"test": true}),
                inserted_at: SystemTime::now(),
            },
        );
        assert_eq!(client.cache_size(), 1);
        client.clear_cache();
        assert_eq!(client.cache_size(), 0);
    }
}
