//! Device pairing and webhook management commands.

use crate::{find_daemon, daemon_client, daemon_json, ui};

/// List paired devices.
pub fn cmd_devices_list(json: bool) {
    if let Some(base) = find_daemon() {
        let client = daemon_client();
        let body = daemon_json(client.get(format!("{base}/api/devices")).send());
        if json {
            println!("{}", serde_json::to_string_pretty(&body).unwrap_or_default());
            return;
        }
        // Human-readable format
        if let Some(arr) = body.as_array() {
            if arr.is_empty() {
                println!("No paired devices.");
                return;
            }
            println!("Paired Devices ({}):", arr.len());
            for device in arr {
                if let Some(id) = device.get("id").and_then(|v| v.as_str()) {
                    let name = device
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("(unnamed)");
                    println!("  {} - {}", id, name);
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

/// Start a new device pairing flow (generates QR code).
pub fn cmd_devices_pair() {
    if let Some(base) = find_daemon() {
        let client = daemon_client();
        let body = daemon_json(client.post(format!("{base}/api/devices/pair")).send());

        if let Some(qr_code) = body.get("qr_code").and_then(|v| v.as_str()) {
            println!("Scan this QR code with your device:");
            println!("\n{}\n", qr_code);
        }
        if let Some(code) = body.get("pairing_code").and_then(|v| v.as_str()) {
            println!("Or enter this code: {}", code);
        }
    } else {
        ui::error("Daemon not running. Start with: skipper start");
        std::process::exit(1);
    }
}

/// Remove a paired device.
pub fn cmd_devices_remove(id: &str) {
    if let Some(base) = find_daemon() {
        let client = daemon_client();
        let url = format!("{base}/api/devices/{id}");
        let body = daemon_json(client.delete(url).send());

        if let Some(msg) = body.get("message").and_then(|v| v.as_str()) {
            ui::success(msg);
        } else {
            ui::success(&format!("Device '{id}' removed"));
        }
    } else {
        ui::error("Daemon not running. Start with: skipper start");
        std::process::exit(1);
    }
}

/// List configured webhooks.
pub fn cmd_webhooks_list(json: bool) {
    if let Some(base) = find_daemon() {
        let client = daemon_client();
        let body = daemon_json(client.get(format!("{base}/api/webhooks")).send());
        if json {
            println!("{}", serde_json::to_string_pretty(&body).unwrap_or_default());
            return;
        }
        // Human-readable format
        if let Some(arr) = body.as_array() {
            if arr.is_empty() {
                println!("No webhooks configured.");
                return;
            }
            println!("Webhooks ({}):", arr.len());
            for webhook in arr {
                if let Some(id) = webhook.get("id").and_then(|v| v.as_str()) {
                    let url = webhook
                        .get("url")
                        .and_then(|v| v.as_str())
                        .unwrap_or("(unknown)");
                    let agent = webhook
                        .get("agent_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("(unknown)");
                    println!("  {} → {} (agent: {})", id, url, agent);
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

/// Create a new webhook trigger.
pub fn cmd_webhooks_create(agent: &str, url: &str) {
    if let Some(base) = find_daemon() {
        let client = daemon_client();
        let payload = serde_json::json!({
            "agent_id": agent,
            "url": url,
        });
        let body = daemon_json(client.post(format!("{base}/api/webhooks")).json(&payload).send());

        if let Some(id) = body.get("id").and_then(|v| v.as_str()) {
            ui::success(&format!("Webhook created: {id}"));
        } else if let Some(msg) = body.get("message").and_then(|v| v.as_str()) {
            ui::success(msg);
        } else {
            println!("{}", serde_json::to_string_pretty(&body).unwrap_or_default());
        }
    } else {
        ui::error("Daemon not running. Start with: skipper start");
        std::process::exit(1);
    }
}

/// Delete a webhook.
pub fn cmd_webhooks_delete(id: &str) {
    if let Some(base) = find_daemon() {
        let client = daemon_client();
        let url = format!("{base}/api/webhooks/{id}");
        let body = daemon_json(client.delete(url).send());

        if let Some(msg) = body.get("message").and_then(|v| v.as_str()) {
            ui::success(msg);
        } else {
            ui::success(&format!("Webhook '{id}' deleted"));
        }
    } else {
        ui::error("Daemon not running. Start with: skipper start");
        std::process::exit(1);
    }
}

/// Send a test payload to a webhook.
pub fn cmd_webhooks_test(id: &str) {
    if let Some(base) = find_daemon() {
        let client = daemon_client();
        let url = format!("{base}/api/webhooks/{id}/test");
        let body = daemon_json(client.post(url).send());

        if let Some(msg) = body.get("message").and_then(|v| v.as_str()) {
            ui::success(msg);
        } else {
            ui::success(&format!("Test payload sent to webhook '{id}'"));
        }
    } else {
        ui::error("Daemon not running. Start with: skipper start");
        std::process::exit(1);
    }
}
