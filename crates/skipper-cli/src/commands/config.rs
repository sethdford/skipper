//! Config command handlers: show, edit, get, set, unset, set-key, delete-key, test-key.

use crate::{dotenv, prompt_input, restrict_file_permissions, skipper_home, test_api_key, ui, provider_to_env_var};
use colored::Colorize;
use std::io::{self, Write};

/// Show the current configuration.
pub fn cmd_config_show() {
    let home = skipper_home();
    let config_path = home.join("config.toml");

    if !config_path.exists() {
        println!("No configuration found at: {}", config_path.display());
        println!("Run `skipper init` to create one.");
        return;
    }

    let content = std::fs::read_to_string(&config_path).unwrap_or_else(|e| {
        eprintln!("Error reading config: {e}");
        std::process::exit(1);
    });

    println!("# {}\n", config_path.display());
    println!("{content}");
}

/// Open the configuration file in your editor.
pub fn cmd_config_edit() {
    let home = skipper_home();
    let config_path = home.join("config.toml");

    let editor = std::env::var("EDITOR")
        .or_else(|_| std::env::var("VISUAL"))
        .unwrap_or_else(|_| {
            if cfg!(windows) {
                "notepad".to_string()
            } else {
                "vi".to_string()
            }
        });

    let status = std::process::Command::new(&editor)
        .arg(&config_path)
        .status();

    match status {
        Ok(s) if s.success() => {}
        Ok(s) => {
            eprintln!("Editor exited with: {s}");
        }
        Err(e) => {
            eprintln!("Failed to open editor '{editor}': {e}");
            eprintln!("Set $EDITOR to your preferred editor.");
        }
    }
}

/// Get a config value by dotted key path.
pub fn cmd_config_get(key: &str) {
    let home = skipper_home();
    let config_path = home.join("config.toml");

    if !config_path.exists() {
        ui::error_with_fix("No config file found", "Run `skipper init` first");
        std::process::exit(1);
    }

    let content = std::fs::read_to_string(&config_path).unwrap_or_else(|e| {
        ui::error(&format!("Failed to read config: {e}"));
        std::process::exit(1);
    });

    let table: toml::Value = toml::from_str(&content).unwrap_or_else(|e| {
        ui::error_with_fix(
            &format!("Config parse error: {e}"),
            "Fix your config.toml syntax, or run `skipper config edit`",
        );
        std::process::exit(1);
    });

    // Navigate dotted path
    let mut current = &table;
    for part in key.split('.') {
        match current.get(part) {
            Some(v) => current = v,
            None => {
                ui::error(&format!("Key not found: {key}"));
                std::process::exit(1);
            }
        }
    }

    // Print value
    match current {
        toml::Value::String(s) => println!("{s}"),
        toml::Value::Integer(i) => println!("{i}"),
        toml::Value::Float(f) => println!("{f}"),
        toml::Value::Boolean(b) => println!("{b}"),
        other => println!("{other}"),
    }
}

/// Set a config value.
pub fn cmd_config_set(key: &str, value: &str) {
    let home = skipper_home();
    let config_path = home.join("config.toml");

    if !config_path.exists() {
        ui::error_with_fix("No config file found", "Run `skipper init` first");
        std::process::exit(1);
    }

    let content = std::fs::read_to_string(&config_path).unwrap_or_else(|e| {
        ui::error(&format!("Failed to read config: {e}"));
        std::process::exit(1);
    });

    let mut table: toml::Value = toml::from_str(&content).unwrap_or_else(|e| {
        ui::error_with_fix(
            &format!("Config parse error: {e}"),
            "Fix your config.toml syntax first",
        );
        std::process::exit(1);
    });

    // Navigate to parent and set key
    let parts: Vec<&str> = key.split('.').collect();
    if parts.is_empty() {
        ui::error("Empty key");
        std::process::exit(1);
    }

    let mut current = &mut table;
    for part in &parts[..parts.len() - 1] {
        current = current
            .as_table_mut()
            .and_then(|t| t.get_mut(*part))
            .unwrap_or_else(|| {
                ui::error(&format!("Key path not found: {key}"));
                std::process::exit(1);
            });
    }

    let last_key = parts[parts.len() - 1];
    let tbl = current.as_table_mut().unwrap_or_else(|| {
        ui::error(&format!("Parent of '{key}' is not a table"));
        std::process::exit(1);
    });

    // Try to preserve type: if the existing value is an integer, parse as int, etc.
    let new_value = if let Some(existing) = tbl.get(last_key) {
        match existing {
            toml::Value::Integer(_) => value
                .parse::<i64>()
                .map(toml::Value::Integer)
                .unwrap_or_else(|_| toml::Value::String(value.to_string())),
            toml::Value::Float(_) => value
                .parse::<f64>()
                .map(toml::Value::Float)
                .unwrap_or_else(|_| toml::Value::String(value.to_string())),
            toml::Value::Boolean(_) => value
                .parse::<bool>()
                .map(toml::Value::Boolean)
                .unwrap_or_else(|_| toml::Value::String(value.to_string())),
            _ => toml::Value::String(value.to_string()),
        }
    } else {
        toml::Value::String(value.to_string())
    };

    tbl.insert(last_key.to_string(), new_value);

    // Write back (note: this strips comments — warned in help text)
    let serialized = toml::to_string_pretty(&table).unwrap_or_else(|e| {
        ui::error(&format!("Failed to serialize config: {e}"));
        std::process::exit(1);
    });

    std::fs::write(&config_path, &serialized).unwrap_or_else(|e| {
        ui::error(&format!("Failed to write config: {e}"));
        std::process::exit(1);
    });
    restrict_file_permissions(&config_path);

    ui::success(&format!("Set {key} = {value}"));
}

/// Remove a config key.
pub fn cmd_config_unset(key: &str) {
    let home = skipper_home();
    let config_path = home.join("config.toml");

    if !config_path.exists() {
        ui::error_with_fix("No config file found", "Run `skipper init` first");
        std::process::exit(1);
    }

    let content = std::fs::read_to_string(&config_path).unwrap_or_else(|e| {
        ui::error(&format!("Failed to read config: {e}"));
        std::process::exit(1);
    });

    let mut table: toml::Value = toml::from_str(&content).unwrap_or_else(|e| {
        ui::error_with_fix(
            &format!("Config parse error: {e}"),
            "Fix your config.toml syntax first",
        );
        std::process::exit(1);
    });

    // Navigate to parent table and remove the final key
    let parts: Vec<&str> = key.split('.').collect();
    if parts.is_empty() {
        ui::error("Empty key");
        std::process::exit(1);
    }

    let mut current = &mut table;
    for part in &parts[..parts.len() - 1] {
        current = current
            .as_table_mut()
            .and_then(|t| t.get_mut(*part))
            .unwrap_or_else(|| {
                ui::error(&format!("Key path not found: {key}"));
                std::process::exit(1);
            });
    }

    let last_key = parts[parts.len() - 1];
    let tbl = current.as_table_mut().unwrap_or_else(|| {
        ui::error(&format!("Parent of '{key}' is not a table"));
        std::process::exit(1);
    });

    if tbl.remove(last_key).is_none() {
        ui::error(&format!("Key not found: {key}"));
        std::process::exit(1);
    }

    // Write back (note: this strips comments — warned in help text)
    let serialized = toml::to_string_pretty(&table).unwrap_or_else(|e| {
        ui::error(&format!("Failed to serialize config: {e}"));
        std::process::exit(1);
    });

    std::fs::write(&config_path, &serialized).unwrap_or_else(|e| {
        ui::error(&format!("Failed to write config: {e}"));
        std::process::exit(1);
    });
    restrict_file_permissions(&config_path);

    ui::success(&format!("Removed key: {key}"));
}

/// Save an API key to ~/.skipper/.env (prompts interactively).
pub fn cmd_config_set_key(provider: &str) {
    let env_var = provider_to_env_var(provider);

    let key = prompt_input(&format!("  Paste your {provider} API key: "));
    if key.is_empty() {
        ui::error("No key provided. Cancelled.");
        return;
    }

    match dotenv::save_env_key(&env_var, &key) {
        Ok(()) => {
            ui::success(&format!("Saved {env_var} to ~/.skipper/.env"));
            // Test the key
            print!("  Testing key... ");
            io::stdout().flush().unwrap();
            if test_api_key(provider, &env_var) {
                println!("{}", "OK".bright_green());
            } else {
                println!("{}", "could not verify (may still work)".bright_yellow());
            }
        }
        Err(e) => {
            ui::error(&format!("Failed to save key: {e}"));
            std::process::exit(1);
        }
    }
}

/// Remove an API key from ~/.skipper/.env.
pub fn cmd_config_delete_key(provider: &str) {
    let env_var = provider_to_env_var(provider);

    match dotenv::remove_env_key(&env_var) {
        Ok(()) => ui::success(&format!("Removed {env_var} from ~/.skipper/.env")),
        Err(e) => {
            ui::error(&format!("Failed to remove key: {e}"));
            std::process::exit(1);
        }
    }
}

/// Test provider connectivity with the stored API key.
pub fn cmd_config_test_key(provider: &str) {
    let env_var = provider_to_env_var(provider);

    if std::env::var(&env_var).is_err() {
        ui::error(&format!("{env_var} not set"));
        ui::hint(&format!("Set it: skipper config set-key {provider}"));
        std::process::exit(1);
    }

    print!("  Testing {provider} ({env_var})... ");
    io::stdout().flush().unwrap();
    if test_api_key(provider, &env_var) {
        println!("{}", "OK".bright_green());
    } else {
        println!("{}", "FAILED (401/403)".bright_red());
        ui::hint(&format!("Update key: skipper config set-key {provider}"));
        std::process::exit(1);
    }
}
