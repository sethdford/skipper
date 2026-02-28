//! Outcome learning and weight adjustment for decision engine.

use serde::{Deserialize, Serialize};

/// An outcome from a decision that was made.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Outcome {
    pub id: String,
    pub candidate_id: String,
    pub predicted_score: f64,
    pub actual_success: bool,
    pub duration_minutes: u32,
    pub cost_usd: f64,
    pub signal_source: String,
    pub created_at: String,
}

impl Outcome {
    /// Create a new outcome record.
    pub fn new(
        candidate_id: String,
        predicted_score: f64,
        signal_source: String,
    ) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            candidate_id,
            predicted_score,
            actual_success: false,
            duration_minutes: 0,
            cost_usd: 0.0,
            signal_source,
            created_at: chrono::Utc::now().to_rfc3339(),
        }
    }

    /// Mark the outcome as successful.
    pub fn mark_successful(mut self) -> Self {
        self.actual_success = true;
        self
    }

    /// Set duration and cost.
    pub fn with_metrics(mut self, duration_minutes: u32, cost_usd: f64) -> Self {
        self.duration_minutes = duration_minutes;
        self.cost_usd = cost_usd;
        self
    }
}

/// A/B testing group assignment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ABGroup {
    Control,
    Treatment,
}

/// A/B test cohort.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ABTest {
    pub id: String,
    pub name: String,
    pub control_outcomes: Vec<Outcome>,
    pub treatment_outcomes: Vec<Outcome>,
}

impl ABTest {
    /// Create a new A/B test.
    pub fn new(name: String) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            name,
            control_outcomes: vec![],
            treatment_outcomes: vec![],
        }
    }

    /// Get the success rate for a group.
    pub fn success_rate(&self, group: ABGroup) -> f64 {
        let outcomes = match group {
            ABGroup::Control => &self.control_outcomes,
            ABGroup::Treatment => &self.treatment_outcomes,
        };

        if outcomes.is_empty() {
            return 0.0;
        }

        let successes = outcomes.iter().filter(|o| o.actual_success).count() as f64;
        successes / outcomes.len() as f64
    }

    /// Record an outcome to a group.
    pub fn record_outcome(&mut self, group: ABGroup, outcome: Outcome) {
        match group {
            ABGroup::Control => self.control_outcomes.push(outcome),
            ABGroup::Treatment => self.treatment_outcomes.push(outcome),
        }
    }
}

/// Scoring weights with learning capability.
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
    /// Adjust weights via exponential moving average based on outcomes.
    /// After 10+ successful outcomes, shifts weight toward that signal.
    pub fn adjust_from_outcomes(&mut self, outcomes: &[Outcome]) {
        if outcomes.len() < 10 {
            return;
        }

        // Simple EMA: shift weights of successful signals higher
        let successful_outcomes: Vec<_> = outcomes.iter().filter(|o| o.actual_success).collect();
        let success_rate = successful_outcomes.len() as f64 / outcomes.len() as f64;

        // Alpha parameter for EMA
        let alpha = 0.1;

        // If success rate is high, increase confidence weight
        if success_rate > 0.7 {
            self.confidence = self.confidence * (1.0 - alpha) + 0.25 * alpha;
            self.risk = self.risk * (1.0 - alpha) + 0.05 * alpha;
        } else if success_rate < 0.3 {
            // If success rate is low, increase risk weight
            self.risk = self.risk * (1.0 - alpha) + 0.15 * alpha;
            self.confidence = self.confidence * (1.0 - alpha) + 0.10 * alpha;
        }
    }

    /// Normalize weights to sum to 1.0.
    pub fn normalize(&mut self) {
        let sum = self.impact + self.urgency + self.effort + self.confidence + self.risk;
        if sum > 0.0 {
            self.impact /= sum;
            self.urgency /= sum;
            self.effort /= sum;
            self.confidence /= sum;
            self.risk /= sum;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_outcome_creation() {
        let outcome = Outcome::new(
            "cand-1".to_string(),
            75.0,
            "security".to_string(),
        );
        assert_eq!(outcome.candidate_id, "cand-1");
        assert_eq!(outcome.predicted_score, 75.0);
        assert!(!outcome.actual_success);
    }

    #[test]
    fn test_outcome_mark_successful() {
        let outcome = Outcome::new(
            "cand-1".to_string(),
            75.0,
            "security".to_string(),
        )
        .mark_successful();
        assert!(outcome.actual_success);
    }

    #[test]
    fn test_outcome_with_metrics() {
        let outcome = Outcome::new(
            "cand-1".to_string(),
            75.0,
            "security".to_string(),
        )
        .with_metrics(30, 5.5);
        assert_eq!(outcome.duration_minutes, 30);
        assert_eq!(outcome.cost_usd, 5.5);
    }

    #[test]
    fn test_ab_test_creation() {
        let test = ABTest::new("test-1".to_string());
        assert_eq!(test.name, "test-1");
        assert!(test.control_outcomes.is_empty());
    }

    #[test]
    fn test_ab_test_success_rate() {
        let mut test = ABTest::new("test-1".to_string());
        let outcome1 = Outcome::new("c1".to_string(), 50.0, "test".to_string()).mark_successful();
        let outcome2 = Outcome::new("c2".to_string(), 50.0, "test".to_string());
        test.record_outcome(ABGroup::Control, outcome1);
        test.record_outcome(ABGroup::Control, outcome2);

        let rate = test.success_rate(ABGroup::Control);
        assert_eq!(rate, 0.5);
    }

    #[test]
    fn test_scoring_weights_default() {
        let weights = ScoringWeights::default();
        assert_eq!(weights.impact, 0.30);
        assert_eq!(weights.urgency, 0.25);
        assert_eq!(weights.effort, 0.20);
        assert_eq!(weights.confidence, 0.15);
        assert_eq!(weights.risk, 0.10);
    }

    #[test]
    fn test_scoring_weights_normalize() {
        let mut weights = ScoringWeights {
            impact: 30.0,
            urgency: 25.0,
            effort: 20.0,
            confidence: 15.0,
            risk: 10.0,
        };
        weights.normalize();
        let sum = weights.impact + weights.urgency + weights.effort + weights.confidence + weights.risk;
        assert!((sum - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_scoring_weights_adjust_from_outcomes() {
        let mut weights = ScoringWeights::default();
        let mut outcomes = vec![];

        // Create 12 successful outcomes
        for i in 0..12 {
            let outcome = Outcome::new(format!("c{}", i), 75.0, "test".to_string())
                .mark_successful();
            outcomes.push(outcome);
        }

        weights.adjust_from_outcomes(&outcomes);
        // After adjustment, confidence should have increased
        assert!(weights.confidence > 0.15);
    }

    #[test]
    fn test_outcome_serialize() {
        let outcome = Outcome::new(
            "cand-1".to_string(),
            75.0,
            "security".to_string(),
        );
        let json = serde_json::to_string(&outcome).unwrap();
        assert!(json.contains("\"candidate_id\":\"cand-1\""));
    }
}
