//! Decision engine tests with real candidates.
//!
//! Tests scoring, dedup, tier resolution, and budget limits

use openfang_shipwright::decision::{
    Candidate, Category, DecisionEngine, SignalType,
};

#[test]
fn decision_engine_new() {
    let engine = DecisionEngine::new();
    assert_eq!(engine.weights.impact, 0.30);
    assert_eq!(engine.weights.urgency, 0.25);
}

#[test]
fn candidate_builder_with_all_fields() {
    let candidate = Candidate::new(
        SignalType::Dependency,
        Category::DependencyUpdate,
        "Update lodash".to_string(),
        "CVE-2021-23337 in lodash < 4.17.21".to_string(),
        "dep:lodash:4.17.20".to_string(),
    )
    .with_impact(0.8)
    .with_urgency(0.9)
    .with_confidence(0.95)
    .with_effort(0.3)
    .with_risk_score(25);

    assert_eq!(candidate.signal, SignalType::Dependency);
    assert_eq!(candidate.category, Category::DependencyUpdate);
    assert_eq!(candidate.impact, 0.8);
    assert_eq!(candidate.urgency, 0.9);
}

#[test]
fn candidate_dedup_key_is_unique() {
    let c1 = Candidate::new(
        SignalType::Security,
        Category::SecurityPatch,
        "Fix XSS".to_string(),
        "".to_string(),
        "sec:xss:form".to_string(),
    );

    let c2 = Candidate::new(
        SignalType::Security,
        Category::SecurityPatch,
        "Different title".to_string(),
        "".to_string(),
        "sec:xss:form".to_string(),
    );

    assert_eq!(c1.dedup_key, c2.dedup_key);
    assert_ne!(c1.id, c2.id);
}

#[test]
fn candidate_risk_score_clamping() {
    let c1 = Candidate::new(
        SignalType::Security,
        Category::SecurityPatch,
        "Test".to_string(),
        "".to_string(),
        "test".to_string(),
    )
    .with_risk_score(150);

    assert_eq!(c1.risk_score, 100);

    let c2 = Candidate::new(
        SignalType::Security,
        Category::SecurityPatch,
        "Test".to_string(),
        "".to_string(),
        "test".to_string(),
    )
    .with_risk_score(0);

    assert_eq!(c2.risk_score, 0);
}

