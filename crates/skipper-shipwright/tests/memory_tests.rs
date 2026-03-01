//! Memory system tests with failure patterns and learning.

use skipper_shipwright::memory::{
    ShipwrightMemory, FailurePattern, ScoringWeights, ABTest,
};
use skipper_shipwright::pipeline::Stage;

#[test]
fn memory_new() {
    let memory = ShipwrightMemory::new();
    assert_eq!(memory.search_similar_failures("test", "repo", 10).len(), 0);
}

#[test]
fn memory_store_and_retrieve_single_failure() {
    let memory = ShipwrightMemory::new();

    let pattern = FailurePattern::with_stage(
        "myrepo".to_string(),
        Stage::Build,
        "CompilationError".to_string(),
        "compilation error".to_string(),
        "missing import".to_string(),
        "added use statement".to_string(),
    );

    memory.store_failure(pattern);

    let results = memory.search_similar_failures("compilation", "myrepo", 10);
    assert_eq!(results.len(), 1);
}

#[test]
fn memory_store_large_number_of_failures() {
    let memory = ShipwrightMemory::new();

    for i in 0..100 {
        let pattern = FailurePattern::with_stage(
            "myrepo".to_string(),
            Stage::Build,
            "Error".to_string(),
            format!("error_signature_{}", i),
            format!("root_cause_{}", i),
            format!("fix_{}", i),
        );
        memory.store_failure(pattern);
    }

    let results = memory.search_similar_failures("error", "myrepo", 1000);
    assert_eq!(results.len(), 100);
}

#[test]
fn memory_search_respects_limit() {
    let memory = ShipwrightMemory::new();

    for i in 0..50 {
        let pattern = FailurePattern::with_stage(
            "myrepo".to_string(),
            Stage::Build,
            "CompilationError".to_string(),
            "compilation error".to_string(),
            format!("cause {}", i),
            format!("fix {}", i),
        );
        memory.store_failure(pattern);
    }

    let results = memory.search_similar_failures("compilation", "myrepo", 10);
    assert_eq!(results.len(), 10, "Should limit to 10 results");
}

#[test]
fn memory_search_case_insensitive() {
    let memory = ShipwrightMemory::new();

    let pattern = FailurePattern::with_stage(
        "myrepo".to_string(),
        Stage::Build,
        "CompilationError".to_string(),
        "COMPILATION ERROR".to_string(),
        "cause".to_string(),
        "fix".to_string(),
    );
    memory.store_failure(pattern);

    let results = memory.search_similar_failures("compilation", "myrepo", 10);
    assert_eq!(results.len(), 1, "Search should be case insensitive");
}

#[test]
fn memory_scoring_weights_default() {
    let weights = ScoringWeights::default();
    assert_eq!(weights.impact, 0.30);
    assert_eq!(weights.urgency, 0.25);
    assert_eq!(weights.effort, 0.20);
    assert_eq!(weights.confidence, 0.15);
    assert_eq!(weights.risk, 0.10);
}

#[test]
fn memory_ab_test_creation() {
    let ab_test = ABTest::new("test_hypothesis".to_string());

    assert_eq!(ab_test.name, "test_hypothesis");
    assert_eq!(ab_test.control_outcomes.len(), 0);
}

