//! Intelligence layer: DORA metrics, risk prediction, self-optimization.
//!
//! Provides:
//! - DORA metrics calculation and performance level classification
//! - Risk prediction based on hotspots, complexity, and coverage
//! - Anomaly detection using statistical methods
//! - Model routing based on risk
//! - Self-optimization suggestions based on DORA trends

pub mod dora;
pub mod optimization;
pub mod prediction;

pub use dora::{classify_dora_level, DoraLevel, DoraMetrics};
pub use optimization::{suggest_config_change, adaptive_cycles, OptimizationSuggestion};
pub use prediction::{
    detect_anomaly, predict_risk, recommend_model, RiskPrediction,
};
pub use crate::pipeline::ModelChoice;

/// Intelligence engine coordinator.
pub struct IntelligenceEngine {
    pub enabled: bool,
    pub cache_ttl_seconds: u64,
}

impl IntelligenceEngine {
    /// Create a new intelligence engine.
    pub fn new(enabled: bool, cache_ttl_seconds: u64) -> Self {
        Self {
            enabled,
            cache_ttl_seconds,
        }
    }

    /// Create with default settings (enabled, 3600 sec TTL).
    pub fn default_enabled() -> Self {
        Self {
            enabled: true,
            cache_ttl_seconds: 3600,
        }
    }
}

impl Default for IntelligenceEngine {
    fn default() -> Self {
        Self::default_enabled()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_intelligence_engine_new() {
        let engine = IntelligenceEngine::new(true, 1800);
        assert!(engine.enabled);
        assert_eq!(engine.cache_ttl_seconds, 1800);
    }

    #[test]
    fn test_intelligence_engine_default_enabled() {
        let engine = IntelligenceEngine::default_enabled();
        assert!(engine.enabled);
        assert_eq!(engine.cache_ttl_seconds, 3600);
    }

    #[test]
    fn test_intelligence_engine_disabled() {
        let engine = IntelligenceEngine::new(false, 3600);
        assert!(!engine.enabled);
    }

    #[test]
    fn test_intelligence_engine_default() {
        let engine = IntelligenceEngine::default();
        assert!(engine.enabled);
    }
}
