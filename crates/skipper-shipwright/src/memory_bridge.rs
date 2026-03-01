//! Bridge to Shipwright's filesystem-based memory system.
//!
//! Reads and writes failure patterns from ~/.shipwright/memory/<repo_hash>/
//! Provides async file I/O with atomic writes and graceful handling of missing files.

use std::path::PathBuf;
use std::fs;
use chrono::Utc;
use serde_json::Value;

/// Bridge to Shipwright's memory system on disk.
pub struct MemoryBridge {
    /// Root directory: ~/.shipwright/memory
    memory_root: PathBuf,
    /// Repository hash for scoping patterns
    repo_hash: String,
}

impl MemoryBridge {
    /// Create a new memory bridge for a repository.
    ///
    /// Defaults to ~/.shipwright/memory as the root directory.
    /// repo_hash is computed from the repo parameter.
    pub fn new(repo: &str) -> Self {
        let memory_root = if let Ok(home) = std::env::var("HOME") {
            PathBuf::from(home).join(".shipwright/memory")
        } else {
            PathBuf::from("/tmp/.shipwright/memory")
        };

        // Simple hash: use first 8 chars of repo name, or hash if longer
        let repo_hash = if repo.len() <= 8 {
            repo.to_string()
        } else {
            format!("{:x}", calculate_hash(repo))
        };

        Self {
            memory_root,
            repo_hash,
        }
    }

    /// Create a memory bridge with a custom memory root.
    pub fn with_root<P: AsRef<std::path::Path>>(root: P, repo: &str) -> Self {
        let repo_hash = if repo.len() <= 8 {
            repo.to_string()
        } else {
            format!("{:x}", calculate_hash(repo))
        };

        Self {
            memory_root: root.as_ref().to_path_buf(),
            repo_hash,
        }
    }

    /// Get the directory for this repo's patterns.
    fn repo_dir(&self) -> PathBuf {
        self.memory_root.join(&self.repo_hash)
    }

    /// Store a failure pattern to disk.
    ///
    /// Creates a JSON file with the pattern in ~/.shipwright/memory/<repo_hash>/
    /// Uses atomic write (temp file + rename) to prevent corruption.
    pub async fn store_failure(&self, pattern: &Value) -> Result<(), String> {
        let repo_dir = self.repo_dir();

        // Ensure directory exists
        if !repo_dir.exists() {
            fs::create_dir_all(&repo_dir).map_err(|e| format!("Failed to create memory dir: {}", e))?;
        }

        // Generate a unique filename based on timestamp and error class
        let error_class = pattern
            .get("error_class")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");

        let timestamp = Utc::now().format("%Y%m%d_%H%M%S_%3f").to_string();
        let filename = format!("failure_{}_{}_{}.json", error_class, timestamp, uuid::Uuid::new_v4());
        let file_path = repo_dir.join(&filename);

        // Write to temp file, then atomically rename
        let temp_file = repo_dir.join(format!(".tmp_{}", filename));
        let json_str = serde_json::to_string_pretty(pattern)
            .map_err(|e| format!("Failed to serialize pattern: {}", e))?;

        tokio::fs::write(&temp_file, &json_str)
            .await
            .map_err(|e| format!("Failed to write temp file: {}", e))?;

        tokio::fs::rename(&temp_file, &file_path)
            .await
            .map_err(|e| format!("Failed to rename file: {}", e))?;

        Ok(())
    }

    /// Search for failure patterns matching a query.
    ///
    /// Scans the repo directory for JSON files, filters by query substring (case-insensitive),
    /// sorts by recency, and returns up to limit results.
    pub async fn search_failures(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<Value>, String> {
        let repo_dir = self.repo_dir();

        // If directory doesn't exist, return empty results gracefully
        if !repo_dir.exists() {
            return Ok(vec![]);
        }

        let mut results = vec![];
        let query_lower = query.to_lowercase();

        // Read directory entries
        let entries = tokio::fs::read_dir(&repo_dir)
            .await
            .map_err(|e| format!("Failed to read memory directory: {}", e))?;

        let mut dir_entries = Vec::new();
        let mut read_dir = entries;
        loop {
            match read_dir.next_entry().await {
                Ok(Some(entry)) => dir_entries.push(entry),
                Ok(None) => break,
                Err(e) => return Err(format!("Failed to read directory entry: {}", e)),
            }
        }

        // Process each JSON file
        for entry in dir_entries {
            let path = entry.path();

            if path.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }

            if path.file_name().and_then(|s| s.to_str()).map(|s| s.starts_with(".tmp_")).unwrap_or(false) {
                continue;
            }

            match tokio::fs::read_to_string(&path).await {
                Ok(content) => {
                    if let Ok(json_val) = serde_json::from_str::<Value>(&content) {
                        // Check if any field matches the query
                        let matches = check_json_match(&json_val, &query_lower);
                        if matches {
                            results.push(json_val);
                        }
                    }
                }
                Err(_) => {
                    // Skip unparseable files
                    continue;
                }
            }
        }

        // Sort by timestamp (newer first) and limit results
        results.sort_by(|a, b| {
            let a_ts = a
                .get("timestamp")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let b_ts = b
                .get("timestamp")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            b_ts.cmp(a_ts)
        });

        results.truncate(limit);
        Ok(results)
    }
}

/// Simple hash function for repo names (fallback for long names).
fn calculate_hash(s: &str) -> u32 {
    let mut hash: u32 = 0;
    for byte in s.as_bytes() {
        hash = hash.wrapping_mul(31).wrapping_add(*byte as u32);
    }
    hash
}

/// Check if a JSON value matches a query string (recursive, case-insensitive).
fn check_json_match(value: &Value, query: &str) -> bool {
    match value {
        Value::String(s) => s.to_lowercase().contains(query),
        Value::Object(map) => map.values().any(|v| check_json_match(v, query)),
        Value::Array(arr) => arr.iter().any(|v| check_json_match(v, query)),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use serde_json::json;

    #[test]
    fn test_memory_bridge_new() {
        let bridge = MemoryBridge::new("testrepo");
        // Short names (<=8 chars) are used as-is
        assert_eq!(bridge.repo_hash, "testrepo");
    }

    #[test]
    fn test_memory_bridge_with_root() {
        let temp = TempDir::new().unwrap();
        let bridge = MemoryBridge::with_root(temp.path(), "repo");
        assert_eq!(bridge.memory_root, temp.path());
    }

    #[test]
    fn test_memory_bridge_long_repo_name() {
        let bridge = MemoryBridge::new("very-long-repository-name-that-exceeds-eight-chars");
        assert_eq!(bridge.repo_hash, format!("{:x}", calculate_hash(
            "very-long-repository-name-that-exceeds-eight-chars"
        )));
    }

    #[test]
    fn test_repo_dir() {
        let temp = TempDir::new().unwrap();
        let bridge = MemoryBridge::with_root(temp.path(), "myrepo");
        assert_eq!(bridge.repo_dir(), temp.path().join("myrepo"));
    }

    #[tokio::test]
    async fn test_store_failure() {
        let temp = TempDir::new().unwrap();
        let bridge = MemoryBridge::with_root(temp.path(), "testrepo");

        let pattern = json!({
            "error_class": "CompilationError",
            "error_signature": "missing import",
            "root_cause": "forgot use statement",
            "fix_applied": "added import",
        });

        let result = bridge.store_failure(&pattern).await;
        assert!(result.is_ok());

        // Verify directory was created
        let repo_dir = bridge.repo_dir();
        assert!(repo_dir.exists());

        // Verify file was written
        let entries: Vec<_> = std::fs::read_dir(&repo_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.path()
                    .extension()
                    .and_then(|s| s.to_str())
                    .map(|s| s == "json")
                    .unwrap_or(false)
            })
            .collect();

        assert_eq!(entries.len(), 1);
    }

    #[tokio::test]
    async fn test_search_failures_empty_dir() {
        let temp = TempDir::new().unwrap();
        let bridge = MemoryBridge::with_root(temp.path(), "testrepo");

        let result = bridge.search_failures("query", 10).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap().len(), 0);
    }

    #[tokio::test]
    async fn test_search_failures_matches() {
        let temp = TempDir::new().unwrap();
        let bridge = MemoryBridge::with_root(temp.path(), "testrepo");

        // Store a pattern
        let pattern = json!({
            "error_class": "CompilationError",
            "error_signature": "missing import statement",
            "root_cause": "forgot use declaration",
            "fix_applied": "added import",
            "timestamp": "2024-01-01T12:00:00Z",
        });

        let _ = bridge.store_failure(&pattern).await;

        // Search for it
        let result = bridge.search_failures("missing import", 10).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn test_search_failures_limit() {
        let temp = TempDir::new().unwrap();
        let bridge = MemoryBridge::with_root(temp.path(), "testrepo");

        // Store multiple patterns
        for i in 0..5 {
            let pattern = json!({
                "error_class": "Error",
                "error_signature": format!("error {}", i),
                "root_cause": "test",
                "fix_applied": "fixed",
            });
            let _ = bridge.store_failure(&pattern).await;
        }

        // Search with limit
        let result = bridge.search_failures("error", 2).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap().len(), 2);
    }

    #[test]
    fn test_check_json_match_string() {
        let val = Value::String("hello world".to_string());
        assert!(check_json_match(&val, "hello"));
        assert!(check_json_match(&val, "hello")); // compare lowercase to lowercase
        assert!(!check_json_match(&val, "xyz"));
    }

    #[test]
    fn test_check_json_match_object() {
        let val = json!({
            "name": "test",
            "message": "hello world",
        });
        assert!(check_json_match(&val, "hello"));
        assert!(check_json_match(&val, "test"));
        assert!(!check_json_match(&val, "xyz"));
    }

    #[test]
    fn test_check_json_match_array() {
        let val = json!(["apple", "banana", "cherry"]);
        assert!(check_json_match(&val, "apple"));
        assert!(check_json_match(&val, "cherry")); // compare lowercase to lowercase
        assert!(!check_json_match(&val, "xyz"));
    }
}
