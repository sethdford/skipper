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
use colored::Colorize;
use skipper_kernel::SkipperKernel;
use skipper_types::agent::AgentManifest;
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

#[derive(Clone, clap::ValueEnum)]
enum ScaffoldKind {
    Skill,
    Integration,
}

#[derive(clap::Args)]
struct MigrateArgs {
    /// Source framework to migrate from.
    #[arg(long, value_enum)]
    from: MigrateSourceArg,
    /// Path to the source workspace (auto-detected if not set).
    #[arg(long)]
    source_dir: Option<PathBuf>,
    /// Dry run — show what would be imported without making changes.
    #[arg(long)]
    dry_run: bool,
}

#[derive(Clone, clap::ValueEnum)]
enum MigrateSourceArg {
    Openclaw,
    Langchain,
    Autogpt,
}

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
                launcher::LauncherChoice::Chat => cmd_quick_chat(cli.config, None),
                launcher::LauncherChoice::Dashboard => cmd_dashboard(),
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
            WorkflowCommands::List => cmd_workflow_list(),
            WorkflowCommands::Create { file } => cmd_workflow_create(file),
            WorkflowCommands::Run { workflow_id, input } => cmd_workflow_run(&workflow_id, &input),
        },
        Some(Commands::Trigger(sub)) => match sub {
            TriggerCommands::List { agent_id } => cmd_trigger_list(agent_id.as_deref()),
            TriggerCommands::Create {
                agent_id,
                pattern_json,
                prompt,
                max_fires,
            } => cmd_trigger_create(&agent_id, &pattern_json, &prompt, max_fires),
            TriggerCommands::Delete { trigger_id } => cmd_trigger_delete(&trigger_id),
        },
        Some(Commands::Migrate(args)) => cmd_migrate(args),
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
        Some(Commands::Chat { agent }) => cmd_quick_chat(cli.config, agent),
        Some(Commands::Status { json }) => commands::cmd_status(cli.config, json),
        Some(Commands::Doctor { json, repair }) => commands::cmd_doctor(json, repair),
        Some(Commands::Dashboard) => cmd_dashboard(),
        Some(Commands::Completion { shell }) => cmd_completion(shell),
        Some(Commands::Mcp) => mcp::run_mcp_server(cli.config),
        Some(Commands::Add { name, key }) => cmd_integration_add(&name, key.as_deref()),
        Some(Commands::Remove { name }) => cmd_integration_remove(&name),
        Some(Commands::Integrations { query }) => cmd_integrations_list(query.as_deref()),
        Some(Commands::Vault(sub)) => match sub {
            VaultCommands::Init => cmd_vault_init(),
            VaultCommands::Set { key } => cmd_vault_set(&key),
            VaultCommands::List => cmd_vault_list(),
            VaultCommands::Remove { key } => cmd_vault_remove(&key),
        },
        Some(Commands::New { kind }) => cmd_scaffold(kind),
        // ── New commands ────────────────────────────────────────────────
        Some(Commands::Models(sub)) => match sub {
            ModelsCommands::List { provider, json } => cmd_models_list(provider.as_deref(), json),
            ModelsCommands::Aliases { json } => cmd_models_aliases(json),
            ModelsCommands::Providers { json } => cmd_models_providers(json),
            ModelsCommands::Set { model } => cmd_models_set(model),
        },
        Some(Commands::Gateway(sub)) => match sub {
            GatewayCommands::Start => commands::cmd_start(cli.config),
            GatewayCommands::Stop => commands::cmd_stop(),
            GatewayCommands::Status { json } => commands::cmd_status(cli.config, json),
        },
        Some(Commands::Approvals(sub)) => match sub {
            ApprovalsCommands::List { json } => cmd_approvals_list(json),
            ApprovalsCommands::Approve { id } => cmd_approvals_respond(&id, true),
            ApprovalsCommands::Reject { id } => cmd_approvals_respond(&id, false),
        },
        Some(Commands::Cron(sub)) => match sub {
            CronCommands::List { json } => cmd_cron_list(json),
            CronCommands::Create {
                agent,
                spec,
                prompt,
            } => cmd_cron_create(&agent, &spec, &prompt),
            CronCommands::Delete { id } => cmd_cron_delete(&id),
            CronCommands::Enable { id } => cmd_cron_toggle(&id, true),
            CronCommands::Disable { id } => cmd_cron_toggle(&id, false),
        },
        Some(Commands::Sessions { agent, json }) => cmd_sessions(agent.as_deref(), json),
        Some(Commands::Logs { lines, follow }) => cmd_logs(lines, follow),
        Some(Commands::Health { json }) => cmd_health(json),
        Some(Commands::Security(sub)) => match sub {
            SecurityCommands::Status { json } => cmd_security_status(json),
            SecurityCommands::Audit { limit, json } => cmd_security_audit(limit, json),
            SecurityCommands::Verify => cmd_security_verify(),
        },
        Some(Commands::Memory(sub)) => match sub {
            MemoryCommands::List { agent, json } => cmd_memory_list(&agent, json),
            MemoryCommands::Get { agent, key, json } => cmd_memory_get(&agent, &key, json),
            MemoryCommands::Set { agent, key, value } => cmd_memory_set(&agent, &key, &value),
            MemoryCommands::Delete { agent, key } => cmd_memory_delete(&agent, &key),
        },
        Some(Commands::Devices(sub)) => match sub {
            DevicesCommands::List { json } => cmd_devices_list(json),
            DevicesCommands::Pair => cmd_devices_pair(),
            DevicesCommands::Remove { id } => cmd_devices_remove(&id),
        },
        Some(Commands::Qr) => cmd_devices_pair(),
        Some(Commands::Webhooks(sub)) => match sub {
            WebhooksCommands::List { json } => cmd_webhooks_list(json),
            WebhooksCommands::Create { agent, url } => cmd_webhooks_create(&agent, &url),
            WebhooksCommands::Delete { id } => cmd_webhooks_delete(&id),
            WebhooksCommands::Test { id } => cmd_webhooks_test(&id),
        },
        Some(Commands::Onboard { quick }) | Some(Commands::Setup { quick }) => commands::cmd_init(quick),
        Some(Commands::Configure) => commands::cmd_init(false),
        Some(Commands::Message { agent, text, json }) => cmd_message(&agent, &text, json),
        Some(Commands::System(sub)) => match sub {
            SystemCommands::Info { json } => cmd_system_info(json),
            SystemCommands::Version { json } => cmd_system_version(json),
        },
        Some(Commands::Reset { confirm }) => cmd_reset(confirm),
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
// Commands (domain-specific command modules below in commands/)
// ---------------------------------------------------------------------------

// Daemon and diagnostic commands are now in commands/daemon.rs and commands/doctor.rs

// cmd_doctor moved to commands/doctor.rs

// ---------------------------------------------------------------------------
// Dashboard command
// ---------------------------------------------------------------------------

fn cmd_dashboard() {
    let base = if let Some(url) = find_daemon() {
        url
    } else {
        // Auto-start the daemon
        ui::hint("No daemon running — starting one now...");
        match start_daemon_background() {
            Ok(url) => {
                ui::success("Daemon started");
                url
            }
            Err(e) => {
                ui::error_with_fix(
                    &format!("Could not start daemon: {e}"),
                    "Start it manually: skipper start",
                );
                std::process::exit(1);
            }
        }
    };

    let url = format!("{base}/");
    ui::success(&format!("Opening dashboard at {url}"));
    if copy_to_clipboard(&url) {
        ui::hint("URL copied to clipboard");
    }
    if !open_in_browser(&url) {
        ui::hint(&format!("Could not open browser. Visit: {url}"));
    }
}

/// Copy text to the system clipboard. Returns true on success.
fn copy_to_clipboard(text: &str) -> bool {
    #[cfg(target_os = "windows")]
    {
        // Use PowerShell to set clipboard (handles special characters better than cmd)
        std::process::Command::new("powershell")
            .args([
                "-NoProfile",
                "-Command",
                &format!("Set-Clipboard '{}'", text.replace('\'', "''")),
            ])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
    #[cfg(target_os = "macos")]
    {
        use std::io::Write as IoWrite;
        std::process::Command::new("pbcopy")
            .stdin(std::process::Stdio::piped())
            .spawn()
            .and_then(|mut child| {
                if let Some(ref mut stdin) = child.stdin {
                    let _ = stdin.write_all(text.as_bytes());
                }
                child.wait()
            })
            .map(|s| s.success())
            .unwrap_or(false)
    }
    #[cfg(target_os = "linux")]
    {
        use std::io::Write as IoWrite;
        // Try xclip first, then xsel
        let result = std::process::Command::new("xclip")
            .args(["-selection", "clipboard"])
            .stdin(std::process::Stdio::piped())
            .spawn()
            .and_then(|mut child| {
                if let Some(ref mut stdin) = child.stdin {
                    let _ = stdin.write_all(text.as_bytes());
                }
                child.wait()
            })
            .map(|s| s.success())
            .unwrap_or(false);
        if result {
            return true;
        }
        std::process::Command::new("xsel")
            .args(["--clipboard", "--input"])
            .stdin(std::process::Stdio::piped())
            .spawn()
            .and_then(|mut child| {
                if let Some(ref mut stdin) = child.stdin {
                    let _ = stdin.write_all(text.as_bytes());
                }
                child.wait()
            })
            .map(|s| s.success())
            .unwrap_or(false)
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        let _ = text;
        false
    }
}

/// Try to open a URL in the default browser. Returns true on success.
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
        std::process::Command::new("xdg-open")
            .arg(url)
            .spawn()
            .is_ok()
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        let _ = url;
        false
    }
}

// ---------------------------------------------------------------------------
// Shell completion command
// ---------------------------------------------------------------------------

fn cmd_completion(shell: clap_complete::Shell) {
    use clap::CommandFactory;
    let mut cmd = Cli::command();
    clap_complete::generate(shell, &mut cmd, "skipper", &mut std::io::stdout());
}

// ---------------------------------------------------------------------------
// Workflow commands
// ---------------------------------------------------------------------------

fn cmd_workflow_list() {
    let base = require_daemon("workflow list");
    let client = daemon_client();
    let body = daemon_json(client.get(format!("{base}/api/workflows")).send());

    match body.as_array() {
        Some(workflows) if workflows.is_empty() => println!("No workflows registered."),
        Some(workflows) => {
            println!("{:<38} {:<20} {:<6} CREATED", "ID", "NAME", "STEPS");
            println!("{}", "-".repeat(80));
            for w in workflows {
                println!(
                    "{:<38} {:<20} {:<6} {}",
                    w["id"].as_str().unwrap_or("?"),
                    w["name"].as_str().unwrap_or("?"),
                    w["steps"].as_u64().unwrap_or(0),
                    w["created_at"].as_str().unwrap_or("?"),
                );
            }
        }
        None => println!("No workflows registered."),
    }
}

fn cmd_workflow_create(file: PathBuf) {
    let base = require_daemon("workflow create");
    if !file.exists() {
        eprintln!("Workflow file not found: {}", file.display());
        std::process::exit(1);
    }
    let contents = std::fs::read_to_string(&file).unwrap_or_else(|e| {
        eprintln!("Error reading workflow file: {e}");
        std::process::exit(1);
    });
    let json_body: serde_json::Value = serde_json::from_str(&contents).unwrap_or_else(|e| {
        eprintln!("Invalid JSON: {e}");
        std::process::exit(1);
    });

    let client = daemon_client();
    let body = daemon_json(
        client
            .post(format!("{base}/api/workflows"))
            .json(&json_body)
            .send(),
    );

    if let Some(id) = body["workflow_id"].as_str() {
        println!("Workflow created successfully!");
        println!("  ID: {id}");
    } else {
        eprintln!(
            "Failed to create workflow: {}",
            body["error"].as_str().unwrap_or("Unknown error")
        );
        std::process::exit(1);
    }
}

fn cmd_workflow_run(workflow_id: &str, input: &str) {
    let base = require_daemon("workflow run");
    let client = daemon_client();
    let body = daemon_json(
        client
            .post(format!("{base}/api/workflows/{workflow_id}/run"))
            .json(&serde_json::json!({"input": input}))
            .send(),
    );

    if let Some(output) = body["output"].as_str() {
        println!("Workflow completed!");
        println!("  Run ID: {}", body["run_id"].as_str().unwrap_or("?"));
        println!("  Output:\n{output}");
    } else {
        eprintln!(
            "Workflow failed: {}",
            body["error"].as_str().unwrap_or("Unknown error")
        );
        std::process::exit(1);
    }
}

// ---------------------------------------------------------------------------
// Trigger commands
// ---------------------------------------------------------------------------

fn cmd_trigger_list(agent_id: Option<&str>) {
    let base = require_daemon("trigger list");
    let client = daemon_client();

    let url = match agent_id {
        Some(id) => format!("{base}/api/triggers?agent_id={id}"),
        None => format!("{base}/api/triggers"),
    };
    let body = daemon_json(client.get(&url).send());

    match body.as_array() {
        Some(triggers) if triggers.is_empty() => println!("No triggers registered."),
        Some(triggers) => {
            println!(
                "{:<38} {:<38} {:<8} {:<6} PATTERN",
                "TRIGGER ID", "AGENT ID", "ENABLED", "FIRES"
            );
            println!("{}", "-".repeat(110));
            for t in triggers {
                println!(
                    "{:<38} {:<38} {:<8} {:<6} {}",
                    t["id"].as_str().unwrap_or("?"),
                    t["agent_id"].as_str().unwrap_or("?"),
                    t["enabled"].as_bool().unwrap_or(false),
                    t["fire_count"].as_u64().unwrap_or(0),
                    t["pattern"],
                );
            }
        }
        None => println!("No triggers registered."),
    }
}

fn cmd_trigger_create(agent_id: &str, pattern_json: &str, prompt: &str, max_fires: u64) {
    let base = require_daemon("trigger create");
    let pattern: serde_json::Value = serde_json::from_str(pattern_json).unwrap_or_else(|e| {
        eprintln!("Invalid pattern JSON: {e}");
        eprintln!("Examples:");
        eprintln!("  '{{\"lifecycle\":{{}}}}'");
        eprintln!("  '{{\"agent_spawned\":{{\"name_pattern\":\"*\"}}}}'");
        eprintln!("  '{{\"agent_terminated\":{{}}}}'");
        eprintln!("  '{{\"all\":{{}}}}'");
        std::process::exit(1);
    });

    let client = daemon_client();
    let body = daemon_json(
        client
            .post(format!("{base}/api/triggers"))
            .json(&serde_json::json!({
                "agent_id": agent_id,
                "pattern": pattern,
                "prompt_template": prompt,
                "max_fires": max_fires,
            }))
            .send(),
    );

    if let Some(id) = body["trigger_id"].as_str() {
        println!("Trigger created successfully!");
        println!("  Trigger ID: {id}");
        println!("  Agent ID:   {agent_id}");
    } else {
        eprintln!(
            "Failed to create trigger: {}",
            body["error"].as_str().unwrap_or("Unknown error")
        );
        std::process::exit(1);
    }
}

fn cmd_trigger_delete(trigger_id: &str) {
    let base = require_daemon("trigger delete");
    let client = daemon_client();
    let body = daemon_json(
        client
            .delete(format!("{base}/api/triggers/{trigger_id}"))
            .send(),
    );

    if body.get("status").is_some() {
        println!("Trigger {trigger_id} deleted.");
    } else {
        eprintln!(
            "Failed to delete trigger: {}",
            body["error"].as_str().unwrap_or("Unknown error")
        );
        std::process::exit(1);
    }
}

/// Require a running daemon — exit with helpful message if not found.
fn require_daemon(command: &str) -> String {
    find_daemon().unwrap_or_else(|| {
        ui::error_with_fix(
            &format!("`skipper {command}` requires a running daemon"),
            "Start the daemon: skipper start",
        );
        ui::hint("Or try `skipper chat` which works without a daemon");
        std::process::exit(1);
    })
}

fn boot_kernel(config: Option<PathBuf>) -> SkipperKernel {
    match SkipperKernel::boot(config.as_deref()) {
        Ok(k) => k,
        Err(e) => {
            boot_kernel_error(&e);
            std::process::exit(1);
        }
    }
}

// ---------------------------------------------------------------------------
// Migrate command
// ---------------------------------------------------------------------------

fn cmd_migrate(args: MigrateArgs) {
    let source = match args.from {
        MigrateSourceArg::Openclaw => skipper_migrate::MigrateSource::OpenClaw,
        MigrateSourceArg::Langchain => skipper_migrate::MigrateSource::LangChain,
        MigrateSourceArg::Autogpt => skipper_migrate::MigrateSource::AutoGpt,
    };

    let source_dir = args.source_dir.unwrap_or_else(|| {
        let home = dirs::home_dir().unwrap_or_else(|| {
            eprintln!("Error: Could not determine home directory");
            std::process::exit(1);
        });
        match source {
            skipper_migrate::MigrateSource::OpenClaw => home.join(".openclaw"),
            skipper_migrate::MigrateSource::LangChain => home.join(".langchain"),
            skipper_migrate::MigrateSource::AutoGpt => home.join("Auto-GPT"),
        }
    });

    let target_dir = dirs::home_dir()
        .unwrap_or_else(|| {
            eprintln!("Error: Could not determine home directory");
            std::process::exit(1);
        })
        .join(".skipper");

    println!("Migrating from {} ({})...", source, source_dir.display());
    if args.dry_run {
        println!("  (dry run — no changes will be made)\n");
    }

    let options = skipper_migrate::MigrateOptions {
        source,
        source_dir,
        target_dir,
        dry_run: args.dry_run,
    };

    match skipper_migrate::run_migration(&options) {
        Ok(report) => {
            report.print_summary();

            // Save migration report
            if !args.dry_run {
                let report_path = options.target_dir.join("migration_report.md");
                if let Err(e) = std::fs::write(&report_path, report.to_markdown()) {
                    eprintln!("Warning: Could not save migration report: {e}");
                } else {
                    println!("\n  Report saved to: {}", report_path.display());
                }
            }
        }
        Err(e) => {
            eprintln!("Migration failed: {e}");
            std::process::exit(1);
        }
    }
}

// (Skill and channel commands extracted to commands/ modules)

/// Map a provider name to its conventional environment variable name.
fn provider_to_env_var(provider: &str) -> String {
    match provider.to_lowercase().as_str() {
        "groq" => "GROQ_API_KEY".to_string(),
        "anthropic" => "ANTHROPIC_API_KEY".to_string(),
        "openai" => "OPENAI_API_KEY".to_string(),
        "gemini" => "GEMINI_API_KEY".to_string(),
        "google" => "GOOGLE_API_KEY".to_string(),
        "deepseek" => "DEEPSEEK_API_KEY".to_string(),
        "openrouter" => "OPENROUTER_API_KEY".to_string(),
        "together" => "TOGETHER_API_KEY".to_string(),
        "mistral" => "MISTRAL_API_KEY".to_string(),
        "fireworks" => "FIREWORKS_API_KEY".to_string(),
        "perplexity" => "PERPLEXITY_API_KEY".to_string(),
        "cohere" => "COHERE_API_KEY".to_string(),
        "xai" => "XAI_API_KEY".to_string(),
        "brave" => "BRAVE_API_KEY".to_string(),
        "tavily" => "TAVILY_API_KEY".to_string(),
        other => format!("{}_API_KEY", other.to_uppercase()),
    }
}

/// Test an API key by hitting the provider's models/health endpoint.
///
/// Returns true if the key is accepted (status != 401/403).
/// Returns true on timeout/network errors (best-effort — don't block setup).
pub(crate) fn test_api_key(provider: &str, env_var: &str) -> bool {
    let key = match std::env::var(env_var) {
        Ok(k) => k,
        Err(_) => return false,
    };

    let client = match reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
    {
        Ok(c) => c,
        Err(_) => return true, // can't build client — assume ok
    };

    let result = match provider.to_lowercase().as_str() {
        "groq" => client
            .get("https://api.groq.com/openai/v1/models")
            .bearer_auth(&key)
            .send(),
        "anthropic" => client
            .get("https://api.anthropic.com/v1/models")
            .header("x-api-key", &key)
            .header("anthropic-version", "2023-06-01")
            .send(),
        "openai" => client
            .get("https://api.openai.com/v1/models")
            .bearer_auth(&key)
            .send(),
        "gemini" | "google" => client
            .get(format!(
                "https://generativelanguage.googleapis.com/v1beta/models?key={key}"
            ))
            .send(),
        "deepseek" => client
            .get("https://api.deepseek.com/models")
            .bearer_auth(&key)
            .send(),
        "openrouter" => client
            .get("https://openrouter.ai/api/v1/models")
            .bearer_auth(&key)
            .send(),
        _ => return true, // unknown provider — skip test
    };

    match result {
        Ok(resp) => {
            let status = resp.status().as_u16();
            status != 401 && status != 403
        }
        Err(_) => true, // network error — don't block setup
    }
}

// ---------------------------------------------------------------------------
// Background daemon start
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Config commands
// ---------------------------------------------------------------------------









pub(crate) fn cmd_quick_chat(config: Option<PathBuf>, agent: Option<String>) {
    tui::chat_runner::run_chat_tui(config, agent);
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

pub(crate) fn skipper_home() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| {
            eprintln!("Error: Could not determine home directory");
            std::process::exit(1);
        })
        .join(".skipper")
}

fn prompt_input(prompt: &str) -> String {
    print!("{prompt}");
    io::stdout().flush().unwrap();
    let mut line = String::new();
    io::stdin().lock().read_line(&mut line).unwrap_or(0);
    line.trim().to_string()
}

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

// ---------------------------------------------------------------------------
// Integration commands (skipper add/remove/integrations)
// ---------------------------------------------------------------------------

fn cmd_integration_add(name: &str, key: Option<&str>) {
    let home = skipper_home();
    let mut registry = skipper_extensions::registry::IntegrationRegistry::new(&home);
    registry.load_bundled();
    let _ = registry.load_installed();

    // Check template exists
    let template = match registry.get_template(name) {
        Some(t) => t.clone(),
        None => {
            ui::error(&format!("Unknown integration: '{name}'"));
            println!("\nAvailable integrations:");
            for t in registry.list_templates() {
                println!("  {} {} — {}", t.icon, t.id, t.description);
            }
            std::process::exit(1);
        }
    };

    // Set up credential resolver
    let dotenv_path = home.join(".env");
    let vault_path = home.join("vault.enc");
    let vault = if vault_path.exists() {
        let mut v = skipper_extensions::vault::CredentialVault::new(vault_path);
        if v.unlock().is_ok() {
            Some(v)
        } else {
            None
        }
    } else {
        None
    };
    let mut resolver =
        skipper_extensions::credentials::CredentialResolver::new(vault, Some(&dotenv_path))
            .with_interactive(true);

    // Build provided keys map
    let mut provided_keys = std::collections::HashMap::new();
    if let Some(key_value) = key {
        // Auto-detect which env var to use (first required_env that's a secret)
        if let Some(env_var) = template.required_env.iter().find(|e| e.is_secret) {
            provided_keys.insert(env_var.name.clone(), key_value.to_string());
        }
    }

    match skipper_extensions::installer::install_integration(
        &mut registry,
        &mut resolver,
        name,
        &provided_keys,
    ) {
        Ok(result) => {
            match &result.status {
                skipper_extensions::IntegrationStatus::Ready => {
                    ui::success(&result.message);
                }
                skipper_extensions::IntegrationStatus::Setup => {
                    println!("{}", result.message.yellow());
                    println!("\nTo add credentials:");
                    for env in &template.required_env {
                        if env.is_secret {
                            println!("  skipper vault set {}  # {}", env.name, env.help);
                            if let Some(ref url) = env.get_url {
                                println!("  Get it here: {url}");
                            }
                        }
                    }
                }
                _ => println!("{}", result.message),
            }

            // If daemon is running, trigger hot-reload
            if let Some(base_url) = find_daemon() {
                let client = daemon_client();
                let _ = client
                    .post(format!("{base_url}/api/integrations/reload"))
                    .send();
            }
        }
        Err(e) => {
            ui::error(&e.to_string());
            std::process::exit(1);
        }
    }
}

fn cmd_integration_remove(name: &str) {
    let home = skipper_home();
    let mut registry = skipper_extensions::registry::IntegrationRegistry::new(&home);
    registry.load_bundled();
    let _ = registry.load_installed();

    match skipper_extensions::installer::remove_integration(&mut registry, name) {
        Ok(msg) => {
            ui::success(&msg);
            // Hot-reload daemon
            if let Some(base_url) = find_daemon() {
                let client = daemon_client();
                let _ = client
                    .post(format!("{base_url}/api/integrations/reload"))
                    .send();
            }
        }
        Err(e) => {
            ui::error(&e.to_string());
            std::process::exit(1);
        }
    }
}

fn cmd_integrations_list(query: Option<&str>) {
    let home = skipper_home();
    let mut registry = skipper_extensions::registry::IntegrationRegistry::new(&home);
    registry.load_bundled();
    let _ = registry.load_installed();

    let dotenv_path = home.join(".env");
    let resolver =
        skipper_extensions::credentials::CredentialResolver::new(None, Some(&dotenv_path));

    let entries = if let Some(q) = query {
        skipper_extensions::installer::search_integrations(&registry, q)
    } else {
        skipper_extensions::installer::list_integrations(&registry, &resolver)
    };

    if entries.is_empty() {
        if let Some(q) = query {
            println!("No integrations matching '{q}'.");
        } else {
            println!("No integrations available.");
        }
        return;
    }

    // Group by category
    let mut by_category: std::collections::BTreeMap<
        String,
        Vec<&skipper_extensions::installer::IntegrationListEntry>,
    > = std::collections::BTreeMap::new();
    for entry in &entries {
        by_category
            .entry(entry.category.clone())
            .or_default()
            .push(entry);
    }

    for (category, items) in &by_category {
        println!("\n{}", format!("  {category}").bold());
        for item in items {
            let status_badge = match &item.status {
                skipper_extensions::IntegrationStatus::Ready => "[Ready]".green().to_string(),
                skipper_extensions::IntegrationStatus::Setup => "[Setup]".yellow().to_string(),
                skipper_extensions::IntegrationStatus::Available => {
                    "[Available]".dimmed().to_string()
                }
                skipper_extensions::IntegrationStatus::Error(msg) => {
                    format!("[Error: {msg}]").red().to_string()
                }
                skipper_extensions::IntegrationStatus::Disabled => {
                    "[Disabled]".dimmed().to_string()
                }
            };
            println!(
                "    {} {:<20} {:<12} {}",
                item.icon, item.id, status_badge, item.description
            );
        }
    }
    println!();
    println!(
        "  {} integrations ({} installed)",
        entries.len(),
        entries
            .iter()
            .filter(|e| matches!(
                e.status,
                skipper_extensions::IntegrationStatus::Ready
                    | skipper_extensions::IntegrationStatus::Setup
            ))
            .count()
    );
    println!("  Use `skipper add <name>` to install an integration.");
}

// ---------------------------------------------------------------------------
// Vault commands (skipper vault init/set/list/remove)
// ---------------------------------------------------------------------------

fn cmd_vault_init() {
    let home = skipper_home();
    let vault_path = home.join("vault.enc");
    let mut vault = skipper_extensions::vault::CredentialVault::new(vault_path);

    match vault.init() {
        Ok(()) => ui::success("Credential vault initialized."),
        Err(e) => {
            ui::error(&e.to_string());
            std::process::exit(1);
        }
    }
}

fn cmd_vault_set(key: &str) {
    use zeroize::Zeroizing;

    let home = skipper_home();
    let vault_path = home.join("vault.enc");
    let mut vault = skipper_extensions::vault::CredentialVault::new(vault_path);

    if !vault.exists() {
        ui::error("Vault not initialized. Run: skipper vault init");
        std::process::exit(1);
    }

    if let Err(e) = vault.unlock() {
        ui::error(&format!("Could not unlock vault: {e}"));
        std::process::exit(1);
    }

    let value = prompt_input(&format!("Enter value for {key}: "));
    if value.is_empty() {
        ui::error("Empty value — not stored.");
        std::process::exit(1);
    }

    match vault.set(key.to_string(), Zeroizing::new(value)) {
        Ok(()) => ui::success(&format!("Stored '{key}' in vault.")),
        Err(e) => {
            ui::error(&format!("Failed to store: {e}"));
            std::process::exit(1);
        }
    }
}

fn cmd_vault_list() {
    let home = skipper_home();
    let vault_path = home.join("vault.enc");
    let mut vault = skipper_extensions::vault::CredentialVault::new(vault_path);

    if !vault.exists() {
        println!("Vault not initialized. Run: skipper vault init");
        return;
    }

    if let Err(e) = vault.unlock() {
        ui::error(&format!("Could not unlock vault: {e}"));
        std::process::exit(1);
    }

    let keys = vault.list_keys();
    if keys.is_empty() {
        println!("Vault is empty.");
    } else {
        println!("Stored credentials ({}):", keys.len());
        for key in keys {
            println!("  {key}");
        }
    }
}

fn cmd_vault_remove(key: &str) {
    let home = skipper_home();
    let vault_path = home.join("vault.enc");
    let mut vault = skipper_extensions::vault::CredentialVault::new(vault_path);

    if !vault.exists() {
        ui::error("Vault not initialized.");
        std::process::exit(1);
    }
    if let Err(e) = vault.unlock() {
        ui::error(&format!("Could not unlock vault: {e}"));
        std::process::exit(1);
    }

    match vault.remove(key) {
        Ok(true) => ui::success(&format!("Removed '{key}' from vault.")),
        Ok(false) => println!("Key '{key}' not found in vault."),
        Err(e) => {
            ui::error(&format!("Failed to remove: {e}"));
            std::process::exit(1);
        }
    }
}

// ---------------------------------------------------------------------------
// Scaffold commands (skipper new skill/integration)
// ---------------------------------------------------------------------------

fn cmd_scaffold(kind: ScaffoldKind) {
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

// ---------------------------------------------------------------------------
// New command handlers
// ---------------------------------------------------------------------------

fn cmd_models_list(provider_filter: Option<&str>, json: bool) {
    if let Some(base) = find_daemon() {
        let client = daemon_client();
        let url = match provider_filter {
            Some(p) => format!("{base}/api/models?provider={p}"),
            None => format!("{base}/api/models"),
        };
        let body = daemon_json(client.get(&url).send());
        if json {
            println!(
                "{}",
                serde_json::to_string_pretty(&body).unwrap_or_default()
            );
            return;
        }
        if let Some(arr) = body.as_array() {
            if arr.is_empty() {
                println!("No models found.");
                return;
            }
            println!("{:<40} {:<16} {:<8} CONTEXT", "MODEL", "PROVIDER", "TIER");
            println!("{}", "-".repeat(80));
            for m in arr {
                println!(
                    "{:<40} {:<16} {:<8} {}",
                    m["id"].as_str().unwrap_or("?"),
                    m["provider"].as_str().unwrap_or("?"),
                    m["tier"].as_str().unwrap_or("?"),
                    m["context_window"].as_u64().unwrap_or(0),
                );
            }
        } else {
            println!(
                "{}",
                serde_json::to_string_pretty(&body).unwrap_or_default()
            );
        }
    } else {
        // Standalone: use ModelCatalog directly
        let catalog = skipper_runtime::model_catalog::ModelCatalog::new();
        let models = catalog.list_models();
        if json {
            let arr: Vec<serde_json::Value> = models
                .iter()
                .filter(|m| provider_filter.is_none_or(|p| m.provider == p))
                .map(|m| {
                    serde_json::json!({
                        "id": m.id,
                        "provider": m.provider,
                        "tier": format!("{:?}", m.tier),
                        "context_window": m.context_window,
                    })
                })
                .collect();
            println!("{}", serde_json::to_string_pretty(&arr).unwrap_or_default());
            return;
        }
        if models.is_empty() {
            println!("No models in catalog.");
            return;
        }
        println!("{:<40} {:<16} {:<8} CONTEXT", "MODEL", "PROVIDER", "TIER");
        println!("{}", "-".repeat(80));
        for m in models {
            if let Some(p) = provider_filter {
                if m.provider != p {
                    continue;
                }
            }
            println!(
                "{:<40} {:<16} {:<8} {}",
                m.id,
                m.provider,
                format!("{:?}", m.tier),
                m.context_window,
            );
        }
    }
}

fn cmd_models_aliases(json: bool) {
    if let Some(base) = find_daemon() {
        let client = daemon_client();
        let body = daemon_json(client.get(format!("{base}/api/models/aliases")).send());
        if json {
            println!(
                "{}",
                serde_json::to_string_pretty(&body).unwrap_or_default()
            );
            return;
        }
        if let Some(obj) = body.as_object() {
            println!("{:<30} RESOLVES TO", "ALIAS");
            println!("{}", "-".repeat(60));
            for (alias, target) in obj {
                println!("{:<30} {}", alias, target.as_str().unwrap_or("?"));
            }
        } else {
            println!(
                "{}",
                serde_json::to_string_pretty(&body).unwrap_or_default()
            );
        }
    } else {
        let catalog = skipper_runtime::model_catalog::ModelCatalog::new();
        let aliases = catalog.list_aliases();
        if json {
            let obj: serde_json::Map<String, serde_json::Value> = aliases
                .iter()
                .map(|(a, t)| (a.to_string(), serde_json::Value::String(t.to_string())))
                .collect();
            println!("{}", serde_json::to_string_pretty(&obj).unwrap_or_default());
            return;
        }
        println!("{:<30} RESOLVES TO", "ALIAS");
        println!("{}", "-".repeat(60));
        for (alias, target) in aliases {
            println!("{:<30} {}", alias, target);
        }
    }
}

fn cmd_models_providers(json: bool) {
    if let Some(base) = find_daemon() {
        let client = daemon_client();
        let body = daemon_json(client.get(format!("{base}/api/providers")).send());
        if json {
            println!(
                "{}",
                serde_json::to_string_pretty(&body).unwrap_or_default()
            );
            return;
        }
        if let Some(arr) = body.as_array() {
            println!(
                "{:<20} {:<12} {:<10} BASE URL",
                "PROVIDER", "AUTH", "MODELS"
            );
            println!("{}", "-".repeat(70));
            for p in arr {
                println!(
                    "{:<20} {:<12} {:<10} {}",
                    p["id"].as_str().unwrap_or("?"),
                    p["auth_status"].as_str().unwrap_or("?"),
                    p["model_count"].as_u64().unwrap_or(0),
                    p["base_url"].as_str().unwrap_or(""),
                );
            }
        } else {
            println!(
                "{}",
                serde_json::to_string_pretty(&body).unwrap_or_default()
            );
        }
    } else {
        let catalog = skipper_runtime::model_catalog::ModelCatalog::new();
        let providers = catalog.list_providers();
        if json {
            let arr: Vec<serde_json::Value> = providers
                .iter()
                .map(|p| {
                    serde_json::json!({
                        "id": p.id,
                        "auth_status": format!("{:?}", p.auth_status),
                        "model_count": p.model_count,
                        "base_url": p.base_url,
                    })
                })
                .collect();
            println!("{}", serde_json::to_string_pretty(&arr).unwrap_or_default());
            return;
        }
        println!(
            "{:<20} {:<12} {:<10} BASE URL",
            "PROVIDER", "AUTH", "MODELS"
        );
        println!("{}", "-".repeat(70));
        for p in providers {
            println!(
                "{:<20} {:<12} {:<10} {}",
                p.id,
                format!("{:?}", p.auth_status),
                p.model_count,
                p.base_url,
            );
        }
    }
}

fn cmd_models_set(model: Option<String>) {
    let model = match model {
        Some(m) => m,
        None => pick_model(),
    };
    let base = require_daemon("models set");
    let client = daemon_client();
    // Use the config set approach through the API
    let body = daemon_json(
        client
            .post(format!("{base}/api/config/set"))
            .json(&serde_json::json!({"key": "default_model.model", "value": model}))
            .send(),
    );
    if body.get("error").is_some() {
        ui::error(&format!(
            "Failed to set model: {}",
            body["error"].as_str().unwrap_or("?")
        ));
    } else {
        ui::success(&format!("Default model set to: {model}"));
    }
}

/// Interactive model picker — shows numbered list, accepts number or model ID.
fn pick_model() -> String {
    let catalog = skipper_runtime::model_catalog::ModelCatalog::new();
    let models = catalog.list_models();

    if models.is_empty() {
        ui::error("No models in catalog.");
        std::process::exit(1);
    }

    // Group by provider for display
    let mut by_provider: std::collections::BTreeMap<
        String,
        Vec<&skipper_types::model_catalog::ModelCatalogEntry>,
    > = std::collections::BTreeMap::new();
    for m in models {
        by_provider.entry(m.provider.clone()).or_default().push(m);
    }

    ui::section("Select a model");
    ui::blank();

    let mut numbered: Vec<&str> = Vec::new();
    let mut idx = 1;
    for (provider, provider_models) in &by_provider {
        println!("  {}:", provider.bold());
        for m in provider_models {
            println!("    {idx:>3}. {:<36} {:?}", m.id, m.tier);
            numbered.push(&m.id);
            idx += 1;
        }
    }
    ui::blank();

    loop {
        let input = prompt_input("  Enter number or model ID: ");
        if input.is_empty() {
            continue;
        }
        // Try as number first
        if let Ok(n) = input.parse::<usize>() {
            if n >= 1 && n <= numbered.len() {
                return numbered[n - 1].to_string();
            }
            ui::error(&format!("Number out of range (1-{})", numbered.len()));
            continue;
        }
        // Accept direct model ID if it exists in catalog
        if models.iter().any(|m| m.id == input) {
            return input;
        }
        // Accept as alias
        if catalog.resolve_alias(&input).is_some() {
            return input;
        }
        // Accept any string (user might know a model not in catalog)
        return input;
    }
}

fn cmd_approvals_list(json: bool) {
    let base = require_daemon("approvals list");
    let client = daemon_client();
    let body = daemon_json(client.get(format!("{base}/api/approvals")).send());
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
        return;
    }
    if let Some(arr) = body.as_array() {
        if arr.is_empty() {
            println!("No pending approvals.");
            return;
        }
        println!("{:<38} {:<16} {:<12} REQUEST", "ID", "AGENT", "TYPE");
        println!("{}", "-".repeat(80));
        for a in arr {
            println!(
                "{:<38} {:<16} {:<12} {}",
                a["id"].as_str().unwrap_or("?"),
                a["agent_name"].as_str().unwrap_or("?"),
                a["approval_type"].as_str().unwrap_or("?"),
                a["description"].as_str().unwrap_or(""),
            );
        }
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
    }
}

fn cmd_approvals_respond(id: &str, approve: bool) {
    let base = require_daemon("approvals");
    let client = daemon_client();
    let endpoint = if approve { "approve" } else { "reject" };
    let body = daemon_json(
        client
            .post(format!("{base}/api/approvals/{id}/{endpoint}"))
            .send(),
    );
    if body.get("error").is_some() {
        ui::error(&format!(
            "Failed: {}",
            body["error"].as_str().unwrap_or("?")
        ));
    } else {
        ui::success(&format!("Approval {id} {endpoint}d."));
    }
}

fn cmd_cron_list(json: bool) {
    let base = require_daemon("cron list");
    let client = daemon_client();
    let body = daemon_json(client.get(format!("{base}/api/cron/jobs")).send());
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
        return;
    }
    if let Some(arr) = body.as_array() {
        if arr.is_empty() {
            println!("No scheduled jobs.");
            return;
        }
        println!(
            "{:<38} {:<16} {:<20} {:<8} PROMPT",
            "ID", "AGENT", "SCHEDULE", "ENABLED"
        );
        println!("{}", "-".repeat(100));
        for j in arr {
            println!(
                "{:<38} {:<16} {:<20} {:<8} {}",
                j["id"].as_str().unwrap_or("?"),
                j["agent_id"].as_str().unwrap_or("?"),
                j["cron_expr"].as_str().unwrap_or("?"),
                if j["enabled"].as_bool().unwrap_or(false) {
                    "yes"
                } else {
                    "no"
                },
                j["prompt"]
                    .as_str()
                    .unwrap_or("")
                    .chars()
                    .take(40)
                    .collect::<String>(),
            );
        }
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
    }
}

fn cmd_cron_create(agent: &str, spec: &str, prompt: &str) {
    let base = require_daemon("cron create");
    let client = daemon_client();
    let body = daemon_json(
        client
            .post(format!("{base}/api/cron/jobs"))
            .json(&serde_json::json!({
                "agent_id": agent,
                "cron_expr": spec,
                "prompt": prompt,
            }))
            .send(),
    );
    if let Some(id) = body["id"].as_str() {
        ui::success(&format!("Cron job created: {id}"));
    } else {
        ui::error(&format!(
            "Failed: {}",
            body["error"].as_str().unwrap_or("?")
        ));
    }
}

fn cmd_cron_delete(id: &str) {
    let base = require_daemon("cron delete");
    let client = daemon_client();
    let body = daemon_json(client.delete(format!("{base}/api/cron/jobs/{id}")).send());
    if body.get("error").is_some() {
        ui::error(&format!(
            "Failed: {}",
            body["error"].as_str().unwrap_or("?")
        ));
    } else {
        ui::success(&format!("Cron job {id} deleted."));
    }
}

fn cmd_cron_toggle(id: &str, enable: bool) {
    let base = require_daemon("cron");
    let client = daemon_client();
    let endpoint = if enable { "enable" } else { "disable" };
    let body = daemon_json(
        client
            .post(format!("{base}/api/cron/jobs/{id}/{endpoint}"))
            .send(),
    );
    if body.get("error").is_some() {
        ui::error(&format!(
            "Failed: {}",
            body["error"].as_str().unwrap_or("?")
        ));
    } else {
        ui::success(&format!("Cron job {id} {endpoint}d."));
    }
}

fn cmd_sessions(agent: Option<&str>, json: bool) {
    let base = require_daemon("sessions");
    let client = daemon_client();
    let url = match agent {
        Some(a) => format!("{base}/api/sessions?agent={a}"),
        None => format!("{base}/api/sessions"),
    };
    let body = daemon_json(client.get(&url).send());
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
        return;
    }
    if let Some(arr) = body.as_array() {
        if arr.is_empty() {
            println!("No sessions found.");
            return;
        }
        println!("{:<38} {:<16} {:<8} LAST ACTIVE", "ID", "AGENT", "MSGS");
        println!("{}", "-".repeat(80));
        for s in arr {
            println!(
                "{:<38} {:<16} {:<8} {}",
                s["id"].as_str().unwrap_or("?"),
                s["agent_name"].as_str().unwrap_or("?"),
                s["message_count"].as_u64().unwrap_or(0),
                s["last_active"].as_str().unwrap_or("?"),
            );
        }
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
    }
}

fn cmd_logs(lines: usize, follow: bool) {
    let log_path = dirs::home_dir()
        .map(|h| h.join(".skipper").join("tui.log"))
        .unwrap_or_else(|| PathBuf::from("tui.log"));

    if !log_path.exists() {
        ui::error_with_fix(
            "Log file not found",
            &format!("Expected at: {}", log_path.display()),
        );
        std::process::exit(1);
    }

    if follow {
        // Use tail -f equivalent
        #[cfg(unix)]
        {
            let _ = std::process::Command::new("tail")
                .args(["-f", "-n", &lines.to_string()])
                .arg(&log_path)
                .status();
        }
        #[cfg(windows)]
        {
            // On Windows, read in a loop
            let content = std::fs::read_to_string(&log_path).unwrap_or_default();
            let all_lines: Vec<&str> = content.lines().collect();
            let start = all_lines.len().saturating_sub(lines);
            for line in &all_lines[start..] {
                println!("{line}");
            }
            println!("--- Following {} (Ctrl+C to stop) ---", log_path.display());
            let mut last_len = content.len();
            loop {
                std::thread::sleep(std::time::Duration::from_millis(500));
                if let Ok(new_content) = std::fs::read_to_string(&log_path) {
                    if new_content.len() > last_len {
                        print!("{}", &new_content[last_len..]);
                        last_len = new_content.len();
                    }
                }
            }
        }
    } else {
        let content = std::fs::read_to_string(&log_path).unwrap_or_default();
        let all_lines: Vec<&str> = content.lines().collect();
        let start = all_lines.len().saturating_sub(lines);
        for line in &all_lines[start..] {
            println!("{line}");
        }
    }
}

fn cmd_health(json: bool) {
    match find_daemon() {
        Some(base) => {
            let client = daemon_client();
            let body = daemon_json(client.get(format!("{base}/api/health")).send());
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&body).unwrap_or_default()
                );
                return;
            }
            ui::success("Daemon is healthy");
            if let Some(status) = body["status"].as_str() {
                ui::kv("Status", status);
            }
            if let Some(uptime) = body.get("uptime_secs").and_then(|v| v.as_u64()) {
                let hours = uptime / 3600;
                let mins = (uptime % 3600) / 60;
                ui::kv("Uptime", &format!("{hours}h {mins}m"));
            }
        }
        None => {
            if json {
                println!("{}", serde_json::json!({"error": "daemon not running"}));
                std::process::exit(1);
            }
            ui::error("Daemon is not running.");
            ui::hint("Start it with: skipper start");
            std::process::exit(1);
        }
    }
}

fn cmd_security_status(json: bool) {
    let base = require_daemon("security status");
    let client = daemon_client();
    let body = daemon_json(client.get(format!("{base}/api/health/detail")).send());
    if json {
        let data = serde_json::json!({
            "audit_trail": "merkle_hash_chain_sha256",
            "taint_tracking": "information_flow_labels",
            "wasm_sandbox": "dual_metering_fuel_epoch",
            "wire_protocol": "ofp_hmac_sha256_mutual_auth",
            "api_keys": "zeroizing_auto_wipe",
            "manifests": "ed25519_signed",
            "agent_count": body.get("agent_count").and_then(|v| v.as_u64()),
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&data).unwrap_or_default()
        );
        return;
    }
    ui::section("Security Status");
    ui::blank();
    ui::kv("Audit trail", "Merkle hash chain (SHA-256)");
    ui::kv("Taint tracking", "Information flow labels");
    ui::kv("WASM sandbox", "Dual metering (fuel + epoch)");
    ui::kv("Wire protocol", "OFP HMAC-SHA256 mutual auth");
    ui::kv("API keys", "Zeroizing<String> (auto-wipe on drop)");
    ui::kv("Manifests", "Ed25519 signed");
    if let Some(agents) = body.get("agent_count").and_then(|v| v.as_u64()) {
        ui::kv("Active agents", &agents.to_string());
    }
}

fn cmd_security_audit(limit: usize, json: bool) {
    let base = require_daemon("security audit");
    let client = daemon_client();
    let body = daemon_json(
        client
            .get(format!("{base}/api/audit/recent?limit={limit}"))
            .send(),
    );
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
        return;
    }
    if let Some(arr) = body.as_array() {
        if arr.is_empty() {
            println!("No audit entries.");
            return;
        }
        println!("{:<24} {:<16} {:<12} EVENT", "TIMESTAMP", "AGENT", "TYPE");
        println!("{}", "-".repeat(80));
        for entry in arr {
            println!(
                "{:<24} {:<16} {:<12} {}",
                entry["timestamp"].as_str().unwrap_or("?"),
                entry["agent_name"].as_str().unwrap_or("?"),
                entry["event_type"].as_str().unwrap_or("?"),
                entry["description"].as_str().unwrap_or(""),
            );
        }
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
    }
}

fn cmd_security_verify() {
    let base = require_daemon("security verify");
    let client = daemon_client();
    let body = daemon_json(client.get(format!("{base}/api/audit/verify")).send());
    if body["valid"].as_bool().unwrap_or(false) {
        ui::success("Audit trail integrity verified (Merkle chain valid).");
    } else {
        ui::error("Audit trail integrity check FAILED.");
        if let Some(msg) = body["error"].as_str() {
            ui::hint(msg);
        }
        std::process::exit(1);
    }
}

fn cmd_memory_list(agent: &str, json: bool) {
    let base = require_daemon("memory list");
    let client = daemon_client();
    let body = daemon_json(
        client
            .get(format!("{base}/api/memory/agents/{agent}/kv"))
            .send(),
    );
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
        return;
    }
    if let Some(arr) = body.as_array() {
        if arr.is_empty() {
            println!("No memory entries for agent '{agent}'.");
            return;
        }
        println!("{:<30} VALUE", "KEY");
        println!("{}", "-".repeat(60));
        for kv in arr {
            println!(
                "{:<30} {}",
                kv["key"].as_str().unwrap_or("?"),
                kv["value"]
                    .as_str()
                    .unwrap_or("")
                    .chars()
                    .take(50)
                    .collect::<String>(),
            );
        }
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
    }
}

fn cmd_memory_get(agent: &str, key: &str, json: bool) {
    let base = require_daemon("memory get");
    let client = daemon_client();
    let body = daemon_json(
        client
            .get(format!("{base}/api/memory/agents/{agent}/kv/{key}"))
            .send(),
    );
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
        return;
    }
    if let Some(val) = body["value"].as_str() {
        println!("{val}");
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
    }
}

fn cmd_memory_set(agent: &str, key: &str, value: &str) {
    let base = require_daemon("memory set");
    let client = daemon_client();
    let body = daemon_json(
        client
            .put(format!("{base}/api/memory/agents/{agent}/kv/{key}"))
            .json(&serde_json::json!({"value": value}))
            .send(),
    );
    if body.get("error").is_some() {
        ui::error(&format!(
            "Failed: {}",
            body["error"].as_str().unwrap_or("?")
        ));
    } else {
        ui::success(&format!("Set {key} for agent '{agent}'."));
    }
}

fn cmd_memory_delete(agent: &str, key: &str) {
    let base = require_daemon("memory delete");
    let client = daemon_client();
    let body = daemon_json(
        client
            .delete(format!("{base}/api/memory/agents/{agent}/kv/{key}"))
            .send(),
    );
    if body.get("error").is_some() {
        ui::error(&format!(
            "Failed: {}",
            body["error"].as_str().unwrap_or("?")
        ));
    } else {
        ui::success(&format!("Deleted key '{key}' for agent '{agent}'."));
    }
}

fn cmd_devices_list(json: bool) {
    let base = require_daemon("devices list");
    let client = daemon_client();
    let body = daemon_json(client.get(format!("{base}/api/pairing/devices")).send());
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
        return;
    }
    if let Some(arr) = body.as_array() {
        if arr.is_empty() {
            println!("No paired devices.");
            return;
        }
        println!("{:<38} {:<20} LAST SEEN", "ID", "NAME");
        println!("{}", "-".repeat(70));
        for d in arr {
            println!(
                "{:<38} {:<20} {}",
                d["id"].as_str().unwrap_or("?"),
                d["name"].as_str().unwrap_or("?"),
                d["last_seen"].as_str().unwrap_or("?"),
            );
        }
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
    }
}

fn cmd_devices_pair() {
    let base = require_daemon("qr");
    let client = daemon_client();
    let body = daemon_json(client.post(format!("{base}/api/pairing/request")).send());
    if let Some(qr) = body["qr_data"].as_str() {
        ui::section("Device Pairing");
        ui::blank();
        // Render a simple text-based QR representation
        println!("  Scan this QR code with the Skipper mobile app:");
        ui::blank();
        println!("  {qr}");
        ui::blank();
        if let Some(code) = body["pairing_code"].as_str() {
            ui::kv("Pairing code", code);
        }
        if let Some(expires) = body["expires_at"].as_str() {
            ui::kv("Expires", expires);
        }
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
    }
}

fn cmd_devices_remove(id: &str) {
    let base = require_daemon("devices remove");
    let client = daemon_client();
    let body = daemon_json(
        client
            .delete(format!("{base}/api/pairing/devices/{id}"))
            .send(),
    );
    if body.get("error").is_some() {
        ui::error(&format!(
            "Failed: {}",
            body["error"].as_str().unwrap_or("?")
        ));
    } else {
        ui::success(&format!("Device {id} removed."));
    }
}

fn cmd_webhooks_list(json: bool) {
    let base = require_daemon("webhooks list");
    let client = daemon_client();
    let body = daemon_json(client.get(format!("{base}/api/triggers")).send());
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
        return;
    }
    if let Some(arr) = body.as_array() {
        if arr.is_empty() {
            println!("No webhooks configured.");
            return;
        }
        println!("{:<38} {:<16} URL", "ID", "AGENT");
        println!("{}", "-".repeat(80));
        for w in arr {
            println!(
                "{:<38} {:<16} {}",
                w["id"].as_str().unwrap_or("?"),
                w["agent_id"].as_str().unwrap_or("?"),
                w["url"].as_str().unwrap_or(""),
            );
        }
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
    }
}

fn cmd_webhooks_create(agent: &str, url: &str) {
    let base = require_daemon("webhooks create");
    let client = daemon_client();
    let body = daemon_json(
        client
            .post(format!("{base}/api/triggers"))
            .json(&serde_json::json!({
                "agent_id": agent,
                "pattern": {"webhook": {"url": url}},
                "prompt_template": "Webhook event: {{event}}",
            }))
            .send(),
    );
    if let Some(id) = body["id"].as_str() {
        ui::success(&format!("Webhook created: {id}"));
    } else {
        ui::error(&format!(
            "Failed: {}",
            body["error"].as_str().unwrap_or("?")
        ));
    }
}

fn cmd_webhooks_delete(id: &str) {
    let base = require_daemon("webhooks delete");
    let client = daemon_client();
    let body = daemon_json(client.delete(format!("{base}/api/triggers/{id}")).send());
    if body.get("error").is_some() {
        ui::error(&format!(
            "Failed: {}",
            body["error"].as_str().unwrap_or("?")
        ));
    } else {
        ui::success(&format!("Webhook {id} deleted."));
    }
}

fn cmd_webhooks_test(id: &str) {
    let base = require_daemon("webhooks test");
    let client = daemon_client();
    let body = daemon_json(client.post(format!("{base}/api/triggers/{id}/test")).send());
    if body["success"].as_bool().unwrap_or(false) {
        ui::success(&format!("Webhook {id} test payload sent successfully."));
    } else {
        ui::error(&format!(
            "Webhook test failed: {}",
            body["error"].as_str().unwrap_or("?")
        ));
    }
}

fn cmd_message(agent: &str, text: &str, json: bool) {
    let base = require_daemon("message");
    let client = daemon_client();
    let body = daemon_json(
        client
            .post(format!("{base}/api/agents/{agent}/message"))
            .json(&serde_json::json!({"message": text}))
            .send(),
    );
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
    } else if let Some(reply) = body["reply"].as_str() {
        println!("{reply}");
    } else if let Some(reply) = body["response"].as_str() {
        println!("{reply}");
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
    }
}

fn cmd_system_info(json: bool) {
    if let Some(base) = find_daemon() {
        let client = daemon_client();
        let body = daemon_json(client.get(format!("{base}/api/status")).send());
        if json {
            let mut data = body.clone();
            if let Some(obj) = data.as_object_mut() {
                obj.insert(
                    "version".to_string(),
                    serde_json::json!(env!("CARGO_PKG_VERSION")),
                );
                obj.insert("api_url".to_string(), serde_json::json!(base));
            }
            println!(
                "{}",
                serde_json::to_string_pretty(&data).unwrap_or_default()
            );
            return;
        }
        ui::section("Skipper System Info");
        ui::blank();
        ui::kv("Version", env!("CARGO_PKG_VERSION"));
        ui::kv("Status", body["status"].as_str().unwrap_or("?"));
        ui::kv(
            "Agents",
            &body["agent_count"].as_u64().unwrap_or(0).to_string(),
        );
        ui::kv("Provider", body["default_provider"].as_str().unwrap_or("?"));
        ui::kv("Model", body["default_model"].as_str().unwrap_or("?"));
        ui::kv("API", &base);
        ui::kv("Data dir", body["data_dir"].as_str().unwrap_or("?"));
        ui::kv(
            "Uptime",
            &format!("{}s", body["uptime_seconds"].as_u64().unwrap_or(0)),
        );
    } else {
        if json {
            println!(
                "{}",
                serde_json::json!({
                    "version": env!("CARGO_PKG_VERSION"),
                    "daemon": "not_running",
                })
            );
            return;
        }
        ui::section("Skipper System Info");
        ui::blank();
        ui::kv("Version", env!("CARGO_PKG_VERSION"));
        ui::kv_warn("Daemon", "NOT RUNNING");
        ui::hint("Start with: skipper start");
    }
}

fn cmd_system_version(json: bool) {
    if json {
        println!(
            "{}",
            serde_json::json!({"version": env!("CARGO_PKG_VERSION")})
        );
        return;
    }
    println!("skipper {}", env!("CARGO_PKG_VERSION"));
}

fn cmd_reset(confirm: bool) {
    let skipper_dir = match dirs::home_dir() {
        Some(h) => h.join(".skipper"),
        None => {
            ui::error("Could not determine home directory");
            std::process::exit(1);
        }
    };

    if !skipper_dir.exists() {
        println!(
            "Nothing to reset — {} does not exist.",
            skipper_dir.display()
        );
        return;
    }

    if !confirm {
        println!("  This will delete all data in {}", skipper_dir.display());
        println!("  Including: config, database, agent manifests, credentials.");
        println!();
        let answer = prompt_input("  Are you sure? Type 'yes' to confirm: ");
        if answer.trim() != "yes" {
            println!("  Cancelled.");
            return;
        }
    }

    match std::fs::remove_dir_all(&skipper_dir) {
        Ok(()) => ui::success(&format!("Removed {}", skipper_dir.display())),
        Err(e) => {
            ui::error(&format!("Failed to remove {}: {e}", skipper_dir.display()));
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {

    // --- Doctor command unit tests ---

    #[test]
    fn test_doctor_skill_registry_loads_bundled() {
        let skills_dir = std::env::temp_dir().join("skipper-doctor-test-skills");
        let mut skill_reg = skipper_skills::registry::SkillRegistry::new(skills_dir);
        let count = skill_reg.load_bundled();
        assert!(count > 0, "Should load bundled skills");
        assert_eq!(skill_reg.count(), count);
    }

    #[test]
    fn test_doctor_extension_registry_loads_bundled() {
        let tmp = std::env::temp_dir().join("skipper-doctor-test-ext");
        let _ = std::fs::create_dir_all(&tmp);
        let mut ext_reg = skipper_extensions::registry::IntegrationRegistry::new(&tmp);
        let count = ext_reg.load_bundled();
        assert!(count > 0, "Should load bundled integration templates");
        assert_eq!(ext_reg.template_count(), count);
    }

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
