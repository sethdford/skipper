//! Risk prediction and anomaly detection.

use crate::pipeline::ModelChoice;
use serde::{Deserialize, Serialize};

/// Risk prediction for a change.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskPrediction {
    pub score: u8,      // 0-100
    pub factors: Vec<RiskFactor>,
}

/// A risk factor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskFactor {
    pub name: String,
    pub impact: f64,
}

impl RiskPrediction {
    /// Create a new risk prediction.
    pub fn new(score: u8, factors: Vec<RiskFactor>) -> Self {
        Self {
            score: score.clamp(0, 100),
            factors,
        }
    }

    /// Create with no factors.
    pub fn simple(score: u8) -> Self {
        Self {
            score: score.clamp(0, 100),
            factors: vec![],
        }
    }
}

/// Predict risk based on various factors.
pub fn predict_risk(
    hotspot_change_count: u32,
    complexity_score: f64,
    test_coverage: f64,
) -> RiskPrediction {
    let mut score = 50u8;
    let mut factors = Vec::new();

    // Hotspot changes increase risk
    if hotspot_change_count > 0 {
        let hotspot_risk = (hotspot_change_count as f64).sqrt() * 10.0;
        score = ((score as f64) + hotspot_risk.min(30.0)) as u8;
        factors.push(RiskFactor {
            name: "hotspot_changes".to_string(),
            impact: hotspot_risk,
        });
    }

    // High complexity increases risk
    if complexity_score > 0.7 {
        let complexity_risk = (complexity_score - 0.7) * 100.0;
        score = ((score as f64) + complexity_risk) as u8;
        factors.push(RiskFactor {
            name: "high_complexity".to_string(),
            impact: complexity_risk,
        });
    }

    // Low test coverage increases risk
    if test_coverage < 0.7 {
        let coverage_risk = (1.0 - test_coverage) * 40.0;
        score = ((score as f64) + coverage_risk) as u8;
        factors.push(RiskFactor {
            name: "low_coverage".to_string(),
            impact: coverage_risk,
        });
    }

    RiskPrediction::new(score.min(100), factors)
}

/// Detect anomalies using z-score.
pub fn detect_anomaly(value: f64, mean: f64, stddev: f64, threshold: f64) -> bool {
    if stddev == 0.0 {
        return false;
    }
    let z_score = (value - mean).abs() / stddev;
    z_score > threshold
}

/// Recommend a model based on risk score.
pub fn recommend_model(risk_score: u8) -> ModelChoice {
    match risk_score {
        0..=30 => ModelChoice::Haiku,
        31..=70 => ModelChoice::Sonnet,
        _ => ModelChoice::Opus,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_risk_prediction_new() {
        let factors = vec![RiskFactor {
            name: "hotspot".to_string(),
            impact: 0.5,
        }];
        let prediction = RiskPrediction::new(75, factors.clone());
        assert_eq!(prediction.score, 75);
        assert_eq!(prediction.factors.len(), 1);
    }

    #[test]
    fn test_risk_prediction_clamped_high() {
        let prediction = RiskPrediction::new(150, vec![]);
        assert_eq!(prediction.score, 100);
    }

    #[test]
    fn test_risk_prediction_clamped_low() {
        let prediction = RiskPrediction::simple(10);
        assert_eq!(prediction.score, 10);
    }

    #[test]
    fn test_risk_prediction_simple() {
        let prediction = RiskPrediction::simple(50);
        assert_eq!(prediction.score, 50);
        assert!(prediction.factors.is_empty());
    }

    #[test]
    fn test_predict_risk_baseline() {
        let prediction = predict_risk(0, 0.0, 1.0);
        assert_eq!(prediction.score, 50);
    }

    #[test]
    fn test_predict_risk_with_hotspots() {
        let prediction = predict_risk(4, 0.0, 1.0);
        // baseline 50 + sqrt(4) * 10 = 50 + 20 = 70
        assert_eq!(prediction.score, 70);
        assert_eq!(prediction.factors.len(), 1);
    }

    #[test]
    fn test_predict_risk_high_complexity() {
        let prediction = predict_risk(0, 0.9, 1.0);
        // baseline 50 + (0.9 - 0.7) * 100 = 50 + 20 = 70
        assert_eq!(prediction.score, 70);
        assert_eq!(prediction.factors.len(), 1);
    }

    #[test]
    fn test_predict_risk_low_coverage() {
        let prediction = predict_risk(0, 0.0, 0.5);
        // baseline 50 + (1 - 0.5) * 40 = 50 + 20 = 70
        assert_eq!(prediction.score, 70);
        assert_eq!(prediction.factors.len(), 1);
    }

    #[test]
    fn test_predict_risk_all_factors() {
        let prediction = predict_risk(4, 0.9, 0.5);
        // baseline 50 + 20 + 20 + 20 = 110 → clamped to 100
        assert_eq!(prediction.score, 100);
        assert_eq!(prediction.factors.len(), 3);
    }

    #[test]
    fn test_detect_anomaly_is_anomaly() {
        let is_anomaly = detect_anomaly(10.0, 5.0, 1.0, 3.0);
        // z_score = |10 - 5| / 1 = 5 > 3 → true
        assert!(is_anomaly);
    }

    #[test]
    fn test_detect_anomaly_not_anomaly() {
        let is_anomaly = detect_anomaly(5.5, 5.0, 1.0, 3.0);
        // z_score = |5.5 - 5| / 1 = 0.5 < 3 → false
        assert!(!is_anomaly);
    }

    #[test]
    fn test_detect_anomaly_zero_stddev() {
        let is_anomaly = detect_anomaly(10.0, 5.0, 0.0, 3.0);
        assert!(!is_anomaly);
    }

    #[test]
    fn test_recommend_model_haiku() {
        assert_eq!(recommend_model(20), ModelChoice::Haiku);
        assert_eq!(recommend_model(30), ModelChoice::Haiku);
    }

    #[test]
    fn test_recommend_model_sonnet() {
        assert_eq!(recommend_model(31), ModelChoice::Sonnet);
        assert_eq!(recommend_model(50), ModelChoice::Sonnet);
        assert_eq!(recommend_model(70), ModelChoice::Sonnet);
    }

    #[test]
    fn test_recommend_model_opus() {
        assert_eq!(recommend_model(71), ModelChoice::Opus);
        assert_eq!(recommend_model(100), ModelChoice::Opus);
    }

    #[test]
    fn test_model_choice_display() {
        assert_eq!(ModelChoice::Haiku.to_string(), "haiku");
        assert_eq!(ModelChoice::Sonnet.to_string(), "sonnet");
        assert_eq!(ModelChoice::Opus.to_string(), "opus");
    }
}
