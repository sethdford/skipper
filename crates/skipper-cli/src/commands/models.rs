//! Model browsing and selection commands.

use crate::{find_daemon, daemon_client, daemon_json, require_daemon, prompt_input, ui};
use colored::Colorize;

/// List available models, optionally filtered by provider.
pub fn cmd_models_list(provider_filter: Option<&str>, json: bool) {
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

/// Show model aliases (shorthand names).
pub fn cmd_models_aliases(json: bool) {
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

/// List known LLM providers and their auth status.
pub fn cmd_models_providers(json: bool) {
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

/// Set the default model for the daemon.
pub fn cmd_models_set(model: Option<String>) {
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
