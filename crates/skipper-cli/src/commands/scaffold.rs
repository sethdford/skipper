//! Scaffold command for creating new skills and integrations.

use crate::ui;

/// What kind of scaffold to create.
#[derive(Clone, clap::ValueEnum)]
pub enum ScaffoldKind {
    /// Scaffold a new skill template
    Skill,
    /// Scaffold a new integration template
    Integration,
}

/// Scaffold a new skill or integration template.
pub fn cmd_scaffold(kind: ScaffoldKind) {
    let cwd = std::env::current_dir().unwrap_or_default();
    let result = match kind {
        ScaffoldKind::Skill => {
            skipper_extensions::installer::scaffold_skill(&cwd.join("my-skill"))
        }
        ScaffoldKind::Integration => {
            skipper_extensions::installer::scaffold_integration(&cwd.join("my-integration"))
        }
    };
    match result {
        Ok(msg) => ui::success(&msg),
        Err(e) => {
            ui::error(&e.to_string());
            std::process::exit(1);
        }
    }
}
