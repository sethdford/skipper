//! Agent memory (KV store) management commands.

use crate::{find_daemon, daemon_client, daemon_json, ui};

/// List KV pairs for an agent.
pub fn cmd_memory_list(agent: &str, json: bool) {
    if let Some(base) = find_daemon() {
        let client = daemon_client();
        let url = format!("{base}/api/agents/{agent}/memory");
        let body = daemon_json(client.get(url).send());
        if json {
            println!("{}", serde_json::to_string_pretty(&body).unwrap_or_default());
            return;
        }
        // Human-readable format
        if let Some(obj) = body.as_object() {
            if obj.is_empty() {
                println!("No memory entries for agent '{agent}'.");
                return;
            }
            println!("Memory entries for '{agent}' ({}):", obj.len());
            for (key, value) in obj {
                println!("  {} = {}", key, value);
            }
        } else {
            println!("{}", serde_json::to_string_pretty(&body).unwrap_or_default());
        }
    } else {
        ui::error("Daemon not running. Start with: skipper start");
        std::process::exit(1);
    }
}

/// Get a specific KV value.
pub fn cmd_memory_get(agent: &str, key: &str, json: bool) {
    if let Some(base) = find_daemon() {
        let client = daemon_client();
        let url = format!("{base}/api/agents/{agent}/memory/{key}");
        let body = daemon_json(client.get(url).send());
        if json {
            println!("{}", serde_json::to_string_pretty(&body).unwrap_or_default());
            return;
        }
        // Human-readable format
        if let Some(value) = body.get("value") {
            println!("{}", value);
        } else if let Some(msg) = body.get("message").and_then(|v| v.as_str()) {
            println!("{msg}");
        } else {
            println!("{}", serde_json::to_string_pretty(&body).unwrap_or_default());
        }
    } else {
        ui::error("Daemon not running. Start with: skipper start");
        std::process::exit(1);
    }
}

/// Set a KV value.
pub fn cmd_memory_set(agent: &str, key: &str, value: &str) {
    if let Some(base) = find_daemon() {
        let client = daemon_client();
        let url = format!("{base}/api/agents/{agent}/memory/{key}");
        let payload = serde_json::json!({ "value": value });
        let body = daemon_json(client.put(url).json(&payload).send());

        if let Some(msg) = body.get("message").and_then(|v| v.as_str()) {
            ui::success(msg);
        } else {
            ui::success(&format!("Set '{key}' for agent '{agent}'"));
        }
    } else {
        ui::error("Daemon not running. Start with: skipper start");
        std::process::exit(1);
    }
}

/// Delete a KV pair.
pub fn cmd_memory_delete(agent: &str, key: &str) {
    if let Some(base) = find_daemon() {
        let client = daemon_client();
        let url = format!("{base}/api/agents/{agent}/memory/{key}");
        let body = daemon_json(client.delete(url).send());

        if let Some(msg) = body.get("message").and_then(|v| v.as_str()) {
            ui::success(msg);
        } else {
            ui::success(&format!("Deleted '{key}' for agent '{agent}'"));
        }
    } else {
        ui::error("Daemon not running. Start with: skipper start");
        std::process::exit(1);
    }
}
