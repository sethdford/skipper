//! Shipwright Hand Definition
//!
//! Embeds the OpenFang Hand definition (HAND.toml), domain expertise (SKILL.md),
//! and operational playbook (system_prompt.md).

/// HAND.toml — OpenFang Hand definition
/// Configures Shipwright Hand with tools, settings, agent config, and dashboard metrics.
pub const HAND_TOML: &str = include_str!("../agents/shipwright/HAND.toml");

/// SKILL.md — Domain expertise and technical patterns
/// Covers pipeline architecture, build loop convergence, GitHub workflow, test strategy,
/// memory usage, DORA metrics, decision engine integration, and error recovery.
pub const SKILL_MD: &str = include_str!("../agents/shipwright/SKILL.md");

/// system_prompt.md — Multi-phase operational playbook
/// Detailed instructions for platform detection, issue analysis, pipeline execution,
/// build loop with convergence detection, quality assurance, PR creation, deployment,
/// monitoring, and outcome learning.
pub const SYSTEM_PROMPT_MD: &str = include_str!("../agents/shipwright/system_prompt.md");

/// Hand definition metadata
#[derive(Debug, Clone)]
pub struct HandDefinition {
    /// Hand ID
    pub id: String,
    /// Hand name
    pub name: String,
    /// Hand description
    pub description: String,
    /// Category (e.g., "development")
    pub category: String,
}

impl HandDefinition {
    /// Create Shipwright Hand definition
    pub fn shipwright() -> Self {
        Self {
            id: "shipwright".to_string(),
            name: "Shipwright Hand".to_string(),
            description: "Autonomous software delivery — Issue to PR to Production".to_string(),
            category: "development".to_string(),
        }
    }

    /// Get embedded HAND.toml
    pub fn hand_toml(&self) -> &'static str {
        HAND_TOML
    }

    /// Get embedded SKILL.md
    pub fn skill_md(&self) -> &'static str {
        SKILL_MD
    }

    /// Get embedded system_prompt.md
    pub fn system_prompt_md(&self) -> &'static str {
        SYSTEM_PROMPT_MD
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hand_definition_shipwright() {
        let hand = HandDefinition::shipwright();
        assert_eq!(hand.id, "shipwright");
        assert_eq!(hand.name, "Shipwright Hand");
        assert_eq!(hand.category, "development");
    }

    #[test]
    fn test_hand_toml_embedded() {
        let hand = HandDefinition::shipwright();
        let toml = hand.hand_toml();
        assert!(!toml.is_empty());
        assert!(toml.contains("id = \"shipwright\""));
        assert!(toml.contains("pipeline_template"));
        assert!(toml.contains("auto_merge"));
    }

    #[test]
    fn test_skill_md_embedded() {
        let hand = HandDefinition::shipwright();
        let skill = hand.skill_md();
        assert!(!skill.is_empty());
        assert!(skill.contains("Pipeline Architecture"));
        assert!(skill.contains("Build Loop Patterns"));
        assert!(skill.contains("DORA Metrics"));
    }

    #[test]
    fn test_system_prompt_embedded() {
        let hand = HandDefinition::shipwright();
        let prompt = hand.system_prompt_md();
        assert!(!prompt.is_empty());
        assert!(prompt.contains("Phase 0"));
        assert!(prompt.contains("Platform Detection"));
        assert!(prompt.contains("Issue Analysis"));
    }

    #[test]
    fn test_hand_toml_contains_settings() {
        assert!(HAND_TOML.contains("pipeline_template"));
        assert!(HAND_TOML.contains("auto_merge"));
        assert!(HAND_TOML.contains("decision_engine"));
        assert!(HAND_TOML.contains("enable_intelligence"));
    }

    #[test]
    fn test_system_prompt_phases() {
        assert!(SYSTEM_PROMPT_MD.contains("Phase 0"));
        assert!(SYSTEM_PROMPT_MD.contains("Phase 1"));
        assert!(SYSTEM_PROMPT_MD.contains("Phase 2"));
        assert!(SYSTEM_PROMPT_MD.contains("Phase 3"));
        assert!(SYSTEM_PROMPT_MD.contains("Phase 4"));
        assert!(SYSTEM_PROMPT_MD.contains("Phase 5"));
        assert!(SYSTEM_PROMPT_MD.contains("Phase 6"));
        assert!(SYSTEM_PROMPT_MD.contains("Phase 7"));
    }
}
