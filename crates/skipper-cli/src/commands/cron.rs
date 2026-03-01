//! Scheduled job (cron) management commands.

use crate::{daemon_client, daemon_json, require_daemon, ui};

/// List scheduled jobs.
pub fn cmd_cron_list(json: bool) {
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

/// Create a new scheduled job.
pub fn cmd_cron_create(agent: &str, spec: &str, prompt: &str) {
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

/// Delete a scheduled job.
pub fn cmd_cron_delete(id: &str) {
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

/// Enable or disable a scheduled job.
pub fn cmd_cron_toggle(id: &str, enable: bool) {
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

/// List conversation sessions.
pub fn cmd_sessions(agent: Option<&str>, json: bool) {
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
            println!("No sessions.");
            return;
        }
        println!("{:<38} {:<16} {:<10} CREATED", "ID", "AGENT", "MESSAGES");
        println!("{}", "-".repeat(80));
        for s in arr {
            println!(
                "{:<38} {:<16} {:<10} {}",
                s["id"].as_str().unwrap_or("?"),
                s["agent_id"].as_str().unwrap_or("?"),
                s["message_count"].as_u64().unwrap_or(0),
                s["created_at"].as_str().unwrap_or(""),
            );
        }
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
    }
}
