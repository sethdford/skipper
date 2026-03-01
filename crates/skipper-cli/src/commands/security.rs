//! Security and audit commands.

use crate::{find_daemon, daemon_client, daemon_json, ui};

/// Show security status summary.
pub fn cmd_security_status(json: bool) {
    if let Some(base) = find_daemon() {
        let client = daemon_client();
        let body = daemon_json(client.get(format!("{base}/api/security/status")).send());
        if json {
            println!("{}", serde_json::to_string_pretty(&body).unwrap_or_default());
            return;
        }
        // Human-readable format
        println!("{}", serde_json::to_string_pretty(&body).unwrap_or_default());
    } else {
        ui::error("Daemon not running. Start with: skipper start");
        std::process::exit(1);
    }
}

/// Show recent audit trail entries.
pub fn cmd_security_audit(limit: usize, json: bool) {
    if let Some(base) = find_daemon() {
        let client = daemon_client();
        let url = format!("{base}/api/security/audit?limit={limit}");
        let body = daemon_json(client.get(url).send());
        if json {
            println!("{}", serde_json::to_string_pretty(&body).unwrap_or_default());
            return;
        }
        // Human-readable format
        if let Some(arr) = body.as_array() {
            if arr.is_empty() {
                println!("No audit entries.");
                return;
            }
            println!("AUDIT TRAIL (last {}):", limit);
            for entry in arr {
                if let (Some(time), Some(event), Some(user)) = (
                    entry.get("timestamp").and_then(|v| v.as_str()),
                    entry.get("event").and_then(|v| v.as_str()),
                    entry.get("user").and_then(|v| v.as_str()),
                ) {
                    println!("  {} | {} | {}", time, user, event);
                }
            }
        } else {
            println!("{}", serde_json::to_string_pretty(&body).unwrap_or_default());
        }
    } else {
        ui::error("Daemon not running. Start with: skipper start");
        std::process::exit(1);
    }
}

/// Verify audit trail integrity (Merkle chain).
pub fn cmd_security_verify() {
    if let Some(base) = find_daemon() {
        let client = daemon_client();
        let body = daemon_json(client.get(format!("{base}/api/security/verify")).send());

        if let Some(valid) = body.get("valid").and_then(|v| v.as_bool()) {
            if valid {
                ui::success("Audit trail integrity verified ✓");
            } else {
                ui::error("Audit trail integrity check FAILED");
                if let Some(msg) = body.get("message").and_then(|v| v.as_str()) {
                    println!("Reason: {msg}");
                }
                std::process::exit(1);
            }
        } else {
            println!("{}", serde_json::to_string_pretty(&body).unwrap_or_default());
        }
    } else {
        ui::error("Daemon not running. Start with: skipper start");
        std::process::exit(1);
    }
}
