//! Convergence detection tests for build loops.

use openfang_shipwright::pipeline::self_healing::{BuildLoop, BuildOutcome};

#[test]
fn convergence_steady_error_decrease() {
    let mut loop_config = BuildLoop::new(20);

    loop_config.progress.record_error("e1".to_string());
    loop_config.progress.record_error("e2".to_string());
    loop_config.progress.record_error("e3".to_string());

    let outcome = loop_config.evaluate();
    assert!(matches!(outcome, BuildOutcome::Converging { .. }));
}

#[test]
fn convergence_from_zero_to_errors_diverges() {
    let mut loop_config = BuildLoop::new(20);

    loop_config.progress.previous_error_count = Some(0);
    for i in 0..5 {
        loop_config.progress.record_error(format!("e{}", i));
    }

    let outcome = loop_config.evaluate();
    assert_eq!(outcome, BuildOutcome::Diverging);
}

#[test]
fn convergence_error_count_equals_previous() {
    let mut loop_config = BuildLoop::new(20);

    loop_config.progress.previous_error_count = Some(5);
    for i in 0..5 {
        loop_config.progress.record_error(format!("e{}", i));
    }

    let outcome = loop_config.evaluate();
    assert!(matches!(outcome, BuildOutcome::Converging { issues_remaining: 5 }));
}

#[test]
fn convergence_at_max_iterations() {
    let mut loop_config = BuildLoop::new(5);
    loop_config.progress.iteration = 5;
    let outcome = loop_config.evaluate();
    assert_eq!(outcome, BuildOutcome::Exhausted);
}

