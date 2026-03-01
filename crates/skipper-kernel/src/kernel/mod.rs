//! SkipperKernel — assembles all subsystems and provides the main API.

use crate::auth::AuthManager;
use crate::background::{self, BackgroundExecutor};
use crate::capabilities::CapabilityManager;
use crate::config::load_config;
use crate::error::{KernelError, KernelResult};
use crate::event_bus::EventBus;
use crate::metering::MeteringEngine;
use crate::registry::AgentRegistry;
use crate::scheduler::AgentScheduler;
use crate::supervisor::Supervisor;
use crate::triggers::{TriggerEngine, TriggerId, TriggerPattern};
use crate::workflow::{StepAgent, Workflow, WorkflowEngine, WorkflowId, WorkflowRunId};

use skipper_memory::MemorySubstrate;
use skipper_runtime::agent_loop::{
    run_agent_loop, run_agent_loop_streaming, strip_provider_prefix, AgentLoopResult,
};
use skipper_runtime::audit::AuditLog;
use skipper_runtime::drivers;
use skipper_runtime::kernel_handle::{self, KernelHandle};
use skipper_runtime::llm_driver::{CompletionRequest, DriverConfig, LlmDriver, StreamEvent};
use skipper_runtime::python_runtime::{self, PythonConfig};
use skipper_runtime::routing::ModelRouter;
use skipper_runtime::sandbox::{SandboxConfig, WasmSandbox};
use skipper_runtime::tool_runner::builtin_tool_definitions;
use skipper_types::agent::*;
use skipper_types::capability::Capability;
use skipper_types::config::KernelConfig;
use skipper_types::error::SkipperError;
use skipper_types::event::*;
use skipper_types::memory::Memory;
use skipper_types::tool::ToolDefinition;

use async_trait::async_trait;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock, Weak};
use tracing::{debug, info, warn};

/// The main Skipper kernel — coordinates all subsystems.
pub struct SkipperKernel {
    /// Kernel configuration.
    pub config: KernelConfig,
    /// Agent registry.
    pub registry: AgentRegistry,
    /// Capability manager.
    pub capabilities: CapabilityManager,
    /// Event bus.
    pub event_bus: EventBus,
    /// Agent scheduler.
    pub scheduler: AgentScheduler,
    /// Memory substrate.
    pub memory: Arc<MemorySubstrate>,
    /// Process supervisor.
    pub supervisor: Supervisor,
    /// Workflow engine.
    pub workflows: WorkflowEngine,
    /// Event-driven trigger engine.
    pub triggers: TriggerEngine,
    /// Background agent executor.
    pub background: BackgroundExecutor,
    /// Merkle hash chain audit trail.
    pub audit_log: Arc<AuditLog>,
    /// Cost metering engine.
    pub metering: Arc<MeteringEngine>,
    /// Default LLM driver (from kernel config).
    default_driver: Arc<dyn LlmDriver>,
    /// WASM sandbox engine (shared across all WASM agent executions).
    wasm_sandbox: WasmSandbox,
    /// RBAC authentication manager.
    pub auth: AuthManager,
    /// Model catalog registry (RwLock for auth status refresh from API).
    pub model_catalog: std::sync::RwLock<skipper_runtime::model_catalog::ModelCatalog>,
    /// Skill registry for plugin skills (RwLock for hot-reload on install/uninstall).
    pub skill_registry: std::sync::RwLock<skipper_skills::registry::SkillRegistry>,
    /// Tracks running agent tasks for cancellation support.
    pub running_tasks: dashmap::DashMap<AgentId, tokio::task::AbortHandle>,
    /// MCP server connections (lazily initialized at start_background_agents).
    pub mcp_connections: tokio::sync::Mutex<Vec<skipper_runtime::mcp::McpConnection>>,
    /// MCP tool definitions cache (populated after connections are established).
    pub mcp_tools: std::sync::Mutex<Vec<ToolDefinition>>,
    /// A2A task store for tracking task lifecycle.
    pub a2a_task_store: skipper_runtime::a2a::A2aTaskStore,
    /// Discovered external A2A agent cards.
    pub a2a_external_agents: std::sync::Mutex<Vec<(String, skipper_runtime::a2a::AgentCard)>>,
    /// Web tools context (multi-provider search + SSRF-protected fetch + caching).
    pub web_ctx: skipper_runtime::web_search::WebToolsContext,
    /// Browser automation manager (Playwright bridge sessions).
    pub browser_ctx: skipper_runtime::browser::BrowserManager,
    /// Media understanding engine (image description, audio transcription).
    pub media_engine: skipper_runtime::media_understanding::MediaEngine,
    /// Text-to-speech engine.
    pub tts_engine: skipper_runtime::tts::TtsEngine,
    /// Device pairing manager.
    pub pairing: crate::pairing::PairingManager,
    /// Embedding driver for vector similarity search (None = text fallback).
    pub embedding_driver:
        Option<Arc<dyn skipper_runtime::embedding::EmbeddingDriver + Send + Sync>>,
    /// Hand registry — curated autonomous capability packages.
    pub hand_registry: skipper_hands::registry::HandRegistry,
    /// Extension/integration registry (bundled MCP templates + install state).
    pub extension_registry: std::sync::RwLock<skipper_extensions::registry::IntegrationRegistry>,
    /// Integration health monitor.
    pub extension_health: skipper_extensions::health::HealthMonitor,
    /// Effective MCP server list (manual config + extension-installed, merged at boot).
    pub effective_mcp_servers: std::sync::RwLock<Vec<skipper_types::config::McpServerConfigEntry>>,
    /// Delivery receipt tracker (bounded LRU, max 10K entries).
    pub delivery_tracker: DeliveryTracker,
    /// Cron job scheduler.
    pub cron_scheduler: crate::cron::CronScheduler,
    /// Execution approval manager.
    pub approval_manager: crate::approval::ApprovalManager,
    /// Agent bindings for multi-account routing (Mutex for runtime add/remove).
    pub bindings: std::sync::Mutex<Vec<skipper_types::config::AgentBinding>>,
    /// Broadcast configuration.
    pub broadcast: skipper_types::config::BroadcastConfig,
    /// Auto-reply engine.
    pub auto_reply_engine: crate::auto_reply::AutoReplyEngine,
    /// Plugin lifecycle hook registry.
    pub hooks: skipper_runtime::hooks::HookRegistry,
    /// Persistent process manager for interactive sessions (REPLs, servers).
    pub process_manager: Arc<skipper_runtime::process_manager::ProcessManager>,
    /// OFP peer registry — tracks connected peers.
    pub peer_registry: Option<skipper_wire::PeerRegistry>,
    /// OFP peer node — the local networking node.
    pub peer_node: Option<Arc<skipper_wire::PeerNode>>,
    /// Boot timestamp for uptime calculation.
    pub booted_at: std::time::Instant,
    /// WhatsApp Web gateway child process PID (for shutdown cleanup).
    pub whatsapp_gateway_pid: Arc<std::sync::Mutex<Option<u32>>>,
    /// Channel adapters registered at bridge startup (for proactive `channel_send` tool).
    pub channel_adapters: dashmap::DashMap<String, Arc<dyn skipper_channels::types::ChannelAdapter>>,
    /// Weak self-reference for trigger dispatch (set after Arc wrapping).
    self_handle: OnceLock<Weak<SkipperKernel>>,
}

/// Bounded in-memory delivery receipt tracker.
/// Stores up to `MAX_RECEIPTS` most recent delivery receipts per agent.
pub struct DeliveryTracker {
    receipts: dashmap::DashMap<AgentId, Vec<skipper_channels::types::DeliveryReceipt>>,
}

impl Default for DeliveryTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl DeliveryTracker {
    const MAX_RECEIPTS: usize = 10_000;
    const MAX_PER_AGENT: usize = 500;

    /// Create a new empty delivery tracker.
    pub fn new() -> Self {
        Self {
            receipts: dashmap::DashMap::new(),
        }
    }

    /// Record a delivery receipt for an agent.
    pub fn record(&self, agent_id: AgentId, receipt: skipper_channels::types::DeliveryReceipt) {
        let mut entry = self.receipts.entry(agent_id).or_default();
        entry.push(receipt);
        // Per-agent cap
        if entry.len() > Self::MAX_PER_AGENT {
            let drain = entry.len() - Self::MAX_PER_AGENT;
            entry.drain(..drain);
        }
        // Global cap: evict oldest agents' receipts if total exceeds limit
        drop(entry);
        let total: usize = self.receipts.iter().map(|e| e.value().len()).sum();
        if total > Self::MAX_RECEIPTS {
            // Simple eviction: remove oldest entries from first agent found
            if let Some(mut oldest) = self.receipts.iter_mut().next() {
                let to_remove = total - Self::MAX_RECEIPTS;
                let drain = to_remove.min(oldest.value().len());
                oldest.value_mut().drain(..drain);
            }
        }
    }

    /// Get recent delivery receipts for an agent (newest first).
    pub fn get_receipts(
        &self,
        agent_id: AgentId,
        limit: usize,
    ) -> Vec<skipper_channels::types::DeliveryReceipt> {
        self.receipts
            .get(&agent_id)
            .map(|entries| entries.iter().rev().take(limit).cloned().collect())
            .unwrap_or_default()
    }

    /// Create a receipt for a successful send.
    pub fn sent_receipt(
        channel: &str,
        recipient: &str,
    ) -> skipper_channels::types::DeliveryReceipt {
        skipper_channels::types::DeliveryReceipt {
            message_id: uuid::Uuid::new_v4().to_string(),
            channel: channel.to_string(),
            recipient: Self::sanitize_recipient(recipient),
            status: skipper_channels::types::DeliveryStatus::Sent,
            timestamp: chrono::Utc::now(),
            error: None,
        }
    }

    /// Create a receipt for a failed send.
    pub fn failed_receipt(
        channel: &str,
        recipient: &str,
        error: &str,
    ) -> skipper_channels::types::DeliveryReceipt {
        skipper_channels::types::DeliveryReceipt {
            message_id: uuid::Uuid::new_v4().to_string(),
            channel: channel.to_string(),
            recipient: Self::sanitize_recipient(recipient),
            status: skipper_channels::types::DeliveryStatus::Failed,
            timestamp: chrono::Utc::now(),
            // Sanitize error: no credentials, max 256 chars
            error: Some(
                error
                    .chars()
                    .take(256)
                    .collect::<String>()
                    .replace(|c: char| c.is_control(), ""),
            ),
        }
    }

    /// Sanitize recipient to avoid PII logging.
    fn sanitize_recipient(recipient: &str) -> String {
        let s: String = recipient
            .chars()
            .filter(|c| !c.is_control())
            .take(64)
            .collect();
        s
    }
}

/// Create workspace directory structure for an agent.
fn ensure_workspace(workspace: &Path) -> KernelResult<()> {
    for subdir in &["data", "output", "sessions", "skills", "logs", "memory"] {
        std::fs::create_dir_all(workspace.join(subdir)).map_err(|e| {
            KernelError::Skipper(SkipperError::Internal(format!(
                "Failed to create workspace dir {}/{subdir}: {e}",
                workspace.display()
            )))
        })?;
    }
    // Write agent metadata file (best-effort)
    let meta = serde_json::json!({
        "created_at": chrono::Utc::now().to_rfc3339(),
        "workspace": workspace.display().to_string(),
    });
    let _ = std::fs::write(
        workspace.join("AGENT.json"),
        serde_json::to_string_pretty(&meta).unwrap_or_default(),
    );
    Ok(())
}

/// Generate workspace identity files for an agent (SOUL.md, USER.md, TOOLS.md, MEMORY.md).
/// Uses `create_new` to never overwrite existing files (preserves user edits).
fn generate_identity_files(workspace: &Path, manifest: &AgentManifest) {
    use std::fs::OpenOptions;
    use std::io::Write;

    let soul_content = format!(
        "# Soul\n\
         You are {}. {}\n\
         Be genuinely helpful. Have opinions. Be resourceful before asking.\n\
         Treat user data with respect \u{2014} you are a guest in their life.\n",
        manifest.name,
        if manifest.description.is_empty() {
            "You are a helpful AI agent."
        } else {
            &manifest.description
        }
    );

    let user_content = "# User\n\
         <!-- Updated by the agent as it learns about the user -->\n\
         - Name:\n\
         - Timezone:\n\
         - Preferences:\n";

    let tools_content = "# Tools & Environment\n\
         <!-- Agent-specific environment notes (not synced) -->\n";

    let memory_content = "# Long-Term Memory\n\
         <!-- Curated knowledge the agent preserves across sessions -->\n";

    let agents_content = "# Agent Behavioral Guidelines\n\n\
         ## Core Principles\n\
         - Act first, narrate second. Use tools to accomplish tasks rather than describing what you'd do.\n\
         - Batch tool calls when possible \u{2014} don't output reasoning between each call.\n\
         - When a task is ambiguous, ask ONE clarifying question, not five.\n\
         - Store important context in memory (memory_store) proactively.\n\
         - Search memory (memory_recall) before asking the user for context they may have given before.\n\n\
         ## Tool Usage Protocols\n\
         - file_read BEFORE file_write \u{2014} always understand what exists.\n\
         - web_search for current info, web_fetch for specific URLs.\n\
         - browser_* for interactive sites that need clicks/forms.\n\
         - shell_exec: explain destructive commands before running.\n\n\
         ## Response Style\n\
         - Lead with the answer or result, not process narration.\n\
         - Keep responses concise unless the user asks for detail.\n\
         - Use formatting (headers, lists, code blocks) for readability.\n\
         - If a task fails, explain what went wrong and suggest alternatives.\n";

    let bootstrap_content = format!(
        "# First-Run Bootstrap\n\n\
         On your FIRST conversation with a new user, follow this protocol:\n\n\
         1. **Greet** \u{2014} Introduce yourself as {name} with a one-line summary of your specialty.\n\
         2. **Discover** \u{2014} Ask the user's name and one key preference relevant to your domain.\n\
         3. **Store** \u{2014} Use memory_store to save: user_name, their preference, and today's date as first_interaction.\n\
         4. **Orient** \u{2014} Briefly explain what you can help with (2-3 bullet points, not a wall of text).\n\
         5. **Serve** \u{2014} If the user included a request in their first message, handle it immediately after steps 1-3.\n\n\
         After bootstrap, this protocol is complete. Focus entirely on the user's needs.\n",
        name = manifest.name
    );

    let identity_content = format!(
        "---\n\
         name: {name}\n\
         archetype: assistant\n\
         vibe: helpful\n\
         emoji:\n\
         avatar_url:\n\
         greeting_style: warm\n\
         color:\n\
         ---\n\
         # Identity\n\
         <!-- Visual identity and personality at a glance. Edit these fields freely. -->\n",
        name = manifest.name
    );

    let files: &[(&str, &str)] = &[
        ("SOUL.md", &soul_content),
        ("USER.md", user_content),
        ("TOOLS.md", tools_content),
        ("MEMORY.md", memory_content),
        ("AGENTS.md", agents_content),
        ("BOOTSTRAP.md", &bootstrap_content),
        ("IDENTITY.md", &identity_content),
    ];

    // Conditionally generate HEARTBEAT.md for autonomous agents
    let heartbeat_content = if manifest.autonomous.is_some() {
        Some(
            "# Heartbeat Checklist\n\
             <!-- Proactive reminders to check during heartbeat cycles -->\n\n\
             ## Every Heartbeat\n\
             - [ ] Check for pending tasks or messages\n\
             - [ ] Review memory for stale items\n\n\
             ## Daily\n\
             - [ ] Summarize today's activity for the user\n\n\
             ## Weekly\n\
             - [ ] Archive old sessions and clean up memory\n"
                .to_string(),
        )
    } else {
        None
    };

    for (filename, content) in files {
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(workspace.join(filename))
        {
            Ok(mut f) => {
                let _ = f.write_all(content.as_bytes());
            }
            Err(_) => {
                // File already exists — preserve user edits
            }
        }
    }

    // Write HEARTBEAT.md for autonomous agents
    if let Some(ref hb) = heartbeat_content {
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(workspace.join("HEARTBEAT.md"))
        {
            Ok(mut f) => {
                let _ = f.write_all(hb.as_bytes());
            }
            Err(_) => {
                // File already exists — preserve user edits
            }
        }
    }
}

/// Append an assistant response summary to the daily memory log (best-effort, append-only).
/// Caps daily log at 1MB to prevent unbounded growth.
fn append_daily_memory_log(workspace: &Path, response: &str) {
    use std::io::Write;
    let trimmed = response.trim();
    if trimmed.is_empty() {
        return;
    }
    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let log_path = workspace.join("memory").join(format!("{today}.md"));
    // Security: cap total daily log to 1MB
    if let Ok(metadata) = std::fs::metadata(&log_path) {
        if metadata.len() > 1_048_576 {
            return;
        }
    }
    // Truncate long responses for the log (UTF-8 safe)
    let summary = skipper_types::truncate_str(trimmed, 500);
    let timestamp = chrono::Utc::now().format("%H:%M:%S").to_string();
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
    {
        let _ = writeln!(f, "\n## {timestamp}\n{summary}\n");
    }
}

/// Read a workspace identity file with a size cap to prevent prompt stuffing.
/// Returns None if the file doesn't exist or is empty.
fn read_identity_file(workspace: &Path, filename: &str) -> Option<String> {
    const MAX_IDENTITY_FILE_BYTES: usize = 32_768; // 32KB cap
    let path = workspace.join(filename);
    // Security: ensure path stays inside workspace
    match path.canonicalize() {
        Ok(canonical) => {
            if let Ok(ws_canonical) = workspace.canonicalize() {
                if !canonical.starts_with(&ws_canonical) {
                    return None; // path traversal attempt
                }
            }
        }
        Err(_) => return None, // file doesn't exist
    }
    let content = std::fs::read_to_string(&path).ok()?;
    if content.trim().is_empty() {
        return None;
    }
    if content.len() > MAX_IDENTITY_FILE_BYTES {
        Some(content[..MAX_IDENTITY_FILE_BYTES].to_string())
    } else {
        Some(content)
    }
}

/// Get the system hostname as a String.
fn gethostname() -> Option<String> {
    #[cfg(unix)]
    {
        std::process::Command::new("hostname")
            .output()
            .ok()
            .and_then(|out| String::from_utf8(out.stdout).ok())
            .map(|s| s.trim().to_string())
    }
    #[cfg(windows)]
    {
        std::env::var("COMPUTERNAME").ok()
    }
    #[cfg(not(any(unix, windows)))]
    {
        None
    }
}

mod agents;
pub use agents::{manifest_to_capabilities, infer_provider_from_model};

mod messaging;
mod sessions;
mod config;
mod hands;

mod background_impl;
pub use background_impl::cron_deliver_response;

mod networking;
mod tools;
mod handle_impl;
pub use handle_impl::shared_memory_agent_id;

impl SkipperKernel {
    /// Boot the kernel with configuration from the given path.
    pub fn boot(config_path: Option<&Path>) -> KernelResult<Self> {
        let config = load_config(config_path);
        Self::boot_with_config(config)
    }

    /// Boot the kernel with an explicit configuration.
    pub fn boot_with_config(mut config: KernelConfig) -> KernelResult<Self> {
        use skipper_types::config::KernelMode;

        // Env var overrides — useful for Docker where config.toml is baked in.
        if let Ok(listen) = std::env::var("SKIPPER_LISTEN") {
            config.api_listen = listen;
        }

        // Clamp configuration bounds to prevent zero-value or unbounded misconfigs
        config.clamp_bounds();

        match config.mode {
            KernelMode::Stable => {
                info!("Booting Skipper kernel in STABLE mode — conservative defaults enforced");
            }
            KernelMode::Dev => {
                warn!("Booting Skipper kernel in DEV mode — experimental features enabled");
            }
            KernelMode::Default => {
                info!("Booting Skipper kernel...");
            }
        }

        // Validate configuration and log warnings
        let warnings = config.validate();
        for w in &warnings {
            warn!("Config: {}", w);
        }

        // Ensure data directory exists
        std::fs::create_dir_all(&config.data_dir)
            .map_err(|e| KernelError::BootFailed(format!("Failed to create data dir: {e}")))?;

        // Initialize memory substrate
        let db_path = config
            .memory
            .sqlite_path
            .clone()
            .unwrap_or_else(|| config.data_dir.join("skipper.db"));
        let memory = Arc::new(
            MemorySubstrate::open(&db_path, config.memory.decay_rate)
                .map_err(|e| KernelError::BootFailed(format!("Memory init failed: {e}")))?,
        );

        // Create LLM driver
        let mut provider = config.default_model.provider.clone();
        let mut api_key = std::env::var(&config.default_model.api_key_env)
            .ok()
            .filter(|v| !v.is_empty());
        let mut base_url = config.default_model.base_url.clone();

        // Auto-detect Claude Code fallback: if the provider is "anthropic" and the
        // resolved key is an OAuth token (sk-ant-oat*), those tokens cannot call
        // api.anthropic.com directly. Switch to the claude-code CLI driver instead.
        if provider == "anthropic" {
            let is_oauth_token = api_key.as_ref().is_some_and(|k| k.starts_with("sk-ant-oat"));
            if (is_oauth_token || api_key.is_none())
                && drivers::claude_code::ClaudeCodeDriver::detect().is_some()
            {
                info!("OAuth token detected or no API key — using Claude Code CLI as LLM backend");
                provider = "claude-code".to_string();
                api_key = None;
                base_url = None;
            }
        }

        let driver_config = DriverConfig {
            provider,
            api_key,
            base_url,
        };
        let primary_driver = drivers::create_driver(&driver_config)
            .map_err(|e| KernelError::BootFailed(format!("LLM driver init failed: {e}")))?;

        // If fallback providers are configured, wrap the primary driver in a FallbackDriver
        let driver: Arc<dyn LlmDriver> = if !config.fallback_providers.is_empty() {
            let mut chain: Vec<Arc<dyn LlmDriver>> = vec![primary_driver.clone()];
            for fb in &config.fallback_providers {
                let fb_config = DriverConfig {
                    provider: fb.provider.clone(),
                    api_key: if fb.api_key_env.is_empty() {
                        None
                    } else {
                        std::env::var(&fb.api_key_env).ok()
                    },
                    base_url: fb.base_url.clone(),
                };
                match drivers::create_driver(&fb_config) {
                    Ok(d) => {
                        info!(
                            provider = %fb.provider,
                            model = %fb.model,
                            "Fallback provider configured"
                        );
                        chain.push(d);
                    }
                    Err(e) => {
                        warn!(
                            provider = %fb.provider,
                            error = %e,
                            "Fallback provider init failed — skipped"
                        );
                    }
                }
            }
            if chain.len() > 1 {
                Arc::new(skipper_runtime::drivers::fallback::FallbackDriver::new(
                    chain,
                ))
            } else {
                primary_driver
            }
        } else {
            primary_driver
        };

        // Initialize metering engine (shares the same SQLite connection as the memory substrate)
        let metering = Arc::new(MeteringEngine::new(Arc::new(
            skipper_memory::usage::UsageStore::new(memory.usage_conn()),
        )));

        let supervisor = Supervisor::new();
        let background = BackgroundExecutor::new(supervisor.subscribe());

        // Initialize WASM sandbox engine (shared across all WASM agents)
        let wasm_sandbox = WasmSandbox::new()
            .map_err(|e| KernelError::BootFailed(format!("WASM sandbox init failed: {e}")))?;

        // Initialize RBAC authentication manager
        let auth = AuthManager::new(&config.users);
        if auth.is_enabled() {
            info!("RBAC enabled with {} users", auth.user_count());
        }

        // Initialize model catalog, detect provider auth, and apply URL overrides
        let mut model_catalog = skipper_runtime::model_catalog::ModelCatalog::new();
        model_catalog.detect_auth();
        if !config.provider_urls.is_empty() {
            model_catalog.apply_url_overrides(&config.provider_urls);
            info!(
                "applied {} provider URL override(s)",
                config.provider_urls.len()
            );
        }
        // Load user's custom models from ~/.skipper/custom_models.json
        let custom_models_path = config.home_dir.join("custom_models.json");
        model_catalog.load_custom_models(&custom_models_path);
        let available_count = model_catalog.available_models().len();
        let total_count = model_catalog.list_models().len();
        let local_count = model_catalog
            .list_providers()
            .iter()
            .filter(|p| !p.key_required)
            .count();
        info!(
            "Model catalog: {total_count} models, {available_count} available from configured providers ({local_count} local)"
        );

        // Initialize skill registry
        let skills_dir = config.home_dir.join("skills");
        let mut skill_registry = skipper_skills::registry::SkillRegistry::new(skills_dir);

        // Load bundled skills first (compile-time embedded)
        let bundled_count = skill_registry.load_bundled();
        if bundled_count > 0 {
            info!("Loaded {bundled_count} bundled skill(s)");
        }

        // Load user-installed skills (overrides bundled ones with same name)
        match skill_registry.load_all() {
            Ok(count) => {
                if count > 0 {
                    info!("Loaded {count} user skill(s) from skill registry");
                }
            }
            Err(e) => {
                warn!("Failed to load skill registry: {e}");
            }
        }
        // In Stable mode, freeze the skill registry
        if config.mode == KernelMode::Stable {
            skill_registry.freeze();
        }

        // Initialize hand registry (curated autonomous packages)
        let mut hand_registry = skipper_hands::registry::HandRegistry::new();
        let hand_count = hand_registry.load_bundled();
        if hand_count > 0 {
            info!("Loaded {hand_count} bundled hand(s)");
        }

        // Initialize extension/integration registry
        let mut extension_registry =
            skipper_extensions::registry::IntegrationRegistry::new(&config.home_dir);
        let ext_bundled = extension_registry.load_bundled();
        match extension_registry.load_installed() {
            Ok(count) => {
                if count > 0 {
                    info!("Loaded {count} installed integration(s)");
                }
            }
            Err(e) => {
                warn!("Failed to load installed integrations: {e}");
            }
        }
        info!(
            "Extension registry: {ext_bundled} templates available, {} installed",
            extension_registry.installed_count()
        );

        // Merge installed integrations into MCP server list
        let ext_mcp_configs = extension_registry.to_mcp_configs();
        let mut all_mcp_servers = config.mcp_servers.clone();
        for ext_cfg in ext_mcp_configs {
            // Avoid duplicates — don't add if a manual config already exists with same name
            if !all_mcp_servers.iter().any(|s| s.name == ext_cfg.name) {
                all_mcp_servers.push(ext_cfg);
            }
        }

        // Initialize integration health monitor
        let health_config = skipper_extensions::health::HealthMonitorConfig {
            auto_reconnect: config.extensions.auto_reconnect,
            max_reconnect_attempts: config.extensions.reconnect_max_attempts,
            max_backoff_secs: config.extensions.reconnect_max_backoff_secs,
            check_interval_secs: config.extensions.health_check_interval_secs,
        };
        let extension_health = skipper_extensions::health::HealthMonitor::new(health_config);
        // Register all installed integrations for health monitoring
        for inst in extension_registry.to_mcp_configs() {
            extension_health.register(&inst.name);
        }

        // Initialize web tools (multi-provider search + SSRF-protected fetch + caching)
        let cache_ttl = std::time::Duration::from_secs(config.web.cache_ttl_minutes * 60);
        let web_cache = Arc::new(skipper_runtime::web_cache::WebCache::new(cache_ttl));
        let web_ctx = skipper_runtime::web_search::WebToolsContext {
            search: skipper_runtime::web_search::WebSearchEngine::new(
                config.web.clone(),
                web_cache.clone(),
            ),
            fetch: skipper_runtime::web_fetch::WebFetchEngine::new(
                config.web.fetch.clone(),
                web_cache,
            ),
        };

        // Auto-detect embedding driver for vector similarity search
        let embedding_driver: Option<
            Arc<dyn skipper_runtime::embedding::EmbeddingDriver + Send + Sync>,
        > = {
            use skipper_runtime::embedding::create_embedding_driver;
            if let Some(ref provider) = config.memory.embedding_provider {
                // Explicit config takes priority
                let api_key_env = config.memory.embedding_api_key_env.as_deref().unwrap_or("");
                match create_embedding_driver(provider, "text-embedding-3-small", api_key_env) {
                    Ok(d) => {
                        info!(provider = %provider, "Embedding driver configured from memory config");
                        Some(Arc::from(d))
                    }
                    Err(e) => {
                        warn!(provider = %provider, error = %e, "Embedding driver init failed — falling back to text search");
                        None
                    }
                }
            } else if std::env::var("OPENAI_API_KEY").is_ok() {
                match create_embedding_driver("openai", "text-embedding-3-small", "OPENAI_API_KEY")
                {
                    Ok(d) => {
                        info!("Embedding driver auto-detected: OpenAI");
                        Some(Arc::from(d))
                    }
                    Err(e) => {
                        warn!(error = %e, "OpenAI embedding auto-detect failed");
                        None
                    }
                }
            } else {
                // Try Ollama (local, no key needed)
                match create_embedding_driver("ollama", "nomic-embed-text", "") {
                    Ok(d) => {
                        info!("Embedding driver auto-detected: Ollama (local)");
                        Some(Arc::from(d))
                    }
                    Err(e) => {
                        debug!("No embedding driver available (Ollama probe failed: {e}) — using text search fallback");
                        None
                    }
                }
            }
        };

        let browser_ctx = skipper_runtime::browser::BrowserManager::new(config.browser.clone());

        // Initialize media understanding engine
        let media_engine =
            skipper_runtime::media_understanding::MediaEngine::new(config.media.clone());
        let tts_engine = skipper_runtime::tts::TtsEngine::new(config.tts.clone());
        let mut pairing = crate::pairing::PairingManager::new(config.pairing.clone());

        // Load paired devices from database and set up persistence callback
        if config.pairing.enabled {
            match memory.load_paired_devices() {
                Ok(rows) => {
                    let devices: Vec<crate::pairing::PairedDevice> = rows
                        .into_iter()
                        .filter_map(|row| {
                            Some(crate::pairing::PairedDevice {
                                device_id: row["device_id"].as_str()?.to_string(),
                                display_name: row["display_name"].as_str()?.to_string(),
                                platform: row["platform"].as_str()?.to_string(),
                                paired_at: chrono::DateTime::parse_from_rfc3339(
                                    row["paired_at"].as_str()?,
                                )
                                .ok()?
                                .with_timezone(&chrono::Utc),
                                last_seen: chrono::DateTime::parse_from_rfc3339(
                                    row["last_seen"].as_str()?,
                                )
                                .ok()?
                                .with_timezone(&chrono::Utc),
                                push_token: row["push_token"].as_str().map(String::from),
                            })
                        })
                        .collect();
                    pairing.load_devices(devices);
                }
                Err(e) => {
                    warn!("Failed to load paired devices from database: {e}");
                }
            }

            let persist_memory = Arc::clone(&memory);
            pairing.set_persist(Box::new(move |device, op| match op {
                crate::pairing::PersistOp::Save => {
                    if let Err(e) = persist_memory.save_paired_device(
                        &device.device_id,
                        &device.display_name,
                        &device.platform,
                        &device.paired_at.to_rfc3339(),
                        &device.last_seen.to_rfc3339(),
                        device.push_token.as_deref(),
                    ) {
                        tracing::warn!("Failed to persist paired device: {e}");
                    }
                }
                crate::pairing::PersistOp::Remove => {
                    if let Err(e) = persist_memory.remove_paired_device(&device.device_id) {
                        tracing::warn!("Failed to remove paired device from DB: {e}");
                    }
                }
            }));
        }

        // Initialize cron scheduler
        let cron_scheduler =
            crate::cron::CronScheduler::new(&config.home_dir, config.max_cron_jobs);
        match cron_scheduler.load() {
            Ok(count) => {
                if count > 0 {
                    info!("Loaded {count} cron job(s) from disk");
                }
            }
            Err(e) => {
                warn!("Failed to load cron jobs: {e}");
            }
        }

        // Initialize execution approval manager
        let approval_manager = crate::approval::ApprovalManager::new(config.approval.clone());

        // Initialize binding/broadcast/auto-reply from config
        let initial_bindings = config.bindings.clone();
        let initial_broadcast = config.broadcast.clone();
        let auto_reply_engine = crate::auto_reply::AutoReplyEngine::new(config.auto_reply.clone());

        let kernel = Self {
            config,
            registry: AgentRegistry::new(),
            capabilities: CapabilityManager::new(),
            event_bus: EventBus::new(),
            scheduler: AgentScheduler::new(),
            memory: memory.clone(),
            supervisor,
            workflows: WorkflowEngine::new(),
            triggers: TriggerEngine::new(),
            background,
            audit_log: Arc::new(AuditLog::new()),
            metering,
            default_driver: driver,
            wasm_sandbox,
            auth,
            model_catalog: std::sync::RwLock::new(model_catalog),
            skill_registry: std::sync::RwLock::new(skill_registry),
            running_tasks: dashmap::DashMap::new(),
            mcp_connections: tokio::sync::Mutex::new(Vec::new()),
            mcp_tools: std::sync::Mutex::new(Vec::new()),
            a2a_task_store: skipper_runtime::a2a::A2aTaskStore::default(),
            a2a_external_agents: std::sync::Mutex::new(Vec::new()),
            web_ctx,
            browser_ctx,
            media_engine,
            tts_engine,
            pairing,
            embedding_driver,
            hand_registry,
            extension_registry: std::sync::RwLock::new(extension_registry),
            extension_health,
            effective_mcp_servers: std::sync::RwLock::new(all_mcp_servers),
            delivery_tracker: DeliveryTracker::new(),
            cron_scheduler,
            approval_manager,
            bindings: std::sync::Mutex::new(initial_bindings),
            broadcast: initial_broadcast,
            auto_reply_engine,
            hooks: skipper_runtime::hooks::HookRegistry::new(),
            process_manager: Arc::new(skipper_runtime::process_manager::ProcessManager::new(5)),
            peer_registry: None,
            peer_node: None,
            booted_at: std::time::Instant::now(),
            whatsapp_gateway_pid: Arc::new(std::sync::Mutex::new(None)),
            channel_adapters: dashmap::DashMap::new(),
            self_handle: OnceLock::new(),
        };

        // Restore persisted agents from SQLite
        match kernel.memory.load_all_agents() {
            Ok(agents) => {
                let count = agents.len();
                for entry in agents {
                    let agent_id = entry.id;
                    let name = entry.name.clone();

                    // Re-grant capabilities
                    let caps = manifest_to_capabilities(&entry.manifest);
                    kernel.capabilities.grant(agent_id, caps);

                    // Re-register with scheduler
                    kernel
                        .scheduler
                        .register(agent_id, entry.manifest.resources.clone());

                    // Re-register in the in-memory registry (set state back to Running)
                    let mut restored_entry = entry;
                    restored_entry.state = AgentState::Running;

                    // Inherit kernel exec_policy for agents that lack one
                    if restored_entry.manifest.exec_policy.is_none() {
                        restored_entry.manifest.exec_policy =
                            Some(kernel.config.exec_policy.clone());
                    }
                    if let Err(e) = kernel.registry.register(restored_entry) {
                        tracing::warn!(agent = %name, "Failed to restore agent: {e}");
                    } else {
                        tracing::debug!(agent = %name, id = %agent_id, "Restored agent");
                    }
                }
                if count > 0 {
                    info!("Restored {count} agent(s) from persistent storage");
                }
            }
            Err(e) => {
                tracing::warn!("Failed to load persisted agents: {e}");
            }
        }

        // Validate routing configs against model catalog
        for entry in kernel.registry.list() {
            if let Some(ref routing_config) = entry.manifest.routing {
                let router = ModelRouter::new(routing_config.clone());
                for warning in router.validate_models(
                    &kernel
                        .model_catalog
                        .read()
                        .unwrap_or_else(|e| e.into_inner()),
                ) {
                    warn!(agent = %entry.name, "{warning}");
                }
            }
        }

        info!("Skipper kernel booted successfully");
        Ok(kernel)
    }


    /// Resolve a module path relative to the kernel's home directory.
    ///
    /// If the path is absolute, return it as-is. Otherwise, resolve relative
    /// to `config.home_dir`.
    fn resolve_module_path(&self, path: &str) -> PathBuf {
        let p = Path::new(path);
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            self.config.home_dir.join(path)
        }
    }


    /// Set the weak self-reference for trigger dispatch.
    ///
    /// Must be called once after the kernel is wrapped in `Arc`.
    pub fn set_self_handle(self: &Arc<Self>) {
        let _ = self.self_handle.set(Arc::downgrade(self));
    }

    // ─── Agent Binding management ──────────────────────────────────────

    /// Publish an event to the bus and evaluate triggers.
    ///
    /// Any matching triggers will dispatch messages to the subscribing agents.
    /// Returns the list of (agent_id, message) pairs that were triggered.
    pub async fn publish_event(&self, event: Event) -> Vec<(AgentId, String)> {
        // Evaluate triggers before publishing (so describe_event works on the event)
        let triggered = self.triggers.evaluate(&event);

        // Publish to the event bus
        self.event_bus.publish(event).await;

        // Actually dispatch triggered messages to agents
        if let Some(weak) = self.self_handle.get() {
            for (agent_id, message) in &triggered {
                if let Some(kernel) = weak.upgrade() {
                    let aid = *agent_id;
                    let msg = message.clone();
                    tokio::spawn(async move {
                        if let Err(e) = kernel.send_message(aid, &msg).await {
                            warn!(agent = %aid, "Trigger dispatch failed: {e}");
                        }
                    });
                }
            }
        }

        triggered
    }

    /// Register a trigger for an agent.
    pub fn register_trigger(
        &self,
        agent_id: AgentId,
        pattern: TriggerPattern,
        prompt_template: String,
        max_fires: u64,
    ) -> KernelResult<TriggerId> {
        // Verify agent exists
        if self.registry.get(agent_id).is_none() {
            return Err(KernelError::Skipper(SkipperError::AgentNotFound(
                agent_id.to_string(),
            )));
        }
        Ok(self
            .triggers
            .register(agent_id, pattern, prompt_template, max_fires))
    }

    /// Remove a trigger by ID.
    pub fn remove_trigger(&self, trigger_id: TriggerId) -> bool {
        self.triggers.remove(trigger_id)
    }

    /// Enable or disable a trigger. Returns true if found.
    pub fn set_trigger_enabled(&self, trigger_id: TriggerId, enabled: bool) -> bool {
        self.triggers.set_enabled(trigger_id, enabled)
    }

    /// List all triggers (optionally filtered by agent).
    pub fn list_triggers(&self, agent_id: Option<AgentId>) -> Vec<crate::triggers::Trigger> {
        match agent_id {
            Some(id) => self.triggers.list_agent_triggers(id),
            None => self.triggers.list_all(),
        }
    }

    /// Register a workflow definition.
    pub async fn register_workflow(&self, workflow: Workflow) -> WorkflowId {
        self.workflows.register(workflow).await
    }

    /// Run a workflow pipeline end-to-end.
    pub async fn run_workflow(
        &self,
        workflow_id: WorkflowId,
        input: String,
    ) -> KernelResult<(WorkflowRunId, String)> {
        let run_id = self
            .workflows
            .create_run(workflow_id, input)
            .await
            .ok_or_else(|| {
                KernelError::Skipper(SkipperError::Internal("Workflow not found".to_string()))
            })?;

        // Agent resolver: looks up by name or ID in the registry
        let resolver = |agent_ref: &StepAgent| -> Option<(AgentId, String)> {
            match agent_ref {
                StepAgent::ById { id } => {
                    let agent_id: AgentId = id.parse().ok()?;
                    let entry = self.registry.get(agent_id)?;
                    Some((agent_id, entry.name.clone()))
                }
                StepAgent::ByName { name } => {
                    let entry = self.registry.find_by_name(name)?;
                    Some((entry.id, entry.name.clone()))
                }
            }
        };

        // Message sender: sends to agent and returns (output, in_tokens, out_tokens)
        let send_message = |agent_id: AgentId, message: String| async move {
            self.send_message(agent_id, &message)
                .await
                .map(|r| {
                    (
                        r.response,
                        r.total_usage.input_tokens,
                        r.total_usage.output_tokens,
                    )
                })
                .map_err(|e| format!("{e}"))
        };

        // SECURITY: Global workflow timeout to prevent runaway execution.
        const MAX_WORKFLOW_SECS: u64 = 3600; // 1 hour

        let output = tokio::time::timeout(
            std::time::Duration::from_secs(MAX_WORKFLOW_SECS),
            self.workflows.execute_run(run_id, resolver, send_message),
        )
        .await
        .map_err(|_| {
            KernelError::Skipper(SkipperError::Internal(format!(
                "Workflow timed out after {MAX_WORKFLOW_SECS}s"
            )))
        })?
        .map_err(|e| {
            KernelError::Skipper(SkipperError::Internal(format!("Workflow failed: {e}")))
        })?;

        Ok((run_id, output))
    }

    /// Gracefully shutdown the kernel.
    ///
    /// This cleanly shuts down in-memory state but preserves persistent agent
    /// data so agents are restored on the next boot.
    pub fn shutdown(&self) {
        info!("Shutting down Skipper kernel...");

        // Kill WhatsApp gateway child process if running
        if let Ok(guard) = self.whatsapp_gateway_pid.lock() {
            if let Some(pid) = *guard {
                info!("Stopping WhatsApp Web gateway (PID {pid})...");
                // Best-effort kill — don't block shutdown on failure
                #[cfg(unix)]
                {
                    unsafe {
                        libc::kill(pid as i32, libc::SIGTERM);
                    }
                }
                #[cfg(windows)]
                {
                    let _ = std::process::Command::new("taskkill")
                        .args(["/PID", &pid.to_string(), "/T", "/F"])
                        .stdout(std::process::Stdio::null())
                        .stderr(std::process::Stdio::null())
                        .status();
                }
            }
        }

        self.supervisor.shutdown();

        // Update agent states to Suspended in persistent storage (not delete)
        for entry in self.registry.list() {
            let _ = self.registry.set_state(entry.id, AgentState::Suspended);
            // Re-save with Suspended state for clean resume on next boot
            if let Some(updated) = self.registry.get(entry.id) {
                let _ = self.memory.save_agent(&updated);
            }
        }

        info!(
            "Skipper kernel shut down ({} agents preserved)",
            self.registry.list().len()
        );
    }
}


// --- OFP Wire Protocol integration ---

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn test_manifest_to_capabilities() {
        let mut manifest = AgentManifest {
            name: "test".to_string(),
            version: "0.1.0".to_string(),
            description: "test".to_string(),
            author: "test".to_string(),
            module: "test".to_string(),
            schedule: ScheduleMode::default(),
            model: ModelConfig::default(),
            fallback_models: vec![],
            resources: ResourceQuota::default(),
            priority: Priority::default(),
            capabilities: ManifestCapabilities::default(),
            profile: None,
            tools: HashMap::new(),
            skills: vec![],
            mcp_servers: vec![],
            metadata: HashMap::new(),
            tags: vec![],
            routing: None,
            autonomous: None,
            pinned_model: None,
            workspace: None,
            generate_identity_files: true,
            exec_policy: None,
        };
        manifest.capabilities.tools = vec!["file_read".to_string(), "web_fetch".to_string()];
        manifest.capabilities.agent_spawn = true;

        let caps = manifest_to_capabilities(&manifest);
        assert!(caps.contains(&Capability::ToolInvoke("file_read".to_string())));
        assert!(caps.contains(&Capability::AgentSpawn));
        assert_eq!(caps.len(), 3); // 2 tools + agent_spawn
    }

    fn test_manifest(name: &str, description: &str, tags: Vec<String>) -> AgentManifest {
        AgentManifest {
            name: name.to_string(),
            version: "0.1.0".to_string(),
            description: description.to_string(),
            author: "test".to_string(),
            module: "builtin:chat".to_string(),
            schedule: ScheduleMode::default(),
            model: ModelConfig::default(),
            fallback_models: vec![],
            resources: ResourceQuota::default(),
            priority: Priority::default(),
            capabilities: ManifestCapabilities::default(),
            profile: None,
            tools: HashMap::new(),
            skills: vec![],
            mcp_servers: vec![],
            metadata: HashMap::new(),
            tags,
            routing: None,
            autonomous: None,
            pinned_model: None,
            workspace: None,
            generate_identity_files: true,
            exec_policy: None,
        }
    }

    #[test]
    fn test_send_to_agent_by_name_resolution() {
        // Test that name resolution works in the registry
        let registry = AgentRegistry::new();
        let manifest = test_manifest("coder", "A coder agent", vec!["coding".to_string()]);
        let agent_id = AgentId::new();
        let entry = AgentEntry {
            id: agent_id,
            name: "coder".to_string(),
            manifest,
            state: AgentState::Running,
            mode: AgentMode::default(),
            created_at: chrono::Utc::now(),
            last_active: chrono::Utc::now(),
            parent: None,
            children: vec![],
            session_id: SessionId::new(),
            tags: vec!["coding".to_string()],
            identity: Default::default(),
            onboarding_completed: false,
            onboarding_completed_at: None,
        };
        registry.register(entry).unwrap();

        // find_by_name should return the agent
        let found = registry.find_by_name("coder");
        assert!(found.is_some());
        assert_eq!(found.unwrap().id, agent_id);

        // UUID lookup should also work
        let found_by_id = registry.get(agent_id);
        assert!(found_by_id.is_some());
    }

    #[test]
    fn test_find_agents_by_tag() {
        let registry = AgentRegistry::new();

        let m1 = test_manifest(
            "coder",
            "Expert coder",
            vec!["coding".to_string(), "rust".to_string()],
        );
        let e1 = AgentEntry {
            id: AgentId::new(),
            name: "coder".to_string(),
            manifest: m1,
            state: AgentState::Running,
            mode: AgentMode::default(),
            created_at: chrono::Utc::now(),
            last_active: chrono::Utc::now(),
            parent: None,
            children: vec![],
            session_id: SessionId::new(),
            tags: vec!["coding".to_string(), "rust".to_string()],
            identity: Default::default(),
            onboarding_completed: false,
            onboarding_completed_at: None,
        };
        registry.register(e1).unwrap();

        let m2 = test_manifest(
            "auditor",
            "Security auditor",
            vec!["security".to_string(), "audit".to_string()],
        );
        let e2 = AgentEntry {
            id: AgentId::new(),
            name: "auditor".to_string(),
            manifest: m2,
            state: AgentState::Running,
            mode: AgentMode::default(),
            created_at: chrono::Utc::now(),
            last_active: chrono::Utc::now(),
            parent: None,
            children: vec![],
            session_id: SessionId::new(),
            tags: vec!["security".to_string(), "audit".to_string()],
            identity: Default::default(),
            onboarding_completed: false,
            onboarding_completed_at: None,
        };
        registry.register(e2).unwrap();

        // Search by tag — should find only the matching agent
        let agents = registry.list();
        let security_agents: Vec<_> = agents
            .iter()
            .filter(|a| a.tags.iter().any(|t| t.to_lowercase().contains("security")))
            .collect();
        assert_eq!(security_agents.len(), 1);
        assert_eq!(security_agents[0].name, "auditor");

        // Search by name substring — should find coder
        let code_agents: Vec<_> = agents
            .iter()
            .filter(|a| a.name.to_lowercase().contains("coder"))
            .collect();
        assert_eq!(code_agents.len(), 1);
        assert_eq!(code_agents[0].name, "coder");
    }

    #[test]
    fn test_manifest_to_capabilities_with_profile() {
        use skipper_types::agent::ToolProfile;
        let manifest = AgentManifest {
            profile: Some(ToolProfile::Coding),
            ..Default::default()
        };
        let caps = manifest_to_capabilities(&manifest);
        // Coding profile gives: file_read, file_write, file_list, shell_exec, web_fetch
        assert!(caps
            .iter()
            .any(|c| matches!(c, Capability::ToolInvoke(name) if name == "file_read")));
        assert!(caps
            .iter()
            .any(|c| matches!(c, Capability::ToolInvoke(name) if name == "shell_exec")));
        assert!(caps.iter().any(|c| matches!(c, Capability::ShellExec(_))));
        assert!(caps.iter().any(|c| matches!(c, Capability::NetConnect(_))));
    }

    #[test]
    fn test_manifest_to_capabilities_profile_overridden_by_explicit_tools() {
        use skipper_types::agent::ToolProfile;
        let mut manifest = AgentManifest {
            profile: Some(ToolProfile::Coding),
            ..Default::default()
        };
        // Set explicit tools — profile should NOT be expanded
        manifest.capabilities.tools = vec!["file_read".to_string()];
        let caps = manifest_to_capabilities(&manifest);
        assert!(caps
            .iter()
            .any(|c| matches!(c, Capability::ToolInvoke(name) if name == "file_read")));
        // Should NOT have shell_exec since explicit tools override profile
        assert!(!caps
            .iter()
            .any(|c| matches!(c, Capability::ToolInvoke(name) if name == "shell_exec")));
    }
}
