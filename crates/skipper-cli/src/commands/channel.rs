//! Channel command handlers: list, setup, test, enable, disable.

use crate::{daemon_client, daemon_json, dotenv, find_daemon, prompt_input, restrict_file_permissions, skipper_home, ui};
use colored::Colorize;

/// List configured channels and their status.
pub fn cmd_channel_list() {
    let home = skipper_home();
    let config_path = home.join("config.toml");

    if !config_path.exists() {
        println!("No configuration found. Run `skipper init` first.");
        return;
    }

    let config_str = std::fs::read_to_string(&config_path).unwrap_or_default();

    println!("Channel Integrations:\n");
    println!("{:<12} {:<10} STATUS", "CHANNEL", "ENV VAR");
    println!("{}", "-".repeat(50));

    let channels: Vec<(&str, &str)> = vec![
        ("webchat", ""),
        ("telegram", "TELEGRAM_BOT_TOKEN"),
        ("discord", "DISCORD_BOT_TOKEN"),
        ("slack", "SLACK_BOT_TOKEN"),
        ("whatsapp", "WA_ACCESS_TOKEN"),
        ("signal", ""),
        ("matrix", "MATRIX_TOKEN"),
        ("email", "EMAIL_PASSWORD"),
    ];

    for (name, env_var) in channels {
        let configured = config_str.contains(&format!("[channels.{name}]"));
        let env_set = if env_var.is_empty() {
            true
        } else {
            std::env::var(env_var).is_ok()
        };

        let status = match (configured, env_set) {
            (true, true) => "Ready",
            (true, false) => "Missing env",
            (false, _) => "Not configured",
        };

        println!(
            "{:<12} {:<10} {}",
            name,
            if env_var.is_empty() { "—" } else { env_var },
            status,
        );
    }

    println!("\nUse `skipper channel setup <channel>` to configure a channel.");
}

/// Interactive setup wizard for a channel.
pub fn cmd_channel_setup(channel: Option<&str>) {
    let channel = match channel {
        Some(c) => c.to_string(),
        None => {
            // Interactive channel picker
            ui::section("Channel Setup");
            ui::blank();
            let channel_list = [
                ("telegram", "Telegram bot (BotFather)"),
                ("discord", "Discord bot"),
                ("slack", "Slack app (Socket Mode)"),
                ("whatsapp", "WhatsApp Cloud API"),
                ("email", "Email (IMAP/SMTP)"),
                ("signal", "Signal (signal-cli)"),
                ("matrix", "Matrix homeserver"),
            ];

            for (i, (name, desc)) in channel_list.iter().enumerate() {
                println!("    {:>2}. {:<12} {}", i + 1, name, desc.dimmed());
            }
            ui::blank();

            let choice = prompt_input("  Choose channel [1]: ");
            let idx = if choice.is_empty() {
                0
            } else {
                choice
                    .parse::<usize>()
                    .unwrap_or(1)
                    .saturating_sub(1)
                    .min(channel_list.len() - 1)
            };
            channel_list[idx].0.to_string()
        }
    };

    match channel.as_str() {
        "telegram" => {
            ui::section("Setting up Telegram");
            ui::blank();
            println!("  1. Open Telegram and message @BotFather");
            println!("  2. Send /newbot and follow the prompts");
            println!("  3. Copy the bot token");
            ui::blank();

            let token = prompt_input("  Paste your bot token: ");
            if token.is_empty() {
                ui::error("No token provided. Setup cancelled.");
                return;
            }

            let config_block = "\n[channels.telegram]\nbot_token_env = \"TELEGRAM_BOT_TOKEN\"\ndefault_agent = \"assistant\"\n";
            maybe_write_channel_config("telegram", config_block);

            // Save token to .env
            match dotenv::save_env_key("TELEGRAM_BOT_TOKEN", &token) {
                Ok(()) => ui::success("Token saved to ~/.skipper/.env"),
                Err(_) => println!("    export TELEGRAM_BOT_TOKEN={token}"),
            }

            ui::blank();
            ui::success("Telegram configured");
            notify_daemon_restart();
        }
        "discord" => {
            ui::section("Setting up Discord");
            ui::blank();
            println!("  1. Go to https://discord.com/developers/applications");
            println!("  2. Create a New Application");
            println!("  3. Go to Bot section and click 'Add Bot'");
            println!("  4. Copy the bot token");
            println!("  5. Under Privileged Gateway Intents, enable:");
            println!("     - Message Content Intent");
            println!("  6. Use OAuth2 URL Generator to invite bot to your server");
            ui::blank();

            let token = prompt_input("  Paste your bot token: ");
            if token.is_empty() {
                ui::error("No token provided. Setup cancelled.");
                return;
            }

            let config_block = "\n[channels.discord]\nbot_token_env = \"DISCORD_BOT_TOKEN\"\ndefault_agent = \"coder\"\n";
            maybe_write_channel_config("discord", config_block);

            match dotenv::save_env_key("DISCORD_BOT_TOKEN", &token) {
                Ok(()) => ui::success("Token saved to ~/.skipper/.env"),
                Err(_) => println!("    export DISCORD_BOT_TOKEN={token}"),
            }

            ui::blank();
            ui::success("Discord configured");
            notify_daemon_restart();
        }
        "slack" => {
            ui::section("Setting up Slack");
            ui::blank();
            println!("  1. Go to https://api.slack.com/apps");
            println!("  2. Create New App -> From Scratch");
            println!("  3. Enable Socket Mode (Settings -> Socket Mode)");
            println!("  4. Copy the App-Level Token (xapp-...)");
            println!("  5. Go to OAuth & Permissions, add scopes:");
            println!("     - chat:write, app_mentions:read, im:history");
            println!("  6. Install to workspace and copy Bot Token (xoxb-...)");
            ui::blank();

            let app_token = prompt_input("  Paste your App Token (xapp-...): ");
            let bot_token = prompt_input("  Paste your Bot Token (xoxb-...): ");

            let config_block = "\n[channels.slack]\napp_token_env = \"SLACK_APP_TOKEN\"\nbot_token_env = \"SLACK_BOT_TOKEN\"\ndefault_agent = \"assistant\"\n";
            maybe_write_channel_config("slack", config_block);

            if !app_token.is_empty() {
                match dotenv::save_env_key("SLACK_APP_TOKEN", &app_token) {
                    Ok(()) => ui::success("App token saved to ~/.skipper/.env"),
                    Err(_) => println!("    export SLACK_APP_TOKEN={app_token}"),
                }
            }
            if !bot_token.is_empty() {
                match dotenv::save_env_key("SLACK_BOT_TOKEN", &bot_token) {
                    Ok(()) => ui::success("Bot token saved to ~/.skipper/.env"),
                    Err(_) => println!("    export SLACK_BOT_TOKEN={bot_token}"),
                }
            }

            ui::blank();
            ui::success("Slack configured");
            notify_daemon_restart();
        }
        "whatsapp" => {
            ui::section("Setting up WhatsApp");
            ui::blank();
            println!("  WhatsApp Cloud API (recommended for production):");
            println!("  1. Go to https://developers.facebook.com");
            println!("  2. Create a Business App");
            println!("  3. Add WhatsApp product");
            println!("  4. Set up a test phone number");
            println!("  5. Copy Phone Number ID and Access Token");
            ui::blank();

            let phone_id = prompt_input("  Phone Number ID: ");
            let access_token = prompt_input("  Access Token: ");
            let verify_token = prompt_input("  Verify Token: ");

            let config_block = "\n[channels.whatsapp]\nmode = \"cloud_api\"\nphone_number_id_env = \"WA_PHONE_ID\"\naccess_token_env = \"WA_ACCESS_TOKEN\"\nverify_token_env = \"WA_VERIFY_TOKEN\"\nwebhook_port = 8443\ndefault_agent = \"assistant\"\n";
            maybe_write_channel_config("whatsapp", config_block);

            for (key, val) in [
                ("WA_PHONE_ID", &phone_id),
                ("WA_ACCESS_TOKEN", &access_token),
                ("WA_VERIFY_TOKEN", &verify_token),
            ] {
                if !val.is_empty() {
                    match dotenv::save_env_key(key, val) {
                        Ok(()) => ui::success(&format!("{key} saved to ~/.skipper/.env")),
                        Err(_) => println!("    export {key}={val}"),
                    }
                }
            }

            ui::blank();
            ui::success("WhatsApp configured");
            notify_daemon_restart();
        }
        "email" => {
            ui::section("Setting up Email");
            ui::blank();
            println!("  For Gmail, use an App Password:");
            println!("  https://myaccount.google.com/apppasswords");
            ui::blank();

            let username = prompt_input("  Email address: ");
            if username.is_empty() {
                ui::error("No email provided. Setup cancelled.");
                return;
            }

            let password = prompt_input("  App password (or Enter to set later): ");

            let config_block = format!(
                "\n[channels.email]\nimap_host = \"imap.gmail.com\"\nimap_port = 993\nsmtp_host = \"smtp.gmail.com\"\nsmtp_port = 587\nusername = \"{username}\"\npassword_env = \"EMAIL_PASSWORD\"\npoll_interval = 30\ndefault_agent = \"assistant\"\n"
            );
            maybe_write_channel_config("email", &config_block);

            if !password.is_empty() {
                match dotenv::save_env_key("EMAIL_PASSWORD", &password) {
                    Ok(()) => ui::success("Password saved to ~/.skipper/.env"),
                    Err(_) => println!("    export EMAIL_PASSWORD=your_app_password"),
                }
            } else {
                ui::hint("Set later: skipper config set-key email (or export EMAIL_PASSWORD=...)");
            }

            ui::blank();
            ui::success("Email configured");
            notify_daemon_restart();
        }
        "signal" => {
            ui::section("Setting up Signal");
            ui::blank();
            println!("  Signal requires signal-cli (https://github.com/AsamK/signal-cli).");
            ui::blank();
            println!("  1. Install signal-cli:");
            println!("     - macOS: brew install signal-cli");
            println!("     - Linux: download from GitHub releases");
            println!("     - Or use the Docker image");
            println!("  2. Register or link a phone number:");
            println!("     signal-cli -u +1YOURPHONE register");
            println!("     signal-cli -u +1YOURPHONE verify CODE");
            println!("  3. Start signal-cli in JSON-RPC mode:");
            println!("     signal-cli -u +1YOURPHONE jsonRpc --socket /tmp/signal-cli.sock");
            ui::blank();

            let phone = prompt_input("  Your phone number (+1XXXX, or Enter to skip): ");

            let config_block = "\n[channels.signal]\nphone_env = \"SIGNAL_PHONE\"\nsocket_path = \"/tmp/signal-cli.sock\"\ndefault_agent = \"assistant\"\n";
            maybe_write_channel_config("signal", config_block);

            if !phone.is_empty() {
                match dotenv::save_env_key("SIGNAL_PHONE", &phone) {
                    Ok(()) => ui::success("Phone saved to ~/.skipper/.env"),
                    Err(_) => println!("    export SIGNAL_PHONE={phone}"),
                }
            }

            ui::blank();
            ui::success("Signal configured");
            notify_daemon_restart();
        }
        "matrix" => {
            ui::section("Setting up Matrix");
            ui::blank();
            println!("  1. Create a bot account on your Matrix homeserver");
            println!("     (e.g., register @skipper-bot:matrix.org)");
            println!("  2. Obtain an access token:");
            println!("     curl -X POST https://matrix.org/_matrix/client/r0/login \\");
            println!("       -d '{{\"type\":\"m.login.password\",\"user\":\"skipper-bot\",\"password\":\"...\"}}'");
            println!("     Copy the access_token from the response.");
            println!("  3. Invite the bot to rooms you want it to monitor.");
            ui::blank();

            let homeserver = prompt_input("  Homeserver URL [https://matrix.org]: ");
            let homeserver = if homeserver.is_empty() {
                "https://matrix.org".to_string()
            } else {
                homeserver
            };
            let token = prompt_input("  Access token: ");

            let config_block = "\n[channels.matrix]\nhomeserver_env = \"MATRIX_HOMESERVER\"\naccess_token_env = \"MATRIX_ACCESS_TOKEN\"\ndefault_agent = \"assistant\"\n";
            maybe_write_channel_config("matrix", config_block);

            let _ = dotenv::save_env_key("MATRIX_HOMESERVER", &homeserver);
            if !token.is_empty() {
                match dotenv::save_env_key("MATRIX_ACCESS_TOKEN", &token) {
                    Ok(()) => ui::success("Token saved to ~/.skipper/.env"),
                    Err(_) => println!("    export MATRIX_ACCESS_TOKEN={token}"),
                }
            }

            ui::blank();
            ui::success("Matrix configured");
            notify_daemon_restart();
        }
        other => {
            ui::error_with_fix(
                &format!("Unknown channel: {other}"),
                "Available: telegram, discord, slack, whatsapp, email, signal, matrix",
            );
            std::process::exit(1);
        }
    }
}

/// Offer to append a channel config block to config.toml if it doesn't already exist.
pub(crate) fn maybe_write_channel_config(channel: &str, config_block: &str) {
    let home = skipper_home();
    let config_path = home.join("config.toml");

    if !config_path.exists() {
        ui::hint("No config.toml found. Run `skipper init` first.");
        return;
    }

    let existing = std::fs::read_to_string(&config_path).unwrap_or_default();
    let section_header = format!("[channels.{channel}]");
    if existing.contains(&section_header) {
        ui::check_ok(&format!("{section_header} already in config.toml"));
        return;
    }

    let answer = prompt_input("  Write to config.toml? [Y/n] ");
    if answer.is_empty() || answer.starts_with('y') || answer.starts_with('Y') {
        let mut content = existing;
        content.push_str(config_block);
        if std::fs::write(&config_path, &content).is_ok() {
            restrict_file_permissions(&config_path);
            ui::check_ok(&format!("Added {section_header} to config.toml"));
        } else {
            ui::check_fail("Failed to write config.toml");
        }
    }
}

/// After channel config changes, warn user if daemon is running.
pub(crate) fn notify_daemon_restart() {
    if find_daemon().is_some() {
        ui::check_warn("Restart the daemon to activate this channel");
    } else {
        ui::hint("Start the daemon: skipper start");
    }
}

/// Test a channel by sending a test message.
pub fn cmd_channel_test(channel: &str) {
    if let Some(base) = find_daemon() {
        let client = daemon_client();
        let body = daemon_json(
            client
                .post(format!("{base}/api/channels/{channel}/test"))
                .send(),
        );
        if body.get("status").is_some() {
            println!("Test message sent to {channel}!");
        } else {
            eprintln!(
                "Failed: {}",
                body["error"].as_str().unwrap_or("Unknown error")
            );
        }
    } else {
        eprintln!("Channel test requires a running daemon. Start with: skipper start");
        std::process::exit(1);
    }
}

/// Enable or disable a channel.
pub fn cmd_channel_toggle(channel: &str, enable: bool) {
    let action = if enable { "enabled" } else { "disabled" };
    if let Some(base) = find_daemon() {
        let client = daemon_client();
        let endpoint = if enable { "enable" } else { "disable" };
        let body = daemon_json(
            client
                .post(format!("{base}/api/channels/{channel}/{endpoint}"))
                .send(),
        );
        if body.get("status").is_some() {
            println!("Channel {channel} {action}.");
        } else {
            eprintln!(
                "Failed: {}",
                body["error"].as_str().unwrap_or("Unknown error")
            );
        }
    } else {
        println!("Note: Channel {channel} will be {action} when the daemon starts.");
        println!("Edit ~/.skipper/config.toml to persist this change.");
    }
}
