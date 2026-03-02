//! System info, version, health, logging, and UI commands.

use crate::{find_daemon, daemon_client, daemon_json, skipper_home, ui, tui};
use std::path::PathBuf;

/// Show detailed system information.
pub fn cmd_system_info(json: bool) {
    if let Some(base) = find_daemon() {
        let client = daemon_client();
        let body = daemon_json(client.get(format!("{base}/api/system/info")).send());
        if json {
            println!("{}", serde_json::to_string_pretty(&body).unwrap_or_default());
            return;
        }
        // Format for human-readable output
        println!("{}", serde_json::to_string_pretty(&body).unwrap_or_default());
    } else {
        // Standalone: return basic system info
        let system_info = serde_json::json!({
            "skipper_home": skipper_home().display().to_string(),
            "daemon_running": false,
        });
        println!("{}", serde_json::to_string_pretty(&system_info).unwrap_or_default());
    }
}

/// Show version information.
pub fn cmd_system_version(json: bool) {
    let version = env!("CARGO_PKG_VERSION");
    if json {
        let info = serde_json::json!({
            "version": version,
        });
        println!("{}", serde_json::to_string_pretty(&info).unwrap_or_default());
    } else {
        println!("Skipper {}", version);
    }
}

/// Open the web dashboard in the default browser.
pub fn cmd_dashboard() {
    let base = if let Some(url) = find_daemon() {
        url
    } else {
        // Auto-start the daemon
        ui::hint("No daemon running — starting one now...");
        match crate::start_daemon_background() {
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
    if !crate::open_in_browser(&url) {
        ui::hint(&format!("Could not open browser. Visit: {url}"));
    }
}

/// Generate shell completion scripts for the given shell.
pub fn cmd_completion(shell: clap_complete::Shell) {
    use clap::Command;

    // Build a minimal command structure for completion generation
    let app = Command::new("skipper")
        .version(env!("CARGO_PKG_VERSION"))
        .about("Skipper — Open-source Agent Operating System");

    clap_complete::generate(shell, &mut { app }, "skipper", &mut std::io::stdout());
}

/// Interactive quick chat with the default or specified agent.
pub fn cmd_quick_chat(config: Option<PathBuf>, agent: Option<String>) {
    tui::chat_runner::run_chat_tui(config, agent);
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

/// Tail the Skipper log file, optionally following in real-time.
pub fn cmd_logs(lines: usize, follow: bool) {
    let log_path = skipper_home().join("tui.log");

    if !log_path.exists() {
        println!("No logs found. Log file not yet created: {}", log_path.display());
        return;
    }

    if follow {
        // Use `tail -f` to follow the log
        #[cfg(unix)]
        {
            use std::process::Command;
            let _ = Command::new("tail")
                .args(["-f", "-n", &lines.to_string()])
                .arg(&log_path)
                .status();
        }
        #[cfg(windows)]
        {
            // On Windows, just read the file and don't follow
            if let Ok(content) = std::fs::read_to_string(&log_path) {
                let lines_vec: Vec<&str> = content.lines().collect();
                let start = if lines_vec.len() > lines {
                    lines_vec.len() - lines
                } else {
                    0
                };
                for line in &lines_vec[start..] {
                    println!("{line}");
                }
            }
        }
    } else {
        // Just read the last N lines
        if let Ok(content) = std::fs::read_to_string(&log_path) {
            let lines_vec: Vec<&str> = content.lines().collect();
            let start = if lines_vec.len() > lines {
                lines_vec.len() - lines
            } else {
                0
            };
            for line in &lines_vec[start..] {
                println!("{line}");
            }
        }
    }
}

/// Quick daemon health check.
pub fn cmd_health(json: bool) {
    if let Some(base) = find_daemon() {
        let client = daemon_client();
        let body = daemon_json(client.get(format!("{base}/api/health")).send());
        if json {
            println!("{}", serde_json::to_string_pretty(&body).unwrap_or_default());
        } else if let Some(status) = body.get("status").and_then(|v| v.as_str()) {
            println!("Daemon: {status}");
        } else {
            println!("Daemon: running");
        }
    } else if json {
        println!("{{\"status\": \"offline\"}}");
    } else {
        println!("Daemon: offline");
    }
}

/// Reset local config and state (with confirmation).
pub fn cmd_reset(confirm: bool) {
    let home = skipper_home();

    if !confirm {
        println!("This will delete all local Skipper configuration and data.");
        println!("Location: {}", home.display());
        print!("Continue? (y/N): ");
        use std::io::{self, Write};
        io::stdout().flush().unwrap();

        let mut input = String::new();
        io::stdin().read_line(&mut input).unwrap_or(0);
        if !input.trim().eq_ignore_ascii_case("y") {
            println!("Cancelled.");
            return;
        }
    }

    match std::fs::remove_dir_all(&home) {
        Ok(()) => {
            ui::success(&format!("Deleted {}", home.display()));
            println!("Run `skipper init` to set up again.");
        }
        Err(e) => {
            ui::error(&format!("Failed to delete {}: {e}", home.display()));
            std::process::exit(1);
        }
    }
}
