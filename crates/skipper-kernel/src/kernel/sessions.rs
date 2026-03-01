use super::*;

impl SkipperKernel {
    pub fn reset_session(&self, agent_id: AgentId) -> KernelResult<()> {
        let entry = self.registry.get(agent_id).ok_or_else(|| {
            KernelError::Skipper(SkipperError::AgentNotFound(agent_id.to_string()))
        })?;

        // Auto-save session context to workspace memory before clearing
        if let Ok(Some(old_session)) = self.memory.get_session(entry.session_id) {
            if old_session.messages.len() >= 2 {
                self.save_session_summary(agent_id, &entry, &old_session);
            }
        }

        // Delete the old session
        let _ = self.memory.delete_session(entry.session_id);

        // Create a fresh session
        let new_session = self
            .memory
            .create_session(agent_id)
            .map_err(KernelError::Skipper)?;

        // Update registry with new session ID
        self.registry
            .update_session_id(agent_id, new_session.id)
            .map_err(KernelError::Skipper)?;

        info!(agent_id = %agent_id, "Session reset (summary saved to memory)");
        Ok(())
    }

    /// List all sessions for a specific agent.
    pub fn list_agent_sessions(&self, agent_id: AgentId) -> KernelResult<Vec<serde_json::Value>> {
        // Verify agent exists
        let entry = self.registry.get(agent_id).ok_or_else(|| {
            KernelError::Skipper(SkipperError::AgentNotFound(agent_id.to_string()))
        })?;

        let mut sessions = self
            .memory
            .list_agent_sessions(agent_id)
            .map_err(KernelError::Skipper)?;

        // Mark the active session
        for s in &mut sessions {
            if let Some(obj) = s.as_object_mut() {
                let is_active = obj
                    .get("session_id")
                    .and_then(|v| v.as_str())
                    .map(|sid| sid == entry.session_id.0.to_string())
                    .unwrap_or(false);
                obj.insert("active".to_string(), serde_json::json!(is_active));
            }
        }

        Ok(sessions)
    }

    /// Create a new named session for an agent.
    pub fn create_agent_session(
        &self,
        agent_id: AgentId,
        label: Option<&str>,
    ) -> KernelResult<serde_json::Value> {
        // Verify agent exists
        let _entry = self.registry.get(agent_id).ok_or_else(|| {
            KernelError::Skipper(SkipperError::AgentNotFound(agent_id.to_string()))
        })?;

        let session = self
            .memory
            .create_session_with_label(agent_id, label)
            .map_err(KernelError::Skipper)?;

        // Switch to the new session
        self.registry
            .update_session_id(agent_id, session.id)
            .map_err(KernelError::Skipper)?;

        info!(agent_id = %agent_id, label = ?label, "Created new session");

        Ok(serde_json::json!({
            "session_id": session.id.0.to_string(),
            "label": session.label,
        }))
    }

    /// Switch an agent to an existing session by session ID.
    pub fn switch_agent_session(
        &self,
        agent_id: AgentId,
        session_id: SessionId,
    ) -> KernelResult<()> {
        // Verify agent exists
        let _entry = self.registry.get(agent_id).ok_or_else(|| {
            KernelError::Skipper(SkipperError::AgentNotFound(agent_id.to_string()))
        })?;

        // Verify session exists and belongs to this agent
        let session = self
            .memory
            .get_session(session_id)
            .map_err(KernelError::Skipper)?
            .ok_or_else(|| {
                KernelError::Skipper(SkipperError::Internal("Session not found".to_string()))
            })?;

        if session.agent_id != agent_id {
            return Err(KernelError::Skipper(SkipperError::Internal(
                "Session belongs to a different agent".to_string(),
            )));
        }

        self.registry
            .update_session_id(agent_id, session_id)
            .map_err(KernelError::Skipper)?;

        info!(agent_id = %agent_id, session_id = %session_id.0, "Switched session");
        Ok(())
    }

    /// Save a summary of the current session to agent memory before reset.
    fn save_session_summary(
        &self,
        agent_id: AgentId,
        entry: &AgentEntry,
        session: &skipper_memory::session::Session,
    ) {
        use skipper_types::message::{MessageContent, Role};

        // Take last 10 messages (or all if fewer)
        let recent = &session.messages[session.messages.len().saturating_sub(10)..];

        // Extract key topics from user messages
        let topics: Vec<&str> = recent
            .iter()
            .filter(|m| m.role == Role::User)
            .filter_map(|m| match &m.content {
                MessageContent::Text(t) => Some(t.as_str()),
                _ => None,
            })
            .collect();

        if topics.is_empty() {
            return;
        }

        // Generate a slug from first user message (first 6 words, slugified)
        let slug: String = topics[0]
            .split_whitespace()
            .take(6)
            .collect::<Vec<_>>()
            .join("-")
            .to_lowercase()
            .chars()
            .filter(|c| c.is_alphanumeric() || *c == '-')
            .take(60)
            .collect();

        let date = chrono::Utc::now().format("%Y-%m-%d");
        let summary = format!(
            "Session on {date}: {slug}\n\nKey exchanges:\n{}",
            topics
                .iter()
                .take(5)
                .enumerate()
                .map(|(i, t)| {
                    let truncated = if t.len() > 200 { &t[..200] } else { t };
                    format!("{}. {}", i + 1, truncated)
                })
                .collect::<Vec<_>>()
                .join("\n")
        );

        // Save to structured memory store (key = "session_{date}_{slug}")
        let key = format!("session_{date}_{slug}");
        let _ =
            self.memory
                .structured_set(agent_id, &key, serde_json::Value::String(summary.clone()));

        // Also write to workspace memory/ dir if workspace exists
        if let Some(ref workspace) = entry.manifest.workspace {
            let mem_dir = workspace.join("memory");
            let filename = format!("{date}-{slug}.md");
            let _ = std::fs::write(mem_dir.join(&filename), &summary);
        }

        debug!(
            agent_id = %agent_id,
            key = %key,
            "Saved session summary to memory before reset"
        );
    }

    /// Cancel an agent's currently running LLM task.
    pub fn stop_agent_run(&self, agent_id: AgentId) -> KernelResult<bool> {
        if let Some((_, handle)) = self.running_tasks.remove(&agent_id) {
            handle.abort();
            info!(agent_id = %agent_id, "Agent run cancelled");
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Compact an agent's session using LLM-based summarization.
    ///
    /// Replaces the existing text-truncation compaction with an intelligent
    /// LLM-generated summary of older messages, keeping only recent messages.
    pub async fn compact_agent_session(&self, agent_id: AgentId) -> KernelResult<String> {
        use skipper_runtime::compactor::{compact_session, needs_compaction, CompactionConfig};

        let entry = self.registry.get(agent_id).ok_or_else(|| {
            KernelError::Skipper(SkipperError::AgentNotFound(agent_id.to_string()))
        })?;

        let session = self
            .memory
            .get_session(entry.session_id)
            .map_err(KernelError::Skipper)?
            .unwrap_or_else(|| skipper_memory::session::Session {
                id: entry.session_id,
                agent_id,
                messages: Vec::new(),
                context_window_tokens: 0,
                label: None,
            });

        let config = CompactionConfig::default();

        if !needs_compaction(&session, &config) {
            return Ok(format!(
                "No compaction needed ({} messages, threshold {})",
                session.messages.len(),
                config.threshold
            ));
        }

        let driver = self.resolve_driver(&entry.manifest)?;
        let model = entry.manifest.model.model.clone();

        let result = compact_session(driver, &model, &session, &config)
            .await
            .map_err(|e| KernelError::Skipper(SkipperError::Internal(e)))?;

        // Store the LLM summary in the canonical session
        self.memory
            .store_llm_summary(agent_id, &result.summary, result.kept_messages.clone())
            .map_err(KernelError::Skipper)?;

        // Post-compaction audit: validate and repair the kept messages
        let (repaired_messages, repair_stats) =
            skipper_runtime::session_repair::validate_and_repair_with_stats(&result.kept_messages);

        // Also update the regular session with the repaired messages
        let mut updated_session = session;
        updated_session.messages = repaired_messages;
        self.memory
            .save_session(&updated_session)
            .map_err(KernelError::Skipper)?;

        // Build result message with audit summary
        let mut msg = format!(
            "Compacted {} messages into summary ({} chars), kept {} recent messages.",
            result.compacted_count,
            result.summary.len(),
            updated_session.messages.len()
        );

        let repairs = repair_stats.orphaned_results_removed
            + repair_stats.synthetic_results_inserted
            + repair_stats.duplicates_removed
            + repair_stats.messages_merged;
        if repairs > 0 {
            msg.push_str(&format!(" Post-audit: repaired ({} orphaned removed, {} synthetic inserted, {} merged, {} deduped).",
                repair_stats.orphaned_results_removed,
                repair_stats.synthetic_results_inserted,
                repair_stats.messages_merged,
                repair_stats.duplicates_removed,
            ));
        } else {
            msg.push_str(" Post-audit: clean.");
        }

        Ok(msg)
    }

    /// Generate a context window usage report for an agent.
    pub fn context_report(
        &self,
        agent_id: AgentId,
    ) -> KernelResult<skipper_runtime::compactor::ContextReport> {
        use skipper_runtime::compactor::generate_context_report;
        use skipper_runtime::tool_runner::builtin_tool_definitions;

        let entry = self.registry.get(agent_id).ok_or_else(|| {
            KernelError::Skipper(SkipperError::AgentNotFound(agent_id.to_string()))
        })?;

        let session = self
            .memory
            .get_session(entry.session_id)
            .map_err(KernelError::Skipper)?
            .unwrap_or_else(|| skipper_memory::session::Session {
                id: entry.session_id,
                agent_id,
                messages: Vec::new(),
                context_window_tokens: 0,
                label: None,
            });

        let system_prompt = &entry.manifest.model.system_prompt;
        let tools = builtin_tool_definitions();
        // Use 200K default or the model's known context window
        let context_window = if session.context_window_tokens > 0 {
            session.context_window_tokens
        } else {
            200_000
        };

        Ok(generate_context_report(
            &session.messages,
            Some(system_prompt),
            Some(&tools),
            context_window as usize,
        ))
    }
}
