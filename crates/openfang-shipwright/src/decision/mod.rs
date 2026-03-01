//! Decision engine: collects signals, scores candidates, resolves autonomy tiers.
//!
//! Orchestrates the decision cycle:
//! 1. Collect signals from all collectors
//! 2. Dedup against open issues and recent decisions
//! 3. Score each candidate
//! 4. Resolve autonomy tier
//! 5. Enforce limits (budget, rate, halt)
//! 6. Execute (create issues, spawn pipelines)

pub mod autonomy;
pub mod scoring;
pub mod signals;

pub use autonomy::{AutonomyTier, DecisionLimits, DecisionState};
pub use scoring::score_candidate;
pub use crate::memory::learning::ScoringWeights;
pub use signals::{
    Candidate, Category, RepoContext, SignalCollector, SignalType, HandSignalCollector,
    SecurityCollector, DependencyCollector, CoverageCollector, DeadCodeCollector,
    PerformanceCollector, ArchitectureCollector, DoraCollector, DocumentationCollector,
    FailureCollector,
};

use std::collections::HashMap;

/// A scored candidate with tier.
#[derive(Debug, Clone)]
pub struct ScoredCandidate {
    pub candidate: Candidate,
    pub score: f64,
    pub tier: AutonomyTier,
}

/// Decision engine coordinator.
pub struct DecisionEngine {
    pub weights: ScoringWeights,
    pub limits: DecisionLimits,
    pub state: DecisionState,
    collectors: Vec<Box<dyn SignalCollector>>,
}

impl DecisionEngine {
    /// Create a new decision engine with default configuration.
    pub fn new() -> Self {
        Self {
            weights: ScoringWeights::default(),
            limits: DecisionLimits::default(),
            state: DecisionState::default(),
            collectors: Self::default_collectors(),
        }
    }

    /// Create a new decision engine with custom weights.
    pub fn with_weights(weights: ScoringWeights) -> Self {
        Self {
            weights,
            limits: DecisionLimits::default(),
            state: DecisionState::default(),
            collectors: Self::default_collectors(),
        }
    }

    /// Get the number of registered collectors.
    pub fn collectors_count(&self) -> usize {
        self.collectors.len()
    }

    /// Create default set of collectors.
    fn default_collectors() -> Vec<Box<dyn SignalCollector>> {
        vec![
            Box::new(signals::SecurityCollector),
            Box::new(signals::DependencyCollector),
            Box::new(signals::CoverageCollector {
                threshold_percent: 80.0,
            }),
            Box::new(signals::DeadCodeCollector),
            Box::new(signals::PerformanceCollector {
                threshold_percent: 10.0,
            }),
            Box::new(signals::ArchitectureCollector),
            Box::new(signals::DoraCollector),
            Box::new(signals::DocumentationCollector),
            Box::new(signals::FailureCollector),
            Box::new(signals::HandSignalCollector),
        ]
    }

    /// Run a decision cycle.
    pub async fn run_cycle(
        &mut self,
        ctx: &RepoContext,
    ) -> Result<Vec<ScoredCandidate>, String> {
        // 1. Check halt, budget, rate limits
        if self.state.check_halt() {
            return Err("Decision engine is halted".to_string());
        }

        if !self.state.check_budget(&self.limits) {
            return Err("Daily budget exceeded".to_string());
        }

        if !self.state.check_rate_limit(&self.limits) {
            return Err("Rate limit cooldown active".to_string());
        }

        // 2. Collect signals
        let mut candidates = Vec::new();
        for collector in &self.collectors {
            match collector.collect(ctx) {
                Ok(mut batch) => candidates.append(&mut batch),
                Err(e) => eprintln!("Collector {} error: {}", collector.name(), e),
            }
        }

        // 3. Dedup by dedup_key
        let mut deduped: HashMap<String, Candidate> = HashMap::new();
        for candidate in candidates {
            deduped
                .entry(candidate.dedup_key.clone())
                .or_insert(candidate);
        }

        // 4. Score each candidate
        let mut scored: Vec<ScoredCandidate> = deduped
            .into_values()
            .map(|candidate| {
                let score = score_candidate(&candidate, &self.weights);
                let tier = autonomy::resolve_tier(candidate.category, score);
                ScoredCandidate {
                    candidate,
                    score,
                    tier,
                }
            })
            .collect();

        // 5. Sort by score descending
        scored.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));

        Ok(scored)
    }
}

impl Default for DecisionEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_decision_engine_new() {
        let engine = DecisionEngine::new();
        assert_eq!(engine.collectors.len(), 10);
        assert_eq!(engine.state.issued_count_today, 0);
    }

    #[tokio::test]
    async fn test_decision_engine_with_weights() {
        let weights = ScoringWeights {
            impact: 0.40,
            urgency: 0.20,
            effort: 0.15,
            confidence: 0.15,
            risk: 0.10,
        };
        let engine = DecisionEngine::with_weights(weights.clone());
        assert_eq!(engine.weights.impact, 0.40);
    }

    #[tokio::test]
    async fn test_run_cycle_halted() {
        let mut engine = DecisionEngine::new();
        engine.state.halt();
        let ctx = RepoContext::new("repo".to_string(), "owner".to_string(), "/path".to_string());
        let result = engine.run_cycle(&ctx).await;
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "Decision engine is halted");
    }

    #[tokio::test]
    async fn test_run_cycle_budget_exceeded() {
        let mut engine = DecisionEngine::new();
        engine.state.issued_count_today = 20;
        let ctx = RepoContext::new("repo".to_string(), "owner".to_string(), "/path".to_string());
        let result = engine.run_cycle(&ctx).await;
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "Daily budget exceeded");
    }

    #[tokio::test]
    async fn test_run_cycle_empty_candidates() {
        let mut engine = DecisionEngine::new();
        let ctx = RepoContext::new("repo".to_string(), "owner".to_string(), "/path".to_string());
        let result = engine.run_cycle(&ctx).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap().len(), 0);
    }

    #[tokio::test]
    async fn test_scored_candidate() {
        let candidate = Candidate::new(
            SignalType::Security,
            Category::SecurityPatch,
            "CVE".to_string(),
            "Test".to_string(),
            "cve-key".to_string(),
        )
        .with_risk_score(90)
        .with_confidence(0.95)
        .with_impact(1.0)
        .with_urgency(1.0)
        .with_effort(0.2);

        let weights = ScoringWeights::default();
        let score = score_candidate(&candidate, &weights);
        let tier = autonomy::resolve_tier(candidate.category, score);

        let scored = ScoredCandidate {
            candidate: candidate.clone(),
            score,
            tier,
        };

        assert_eq!(scored.candidate.title, "CVE");
        // score = (1.0 * 0.30) + (1.0 * 0.25) + (0.95 * 0.15) - (0.2 * 0.20) - (0.90 * 0.10)
        // = 0.30 + 0.25 + 0.1425 - 0.04 - 0.09 = 0.5625
        assert!(scored.score > 0.55);
        assert!(scored.score < 0.6);
        assert_eq!(scored.tier, AutonomyTier::Auto);
    }

    #[test]
    fn test_default_collectors_count() {
        let engine = DecisionEngine::new();
        assert_eq!(engine.collectors.len(), 10);
    }

    #[test]
    fn test_decision_engine_default() {
        let engine = DecisionEngine::default();
        assert!(!engine.state.halt_flag);
        assert!(engine.weights.is_valid());
    }
}
