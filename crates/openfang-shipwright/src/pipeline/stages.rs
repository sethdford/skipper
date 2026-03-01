//! Pipeline stage definitions and state machine.

use serde::{Deserialize, Serialize};

/// A pipeline stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    Intake,
    Plan,
    Design,
    Build,
    Test,
    Review,
    CompoundQuality,
    Pr,
    Merge,
    Deploy,
    Validate,
    Monitor,
}

/// All stages in order (static array to avoid allocations).
const ALL_STAGES: &[Stage] = &[
    Stage::Intake,
    Stage::Plan,
    Stage::Design,
    Stage::Build,
    Stage::Test,
    Stage::Review,
    Stage::CompoundQuality,
    Stage::Pr,
    Stage::Merge,
    Stage::Deploy,
    Stage::Validate,
    Stage::Monitor,
];

impl Stage {
    /// Get all stages in order.
    /// Returns a static slice instead of allocating a new Vec each time.
    pub fn all() -> &'static [Stage] {
        ALL_STAGES
    }

    /// Get the next stage after this one.
    pub fn next(&self) -> Option<Stage> {
        let stages = Self::all();
        let index = stages.iter().position(|s| s == self)?;
        stages.get(index + 1).copied()
    }
}

/// Gate type for stage approval.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Gate {
    Auto,
    Approve,
}

/// Model choice for stage execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelChoice {
    Haiku,
    Sonnet,
    Opus,
}

impl std::fmt::Display for ModelChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                ModelChoice::Haiku => "haiku",
                ModelChoice::Sonnet => "sonnet",
                ModelChoice::Opus => "opus",
            }
        )
    }
}

/// Configuration for a single stage.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StageConfig {
    pub stage: Stage,
    pub enabled: bool,
    pub gate: Gate,
    pub max_iterations: u32,
    pub model: ModelChoice,
    pub timeout_seconds: u64,
}

impl Default for StageConfig {
    fn default() -> Self {
        Self {
            stage: Stage::Build,
            enabled: true,
            gate: Gate::Auto,
            max_iterations: 5,
            model: ModelChoice::Sonnet,
            timeout_seconds: 3600,
        }
    }
}

/// Pipeline state machine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PipelineState {
    Pending,
    Running {
        current_stage: Stage,
        iteration: u32,
    },
    Paused {
        at_stage: Stage,
        iteration: u32,
        reason: String,
    },
    Completed {
        pr_url: Option<String>,
    },
    Failed {
        at_stage: Stage,
        error: String,
        retries: u32,
    },
}

impl PipelineState {
    /// Transition to the next stage using global stage ordering.
    /// WARNING: This uses Stage::all() which includes all 12 stages.
    /// For template-aware advancement, use advance_with_template() instead.
    #[deprecated(
        since = "0.2.2",
        note = "Use advance_with_stages() instead to respect template stages"
    )]
    pub fn advance(&self) -> Result<PipelineState, String> {
        match self {
            PipelineState::Running {
                current_stage,
                ..
            } => {
                if let Some(next) = current_stage.next() {
                    Ok(PipelineState::Running {
                        current_stage: next,
                        iteration: 0,
                    })
                } else {
                    Ok(PipelineState::Completed { pr_url: None })
                }
            }
            _ => Err("Cannot advance from this state".to_string()),
        }
    }

    /// Transition to the next stage with a list of enabled stages.
    /// This respects template-specific stage ordering.
    pub fn advance_with_stages(&self, enabled_stages: &[Stage]) -> Result<PipelineState, String> {
        match self {
            PipelineState::Running {
                current_stage,
                ..
            } => {
                let index = enabled_stages
                    .iter()
                    .position(|s| s == current_stage)
                    .ok_or_else(|| format!("Current stage {:?} not in template", current_stage))?;

                if let Some(next_stage) = enabled_stages.get(index + 1) {
                    Ok(PipelineState::Running {
                        current_stage: *next_stage,
                        iteration: 0,
                    })
                } else {
                    Ok(PipelineState::Completed { pr_url: None })
                }
            }
            _ => Err("Cannot advance from this state".to_string()),
        }
    }

    /// Move to the next iteration of the current stage.
    pub fn next_iteration(&self) -> Result<PipelineState, String> {
        match self {
            PipelineState::Running {
                current_stage,
                iteration: iter,
            } => Ok(PipelineState::Running {
                current_stage: *current_stage,
                iteration: iter + 1,
            }),
            _ => Err("Cannot iterate from this state".to_string()),
        }
    }

    /// Fail the pipeline at the current stage.
    pub fn fail(&self, error: String) -> Result<PipelineState, String> {
        match self {
            PipelineState::Running { current_stage, .. } => Ok(PipelineState::Failed {
                at_stage: *current_stage,
                error,
                retries: 0,
            }),
            PipelineState::Failed {
                at_stage,
                error: _,
                retries,
            } => Ok(PipelineState::Failed {
                at_stage: *at_stage,
                error,
                retries: retries + 1,
            }),
            _ => Err("Cannot fail from this state".to_string()),
        }
    }

    /// Pause the pipeline at the current stage.
    pub fn pause(&self, reason: String) -> Result<PipelineState, String> {
        match self {
            PipelineState::Running {
                current_stage,
                iteration,
            } => Ok(PipelineState::Paused {
                at_stage: *current_stage,
                iteration: *iteration,
                reason,
            }),
            _ => Err("Cannot pause from this state".to_string()),
        }
    }

    /// Resume from a paused state, preserving iteration count.
    pub fn resume(&self) -> Result<PipelineState, String> {
        match self {
            PipelineState::Paused {
                at_stage,
                iteration,
                ..
            } => Ok(PipelineState::Running {
                current_stage: *at_stage,
                iteration: *iteration,
            }),
            _ => Err("Cannot resume from this state".to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stage_all() {
        let stages = Stage::all();
        assert_eq!(stages.len(), 12);
        assert_eq!(stages[0], Stage::Intake);
        assert_eq!(stages[11], Stage::Monitor);
    }

    #[test]
    fn test_low002_stage_all_returns_static_slice() {
        // LOW-002: Stage::all() should return &'static [Stage] to avoid allocations
        let stages1 = Stage::all();
        let stages2 = Stage::all();

        // Both should point to the same memory location (static slice)
        assert_eq!(stages1.as_ptr(), stages2.as_ptr(), "Stage::all() should return the same static slice");

        // Verify it's a slice, not a Vec
        assert_eq!(stages1.len(), 12);
    }

    #[test]
    fn test_stage_next() {
        assert_eq!(Stage::Intake.next(), Some(Stage::Plan));
        assert_eq!(Stage::Monitor.next(), None);
    }

    #[test]
    fn test_stage_config_default() {
        let config = StageConfig::default();
        assert_eq!(config.stage, Stage::Build);
        assert!(config.enabled);
        assert_eq!(config.gate, Gate::Auto);
    }

    #[test]
    #[allow(deprecated)]
    fn test_pipeline_state_advance() {
        let state = PipelineState::Running {
            current_stage: Stage::Build,
            iteration: 1,
        };
        let next = state.advance().unwrap();
        match next {
            PipelineState::Running {
                current_stage,
                iteration,
            } => {
                assert_eq!(current_stage, Stage::Test);
                assert_eq!(iteration, 0);
            }
            _ => panic!("Expected Running state"),
        }
    }

    #[test]
    #[allow(deprecated)]
    fn test_pipeline_state_advance_to_completed() {
        let state = PipelineState::Running {
            current_stage: Stage::Monitor,
            iteration: 1,
        };
        let next = state.advance().unwrap();
        assert!(matches!(next, PipelineState::Completed { .. }));
    }

    #[test]
    fn test_pipeline_state_next_iteration() {
        let state = PipelineState::Running {
            current_stage: Stage::Build,
            iteration: 1,
        };
        let next = state.next_iteration().unwrap();
        match next {
            PipelineState::Running {
                current_stage,
                iteration,
            } => {
                assert_eq!(current_stage, Stage::Build);
                assert_eq!(iteration, 2);
            }
            _ => panic!("Expected Running state"),
        }
    }

    #[test]
    fn test_pipeline_state_fail() {
        let state = PipelineState::Running {
            current_stage: Stage::Build,
            iteration: 1,
        };
        let failed = state.fail("Compilation error".to_string()).unwrap();
        match failed {
            PipelineState::Failed {
                at_stage,
                error,
                retries,
            } => {
                assert_eq!(at_stage, Stage::Build);
                assert_eq!(error, "Compilation error");
                assert_eq!(retries, 0);
            }
            _ => panic!("Expected Failed state"),
        }
    }

    #[test]
    fn test_pipeline_state_pause() {
        let state = PipelineState::Running {
            current_stage: Stage::Review,
            iteration: 1,
        };
        let paused = state.pause("Waiting for approval".to_string()).unwrap();
        match paused {
            PipelineState::Paused {
                at_stage,
                iteration,
                reason,
            } => {
                assert_eq!(at_stage, Stage::Review);
                assert_eq!(iteration, 1);
                assert_eq!(reason, "Waiting for approval");
            }
            _ => panic!("Expected Paused state"),
        }
    }

    #[test]
    fn test_pipeline_state_resume() {
        let state = PipelineState::Paused {
            at_stage: Stage::Review,
            iteration: 0,
            reason: "Waiting for approval".to_string(),
        };
        let running = state.resume().unwrap();
        match running {
            PipelineState::Running {
                current_stage,
                iteration,
            } => {
                assert_eq!(current_stage, Stage::Review);
                assert_eq!(iteration, 0);
            }
            _ => panic!("Expected Running state"),
        }
    }

    #[test]
    fn test_pipeline_state_pause_preserves_iteration() {
        // H9: Paused state should preserve iteration count
        let state = PipelineState::Running {
            current_stage: Stage::Build,
            iteration: 5,
        };
        let paused = state.pause("Waiting for approval".to_string()).unwrap();
        match paused {
            PipelineState::Paused {
                at_stage,
                iteration,
                reason,
            } => {
                assert_eq!(at_stage, Stage::Build);
                assert_eq!(iteration, 5); // Must preserve iteration
                assert_eq!(reason, "Waiting for approval");
            }
            _ => panic!("Expected Paused state"),
        }
    }

    #[test]
    fn test_pipeline_state_resume_restores_iteration() {
        // H9: Resume must restore the iteration count that was preserved during pause
        let state = PipelineState::Paused {
            at_stage: Stage::Build,
            iteration: 5,
            reason: "Waiting for approval".to_string(),
        };
        let running = state.resume().unwrap();
        match running {
            PipelineState::Running {
                current_stage,
                iteration,
            } => {
                assert_eq!(current_stage, Stage::Build);
                assert_eq!(iteration, 5); // Must be restored from paused state
            }
            _ => panic!("Expected Running state"),
        }
    }

    #[test]
    #[allow(deprecated)]
    fn test_invalid_state_transition() {
        let state = PipelineState::Completed { pr_url: None };
        assert!(state.advance().is_err());
    }

    #[test]
    fn test_advance_with_stages_fast_template() {
        // Fast template: Intake -> Build -> Test -> Pr
        let fast_stages = vec![Stage::Intake, Stage::Build, Stage::Test, Stage::Pr];

        let state = PipelineState::Running {
            current_stage: Stage::Intake,
            iteration: 0,
        };
        let next = state.advance_with_stages(&fast_stages).unwrap();
        match next {
            PipelineState::Running {
                current_stage,
                iteration,
            } => {
                assert_eq!(current_stage, Stage::Build);
                assert_eq!(iteration, 0);
            }
            _ => panic!("Expected Running state"),
        }
    }

    #[test]
    fn test_advance_with_stages_respects_template() {
        // Fast template only has 4 stages, not all 12
        let fast_stages = vec![Stage::Intake, Stage::Build, Stage::Test, Stage::Pr];

        // In global ordering, Build -> Test -> Review, but fast template skips Review
        let state = PipelineState::Running {
            current_stage: Stage::Build,
            iteration: 1,
        };
        let next = state.advance_with_stages(&fast_stages).unwrap();
        match next {
            PipelineState::Running {
                current_stage,
                iteration,
            } => {
                // Should advance to Test, not Review or Plan
                assert_eq!(current_stage, Stage::Test);
                assert_eq!(iteration, 0);
            }
            _ => panic!("Expected Running state"),
        }
    }

    #[test]
    fn test_advance_with_stages_reaches_end() {
        let fast_stages = vec![Stage::Intake, Stage::Build, Stage::Test, Stage::Pr];

        let state = PipelineState::Running {
            current_stage: Stage::Pr,
            iteration: 0,
        };
        let next = state.advance_with_stages(&fast_stages).unwrap();
        assert!(matches!(next, PipelineState::Completed { .. }));
    }

    #[test]
    fn test_advance_with_stages_invalid_stage() {
        let fast_stages = vec![Stage::Intake, Stage::Build, Stage::Test, Stage::Pr];

        // Current stage not in template
        let state = PipelineState::Running {
            current_stage: Stage::Plan,
            iteration: 0,
        };
        assert!(state.advance_with_stages(&fast_stages).is_err());
    }

    #[test]
    fn test_advance_with_stages_full_template() {
        let full_stages = Stage::all();

        let state = PipelineState::Running {
            current_stage: Stage::Review,
            iteration: 0,
        };
        let next = state.advance_with_stages(full_stages).unwrap();
        match next {
            PipelineState::Running {
                current_stage,
                iteration,
            } => {
                assert_eq!(current_stage, Stage::CompoundQuality);
                assert_eq!(iteration, 0);
            }
            _ => panic!("Expected Running state"),
        }
    }
}
