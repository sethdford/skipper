//! DORA metrics calculation and level classification.
//!
//! DORA (DevOps Research and Assessment) metrics:
//! - Lead Time: Time from first commit to production deployment
//! - Deploy Frequency: Number of deployments per day
//! - Change Failure Rate: Ratio of failed deployments to total deployments
//! - MTTR: Mean time to recovery from production incident

use serde::{Deserialize, Serialize};

/// DORA metrics for a repository.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DoraMetrics {
    pub lead_time_hours: f64,
    pub deploy_frequency_per_day: f64,
    pub change_failure_rate: f64,
    pub mttr_hours: f64,
}

impl DoraMetrics {
    /// Create new DORA metrics.
    pub fn new(
        lead_time_hours: f64,
        deploy_frequency_per_day: f64,
        change_failure_rate: f64,
        mttr_hours: f64,
    ) -> Self {
        Self {
            lead_time_hours: lead_time_hours.max(0.0),
            deploy_frequency_per_day: deploy_frequency_per_day.max(0.0),
            change_failure_rate: change_failure_rate.clamp(0.0, 1.0),
            mttr_hours: mttr_hours.max(0.0),
        }
    }
}

/// DORA performance level classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DoraLevel {
    Elite,
    High,
    Medium,
    Low,
}

impl std::fmt::Display for DoraLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                DoraLevel::Elite => "elite",
                DoraLevel::High => "high",
                DoraLevel::Medium => "medium",
                DoraLevel::Low => "low",
            }
        )
    }
}

/// Classify DORA metrics into a performance level.
pub fn classify_dora_level(metrics: &DoraMetrics) -> DoraLevel {
    let lead_time_score = if metrics.lead_time_hours <= 1.0 {
        4 // Elite: < 1 hour
    } else if metrics.lead_time_hours <= 1.0 * 24.0 {
        3 // High: 1-24 hours
    } else if metrics.lead_time_hours <= 7.0 * 24.0 {
        2 // Medium: 1-7 days
    } else {
        1 // Low: > 1 week
    };

    let frequency_score = if metrics.deploy_frequency_per_day >= 1.0 {
        4 // Elite: multiple per day
    } else if metrics.deploy_frequency_per_day >= 1.0 / 7.0 {
        3 // High: weekly
    } else if metrics.deploy_frequency_per_day >= 1.0 / 30.0 {
        2 // Medium: monthly
    } else {
        1 // Low: < monthly
    };

    let cfr_score = if metrics.change_failure_rate <= 0.15 {
        4 // Elite: < 15%
    } else if metrics.change_failure_rate <= 0.30 {
        3 // High: < 30%
    } else if metrics.change_failure_rate <= 0.50 {
        2 // Medium: < 50%
    } else {
        1 // Low: >= 50%
    };

    let mttr_score = if metrics.mttr_hours <= 1.0 {
        4 // Elite: < 1 hour
    } else if metrics.mttr_hours <= 24.0 {
        3 // High: < 1 day
    } else if metrics.mttr_hours <= 7.0 * 24.0 {
        2 // Medium: < 1 week
    } else {
        1 // Low: >= 1 week
    };

    let average_score = (lead_time_score + frequency_score + cfr_score + mttr_score) as f64 / 4.0;

    if average_score >= 3.5 {
        DoraLevel::Elite
    } else if average_score >= 2.5 {
        DoraLevel::High
    } else if average_score >= 1.5 {
        DoraLevel::Medium
    } else {
        DoraLevel::Low
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dora_metrics_new() {
        let metrics = DoraMetrics::new(2.5, 5.0, 0.1, 0.5);
        assert_eq!(metrics.lead_time_hours, 2.5);
        assert_eq!(metrics.deploy_frequency_per_day, 5.0);
        assert_eq!(metrics.change_failure_rate, 0.1);
        assert_eq!(metrics.mttr_hours, 0.5);
    }

    #[test]
    fn test_dora_metrics_clamp_clamping() {
        let metrics = DoraMetrics::new(2.5, 5.0, 1.5, 0.5);
        assert_eq!(metrics.change_failure_rate, 1.0);
    }

    #[test]
    fn test_dora_metrics_negative_clamp() {
        let metrics = DoraMetrics::new(-2.5, -5.0, -0.1, -0.5);
        assert_eq!(metrics.lead_time_hours, 0.0);
        assert_eq!(metrics.deploy_frequency_per_day, 0.0);
        assert_eq!(metrics.change_failure_rate, 0.0);
        assert_eq!(metrics.mttr_hours, 0.0);
    }

    #[test]
    fn test_classify_elite() {
        let metrics = DoraMetrics::new(0.5, 5.0, 0.1, 0.5);
        let level = classify_dora_level(&metrics);
        assert_eq!(level, DoraLevel::Elite);
    }

    #[test]
    fn test_classify_high() {
        let metrics = DoraMetrics::new(12.0, 1.0, 0.2, 12.0);
        let level = classify_dora_level(&metrics);
        assert_eq!(level, DoraLevel::High);
    }

    #[test]
    fn test_classify_medium() {
        let metrics = DoraMetrics::new(48.0, 1.0 / 7.0, 0.4, 72.0);
        let level = classify_dora_level(&metrics);
        assert_eq!(level, DoraLevel::Medium);
    }

    #[test]
    fn test_classify_low() {
        let metrics = DoraMetrics::new(200.0, 1.0 / 30.0, 0.6, 200.0);
        let level = classify_dora_level(&metrics);
        assert_eq!(level, DoraLevel::Low);
    }

    #[test]
    fn test_dora_level_display() {
        assert_eq!(DoraLevel::Elite.to_string(), "elite");
        assert_eq!(DoraLevel::High.to_string(), "high");
        assert_eq!(DoraLevel::Medium.to_string(), "medium");
        assert_eq!(DoraLevel::Low.to_string(), "low");
    }

    #[test]
    fn test_classify_elite_all_elite_metrics() {
        let metrics = DoraMetrics::new(0.5, 10.0, 0.1, 0.25);
        let level = classify_dora_level(&metrics);
        assert_eq!(level, DoraLevel::Elite);
    }

    #[test]
    fn test_classify_mixed_scores() {
        // 1 Elite, 1 High, 1 Medium, 1 Low = avg 2.5 = High
        let metrics = DoraMetrics::new(0.5, 1.0 / 7.0, 0.4, 200.0);
        let level = classify_dora_level(&metrics);
        assert_eq!(level, DoraLevel::High);
    }
}
