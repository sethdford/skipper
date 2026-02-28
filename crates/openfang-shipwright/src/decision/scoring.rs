//! Candidate scoring with configurable weights.
//!
//! Implements the scoring formula:
//! value = (impact * 0.30) + (urgency * 0.25) + (confidence * 0.15) - (effort * 0.20) - (risk * 0.10)

use super::signals::Candidate;
use serde::{Deserialize, Serialize};

/// Configurable scoring weights.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoringWeights {
    pub impact: f64,
    pub urgency: f64,
    pub effort: f64,
    pub confidence: f64,
    pub risk: f64,
}

impl Default for ScoringWeights {
    fn default() -> Self {
        Self {
            impact: 0.30,
            urgency: 0.25,
            effort: 0.20,
            confidence: 0.15,
            risk: 0.10,
        }
    }
}

impl ScoringWeights {
    /// Validate that weights sum to 1.0.
    pub fn is_valid(&self) -> bool {
        let sum = self.impact + self.urgency + self.effort + self.confidence + self.risk;
        (sum - 1.0).abs() < 0.01
    }
}

/// Score a candidate using the scoring formula.
pub fn score_candidate(candidate: &Candidate, weights: &ScoringWeights) -> f64 {
    let risk_normalized = (candidate.risk_score as f64) / 100.0;
    let value = (candidate.impact * weights.impact)
        + (candidate.urgency * weights.urgency)
        + (candidate.confidence * weights.confidence)
        - (candidate.effort * weights.effort)
        - (risk_normalized * weights.risk);
    value.clamp(0.0, 1.0)
}

/// Exponential moving average for weight adjustment.
pub fn adjust_weights_ema(
    current: &ScoringWeights,
    success_count: u32,
    alpha: f64,
) -> ScoringWeights {
    if success_count < 10 {
        return current.clone();
    }

    let adjustment = 1.0 / success_count as f64;
    ScoringWeights {
        impact: current.impact + (adjustment * alpha),
        urgency: current.urgency + (adjustment * alpha),
        effort: current.effort,
        confidence: current.confidence,
        risk: current.risk - (adjustment * alpha),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decision::signals::{Category, Candidate, SignalType};

    #[test]
    fn test_default_weights() {
        let weights = ScoringWeights::default();
        assert!(weights.is_valid());
    }

    #[test]
    fn test_score_critical_cve() {
        let candidate = Candidate::new(
            SignalType::Security,
            Category::SecurityPatch,
            "Critical CVE".to_string(),
            "Critical vulnerability".to_string(),
            "cve-key".to_string(),
        )
        .with_risk_score(95)
        .with_confidence(0.99)
        .with_impact(1.0)
        .with_urgency(1.0)
        .with_effort(0.3);

        let weights = ScoringWeights::default();
        let score = score_candidate(&candidate, &weights);
        // score = (1.0 * 0.30) + (1.0 * 0.25) + (0.99 * 0.15) - (0.3 * 0.20) - (0.95 * 0.10)
        // = 0.30 + 0.25 + 0.1485 - 0.06 - 0.095 = 0.5635
        assert!(score > 0.5);
        assert!(score < 0.6);
    }

    #[test]
    fn test_score_style_nit() {
        let candidate = Candidate::new(
            SignalType::Documentation,
            Category::Refactoring,
            "Fix typo in comment".to_string(),
            "Minor typo".to_string(),
            "typo-key".to_string(),
        )
        .with_risk_score(5)
        .with_confidence(1.0)
        .with_impact(0.05)
        .with_urgency(0.1)
        .with_effort(0.95);

        let weights = ScoringWeights::default();
        let score = score_candidate(&candidate, &weights);
        // score = (0.05 * 0.30) + (0.1 * 0.25) + (1.0 * 0.15) - (0.95 * 0.20) - (0.05 * 0.10)
        // = 0.015 + 0.025 + 0.15 - 0.19 - 0.005 = -0.005, clamped to 0.0
        assert!(score == 0.0);
    }

    #[test]
    fn test_score_with_custom_weights() {
        let candidate = Candidate::new(
            SignalType::Performance,
            Category::Performance,
            "Optimize query".to_string(),
            "Database query too slow".to_string(),
            "perf-key".to_string(),
        )
        .with_impact(0.8)
        .with_urgency(0.6)
        .with_effort(0.5)
        .with_confidence(0.85)
        .with_risk_score(30);

        let custom_weights = ScoringWeights {
            impact: 0.40,
            urgency: 0.20,
            effort: 0.15,
            confidence: 0.15,
            risk: 0.10,
        };

        let score = score_candidate(&candidate, &custom_weights);
        // Score: 0.4625
        // = (0.8 * 0.40) + (0.6 * 0.20) + (0.85 * 0.15) - (0.5 * 0.15) - (0.30 * 0.10)
        assert!(score > 0.46);
        assert!(score < 0.47);
    }

    #[test]
    fn test_score_clamped_to_one() {
        let candidate = Candidate::new(
            SignalType::Security,
            Category::SecurityPatch,
            "test".to_string(),
            "test".to_string(),
            "test".to_string(),
        )
        .with_impact(1.0)
        .with_urgency(1.0)
        .with_effort(0.0)
        .with_confidence(1.0)
        .with_risk_score(0);

        let weights = ScoringWeights::default();
        let score = score_candidate(&candidate, &weights);
        // score = (1.0 * 0.30) + (1.0 * 0.25) + (1.0 * 0.15) - (0.0 * 0.20) - (0.0 * 0.10)
        // = 0.30 + 0.25 + 0.15 - 0 - 0 = 0.70
        assert!(score > 0.69);
        assert!(score < 0.71);
    }

    #[test]
    fn test_score_clamped_to_zero() {
        let candidate = Candidate::new(
            SignalType::Documentation,
            Category::Feature,
            "test".to_string(),
            "test".to_string(),
            "test".to_string(),
        )
        .with_impact(0.0)
        .with_urgency(0.0)
        .with_effort(1.0)
        .with_confidence(0.0)
        .with_risk_score(100);

        let weights = ScoringWeights::default();
        let score = score_candidate(&candidate, &weights);
        // score = (0.0 * 0.30) + (0.0 * 0.25) + (0.0 * 0.15) - (1.0 * 0.20) - (1.0 * 0.10)
        // = 0 + 0 + 0 - 0.20 - 0.10 = -0.30, clamped to 0.0
        assert_eq!(score, 0.0);
    }

    #[test]
    fn test_adjust_weights_ema_insufficient_data() {
        let original = ScoringWeights::default();
        let adjusted = adjust_weights_ema(&original, 5, 0.05);
        // Should not adjust with fewer than 10 successes
        assert_eq!(adjusted.impact, original.impact);
    }

    #[test]
    fn test_adjust_weights_ema_with_sufficient_data() {
        let original = ScoringWeights::default();
        let adjusted = adjust_weights_ema(&original, 10, 0.05);
        // Should adjust
        assert!(adjusted.impact > original.impact);
        assert!(adjusted.risk < original.risk);
    }

    #[test]
    fn test_invalid_weights_sum() {
        let weights = ScoringWeights {
            impact: 0.3,
            urgency: 0.25,
            effort: 0.2,
            confidence: 0.1, // Total = 0.85, not 1.0
            risk: 0.1,
        };
        assert!(!weights.is_valid());
    }
}
