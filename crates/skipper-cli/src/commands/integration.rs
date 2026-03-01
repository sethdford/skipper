//! Integration and vault management commands.

use crate::{find_daemon, daemon_client, skipper_home, prompt_input, ui};
use colored::Colorize;

/// Add (install) an integration by name, optionally with an API key.
pub fn cmd_integration_add(name: &str, key: Option<&str>) {
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

/// Remove an installed integration by name.
pub fn cmd_integration_remove(name: &str) {
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

/// List available integrations, optionally filtered by a search query.
pub fn cmd_integrations_list(query: Option<&str>) {
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

/// Initialize the credential vault.
pub fn cmd_vault_init() {
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

/// Store a credential in the vault.
pub fn cmd_vault_set(key: &str) {
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

/// List all keys stored in the vault (values are hidden).
pub fn cmd_vault_list() {
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

/// Remove a credential from the vault.
pub fn cmd_vault_remove(key: &str) {
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
