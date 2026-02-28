//! Pipeline engine: 12-stage delivery pipeline with templates and self-healing.

pub mod composer;
pub mod self_healing;
pub mod stages;
pub mod templates;

pub use composer::{IntelligenceInput, PipelineComposer};
pub use self_healing::{BuildLoop, BuildOutcome, ProgressState};
pub use stages::{Gate, ModelChoice, PipelineState, Stage, StageConfig};
pub use templates::PipelineTemplate;

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// A complete pipeline instance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pipeline {
    pub id: String,
    pub issue: Option<u64>,
    pub goal: String,
    pub template: String,
    pub stages: Vec<StageConfig>,
    pub state: PipelineState,
    pub artifacts_dir: PathBuf,
    pub created_at: String,
    pub updated_at: String,
}

impl Pipeline {
    /// Create a new pipeline from an issue.
    pub fn from_issue(issue_id: u64, goal: String, template: PipelineTemplate) -> Self {
        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();
        let artifacts_dir = PathBuf::from(format!("./.shipwright/pipelines/{}", id));

        Self {
            id: id.clone(),
            issue: Some(issue_id),
            goal,
            template: template.name,
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

        Self {
            id: id.clone(),
            issue: None,
            goal,
            template: template.name,
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

    /// Get the current stage configuration.
    pub fn current_stage_config(&self) -> Option<&StageConfig> {
        match &self.state {
            PipelineState::Running { current_stage, .. } => {
                self.stages.iter().find(|s| s.stage == *current_stage)
            }
            _ => None,
        }
    }

    /// Advance to the next stage.
    pub fn advance_stage(&mut self) -> Result<(), String> {
        let new_state = self.state.advance()?;
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

    /// Get progress percentage.
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
            PipelineState::Failed { .. } => 0,
            PipelineState::Paused { .. } => 50,
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
    fn test_pipeline_progress_percent() {
        let template = PipelineTemplate::fast();
        let mut pipeline = Pipeline::from_issue(1, "test".to_string(), template);
        // Start at 0% (Intake is first stage)
        assert_eq!(pipeline.progress_percent(), 0);
        // After advancing, should be > 0%
        let _ = pipeline.advance_stage();
        assert!(pipeline.progress_percent() > 0);
    }
}
