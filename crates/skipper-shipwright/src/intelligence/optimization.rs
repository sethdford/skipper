//! Self-optimization based on DORA metrics and learning.

use super::dora::{classify_dora_level, DoraMetrics};
use serde::{Deserialize, Serialize};

/// Optimization suggestion.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OptimizationSuggestion {
    pub field: String,
    pub current_value: String,
    pub suggested_value: String,
    pub reason: String,
    pub impact: f64,
}

impl OptimizationSuggestion {
    /// Create a new suggestion.
    pub fn new(
        field: String,
        current_value: String,
        suggested_value: String,
        reason: String,
        impact: f64,
    ) -> Self {
        Self {
            field,
            current_value,
            suggested_value,
            reason,
            impact: impact.clamp(0.0, 1.0),
        }
    }
}

/// Suggest config changes based on DORA metrics.
pub fn suggest_config_change(metrics: &DoraMetrics) -> Vec<OptimizationSuggestion> {
    let mut suggestions = Vec::new();
    let _level = classify_dora_level(metrics);

    // If lead time is high, suggest more workers
    if metrics.lead_time_hours > 24.0 {
        suggestions.push(OptimizationSuggestion::new(
            "max_workers".to_string(),
            "4".to_string(),
            "8".to_string(),
            "High lead time suggests parallelization opportunity".to_string(),
            0.7,
        ));
    }

    // If deploy frequency is low, suggest faster cycles
    if metrics.deploy_frequency_per_day < 1.0 / 7.0 {
        suggestions.push(OptimizationSuggestion::new(
            "poll_interval_seconds".to_string(),
            "3600".to_string(),
            "1800".to_string(),
            "Low deploy frequency suggests more frequent polling".to_string(),
            0.6,
        ));
    }

    // If CFR is high, suggest more testing
    if metrics.change_failure_rate > 0.3 {
        suggestions.push(OptimizationSuggestion::new(
            "test_intensity".to_string(),
            "normal".to_string(),
            "strict".to_string(),
            "High change failure rate suggests more rigorous testing".to_string(),
            0.8,
        ));
    }

    // If MTTR is high, suggest better monitoring
    if metrics.mttr_hours > 24.0 {
        suggestions.push(OptimizationSuggestion::new(
            "monitoring_enabled".to_string(),
            "false".to_string(),
            "true".to_string(),
            "High MTTR suggests better observability needed".to_string(),
            0.75,
        ));
    }

    suggestions
}

/// Calculate adaptive cycle count based on convergence pattern.
pub fn adaptive_cycles(
    base_cycles: u32,
    prev_error_count: u32,
    current_error_count: u32,
) -> u32 {
    if current_error_count >= prev_error_count {
        // Diverging or flat: stick with base
        base_cycles
    } else {
        // Converging: extend by 1 (up to 6 max)
        (base_cycles + 1).min(6)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_optimization_suggestion_new() {
        let sugg = OptimizationSuggestion::new(
            "max_workers".to_string(),
            "4".to_string(),
            "8".to_string(),
            "reason".to_string(),
            0.7,
        );
        assert_eq!(sugg.field, "max_workers");
        assert_eq!(sugg.impact, 0.7);
    }

    #[test]
    fn test_optimization_suggestion_impact_clamped() {
        let sugg = OptimizationSuggestion::new(
            "test".to_string(),
            "old".to_string(),
            "new".to_string(),
            "test".to_string(),
            1.5,
        );
        assert_eq!(sugg.impact, 1.0);
    }

    #[test]
    fn test_suggest_config_change_high_lead_time() {
        let metrics = DoraMetrics::new(48.0, 1.0, 0.1, 1.0);
        let suggestions = suggest_config_change(&metrics);
        assert!(suggestions.iter().any(|s| s.field == "max_workers"));
    }

    #[test]
    fn test_suggest_config_change_low_frequency() {
        let metrics = DoraMetrics::new(1.0, 1.0 / 30.0, 0.1, 1.0);
        let suggestions = suggest_config_change(&metrics);
        assert!(suggestions.iter().any(|s| s.field == "poll_interval_seconds"));
    }

    #[test]
    fn test_suggest_config_change_high_cfr() {
        let metrics = DoraMetrics::new(1.0, 1.0, 0.5, 1.0);
        let suggestions = suggest_config_change(&metrics);
        assert!(suggestions.iter().any(|s| s.field == "test_intensity"));
    }

    #[test]
    fn test_suggest_config_change_high_mttr() {
        let metrics = DoraMetrics::new(1.0, 1.0, 0.1, 48.0);
        let suggestions = suggest_config_change(&metrics);
        assert!(suggestions.iter().any(|s| s.field == "monitoring_enabled"));
    }

    #[test]
    fn test_suggest_config_change_elite_metrics() {
        let metrics = DoraMetrics::new(0.5, 10.0, 0.1, 0.25);
        let suggestions = suggest_config_change(&metrics);
        // Elite metrics should generate no suggestions
        assert!(suggestions.is_empty());
    }

    #[test]
    fn test_adaptive_cycles_converging() {
        let cycles = adaptive_cycles(3, 10, 5);
        assert_eq!(cycles, 4);
    }

    #[test]
    fn test_adaptive_cycles_converging_at_max() {
        let cycles = adaptive_cycles(6, 10, 5);
        assert_eq!(cycles, 6);
    }

    #[test]
    fn test_adaptive_cycles_diverging() {
        let cycles = adaptive_cycles(3, 5, 10);
        assert_eq!(cycles, 3);
    }

    #[test]
    fn test_adaptive_cycles_flat() {
        let cycles = adaptive_cycles(3, 10, 10);
        assert_eq!(cycles, 3);
    }

    #[test]
    fn test_adaptive_cycles_from_zero() {
        let cycles = adaptive_cycles(3, 0, 0);
        assert_eq!(cycles, 3);
    }
}
