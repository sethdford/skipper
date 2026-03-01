//! Pipeline templates for different execution strategies.

use super::stages::{Gate, ModelChoice, Stage, StageConfig};
use serde::{Deserialize, Serialize};

/// Pipeline template definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineTemplate {
    pub name: String,
    pub stages: Vec<StageConfig>,
}

impl PipelineTemplate {
    /// Fast template: skip review, focus on speed.
    pub fn fast() -> Self {
        Self {
            name: "fast".to_string(),
            stages: vec![
                StageConfig {
                    stage: Stage::Intake,
                    enabled: true,
                    gate: Gate::Auto,
                    max_iterations: 1,
                    model: ModelChoice::Haiku,
                    timeout_seconds: 300,
                },
                StageConfig {
                    stage: Stage::Build,
                    enabled: true,
                    gate: Gate::Auto,
                    max_iterations: 3,
                    model: ModelChoice::Sonnet,
                    timeout_seconds: 1800,
                },
                StageConfig {
                    stage: Stage::Test,
                    enabled: true,
                    gate: Gate::Auto,
                    max_iterations: 2,
                    model: ModelChoice::Sonnet,
                    timeout_seconds: 1200,
                },
                StageConfig {
                    stage: Stage::Pr,
                    enabled: true,
                    gate: Gate::Auto,
                    max_iterations: 1,
                    model: ModelChoice::Haiku,
                    timeout_seconds: 600,
                },
            ],
        }
    }

    /// Standard template: balanced approach with review.
    pub fn standard() -> Self {
        Self {
            name: "standard".to_string(),
            stages: vec![
                StageConfig {
                    stage: Stage::Intake,
                    enabled: true,
                    gate: Gate::Auto,
                    max_iterations: 1,
                    model: ModelChoice::Haiku,
                    timeout_seconds: 300,
                },
                StageConfig {
                    stage: Stage::Plan,
                    enabled: true,
                    gate: Gate::Approve,
                    max_iterations: 1,
                    model: ModelChoice::Sonnet,
                    timeout_seconds: 600,
                },
                StageConfig {
                    stage: Stage::Design,
                    enabled: true,
                    gate: Gate::Auto,
                    max_iterations: 2,
                    model: ModelChoice::Sonnet,
                    timeout_seconds: 1200,
                },
                StageConfig {
                    stage: Stage::Build,
                    enabled: true,
                    gate: Gate::Auto,
                    max_iterations: 5,
                    model: ModelChoice::Sonnet,
                    timeout_seconds: 1800,
                },
                StageConfig {
                    stage: Stage::Test,
                    enabled: true,
                    gate: Gate::Auto,
                    max_iterations: 2,
                    model: ModelChoice::Sonnet,
                    timeout_seconds: 1200,
                },
                StageConfig {
                    stage: Stage::Review,
                    enabled: true,
                    gate: Gate::Approve,
                    max_iterations: 1,
                    model: ModelChoice::Opus,
                    timeout_seconds: 900,
                },
                StageConfig {
                    stage: Stage::Pr,
                    enabled: true,
                    gate: Gate::Auto,
                    max_iterations: 1,
                    model: ModelChoice::Haiku,
                    timeout_seconds: 600,
                },
            ],
        }
    }

    /// Full template: all 12 stages with maximum rigor.
    pub fn full() -> Self {
        Self {
            name: "full".to_string(),
            stages: Stage::all()
                .iter()
                .map(|stage| StageConfig {
                    stage: *stage,
                    enabled: true,
                    gate: match stage {
                        Stage::Plan | Stage::Review | Stage::Pr | Stage::Deploy => Gate::Approve,
                        _ => Gate::Auto,
                    },
                    max_iterations: match stage {
                        Stage::Build => 6,
                        Stage::Test => 3,
                        _ => 2,
                    },
                    model: match stage {
                        Stage::Review | Stage::CompoundQuality => ModelChoice::Opus,
                        Stage::Intake => ModelChoice::Haiku,
                        _ => ModelChoice::Sonnet,
                    },
                    timeout_seconds: match stage {
                        Stage::Build => 3600,
                        Stage::Monitor => 600,
                        _ => 1800,
                    },
                })
                .collect(),
        }
    }

    /// Hotfix template: rapid deployment with minimal gates.
    pub fn hotfix() -> Self {
        Self {
            name: "hotfix".to_string(),
            stages: vec![
                StageConfig {
                    stage: Stage::Intake,
                    enabled: true,
                    gate: Gate::Auto,
                    max_iterations: 1,
                    model: ModelChoice::Haiku,
                    timeout_seconds: 300,
                },
                StageConfig {
                    stage: Stage::Build,
                    enabled: true,
                    gate: Gate::Auto,
                    max_iterations: 2,
                    model: ModelChoice::Sonnet,
                    timeout_seconds: 1200,
                },
                StageConfig {
                    stage: Stage::Test,
                    enabled: true,
                    gate: Gate::Auto,
                    max_iterations: 1,
                    model: ModelChoice::Sonnet,
                    timeout_seconds: 900,
                },
                StageConfig {
                    stage: Stage::Pr,
                    enabled: true,
                    gate: Gate::Auto,
                    max_iterations: 1,
                    model: ModelChoice::Haiku,
                    timeout_seconds: 600,
                },
            ],
        }
    }

    /// Autonomous template: all stages, all auto gates.
    pub fn autonomous() -> Self {
        Self {
            name: "autonomous".to_string(),
            stages: Stage::all()
                .iter()
                .map(|stage| StageConfig {
                    stage: *stage,
                    enabled: true,
                    gate: Gate::Auto,
                    max_iterations: match stage {
                        Stage::Build => 6,
                        Stage::Test => 3,
                        _ => 2,
                    },
                    model: match stage {
                        Stage::Review | Stage::CompoundQuality => ModelChoice::Opus,
                        Stage::Intake => ModelChoice::Haiku,
                        _ => ModelChoice::Sonnet,
                    },
                    timeout_seconds: match stage {
                        Stage::Build => 3600,
                        _ => 1800,
                    },
                })
                .collect(),
        }
    }

    /// Cost-aware template: routes models by complexity.
    pub fn cost_aware() -> Self {
        Self {
            name: "cost-aware".to_string(),
            stages: vec![
                StageConfig {
                    stage: Stage::Intake,
                    enabled: true,
                    gate: Gate::Auto,
                    max_iterations: 1,
                    model: ModelChoice::Haiku,
                    timeout_seconds: 300,
                },
                StageConfig {
                    stage: Stage::Plan,
                    enabled: true,
                    gate: Gate::Auto,
                    max_iterations: 1,
                    model: ModelChoice::Sonnet,
                    timeout_seconds: 600,
                },
                StageConfig {
                    stage: Stage::Design,
                    enabled: true,
                    gate: Gate::Auto,
                    max_iterations: 2,
                    model: ModelChoice::Opus,
                    timeout_seconds: 1200,
                },
                StageConfig {
                    stage: Stage::Build,
                    enabled: true,
                    gate: Gate::Auto,
                    max_iterations: 4,
                    model: ModelChoice::Sonnet,
                    timeout_seconds: 1800,
                },
                StageConfig {
                    stage: Stage::Test,
                    enabled: true,
                    gate: Gate::Auto,
                    max_iterations: 2,
                    model: ModelChoice::Sonnet,
                    timeout_seconds: 1200,
                },
                StageConfig {
                    stage: Stage::Review,
                    enabled: true,
                    gate: Gate::Auto,
                    max_iterations: 1,
                    model: ModelChoice::Opus,
                    timeout_seconds: 900,
                },
                StageConfig {
                    stage: Stage::Pr,
                    enabled: true,
                    gate: Gate::Auto,
                    max_iterations: 1,
                    model: ModelChoice::Haiku,
                    timeout_seconds: 600,
                },
            ],
        }
    }

    /// Count enabled stages.
    pub fn enabled_stage_count(&self) -> usize {
        self.stages.iter().filter(|s| s.enabled).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fast_template() {
        let template = PipelineTemplate::fast();
        assert_eq!(template.name, "fast");
        assert_eq!(template.enabled_stage_count(), 4);
        assert!(template.stages[0].enabled);
    }

    #[test]
    fn test_standard_template() {
        let template = PipelineTemplate::standard();
        assert_eq!(template.name, "standard");
        assert!(template.enabled_stage_count() > 4);
    }

    #[test]
    fn test_full_template() {
        let template = PipelineTemplate::full();
        assert_eq!(template.name, "full");
        assert_eq!(template.enabled_stage_count(), 12);
    }

    #[test]
    fn test_hotfix_template() {
        let template = PipelineTemplate::hotfix();
        assert_eq!(template.name, "hotfix");
        assert_eq!(template.enabled_stage_count(), 4);
        // All gates should be Auto for hotfix
        assert!(template.stages.iter().all(|s| s.gate == Gate::Auto));
    }

    #[test]
    fn test_autonomous_template() {
        let template = PipelineTemplate::autonomous();
        assert_eq!(template.name, "autonomous");
        assert_eq!(template.enabled_stage_count(), 12);
        assert!(template.stages.iter().all(|s| s.gate == Gate::Auto));
    }

    #[test]
    fn test_cost_aware_template() {
        let template = PipelineTemplate::cost_aware();
        assert_eq!(template.name, "cost-aware");
        // Verify that complex stages use Opus
        let design_stage = template.stages.iter().find(|s| s.stage == Stage::Design);
        assert!(design_stage.is_some());
        assert_eq!(design_stage.unwrap().model, ModelChoice::Opus);
    }

    #[test]
    fn test_template_serialization() {
        let template = PipelineTemplate::fast();
        let json = serde_json::to_string(&template).unwrap();
        assert!(json.contains("\"name\":\"fast\""));
    }
}
