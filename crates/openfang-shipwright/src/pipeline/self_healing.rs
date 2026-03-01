//! Build loop with convergence detection and self-healing.

use serde::{Deserialize, Serialize};

/// Evaluation result from a build loop iteration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BuildOutcome {
    TestsPassing,
    Converging { issues_remaining: u32 },
    Diverging,
    Exhausted,
}

/// Progress state for a build loop.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProgressState {
    pub iteration: u32,
    pub error_count: u32,
    pub previous_error_count: Option<u32>,
    pub last_errors: Vec<String>,
}

impl ProgressState {
    /// Create a new progress state.
    pub fn new() -> Self {
        Self {
            iteration: 0,
            error_count: 0,
            previous_error_count: None,
            last_errors: vec![],
        }
    }

    /// Record an error.
    pub fn record_error(&mut self, error: String) {
        self.last_errors.push(error);
        if self.last_errors.len() > 10 {
            self.last_errors.remove(0);
        }
        // Update error_count AFTER trimming to keep it in sync with last_errors (M17 fix)
        self.error_count = self.last_errors.len() as u32;
    }

    /// Calculate error reduction percentage.
    /// Returns 100.0 only when both previous and current are 0.
    /// Returns 0.0 when errors increase from 0 (regression from clean state).
    /// Returns negative values when errors increase (diverging).
    pub fn error_reduction_percent(&self) -> Option<f64> {
        self.previous_error_count.map(|prev| {
            if prev == 0 {
                if self.error_count == 0 {
                    100.0 // Both zero — fully reduced (no errors)
                } else {
                    0.0 // Regression from clean state — no reduction
                }
            } else {
                (prev as f64 - self.error_count as f64) / prev as f64 * 100.0
            }
        })
    }
}

impl Default for ProgressState {
    fn default() -> Self {
        Self::new()
    }
}

/// Build loop configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildLoop {
    pub max_iterations: u32,
    pub max_restarts: u32,
    pub fast_test_cmd: Option<String>,
    pub fast_test_interval: u32,
    pub convergence_window: u32,
    pub progress: ProgressState,
}

impl BuildLoop {
    /// Create a new build loop.
    pub fn new(max_iterations: u32) -> Self {
        Self {
            max_iterations,
            max_restarts: 0,
            fast_test_cmd: None,
            fast_test_interval: 5,
            convergence_window: 3,
            progress: ProgressState::new(),
        }
    }

    /// Evaluate the current build state.
    pub fn evaluate(&self) -> BuildOutcome {
        if self.progress.iteration >= self.max_iterations {
            return BuildOutcome::Exhausted;
        }

        // If no errors, tests are passing
        if self.progress.error_count == 0 {
            return BuildOutcome::TestsPassing;
        }

        // Check convergence
        if let Some(prev_count) = self.progress.previous_error_count {
            let reduction_percent = self.progress.error_reduction_percent().unwrap_or(0.0);

            // If error count increased, diverging
            if self.progress.error_count > prev_count {
                return BuildOutcome::Diverging;
            }

            // If reducing errors by >50%, converging
            if reduction_percent > 50.0 {
                return BuildOutcome::Converging {
                    issues_remaining: self.progress.error_count,
                };
            }
        }

        BuildOutcome::Converging {
            issues_remaining: self.progress.error_count,
        }
    }

    /// Increment iteration counter.
    pub fn next_iteration(&mut self) {
        self.progress.iteration += 1;
        self.progress.previous_error_count = Some(self.progress.error_count);
    }

    /// Check if we should run full test suite this iteration.
    /// Runs full test on iteration 1 (first iteration after initial), every N iterations,
    /// and the final iteration. Avoids double-triggering on both iteration 0 and 1.
    pub fn should_run_full_test(&self) -> bool {
        self.progress.iteration == 1
            || (self.progress.iteration > 1 && (self.progress.iteration % self.fast_test_interval) == 0)
            || self.progress.iteration >= self.max_iterations - 1
    }
}

impl Default for BuildLoop {
    fn default() -> Self {
        Self::new(10)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_progress_state_new() {
        let progress = ProgressState::new();
        assert_eq!(progress.iteration, 0);
        assert_eq!(progress.error_count, 0);
    }

    #[test]
    fn test_progress_state_record_error() {
        let mut progress = ProgressState::new();
        progress.record_error("error 1".to_string());
        progress.record_error("error 2".to_string());
        assert_eq!(progress.error_count, 2);
    }

    #[test]
    fn test_progress_state_error_reduction() {
        let mut progress = ProgressState::new();
        progress.previous_error_count = Some(10);
        progress.record_error("error 1".to_string());
        progress.record_error("error 2".to_string());
        progress.record_error("error 3".to_string());
        progress.record_error("error 4".to_string());
        progress.record_error("error 5".to_string());

        let reduction = progress.error_reduction_percent().unwrap();
        assert!(reduction > 40.0 && reduction < 60.0);
    }

    #[test]
    fn test_build_loop_new() {
        let loop_config = BuildLoop::new(5);
        assert_eq!(loop_config.max_iterations, 5);
        assert_eq!(loop_config.max_restarts, 0);
    }

    #[test]
    fn test_build_loop_evaluate_passing() {
        let loop_config = BuildLoop::new(10);
        let outcome = loop_config.evaluate();
        assert_eq!(outcome, BuildOutcome::TestsPassing);
    }

    #[test]
    fn test_build_loop_evaluate_converging() {
        let mut loop_config = BuildLoop::new(10);
        loop_config.progress.previous_error_count = Some(10);
        loop_config.progress.record_error("error 1".to_string());
        loop_config.progress.record_error("error 2".to_string());
        loop_config.progress.record_error("error 3".to_string());
        loop_config.progress.record_error("error 4".to_string());
        loop_config.progress.record_error("error 5".to_string());

        let outcome = loop_config.evaluate();
        assert!(matches!(outcome, BuildOutcome::Converging { .. }));
    }

    #[test]
    fn test_build_loop_evaluate_diverging() {
        let mut loop_config = BuildLoop::new(10);
        loop_config.progress.previous_error_count = Some(3);
        for i in 0..5 {
            loop_config
                .progress
                .record_error(format!("error {}", i));
        }

        let outcome = loop_config.evaluate();
        assert_eq!(outcome, BuildOutcome::Diverging);
    }

    #[test]
    fn test_build_loop_evaluate_exhausted() {
        let mut loop_config = BuildLoop::new(5);
        loop_config.progress.iteration = 5;
        let outcome = loop_config.evaluate();
        assert_eq!(outcome, BuildOutcome::Exhausted);
    }

    #[test]
    fn test_build_loop_next_iteration() {
        let mut loop_config = BuildLoop::new(10);
        loop_config.next_iteration();
        assert_eq!(loop_config.progress.iteration, 1);
    }

    #[test]
    fn test_build_loop_should_run_full_test() {
        let mut loop_config = BuildLoop::new(10);
        loop_config.fast_test_interval = 5;

        // Iteration 0 should NOT run full test (initial state, no work done)
        assert!(!loop_config.should_run_full_test());

        // Iteration 1 should run full test (first iteration after initial)
        loop_config.progress.iteration = 1;
        assert!(loop_config.should_run_full_test());

        // Iteration 2 should NOT run full test
        loop_config.progress.iteration = 2;
        assert!(!loop_config.should_run_full_test());

        // Iteration 5 should run full test (5 % 5 == 0, and iteration > 1)
        loop_config.progress.iteration = 5;
        assert!(loop_config.should_run_full_test());

        // Iteration 9 should be final iteration, so should run full test
        loop_config.progress.iteration = 9;
        assert!(loop_config.should_run_full_test());
    }

    #[test]
    fn test_progress_state_error_count_vec_sync() {
        // M17: error_count must stay in sync with last_errors.len()
        let mut progress = ProgressState::new();

        for i in 0..15 {
            progress.record_error(format!("error {}", i));
            // error_count must always match last_errors.len()
            assert_eq!(
                progress.error_count, progress.last_errors.len() as u32,
                "After recording error {}, error_count ({}) doesn't match last_errors.len() ({})",
                i, progress.error_count, progress.last_errors.len()
            );
        }

        // After 15 errors, should have trimmed to max 10
        assert_eq!(progress.last_errors.len(), 10);
        assert_eq!(progress.error_count, 10);
    }

    #[test]
    fn test_progress_state_error_count_after_trim() {
        // M17: When errors exceed 10, error_count must be updated after trimming
        let mut progress = ProgressState::new();

        // Record 11 errors to trigger trim
        for i in 0..11 {
            progress.record_error(format!("error {}", i));
        }

        // Should have exactly 10 errors after trim
        assert_eq!(progress.last_errors.len(), 10);
        // error_count must match, not be 11
        assert_eq!(progress.error_count, 10, "error_count should be 10 after trim, not 11");
    }

    #[test]
    fn test_crit006_error_reduction_clean_to_errors() {
        // CRIT-006: When previous=0 (clean state) and current > 0 (errors introduced),
        // should return 0.0 (divergence), not 100.0 (false convergence)
        let mut progress = ProgressState::new();
        progress.previous_error_count = Some(0);
        progress.record_error("error 1".to_string());

        let reduction = progress.error_reduction_percent().unwrap();
        assert_eq!(
            reduction, 0.0,
            "Regression from clean state should have 0% reduction, not 100%"
        );
    }

    #[test]
    fn test_crit006_error_reduction_clean_stays_clean() {
        // CRIT-006: When previous=0 and current=0, should return 100.0 (full reduction)
        let mut progress = ProgressState::new();
        progress.previous_error_count = Some(0);

        let reduction = progress.error_reduction_percent().unwrap();
        assert_eq!(reduction, 100.0, "Clean state maintained should be 100% reduction");
    }

    #[test]
    fn test_compound001_clean_to_errors_to_capped() {
        // COMPOUND-001: Pipeline starts clean (0 errors) → gains errors → verify
        // self-healing correctly detects DIVERGENCE, not convergence.
        // This tests the interaction of CRIT-006 + CRIT-007.

        let mut loop_config = BuildLoop::new(10);

        // Initial state: clean (0 errors, no previous)
        assert_eq!(loop_config.evaluate(), BuildOutcome::TestsPassing);

        // Move to iteration 1 and record that it was clean
        loop_config.next_iteration();
        assert_eq!(loop_config.progress.previous_error_count, Some(0));

        // Now introduce errors (simulating a regression)
        for i in 0..5 {
            loop_config.progress.record_error(format!("error {}", i));
        }
        assert_eq!(loop_config.progress.error_count, 5);

        // CRITICAL: Should evaluate as Diverging (errors introduced from clean state)
        let outcome = loop_config.evaluate();
        assert_eq!(
            outcome,
            BuildOutcome::Diverging,
            "Regression from clean state should be detected as Diverging, not Converging"
        );

        // Move to iteration 2
        loop_config.next_iteration();
        assert_eq!(loop_config.progress.iteration, 2);
        assert_eq!(loop_config.progress.previous_error_count, Some(5));

        // Now reduce errors (converging)
        loop_config.progress.last_errors.clear();
        for i in 0..2 {
            loop_config.progress.record_error(format!("error {}", i));
        }
        assert_eq!(loop_config.progress.error_count, 2);

        // Should evaluate as Converging (errors reduced from 5 to 2)
        let outcome = loop_config.evaluate();
        assert!(matches!(outcome, BuildOutcome::Converging { .. }));
    }

    #[test]
    fn test_low001_should_run_full_test_no_double_trigger() {
        // LOW-001: should_run_full_test should not trigger on both iteration 0 and 1.
        // Iteration 0 is the initial state (no work done), iteration 1 is the first real iteration.
        let mut loop_config = BuildLoop::new(10);
        loop_config.fast_test_interval = 5;

        // Iteration 0 (initial state) should NOT trigger full test
        // (since 0 % 5 == 0 matches the modulo condition)
        assert!(!loop_config.should_run_full_test(), "Iteration 0 should not run full test");

        // Iteration 1 (first iteration) should trigger full test
        loop_config.next_iteration();
        assert_eq!(loop_config.progress.iteration, 1);
        assert!(loop_config.should_run_full_test(), "Iteration 1 should run full test");

        // Iteration 2 should NOT trigger (neither condition met)
        loop_config.next_iteration();
        assert_eq!(loop_config.progress.iteration, 2);
        assert!(!loop_config.should_run_full_test(), "Iteration 2 should not run full test");

        // Iteration 5 should trigger (modulo condition, but only when > 1)
        loop_config.progress.iteration = 5;
        assert!(
            loop_config.should_run_full_test(),
            "Iteration 5 should run full test (5 % 5 == 0)"
        );

        // Verify no double-trigger: iteration 0 % 5 == 0, but should still be false
        loop_config.progress.iteration = 0;
        assert!(
            !loop_config.should_run_full_test(),
            "Iteration 0 should not double-trigger on modulo"
        );
    }
}
