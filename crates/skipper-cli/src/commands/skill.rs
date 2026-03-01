//! Skill command handlers: install, list, remove, search, create.

use crate::{copy_dir_recursive, prompt_input, skipper_home};
use std::path::PathBuf;

/// Install a skill from FangHub or a local directory.
pub fn cmd_skill_install(source: &str) {
    let home = skipper_home();
    let skills_dir = home.join("skills");
    std::fs::create_dir_all(&skills_dir).unwrap_or_else(|e| {
        eprintln!("Error creating skills directory: {e}");
        std::process::exit(1);
    });

    let source_path = PathBuf::from(source);
    if source_path.exists() && source_path.is_dir() {
        // Local directory install
        let manifest_path = source_path.join("skill.toml");
        if !manifest_path.exists() {
            // Check if it's an OpenClaw skill
            if skipper_skills::openclaw_compat::detect_openclaw_skill(&source_path) {
                println!("Detected OpenClaw skill format. Converting...");
                match skipper_skills::openclaw_compat::convert_openclaw_skill(&source_path) {
                    Ok(manifest) => {
                        let dest = skills_dir.join(&manifest.skill.name);
                        // Copy skill directory
                        copy_dir_recursive(&source_path, &dest);
                        if let Err(e) = skipper_skills::openclaw_compat::write_skipper_manifest(
                            &dest, &manifest,
                        ) {
                            eprintln!("Failed to write manifest: {e}");
                            std::process::exit(1);
                        }
                        println!("Installed OpenClaw skill: {}", manifest.skill.name);
                    }
                    Err(e) => {
                        eprintln!("Failed to convert OpenClaw skill: {e}");
                        std::process::exit(1);
                    }
                }
                return;
            }
            eprintln!("No skill.toml found in {source}");
            std::process::exit(1);
        }

        // Read manifest to get skill name
        let toml_str = std::fs::read_to_string(&manifest_path).unwrap_or_else(|e| {
            eprintln!("Error reading skill.toml: {e}");
            std::process::exit(1);
        });
        let manifest: skipper_skills::SkillManifest =
            toml::from_str(&toml_str).unwrap_or_else(|e| {
                eprintln!("Error parsing skill.toml: {e}");
                std::process::exit(1);
            });

        let dest = skills_dir.join(&manifest.skill.name);
        copy_dir_recursive(&source_path, &dest);
        println!(
            "Installed skill: {} v{}",
            manifest.skill.name, manifest.skill.version
        );
    } else {
        // Remote install from FangHub
        println!("Installing {source} from FangHub...");
        let rt = tokio::runtime::Runtime::new().unwrap();
        let client = skipper_skills::marketplace::MarketplaceClient::new(
            skipper_skills::marketplace::MarketplaceConfig::default(),
        );
        match rt.block_on(client.install(source, &skills_dir)) {
            Ok(version) => println!("Installed {source} {version}"),
            Err(e) => {
                eprintln!("Failed to install skill: {e}");
                std::process::exit(1);
            }
        }
    }
}

/// List installed skills.
pub fn cmd_skill_list() {
    let home = skipper_home();
    let skills_dir = home.join("skills");

    let mut registry = skipper_skills::registry::SkillRegistry::new(skills_dir);
    match registry.load_all() {
        Ok(0) => println!("No skills installed."),
        Ok(count) => {
            println!("{count} skill(s) installed:\n");
            println!(
                "{:<20} {:<10} {:<8} DESCRIPTION",
                "NAME", "VERSION", "TOOLS"
            );
            println!("{}", "-".repeat(70));
            for skill in registry.list() {
                println!(
                    "{:<20} {:<10} {:<8} {}",
                    skill.manifest.skill.name,
                    skill.manifest.skill.version,
                    skill.manifest.tools.provided.len(),
                    skill.manifest.skill.description,
                );
            }
        }
        Err(e) => {
            eprintln!("Error loading skills: {e}");
            std::process::exit(1);
        }
    }
}

/// Remove an installed skill.
pub fn cmd_skill_remove(name: &str) {
    let home = skipper_home();
    let skills_dir = home.join("skills");

    let mut registry = skipper_skills::registry::SkillRegistry::new(skills_dir);
    let _ = registry.load_all();
    match registry.remove(name) {
        Ok(()) => println!("Removed skill: {name}"),
        Err(e) => {
            eprintln!("Failed to remove skill: {e}");
            std::process::exit(1);
        }
    }
}

/// Search FangHub for skills.
pub fn cmd_skill_search(query: &str) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let client = skipper_skills::marketplace::MarketplaceClient::new(
        skipper_skills::marketplace::MarketplaceConfig::default(),
    );
    match rt.block_on(client.search(query)) {
        Ok(results) if results.is_empty() => println!("No skills found for \"{query}\"."),
        Ok(results) => {
            println!("Skills matching \"{query}\":\n");
            for r in results {
                println!("  {} ({})", r.name, r.stars);
                if !r.description.is_empty() {
                    println!("    {}", r.description);
                }
                println!("    {}", r.url);
                println!();
            }
        }
        Err(e) => {
            eprintln!("Search failed: {e}");
            std::process::exit(1);
        }
    }
}

/// Create a new skill scaffold.
pub fn cmd_skill_create() {
    let name = prompt_input("Skill name: ");
    let description = prompt_input("Description: ");
    let runtime = prompt_input("Runtime (python/node/wasm) [python]: ");
    let runtime = if runtime.is_empty() {
        "python".to_string()
    } else {
        runtime
    };

    let home = skipper_home();
    let skill_dir = home.join("skills").join(&name);
    std::fs::create_dir_all(skill_dir.join("src")).unwrap_or_else(|e| {
        eprintln!("Error creating skill directory: {e}");
        std::process::exit(1);
    });

    let manifest = format!(
        r#"[skill]
name = "{name}"
version = "0.1.0"
description = "{description}"
author = ""
license = "MIT"
tags = []

[runtime]
type = "{runtime}"
entry = "src/main.py"

[[tools.provided]]
name = "{tool_name}"
description = "{description}"
input_schema = {{ type = "object", properties = {{ input = {{ type = "string" }} }}, required = ["input"] }}

[requirements]
tools = []
capabilities = []
"#,
        tool_name = name.replace('-', "_"),
    );

    std::fs::write(skill_dir.join("skill.toml"), &manifest).unwrap();

    // Create entry point
    let entry_content = match runtime.as_str() {
        "python" => format!(
            r#"#!/usr/bin/env python3
"""Skipper skill: {name}"""
import json
import sys

def main():
    payload = json.loads(sys.stdin.read())
    tool_name = payload["tool"]
    input_data = payload["input"]

    # TODO: Implement your skill logic here
    result = {{"result": f"Processed: {{input_data.get('input', '')}}"}}

    print(json.dumps(result))

if __name__ == "__main__":
    main()
"#
        ),
        _ => "// TODO: Implement your skill\n".to_string(),
    };

    let entry_path = if runtime == "python" {
        "src/main.py"
    } else {
        "src/index.js"
    };
    std::fs::write(skill_dir.join(entry_path), entry_content).unwrap();

    println!("\nSkill created: {}", skill_dir.display());
    println!("\nFiles:");
    println!("  skill.toml");
    println!("  {entry_path}");
    println!("\nNext steps:");
    println!("  1. Edit the entry point to implement your skill logic");
    println!("  2. Test locally: skipper skill test");
    println!(
        "  3. Install: skipper skill install {}",
        skill_dir.display()
    );
}
