//! Skipper CLI — command-line interface for the Skipper Agent OS.
//!
//! When a daemon is running (`skipper start`), the CLI talks to it over HTTP.
//! Otherwise, commands boot an in-process kernel (single-shot mode).

mod bundled_agents;
mod commands;
mod dotenv;
mod launcher;
mod mcp;
pub mod progress;
pub mod table;
mod templates;
mod tui;
mod ui;

use clap::{Parser, Subcommand};
use skipper_api::server::read_daemon_info;
use skipper_kernel::SkipperKernel;
use std::io::{self, BufRead, Write};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use crate::commands::daemon::boot_kernel_error;
#[cfg(windows)]
use std::sync::atomic::Ordering;

/// Global flag set by the Ctrl+C handler.
static CTRLC_PRESSED: AtomicBool = AtomicBool::new(false);

/// Install a Ctrl+C handler that force-exits the process.
/// On Windows/MINGW, the default handler doesn't reliably interrupt blocking
/// `read_line` calls, so we explicitly call `process::exit`.
fn install_ctrlc_handler() {
    #[cfg(windows)]
    {
        extern "system" {
            fn SetConsoleCtrlHandler(
                handler: Option<unsafe extern "system" fn(u32) -> i32>,
                add: i32,
            ) -> i32;
        }
        unsafe extern "system" fn handler(_ctrl_type: u32) -> i32 {
            if CTRLC_PRESSED.swap(true, Ordering::SeqCst) {
                // Second press: hard exit
                std::process::exit(130);
            }
            // First press: print message and exit cleanly
            let _ = std::io::Write::write_all(&mut std::io::stderr(), b"\nInterrupted.\n");
            std::process::exit(0);
        }
        unsafe { SetConsoleCtrlHandler(Some(handler), 1) };
    }

    #[cfg(not(windows))]
    {
        // On Unix, the default SIGINT handler already interrupts read_line
        // and terminates the process.
        let _ = &CTRLC_PRESSED;
    }
}

const AFTER_HELP: &str = "\
\x1b[1mHint:\x1b[0m Commands suffixed with [*] have subcommands. Run `<command> --help` for details.

\x1b[1;36mExamples:\x1b[0m
  skipper init                 Initialize config and data directories
  skipper start                Start the kernel daemon
  skipper tui                  Launch the interactive terminal dashboard
  skipper chat                 Quick chat with the default agent
  skipper agent new coder      Spawn a new agent from a template
  skipper models list          Browse available LLM models
  skipper add github           Install the GitHub integration
  skipper doctor               Run diagnostic health checks
  skipper channel setup        Interactive channel setup wizard
  skipper cron list            List scheduled jobs

\x1b[1;36mQuick Start:\x1b[0m
  1. skipper init              Set up config + API key
  2. skipper start             Launch the daemon
  3. skipper chat              Start chatting!

\x1b[1;36mMore:\x1b[0m
  Docs:       https://github.com/sethdford/skipper
  Dashboard:  http://127.0.0.1:4200/ (when daemon is running)";

/// Skipper — the open-source Agent Operating System.
#[derive(Parser)]
#[command(
    name = "skipper",
    version,
    about = "\u{1F40D} Skipper \u{2014} Open-source Agent Operating System",
    long_about = "\u{1F40D} Skipper \u{2014} Open-source Agent Operating System\n\n\
                  Deploy, manage, and orchestrate AI agents from your terminal.\n\
                  40 channels \u{00b7} 60 skills \u{00b7} 50+ models \u{00b7} infinite possibilities.",
    after_help = AFTER_HELP,
)]
struct Cli {
    /// Path to config file.
    #[arg(long, global = true)]
    config: Option<PathBuf>,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize Skipper (create ~/.skipper/ and default config).
    Init {
        /// Quick mode: no prompts, just write config + .env (for CI/scripts).
        #[arg(long)]
        quick: bool,
    },
    /// Start the Skipper kernel daemon (API server + kernel).
    Start,
    /// Stop the running daemon.
    Stop,
    /// Manage agents (new, list, chat, kill, spawn) [*].
    #[command(subcommand)]
    Agent(AgentCommands),
    /// Manage workflows (list, create, run) [*].
    #[command(subcommand)]
    Workflow(WorkflowCommands),
    /// Manage event triggers (list, create, delete) [*].
    #[command(subcommand)]
    Trigger(TriggerCommands),
    /// Migrate from another agent framework to Skipper.
    Migrate(MigrateArgs),
    /// Manage skills (install, list, search, create, remove) [*].
    #[command(subcommand)]
    Skill(SkillCommands),
    /// Manage channel integrations (setup, test, enable, disable) [*].
    #[command(subcommand)]
    Channel(ChannelCommands),
    /// Show or edit configuration (show, edit, get, set, keys) [*].
    #[command(subcommand)]
    Config(ConfigCommands),
    /// Quick chat with the default agent.
    Chat {
        /// Optional agent name or ID to chat with.
        agent: Option<String>,
    },
    /// Show kernel status.
    Status {
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
    /// Run diagnostic health checks.
    Doctor {
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
        /// Attempt to auto-fix issues (create missing dirs/config).
        #[arg(long)]
        repair: bool,
    },
    /// Open the web dashboard in the default browser.
    Dashboard,
    /// Generate shell completion scripts.
    Completion {
        /// Shell to generate completions for.
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },
    /// Start MCP (Model Context Protocol) server over stdio.
    Mcp,
    /// Add an integration (one-click MCP server setup).
    Add {
        /// Integration name (e.g., "github", "slack", "notion").
        name: String,
        /// API key or token to store in the vault.
        #[arg(long)]
        key: Option<String>,
    },
    /// Remove an installed integration.
    Remove {
        /// Integration name.
        name: String,
    },
    /// List or search integrations.
    Integrations {
        /// Search query (optional — lists all if omitted).
        query: Option<String>,
    },
    /// Manage the credential vault (init, set, list, remove) [*].
    #[command(subcommand)]
    Vault(VaultCommands),
    /// Scaffold a new skill or integration template.
    New {
        /// What to scaffold.
        #[arg(value_enum)]
        kind: ScaffoldKind,
    },
    /// Launch the interactive terminal dashboard.
    Tui,
    /// Browse models, aliases, and providers [*].
    #[command(subcommand)]
    Models(ModelsCommands),
    /// Daemon control (start, stop, status) [*].
    #[command(subcommand)]
    Gateway(GatewayCommands),
    /// Manage execution approvals (list, approve, reject) [*].
    #[command(subcommand)]
    Approvals(ApprovalsCommands),
    /// Manage scheduled jobs (list, create, delete, enable, disable) [*].
    #[command(subcommand)]
    Cron(CronCommands),
    /// List conversation sessions.
    Sessions {
        /// Optional agent name or ID to filter by.
        agent: Option<String>,
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
    /// Tail the Skipper log file.
    Logs {
        /// Number of lines to show.
        #[arg(long, default_value = "50")]
        lines: usize,
        /// Follow log output in real time.
        #[arg(long, short)]
        follow: bool,
    },
    /// Quick daemon health check.
    Health {
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
    /// Security tools and audit trail [*].
    #[command(subcommand)]
    Security(SecurityCommands),
    /// Search and manage agent memory (KV store) [*].
    #[command(subcommand)]
    Memory(MemoryCommands),
    /// Device pairing and token management [*].
    #[command(subcommand)]
    Devices(DevicesCommands),
    /// Generate device pairing QR code.
    Qr,
    /// Webhook helpers and trigger management [*].
    #[command(subcommand)]
    Webhooks(WebhooksCommands),
    /// Interactive onboarding wizard.
    Onboard {
        /// Quick non-interactive mode.
        #[arg(long)]
        quick: bool,
    },
    /// Quick non-interactive initialization.
    Setup {
        /// Quick mode (same as `init --quick`).
        #[arg(long)]
        quick: bool,
    },
    /// Interactive setup wizard for credentials and channels.
    Configure,
    /// Send a one-shot message to an agent.
    Message {
        /// Agent name or ID.
        agent: String,
        /// Message text.
        text: String,
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
    /// System info and version [*].
    #[command(subcommand)]
    System(SystemCommands),
    /// Reset local config and state.
    Reset {
        /// Skip confirmation prompt.
        #[arg(long)]
        confirm: bool,
    },
}

#[derive(Subcommand)]
enum VaultCommands {
    /// Initialize the credential vault.
    Init,
    /// Store a credential in the vault.
    Set {
        /// Credential key (env var name).
        key: String,
    },
    /// List all keys in the vault (values are hidden).
    List,
    /// Remove a credential from the vault.
    Remove {
        /// Credential key.
        key: String,
    },
}

// Use ScaffoldKind and MigrateArgs from commands module
use commands::scaffold::ScaffoldKind;
use commands::migrate::MigrateArgs;

#[derive(Subcommand)]
enum SkillCommands {
    /// Install a skill from SkipperHub or a local directory.
    Install {
        /// Skill name, local path, or git URL.
        source: String,
    },
    /// List installed skills.
    List,
    /// Remove an installed skill.
    Remove {
        /// Skill name.
        name: String,
    },
    /// Search SkipperHub for skills.
    Search {
        /// Search query.
        query: String,
    },
    /// Create a new skill scaffold.
    Create,
}

#[derive(Subcommand)]
enum ChannelCommands {
    /// List configured channels and their status.
    List,
    /// Interactive setup wizard for a channel.
    Setup {
        /// Channel name (telegram, discord, slack, whatsapp, etc.). Shows picker if omitted.
        channel: Option<String>,
    },
    /// Test a channel by sending a test message.
    Test {
        /// Channel name.
        channel: String,
    },
    /// Enable a channel.
    Enable {
        /// Channel name.
        channel: String,
    },
    /// Disable a channel without removing its configuration.
    Disable {
        /// Channel name.
        channel: String,
    },
}

#[derive(Subcommand)]
enum ConfigCommands {
    /// Show the current configuration.
    Show,
    /// Open the configuration file in your editor.
    Edit,
    /// Get a config value by dotted key path (e.g. "default_model.provider").
    Get {
        /// Dotted key path (e.g. "default_model.provider", "api_listen").
        key: String,
    },
    /// Set a config value (warning: strips TOML comments).
    Set {
        /// Dotted key path.
        key: String,
        /// New value.
        value: String,
    },
    /// Remove a config key (warning: strips TOML comments).
    Unset {
        /// Dotted key path to remove (e.g. "api.cors_origin").
        key: String,
    },
    /// Save an API key to ~/.skipper/.env (prompts interactively).
    SetKey {
        /// Provider name (groq, anthropic, openai, gemini, deepseek, etc.).
        provider: String,
    },
    /// Remove an API key from ~/.skipper/.env.
    DeleteKey {
        /// Provider name.
        provider: String,
    },
    /// Test provider connectivity with the stored API key.
    TestKey {
        /// Provider name.
        provider: String,
    },
}

#[derive(Subcommand)]
enum AgentCommands {
    /// Spawn a new agent from a template (interactive or by name).
    New {
        /// Template name (e.g., "coder", "assistant"). Interactive picker if omitted.
        template: Option<String>,
    },
    /// Spawn a new agent from a manifest file.
    Spawn {
        /// Path to the agent manifest TOML file.
        manifest: PathBuf,
    },
    /// List all running agents.
    List {
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
    /// Interactive chat with an agent.
    Chat {
        /// Agent ID (UUID).
        agent_id: String,
    },
    /// Kill an agent.
    Kill {
        /// Agent ID (UUID).
        agent_id: String,
    },
}

#[derive(Subcommand)]
enum WorkflowCommands {
    /// List all registered workflows.
    List,
    /// Create a workflow from a JSON file.
    Create {
        /// Path to a JSON file describing the workflow.
        file: PathBuf,
    },
    /// Run a workflow by ID.
    Run {
        /// Workflow ID (UUID).
        workflow_id: String,
        /// Input text for the workflow.
        input: String,
    },
}

#[derive(Subcommand)]
enum TriggerCommands {
    /// List all triggers (optionally filtered by agent).
    List {
        /// Optional agent ID to filter by.
        #[arg(long)]
        agent_id: Option<String>,
    },
    /// Create a trigger for an agent.
    Create {
        /// Agent ID (UUID) that owns the trigger.
        agent_id: String,
        /// Trigger pattern as JSON (e.g. '{"lifecycle":{}}' or '{"agent_spawned":{"name_pattern":"*"}}').
        pattern_json: String,
        /// Prompt template (use {{event}} placeholder).
        #[arg(long, default_value = "Event: {{event}}")]
        prompt: String,
        /// Maximum number of times to fire (0 = unlimited).
        #[arg(long, default_value = "0")]
        max_fires: u64,
    },
    /// Delete a trigger by ID.
    Delete {
        /// Trigger ID (UUID).
        trigger_id: String,
    },
}

#[derive(Subcommand)]
enum ModelsCommands {
    /// List available models (optionally filter by provider).
    List {
        /// Filter by provider name.
        #[arg(long)]
        provider: Option<String>,
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
    /// Show model aliases (shorthand names).
    Aliases {
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
    /// List known LLM providers and their auth status.
    Providers {
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
    /// Set the default model for the daemon.
    Set {
        /// Model ID or alias (e.g. "gpt-4o", "claude-sonnet"). Interactive picker if omitted.
        model: Option<String>,
    },
}

#[derive(Subcommand)]
enum GatewayCommands {
    /// Start the kernel daemon.
    Start,
    /// Stop the running daemon.
    Stop,
    /// Show daemon status.
    Status {
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum ApprovalsCommands {
    /// List pending approvals.
    List {
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
    /// Approve a pending request.
    Approve {
        /// Approval ID.
        id: String,
    },
    /// Reject a pending request.
    Reject {
        /// Approval ID.
        id: String,
    },
}

#[derive(Subcommand)]
enum CronCommands {
    /// List scheduled jobs.
    List {
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
    /// Create a new scheduled job.
    Create {
        /// Agent name or ID to run.
        agent: String,
        /// Cron expression (e.g. "0 */6 * * *").
        spec: String,
        /// Prompt to send when the job fires.
        prompt: String,
    },
    /// Delete a scheduled job.
    Delete {
        /// Job ID.
        id: String,
    },
    /// Enable a disabled job.
    Enable {
        /// Job ID.
        id: String,
    },
    /// Disable a job without deleting it.
    Disable {
        /// Job ID.
        id: String,
    },
}

#[derive(Subcommand)]
enum SecurityCommands {
    /// Show security status summary.
    Status {
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
    /// Show recent audit trail entries.
    Audit {
        /// Maximum number of entries to show.
        #[arg(long, default_value = "20")]
        limit: usize,
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
    /// Verify audit trail integrity (Merkle chain).
    Verify,
}

#[derive(Subcommand)]
enum MemoryCommands {
    /// List KV pairs for an agent.
    List {
        /// Agent name or ID.
        agent: String,
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
    /// Get a specific KV value.
    Get {
        /// Agent name or ID.
        agent: String,
        /// Key name.
        key: String,
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
    /// Set a KV value.
    Set {
        /// Agent name or ID.
        agent: String,
        /// Key name.
        key: String,
        /// Value to store.
        value: String,
    },
    /// Delete a KV pair.
    Delete {
        /// Agent name or ID.
        agent: String,
        /// Key name.
        key: String,
    },
}

#[derive(Subcommand)]
enum DevicesCommands {
    /// List paired devices.
    List {
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
    /// Start a new device pairing flow.
    Pair,
    /// Remove a paired device.
    Remove {
        /// Device ID.
        id: String,
    },
}

#[derive(Subcommand)]
enum WebhooksCommands {
    /// List configured webhooks.
    List {
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
    /// Create a new webhook trigger.
    Create {
        /// Agent name or ID.
        agent: String,
        /// Webhook callback URL.
        url: String,
    },
    /// Delete a webhook.
    Delete {
        /// Webhook ID.
        id: String,
    },
    /// Send a test payload to a webhook.
    Test {
        /// Webhook ID.
        id: String,
    },
}

#[derive(Subcommand)]
enum SystemCommands {
    /// Show detailed system info.
    Info {
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
    /// Show version information.
    Version {
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
}

fn init_tracing_stderr() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
}

/// Redirect tracing to a log file so it doesn't corrupt the ratatui TUI.
fn init_tracing_file() {
    let log_dir = dirs::home_dir()
        .map(|h| h.join(".skipper"))
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    let _ = std::fs::create_dir_all(&log_dir);
    let log_path = log_dir.join("tui.log");

    match std::fs::File::create(&log_path) {
        Ok(file) => {
            tracing_subscriber::fmt()
                .with_env_filter(
                    tracing_subscriber::EnvFilter::try_from_default_env()
                        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
                )
                .with_writer(std::sync::Mutex::new(file))
                .with_ansi(false)
                .init();
        }
        Err(_) => {
            // Fallback: suppress all output rather than corrupt the TUI
            tracing_subscriber::fmt()
                .with_max_level(tracing::Level::ERROR)
                .with_writer(std::io::sink)
                .init();
        }
    }
}

fn main() {
    // Load ~/.skipper/.env into process environment (system env takes priority).
    dotenv::load_dotenv();

    let cli = Cli::parse();

    // Determine if this invocation launches a ratatui TUI.
    // TUI modes must NOT install the Ctrl+C handler (it calls process::exit
    // which bypasses ratatui::restore and leaves the terminal in raw mode).
    // TUI modes also need file-based tracing (stderr output corrupts the TUI).
    let is_launcher = cli.command.is_none() && std::io::IsTerminal::is_terminal(&std::io::stdout());
    let is_tui_mode = is_launcher
        || matches!(cli.command, Some(Commands::Tui))
        || matches!(cli.command, Some(Commands::Chat { .. }))
        || matches!(
            cli.command,
            Some(Commands::Agent(AgentCommands::Chat { .. }))
        );

    if is_tui_mode {
        init_tracing_file();
    } else {
        // CLI subcommands: install Ctrl+C handler for clean interrupt of
        // blocking read_line calls, and trace to stderr.
        install_ctrlc_handler();
        init_tracing_stderr();
    }

    match cli.command {
        None => {
            if !std::io::IsTerminal::is_terminal(&std::io::stdout()) {
                // Piped: fall back to text help
                use clap::CommandFactory;
                Cli::command().print_help().unwrap();
                println!();
                return;
            }
            match launcher::run(cli.config.clone()) {
                launcher::LauncherChoice::GetStarted => commands::cmd_init(false),
                launcher::LauncherChoice::Chat => commands::cmd_quick_chat(cli.config, None),
                launcher::LauncherChoice::Dashboard => commands::cmd_dashboard(),
                launcher::LauncherChoice::DesktopApp => launcher::launch_desktop_app(),
                launcher::LauncherChoice::TerminalUI => tui::run(cli.config),
                launcher::LauncherChoice::ShowHelp => {
                    use clap::CommandFactory;
                    Cli::command().print_help().unwrap();
                    println!();
                }
                launcher::LauncherChoice::Quit => {}
            }
        }
        Some(Commands::Tui) => tui::run(cli.config),
        Some(Commands::Init { quick }) => commands::cmd_init(quick),
        Some(Commands::Start) => commands::cmd_start(cli.config),
        Some(Commands::Stop) => commands::cmd_stop(),
        Some(Commands::Agent(sub)) => match sub {
            AgentCommands::New { template } => commands::cmd_agent_new(cli.config, template),
            AgentCommands::Spawn { manifest } => commands::cmd_agent_spawn(cli.config, manifest),
            AgentCommands::List { json } => commands::cmd_agent_list(cli.config, json),
            AgentCommands::Chat { agent_id } => commands::cmd_agent_chat(cli.config, &agent_id),
            AgentCommands::Kill { agent_id } => commands::cmd_agent_kill(cli.config, &agent_id),
        },
        Some(Commands::Workflow(sub)) => match sub {
            WorkflowCommands::List => commands::cmd_workflow_list(),
            WorkflowCommands::Create { file } => commands::cmd_workflow_create(file),
            WorkflowCommands::Run { workflow_id, input } => commands::cmd_workflow_run(&workflow_id, &input),
        },
        Some(Commands::Trigger(sub)) => match sub {
            TriggerCommands::List { agent_id } => commands::cmd_trigger_list(agent_id.as_deref()),
            TriggerCommands::Create {
                agent_id,
                pattern_json,
                prompt,
                max_fires,
            } => commands::cmd_trigger_create(&agent_id, &pattern_json, &prompt, max_fires),
            TriggerCommands::Delete { trigger_id } => commands::cmd_trigger_delete(&trigger_id),
        },
        Some(Commands::Migrate(args)) => commands::cmd_migrate(args),
        Some(Commands::Skill(sub)) => match sub {
            SkillCommands::Install { source } => commands::cmd_skill_install(&source),
            SkillCommands::List => commands::cmd_skill_list(),
            SkillCommands::Remove { name } => commands::cmd_skill_remove(&name),
            SkillCommands::Search { query } => commands::cmd_skill_search(&query),
            SkillCommands::Create => commands::cmd_skill_create(),
        },
        Some(Commands::Channel(sub)) => match sub {
            ChannelCommands::List => commands::cmd_channel_list(),
            ChannelCommands::Setup { channel } => commands::cmd_channel_setup(channel.as_deref()),
            ChannelCommands::Test { channel } => commands::cmd_channel_test(&channel),
            ChannelCommands::Enable { channel } => commands::cmd_channel_toggle(&channel, true),
            ChannelCommands::Disable { channel } => commands::cmd_channel_toggle(&channel, false),
        },
        Some(Commands::Config(sub)) => match sub {
            ConfigCommands::Show => commands::cmd_config_show(),
            ConfigCommands::Edit => commands::cmd_config_edit(),
            ConfigCommands::Get { key } => commands::cmd_config_get(&key),
            ConfigCommands::Set { key, value } => commands::cmd_config_set(&key, &value),
            ConfigCommands::Unset { key } => commands::cmd_config_unset(&key),
            ConfigCommands::SetKey { provider } => commands::cmd_config_set_key(&provider),
            ConfigCommands::DeleteKey { provider } => commands::cmd_config_delete_key(&provider),
            ConfigCommands::TestKey { provider } => commands::cmd_config_test_key(&provider),
        },
        Some(Commands::Chat { agent }) => commands::cmd_quick_chat(cli.config, agent),
        Some(Commands::Status { json }) => commands::cmd_status(cli.config, json),
        Some(Commands::Doctor { json, repair }) => commands::cmd_doctor(json, repair),
        Some(Commands::Dashboard) => commands::cmd_dashboard(),
        Some(Commands::Completion { shell }) => commands::cmd_completion(shell),
        Some(Commands::Mcp) => mcp::run_mcp_server(cli.config),
        Some(Commands::Add { name, key }) => commands::cmd_integration_add(&name, key.as_deref()),
        Some(Commands::Remove { name }) => commands::cmd_integration_remove(&name),
        Some(Commands::Integrations { query }) => commands::cmd_integrations_list(query.as_deref()),
        Some(Commands::Vault(sub)) => match sub {
            VaultCommands::Init => commands::cmd_vault_init(),
            VaultCommands::Set { key } => commands::cmd_vault_set(&key),
            VaultCommands::List => commands::cmd_vault_list(),
            VaultCommands::Remove { key } => commands::cmd_vault_remove(&key),
        },
        Some(Commands::New { kind }) => commands::cmd_scaffold(kind),
        // ── New commands ────────────────────────────────────────────────
        Some(Commands::Models(sub)) => match sub {
            ModelsCommands::List { provider, json } => commands::cmd_models_list(provider.as_deref(), json),
            ModelsCommands::Aliases { json } => commands::cmd_models_aliases(json),
            ModelsCommands::Providers { json } => commands::cmd_models_providers(json),
            ModelsCommands::Set { model } => commands::cmd_models_set(model),
        },
        Some(Commands::Gateway(sub)) => match sub {
            GatewayCommands::Start => commands::cmd_start(cli.config),
            GatewayCommands::Stop => commands::cmd_stop(),
            GatewayCommands::Status { json } => commands::cmd_status(cli.config, json),
        },
        Some(Commands::Approvals(sub)) => match sub {
            ApprovalsCommands::List { json } => commands::cmd_approvals_list(json),
            ApprovalsCommands::Approve { id } => commands::cmd_approvals_respond(&id, true),
            ApprovalsCommands::Reject { id } => commands::cmd_approvals_respond(&id, false),
        },
        Some(Commands::Cron(sub)) => match sub {
            CronCommands::List { json } => commands::cmd_cron_list(json),
            CronCommands::Create {
                agent,
                spec,
                prompt,
            } => commands::cmd_cron_create(&agent, &spec, &prompt),
            CronCommands::Delete { id } => commands::cmd_cron_delete(&id),
            CronCommands::Enable { id } => commands::cmd_cron_toggle(&id, true),
            CronCommands::Disable { id } => commands::cmd_cron_toggle(&id, false),
        },
        Some(Commands::Sessions { agent, json }) => commands::cmd_sessions(agent.as_deref(), json),
        Some(Commands::Logs { lines, follow }) => commands::cmd_logs(lines, follow),
        Some(Commands::Health { json }) => commands::cmd_health(json),
        Some(Commands::Security(sub)) => match sub {
            SecurityCommands::Status { json } => commands::cmd_security_status(json),
            SecurityCommands::Audit { limit, json } => commands::cmd_security_audit(limit, json),
            SecurityCommands::Verify => commands::cmd_security_verify(),
        },
        Some(Commands::Memory(sub)) => match sub {
            MemoryCommands::List { agent, json } => commands::cmd_memory_list(&agent, json),
            MemoryCommands::Get { agent, key, json } => commands::cmd_memory_get(&agent, &key, json),
            MemoryCommands::Set { agent, key, value } => commands::cmd_memory_set(&agent, &key, &value),
            MemoryCommands::Delete { agent, key } => commands::cmd_memory_delete(&agent, &key),
        },
        Some(Commands::Devices(sub)) => match sub {
            DevicesCommands::List { json } => commands::cmd_devices_list(json),
            DevicesCommands::Pair => commands::cmd_devices_pair(),
            DevicesCommands::Remove { id } => commands::cmd_devices_remove(&id),
        },
        Some(Commands::Qr) => commands::cmd_devices_pair(),
        Some(Commands::Webhooks(sub)) => match sub {
            WebhooksCommands::List { json } => commands::cmd_webhooks_list(json),
            WebhooksCommands::Create { agent, url } => commands::cmd_webhooks_create(&agent, &url),
            WebhooksCommands::Delete { id } => commands::cmd_webhooks_delete(&id),
            WebhooksCommands::Test { id } => commands::cmd_webhooks_test(&id),
        },
        Some(Commands::Onboard { quick }) | Some(Commands::Setup { quick }) => commands::cmd_init(quick),
        Some(Commands::Configure) => commands::cmd_init(false),
        Some(Commands::Message { agent, text, json }) => commands::cmd_message(&agent, &text, json),
        Some(Commands::System(sub)) => match sub {
            SystemCommands::Info { json } => commands::cmd_system_info(json),
            SystemCommands::Version { json } => commands::cmd_system_version(json),
        },
        Some(Commands::Reset { confirm }) => commands::cmd_reset(confirm),
    }
}

// ---------------------------------------------------------------------------
// Daemon detection helpers
// ---------------------------------------------------------------------------

/// Try to find a running daemon. Returns its base URL if found.
/// SECURITY: Restrict file permissions to owner-only (0600) on Unix.
#[cfg(unix)]
pub(crate) fn restrict_file_permissions(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
}

#[cfg(not(unix))]
pub(crate) fn restrict_file_permissions(_path: &std::path::Path) {}

/// SECURITY: Restrict directory permissions to owner-only (0700) on Unix.
#[cfg(unix)]
pub(crate) fn restrict_dir_permissions(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700));
}

#[cfg(not(unix))]
pub(crate) fn restrict_dir_permissions(_path: &std::path::Path) {}

pub(crate) fn find_daemon() -> Option<String> {
    let home_dir = dirs::home_dir()?.join(".skipper");
    let info = read_daemon_info(&home_dir)?;

    // Normalize listen address: replace 0.0.0.0 with 127.0.0.1 to avoid
    // DNS/connectivity issues on macOS where 0.0.0.0 can hang.
    let addr = info.listen_addr.replace("0.0.0.0", "127.0.0.1");
    let url = format!("http://{addr}/api/health");

    let client = reqwest::blocking::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(1))
        .timeout(std::time::Duration::from_secs(2))
        .build()
        .ok()?;
    let resp = client.get(&url).send().ok()?;
    if resp.status().is_success() {
        Some(format!("http://{addr}"))
    } else {
        None
    }
}

/// Build an HTTP client for daemon calls.
pub(crate) fn daemon_client() -> reqwest::blocking::Client {
    reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .expect("Failed to build HTTP client")
}

/// Helper: send a request to the daemon and parse the JSON body.
/// Exits with error on connection failure.
pub(crate) fn daemon_json(
    resp: Result<reqwest::blocking::Response, reqwest::Error>,
) -> serde_json::Value {
    match resp {
        Ok(r) => {
            let status = r.status();
            let body = r.json::<serde_json::Value>().unwrap_or_default();
            if status.is_server_error() {
                ui::error_with_fix(
                    &format!("Daemon returned error ({})", status),
                    "Check daemon logs: ~/.skipper/tui.log",
                );
            }
            body
        }
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("timed out") || msg.contains("Timeout") {
                ui::error_with_fix(
                    "Request timed out",
                    "The agent may be processing a complex request. Try again, or check `skipper status`",
                );
            } else if msg.contains("Connection refused") || msg.contains("connect") {
                ui::error_with_fix(
                    "Cannot connect to daemon",
                    "Is the daemon running? Start it with: skipper start",
                );
            } else {
                ui::error_with_fix(
                    &format!("Daemon communication error: {msg}"),
                    "Check `skipper status` or restart: skipper start",
                );
            }
            std::process::exit(1);
        }
    }
}

// ---------------------------------------------------------------------------
// Essential helpers (shared across commands)
// ---------------------------------------------------------------------------

/// Require a running daemon — exit with helpful message if not found.
pub(crate) fn require_daemon(command: &str) -> String {
    find_daemon().unwrap_or_else(|| {
        ui::error_with_fix(
            &format!("`skipper {command}` requires a running daemon"),
            "Start the daemon: skipper start",
        );
        ui::hint("Or try `skipper chat` which works without a daemon");
        std::process::exit(1);
    })
}

/// Boot kernel in single-shot mode (without running daemon).
pub(crate) fn boot_kernel(config: Option<PathBuf>) -> SkipperKernel {
    match SkipperKernel::boot(config.as_deref()) {
        Ok(k) => k,
        Err(e) => {
            boot_kernel_error(&e);
            std::process::exit(1);
        }
    }
}

/// Spawn `skipper start` as a detached background process.
///
/// Polls for daemon health for up to 10 seconds. Returns the daemon URL on success.
pub(crate) fn start_daemon_background() -> Result<String, String> {
    let exe = std::env::current_exe().map_err(|e| format!("Cannot find executable: {e}"))?;

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x00000008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x00000200;
        std::process::Command::new(&exe)
            .arg("start")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP)
            .spawn()
            .map_err(|e| format!("Failed to spawn daemon: {e}"))?;
    }

    #[cfg(not(windows))]
    {
        std::process::Command::new(&exe)
            .arg("start")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| format!("Failed to spawn daemon: {e}"))?;
    }

    // Poll for daemon readiness
    for _ in 0..20 {
        std::thread::sleep(std::time::Duration::from_millis(500));
        if let Some(url) = find_daemon() {
            return Ok(url);
        }
    }

    Err("Daemon did not become ready within 10 seconds".to_string())
}

/// Get the Skipper home directory (~/.skipper).
pub(crate) fn skipper_home() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| {
            eprintln!("Error: Could not determine home directory");
            std::process::exit(1);
        })
        .join(".skipper")
}

/// Prompt the user for input from stdin.
pub(crate) fn prompt_input(prompt: &str) -> String {
    print!("{prompt}");
    io::stdout().flush().unwrap();
    let mut line = String::new();
    io::stdin().lock().read_line(&mut line).unwrap_or(0);
    line.trim().to_string()
}

/// Recursively copy a directory.
pub(crate) fn copy_dir_recursive(src: &PathBuf, dst: &PathBuf) {
    std::fs::create_dir_all(dst).unwrap();
    if let Ok(entries) = std::fs::read_dir(src) {
        for entry in entries.flatten() {
            let path = entry.path();
            let dest_path = dst.join(entry.file_name());
            if path.is_dir() {
                copy_dir_recursive(&path, &dest_path);
            } else {
                let _ = std::fs::copy(&path, &dest_path);
            }
        }
    }
}

/// Test an API key by hitting the provider's models/health endpoint.
/// Re-exports from commands::providers.
pub(crate) fn test_api_key(provider: &str, env_var: &str) -> bool {
    commands::providers::test_api_key(provider, env_var)
}

/// Map provider name to env var name.
/// Re-exports from commands::providers.
pub(crate) fn provider_to_env_var(provider: &str) -> String {
    commands::providers::provider_to_env_var(provider)
}

/// Open a URL in the default browser. Returns true on success.
pub(crate) fn open_in_browser(url: &str) -> bool {
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("cmd")
            .args(["/C", "start", "", url])
            .spawn()
            .is_ok()
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open").arg(url).spawn().is_ok()
    }
    #[cfg(target_os = "linux")]
    {
        // Try xdg-open, then firefox, then chromium
        let result = std::process::Command::new("xdg-open")
            .arg(url)
            .spawn()
            .map(|mut c| c.wait().map(|s| s.success()).unwrap_or(false));
        if result.is_ok() && result.unwrap() {
            return true;
        }
        let result = std::process::Command::new("firefox")
            .arg(url)
            .spawn()
            .map(|mut c| c.wait().map(|s| s.success()).unwrap_or(false));
        if result.is_ok() && result.unwrap() {
            return true;
        }
        std::process::Command::new("chromium")
            .arg(url)
            .spawn()
            .map(|mut c| c.wait().map(|s| s.success()).unwrap_or(false))
            .unwrap_or(false)
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        false
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_doctor_config_deser_default() {
        // Default KernelConfig should serialize/deserialize round-trip
        let config = skipper_types::config::KernelConfig::default();
        let toml_str = toml::to_string_pretty(&config).unwrap();
        let parsed: skipper_types::config::KernelConfig = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed.api_listen, config.api_listen);
    }

    #[test]
    fn test_doctor_config_include_field() {
        let config_toml = r#"
api_listen = "127.0.0.1:4200"
include = ["providers.toml", "agents.toml"]

[default_model]
provider = "groq"
model = "llama-3.3-70b-versatile"
api_key_env = "GROQ_API_KEY"
"#;
        let config: skipper_types::config::KernelConfig = toml::from_str(config_toml).unwrap();
        assert_eq!(config.include.len(), 2);
        assert_eq!(config.include[0], "providers.toml");
        assert_eq!(config.include[1], "agents.toml");
    }

    #[test]
    fn test_doctor_exec_policy_field() {
        let config_toml = r#"
api_listen = "127.0.0.1:4200"

[exec_policy]
mode = "allowlist"
safe_bins = ["ls", "cat", "echo"]
timeout_secs = 30

[default_model]
provider = "groq"
model = "llama-3.3-70b-versatile"
api_key_env = "GROQ_API_KEY"
"#;
        let config: skipper_types::config::KernelConfig = toml::from_str(config_toml).unwrap();
        assert_eq!(
            config.exec_policy.mode,
            skipper_types::config::ExecSecurityMode::Allowlist
        );
        assert_eq!(config.exec_policy.safe_bins.len(), 3);
        assert_eq!(config.exec_policy.timeout_secs, 30);
    }

    #[test]
    fn test_doctor_mcp_transport_validation() {
        let config_toml = r#"
api_listen = "127.0.0.1:4200"

[default_model]
provider = "groq"
model = "llama-3.3-70b-versatile"
api_key_env = "GROQ_API_KEY"

[[mcp_servers]]
name = "github"
timeout_secs = 30

[mcp_servers.transport]
type = "stdio"
command = "npx"
args = ["-y", "@modelcontextprotocol/server-github"]
"#;
        let config: skipper_types::config::KernelConfig = toml::from_str(config_toml).unwrap();
        assert_eq!(config.mcp_servers.len(), 1);
        assert_eq!(config.mcp_servers[0].name, "github");
        match &config.mcp_servers[0].transport {
            skipper_types::config::McpTransportEntry::Stdio { command, args } => {
                assert_eq!(command, "npx");
                assert_eq!(args.len(), 2);
            }
            _ => panic!("Expected Stdio transport"),
        }
    }

    #[test]
    fn test_doctor_skill_injection_scan_clean() {
        let clean_content = "This is a normal skill prompt with helpful instructions.";
        let warnings = skipper_skills::verify::SkillVerifier::scan_prompt_content(clean_content);
        assert!(warnings.is_empty(), "Clean content should have no warnings");
    }

    #[test]
    fn test_doctor_hook_event_variants() {
        // Verify all 4 hook event types are constructable
        use skipper_types::agent::HookEvent;
        let events = [
            HookEvent::BeforeToolCall,
            HookEvent::AfterToolCall,
            HookEvent::BeforePromptBuild,
            HookEvent::AgentLoopEnd,
        ];
        assert_eq!(events.len(), 4);
    }
}
