//! Initialization command: init, init_quick, init_interactive.

use crate::{
    find_daemon, open_in_browser, restrict_dir_permissions, restrict_file_permissions,
    ui, bundled_agents, tui,
};
use crate::commands::cmd_quick_chat;

/// Initialize Skipper configuration and data directories.
pub fn cmd_init(quick: bool) {
    let home = match dirs::home_dir() {
        Some(h) => h,
        None => {
            ui::error("Could not determine home directory");
            std::process::exit(1);
        }
    };

    let skipper_dir = home.join(".skipper");

    // --- Ensure directories exist ---
    if !skipper_dir.exists() {
        std::fs::create_dir_all(&skipper_dir).unwrap_or_else(|e| {
            ui::error_with_fix(
                &format!("Failed to create {}", skipper_dir.display()),
                &format!("Check permissions on {}", home.display()),
            );
            eprintln!("  {e}");
            std::process::exit(1);
        });
        restrict_dir_permissions(&skipper_dir);
    }

    for sub in ["data", "agents"] {
        let dir = skipper_dir.join(sub);
        if !dir.exists() {
            std::fs::create_dir_all(&dir).unwrap_or_else(|e| {
                eprintln!("Error creating {sub} dir: {e}");
                std::process::exit(1);
            });
        }
    }

    // Install bundled agent templates (skips existing ones to preserve user edits)
    bundled_agents::install_bundled_agents(&skipper_dir.join("agents"));

    if quick {
        cmd_init_quick(&skipper_dir);
    } else {
        cmd_init_interactive(&skipper_dir);
    }
}

/// Quick init: no prompts, auto-detect, write config + .env, print next steps.
fn cmd_init_quick(skipper_dir: &std::path::Path) {
    ui::banner();
    ui::blank();

    let (provider, api_key_env, model) = detect_best_provider();

    write_config_if_missing(skipper_dir, provider, model, api_key_env);

    ui::blank();
    ui::success("Skipper initialized (quick mode)");
    ui::kv("Provider", provider);
    ui::kv("Model", model);
    ui::blank();
    ui::next_steps(&[
        "Start the daemon:  skipper start",
        "Chat:              skipper chat",
    ]);
}

/// Interactive 5-step onboarding wizard (ratatui TUI).
fn cmd_init_interactive(skipper_dir: &std::path::Path) {
    use tui::screens::init_wizard::{self, InitResult, LaunchChoice};

    match init_wizard::run() {
        InitResult::Completed {
            provider,
            model,
            daemon_started,
            launch,
        } => {
            // Print summary after TUI restores terminal
            ui::blank();
            ui::success("Skipper initialized!");
            ui::kv("Provider", &provider);
            ui::kv("Model", &model);

            if daemon_started {
                ui::kv_ok("Daemon", "running");
            }
            ui::blank();

            // Execute the user's chosen launch action.
            match launch {
                LaunchChoice::Desktop => {
                    launch_desktop_app(skipper_dir);
                }
                LaunchChoice::Dashboard => {
                    if let Some(base) = find_daemon() {
                        let url = format!("{base}/");
                        ui::success(&format!("Opening dashboard at {url}"));
                        if !open_in_browser(&url) {
                            ui::hint(&format!("Could not open browser. Visit: {url}"));
                        }
                    } else {
                        ui::error("Daemon is not running. Start it with: skipper start");
                    }
                }
                LaunchChoice::Chat => {
                    ui::hint("Starting chat session...");
                    ui::blank();
                    // Note: tracing was initialized for stderr (init is a CLI
                    // subcommand).  The chat TUI takes over the terminal with
                    // raw mode so stderr output is suppressed.  We can't
                    // reinitialize tracing (global subscriber is set once).
                    cmd_quick_chat(None, None);
                }
            }
        }
        InitResult::Cancelled => {
            println!("  Setup cancelled.");
        }
    }
}

/// Launch the skipper-desktop Tauri app, connecting to the running daemon.
fn launch_desktop_app(_skipper_dir: &std::path::Path) {
    // Look for the desktop binary next to our own executable.
    let desktop_bin = {
        let exe = std::env::current_exe().ok();
        let dir = exe.as_ref().and_then(|e| e.parent());

        #[cfg(windows)]
        let name = "skipper-desktop.exe";
        #[cfg(not(windows))]
        let name = "skipper-desktop";

        dir.map(|d| d.join(name))
    };

    match desktop_bin {
        Some(ref path) if path.exists() => {
            ui::success("Launching Skipper Desktop...");
            match std::process::Command::new(path)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
            {
                Ok(_) => {
                    ui::success("Desktop app started.");
                }
                Err(e) => {
                    ui::error(&format!("Failed to launch desktop app: {e}"));
                    ui::hint("Try: skipper dashboard");
                }
            }
        }
        _ => {
            ui::error("Desktop app not found.");
            ui::hint("Install it with: cargo install skipper-desktop");
            ui::hint("Falling back to web dashboard...");
            ui::blank();
            if let Some(base) = find_daemon() {
                let url = format!("{base}/");
                if !open_in_browser(&url) {
                    ui::hint(&format!("Visit: {url}"));
                }
            }
        }
    }
}

/// Auto-detect the best available provider.
fn detect_best_provider() -> (&'static str, &'static str, &'static str) {
    let providers = provider_list();

    for (p, env_var, m, display) in &providers {
        if std::env::var(env_var).is_ok() {
            ui::success(&format!("Detected {display} ({env_var})"));
            return (p, env_var, m);
        }
    }
    // Also check GOOGLE_API_KEY
    if std::env::var("GOOGLE_API_KEY").is_ok() {
        ui::success("Detected Gemini (GOOGLE_API_KEY)");
        return ("gemini", "GOOGLE_API_KEY", "gemini-2.5-flash");
    }
    // Check CLAUDE_CODE_OAUTH_TOKEN with Claude Code CLI
    if std::env::var("CLAUDE_CODE_OAUTH_TOKEN").is_ok()
        && skipper_runtime::drivers::claude_code::claude_code_available()
    {
        ui::success("Detected Claude Code (CLAUDE_CODE_OAUTH_TOKEN)");
        return ("claude-code", "CLAUDE_CODE_OAUTH_TOKEN", "claude-code/sonnet");
    }
    // Check if Claude Code CLI is available (uses OAuth token internally)
    if skipper_runtime::drivers::claude_code::claude_code_available() {
        ui::success("Detected Claude Code CLI (uses your Claude subscription)");
        return ("claude-code", "", "claude-code/sonnet");
    }
    ui::hint("No LLM provider API keys found");
    ui::hint("Groq offers a free tier: https://console.groq.com");
    ("groq", "GROQ_API_KEY", "llama-3.3-70b-versatile")
}

/// Static list of supported providers: (id, env_var, default_model, display_name).
fn provider_list() -> Vec<(&'static str, &'static str, &'static str, &'static str)> {
    vec![
        ("groq", "GROQ_API_KEY", "llama-3.3-70b-versatile", "Groq"),
        ("gemini", "GEMINI_API_KEY", "gemini-2.5-flash", "Gemini"),
        ("deepseek", "DEEPSEEK_API_KEY", "deepseek-chat", "DeepSeek"),
        (
            "anthropic",
            "ANTHROPIC_API_KEY",
            "claude-sonnet-4-20250514",
            "Anthropic",
        ),
        ("openai", "OPENAI_API_KEY", "gpt-4o", "OpenAI"),
        (
            "openrouter",
            "OPENROUTER_API_KEY",
            "openrouter/auto",
            "OpenRouter",
        ),
    ]
}

/// Write config.toml if it doesn't already exist.
fn write_config_if_missing(
    skipper_dir: &std::path::Path,
    provider: &str,
    model: &str,
    api_key_env: &str,
) {
    let config_path = skipper_dir.join("config.toml");
    if config_path.exists() {
        ui::check_ok(&format!("Config already exists: {}", config_path.display()));
    } else {
        let default_config = format!(
            r#"# Skipper Agent OS configuration
# See https://github.com/sethdford/skipper for documentation

# For Docker, change to "0.0.0.0:4200" or set SKIPPER_LISTEN env var.
api_listen = "127.0.0.1:4200"

[default_model]
provider = "{provider}"
model = "{model}"
api_key_env = "{api_key_env}"

[memory]
decay_rate = 0.05
"#
        );
        std::fs::write(&config_path, &default_config).unwrap_or_else(|e| {
            ui::error_with_fix("Failed to write config", &e.to_string());
            std::process::exit(1);
        });
        restrict_file_permissions(&config_path);
        ui::success(&format!("Created: {}", config_path.display()));
    }
}
