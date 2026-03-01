//! Bridge to Shipwright bash scripts via subprocess.
//!
//! Provides async execution of shell scripts with JSON parsing support.
//! Used to integrate with the real Shipwright CLI tools for pipeline
//! management, decision engine, and intelligence analysis.

use std::path::{Path, PathBuf};
use std::fmt;
use tokio::process::Command;
use std::time::Duration;

/// Error type for bash execution failures.
#[derive(Debug)]
pub enum BashError {
    /// Script file not found.
    ScriptNotFound(String),
    /// Script execution failed with non-zero exit code.
    ExecutionFailed(String),
    /// Command timed out.
    Timeout,
    /// Failed to parse JSON output.
    JsonParse(String),
    /// I/O or system error.
    IoError(String),
}

impl fmt::Display for BashError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BashError::ScriptNotFound(s) => write!(f, "Script not found: {}", s),
            BashError::ExecutionFailed(s) => write!(f, "Script execution failed: {}", s),
            BashError::Timeout => write!(f, "Script execution timed out"),
            BashError::JsonParse(s) => write!(f, "JSON parse error: {}", s),
            BashError::IoError(s) => write!(f, "I/O error: {}", s),
        }
    }
}

impl std::error::Error for BashError {}

/// Async bash script runner with timeout and JSON support.
pub struct BashRunner {
    scripts_dir: PathBuf,
    timeout_seconds: u64,
}

impl BashRunner {
    /// Create a new BashRunner with defaults.
    ///
    /// - scripts_dir: From SHIPWRIGHT_HOME env var, or ~/.shipwright/scripts
    /// - timeout_seconds: 300 (5 minutes)
    pub fn new() -> Self {
        let scripts_dir = if let Ok(home) = std::env::var("SHIPWRIGHT_HOME") {
            PathBuf::from(home).join("scripts")
        } else if let Ok(home) = std::env::var("HOME") {
            PathBuf::from(home).join(".shipwright/scripts")
        } else {
            PathBuf::from("/tmp/.shipwright/scripts")
        };

        Self {
            scripts_dir,
            timeout_seconds: 300,
        }
    }

    /// Create a BashRunner with a custom scripts directory.
    pub fn with_scripts_dir<P: AsRef<Path>>(scripts_dir: P) -> Self {
        Self {
            scripts_dir: scripts_dir.as_ref().to_path_buf(),
            timeout_seconds: 300,
        }
    }

    /// Set the timeout duration.
    pub fn with_timeout(mut self, seconds: u64) -> Self {
        self.timeout_seconds = seconds;
        self
    }

    /// Run a bash script and return stdout as a string.
    ///
    /// # Arguments
    /// - script_name: Name of the script (without directory path)
    /// - args: Command-line arguments to pass to the script
    ///
    /// # Returns
    /// - Ok(stdout) on success
    /// - Err(BashError) on failure (not found, timeout, non-zero exit)
    pub async fn run(&self, script_name: &str, args: &[&str]) -> Result<String, BashError> {
        let script_path = self.scripts_dir.join(script_name);

        if !script_path.exists() {
            return Err(BashError::ScriptNotFound(script_path.display().to_string()));
        }

        let mut cmd = Command::new("bash");
        cmd.arg(&script_path);
        for arg in args {
            cmd.arg(arg);
        }

        let timeout = Duration::from_secs(self.timeout_seconds);
        let output = tokio::time::timeout(timeout, cmd.output())
            .await
            .map_err(|_| BashError::Timeout)?
            .map_err(|e| BashError::IoError(e.to_string()))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            return Err(BashError::ExecutionFailed(format!(
                "Exit code: {}\nStdout: {}\nStderr: {}",
                output.status.code().unwrap_or(-1),
                stdout,
                stderr
            )));
        }

        String::from_utf8(output.stdout).map_err(|e| BashError::IoError(e.to_string()))
    }

    /// Run a bash script and parse the JSON output.
    ///
    /// # Arguments
    /// - script_name: Name of the script
    /// - args: Command-line arguments
    ///
    /// # Returns
    /// - Ok(T) where T: serde::de::DeserializeOwned
    /// - Err(BashError) on execution failure or JSON parse error
    pub async fn run_json<T: serde::de::DeserializeOwned>(
        &self,
        script_name: &str,
        args: &[&str],
    ) -> Result<T, BashError> {
        let output = self.run(script_name, args).await?;
        serde_json::from_str(&output).map_err(|e| BashError::JsonParse(e.to_string()))
    }
}

impl Default for BashRunner {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bash_runner_new() {
        let runner = BashRunner::new();
        assert!(runner.timeout_seconds > 0);
        assert!(!runner.scripts_dir.to_string_lossy().is_empty());
    }

    #[test]
    fn test_bash_runner_with_custom_dir() {
        let runner = BashRunner::with_scripts_dir("/tmp/scripts");
        assert_eq!(runner.scripts_dir, PathBuf::from("/tmp/scripts"));
    }

    #[test]
    fn test_bash_runner_with_timeout() {
        let runner = BashRunner::new().with_timeout(120);
        assert_eq!(runner.timeout_seconds, 120);
    }

    #[test]
    fn test_bash_error_display() {
        let err = BashError::ScriptNotFound("test.sh".to_string());
        assert!(format!("{}", err).contains("test.sh"));

        let err = BashError::Timeout;
        assert!(format!("{}", err).contains("timed out"));

        let err = BashError::ExecutionFailed("exit 1".to_string());
        assert!(format!("{}", err).contains("exit 1"));
    }

    #[tokio::test]
    async fn test_script_not_found() {
        let runner = BashRunner::with_scripts_dir("/tmp/nonexistent");
        let result = runner.run("nonexistent.sh", &[]).await;
        assert!(result.is_err());
        match result {
            Err(BashError::ScriptNotFound(_)) => (),
            _ => panic!("Expected ScriptNotFound"),
        }
    }

    #[tokio::test]
    async fn test_run_simple_script() {
        // Create a temporary script
        let temp_dir = std::env::temp_dir();
        let script_path = temp_dir.join("test_script.sh");
        std::fs::write(&script_path, "#!/bin/bash\necho 'hello'").ok();

        let runner = BashRunner::with_scripts_dir(&temp_dir);
        let result = runner.run("test_script.sh", &[]).await;

        // Clean up
        let _ = std::fs::remove_file(&script_path);

        assert!(result.is_ok());
        assert!(result.unwrap().contains("hello"));
    }

    #[tokio::test]
    async fn test_run_json_valid() {
        // Create a temporary script that outputs JSON
        let temp_dir = std::env::temp_dir();
        let script_path = temp_dir.join("test_json.sh");
        std::fs::write(&script_path, "#!/bin/bash\necho '{\"key\": \"value\"}'").ok();

        let runner = BashRunner::with_scripts_dir(&temp_dir);
        let result: Result<serde_json::Value, _> = runner.run_json("test_json.sh", &[]).await;

        // Clean up
        let _ = std::fs::remove_file(&script_path);

        assert!(result.is_ok());
        let value = result.unwrap();
        assert_eq!(value["key"], "value");
    }

    #[tokio::test]
    async fn test_run_json_invalid() {
        // Create a temporary script with invalid JSON
        let temp_dir = std::env::temp_dir();
        let script_path = temp_dir.join("test_bad_json.sh");
        std::fs::write(&script_path, "#!/bin/bash\necho 'not json'").ok();

        let runner = BashRunner::with_scripts_dir(&temp_dir);
        let result: Result<serde_json::Value, _> = runner.run_json("test_bad_json.sh", &[]).await;

        // Clean up
        let _ = std::fs::remove_file(&script_path);

        assert!(result.is_err());
        match result {
            Err(BashError::JsonParse(_)) => (),
            _ => panic!("Expected JsonParse"),
        }
    }
}
