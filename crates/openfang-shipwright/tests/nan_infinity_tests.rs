//! Edge case tests for NaN/Infinity handling in scoring and DORA calculations.

#[cfg(test)]
mod tests {
    use openfang_shipwright::decision::signals::{Candidate, Category, SignalType};
    use openfang_shipwright::intelligence::dora::DoraMetrics;

    #[test]
    fn test_candidate_confidence_clamp_normal_value() {
        let mut candidate = Candidate::new(
            SignalType::Security,
            Category::SecurityPatch,
            "test".to_string(),
            "desc".to_string(),
            "key".to_string(),
        );
        candidate = candidate.with_confidence(0.75);
        assert_eq!(candidate.confidence, 0.75);
    }

    #[test]
    fn test_candidate_confidence_clamp_zero() {
        let mut candidate = Candidate::new(
            SignalType::Security,
            Category::SecurityPatch,
            "test".to_string(),
            "desc".to_string(),
            "key".to_string(),
        );
        candidate = candidate.with_confidence(0.0);
        assert_eq!(candidate.confidence, 0.0);
    }

    #[test]
    fn test_candidate_confidence_clamp_one() {
        let mut candidate = Candidate::new(
            SignalType::Security,
            Category::SecurityPatch,
            "test".to_string(),
            "desc".to_string(),
            "key".to_string(),
        );
        candidate = candidate.with_confidence(1.0);
        assert_eq!(candidate.confidence, 1.0);
    }

    #[test]
    fn test_candidate_confidence_clamp_over_one() {
        let mut candidate = Candidate::new(
            SignalType::Security,
            Category::SecurityPatch,
            "test".to_string(),
            "desc".to_string(),
            "key".to_string(),
        );
        candidate = candidate.with_confidence(1.5);  // Over 1.0, should clamp
        assert_eq!(candidate.confidence, 1.0);
    }

    #[test]
    fn test_candidate_confidence_clamp_negative() {
        let mut candidate = Candidate::new(
            SignalType::Security,
            Category::SecurityPatch,
            "test".to_string(),
            "desc".to_string(),
            "key".to_string(),
        );
        candidate = candidate.with_confidence(-0.5);  // Negative, should clamp to 0.0
        assert_eq!(candidate.confidence, 0.0);
    }

    #[test]
    fn test_candidate_impact_clamp_boundaries() {
        let mut candidate = Candidate::new(
            SignalType::Security,
            Category::SecurityPatch,
            "test".to_string(),
            "desc".to_string(),
            "key".to_string(),
        );

        // Test lower boundary
        candidate = candidate.with_impact(0.0);
        assert_eq!(candidate.impact, 0.0);

        // Test upper boundary
        candidate = candidate.with_impact(1.0);
        assert_eq!(candidate.impact, 1.0);

        // Test over-clamp
        candidate = candidate.with_impact(2.0);
        assert_eq!(candidate.impact, 1.0);
    }

    #[test]
    fn test_candidate_urgency_clamp_boundaries() {
        let mut candidate = Candidate::new(
            SignalType::Security,
            Category::SecurityPatch,
            "test".to_string(),
            "desc".to_string(),
            "key".to_string(),
        );

        // Test lower boundary
        candidate = candidate.with_urgency(0.0);
        assert_eq!(candidate.urgency, 0.0);

        // Test upper boundary
        candidate = candidate.with_urgency(1.0);
        assert_eq!(candidate.urgency, 1.0);

        // Test over-clamp
        candidate = candidate.with_urgency(5.0);
        assert_eq!(candidate.urgency, 1.0);
    }

    #[test]
    fn test_candidate_effort_clamp_boundaries() {
        let mut candidate = Candidate::new(
            SignalType::Security,
            Category::SecurityPatch,
            "test".to_string(),
            "desc".to_string(),
            "key".to_string(),
        );

        candidate = candidate.with_effort(0.0);
        assert_eq!(candidate.effort, 0.0);

        candidate = candidate.with_effort(1.0);
        assert_eq!(candidate.effort, 1.0);

        candidate = candidate.with_effort(1.5);
        assert_eq!(candidate.effort, 1.0);
    }

    #[test]
    fn test_risk_score_clamp_boundaries() {
        let mut candidate = Candidate::new(
            SignalType::Security,
            Category::SecurityPatch,
            "test".to_string(),
            "desc".to_string(),
            "key".to_string(),
        );

        candidate = candidate.with_risk_score(0);
        assert_eq!(candidate.risk_score, 0);

        candidate = candidate.with_risk_score(100);
        assert_eq!(candidate.risk_score, 100);

        candidate = candidate.with_risk_score(255);  // Over 100, should clamp
        assert_eq!(candidate.risk_score, 100);
    }

    #[test]
    fn test_dora_metrics_normal_values() {
        let metrics = DoraMetrics::new(2.3, 1.5, 0.18, 0.5);
        assert!(metrics.lead_time_hours.is_finite());
        assert!(metrics.deploy_frequency_per_day.is_finite());
        assert!(metrics.change_failure_rate.is_finite());
        assert!(metrics.mttr_hours.is_finite());
    }

    #[test]
    fn test_dora_metrics_zero_values() {
        let metrics = DoraMetrics::new(0.0, 0.0, 0.0, 0.0);
        assert_eq!(metrics.lead_time_hours, 0.0);
        assert_eq!(metrics.deploy_frequency_per_day, 0.0);
        assert_eq!(metrics.change_failure_rate, 0.0);
        assert_eq!(metrics.mttr_hours, 0.0);
    }

    #[test]
    fn test_dora_metrics_all_finite() {
        let metrics = DoraMetrics::new(10.5, 2.3, 0.05, 60.0);
        // Verify all values are finite (not NaN or Infinity)
        assert!(metrics.lead_time_hours.is_finite());
        assert!(metrics.deploy_frequency_per_day.is_finite());
        assert!(metrics.change_failure_rate.is_finite());
        assert!(metrics.mttr_hours.is_finite());
    }
}
