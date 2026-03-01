use super::*;

impl SkipperKernel {
    /// Activate a hand: check requirements, create instance, spawn agent.
    pub fn activate_hand(
        &self,
        hand_id: &str,
        config: std::collections::HashMap<String, serde_json::Value>,
    ) -> KernelResult<skipper_hands::HandInstance> {
        use skipper_hands::HandError;

        let def = self
            .hand_registry
            .get_definition(hand_id)
            .ok_or_else(|| {
                KernelError::Skipper(SkipperError::AgentNotFound(format!(
                    "Hand not found: {hand_id}"
                )))
            })?
            .clone();

        // Create the instance in the registry
        let instance = self
            .hand_registry
            .activate(hand_id, config)
            .map_err(|e| match e {
                HandError::AlreadyActive(id) => KernelError::Skipper(SkipperError::Internal(
                    format!("Hand already active: {id}"),
                )),
                other => KernelError::Skipper(SkipperError::Internal(other.to_string())),
            })?;

        // Build an agent manifest from the hand definition.
        // If the hand declares provider/model as "default", inherit the kernel's configured LLM.
        let hand_provider = if def.agent.provider == "default" {
            self.config.default_model.provider.clone()
        } else {
            def.agent.provider.clone()
        };
        let hand_model = if def.agent.model == "default" {
            self.config.default_model.model.clone()
        } else {
            def.agent.model.clone()
        };

        let mut manifest = AgentManifest {
            name: def.agent.name.clone(),
            description: def.agent.description.clone(),
            module: def.agent.module.clone(),
            model: ModelConfig {
                provider: hand_provider,
                model: hand_model,
                max_tokens: def.agent.max_tokens,
                temperature: def.agent.temperature,
                system_prompt: def.agent.system_prompt.clone(),
                api_key_env: def.agent.api_key_env.clone(),
                base_url: def.agent.base_url.clone(),
            },
            capabilities: ManifestCapabilities {
                tools: def.tools.clone(),
                ..Default::default()
            },
            tags: vec![
                format!("hand:{hand_id}"),
                format!("hand_instance:{}", instance.instance_id),
            ],
            autonomous: def.agent.max_iterations.map(|max_iter| AutonomousConfig {
                max_iterations: max_iter,
                ..Default::default()
            }),
            skills: def.skills.clone(),
            mcp_servers: def.mcp_servers.clone(),
            // Hands are curated packages — if they declare shell_exec, grant full exec access
            exec_policy: if def.tools.iter().any(|t| t == "shell_exec") {
                Some(skipper_types::config::ExecPolicy {
                    mode: skipper_types::config::ExecSecurityMode::Full,
                    timeout_secs: 300, // hands may run long commands (ffmpeg, yt-dlp)
                    no_output_timeout_secs: 120,
                    ..Default::default()
                })
            } else {
                None
            },
            ..Default::default()
        };

        // Resolve hand settings → prompt block + env vars
        let resolved = skipper_hands::resolve_settings(&def.settings, &instance.config);
        if !resolved.prompt_block.is_empty() {
            manifest.model.system_prompt = format!(
                "{}\n\n---\n\n{}",
                manifest.model.system_prompt, resolved.prompt_block
            );
        }
        if !resolved.env_vars.is_empty() {
            manifest.metadata.insert(
                "hand_allowed_env".to_string(),
                serde_json::to_value(&resolved.env_vars).unwrap_or_default(),
            );
        }

        // Inject skill content into system prompt
        if let Some(ref skill_content) = def.skill_content {
            manifest.model.system_prompt = format!(
                "{}\n\n---\n\n## Reference Knowledge\n\n{}",
                manifest.model.system_prompt, skill_content
            );
        }

        // Spawn the agent
        let agent_id = self.spawn_agent(manifest)?;

        // Link agent to instance
        self.hand_registry
            .set_agent(instance.instance_id, agent_id)
            .map_err(|e| KernelError::Skipper(SkipperError::Internal(e.to_string())))?;

        info!(
            hand = %hand_id,
            instance = %instance.instance_id,
            agent = %agent_id,
            "Hand activated with agent"
        );

        // Return instance with agent set
        Ok(self
            .hand_registry
            .get_instance(instance.instance_id)
            .unwrap_or(instance))
    }

    /// Deactivate a hand: kill agent and remove instance.
    pub fn deactivate_hand(&self, instance_id: uuid::Uuid) -> KernelResult<()> {
        let instance = self
            .hand_registry
            .deactivate(instance_id)
            .map_err(|e| KernelError::Skipper(SkipperError::Internal(e.to_string())))?;

        if let Some(agent_id) = instance.agent_id {
            if let Err(e) = self.kill_agent(agent_id) {
                warn!(agent = %agent_id, error = %e, "Failed to kill hand agent (may already be dead)");
            }
        }
        Ok(())
    }

    /// Pause a hand (marks it paused; agent stays alive but won't receive new work).
    pub fn pause_hand(&self, instance_id: uuid::Uuid) -> KernelResult<()> {
        self.hand_registry
            .pause(instance_id)
            .map_err(|e| KernelError::Skipper(SkipperError::Internal(e.to_string())))
    }

    /// Resume a paused hand.
    pub fn resume_hand(&self, instance_id: uuid::Uuid) -> KernelResult<()> {
        self.hand_registry
            .resume(instance_id)
            .map_err(|e| KernelError::Skipper(SkipperError::Internal(e.to_string())))
    }
}
