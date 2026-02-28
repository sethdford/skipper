//! Dynamic pipeline composition based on intelligence and complexity.

use super::stages::{ModelChoice, Stage};
use super::templates::PipelineTemplate;
use serde::{Deserialize, Serialize};

/// Intelligence output that can affect pipeline composition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntelligenceInput {
    pub complexity_score: u8,
    pub has_risky_areas: bool,
    pub test_intensity_needed: bool,
    pub estimated_effort_hours: f64,
}

/// Dynamic pipeline composer.
pub struct PipelineComposer;

impl PipelineComposer {
    /// Compose a custom pipeline based on complexity and intelligence.
    pub fn compose(
        base_template: PipelineTemplate,
        intelligence: Option<IntelligenceInput>,
    ) -> PipelineTemplate {
        let mut template = base_template;

        if let Some(intel) = intelligence {
            // Adjust iterations based on complexity
            for stage in &mut template.stages {
                // Higher complexity → more iterations
                if intel.complexity_score > 70 {
                    stage.max_iterations = (stage.max_iterations as f64 * 1.5) as u32;
                } else if intel.complexity_score < 30 {
                    stage.max_iterations = stage.max_iterations.max(1);
                }

                // Route models: complex stages use Opus for high complexity
                if intel.complexity_score > 80 {
                    match stage.stage {
                        Stage::Design | Stage::Review | Stage::CompoundQuality => {
                            stage.model = ModelChoice::Opus;
                        }
                        _ => {}
                    }
                }

                // Test intensity
                if intel.test_intensity_needed && stage.stage == Stage::Test {
                    stage.max_iterations = (stage.max_iterations as f64 * 2.0) as u32;
                }
            }

            // Add more time for risky areas
            if intel.has_risky_areas {
                for stage in &mut template.stages {
                    if stage.stage == Stage::Review || stage.stage == Stage::Design {
                        stage.timeout_seconds = (stage.timeout_seconds as f64 * 1.2) as u64;
                    }
                }
            }
        }

        template
    }

    /// Score complexity (0-100) based on various factors.
    pub fn score_complexity(
        files_changed: usize,
        test_coverage: f64,
        dependencies_changed: bool,
    ) -> u8 {
        let mut score = 30; // Base score

        // File changes impact
        if files_changed > 20 {
            score += 30;
        } else if files_changed > 10 {
            score += 15;
        } else if files_changed > 5 {
            score += 5;
        }

        // Test coverage impact
        if test_coverage < 0.5 {
            score += 20;
        } else if test_coverage < 0.7 {
            score += 10;
        }

        // Dependencies impact
        if dependencies_changed {
            score += 15;
        }

        (score as u8).min(100)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_score_complexity_low() {
        let score = PipelineComposer::score_complexity(2, 0.9, false);
        assert!(score < 50);
    }

    #[test]
    fn test_score_complexity_high() {
        let score = PipelineComposer::score_complexity(25, 0.4, true);
        assert!(score > 60);
    }

    #[test]
    fn test_compose_with_high_complexity() {
        let base = PipelineTemplate::standard();
        let intel = IntelligenceInput {
            complexity_score: 85,
            has_risky_areas: true,
            test_intensity_needed: true,
            estimated_effort_hours: 4.0,
        };

        let composed = PipelineComposer::compose(base, Some(intel));

        // Check that iterations increased
        let build_stage = composed
            .stages
            .iter()
            .find(|s| s.stage == Stage::Build)
            .unwrap();
        assert!(build_stage.max_iterations > 5);

        // Check model routing for complex stages
        let design = composed
            .stages
            .iter()
            .find(|s| s.stage == Stage::Design);
        if let Some(stage) = design {
            assert_eq!(stage.model, ModelChoice::Opus);
        }
    }

    #[test]
    fn test_compose_with_no_intelligence() {
        let base = PipelineTemplate::fast();
        let composed = PipelineComposer::compose(base.clone(), None);
        // Should be identical if no intelligence
        assert_eq!(composed.stages.len(), base.stages.len());
    }

    #[test]
    fn test_compose_test_intensity() {
        let base = PipelineTemplate::standard();
        let original_test_iterations = base
            .stages
            .iter()
            .find(|s| s.stage == Stage::Test)
            .map(|s| s.max_iterations)
            .unwrap_or(0);

        let intel = IntelligenceInput {
            complexity_score: 50,
            has_risky_areas: false,
            test_intensity_needed: true,
            estimated_effort_hours: 2.0,
        };

        let composed = PipelineComposer::compose(base, Some(intel));
        let new_test_iterations = composed
            .stages
            .iter()
            .find(|s| s.stage == Stage::Test)
            .map(|s| s.max_iterations)
            .unwrap_or(0);

        assert!(new_test_iterations > original_test_iterations);
    }
}
