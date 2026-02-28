//! Pipeline engine: 12-stage delivery pipeline with templates and self-healing.

pub mod composer;
pub mod self_healing;
pub mod stages;
pub mod templates;

pub use composer::{IntelligenceInput, PipelineComposer};
pub use self_healing::{BuildLoop, BuildOutcome, ProgressState};
pub use stages::{Gate, ModelChoice, PipelineState, Stage, StageConfig};
pub use templates::PipelineTemplate;

use crate::config::PipelineTemplateName;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// A complete pipeline instance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pipeline {
    pub id: String,
    pub issue: Option<u64>,
    pub goal: String,
    pub template: PipelineTemplateName,
    pub stages: Vec<StageConfig>,
    pub(crate) state: PipelineState,
    pub artifacts_dir: PathBuf,
    pub created_at: String,
    pub updated_at: String,
}

impl Pipeline {
    /// Get a reference to the pipeline's current state.
    pub fn state(&self) -> &PipelineState {
        &self.state
    }

    /// Create a new pipeline from an issue.
    pub fn from_issue(issue_id: u64, goal: String, template: PipelineTemplate) -> Self {
        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();
        let artifacts_dir = PathBuf::from(format!("./.shipwright/pipelines/{}", id));

        // Extract template name and convert to PipelineTemplateName
        let template_name = Self::parse_template_name(&template.name);

        Self {
            id: id.clone(),
            issue: Some(issue_id),
            goal,
            template: template_name,
            stages: template.stages,
            state: PipelineState::Running {
                current_stage: Stage::Intake,
                iteration: 0,
            },
            artifacts_dir,
            created_at: now.clone(),
            updated_at: now,
        }
    }

    /// Create a new pipeline from a goal description.
    pub fn from_goal(goal: String, template: PipelineTemplate) -> Self {
        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();
        let artifacts_dir = PathBuf::from(format!("./.shipwright/pipelines/{}", id));

        // Extract template name and convert to PipelineTemplateName
        let template_name = Self::parse_template_name(&template.name);

        Self {
            id: id.clone(),
            issue: None,
            goal,
            template: template_name,
            stages: template.stages,
            state: PipelineState::Running {
                current_stage: Stage::Intake,
                iteration: 0,
            },
            artifacts_dir,
            created_at: now.clone(),
            updated_at: now,
        }
    }

    /// Parse a template name string to PipelineTemplateName.
    fn parse_template_name(name: &str) -> PipelineTemplateName {
        match name.to_lowercase().as_str() {
            "fast" => PipelineTemplateName::Fast,
            "standard" => PipelineTemplateName::Standard,
            "full" => PipelineTemplateName::Full,
            "hotfix" => PipelineTemplateName::Hotfix,
            "autonomous" => PipelineTemplateName::Autonomous,
            "cost-aware" | "costawaree" => PipelineTemplateName::CostAware,
            _ => PipelineTemplateName::Standard, // Default fallback
        }
    }

    /// Get the current stage configuration.
    pub fn current_stage_config(&self) -> Option<&StageConfig> {
        match &self.state {
            PipelineState::Running { current_stage, .. } => {
                self.stages.iter().find(|s| s.stage == *current_stage)
            }
            _ => None,
        }
    }

    /// Advance to the next stage, respecting template-enabled stages.
    pub fn advance_stage(&mut self) -> Result<(), String> {
        let enabled_stages: Vec<Stage> = self
            .stages
            .iter()
            .filter(|sc| sc.enabled)
            .map(|sc| sc.stage)
            .collect();
        let new_state = self.state.advance_with_stages(&enabled_stages)?;
        self.state = new_state;
        self.updated_at = chrono::Utc::now().to_rfc3339();
        Ok(())
    }

    /// Move to the next iteration of current stage.
    pub fn next_iteration(&mut self) -> Result<(), String> {
        let new_state = self.state.next_iteration()?;
        self.state = new_state;
        self.updated_at = chrono::Utc::now().to_rfc3339();
        Ok(())
    }

    /// Fail the pipeline.
    pub fn fail(&mut self, error: String) -> Result<(), String> {
        let new_state = self.state.fail(error)?;
        self.state = new_state;
        self.updated_at = chrono::Utc::now().to_rfc3339();
        Ok(())
    }

    /// Pause the pipeline for approval.
    pub fn pause(&mut self, reason: String) -> Result<(), String> {
        let new_state = self.state.pause(reason)?;
        self.state = new_state;
        self.updated_at = chrono::Utc::now().to_rfc3339();
        Ok(())
    }

    /// Resume from pause.
    pub fn resume(&mut self) -> Result<(), String> {
        let new_state = self.state.resume()?;
        self.state = new_state;
        self.updated_at = chrono::Utc::now().to_rfc3339();
        Ok(())
    }

    /// Check if pipeline is complete.
    pub fn is_complete(&self) -> bool {
        matches!(self.state, PipelineState::Completed { .. })
    }

    /// Check if pipeline has failed.
    pub fn is_failed(&self) -> bool {
        matches!(self.state, PipelineState::Failed { .. })
    }

    /// Run the pipeline through all enabled stages.
    ///
    /// This async method executes the pipeline sequentially through each enabled stage
    /// in the template. It advances through stages, tracks iteration count, and stops
    /// on failure or completion.
    ///
    /// # Returns
    /// - `Ok(())` if pipeline completes successfully
    /// - `Err(msg)` if pipeline fails or encounters an error
    pub async fn run(&mut self) -> Result<(), String> {
        loop {
            // Check if pipeline is already in terminal state
            if self.is_complete() {
                return Ok(());
            }

            if self.is_failed() {
                return Err(format!(
                    "Pipeline failed at stage: {:?}",
                    match &self.state {
                        PipelineState::Failed { at_stage, .. } => at_stage,
                        _ => &Stage::Intake,
                    }
                ));
            }

            // Advance to next stage
            if let Err(e) = self.advance_stage() {
                // If we can't advance (reached end), mark as complete
                if e.contains("no next stage") {
                    let new_state = PipelineState::Completed { pr_url: None };
                    self.state = new_state;
                    self.updated_at = chrono::Utc::now().to_rfc3339();
                    return Ok(());
                } else {
                    return Err(e);
                }
            }

            // Yield to allow async operations
            tokio::task::yield_now().await;
        }
    }

    /// Get progress percentage.
    /// For Failed and Paused states, uses the at_stage field to calculate actual progress
    /// instead of returning hardcoded values.
    pub fn progress_percent(&self) -> u8 {
        if self.is_complete() {
            return 100;
        }

        match &self.state {
            PipelineState::Running { current_stage, .. } => {
                let all_stages = Stage::all();
                let current_index = all_stages.iter().position(|s| s == current_stage).unwrap_or(0);
                ((current_index * 100) / all_stages.len()) as u8
            }
            PipelineState::Failed { at_stage, .. } => {
                // Calculate progress based on the stage where failure occurred
                let all_stages = Stage::all();
                let stage_index = all_stages.iter().position(|s| s == at_stage).unwrap_or(0);
                ((stage_index * 100) / all_stages.len()) as u8
            }
            PipelineState::Paused { at_stage, .. } => {
                // Calculate progress based on the stage where paused
                let all_stages = Stage::all();
                let stage_index = all_stages.iter().position(|s| s == at_stage).unwrap_or(0);
                ((stage_index * 100) / all_stages.len()) as u8
            }
            PipelineState::Pending => 0,
            PipelineState::Completed { .. } => 100,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pipeline_from_issue() {
        let template = PipelineTemplate::fast();
        let pipeline = Pipeline::from_issue(42, "Fix bug".to_string(), template);
        assert_eq!(pipeline.issue, Some(42));
        assert_eq!(pipeline.goal, "Fix bug");
    }

    #[test]
    fn test_pipeline_from_goal() {
        let template = PipelineTemplate::standard();
        let pipeline = Pipeline::from_goal("Add feature".to_string(), template);
        assert_eq!(pipeline.issue, None);
        assert_eq!(pipeline.goal, "Add feature");
    }

    #[test]
    fn test_pipeline_current_stage_config() {
        let template = PipelineTemplate::fast();
        let pipeline = Pipeline::from_issue(1, "test".to_string(), template);
        let config = pipeline.current_stage_config();
        assert!(config.is_some());
        assert_eq!(config.unwrap().stage, Stage::Intake);
    }

    #[test]
    fn test_pipeline_advance_stage() {
        let template = PipelineTemplate::fast();
        let mut pipeline = Pipeline::from_issue(1, "test".to_string(), template);
        // First stage is Intake, next should be Build
        assert!(matches!(
            pipeline.state,
            PipelineState::Running {
                current_stage: Stage::Intake,
                ..
            }
        ));
        assert!(pipeline.advance_stage().is_ok());
        // Check that we advanced (doesn't need to be Build, just not Intake)
        match &pipeline.state {
            PipelineState::Running {
                current_stage, ..
            } => {
                assert_ne!(*current_stage, Stage::Intake);
            }
            _ => panic!("Expected Running state"),
        }
    }

    #[test]
    fn test_pipeline_next_iteration() {
        let template = PipelineTemplate::fast();
        let mut pipeline = Pipeline::from_issue(1, "test".to_string(), template);
        assert!(pipeline.next_iteration().is_ok());
        match &pipeline.state {
            PipelineState::Running { iteration, .. } => {
                assert_eq!(*iteration, 1);
            }
            _ => panic!("Expected Running state"),
        }
    }

    #[test]
    fn test_pipeline_fail() {
        let template = PipelineTemplate::fast();
        let mut pipeline = Pipeline::from_issue(1, "test".to_string(), template);
        assert!(pipeline.fail("Error".to_string()).is_ok());
        assert!(pipeline.is_failed());
    }

    #[test]
    fn test_pipeline_pause_and_resume() {
        let template = PipelineTemplate::fast();
        let mut pipeline = Pipeline::from_issue(1, "test".to_string(), template);
        assert!(pipeline.pause("Waiting".to_string()).is_ok());
        assert!(pipeline.resume().is_ok());
        assert!(matches!(
            pipeline.state,
            PipelineState::Running { .. }
        ));
    }

    #[test]
    fn test_pipeline_pause_and_resume_preserves_iteration() {
        let template = PipelineTemplate::fast();
        let mut pipeline = Pipeline::from_issue(1, "test".to_string(), template);

        // Advance to iteration 3
        pipeline.next_iteration().unwrap();
        pipeline.next_iteration().unwrap();
        pipeline.next_iteration().unwrap();

        match &pipeline.state {
            PipelineState::Running { iteration, .. } => {
                assert_eq!(*iteration, 3);
            }
            _ => panic!("Expected Running state"),
        }

        // Pause the pipeline
        pipeline.pause("Waiting for approval".to_string()).unwrap();

        // Resume the pipeline
        pipeline.resume().unwrap();

        // Iteration count must be preserved (H9 fix)
        match &pipeline.state {
            PipelineState::Running { iteration, .. } => {
                assert_eq!(*iteration, 3, "Iteration count should be preserved across pause/resume");
            }
            _ => panic!("Expected Running state"),
        }
    }

    #[test]
    fn test_pipeline_progress_percent() {
        let template = PipelineTemplate::fast();
        let mut pipeline = Pipeline::from_issue(1, "test".to_string(), template);
        // Start at 0% (Intake is first stage)
        assert_eq!(pipeline.progress_percent(), 0);
        // After advancing, should be > 0%
        let _ = pipeline.advance_stage();
        assert!(pipeline.progress_percent() > 0);
    }

    #[test]
    fn test_pipeline_fast_template_only_visits_four_stages() {
        let template = PipelineTemplate::fast();
        let mut pipeline = Pipeline::from_issue(1, "test".to_string(), template);

        // Fast template should only have 4 stages: Intake, Build, Test, Pr
        assert_eq!(pipeline.stages.len(), 4);
        assert_eq!(pipeline.stages[0].stage, Stage::Intake);
        assert_eq!(pipeline.stages[1].stage, Stage::Build);
        assert_eq!(pipeline.stages[2].stage, Stage::Test);
        assert_eq!(pipeline.stages[3].stage, Stage::Pr);

        // Advance through all stages and verify they stay within template
        let mut current_stage = Stage::Intake;
        for i in 0..4 {
            match &pipeline.state {
                PipelineState::Running { current_stage: c, .. } => {
                    assert_eq!(*c, current_stage);
                }
                _ => panic!("Expected Running state at stage {}", i),
            }

            if i < 3 {
                assert!(pipeline.advance_stage().is_ok());
                // Get next expected stage
                current_stage = match current_stage {
                    Stage::Intake => Stage::Build,
                    Stage::Build => Stage::Test,
                    Stage::Test => Stage::Pr,
                    _ => panic!("Unexpected stage"),
                };
            } else {
                // Last stage should complete when advanced
                assert!(pipeline.advance_stage().is_ok());
                assert!(matches!(
                    pipeline.state,
                    PipelineState::Completed { .. }
                ));
            }
        }
    }

    #[test]
    fn test_pipeline_standard_template_visits_correct_stages() {
        let template = PipelineTemplate::standard();
        let pipeline = Pipeline::from_issue(1, "test".to_string(), template);

        // Standard template has 7 stages
        assert_eq!(pipeline.stages.len(), 7);

        // Verify stages are in correct order
        let expected = vec![
            Stage::Intake,
            Stage::Plan,
            Stage::Design,
            Stage::Build,
            Stage::Test,
            Stage::Review,
            Stage::Pr,
        ];

        for (i, expected_stage) in expected.iter().enumerate() {
            assert_eq!(
                pipeline.stages[i].stage, *expected_stage,
                "Stage at index {} mismatch",
                i
            );
        }
    }

    #[test]
    fn test_pipeline_fast_does_not_visit_plan_design_review() {
        let template = PipelineTemplate::fast();
        let mut pipeline = Pipeline::from_issue(1, "test".to_string(), template);

        // Start at Intake
        assert!(matches!(
            pipeline.state,
            PipelineState::Running {
                current_stage: Stage::Intake,
                ..
            }
        ));

        // Advance to Build (skipping Plan and Design that are in standard template)
        pipeline.advance_stage().unwrap();
        assert!(matches!(
            pipeline.state,
            PipelineState::Running {
                current_stage: Stage::Build,
                ..
            }
        ));

        // Advance to Test (skipping Design, Review, etc.)
        pipeline.advance_stage().unwrap();
        assert!(matches!(
            pipeline.state,
            PipelineState::Running {
                current_stage: Stage::Test,
                ..
            }
        ));

        // Advance to Pr (skipping Review)
        pipeline.advance_stage().unwrap();
        assert!(matches!(
            pipeline.state,
            PipelineState::Running {
                current_stage: Stage::Pr,
                ..
            }
        ));

        // Final advance completes the pipeline
        pipeline.advance_stage().unwrap();
        assert!(matches!(
            pipeline.state,
            PipelineState::Completed { .. }
        ));
    }

    #[test]
    fn test_high003_template_stored_as_typed_enum() {
        // HIGH-003: Pipeline should store template as PipelineTemplateName (typed enum),
        // not String, to ensure type safety
        let template = PipelineTemplate::fast();
        let pipeline = Pipeline::from_issue(42, "Fix bug".to_string(), template);

        // Template field should be PipelineTemplateName::Fast, not String "fast"
        assert_eq!(pipeline.template, PipelineTemplateName::Fast);
        assert_eq!(pipeline.issue, Some(42));
    }

    #[test]
    fn test_high003_template_name_parsing() {
        // HIGH-003: Template name parsing should correctly convert strings to enums
        assert_eq!(
            Pipeline::parse_template_name("fast"),
            PipelineTemplateName::Fast
        );
        assert_eq!(
            Pipeline::parse_template_name("standard"),
            PipelineTemplateName::Standard
        );
        assert_eq!(
            Pipeline::parse_template_name("full"),
            PipelineTemplateName::Full
        );
        assert_eq!(
            Pipeline::parse_template_name("hotfix"),
            PipelineTemplateName::Hotfix
        );
        assert_eq!(
            Pipeline::parse_template_name("autonomous"),
            PipelineTemplateName::Autonomous
        );
        assert_eq!(
            Pipeline::parse_template_name("cost-aware"),
            PipelineTemplateName::CostAware
        );
        // Case-insensitive
        assert_eq!(
            Pipeline::parse_template_name("FAST"),
            PipelineTemplateName::Fast
        );
    }

    #[test]
    fn test_med002_progress_percent_failed_at_stage() {
        // MED-002: Progress percent for Failed state should use at_stage,
        // not return hardcoded 0%
        let template = PipelineTemplate::fast();
        let mut pipeline = Pipeline::from_issue(1, "test".to_string(), template);

        // Advance to Monitor stage (final stage)
        while !matches!(pipeline.state, PipelineState::Completed { .. }) {
            let _ = pipeline.advance_stage();
        }

        // Reset to Running at Monitor
        pipeline.state = PipelineState::Running {
            current_stage: Stage::Monitor,
            iteration: 0,
        };

        // Fail at Monitor (last stage, ~92% progress)
        pipeline.fail("Error at Monitor".to_string()).unwrap();

        let progress = pipeline.progress_percent();
        // Monitor is the 12th stage (index 11), so (11 * 100) / 12 = 91%
        assert!(
            progress >= 85 && progress <= 95,
            "Failed at Monitor should show ~91% progress, got {}%",
            progress
        );
        assert_ne!(progress, 0, "Failed state should not return hardcoded 0%");
    }

    #[test]
    fn test_med002_progress_percent_paused_at_stage() {
        // MED-002: Progress percent for Paused state should use at_stage,
        // not return hardcoded 50%
        let template = PipelineTemplate::fast();
        let mut pipeline = Pipeline::from_issue(1, "test".to_string(), template);

        // Advance to Intake (first stage, 0% progress)
        pipeline.pause("Waiting for approval".to_string()).unwrap();

        let progress = pipeline.progress_percent();
        // Intake is the 1st stage (index 0), so (0 * 100) / 12 = 0%
        assert_eq!(
            progress, 0,
            "Paused at Intake should show 0% progress, got {}%",
            progress
        );

        // Now test paused at Build (4th stage, ~25% progress)
        pipeline.state = PipelineState::Paused {
            at_stage: Stage::Build,
            iteration: 2,
            reason: "Waiting".to_string(),
        };

        let progress = pipeline.progress_percent();
        // Build is the 4th stage (index 3), so (3 * 100) / 12 = 25%
        assert!(
            progress >= 20 && progress <= 30,
            "Paused at Build should show ~25% progress, got {}%",
            progress
        );
        assert_ne!(
            progress, 50,
            "Paused state should not return hardcoded 50%"
        );
    }
}
