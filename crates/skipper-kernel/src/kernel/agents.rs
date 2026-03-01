use super::*;

impl SkipperKernel {
    /// Spawn a new agent from a manifest, optionally linking to a parent agent.
    pub fn spawn_agent(&self, manifest: AgentManifest) -> KernelResult<AgentId> {
        self.spawn_agent_with_parent(manifest, None)
    }

    /// Spawn a new agent with an optional parent for lineage tracking.
    pub fn spawn_agent_with_parent(
        &self,
        manifest: AgentManifest,
        parent: Option<AgentId>,
    ) -> KernelResult<AgentId> {
        let agent_id = AgentId::new();
        let session_id = SessionId::new();
        let name = manifest.name.clone();

        info!(agent = %name, id = %agent_id, parent = ?parent, "Spawning agent");

        // Create session
        self.memory
            .create_session(agent_id)
            .map_err(KernelError::Skipper)?;

        // Inherit kernel exec_policy as fallback if agent manifest doesn't have one
        let mut manifest = manifest;
        if manifest.exec_policy.is_none() {
            manifest.exec_policy = Some(self.config.exec_policy.clone());
        }

        // Overlay kernel default_model onto agent if no custom key/url is set.
        // This ensures agents respect the user's configured provider from `skipper init`.
        if manifest.model.api_key_env.is_none() && manifest.model.base_url.is_none() {
            let dm = &self.config.default_model;
            if !dm.provider.is_empty() {
                manifest.model.provider = dm.provider.clone();
            }
            if !dm.model.is_empty() {
                manifest.model.model = dm.model.clone();
            }
            if dm.base_url.is_some() {
                manifest.model.base_url = dm.base_url.clone();
            }
        }

        // Create workspace directory for the agent
        let workspace_dir = manifest.workspace.clone().unwrap_or_else(|| {
            self.config.effective_workspaces_dir().join(format!(
                "{}-{}",
                &name,
                &agent_id.0.to_string()[..8]
            ))
        });
        ensure_workspace(&workspace_dir)?;
        if manifest.generate_identity_files {
            generate_identity_files(&workspace_dir, &manifest);
        }
        manifest.workspace = Some(workspace_dir);

        // Register capabilities
        let caps = manifest_to_capabilities(&manifest);
        self.capabilities.grant(agent_id, caps);

        // Register with scheduler
        self.scheduler
            .register(agent_id, manifest.resources.clone());

        // Create registry entry
        let tags = manifest.tags.clone();
        let entry = AgentEntry {
            id: agent_id,
            name: manifest.name.clone(),
            manifest,
            state: AgentState::Running,
            mode: AgentMode::default(),
            created_at: chrono::Utc::now(),
            last_active: chrono::Utc::now(),
            parent,
            children: vec![],
            session_id,
            tags,
            identity: Default::default(),
            onboarding_completed: false,
            onboarding_completed_at: None,
        };
        self.registry
            .register(entry.clone())
            .map_err(KernelError::Skipper)?;

        // Update parent's children list
        if let Some(parent_id) = parent {
            self.registry.add_child(parent_id, agent_id);
        }

        // Persist agent to SQLite so it survives restarts
        self.memory
            .save_agent(&entry)
            .map_err(KernelError::Skipper)?;

        info!(agent = %name, id = %agent_id, "Agent spawned");

        // SECURITY: Record agent spawn in audit trail
        self.audit_log.record(
            agent_id.to_string(),
            skipper_runtime::audit::AuditAction::AgentSpawn,
            format!("name={name}, parent={parent:?}"),
            "ok",
        );

        // For proactive agents spawned at runtime, auto-register triggers
        if let ScheduleMode::Proactive { conditions } = &entry.manifest.schedule {
            for condition in conditions {
                if let Some(pattern) = background::parse_condition(condition) {
                    let prompt = format!(
                        "[PROACTIVE ALERT] Condition '{condition}' matched: {{{{event}}}}. \
                         Review and take appropriate action. Agent: {name}"
                    );
                    self.triggers.register(agent_id, pattern, prompt, 0);
                }
            }
        }

        // Publish lifecycle event (triggers evaluated synchronously on the event)
        let event = Event::new(
            agent_id,
            EventTarget::Broadcast,
            EventPayload::Lifecycle(LifecycleEvent::Spawned {
                agent_id,
                name: name.clone(),
            }),
        );
        // Evaluate triggers synchronously (we can't await in a sync fn, so just evaluate)
        let _triggered = self.triggers.evaluate(&event);

        Ok(agent_id)
    }

    /// Verify a signed manifest envelope (Ed25519 + SHA-256).
    ///
    /// Call this before `spawn_agent` when a `SignedManifest` JSON is provided
    /// alongside the TOML. Returns the verified manifest TOML string on success.
    pub fn verify_signed_manifest(&self, signed_json: &str) -> KernelResult<String> {
        let signed: skipper_types::manifest_signing::SignedManifest =
            serde_json::from_str(signed_json).map_err(|e| {
                KernelError::Skipper(skipper_types::error::SkipperError::Config(format!(
                    "Invalid signed manifest JSON: {e}"
                )))
            })?;
        signed.verify().map_err(|e| {
            KernelError::Skipper(skipper_types::error::SkipperError::Config(format!(
                "Manifest signature verification failed: {e}"
            )))
        })?;
        info!(signer = %signed.signer_id, hash = %signed.content_hash, "Signed manifest verified");
        Ok(signed.manifest)
    }

    /// Kill an agent.
    pub fn kill_agent(&self, agent_id: AgentId) -> KernelResult<()> {
        let entry = self
            .registry
            .remove(agent_id)
            .map_err(KernelError::Skipper)?;
        self.background.stop_agent(agent_id);
        self.scheduler.unregister(agent_id);
        self.capabilities.revoke_all(agent_id);
        self.event_bus.unsubscribe_agent(agent_id);
        self.triggers.remove_agent_triggers(agent_id);

        // Remove from persistent storage
        let _ = self.memory.remove_agent(agent_id);

        // SECURITY: Record agent kill in audit trail
        self.audit_log.record(
            agent_id.to_string(),
            skipper_runtime::audit::AuditAction::AgentKill,
            format!("name={}", entry.name),
            "ok",
        );

        info!(agent = %entry.name, id = %agent_id, "Agent killed");
        Ok(())
    }
}

/// Convert a manifest's capability declarations into Capability enums.
///
/// If a `profile` is set and the manifest has no explicit tools, the profile's
/// implied capabilities are used as a base — preserving any non-tool overrides
/// from the manifest.
pub fn manifest_to_capabilities(manifest: &AgentManifest) -> Vec<Capability> {
    let mut caps = Vec::new();

    // Profile expansion: use profile's implied capabilities when no explicit tools
    let effective_caps = if let Some(ref profile) = manifest.profile {
        if manifest.capabilities.tools.is_empty() {
            let mut merged = profile.implied_capabilities();
            if !manifest.capabilities.network.is_empty() {
                merged.network = manifest.capabilities.network.clone();
            }
            if !manifest.capabilities.shell.is_empty() {
                merged.shell = manifest.capabilities.shell.clone();
            }
            if !manifest.capabilities.agent_message.is_empty() {
                merged.agent_message = manifest.capabilities.agent_message.clone();
            }
            if manifest.capabilities.agent_spawn {
                merged.agent_spawn = true;
            }
            if !manifest.capabilities.memory_read.is_empty() {
                merged.memory_read = manifest.capabilities.memory_read.clone();
            }
            if !manifest.capabilities.memory_write.is_empty() {
                merged.memory_write = manifest.capabilities.memory_write.clone();
            }
            if manifest.capabilities.ofp_discover {
                merged.ofp_discover = true;
            }
            if !manifest.capabilities.ofp_connect.is_empty() {
                merged.ofp_connect = manifest.capabilities.ofp_connect.clone();
            }
            merged
        } else {
            manifest.capabilities.clone()
        }
    } else {
        manifest.capabilities.clone()
    };

    for host in &effective_caps.network {
        caps.push(Capability::NetConnect(host.clone()));
    }
    for tool in &effective_caps.tools {
        caps.push(Capability::ToolInvoke(tool.clone()));
    }
    for scope in &effective_caps.memory_read {
        caps.push(Capability::MemoryRead(scope.clone()));
    }
    for scope in &effective_caps.memory_write {
        caps.push(Capability::MemoryWrite(scope.clone()));
    }
    if effective_caps.agent_spawn {
        caps.push(Capability::AgentSpawn);
    }
    for pattern in &effective_caps.agent_message {
        caps.push(Capability::AgentMessage(pattern.clone()));
    }
    for cmd in &effective_caps.shell {
        caps.push(Capability::ShellExec(cmd.clone()));
    }
    if effective_caps.ofp_discover {
        caps.push(Capability::OfpDiscover);
    }
    for peer in &effective_caps.ofp_connect {
        caps.push(Capability::OfpConnect(peer.clone()));
    }

    caps
}

/// Infer provider from a model name when catalog lookup fails.
///
/// Uses well-known model name prefixes to map to the correct provider.
/// This is a defense-in-depth fallback — models should ideally be in the catalog.
pub fn infer_provider_from_model(model: &str) -> Option<String> {
    let lower = model.to_lowercase();
    // Check for explicit provider prefix (e.g., "minimax/MiniMax-M2.5")
    if let Some(prefix) = lower.split('/').next() {
        match prefix {
            "minimax" | "gemini" | "anthropic" | "openai" | "groq" | "deepseek" | "mistral"
            | "cohere" | "xai" | "ollama" | "together" | "fireworks" | "perplexity"
            | "cerebras" | "sambanova" | "replicate" | "huggingface" | "ai21" | "codex"
            | "claude-code" | "copilot" | "github-copilot" | "qwen" | "zhipu" | "moonshot"
            | "openrouter" => {
                if model.contains('/') {
                    return Some(prefix.to_string());
                }
            }
            _ => {}
        }
    }
    // Infer from well-known model name patterns
    if lower.starts_with("minimax") {
        Some("minimax".to_string())
    } else if lower.starts_with("gemini") {
        Some("gemini".to_string())
    } else if lower.starts_with("claude") {
        Some("anthropic".to_string())
    } else if lower.starts_with("gpt") || lower.starts_with("o1") || lower.starts_with("o3") || lower.starts_with("o4") {
        Some("openai".to_string())
    } else if lower.starts_with("llama") || lower.starts_with("mixtral") || lower.starts_with("qwen") {
        // These could be on multiple providers; don't infer
        None
    } else if lower.starts_with("grok") {
        Some("xai".to_string())
    } else if lower.starts_with("deepseek") {
        Some("deepseek".to_string())
    } else if lower.starts_with("mistral") || lower.starts_with("codestral") || lower.starts_with("pixtral") {
        Some("mistral".to_string())
    } else if lower.starts_with("command") || lower.starts_with("embed-") {
        Some("cohere".to_string())
    } else if lower.starts_with("jamba") {
        Some("ai21".to_string())
    } else if lower.starts_with("sonar") {
        Some("perplexity".to_string())
    } else if lower.starts_with("glm") {
        Some("zhipu".to_string())
    } else if lower.starts_with("ernie") {
        Some("qianfan".to_string())
    } else if lower.starts_with("abab") {
        Some("minimax".to_string())
    } else {
        None
    }
}
