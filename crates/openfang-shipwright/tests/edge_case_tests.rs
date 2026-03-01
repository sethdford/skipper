//! Edge case tests for numeric boundaries.

use openfang_shipwright::pipeline::self_healing::{BuildLoop, ProgressState};

#[test]
fn progress_state_error_reduction_with_zero_previous() {
    let mut progress = ProgressState::new();
    progress.previous_error_count = Some(0);
    progress.record_error("error 1".to_string());
    progress.record_error("error 2".to_string());

    let reduction = progress.error_reduction_percent().unwrap();
    assert_eq!(reduction, 0.0, "Regression from clean state should be 0.0");
}

#[test]
fn progress_state_error_reduction_with_both_zero() {
    let mut progress = ProgressState::new();
    progress.previous_error_count = Some(0);

    let reduction = progress.error_reduction_percent().unwrap();
    assert_eq!(reduction, 100.0, "Both zero should be 100% reduction");
}

#[test]
fn build_loop_with_zero_max_iterations() {
    let loop_config = BuildLoop::new(0);
    let outcome = loop_config.evaluate();
    assert!(matches!(outcome, openfang_shipwright::pipeline::self_healing::BuildOutcome::Exhausted));
}

#[test]
fn progress_state_record_error_trim_at_11() {
    let mut progress = ProgressState::new();

    for i in 0..11 {
        progress.record_error(format!("error {}", i));
    }

    assert_eq!(progress.error_count, 10, "Should be trimmed to 10");
    assert_eq!(progress.last_errors.len(), 10);
}

#[test]
fn progress_state_many_errors_stays_at_cap() {
    let mut progress = ProgressState::new();

    for i in 0..100 {
        progress.record_error(format!("error {}", i));
    }

    assert_eq!(progress.error_count, 10, "Should stay capped at 10");
    assert_eq!(progress.last_errors.len(), 10);
}

