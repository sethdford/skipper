//! CLI command handlers organized by domain.

pub mod agent;
pub mod channel;
pub mod config;
pub mod skill;
pub mod init;
pub mod doctor;
pub mod daemon;
pub mod workflow;
pub mod integration;
pub mod system;
pub mod security;
pub mod memory;
pub mod devices;
pub mod scaffold;
pub mod migrate;
pub mod message;
pub mod providers;
pub mod models;
pub mod approvals;
pub mod cron;

// Re-export public command functions for use in main.rs
pub use agent::{cmd_agent_chat, cmd_agent_kill, cmd_agent_list, cmd_agent_new, cmd_agent_spawn};
pub use channel::{cmd_channel_list, cmd_channel_setup, cmd_channel_test, cmd_channel_toggle};
pub use config::{cmd_config_delete_key, cmd_config_edit, cmd_config_get, cmd_config_set, cmd_config_set_key, cmd_config_show, cmd_config_test_key, cmd_config_unset};
pub use skill::{cmd_skill_create, cmd_skill_install, cmd_skill_list, cmd_skill_remove, cmd_skill_search};
pub use init::cmd_init;
pub use doctor::cmd_doctor;
pub use daemon::{cmd_start, cmd_stop, cmd_status};
pub use workflow::{cmd_workflow_list, cmd_workflow_create, cmd_workflow_run, cmd_trigger_list, cmd_trigger_create, cmd_trigger_delete};
pub use integration::{cmd_integration_add, cmd_integration_remove, cmd_integrations_list, cmd_vault_init, cmd_vault_set, cmd_vault_list, cmd_vault_remove};
pub use system::{cmd_system_info, cmd_system_version, cmd_dashboard, cmd_completion, cmd_quick_chat, cmd_logs, cmd_health, cmd_reset};
pub use security::{cmd_security_status, cmd_security_audit, cmd_security_verify};
pub use memory::{cmd_memory_list, cmd_memory_get, cmd_memory_set, cmd_memory_delete};
pub use devices::{cmd_devices_list, cmd_devices_pair, cmd_devices_remove, cmd_webhooks_list, cmd_webhooks_create, cmd_webhooks_delete, cmd_webhooks_test};
pub use scaffold::cmd_scaffold;
pub use migrate::cmd_migrate;
pub use message::cmd_message;
pub use models::{cmd_models_list, cmd_models_aliases, cmd_models_providers, cmd_models_set};
pub use approvals::{cmd_approvals_list, cmd_approvals_respond};
pub use cron::{cmd_cron_list, cmd_cron_create, cmd_cron_delete, cmd_cron_toggle, cmd_sessions};
