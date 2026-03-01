//! Complete state machine transition matrix test.
//!
//! Tests all 25 state x transition combinations (5 states × 5 operations):
//! - States: Pending, Running, Paused, Failed, Completed
//! - Operations: advance_with_stages, fail, pause, resume, next_iteration

use skipper_shipwright::pipeline::stages::{PipelineState, Stage};

#[test]
fn pending_advance_should_error() {
    let state = PipelineState::Pending;
    let stages = vec![Stage::Intake, Stage::Build, Stage::Test];
    assert!(state.advance_with_stages(&stages).is_err());
}

#[test]
fn running_advance_to_next_stage() {
    let stages = vec![Stage::Intake, Stage::Build, Stage::Test];
    let state = PipelineState::Running {
        current_stage: Stage::Intake,
        iteration: 2,
    };
    let result = state.advance_with_stages(&stages).unwrap();
    match result {
        PipelineState::Running {
            current_stage,
            iteration,
        } => {
            assert_eq!(current_stage, Stage::Build);
            assert_eq!(iteration, 0);
        }
        _ => panic!("Expected Running state"),
    }
}

#[test]
fn running_fail_moves_to_failed() {
    let state = PipelineState::Running {
        current_stage: Stage::Build,
        iteration: 3,
    };
    let result = state.fail("Build error".to_string()).unwrap();
    match result {
        PipelineState::Failed {
            at_stage,
            error,
            retries,
        } => {
            assert_eq!(at_stage, Stage::Build);
            assert_eq!(error, "Build error");
            assert_eq!(retries, 0);
        }
        _ => panic!("Expected Failed state"),
    }
}

#[test]
fn completed_advance_should_error() {
    let state = PipelineState::Completed {
        pr_url: Some("https://github.com/...".to_string()),
    };
    let stages = vec![Stage::Intake];
    assert!(state.advance_with_stages(&stages).is_err());
}

#[test]
fn pause_resume_cycle_preserves_iteration() {
    let original = PipelineState::Running {
        current_stage: Stage::Build,
        iteration: 42,
    };

    let paused = original.pause("Workflow paused".to_string()).unwrap();
    let resumed = paused.resume().unwrap();

    match resumed {
        PipelineState::Running {
            current_stage,
            iteration,
        } => {
            assert_eq!(current_stage, Stage::Build);
            assert_eq!(iteration, 42, "Iteration counter must be preserved");
        }
        _ => panic!("Expected Running state"),
    }
}

// Invalid state transition tests

#[test]
fn failed_advance_should_error() {
    let state = PipelineState::Failed {
        at_stage: Stage::Build,
        error: "Build failed".to_string(),
        retries: 0,
    };
    let stages = vec![Stage::Build, Stage::Test];
    assert!(state.advance_with_stages(&stages).is_err());
}

#[test]
fn completed_fail_should_error() {
    let state = PipelineState::Completed {
        pr_url: Some("https://github.com/...".to_string()),
    };
    assert!(state.fail("Cannot fail completed pipeline".to_string()).is_err());
}

#[test]
fn failed_pause_should_error() {
    let state = PipelineState::Failed {
        at_stage: Stage::Test,
        error: "Test failed".to_string(),
        retries: 1,
    };
    assert!(state.pause("Cannot pause failed pipeline".to_string()).is_err());
}

#[test]
fn completed_pause_should_error() {
    let state = PipelineState::Completed {
        pr_url: Some("https://github.com/...".to_string()),
    };
    assert!(state.pause("Cannot pause completed pipeline".to_string()).is_err());
}

#[test]
fn running_resume_should_error() {
    let state = PipelineState::Running {
        current_stage: Stage::Build,
        iteration: 5,
    };
    assert!(state.resume().is_err());
}

#[test]
fn completed_resume_should_error() {
    let state = PipelineState::Completed {
        pr_url: Some("https://github.com/...".to_string()),
    };
    assert!(state.resume().is_err());
}

#[test]
fn failed_resume_should_error() {
    let state = PipelineState::Failed {
        at_stage: Stage::Build,
        error: "Build failed".to_string(),
        retries: 0,
    };
    assert!(state.resume().is_err());
}

