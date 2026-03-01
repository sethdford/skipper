//! Daemon commands: start, stop, status.

use crate::{find_daemon, daemon_client, daemon_json, ui};
use skipper_api::server::read_daemon_info;
use skipper_kernel::SkipperKernel;
use std::path::PathBuf;

/// Start the Skipper daemon.
pub fn cmd_start(config: Option<PathBuf>) {
    if let Some(base) = find_daemon() {
        ui::error_with_fix(
            &format!("Daemon already running at {base}"),
            "Use `skipper status` to check it, or stop it first",
        );
        std::process::exit(1);
    }

    ui::banner();
    ui::blank();
    println!("  Starting daemon...");
    ui::blank();

    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let kernel = match SkipperKernel::boot(config.as_deref()) {
            Ok(k) => k,
            Err(e) => {
                boot_kernel_error(&e);
                std::process::exit(1);
            }
        };

        let listen_addr = kernel.config.api_listen.clone();
        let daemon_info_path = kernel.config.home_dir.join("daemon.json");
        let provider = kernel.config.default_model.provider.clone();
        let model = kernel.config.default_model.model.clone();
        let agent_count = kernel.registry.count();
        let model_count = kernel
            .model_catalog
            .read()
            .map(|c| c.list_models().len())
            .unwrap_or(0);

        ui::success(&format!("Kernel booted ({provider}/{model})"));
        if model_count > 0 {
            ui::success(&format!("{model_count} models available"));
        }
        if agent_count > 0 {
            ui::success(&format!("{agent_count} agent(s) loaded"));
        }
        ui::blank();
        ui::kv("API", &format!("http://{listen_addr}"));
        ui::kv("Dashboard", &format!("http://{listen_addr}/"));
        ui::kv("Provider", &provider);
        ui::kv("Model", &model);
        ui::blank();
        ui::hint("Open the dashboard in your browser, or run `skipper chat`");
        ui::hint("Press Ctrl+C to stop the daemon");
        ui::blank();

        if let Err(e) =
            skipper_api::server::run_daemon(kernel, &listen_addr, Some(&daemon_info_path)).await
        {
            ui::error(&format!("Daemon error: {e}"));
            std::process::exit(1);
        }

        ui::blank();
        println!("  Skipper daemon stopped.");
    });
}

/// Stop the running Skipper daemon.
pub fn cmd_stop() {
    match find_daemon() {
        Some(base) => {
            let client = daemon_client();
            match client.post(format!("{base}/api/shutdown")).send() {
                Ok(r) if r.status().is_success() => {
                    // Wait for daemon to actually stop (up to 5 seconds)
                    for _ in 0..10 {
                        std::thread::sleep(std::time::Duration::from_millis(500));
                        if find_daemon().is_none() {
                            ui::success("Daemon stopped");
                            return;
                        }
                    }
                    // Still alive — force kill via PID
                    if let Some(home) = dirs::home_dir() {
                        let of_dir = home.join(".skipper");
                        if let Some(info) = read_daemon_info(&of_dir) {
                            force_kill_pid(info.pid);
                            let _ = std::fs::remove_file(of_dir.join("daemon.json"));
                        }
                    }
                    ui::success("Daemon stopped (forced)");
                }
                Ok(r) => {
                    ui::error(&format!("Shutdown request failed ({})", r.status()));
                }
                Err(e) => {
                    ui::error(&format!("Could not reach daemon: {e}"));
                }
            }
        }
        None => {
            ui::warn_with_fix(
                "No running daemon found",
                "Is it running? Check with: skipper status",
            );
        }
    }
}

/// Force-kill a process by PID.
fn force_kill_pid(pid: u32) {
    #[cfg(unix)]
    {
        let _ = std::process::Command::new("kill")
            .args(["-9", &pid.to_string()])
            .output();
    }
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/F"])
            .output();
    }
}

/// Show context-aware error for kernel boot failures.
pub fn boot_kernel_error(e: &skipper_kernel::error::KernelError) {
    let msg = e.to_string();
    if msg.contains("parse") || msg.contains("toml") || msg.contains("config") {
        ui::error_with_fix(
            "Failed to parse configuration",
            "Check your config.toml syntax: skipper config show",
        );
    } else if msg.contains("database") || msg.contains("locked") || msg.contains("sqlite") {
        ui::error_with_fix(
            "Database error (file may be locked)",
            "Check if another Skipper process is running: skipper status",
        );
    } else if msg.contains("key") || msg.contains("API") || msg.contains("auth") {
        ui::error_with_fix(
            "LLM provider authentication failed",
            "Run `skipper doctor` to check your API key configuration",
        );
    } else {
        ui::error_with_fix(
            &format!("Failed to boot kernel: {msg}"),
            "Run `skipper doctor` to diagnose the issue",
        );
    }
}

/// Show runtime status.
pub fn cmd_status(config: Option<PathBuf>, json: bool) {
    if let Some(base) = find_daemon() {
        let client = daemon_client();
        let body = daemon_json(client.get(format!("{base}/api/status")).send());

        if json {
            println!(
                "{}",
                serde_json::to_string_pretty(&body).unwrap_or_default()
            );
            return;
        }

        ui::section("Skipper Daemon Status");
        ui::blank();
        ui::kv_ok("Status", body["status"].as_str().unwrap_or("?"));
        ui::kv(
            "Agents",
            &body["agent_count"].as_u64().unwrap_or(0).to_string(),
        );
        ui::kv("Provider", body["default_provider"].as_str().unwrap_or("?"));
        ui::kv("Model", body["default_model"].as_str().unwrap_or("?"));
        ui::kv("API", &base);
        ui::kv("Dashboard", &format!("{base}/"));
        ui::kv("Data dir", body["data_dir"].as_str().unwrap_or("?"));
        ui::kv(
            "Uptime",
            &format!("{}s", body["uptime_seconds"].as_u64().unwrap_or(0)),
        );

        if let Some(agents) = body["agents"].as_array() {
            if !agents.is_empty() {
                ui::blank();
                ui::section("Active Agents");
                for a in agents {
                    println!(
                        "    {} ({}) -- {} [{}:{}]",
                        a["name"].as_str().unwrap_or("?"),
                        a["id"].as_str().unwrap_or("?"),
                        a["state"].as_str().unwrap_or("?"),
                        a["model_provider"].as_str().unwrap_or("?"),
                        a["model_name"].as_str().unwrap_or("?"),
                    );
                }
            }
        }
    } else {
        let kernel = boot_kernel(config);
        let agent_count = kernel.registry.count();

        if json {
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "status": "offline",
                    "agent_count": agent_count,
                    "agents": []
                }))
                .unwrap_or_default()
            );
            return;
        }

        ui::section("Skipper Daemon Status");
        ui::blank();
        ui::kv_warn("Status", "offline");
        ui::kv("Agents", &agent_count.to_string());
        ui::blank();
        ui::hint("Start the daemon: skipper start");
    }
}

/// Require a running daemon — exit with helpful message if not found.
pub fn require_daemon(command: &str) -> String {
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
pub fn boot_kernel(config: Option<PathBuf>) -> SkipperKernel {
    match SkipperKernel::boot(config.as_deref()) {
        Ok(k) => k,
        Err(e) => {
            boot_kernel_error(&e);
            std::process::exit(1);
        }
    }
}
