//! CLI command handlers organized by domain.

pub mod agent;
pub mod channel;
pub mod config;
pub mod skill;

// Re-export public command functions for use in main.rs
pub use agent::{cmd_agent_chat, cmd_agent_kill, cmd_agent_list, cmd_agent_new, cmd_agent_spawn, spawn_template_agent};
pub use channel::{cmd_channel_list, cmd_channel_setup, cmd_channel_test, cmd_channel_toggle};
pub use config::{cmd_config_delete_key, cmd_config_edit, cmd_config_get, cmd_config_set, cmd_config_set_key, cmd_config_show, cmd_config_test_key, cmd_config_unset};
pub use skill::{cmd_skill_create, cmd_skill_install, cmd_skill_list, cmd_skill_remove, cmd_skill_search};
