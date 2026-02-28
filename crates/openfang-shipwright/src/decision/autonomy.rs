//! Autonomy tier resolution and enforcement.
//!
//! Determines whether a candidate should be automatically executed, proposed for review,
//! or drafted only. Enforces budget, rate, and halt limits.

use super::signals::Category;
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

/// Autonomy tier for a decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AutonomyTier {
    /// Create issue and spawn pipeline immediately.
    Auto,
    /// Create issue, wait for human approval before spawning.
    Propose,
    /// Write to drafts only, no issue created.
    Draft,
}

impl std::fmt::Display for AutonomyTier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                AutonomyTier::Auto => "auto",
                AutonomyTier::Propose => "propose",
                AutonomyTier::Draft => "draft",
            }
        )
    }
}

/// Tier resolution rules.
pub fn resolve_tier(category: Category, score: f64) -> AutonomyTier {
    match category {
        // Security patches are always auto
        Category::SecurityPatch => AutonomyTier::Auto,
        // High-scoring items are auto, medium are proposed, low are drafted
        Category::BugFix | Category::DependencyUpdate => {
            if score > 0.75 {
                AutonomyTier::Auto
            } else if score > 0.5 {
                AutonomyTier::Propose
            } else {
                AutonomyTier::Draft
            }
        }
        // Refactoring of hotspots is proposed
        Category::Refactoring => AutonomyTier::Propose,
        // Performance improvements are proposed
        Category::Performance => AutonomyTier::Propose,
        // New features are drafted
        Category::Feature => AutonomyTier::Draft,
    }
}

/// Decision limits and constraints.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionLimits {
    pub max_issues_per_day: u32,
    pub max_cost_per_day_usd: f64,
    pub cooldown_seconds: u64,
    pub halt_after_failures: u32,
}

impl Default for DecisionLimits {
    fn default() -> Self {
        Self {
            max_issues_per_day: 15,
            max_cost_per_day_usd: 25.0,
            cooldown_seconds: 300,
            halt_after_failures: 3,
        }
    }
}

/// Decision state tracker.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionState {
    pub issued_count_today: u32,
    pub cost_today_usd: f64,
    pub last_decision_time: Option<u64>,
    pub halt_flag: bool,
    pub failure_count: u32,
}

impl Default for DecisionState {
    fn default() -> Self {
        Self {
            issued_count_today: 0,
            cost_today_usd: 0.0,
            last_decision_time: None,
            halt_flag: false,
            failure_count: 0,
        }
    }
}

impl DecisionState {
    /// Check if budget allows issuing another item.
    pub fn check_budget(&self, limits: &DecisionLimits) -> bool {
        self.issued_count_today < limits.max_issues_per_day
            && self.cost_today_usd < limits.max_cost_per_day_usd
    }

    /// Check if rate limit allows another decision.
    pub fn check_rate_limit(&self, limits: &DecisionLimits) -> bool {
        if let Some(last_time) = self.last_decision_time {
            let now = now_timestamp();
            now.saturating_sub(last_time) >= limits.cooldown_seconds
        } else {
            true
        }
    }

    /// Check if halt flag is set.
    pub fn check_halt(&self) -> bool {
        self.halt_flag
    }

    /// Check if failure limit exceeded.
    pub fn check_halt_limit(&self, limits: &DecisionLimits) -> bool {
        self.failure_count >= limits.halt_after_failures
    }

    /// Record a decision.
    pub fn record_decision(&mut self, cost_usd: f64) {
        self.issued_count_today += 1;
        self.cost_today_usd += cost_usd;
        self.last_decision_time = Some(now_timestamp());
    }

    /// Record a failure.
    pub fn record_failure(&mut self) {
        self.failure_count += 1;
    }

    /// Set halt flag.
    pub fn halt(&mut self) {
        self.halt_flag = true;
    }

    /// Clear halt flag.
    pub fn resume(&mut self) {
        self.halt_flag = false;
    }

    /// Reset daily counters (call at midnight).
    pub fn reset_daily(&mut self) {
        self.issued_count_today = 0;
        self.cost_today_usd = 0.0;
        self.failure_count = 0;
    }
}

/// Get current Unix timestamp.
fn now_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_tier_security_patch() {
        let tier = resolve_tier(Category::SecurityPatch, 0.3);
        assert_eq!(tier, AutonomyTier::Auto);
    }

    #[test]
    fn test_resolve_tier_bug_fix_high_score() {
        let tier = resolve_tier(Category::BugFix, 0.8);
        assert_eq!(tier, AutonomyTier::Auto);
    }

    #[test]
    fn test_resolve_tier_bug_fix_medium_score() {
        let tier = resolve_tier(Category::BugFix, 0.6);
        assert_eq!(tier, AutonomyTier::Propose);
    }

    #[test]
    fn test_resolve_tier_bug_fix_low_score() {
        let tier = resolve_tier(Category::BugFix, 0.3);
        assert_eq!(tier, AutonomyTier::Draft);
    }

    #[test]
    fn test_resolve_tier_dependency_update() {
        let tier = resolve_tier(Category::DependencyUpdate, 0.6);
        assert_eq!(tier, AutonomyTier::Propose);
    }

    #[test]
    fn test_resolve_tier_refactoring() {
        let tier = resolve_tier(Category::Refactoring, 0.9);
        assert_eq!(tier, AutonomyTier::Propose);
    }

    #[test]
    fn test_resolve_tier_performance() {
        let tier = resolve_tier(Category::Performance, 0.9);
        assert_eq!(tier, AutonomyTier::Propose);
    }

    #[test]
    fn test_resolve_tier_feature() {
        let tier = resolve_tier(Category::Feature, 0.9);
        assert_eq!(tier, AutonomyTier::Draft);
    }

    #[test]
    fn test_decision_state_default() {
        let state = DecisionState::default();
        assert_eq!(state.issued_count_today, 0);
        assert_eq!(state.cost_today_usd, 0.0);
        assert!(!state.halt_flag);
        assert_eq!(state.failure_count, 0);
    }

    #[test]
    fn test_check_budget_allows() {
        let state = DecisionState {
            issued_count_today: 5,
            cost_today_usd: 10.0,
            ..Default::default()
        };
        let limits = DecisionLimits::default();
        assert!(state.check_budget(&limits));
    }

    #[test]
    fn test_check_budget_exceeds_issue_count() {
        let state = DecisionState {
            issued_count_today: 16,
            cost_today_usd: 10.0,
            ..Default::default()
        };
        let limits = DecisionLimits::default();
        assert!(!state.check_budget(&limits));
    }

    #[test]
    fn test_check_budget_exceeds_cost() {
        let state = DecisionState {
            issued_count_today: 5,
            cost_today_usd: 26.0,
            ..Default::default()
        };
        let limits = DecisionLimits::default();
        assert!(!state.check_budget(&limits));
    }

    #[test]
    fn test_check_rate_limit_allows() {
        let mut state = DecisionState::default();
        state.last_decision_time = Some(now_timestamp() - 500);
        let limits = DecisionLimits::default();
        assert!(state.check_rate_limit(&limits));
    }

    #[test]
    fn test_check_rate_limit_denies() {
        let mut state = DecisionState::default();
        state.last_decision_time = Some(now_timestamp() - 100);
        let limits = DecisionLimits::default();
        assert!(!state.check_rate_limit(&limits));
    }

    #[test]
    fn test_check_halt() {
        let mut state = DecisionState::default();
        assert!(!state.check_halt());
        state.halt();
        assert!(state.check_halt());
    }

    #[test]
    fn test_halt_and_resume() {
        let mut state = DecisionState::default();
        state.halt();
        assert!(state.halt_flag);
        state.resume();
        assert!(!state.halt_flag);
    }

    #[test]
    fn test_check_halt_limit() {
        let state = DecisionState {
            failure_count: 2,
            ..Default::default()
        };
        let limits = DecisionLimits::default();
        assert!(!state.check_halt_limit(&limits));

        let state = DecisionState {
            failure_count: 3,
            ..Default::default()
        };
        assert!(state.check_halt_limit(&limits));
    }

    #[test]
    fn test_record_decision() {
        let mut state = DecisionState::default();
        state.record_decision(5.0);
        assert_eq!(state.issued_count_today, 1);
        assert_eq!(state.cost_today_usd, 5.0);
        assert!(state.last_decision_time.is_some());
    }

    #[test]
    fn test_record_failure() {
        let mut state = DecisionState::default();
        state.record_failure();
        assert_eq!(state.failure_count, 1);
    }

    #[test]
    fn test_reset_daily() {
        let mut state = DecisionState {
            issued_count_today: 10,
            cost_today_usd: 15.0,
            failure_count: 2,
            ..Default::default()
        };
        state.reset_daily();
        assert_eq!(state.issued_count_today, 0);
        assert_eq!(state.cost_today_usd, 0.0);
        assert_eq!(state.failure_count, 0);
    }

    #[test]
    fn test_tier_display() {
        assert_eq!(AutonomyTier::Auto.to_string(), "auto");
        assert_eq!(AutonomyTier::Propose.to_string(), "propose");
        assert_eq!(AutonomyTier::Draft.to_string(), "draft");
    }
}
