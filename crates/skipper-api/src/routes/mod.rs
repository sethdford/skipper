//! Route handlers for the Skipper API.

use crate::types::*;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use dashmap::DashMap;
use skipper_kernel::triggers::{TriggerId, TriggerPattern};
use skipper_kernel::workflow::{
    ErrorMode, StepAgent, StepMode, Workflow, WorkflowId, WorkflowStep,
};
use skipper_kernel::SkipperKernel;
use skipper_runtime::kernel_handle::KernelHandle;
use skipper_runtime::tool_runner::builtin_tool_definitions;
use skipper_types::agent::{AgentId, AgentIdentity, AgentManifest};
use std::collections::HashMap;
use std::sync::{Arc, LazyLock};
use std::time::Instant;

mod health;
pub use health::*;

mod budget;
pub use budget::*;

mod workflows;
pub use workflows::*;


mod channels;
pub use channels::*;

/// Shared application state.
///
/// The kernel is wrapped in Arc so it can serve as both the main kernel
/// and the KernelHandle for inter-agent tool access.
pub struct AppState {
    pub kernel: Arc<SkipperKernel>,
    pub started_at: Instant,
    /// Optional peer registry for OFP mesh networking status.
    pub peer_registry: Option<Arc<skipper_wire::registry::PeerRegistry>>,
    /// Channel bridge manager — held behind a Mutex so it can be swapped on hot-reload.
    pub bridge_manager: tokio::sync::Mutex<Option<skipper_channels::bridge::BridgeManager>>,
    /// Live channel config — updated on every hot-reload so list_channels() reflects reality.
    pub channels_config: tokio::sync::RwLock<skipper_types::config::ChannelsConfig>,
    /// Notify handle to trigger graceful HTTP server shutdown from the API.
    pub shutdown_notify: Arc<tokio::sync::Notify>,
}

/// POST /api/agents — Spawn a new agent.
pub async fn spawn_agent(
    State(state): State<Arc<AppState>>,
    Json(req): Json<SpawnRequest>,
) -> impl IntoResponse {
    // SECURITY: Reject oversized manifests to prevent parser memory exhaustion.
    const MAX_MANIFEST_SIZE: usize = 1024 * 1024; // 1MB
    if req.manifest_toml.len() > MAX_MANIFEST_SIZE {
        return (
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(serde_json::json!({"error": "Manifest too large (max 1MB)"})),
        );
    }

    // SECURITY: Verify Ed25519 signature when a signed manifest is provided
    if let Some(ref signed_json) = req.signed_manifest {
        match state.kernel.verify_signed_manifest(signed_json) {
            Ok(verified_toml) => {
                // Ensure the signed manifest matches the provided manifest_toml
                if verified_toml.trim() != req.manifest_toml.trim() {
                    tracing::warn!("Signed manifest content does not match manifest_toml");
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(
                            serde_json::json!({"error": "Signed manifest content does not match manifest_toml"}),
                        ),
                    );
                }
            }
            Err(e) => {
                tracing::warn!("Manifest signature verification failed: {e}");
                state.kernel.audit_log.record(
                    "system",
                    skipper_runtime::audit::AuditAction::AuthAttempt,
                    "manifest signature verification failed",
                    format!("error: {e}"),
                );
                return (
                    StatusCode::FORBIDDEN,
                    Json(serde_json::json!({"error": "Manifest signature verification failed"})),
                );
            }
        }
    }

    let manifest: AgentManifest = match toml::from_str(&req.manifest_toml) {
        Ok(m) => m,
        Err(e) => {
            tracing::warn!("Invalid manifest TOML: {e}");
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Invalid manifest format"})),
            );
        }
    };

    let name = manifest.name.clone();
    match state.kernel.spawn_agent(manifest) {
        Ok(id) => (
            StatusCode::CREATED,
            Json(serde_json::json!(SpawnResponse {
                agent_id: id.to_string(),
                name,
            })),
        ),
        Err(e) => {
            tracing::warn!("Spawn failed: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "Agent spawn failed"})),
            )
        }
    }
}

/// GET /api/agents — List all agents.
pub async fn list_agents(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let agents: Vec<serde_json::Value> = state
        .kernel
        .registry
        .list()
        .into_iter()
        .map(|e| {
            serde_json::json!({
                "id": e.id.to_string(),
                "name": e.name,
                "state": format!("{:?}", e.state),
                "mode": e.mode,
                "created_at": e.created_at.to_rfc3339(),
                "model_provider": e.manifest.model.provider,
                "model_name": e.manifest.model.model,
                "profile": e.manifest.profile,
                "identity": {
                    "emoji": e.identity.emoji,
                    "avatar_url": e.identity.avatar_url,
                    "color": e.identity.color,
                },
            })
        })
        .collect();

    Json(agents)
}

/// Resolve uploaded file attachments into ContentBlock::Image blocks.
///
/// Reads each file from the upload directory, base64-encodes it, and
/// returns image content blocks ready to insert into a session message.
pub fn resolve_attachments(
    attachments: &[AttachmentRef],
) -> Vec<skipper_types::message::ContentBlock> {
    use base64::Engine;

    let upload_dir = std::env::temp_dir().join("skipper_uploads");
    let mut blocks = Vec::new();

    for att in attachments {
        // Look up metadata from the upload registry
        let meta = UPLOAD_REGISTRY.get(&att.file_id);
        let content_type = if let Some(ref m) = meta {
            m.content_type.clone()
        } else if !att.content_type.is_empty() {
            att.content_type.clone()
        } else {
            continue; // Skip unknown attachments
        };

        // Only process image types
        if !content_type.starts_with("image/") {
            continue;
        }

        // Validate file_id is a UUID to prevent path traversal
        if uuid::Uuid::parse_str(&att.file_id).is_err() {
            continue;
        }

        let file_path = upload_dir.join(&att.file_id);
        match std::fs::read(&file_path) {
            Ok(data) => {
                let b64 = base64::engine::general_purpose::STANDARD.encode(&data);
                blocks.push(skipper_types::message::ContentBlock::Image {
                    media_type: content_type,
                    data: b64,
                });
            }
            Err(e) => {
                tracing::warn!(file_id = %att.file_id, error = %e, "Failed to read upload for attachment");
            }
        }
    }

    blocks
}

/// Pre-insert image attachments into an agent's session so the LLM can see them.
///
/// This injects image content blocks into the session BEFORE the kernel
/// adds the text user message, so the LLM receives: [..., User(images), User(text)].
pub fn inject_attachments_into_session(
    kernel: &SkipperKernel,
    agent_id: AgentId,
    image_blocks: Vec<skipper_types::message::ContentBlock>,
) {
    use skipper_types::message::{Message, MessageContent, Role};

    let entry = match kernel.registry.get(agent_id) {
        Some(e) => e,
        None => return,
    };

    let mut session = match kernel.memory.get_session(entry.session_id) {
        Ok(Some(s)) => s,
        _ => skipper_memory::session::Session {
            id: entry.session_id,
            agent_id,
            messages: Vec::new(),
            context_window_tokens: 0,
            label: None,
        },
    };

    session.messages.push(Message {
        role: Role::User,
        content: MessageContent::Blocks(image_blocks),
    });

    if let Err(e) = kernel.memory.save_session(&session) {
        tracing::warn!(error = %e, "Failed to save session with image attachments");
    }
}

/// POST /api/agents/:id/message — Send a message to an agent.
pub async fn send_message(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<MessageRequest>,
) -> impl IntoResponse {
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Invalid agent ID"})),
            );
        }
    };

    // SECURITY: Reject oversized messages to prevent OOM / LLM token abuse.
    const MAX_MESSAGE_SIZE: usize = 64 * 1024; // 64KB
    if req.message.len() > MAX_MESSAGE_SIZE {
        return (
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(serde_json::json!({"error": "Message too large (max 64KB)"})),
        );
    }

    // Resolve file attachments into image content blocks
    if !req.attachments.is_empty() {
        let image_blocks = resolve_attachments(&req.attachments);
        if !image_blocks.is_empty() {
            inject_attachments_into_session(&state.kernel, agent_id, image_blocks);
        }
    }

    let kernel_handle: Arc<dyn KernelHandle> = state.kernel.clone() as Arc<dyn KernelHandle>;
    match state
        .kernel
        .send_message_with_handle(agent_id, &req.message, Some(kernel_handle))
        .await
    {
        Ok(result) => {
            // Guard: ensure we never return an empty response to the client
            let response = if result.response.trim().is_empty() {
                format!(
                    "[The agent completed processing but returned no text response. ({} in / {} out | {} iter)]",
                    result.total_usage.input_tokens,
                    result.total_usage.output_tokens,
                    result.iterations,
                )
            } else {
                result.response
            };
            (
                StatusCode::OK,
                Json(serde_json::json!(MessageResponse {
                    response,
                    input_tokens: result.total_usage.input_tokens,
                    output_tokens: result.total_usage.output_tokens,
                    iterations: result.iterations,
                    cost_usd: result.cost_usd,
                })),
            )
        }
        Err(e) => {
            tracing::warn!("send_message failed for agent {id}: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": format!("Message delivery failed: {e}")})),
            )
        }
    }
}

/// GET /api/agents/:id/session — Get agent session (conversation history).
pub async fn get_agent_session(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Invalid agent ID"})),
            );
        }
    };

    let entry = match state.kernel.registry.get(agent_id) {
        Some(e) => e,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "Agent not found"})),
            );
        }
    };

    match state.kernel.memory.get_session(entry.session_id) {
        Ok(Some(session)) => {
            let messages: Vec<serde_json::Value> = session
                .messages
                .iter()
                .filter_map(|m| {
                    let mut tools: Vec<serde_json::Value> = Vec::new();
                    let content = match &m.content {
                        skipper_types::message::MessageContent::Text(t) => t.clone(),
                        skipper_types::message::MessageContent::Blocks(blocks) => {
                            // Extract human-readable text and tool info from blocks
                            let mut texts = Vec::new();
                            for b in blocks {
                                match b {
                                    skipper_types::message::ContentBlock::Text { text } => {
                                        texts.push(text.clone());
                                    }
                                    skipper_types::message::ContentBlock::Image { .. } => {
                                        texts.push("[Image]".to_string());
                                    }
                                    skipper_types::message::ContentBlock::ToolUse {
                                        name, ..
                                    } => {
                                        tools.push(serde_json::json!({
                                            "name": name,
                                            "running": false,
                                            "expanded": false,
                                        }));
                                    }
                                    skipper_types::message::ContentBlock::ToolResult {
                                        content: result,
                                        is_error,
                                        ..
                                    } => {
                                        // Attach result to the most recent tool without a result
                                        if let Some(last_tool) = tools.last_mut() {
                                            let preview: String =
                                                result.chars().take(300).collect();
                                            last_tool["result"] =
                                                serde_json::Value::String(preview);
                                            last_tool["is_error"] =
                                                serde_json::Value::Bool(*is_error);
                                        }
                                    }
                                    _ => {}
                                }
                            }
                            texts.join("\n")
                        }
                    };
                    // Skip messages that are purely tool results (User role with only ToolResult blocks)
                    if content.is_empty() && tools.is_empty() {
                        return None;
                    }
                    let mut msg = serde_json::json!({
                        "role": format!("{:?}", m.role),
                        "content": content,
                    });
                    if !tools.is_empty() {
                        msg["tools"] = serde_json::Value::Array(tools);
                    }
                    Some(msg)
                })
                .collect();
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "session_id": session.id.0.to_string(),
                    "agent_id": session.agent_id.0.to_string(),
                    "message_count": session.messages.len(),
                    "context_window_tokens": session.context_window_tokens,
                    "label": session.label,
                    "messages": messages,
                })),
            )
        }
        Ok(None) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "session_id": entry.session_id.0.to_string(),
                "agent_id": agent_id.to_string(),
                "message_count": 0,
                "context_window_tokens": 0,
                "messages": [],
            })),
        ),
        Err(e) => {
            tracing::warn!("Session load failed for agent {id}: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "Session load failed"})),
            )
        }
    }
}

/// DELETE /api/agents/:id — Kill an agent.
pub async fn kill_agent(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Invalid agent ID"})),
            );
        }
    };

    match state.kernel.kill_agent(agent_id) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "killed", "agent_id": id})),
        ),
        Err(e) => {
            tracing::warn!("kill_agent failed for {id}: {e}");
            (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "Agent not found or already terminated"})),
            )
        }
    }
}


// ---------------------------------------------------------------------------
// Profile + Mode endpoints
// ---------------------------------------------------------------------------

/// GET /api/profiles — List all tool profiles and their tool lists.
pub async fn list_profiles() -> impl IntoResponse {
    use skipper_types::agent::ToolProfile;

    let profiles = [
        ("minimal", ToolProfile::Minimal),
        ("coding", ToolProfile::Coding),
        ("research", ToolProfile::Research),
        ("messaging", ToolProfile::Messaging),
        ("automation", ToolProfile::Automation),
        ("full", ToolProfile::Full),
    ];

    let result: Vec<serde_json::Value> = profiles
        .iter()
        .map(|(name, profile)| {
            serde_json::json!({
                "name": name,
                "tools": profile.tools(),
            })
        })
        .collect();

    Json(result)
}

/// PUT /api/agents/:id/mode — Change an agent's operational mode.
pub async fn set_agent_mode(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<SetModeRequest>,
) -> impl IntoResponse {
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Invalid agent ID"})),
            );
        }
    };

    match state.kernel.registry.set_mode(agent_id, body.mode) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "updated",
                "agent_id": id,
                "mode": body.mode,
            })),
        ),
        Err(_) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "Agent not found"})),
        ),
    }
}

// ---------------------------------------------------------------------------
// Version endpoint
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Single agent detail + SSE streaming
// ---------------------------------------------------------------------------

/// GET /api/agents/:id — Get a single agent's detailed info.
pub async fn get_agent(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Invalid agent ID"})),
            );
        }
    };

    let entry = match state.kernel.registry.get(agent_id) {
        Some(e) => e,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "Agent not found"})),
            );
        }
    };

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "id": entry.id.to_string(),
            "name": entry.name,
            "state": format!("{:?}", entry.state),
            "mode": entry.mode,
            "profile": entry.manifest.profile,
            "created_at": entry.created_at.to_rfc3339(),
            "session_id": entry.session_id.0.to_string(),
            "model": {
                "provider": entry.manifest.model.provider,
                "model": entry.manifest.model.model,
            },
            "capabilities": {
                "tools": entry.manifest.capabilities.tools,
                "network": entry.manifest.capabilities.network,
            },
            "description": entry.manifest.description,
            "tags": entry.manifest.tags,
            "identity": {
                "emoji": entry.identity.emoji,
                "avatar_url": entry.identity.avatar_url,
                "color": entry.identity.color,
            },
            "skills": entry.manifest.skills,
            "skills_mode": if entry.manifest.skills.is_empty() { "all" } else { "allowlist" },
            "mcp_servers": entry.manifest.mcp_servers,
            "mcp_servers_mode": if entry.manifest.mcp_servers.is_empty() { "all" } else { "allowlist" },
        })),
    )
}

/// POST /api/agents/:id/message/stream — SSE streaming response.
pub async fn send_message_stream(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<MessageRequest>,
) -> axum::response::Response {
    use axum::response::sse::{Event, Sse};
    use futures::stream;
    use skipper_runtime::llm_driver::StreamEvent;

    // SECURITY: Reject oversized messages to prevent OOM / LLM token abuse.
    const MAX_MESSAGE_SIZE: usize = 64 * 1024; // 64KB
    if req.message.len() > MAX_MESSAGE_SIZE {
        return (
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(serde_json::json!({"error": "Message too large (max 64KB)"})),
        )
            .into_response();
    }

    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Invalid agent ID"})),
            )
                .into_response();
        }
    };

    if state.kernel.registry.get(agent_id).is_none() {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "Agent not found"})),
        )
            .into_response();
    }

    let kernel_handle: Arc<dyn KernelHandle> = state.kernel.clone() as Arc<dyn KernelHandle>;
    let (rx, _handle) =
        match state
            .kernel
            .send_message_streaming(agent_id, &req.message, Some(kernel_handle))
        {
            Ok(pair) => pair,
            Err(e) => {
                tracing::warn!("Streaming message failed for agent {id}: {e}");
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({"error": "Streaming message failed"})),
                )
                    .into_response();
            }
        };

    let sse_stream = stream::unfold(rx, |mut rx| async move {
        match rx.recv().await {
            Some(event) => {
                let sse_event: Result<Event, std::convert::Infallible> = Ok(match event {
                    StreamEvent::TextDelta { text } => Event::default()
                        .event("chunk")
                        .json_data(serde_json::json!({"content": text, "done": false}))
                        .unwrap_or_else(|_| Event::default().data("error")),
                    StreamEvent::ToolUseStart { name, .. } => Event::default()
                        .event("tool_use")
                        .json_data(serde_json::json!({"tool": name}))
                        .unwrap_or_else(|_| Event::default().data("error")),
                    StreamEvent::ToolUseEnd { name, input, .. } => Event::default()
                        .event("tool_result")
                        .json_data(serde_json::json!({"tool": name, "input": input}))
                        .unwrap_or_else(|_| Event::default().data("error")),
                    StreamEvent::ContentComplete { usage, .. } => Event::default()
                        .event("done")
                        .json_data(serde_json::json!({
                            "done": true,
                            "usage": {
                                "input_tokens": usage.input_tokens,
                                "output_tokens": usage.output_tokens,
                            }
                        }))
                        .unwrap_or_else(|_| Event::default().data("error")),
                    StreamEvent::PhaseChange { phase, detail } => Event::default()
                        .event("phase")
                        .json_data(serde_json::json!({
                            "phase": phase,
                            "detail": detail,
                        }))
                        .unwrap_or_else(|_| Event::default().data("error")),
                    _ => Event::default().comment("skip"),
                });
                Some((sse_event, rx))
            }
            None => None,
        }
    });

    Sse::new(sse_stream).into_response()
}

/// GET /api/memory/agents/:id/kv — List KV pairs for an agent.
pub async fn get_agent_kv(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Invalid agent ID"})),
            );
        }
    };

    match state.kernel.memory.list_kv(agent_id) {
        Ok(pairs) => {
            let kv: Vec<serde_json::Value> = pairs
                .into_iter()
                .map(|(k, v)| serde_json::json!({"key": k, "value": v}))
                .collect();
            (StatusCode::OK, Json(serde_json::json!({"kv_pairs": kv})))
        }
        Err(e) => {
            tracing::warn!("Memory list_kv failed for agent {id}: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "Memory operation failed"})),
            )
        }
    }
}

/// GET /api/memory/agents/:id/kv/:key — Get a specific KV value.
pub async fn get_agent_kv_key(
    State(state): State<Arc<AppState>>,
    Path((id, key)): Path<(String, String)>,
) -> impl IntoResponse {
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Invalid agent ID"})),
            );
        }
    };

    match state.kernel.memory.structured_get(agent_id, &key) {
        Ok(Some(val)) => (
            StatusCode::OK,
            Json(serde_json::json!({"key": key, "value": val})),
        ),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "Key not found"})),
        ),
        Err(e) => {
            tracing::warn!("Memory get failed for agent {id}, key '{key}': {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "Memory operation failed"})),
            )
        }
    }
}

/// PUT /api/memory/agents/:id/kv/:key — Set a KV value.
pub async fn set_agent_kv_key(
    State(state): State<Arc<AppState>>,
    Path((id, key)): Path<(String, String)>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Invalid agent ID"})),
            );
        }
    };

    let value = body.get("value").cloned().unwrap_or(body);

    match state.kernel.memory.structured_set(agent_id, &key, value) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "stored", "key": key})),
        ),
        Err(e) => {
            tracing::warn!("Memory set failed for agent {id}, key '{key}': {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "Memory operation failed"})),
            )
        }
    }
}

/// DELETE /api/memory/agents/:id/kv/:key — Delete a KV value.
pub async fn delete_agent_kv_key(
    State(state): State<Arc<AppState>>,
    Path((id, key)): Path<(String, String)>,
) -> impl IntoResponse {
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Invalid agent ID"})),
            );
        }
    };

    match state.kernel.memory.structured_delete(agent_id, &key) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "deleted", "key": key})),
        ),
        Err(e) => {
            tracing::warn!("Memory delete failed for agent {id}, key '{key}': {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "Memory operation failed"})),
            )
        }
    }
}

// ---------------------------------------------------------------------------
// Skills endpoints
// ---------------------------------------------------------------------------

/// GET /api/skills — List installed skills.
pub async fn list_skills(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let skills_dir = state.kernel.config.home_dir.join("skills");
    let mut registry = skipper_skills::registry::SkillRegistry::new(skills_dir);
    let _ = registry.load_all();

    let skills: Vec<serde_json::Value> = registry
        .list()
        .iter()
        .map(|s| {
            let source = match &s.manifest.source {
                Some(skipper_skills::SkillSource::ClawHub { slug, version }) => {
                    serde_json::json!({"type": "clawhub", "slug": slug, "version": version})
                }
                Some(skipper_skills::SkillSource::OpenClaw) => {
                    serde_json::json!({"type": "openclaw"})
                }
                Some(skipper_skills::SkillSource::Bundled) => {
                    serde_json::json!({"type": "bundled"})
                }
                Some(skipper_skills::SkillSource::Native) | None => {
                    serde_json::json!({"type": "local"})
                }
            };
            serde_json::json!({
                "name": s.manifest.skill.name,
                "description": s.manifest.skill.description,
                "version": s.manifest.skill.version,
                "author": s.manifest.skill.author,
                "runtime": format!("{:?}", s.manifest.runtime.runtime_type),
                "tools_count": s.manifest.tools.provided.len(),
                "tags": s.manifest.skill.tags,
                "enabled": s.enabled,
                "source": source,
                "has_prompt_context": s.manifest.prompt_context.is_some(),
            })
        })
        .collect();

    Json(serde_json::json!({ "skills": skills, "total": skills.len() }))
}

/// POST /api/skills/install — Install a skill from SkipperHub (GitHub).
pub async fn install_skill(
    State(state): State<Arc<AppState>>,
    Json(req): Json<SkillInstallRequest>,
) -> impl IntoResponse {
    let skills_dir = state.kernel.config.home_dir.join("skills");
    let config = skipper_skills::marketplace::MarketplaceConfig::default();
    let client = skipper_skills::marketplace::MarketplaceClient::new(config);

    match client.install(&req.name, &skills_dir).await {
        Ok(version) => {
            // Hot-reload so agents see the new skill immediately
            state.kernel.reload_skills();
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "status": "installed",
                    "name": req.name,
                    "version": version,
                })),
            )
        }
        Err(e) => {
            tracing::warn!("Skill install failed: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": format!("Install failed: {e}")})),
            )
        }
    }
}

/// POST /api/skills/uninstall — Uninstall a skill.
pub async fn uninstall_skill(
    State(state): State<Arc<AppState>>,
    Json(req): Json<SkillUninstallRequest>,
) -> impl IntoResponse {
    let skills_dir = state.kernel.config.home_dir.join("skills");
    let mut registry = skipper_skills::registry::SkillRegistry::new(skills_dir);
    let _ = registry.load_all();

    match registry.remove(&req.name) {
        Ok(()) => {
            // Hot-reload so agents stop seeing the removed skill
            state.kernel.reload_skills();
            (
                StatusCode::OK,
                Json(serde_json::json!({"status": "uninstalled", "name": req.name})),
            )
        }
        Err(e) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": format!("{e}")})),
        ),
    }
}

/// GET /api/marketplace/search — Search the SkipperHub marketplace.
pub async fn marketplace_search(
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let query = params.get("q").cloned().unwrap_or_default();
    if query.is_empty() {
        return Json(serde_json::json!({"results": [], "total": 0}));
    }

    let config = skipper_skills::marketplace::MarketplaceConfig::default();
    let client = skipper_skills::marketplace::MarketplaceClient::new(config);

    match client.search(&query).await {
        Ok(results) => {
            let items: Vec<serde_json::Value> = results
                .iter()
                .map(|r| {
                    serde_json::json!({
                        "name": r.name,
                        "description": r.description,
                        "stars": r.stars,
                        "url": r.url,
                    })
                })
                .collect();
            Json(serde_json::json!({"results": items, "total": items.len()}))
        }
        Err(e) => {
            tracing::warn!("Marketplace search failed: {e}");
            Json(serde_json::json!({"results": [], "total": 0, "error": format!("{e}")}))
        }
    }
}

// ---------------------------------------------------------------------------
// ClawHub (OpenClaw ecosystem) endpoints
// ---------------------------------------------------------------------------

/// GET /api/clawhub/search — Search ClawHub skills using vector/semantic search.
///
/// Query parameters:
/// - `q` — search query (required)
/// - `limit` — max results (default: 20, max: 50)
pub async fn clawhub_search(
    State(state): State<Arc<AppState>>,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let query = params.get("q").cloned().unwrap_or_default();
    if query.is_empty() {
        return (
            StatusCode::OK,
            Json(serde_json::json!({"items": [], "next_cursor": null})),
        );
    }

    let limit: u32 = params
        .get("limit")
        .and_then(|v| v.parse().ok())
        .unwrap_or(20);

    let cache_dir = state.kernel.config.home_dir.join(".cache").join("clawhub");
    let client = skipper_skills::clawhub::ClawHubClient::new(cache_dir);

    match client.search(&query, limit).await {
        Ok(results) => {
            let items: Vec<serde_json::Value> = results
                .results
                .iter()
                .map(|e| {
                    serde_json::json!({
                        "slug": e.slug,
                        "name": e.display_name,
                        "description": e.summary,
                        "version": e.version,
                        "score": e.score,
                        "updated_at": e.updated_at,
                    })
                })
                .collect();
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "items": items,
                    "next_cursor": null,
                })),
            )
        }
        Err(e) => {
            tracing::warn!("ClawHub search failed: {e}");
            (
                StatusCode::OK,
                Json(
                    serde_json::json!({"items": [], "next_cursor": null, "error": format!("{e}")}),
                ),
            )
        }
    }
}

/// GET /api/clawhub/browse — Browse ClawHub skills by sort order.
///
/// Query parameters:
/// - `sort` — sort order: "trending", "downloads", "stars", "updated", "rating" (default: "trending")
/// - `limit` — max results (default: 20, max: 50)
/// - `cursor` — pagination cursor from previous response
pub async fn clawhub_browse(
    State(state): State<Arc<AppState>>,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let sort = match params.get("sort").map(|s| s.as_str()) {
        Some("downloads") => skipper_skills::clawhub::ClawHubSort::Downloads,
        Some("stars") => skipper_skills::clawhub::ClawHubSort::Stars,
        Some("updated") => skipper_skills::clawhub::ClawHubSort::Updated,
        Some("rating") => skipper_skills::clawhub::ClawHubSort::Rating,
        _ => skipper_skills::clawhub::ClawHubSort::Trending,
    };

    let limit: u32 = params
        .get("limit")
        .and_then(|v| v.parse().ok())
        .unwrap_or(20);

    let cursor = params.get("cursor").map(|s| s.as_str());

    let cache_dir = state.kernel.config.home_dir.join(".cache").join("clawhub");
    let client = skipper_skills::clawhub::ClawHubClient::new(cache_dir);

    match client.browse(sort, limit, cursor).await {
        Ok(results) => {
            let items: Vec<serde_json::Value> = results
                .items
                .iter()
                .map(clawhub_browse_entry_to_json)
                .collect();
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "items": items,
                    "next_cursor": results.next_cursor,
                })),
            )
        }
        Err(e) => {
            tracing::warn!("ClawHub browse failed: {e}");
            (
                StatusCode::OK,
                Json(
                    serde_json::json!({"items": [], "next_cursor": null, "error": format!("{e}")}),
                ),
            )
        }
    }
}

/// GET /api/clawhub/skill/{slug} — Get detailed info about a ClawHub skill.
pub async fn clawhub_skill_detail(
    State(state): State<Arc<AppState>>,
    Path(slug): Path<String>,
) -> impl IntoResponse {
    let cache_dir = state.kernel.config.home_dir.join(".cache").join("clawhub");
    let client = skipper_skills::clawhub::ClawHubClient::new(cache_dir);

    let skills_dir = state.kernel.config.home_dir.join("skills");
    let is_installed = client.is_installed(&slug, &skills_dir);

    match client.get_skill(&slug).await {
        Ok(detail) => {
            let version = detail
                .latest_version
                .as_ref()
                .map(|v| v.version.as_str())
                .unwrap_or("");
            let author = detail
                .owner
                .as_ref()
                .map(|o| o.handle.as_str())
                .unwrap_or("");
            let author_name = detail
                .owner
                .as_ref()
                .map(|o| o.display_name.as_str())
                .unwrap_or("");
            let author_image = detail
                .owner
                .as_ref()
                .map(|o| o.image.as_str())
                .unwrap_or("");

            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "slug": detail.skill.slug,
                    "name": detail.skill.display_name,
                    "description": detail.skill.summary,
                    "version": version,
                    "downloads": detail.skill.stats.downloads,
                    "stars": detail.skill.stats.stars,
                    "author": author,
                    "author_name": author_name,
                    "author_image": author_image,
                    "tags": detail.skill.tags,
                    "updated_at": detail.skill.updated_at,
                    "created_at": detail.skill.created_at,
                    "installed": is_installed,
                })),
            )
        }
        Err(e) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": format!("{e}")})),
        ),
    }
}

/// POST /api/clawhub/install — Install a skill from ClawHub.
///
/// Runs the full security pipeline: SHA256 verification, format detection,
/// manifest security scan, prompt injection scan, and binary dependency check.
pub async fn clawhub_install(
    State(state): State<Arc<AppState>>,
    Json(req): Json<crate::types::ClawHubInstallRequest>,
) -> impl IntoResponse {
    let skills_dir = state.kernel.config.home_dir.join("skills");
    let cache_dir = state.kernel.config.home_dir.join(".cache").join("clawhub");
    let client = skipper_skills::clawhub::ClawHubClient::new(cache_dir);

    // Check if already installed
    if client.is_installed(&req.slug, &skills_dir) {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "error": format!("Skill '{}' is already installed", req.slug),
                "status": "already_installed",
            })),
        );
    }

    match client.install(&req.slug, &skills_dir).await {
        Ok(result) => {
            let warnings: Vec<serde_json::Value> = result
                .warnings
                .iter()
                .map(|w| {
                    serde_json::json!({
                        "severity": format!("{:?}", w.severity),
                        "message": w.message,
                    })
                })
                .collect();

            let translations: Vec<serde_json::Value> = result
                .tool_translations
                .iter()
                .map(|(from, to)| serde_json::json!({"from": from, "to": to}))
                .collect();

            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "status": "installed",
                    "name": result.skill_name,
                    "version": result.version,
                    "slug": result.slug,
                    "is_prompt_only": result.is_prompt_only,
                    "warnings": warnings,
                    "tool_translations": translations,
                })),
            )
        }
        Err(e) => {
            let status = if e.to_string().contains("SecurityBlocked") {
                StatusCode::FORBIDDEN
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            };
            tracing::warn!("ClawHub install failed: {e}");
            (status, Json(serde_json::json!({"error": format!("{e}")})))
        }
    }
}

/// Convert a browse entry (nested stats/tags) to a flat JSON object for the frontend.
fn clawhub_browse_entry_to_json(
    entry: &skipper_skills::clawhub::ClawHubBrowseEntry,
) -> serde_json::Value {
    let version = skipper_skills::clawhub::ClawHubClient::entry_version(entry);
    serde_json::json!({
        "slug": entry.slug,
        "name": entry.display_name,
        "description": entry.summary,
        "version": version,
        "downloads": entry.stats.downloads,
        "stars": entry.stats.stars,
        "updated_at": entry.updated_at,
    })
}

// ---------------------------------------------------------------------------
// Hands endpoints
// ---------------------------------------------------------------------------

/// Detect the server platform for install command selection.
fn server_platform() -> &'static str {
    if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else {
        "linux"
    }
}

/// GET /api/hands — List all hand definitions (marketplace).
pub async fn list_hands(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let defs = state.kernel.hand_registry.list_definitions();
    let hands: Vec<serde_json::Value> = defs
        .iter()
        .map(|d| {
            let reqs = state
                .kernel
                .hand_registry
                .check_requirements(&d.id)
                .unwrap_or_default();
            let all_satisfied = reqs.iter().all(|(_, ok)| *ok);
            serde_json::json!({
                "id": d.id,
                "name": d.name,
                "description": d.description,
                "category": d.category,
                "icon": d.icon,
                "tools": d.tools,
                "requirements_met": all_satisfied,
                "requirements": reqs.iter().map(|(r, ok)| serde_json::json!({
                    "key": r.key,
                    "label": r.label,
                    "satisfied": ok,
                })).collect::<Vec<_>>(),
                "dashboard_metrics": d.dashboard.metrics.len(),
                "has_settings": !d.settings.is_empty(),
                "settings_count": d.settings.len(),
            })
        })
        .collect();

    Json(serde_json::json!({ "hands": hands, "total": hands.len() }))
}

/// GET /api/hands/active — List active hand instances.
pub async fn list_active_hands(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let instances = state.kernel.hand_registry.list_instances();
    let items: Vec<serde_json::Value> = instances
        .iter()
        .map(|i| {
            serde_json::json!({
                "instance_id": i.instance_id,
                "hand_id": i.hand_id,
                "status": format!("{}", i.status),
                "agent_id": i.agent_id.map(|a| a.to_string()),
                "agent_name": i.agent_name,
                "activated_at": i.activated_at.to_rfc3339(),
                "updated_at": i.updated_at.to_rfc3339(),
            })
        })
        .collect();

    Json(serde_json::json!({ "instances": items, "total": items.len() }))
}

/// GET /api/hands/{hand_id} — Get a single hand definition with requirements check.
pub async fn get_hand(
    State(state): State<Arc<AppState>>,
    Path(hand_id): Path<String>,
) -> impl IntoResponse {
    match state.kernel.hand_registry.get_definition(&hand_id) {
        Some(def) => {
            let reqs = state
                .kernel
                .hand_registry
                .check_requirements(&hand_id)
                .unwrap_or_default();
            let all_satisfied = reqs.iter().all(|(_, ok)| *ok);
            let settings_status = state
                .kernel
                .hand_registry
                .check_settings_availability(&hand_id)
                .unwrap_or_default();
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "id": def.id,
                    "name": def.name,
                    "description": def.description,
                    "category": def.category,
                    "icon": def.icon,
                    "tools": def.tools,
                    "requirements_met": all_satisfied,
                    "requirements": reqs.iter().map(|(r, ok)| {
                        let mut req_json = serde_json::json!({
                            "key": r.key,
                            "label": r.label,
                            "type": format!("{:?}", r.requirement_type),
                            "check_value": r.check_value,
                            "satisfied": ok,
                        });
                        if let Some(ref desc) = r.description {
                            req_json["description"] = serde_json::json!(desc);
                        }
                        if let Some(ref install) = r.install {
                            req_json["install"] = serde_json::to_value(install).unwrap_or_default();
                        }
                        req_json
                    }).collect::<Vec<_>>(),
                    "server_platform": server_platform(),
                    "agent": {
                        "name": def.agent.name,
                        "description": def.agent.description,
                        "provider": if def.agent.provider == "default" {
                            &state.kernel.config.default_model.provider
                        } else { &def.agent.provider },
                        "model": if def.agent.model == "default" {
                            &state.kernel.config.default_model.model
                        } else { &def.agent.model },
                    },
                    "dashboard": def.dashboard.metrics.iter().map(|m| serde_json::json!({
                        "label": m.label,
                        "memory_key": m.memory_key,
                        "format": m.format,
                    })).collect::<Vec<_>>(),
                    "settings": settings_status,
                })),
            )
        }
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": format!("Hand not found: {hand_id}")})),
        ),
    }
}

/// POST /api/hands/{hand_id}/check-deps — Re-check dependency status for a hand.
pub async fn check_hand_deps(
    State(state): State<Arc<AppState>>,
    Path(hand_id): Path<String>,
) -> impl IntoResponse {
    match state.kernel.hand_registry.get_definition(&hand_id) {
        Some(def) => {
            let reqs = state
                .kernel
                .hand_registry
                .check_requirements(&hand_id)
                .unwrap_or_default();
            let all_satisfied = reqs.iter().all(|(_, ok)| *ok);
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "hand_id": def.id,
                    "requirements_met": all_satisfied,
                    "server_platform": server_platform(),
                    "requirements": reqs.iter().map(|(r, ok)| {
                        let mut req_json = serde_json::json!({
                            "key": r.key,
                            "label": r.label,
                            "type": format!("{:?}", r.requirement_type),
                            "check_value": r.check_value,
                            "satisfied": ok,
                        });
                        if let Some(ref desc) = r.description {
                            req_json["description"] = serde_json::json!(desc);
                        }
                        if let Some(ref install) = r.install {
                            req_json["install"] = serde_json::to_value(install).unwrap_or_default();
                        }
                        req_json
                    }).collect::<Vec<_>>(),
                })),
            )
        }
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": format!("Hand not found: {hand_id}")})),
        ),
    }
}

/// POST /api/hands/{hand_id}/install-deps — Auto-install missing dependencies for a hand.
pub async fn install_hand_deps(
    State(state): State<Arc<AppState>>,
    Path(hand_id): Path<String>,
) -> impl IntoResponse {
    let def = match state.kernel.hand_registry.get_definition(&hand_id) {
        Some(d) => d.clone(),
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": format!("Hand not found: {hand_id}")})),
            );
        }
    };

    let reqs = state
        .kernel
        .hand_registry
        .check_requirements(&hand_id)
        .unwrap_or_default();

    let platform = server_platform();
    let mut results = Vec::new();

    for (req, already_satisfied) in &reqs {
        if *already_satisfied {
            results.push(serde_json::json!({
                "key": req.key,
                "status": "already_installed",
                "message": format!("{} is already available", req.label),
            }));
            continue;
        }

        let install = match &req.install {
            Some(i) => i,
            None => {
                results.push(serde_json::json!({
                    "key": req.key,
                    "status": "skipped",
                    "message": "No install instructions available",
                }));
                continue;
            }
        };

        // Pick the best install command for this platform
        let cmd = match platform {
            "windows" => install.windows.as_deref().or(install.pip.as_deref()),
            "macos" => install.macos.as_deref().or(install.pip.as_deref()),
            _ => install
                .linux_apt
                .as_deref()
                .or(install.linux_dnf.as_deref())
                .or(install.linux_pacman.as_deref())
                .or(install.pip.as_deref()),
        };

        let cmd = match cmd {
            Some(c) => c,
            None => {
                results.push(serde_json::json!({
                    "key": req.key,
                    "status": "no_command",
                    "message": format!("No install command for platform: {platform}"),
                }));
                continue;
            }
        };

        // Execute the install command
        let (shell, flag) = if cfg!(windows) {
            ("cmd", "/C")
        } else {
            ("sh", "-c")
        };

        // For winget on Windows, add --accept flags to avoid interactive prompts
        let final_cmd = if cfg!(windows) && cmd.starts_with("winget ") {
            format!("{cmd} --accept-source-agreements --accept-package-agreements")
        } else {
            cmd.to_string()
        };

        tracing::info!(hand = %hand_id, dep = %req.key, cmd = %final_cmd, "Auto-installing dependency");

        let output = match tokio::time::timeout(
            std::time::Duration::from_secs(300),
            tokio::process::Command::new(shell)
                .arg(flag)
                .arg(&final_cmd)
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .stdin(std::process::Stdio::null())
                .output(),
        )
        .await
        {
            Ok(Ok(out)) => out,
            Ok(Err(e)) => {
                results.push(serde_json::json!({
                    "key": req.key,
                    "status": "error",
                    "command": final_cmd,
                    "message": format!("Failed to execute: {e}"),
                }));
                continue;
            }
            Err(_) => {
                results.push(serde_json::json!({
                    "key": req.key,
                    "status": "timeout",
                    "command": final_cmd,
                    "message": "Installation timed out after 5 minutes",
                }));
                continue;
            }
        };

        let exit_code = output.status.code().unwrap_or(-1);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        if exit_code == 0 {
            results.push(serde_json::json!({
                "key": req.key,
                "status": "installed",
                "command": final_cmd,
                "message": format!("{} installed successfully", req.label),
            }));
        } else {
            // On Windows, winget may return non-zero even on success (e.g., already installed)
            let combined = format!("{stdout}{stderr}");
            let likely_ok = combined.contains("already installed")
                || combined.contains("No applicable update")
                || combined.contains("No available upgrade");
            results.push(serde_json::json!({
                "key": req.key,
                "status": if likely_ok { "installed" } else { "error" },
                "command": final_cmd,
                "exit_code": exit_code,
                "message": if likely_ok {
                    format!("{} is already installed", req.label)
                } else {
                    let msg = stderr.chars().take(500).collect::<String>();
                    format!("Install failed (exit {}): {}", exit_code, msg.trim())
                },
            }));
        }
    }

    // On Windows, refresh PATH to pick up newly installed binaries from winget/pip
    #[cfg(windows)]
    {
        let home = std::env::var("USERPROFILE").unwrap_or_default();
        if !home.is_empty() {
            let winget_pkgs =
                std::path::Path::new(&home).join("AppData\\Local\\Microsoft\\WinGet\\Packages");
            if winget_pkgs.is_dir() {
                let mut extra_paths = Vec::new();
                if let Ok(entries) = std::fs::read_dir(&winget_pkgs) {
                    for entry in entries.flatten() {
                        let pkg_dir = entry.path();
                        // Look for bin/ subdirectory (ffmpeg style)
                        if let Ok(sub_entries) = std::fs::read_dir(&pkg_dir) {
                            for sub in sub_entries.flatten() {
                                let bin_dir = sub.path().join("bin");
                                if bin_dir.is_dir() {
                                    extra_paths.push(bin_dir.to_string_lossy().to_string());
                                }
                            }
                        }
                        // Direct exe in package dir (yt-dlp style)
                        if std::fs::read_dir(&pkg_dir)
                            .map(|rd| {
                                rd.flatten().any(|e| {
                                    e.path().extension().map(|x| x == "exe").unwrap_or(false)
                                })
                            })
                            .unwrap_or(false)
                        {
                            extra_paths.push(pkg_dir.to_string_lossy().to_string());
                        }
                    }
                }
                // Also add pip Scripts dir
                let pip_scripts =
                    std::path::Path::new(&home).join("AppData\\Local\\Programs\\Python");
                if pip_scripts.is_dir() {
                    if let Ok(entries) = std::fs::read_dir(&pip_scripts) {
                        for entry in entries.flatten() {
                            let scripts = entry.path().join("Scripts");
                            if scripts.is_dir() {
                                extra_paths.push(scripts.to_string_lossy().to_string());
                            }
                        }
                    }
                }
                if !extra_paths.is_empty() {
                    let current_path = std::env::var("PATH").unwrap_or_default();
                    let new_path = format!("{};{}", extra_paths.join(";"), current_path);
                    std::env::set_var("PATH", &new_path);
                    tracing::info!(
                        added = extra_paths.len(),
                        "Refreshed PATH with winget/pip directories"
                    );
                }
            }
        }
    }

    // Re-check requirements after installation
    let reqs_after = state
        .kernel
        .hand_registry
        .check_requirements(&hand_id)
        .unwrap_or_default();
    let all_satisfied = reqs_after.iter().all(|(_, ok)| *ok);

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "hand_id": def.id,
            "results": results,
            "requirements_met": all_satisfied,
            "requirements": reqs_after.iter().map(|(r, ok)| {
                serde_json::json!({
                    "key": r.key,
                    "label": r.label,
                    "satisfied": ok,
                })
            }).collect::<Vec<_>>(),
        })),
    )
}

/// POST /api/hands/{hand_id}/activate — Activate a hand (spawns agent).
pub async fn activate_hand(
    State(state): State<Arc<AppState>>,
    Path(hand_id): Path<String>,
    body: Option<Json<skipper_hands::ActivateHandRequest>>,
) -> impl IntoResponse {
    let config = body.map(|b| b.0.config).unwrap_or_default();

    match state.kernel.activate_hand(&hand_id, config) {
        Ok(instance) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "instance_id": instance.instance_id,
                "hand_id": instance.hand_id,
                "status": format!("{}", instance.status),
                "agent_id": instance.agent_id.map(|a| a.to_string()),
                "agent_name": instance.agent_name,
                "activated_at": instance.activated_at.to_rfc3339(),
            })),
        ),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": format!("{e}")})),
        ),
    }
}

/// POST /api/hands/instances/{id}/pause — Pause a hand instance.
pub async fn pause_hand(
    State(state): State<Arc<AppState>>,
    Path(id): Path<uuid::Uuid>,
) -> impl IntoResponse {
    match state.kernel.pause_hand(id) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "paused", "instance_id": id})),
        ),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": format!("{e}")})),
        ),
    }
}

/// POST /api/hands/instances/{id}/resume — Resume a paused hand instance.
pub async fn resume_hand(
    State(state): State<Arc<AppState>>,
    Path(id): Path<uuid::Uuid>,
) -> impl IntoResponse {
    match state.kernel.resume_hand(id) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "resumed", "instance_id": id})),
        ),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": format!("{e}")})),
        ),
    }
}

/// DELETE /api/hands/instances/{id} — Deactivate a hand (kills agent).
pub async fn deactivate_hand(
    State(state): State<Arc<AppState>>,
    Path(id): Path<uuid::Uuid>,
) -> impl IntoResponse {
    match state.kernel.deactivate_hand(id) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "deactivated", "instance_id": id})),
        ),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": format!("{e}")})),
        ),
    }
}

/// GET /api/hands/instances/{id}/stats — Get dashboard stats for a hand instance.
pub async fn hand_stats(
    State(state): State<Arc<AppState>>,
    Path(id): Path<uuid::Uuid>,
) -> impl IntoResponse {
    let instance = match state.kernel.hand_registry.get_instance(id) {
        Some(i) => i,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "Instance not found"})),
            );
        }
    };

    let def = match state.kernel.hand_registry.get_definition(&instance.hand_id) {
        Some(d) => d,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "Hand definition not found"})),
            );
        }
    };

    let agent_id = match instance.agent_id {
        Some(aid) => aid,
        None => {
            return (
                StatusCode::OK,
                Json(serde_json::json!({
                    "instance_id": id,
                    "hand_id": instance.hand_id,
                    "metrics": {},
                })),
            );
        }
    };

    // Read dashboard metrics from agent's structured memory
    let mut metrics = serde_json::Map::new();
    for metric in &def.dashboard.metrics {
        let value = state
            .kernel
            .memory
            .structured_get(agent_id, &metric.memory_key)
            .ok()
            .flatten()
            .unwrap_or(serde_json::Value::Null);
        metrics.insert(
            metric.label.clone(),
            serde_json::json!({
                "value": value,
                "format": metric.format,
            }),
        );
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "instance_id": id,
            "hand_id": instance.hand_id,
            "status": format!("{}", instance.status),
            "agent_id": agent_id.to_string(),
            "metrics": metrics,
        })),
    )
}

/// GET /api/hands/instances/{id}/browser — Get live browser state for a hand instance.
pub async fn hand_instance_browser(
    State(state): State<Arc<AppState>>,
    Path(id): Path<uuid::Uuid>,
) -> impl IntoResponse {
    // 1. Look up instance
    let instance = match state.kernel.hand_registry.get_instance(id) {
        Some(i) => i,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "Instance not found"})),
            );
        }
    };

    // 2. Get agent_id
    let agent_id = match instance.agent_id {
        Some(aid) => aid,
        None => {
            return (StatusCode::OK, Json(serde_json::json!({"active": false})));
        }
    };

    let agent_id_str = agent_id.to_string();

    // 3. Check if a browser session exists (without creating one)
    if !state.kernel.browser_ctx.has_session(&agent_id_str) {
        return (StatusCode::OK, Json(serde_json::json!({"active": false})));
    }

    // 4. Send ReadPage command to get page info
    let mut url = String::new();
    let mut title = String::new();
    let mut content = String::new();

    match state
        .kernel
        .browser_ctx
        .send_command(
            &agent_id_str,
            skipper_runtime::browser::BrowserCommand::ReadPage,
        )
        .await
    {
        Ok(resp) if resp.success => {
            if let Some(data) = &resp.data {
                url = data["url"].as_str().unwrap_or("").to_string();
                title = data["title"].as_str().unwrap_or("").to_string();
                content = data["content"].as_str().unwrap_or("").to_string();
                // Truncate content to avoid huge payloads (UTF-8 safe)
                if content.len() > 2000 {
                    content = format!("{}... (truncated)", skipper_types::truncate_str(&content, 2000));
                }
            }
        }
        Ok(_) => {}  // Non-success: leave defaults
        Err(_) => {} // Error: leave defaults
    }

    // 5. Send Screenshot command to get visual state
    let mut screenshot_base64 = String::new();

    match state
        .kernel
        .browser_ctx
        .send_command(
            &agent_id_str,
            skipper_runtime::browser::BrowserCommand::Screenshot,
        )
        .await
    {
        Ok(resp) if resp.success => {
            if let Some(data) = &resp.data {
                screenshot_base64 = data["image_base64"].as_str().unwrap_or("").to_string();
            }
        }
        Ok(_) => {}
        Err(_) => {}
    }

    // 6. Return combined state
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "active": true,
            "url": url,
            "title": title,
            "content": content,
            "screenshot_base64": screenshot_base64,
        })),
    )
}

// ---------------------------------------------------------------------------
// MCP server endpoints
// ---------------------------------------------------------------------------

/// GET /api/mcp/servers — List configured MCP servers and their tools.
pub async fn list_mcp_servers(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    // Get configured servers from config
    let config_servers: Vec<serde_json::Value> = state
        .kernel
        .config
        .mcp_servers
        .iter()
        .map(|s| {
            let transport = match &s.transport {
                skipper_types::config::McpTransportEntry::Stdio { command, args } => {
                    serde_json::json!({
                        "type": "stdio",
                        "command": command,
                        "args": args,
                    })
                }
                skipper_types::config::McpTransportEntry::Sse { url } => {
                    serde_json::json!({
                        "type": "sse",
                        "url": url,
                    })
                }
            };
            serde_json::json!({
                "name": s.name,
                "transport": transport,
                "timeout_secs": s.timeout_secs,
                "env": s.env,
            })
        })
        .collect();

    // Get connected servers and their tools from the live MCP connections
    let connections = state.kernel.mcp_connections.lock().await;
    let connected: Vec<serde_json::Value> = connections
        .iter()
        .map(|conn| {
            let tools: Vec<serde_json::Value> = conn
                .tools()
                .iter()
                .map(|t| {
                    serde_json::json!({
                        "name": t.name,
                        "description": t.description,
                    })
                })
                .collect();
            serde_json::json!({
                "name": conn.name(),
                "tools_count": tools.len(),
                "tools": tools,
                "connected": true,
            })
        })
        .collect();

    Json(serde_json::json!({
        "configured": config_servers,
        "connected": connected,
        "total_configured": config_servers.len(),
        "total_connected": connected.len(),
    }))
}

// ---------------------------------------------------------------------------
// Audit endpoints
// ---------------------------------------------------------------------------

/// GET /api/audit/recent — Get recent audit log entries.
pub async fn audit_recent(
    State(state): State<Arc<AppState>>,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let n: usize = params
        .get("n")
        .and_then(|v| v.parse().ok())
        .unwrap_or(50)
        .min(1000); // Cap at 1000

    let entries = state.kernel.audit_log.recent(n);
    let tip = state.kernel.audit_log.tip_hash();

    let items: Vec<serde_json::Value> = entries
        .iter()
        .map(|e| {
            serde_json::json!({
                "seq": e.seq,
                "timestamp": e.timestamp,
                "agent_id": e.agent_id,
                "action": format!("{:?}", e.action),
                "detail": e.detail,
                "outcome": e.outcome,
                "hash": e.hash,
            })
        })
        .collect();

    Json(serde_json::json!({
        "entries": items,
        "total": state.kernel.audit_log.len(),
        "tip_hash": tip,
    }))
}

/// GET /api/audit/verify — Verify the audit chain integrity.
pub async fn audit_verify(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let entry_count = state.kernel.audit_log.len();
    match state.kernel.audit_log.verify_integrity() {
        Ok(()) => {
            if entry_count == 0 {
                // SECURITY: Warn that an empty audit log has no forensic value
                Json(serde_json::json!({
                    "valid": true,
                    "entries": 0,
                    "warning": "Audit log is empty — no events have been recorded yet",
                    "tip_hash": state.kernel.audit_log.tip_hash(),
                }))
            } else {
                Json(serde_json::json!({
                    "valid": true,
                    "entries": entry_count,
                    "tip_hash": state.kernel.audit_log.tip_hash(),
                }))
            }
        }
        Err(msg) => Json(serde_json::json!({
            "valid": false,
            "error": msg,
            "entries": entry_count,
        })),
    }
}

/// GET /api/logs/stream — SSE endpoint for real-time audit log streaming.
///
/// Streams new audit entries as Server-Sent Events. Accepts optional query
/// parameters for filtering:
///   - `level`  — filter by classified level (info, warn, error)
///   - `filter` — text substring filter across action/detail/agent_id
///   - `token`  — auth token (for EventSource clients that cannot set headers)
///
/// A heartbeat ping is sent every 15 seconds to keep the connection alive.
/// The endpoint polls the audit log every second and sends only new entries
/// (tracked by sequence number). On first connect, existing entries are sent
/// as a backfill so the client has immediate context.
pub async fn logs_stream(
    State(state): State<Arc<AppState>>,
    Query(params): Query<HashMap<String, String>>,
) -> axum::response::Response {
    use axum::response::sse::{Event, KeepAlive, Sse};

    let level_filter = params.get("level").cloned().unwrap_or_default();
    let text_filter = params
        .get("filter")
        .cloned()
        .unwrap_or_default()
        .to_lowercase();

    let (tx, rx) = tokio::sync::mpsc::channel::<
        Result<axum::response::sse::Event, std::convert::Infallible>,
    >(256);

    tokio::spawn(async move {
        let mut last_seq: u64 = 0;
        let mut first_poll = true;

        loop {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;

            let entries = state.kernel.audit_log.recent(200);

            for entry in &entries {
                // On first poll, send all existing entries as backfill.
                // After that, only send entries newer than last_seq.
                if !first_poll && entry.seq <= last_seq {
                    continue;
                }

                let action_str = format!("{:?}", entry.action);

                // Apply level filter
                if !level_filter.is_empty() {
                    let classified = classify_audit_level(&action_str);
                    if classified != level_filter {
                        continue;
                    }
                }

                // Apply text filter
                if !text_filter.is_empty() {
                    let haystack = format!("{} {} {}", action_str, entry.detail, entry.agent_id)
                        .to_lowercase();
                    if !haystack.contains(&text_filter) {
                        continue;
                    }
                }

                let json = serde_json::json!({
                    "seq": entry.seq,
                    "timestamp": entry.timestamp,
                    "agent_id": entry.agent_id,
                    "action": action_str,
                    "detail": entry.detail,
                    "outcome": entry.outcome,
                    "hash": entry.hash,
                });
                let data = serde_json::to_string(&json).unwrap_or_default();
                if tx.send(Ok(Event::default().data(data))).await.is_err() {
                    return; // Client disconnected
                }
            }

            // Update tracking state
            if let Some(last) = entries.last() {
                last_seq = last.seq;
            }
            first_poll = false;
        }
    });

    let rx_stream = tokio_stream::wrappers::ReceiverStream::new(rx);
    Sse::new(rx_stream)
        .keep_alive(
            KeepAlive::new()
                .interval(std::time::Duration::from_secs(15))
                .text("ping"),
        )
        .into_response()
}

/// Classify an audit action string into a level (info, warn, error).
fn classify_audit_level(action: &str) -> &'static str {
    let a = action.to_lowercase();
    if a.contains("error") || a.contains("fail") || a.contains("crash") || a.contains("denied") {
        "error"
    } else if a.contains("warn") || a.contains("block") || a.contains("kill") {
        "warn"
    } else {
        "info"
    }
}

// ---------------------------------------------------------------------------
// Peer endpoints
// ---------------------------------------------------------------------------

/// GET /api/peers — List known OFP peers.
pub async fn list_peers(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    // Peers are tracked in the wire module's PeerRegistry.
    // The kernel doesn't directly hold a PeerRegistry, so we return an empty list
    // unless one is available. The API server can be extended to inject a registry.
    if let Some(ref peer_registry) = state.peer_registry {
        let peers: Vec<serde_json::Value> = peer_registry
            .all_peers()
            .iter()
            .map(|p| {
                serde_json::json!({
                    "node_id": p.node_id,
                    "node_name": p.node_name,
                    "address": p.address.to_string(),
                    "state": format!("{:?}", p.state),
                    "agents": p.agents.iter().map(|a| serde_json::json!({
                        "id": a.id,
                        "name": a.name,
                    })).collect::<Vec<_>>(),
                    "connected_at": p.connected_at.to_rfc3339(),
                    "protocol_version": p.protocol_version,
                })
            })
            .collect();
        Json(serde_json::json!({"peers": peers, "total": peers.len()}))
    } else {
        Json(serde_json::json!({"peers": [], "total": 0}))
    }
}

/// GET /api/network/status — OFP network status summary.
pub async fn network_status(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let enabled = state.kernel.config.network_enabled
        && !state.kernel.config.network.shared_secret.is_empty();

    let (node_id, listen_address, connected_peers, total_peers) =
        if let Some(ref peer_node) = state.kernel.peer_node {
            let registry = peer_node.registry();
            (
                peer_node.node_id().to_string(),
                peer_node.local_addr().to_string(),
                registry.connected_count(),
                registry.total_count(),
            )
        } else {
            (String::new(), String::new(), 0, 0)
        };

    Json(serde_json::json!({
        "enabled": enabled,
        "node_id": node_id,
        "listen_address": listen_address,
        "connected_peers": connected_peers,
        "total_peers": total_peers,
    }))
}

// ---------------------------------------------------------------------------
// Tools endpoint
// ---------------------------------------------------------------------------

/// GET /api/tools — List all tool definitions (built-in + MCP).
pub async fn list_tools(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let mut tools: Vec<serde_json::Value> = builtin_tool_definitions()
        .iter()
        .map(|t| {
            serde_json::json!({
                "name": t.name,
                "description": t.description,
                "input_schema": t.input_schema,
            })
        })
        .collect();

    // Include MCP tools so they're visible in Settings -> Tools
    if let Ok(mcp_tools) = state.kernel.mcp_tools.lock() {
        for t in mcp_tools.iter() {
            tools.push(serde_json::json!({
                "name": t.name,
                "description": t.description,
                "input_schema": t.input_schema,
                "source": "mcp",
            }));
        }
    }

    Json(serde_json::json!({"tools": tools, "total": tools.len()}))
}

// ---------------------------------------------------------------------------
// Config endpoint
// ---------------------------------------------------------------------------

/// GET /api/config — Get kernel configuration (secrets redacted).
pub async fn get_config(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    // Return a redacted view of the kernel config
    let config = &state.kernel.config;
    Json(serde_json::json!({
        "home_dir": config.home_dir.to_string_lossy(),
        "data_dir": config.data_dir.to_string_lossy(),
        "api_key": if config.api_key.is_empty() { "not set" } else { "***" },
        "default_model": {
            "provider": config.default_model.provider,
            "model": config.default_model.model,
            "api_key_env": config.default_model.api_key_env,
        },
        "memory": {
            "decay_rate": config.memory.decay_rate,
        },
    }))
}


// ---------------------------------------------------------------------------
// Session listing endpoints
// ---------------------------------------------------------------------------

/// GET /api/sessions — List all sessions with metadata.
pub async fn list_sessions(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    match state.kernel.memory.list_sessions() {
        Ok(sessions) => Json(serde_json::json!({"sessions": sessions})),
        Err(_) => Json(serde_json::json!({"sessions": []})),
    }
}

/// DELETE /api/sessions/:id — Delete a session.
pub async fn delete_session(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let session_id = match id.parse::<uuid::Uuid>() {
        Ok(u) => skipper_types::agent::SessionId(u),
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Invalid session ID"})),
            );
        }
    };

    match state.kernel.memory.delete_session(session_id) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "deleted", "session_id": id})),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        ),
    }
}

/// PUT /api/sessions/:id/label — Set a session label.
pub async fn set_session_label(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<serde_json::Value>,
) -> impl IntoResponse {
    let session_id = match id.parse::<uuid::Uuid>() {
        Ok(u) => skipper_types::agent::SessionId(u),
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Invalid session ID"})),
            );
        }
    };

    let label = req.get("label").and_then(|v| v.as_str());

    // Validate label if present
    if let Some(lbl) = label {
        if let Err(e) = skipper_types::agent::SessionLabel::new(lbl) {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": e.to_string()})),
            );
        }
    }

    match state.kernel.memory.set_session_label(session_id, label) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "updated",
                "session_id": id,
                "label": label,
            })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        ),
    }
}

/// GET /api/sessions/by-label/:label — Find session by label (scoped to agent).
pub async fn find_session_by_label(
    State(state): State<Arc<AppState>>,
    Path((agent_id_str, label)): Path<(String, String)>,
) -> impl IntoResponse {
    let agent_id = match agent_id_str.parse::<uuid::Uuid>() {
        Ok(u) => skipper_types::agent::AgentId(u),
        Err(_) => {
            // Try name lookup
            match state.kernel.registry.find_by_name(&agent_id_str) {
                Some(entry) => entry.id,
                None => {
                    return (
                        StatusCode::NOT_FOUND,
                        Json(serde_json::json!({"error": "Agent not found"})),
                    );
                }
            }
        }
    };

    match state.kernel.memory.find_session_by_label(agent_id, &label) {
        Ok(Some(session)) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "session_id": session.id.0.to_string(),
                "agent_id": session.agent_id.0.to_string(),
                "label": session.label,
                "message_count": session.messages.len(),
            })),
        ),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "No session found with that label"})),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        ),
    }
}

// ---------------------------------------------------------------------------
// Agent update endpoint
// ---------------------------------------------------------------------------

/// PUT /api/agents/:id — Update an agent (currently: re-set manifest fields).
pub async fn update_agent(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<AgentUpdateRequest>,
) -> impl IntoResponse {
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Invalid agent ID"})),
            );
        }
    };

    if state.kernel.registry.get(agent_id).is_none() {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "Agent not found"})),
        );
    }

    // Parse the new manifest
    let _manifest: AgentManifest = match toml::from_str(&req.manifest_toml) {
        Ok(m) => m,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": format!("Invalid manifest: {e}")})),
            );
        }
    };

    // Note: Full manifest update requires kill + respawn. For now, acknowledge receipt.
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "acknowledged",
            "agent_id": id,
            "note": "Full manifest update requires agent restart. Use DELETE + POST to apply.",
        })),
    )
}

// ---------------------------------------------------------------------------
// Migration endpoint
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Security dashboard endpoint
// ---------------------------------------------------------------------------

/// GET /api/security — Security feature status for the dashboard.
pub async fn security_status(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let auth_mode = if state.kernel.config.api_key.is_empty() {
        "localhost_only"
    } else {
        "bearer_token"
    };

    let audit_count = state.kernel.audit_log.len();

    Json(serde_json::json!({
        "core_protections": {
            "path_traversal": true,
            "ssrf_protection": true,
            "capability_system": true,
            "privilege_escalation_prevention": true,
            "subprocess_isolation": true,
            "security_headers": true,
            "wire_hmac_auth": true,
            "request_id_tracking": true
        },
        "configurable": {
            "rate_limiter": {
                "enabled": true,
                "tokens_per_minute": 500,
                "algorithm": "GCRA"
            },
            "websocket_limits": {
                "max_per_ip": 5,
                "idle_timeout_secs": 1800,
                "max_message_size": 65536,
                "max_messages_per_minute": 10
            },
            "wasm_sandbox": {
                "fuel_metering": true,
                "epoch_interruption": true,
                "default_timeout_secs": 30,
                "default_fuel_limit": 1_000_000u64
            },
            "auth": {
                "mode": auth_mode,
                "api_key_set": !state.kernel.config.api_key.is_empty()
            }
        },
        "monitoring": {
            "audit_trail": {
                "enabled": true,
                "algorithm": "SHA-256 Merkle Chain",
                "entry_count": audit_count
            },
            "taint_tracking": {
                "enabled": true,
                "tracked_labels": [
                    "ExternalNetwork",
                    "UserInput",
                    "PII",
                    "Secret",
                    "UntrustedAgent"
                ]
            },
            "manifest_signing": {
                "algorithm": "Ed25519",
                "available": true
            }
        },
        "secret_zeroization": true,
        "total_features": 15
    }))
}

/// GET /api/migrate/detect — Auto-detect OpenClaw installation.
pub async fn migrate_detect() -> impl IntoResponse {
    match skipper_migrate::openclaw::detect_openclaw_home() {
        Some(path) => {
            let scan = skipper_migrate::openclaw::scan_openclaw_workspace(&path);
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "detected": true,
                    "path": path.display().to_string(),
                    "scan": scan,
                })),
            )
        }
        None => (
            StatusCode::OK,
            Json(serde_json::json!({
                "detected": false,
                "path": null,
                "scan": null,
            })),
        ),
    }
}

/// POST /api/migrate/scan — Scan a specific directory for OpenClaw workspace.
pub async fn migrate_scan(Json(req): Json<MigrateScanRequest>) -> impl IntoResponse {
    let path = std::path::PathBuf::from(&req.path);
    if !path.exists() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Directory not found"})),
        );
    }
    let scan = skipper_migrate::openclaw::scan_openclaw_workspace(&path);
    (StatusCode::OK, Json(serde_json::json!(scan)))
}

/// POST /api/migrate — Run migration from another agent framework.
pub async fn run_migrate(Json(req): Json<MigrateRequest>) -> impl IntoResponse {
    let source = match req.source.as_str() {
        "openclaw" => skipper_migrate::MigrateSource::OpenClaw,
        "langchain" => skipper_migrate::MigrateSource::LangChain,
        "autogpt" => skipper_migrate::MigrateSource::AutoGpt,
        other => {
            return (
                StatusCode::BAD_REQUEST,
                Json(
                    serde_json::json!({"error": format!("Unknown source: {other}. Use 'openclaw', 'langchain', or 'autogpt'")}),
                ),
            );
        }
    };

    let options = skipper_migrate::MigrateOptions {
        source,
        source_dir: std::path::PathBuf::from(&req.source_dir),
        target_dir: std::path::PathBuf::from(&req.target_dir),
        dry_run: req.dry_run,
    };

    match skipper_migrate::run_migration(&options) {
        Ok(report) => {
            let imported: Vec<serde_json::Value> = report
                .imported
                .iter()
                .map(|i| {
                    serde_json::json!({
                        "kind": format!("{}", i.kind),
                        "name": i.name,
                        "destination": i.destination,
                    })
                })
                .collect();

            let skipped: Vec<serde_json::Value> = report
                .skipped
                .iter()
                .map(|s| {
                    serde_json::json!({
                        "kind": format!("{}", s.kind),
                        "name": s.name,
                        "reason": s.reason,
                    })
                })
                .collect();

            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "status": "completed",
                    "dry_run": req.dry_run,
                    "imported": imported,
                    "imported_count": imported.len(),
                    "skipped": skipped,
                    "skipped_count": skipped.len(),
                    "warnings": report.warnings,
                    "report_markdown": report.to_markdown(),
                })),
            )
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Migration failed: {e}")})),
        ),
    }
}

// ── Model Catalog Endpoints ─────────────────────────────────────────

/// GET /api/models — List all models in the catalog.
///
/// Query parameters:
/// - `provider` — filter by provider (e.g. `?provider=anthropic`)
/// - `tier` — filter by tier (e.g. `?tier=smart`)
/// - `available` — only show models from configured providers (`?available=true`)
pub async fn list_models(
    State(state): State<Arc<AppState>>,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let catalog = state
        .kernel
        .model_catalog
        .read()
        .unwrap_or_else(|e| e.into_inner());
    let provider_filter = params.get("provider").map(|s| s.to_lowercase());
    let tier_filter = params.get("tier").map(|s| s.to_lowercase());
    let available_only = params
        .get("available")
        .map(|v| v == "true" || v == "1")
        .unwrap_or(false);

    let models: Vec<serde_json::Value> = catalog
        .list_models()
        .iter()
        .filter(|m| {
            if let Some(ref p) = provider_filter {
                if m.provider.to_lowercase() != *p {
                    return false;
                }
            }
            if let Some(ref t) = tier_filter {
                if m.tier.to_string() != *t {
                    return false;
                }
            }
            if available_only {
                let provider = catalog.get_provider(&m.provider);
                if let Some(p) = provider {
                    if p.auth_status == skipper_types::model_catalog::AuthStatus::Missing {
                        return false;
                    }
                }
            }
            true
        })
        .map(|m| {
            let available = catalog
                .get_provider(&m.provider)
                .map(|p| p.auth_status != skipper_types::model_catalog::AuthStatus::Missing)
                .unwrap_or(false);
            serde_json::json!({
                "id": m.id,
                "display_name": m.display_name,
                "provider": m.provider,
                "tier": m.tier,
                "context_window": m.context_window,
                "max_output_tokens": m.max_output_tokens,
                "input_cost_per_m": m.input_cost_per_m,
                "output_cost_per_m": m.output_cost_per_m,
                "supports_tools": m.supports_tools,
                "supports_vision": m.supports_vision,
                "supports_streaming": m.supports_streaming,
                "available": available,
            })
        })
        .collect();

    let total = catalog.list_models().len();
    let available_count = catalog.available_models().len();

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "models": models,
            "total": total,
            "available": available_count,
        })),
    )
}

/// GET /api/models/aliases — List all alias-to-model mappings.
pub async fn list_aliases(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let aliases = state
        .kernel
        .model_catalog
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .list_aliases()
        .clone();
    let entries: Vec<serde_json::Value> = aliases
        .iter()
        .map(|(alias, model_id)| {
            serde_json::json!({
                "alias": alias,
                "model_id": model_id,
            })
        })
        .collect();

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "aliases": entries,
            "total": entries.len(),
        })),
    )
}

/// GET /api/models/{id} — Get a single model by ID or alias.
pub async fn get_model(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let catalog = state
        .kernel
        .model_catalog
        .read()
        .unwrap_or_else(|e| e.into_inner());
    match catalog.find_model(&id) {
        Some(m) => {
            let available = catalog
                .get_provider(&m.provider)
                .map(|p| p.auth_status != skipper_types::model_catalog::AuthStatus::Missing)
                .unwrap_or(false);
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "id": m.id,
                    "display_name": m.display_name,
                    "provider": m.provider,
                    "tier": m.tier,
                    "context_window": m.context_window,
                    "max_output_tokens": m.max_output_tokens,
                    "input_cost_per_m": m.input_cost_per_m,
                    "output_cost_per_m": m.output_cost_per_m,
                    "supports_tools": m.supports_tools,
                    "supports_vision": m.supports_vision,
                    "supports_streaming": m.supports_streaming,
                    "aliases": m.aliases,
                    "available": available,
                })),
            )
        }
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": format!("Model '{}' not found", id)})),
        ),
    }
}

/// GET /api/providers — List all providers with auth status.
///
/// For local providers (ollama, vllm, lmstudio), also probes reachability and
/// discovers available models via their health endpoints.
pub async fn list_providers(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let provider_list: Vec<skipper_types::model_catalog::ProviderInfo> = {
        let catalog = state
            .kernel
            .model_catalog
            .read()
            .unwrap_or_else(|e| e.into_inner());
        catalog.list_providers().to_vec()
    };

    let mut providers: Vec<serde_json::Value> = Vec::with_capacity(provider_list.len());

    for p in &provider_list {
        let mut entry = serde_json::json!({
            "id": p.id,
            "display_name": p.display_name,
            "auth_status": p.auth_status,
            "model_count": p.model_count,
            "key_required": p.key_required,
            "api_key_env": p.api_key_env,
            "base_url": p.base_url,
        });

        // For local providers, add reachability info via health probe
        if !p.key_required {
            entry["is_local"] = serde_json::json!(true);
            let probe = skipper_runtime::provider_health::probe_provider(&p.id, &p.base_url).await;
            entry["reachable"] = serde_json::json!(probe.reachable);
            entry["latency_ms"] = serde_json::json!(probe.latency_ms);
            if !probe.discovered_models.is_empty() {
                entry["discovered_models"] = serde_json::json!(probe.discovered_models);
            }
            if let Some(err) = &probe.error {
                entry["error"] = serde_json::json!(err);
            }
        }

        providers.push(entry);
    }

    let total = providers.len();
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "providers": providers,
            "total": total,
        })),
    )
}

/// POST /api/models/custom — Add a custom model to the catalog.
///
/// Persists to `~/.skipper/custom_models.json` and makes the model immediately
/// available for agent assignment.
pub async fn add_custom_model(
    State(state): State<Arc<AppState>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let id = body
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let provider = body
        .get("provider")
        .and_then(|v| v.as_str())
        .unwrap_or("openrouter")
        .to_string();
    let context_window = body
        .get("context_window")
        .and_then(|v| v.as_u64())
        .unwrap_or(128_000);
    let max_output = body
        .get("max_output_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(8_192);

    if id.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Missing required field: id"})),
        );
    }

    let display = body
        .get("display_name")
        .and_then(|v| v.as_str())
        .unwrap_or(&id)
        .to_string();

    let entry = skipper_types::model_catalog::ModelCatalogEntry {
        id: id.clone(),
        display_name: display,
        provider: provider.clone(),
        tier: skipper_types::model_catalog::ModelTier::Custom,
        context_window,
        max_output_tokens: max_output,
        input_cost_per_m: body
            .get("input_cost_per_m")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0),
        output_cost_per_m: body
            .get("output_cost_per_m")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0),
        supports_tools: body
            .get("supports_tools")
            .and_then(|v| v.as_bool())
            .unwrap_or(true),
        supports_vision: body
            .get("supports_vision")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        supports_streaming: body
            .get("supports_streaming")
            .and_then(|v| v.as_bool())
            .unwrap_or(true),
        aliases: vec![],
    };

    let mut catalog = state
        .kernel
        .model_catalog
        .write()
        .unwrap_or_else(|e| e.into_inner());

    if !catalog.add_custom_model(entry) {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({"error": format!("Model '{}' already exists", id)})),
        );
    }

    // Persist to disk
    let custom_path = state.kernel.config.home_dir.join("custom_models.json");
    if let Err(e) = catalog.save_custom_models(&custom_path) {
        tracing::warn!("Failed to persist custom models: {e}");
    }

    (
        StatusCode::CREATED,
        Json(serde_json::json!({
            "id": id,
            "provider": provider,
            "status": "added"
        })),
    )
}

/// DELETE /api/models/custom/{id} — Remove a custom model.
pub async fn remove_custom_model(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(model_id): axum::extract::Path<String>,
) -> impl IntoResponse {
    let mut catalog = state
        .kernel
        .model_catalog
        .write()
        .unwrap_or_else(|e| e.into_inner());

    if !catalog.remove_custom_model(&model_id) {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": format!("Custom model '{}' not found", model_id)})),
        );
    }

    let custom_path = state.kernel.config.home_dir.join("custom_models.json");
    if let Err(e) = catalog.save_custom_models(&custom_path) {
        tracing::warn!("Failed to persist custom models: {e}");
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({"status": "removed"})),
    )
}

// ── A2A (Agent-to-Agent) Protocol Endpoints ─────────────────────────

/// GET /.well-known/agent.json — A2A Agent Card for the default agent.
pub async fn a2a_agent_card(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let agents = state.kernel.registry.list();
    let base_url = format!("http://{}", state.kernel.config.api_listen);

    if let Some(first) = agents.first() {
        let card = skipper_runtime::a2a::build_agent_card(&first.manifest, &base_url);
        (
            StatusCode::OK,
            Json(serde_json::to_value(&card).unwrap_or_default()),
        )
    } else {
        let card = serde_json::json!({
            "name": "skipper",
            "description": "Skipper Agent OS — no agents spawned yet",
            "url": format!("{base_url}/a2a"),
            "version": "0.1.0",
            "capabilities": { "streaming": true },
            "skills": [],
            "defaultInputModes": ["text"],
            "defaultOutputModes": ["text"],
        });
        (StatusCode::OK, Json(card))
    }
}

/// GET /a2a/agents — List all A2A agent cards.
pub async fn a2a_list_agents(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let agents = state.kernel.registry.list();
    let base_url = format!("http://{}", state.kernel.config.api_listen);

    let cards: Vec<serde_json::Value> = agents
        .iter()
        .map(|entry| {
            let card = skipper_runtime::a2a::build_agent_card(&entry.manifest, &base_url);
            serde_json::to_value(&card).unwrap_or_default()
        })
        .collect();

    let total = cards.len();
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "agents": cards,
            "total": total,
        })),
    )
}

/// POST /a2a/tasks/send — Submit a task to an agent via A2A.
pub async fn a2a_send_task(
    State(state): State<Arc<AppState>>,
    Json(request): Json<serde_json::Value>,
) -> impl IntoResponse {
    // Extract message text from A2A format
    let message_text = request["params"]["message"]["parts"]
        .as_array()
        .and_then(|parts| {
            parts.iter().find_map(|p| {
                if p["type"].as_str() == Some("text") {
                    p["text"].as_str().map(String::from)
                } else {
                    None
                }
            })
        })
        .unwrap_or_else(|| "No message provided".to_string());

    // Find target agent (use first available or specified)
    let agents = state.kernel.registry.list();
    if agents.is_empty() {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "No agents available"})),
        );
    }

    let agent = &agents[0];
    let task_id = uuid::Uuid::new_v4().to_string();
    let session_id = request["params"]["sessionId"].as_str().map(String::from);

    // Create the task in the store as Working
    let task = skipper_runtime::a2a::A2aTask {
        id: task_id.clone(),
        session_id: session_id.clone(),
        status: skipper_runtime::a2a::A2aTaskStatus::Working,
        messages: vec![skipper_runtime::a2a::A2aMessage {
            role: "user".to_string(),
            parts: vec![skipper_runtime::a2a::A2aPart::Text {
                text: message_text.clone(),
            }],
        }],
        artifacts: vec![],
    };
    state.kernel.a2a_task_store.insert(task);

    // Send message to agent
    match state.kernel.send_message(agent.id, &message_text).await {
        Ok(result) => {
            let response_msg = skipper_runtime::a2a::A2aMessage {
                role: "agent".to_string(),
                parts: vec![skipper_runtime::a2a::A2aPart::Text {
                    text: result.response,
                }],
            };
            state
                .kernel
                .a2a_task_store
                .complete(&task_id, response_msg, vec![]);
            match state.kernel.a2a_task_store.get(&task_id) {
                Some(completed_task) => (
                    StatusCode::OK,
                    Json(serde_json::to_value(&completed_task).unwrap_or_default()),
                ),
                None => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({"error": "Task disappeared after completion"})),
                ),
            }
        }
        Err(e) => {
            let error_msg = skipper_runtime::a2a::A2aMessage {
                role: "agent".to_string(),
                parts: vec![skipper_runtime::a2a::A2aPart::Text {
                    text: format!("Error: {e}"),
                }],
            };
            state.kernel.a2a_task_store.fail(&task_id, error_msg);
            match state.kernel.a2a_task_store.get(&task_id) {
                Some(failed_task) => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::to_value(&failed_task).unwrap_or_default()),
                ),
                None => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({"error": format!("Agent error: {e}")})),
                ),
            }
        }
    }
}

/// GET /a2a/tasks/{id} — Get task status from the task store.
pub async fn a2a_get_task(
    State(state): State<Arc<AppState>>,
    Path(task_id): Path<String>,
) -> impl IntoResponse {
    match state.kernel.a2a_task_store.get(&task_id) {
        Some(task) => (
            StatusCode::OK,
            Json(serde_json::to_value(&task).unwrap_or_default()),
        ),
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": format!("Task '{}' not found", task_id)})),
        ),
    }
}

/// POST /a2a/tasks/{id}/cancel — Cancel a tracked task.
pub async fn a2a_cancel_task(
    State(state): State<Arc<AppState>>,
    Path(task_id): Path<String>,
) -> impl IntoResponse {
    if state.kernel.a2a_task_store.cancel(&task_id) {
        match state.kernel.a2a_task_store.get(&task_id) {
            Some(task) => (
                StatusCode::OK,
                Json(serde_json::to_value(&task).unwrap_or_default()),
            ),
            None => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "Task disappeared after cancellation"})),
            ),
        }
    } else {
        (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": format!("Task '{}' not found", task_id)})),
        )
    }
}

// ── A2A Management Endpoints (outbound) ─────────────────────────────────

/// GET /api/a2a/agents — List discovered external A2A agents.
pub async fn a2a_list_external_agents(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let agents = state
        .kernel
        .a2a_external_agents
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let items: Vec<serde_json::Value> = agents
        .iter()
        .map(|(url, card)| {
            serde_json::json!({
                "name": card.name,
                "url": url,
                "description": card.description,
                "skills": card.skills,
                "version": card.version,
            })
        })
        .collect();
    Json(serde_json::json!({"agents": items, "total": items.len()}))
}

/// POST /api/a2a/discover — Discover a new external A2A agent by URL.
pub async fn a2a_discover_external(
    State(state): State<Arc<AppState>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let url = match body["url"].as_str() {
        Some(u) => u.to_string(),
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Missing 'url' field"})),
            )
        }
    };

    let client = skipper_runtime::a2a::A2aClient::new();
    match client.discover(&url).await {
        Ok(card) => {
            let card_json = serde_json::to_value(&card).unwrap_or_default();
            // Store in kernel's external agents list
            {
                let mut agents = state
                    .kernel
                    .a2a_external_agents
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                // Update or add
                if let Some(existing) = agents.iter_mut().find(|(u, _)| u == &url) {
                    existing.1 = card;
                } else {
                    agents.push((url.clone(), card));
                }
            }
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "url": url,
                    "agent": card_json,
                })),
            )
        }
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({"error": e})),
        ),
    }
}

/// POST /api/a2a/send — Send a task to an external A2A agent.
pub async fn a2a_send_external(
    State(_state): State<Arc<AppState>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let url = match body["url"].as_str() {
        Some(u) => u.to_string(),
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Missing 'url' field"})),
            )
        }
    };
    let message = match body["message"].as_str() {
        Some(m) => m.to_string(),
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Missing 'message' field"})),
            )
        }
    };
    let session_id = body["session_id"].as_str();

    let client = skipper_runtime::a2a::A2aClient::new();
    match client.send_task(&url, &message, session_id).await {
        Ok(task) => (
            StatusCode::OK,
            Json(serde_json::to_value(&task).unwrap_or_default()),
        ),
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({"error": e})),
        ),
    }
}

/// GET /api/a2a/tasks/{id}/status — Get task status from an external A2A agent.
pub async fn a2a_external_task_status(
    State(_state): State<Arc<AppState>>,
    Path(task_id): Path<String>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> impl IntoResponse {
    let url = match params.get("url") {
        Some(u) => u.clone(),
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Missing 'url' query parameter"})),
            )
        }
    };

    let client = skipper_runtime::a2a::A2aClient::new();
    match client.get_task(&url, &task_id).await {
        Ok(task) => (
            StatusCode::OK,
            Json(serde_json::to_value(&task).unwrap_or_default()),
        ),
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({"error": e})),
        ),
    }
}

// ── MCP HTTP Endpoint ───────────────────────────────────────────────────

/// POST /mcp — Handle MCP JSON-RPC requests over HTTP.
///
/// Exposes the same MCP protocol normally served via stdio, allowing
/// external MCP clients to connect over HTTP instead.
pub async fn mcp_http(
    State(state): State<Arc<AppState>>,
    Json(request): Json<serde_json::Value>,
) -> impl IntoResponse {
    // Gather all available tools (builtin + skills + MCP)
    let mut tools = builtin_tool_definitions();
    {
        let registry = state
            .kernel
            .skill_registry
            .read()
            .unwrap_or_else(|e| e.into_inner());
        for skill_tool in registry.all_tool_definitions() {
            tools.push(skipper_types::tool::ToolDefinition {
                name: skill_tool.name.clone(),
                description: skill_tool.description.clone(),
                input_schema: skill_tool.input_schema.clone(),
            });
        }
    }
    if let Ok(mcp_tools) = state.kernel.mcp_tools.lock() {
        tools.extend(mcp_tools.iter().cloned());
    }

    // Check if this is a tools/call that needs real execution
    let method = request["method"].as_str().unwrap_or("");
    if method == "tools/call" {
        let tool_name = request["params"]["name"].as_str().unwrap_or("");
        let arguments = request["params"]
            .get("arguments")
            .cloned()
            .unwrap_or(serde_json::json!({}));

        // Verify the tool exists
        if !tools.iter().any(|t| t.name == tool_name) {
            return Json(serde_json::json!({
                "jsonrpc": "2.0",
                "id": request.get("id").cloned(),
                "error": {"code": -32602, "message": format!("Unknown tool: {tool_name}")}
            }));
        }

        // Snapshot skill registry before async call (RwLockReadGuard is !Send)
        let skill_snapshot = state
            .kernel
            .skill_registry
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .snapshot();

        // Execute the tool via the kernel's tool runner
        let kernel_handle: Arc<dyn skipper_runtime::kernel_handle::KernelHandle> =
            state.kernel.clone() as Arc<dyn skipper_runtime::kernel_handle::KernelHandle>;
        let result = skipper_runtime::tool_runner::execute_tool(
            "mcp-http",
            tool_name,
            &arguments,
            Some(&kernel_handle),
            None,
            None,
            Some(&skill_snapshot),
            Some(&state.kernel.mcp_connections),
            Some(&state.kernel.web_ctx),
            Some(&state.kernel.browser_ctx),
            None,
            None,
            Some(&state.kernel.media_engine),
            None, // exec_policy
            if state.kernel.config.tts.enabled {
                Some(&state.kernel.tts_engine)
            } else {
                None
            },
            if state.kernel.config.docker.enabled {
                Some(&state.kernel.config.docker)
            } else {
                None
            },
            Some(&*state.kernel.process_manager),
        )
        .await;

        return Json(serde_json::json!({
            "jsonrpc": "2.0",
            "id": request.get("id").cloned(),
            "result": {
                "content": [{"type": "text", "text": result.content}],
                "isError": result.is_error,
            }
        }));
    }

    // For non-tools/call methods (initialize, tools/list, etc.), delegate to the handler
    let response = skipper_runtime::mcp_server::handle_mcp_request(&request, &tools).await;
    Json(response)
}

// ── Multi-Session Endpoints ─────────────────────────────────────────────

/// GET /api/agents/{id}/sessions — List all sessions for an agent.
pub async fn list_agent_sessions(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Invalid agent ID"})),
            )
        }
    };
    match state.kernel.list_agent_sessions(agent_id) {
        Ok(sessions) => (
            StatusCode::OK,
            Json(serde_json::json!({"sessions": sessions})),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("{e}")})),
        ),
    }
}

/// POST /api/agents/{id}/sessions — Create a new session for an agent.
pub async fn create_agent_session(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<serde_json::Value>,
) -> impl IntoResponse {
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Invalid agent ID"})),
            )
        }
    };
    let label = req.get("label").and_then(|v| v.as_str());
    match state.kernel.create_agent_session(agent_id, label) {
        Ok(session) => (StatusCode::OK, Json(session)),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("{e}")})),
        ),
    }
}

/// POST /api/agents/{id}/sessions/{session_id}/switch — Switch to an existing session.
pub async fn switch_agent_session(
    State(state): State<Arc<AppState>>,
    Path((id, session_id_str)): Path<(String, String)>,
) -> impl IntoResponse {
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Invalid agent ID"})),
            )
        }
    };
    let session_id = match session_id_str.parse::<uuid::Uuid>() {
        Ok(uuid) => skipper_types::agent::SessionId(uuid),
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Invalid session ID"})),
            )
        }
    };
    match state.kernel.switch_agent_session(agent_id, session_id) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "ok", "message": "Session switched"})),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("{e}")})),
        ),
    }
}

// ── Extended Chat Command API Endpoints ─────────────────────────────────

/// POST /api/agents/{id}/session/reset — Reset an agent's session.
pub async fn reset_session(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Invalid agent ID"})),
            )
        }
    };
    match state.kernel.reset_session(agent_id) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "ok", "message": "Session reset"})),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("{e}")})),
        ),
    }
}

/// POST /api/agents/{id}/session/compact — Trigger LLM session compaction.
pub async fn compact_session(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Invalid agent ID"})),
            )
        }
    };
    match state.kernel.compact_agent_session(agent_id).await {
        Ok(msg) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "ok", "message": msg})),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("{e}")})),
        ),
    }
}

/// POST /api/agents/{id}/stop — Cancel an agent's current LLM run.
pub async fn stop_agent(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Invalid agent ID"})),
            )
        }
    };
    match state.kernel.stop_agent_run(agent_id) {
        Ok(true) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "ok", "message": "Run cancelled"})),
        ),
        Ok(false) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "ok", "message": "No active run"})),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("{e}")})),
        ),
    }
}

/// PUT /api/agents/{id}/model — Switch an agent's model.
pub async fn set_model(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Invalid agent ID"})),
            )
        }
    };
    let model = match body["model"].as_str() {
        Some(m) if !m.is_empty() => m,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Missing 'model' field"})),
            )
        }
    };
    match state.kernel.set_agent_model(agent_id, model) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "ok", "model": model})),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("{e}")})),
        ),
    }
}

// ── Per-Agent Skill & MCP Endpoints ────────────────────────────────────

/// GET /api/agents/{id}/skills — Get an agent's skill assignment info.
pub async fn get_agent_skills(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Invalid agent ID"})),
            )
        }
    };
    let entry = match state.kernel.registry.get(agent_id) {
        Some(e) => e,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "Agent not found"})),
            )
        }
    };
    let available = state
        .kernel
        .skill_registry
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .skill_names();
    let mode = if entry.manifest.skills.is_empty() {
        "all"
    } else {
        "allowlist"
    };
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "assigned": entry.manifest.skills,
            "available": available,
            "mode": mode,
        })),
    )
}

/// PUT /api/agents/{id}/skills — Update an agent's skill allowlist.
pub async fn set_agent_skills(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Invalid agent ID"})),
            )
        }
    };
    let skills: Vec<String> = body["skills"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    match state.kernel.set_agent_skills(agent_id, skills.clone()) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "ok", "skills": skills})),
        ),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": format!("{e}")})),
        ),
    }
}

/// GET /api/agents/{id}/mcp_servers — Get an agent's MCP server assignment info.
pub async fn get_agent_mcp_servers(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Invalid agent ID"})),
            )
        }
    };
    let entry = match state.kernel.registry.get(agent_id) {
        Some(e) => e,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "Agent not found"})),
            )
        }
    };
    // Collect known MCP server names from connected tools
    let mut available: Vec<String> = Vec::new();
    if let Ok(mcp_tools) = state.kernel.mcp_tools.lock() {
        let mut seen = std::collections::HashSet::new();
        for tool in mcp_tools.iter() {
            if let Some(server) = skipper_runtime::mcp::extract_mcp_server(&tool.name) {
                if seen.insert(server.to_string()) {
                    available.push(server.to_string());
                }
            }
        }
    }
    let mode = if entry.manifest.mcp_servers.is_empty() {
        "all"
    } else {
        "allowlist"
    };
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "assigned": entry.manifest.mcp_servers,
            "available": available,
            "mode": mode,
        })),
    )
}

/// PUT /api/agents/{id}/mcp_servers — Update an agent's MCP server allowlist.
pub async fn set_agent_mcp_servers(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Invalid agent ID"})),
            )
        }
    };
    let servers: Vec<String> = body["mcp_servers"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    match state
        .kernel
        .set_agent_mcp_servers(agent_id, servers.clone())
    {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "ok", "mcp_servers": servers})),
        ),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": format!("{e}")})),
        ),
    }
}

// ── Provider Key Management Endpoints ──────────────────────────────────

/// POST /api/providers/{name}/key — Save an API key for a provider.
///
/// SECURITY: Writes to `~/.skipper/secrets.env`, sets env var in process,
/// and refreshes auth detection. Key is zeroized after use.
pub async fn set_provider_key(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    // Validate provider name against known list
    {
        let catalog = state
            .kernel
            .model_catalog
            .read()
            .unwrap_or_else(|e| e.into_inner());
        if catalog.get_provider(&name).is_none() {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": format!("Unknown provider '{}'", name)})),
            );
        }
    }

    let key = match body["key"].as_str() {
        Some(k) if !k.trim().is_empty() => k.trim().to_string(),
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Missing or empty 'key' field"})),
            );
        }
    };

    let env_var = {
        let catalog = state
            .kernel
            .model_catalog
            .read()
            .unwrap_or_else(|e| e.into_inner());
        catalog
            .get_provider(&name)
            .map(|p| p.api_key_env.clone())
            .unwrap_or_default()
    };

    if env_var.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Provider does not require an API key"})),
        );
    }

    // Write to secrets.env file
    let secrets_path = state.kernel.config.home_dir.join("secrets.env");
    if let Err(e) = write_secret_env(&secrets_path, &env_var, &key) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Failed to write secrets.env: {e}")})),
        );
    }

    // Set env var in current process so detect_auth picks it up
    std::env::set_var(&env_var, &key);

    // Refresh auth detection
    state
        .kernel
        .model_catalog
        .write()
        .unwrap_or_else(|e| e.into_inner())
        .detect_auth();

    (
        StatusCode::OK,
        Json(serde_json::json!({"status": "saved", "provider": name})),
    )
}

/// DELETE /api/providers/{name}/key — Remove an API key for a provider.
pub async fn delete_provider_key(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let env_var = {
        let catalog = state
            .kernel
            .model_catalog
            .read()
            .unwrap_or_else(|e| e.into_inner());
        match catalog.get_provider(&name) {
            Some(p) => p.api_key_env.clone(),
            None => {
                return (
                    StatusCode::NOT_FOUND,
                    Json(serde_json::json!({"error": format!("Unknown provider '{}'", name)})),
                );
            }
        }
    };

    if env_var.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Provider does not require an API key"})),
        );
    }

    // Remove from secrets.env
    let secrets_path = state.kernel.config.home_dir.join("secrets.env");
    if let Err(e) = remove_secret_env(&secrets_path, &env_var) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Failed to update secrets.env: {e}")})),
        );
    }

    // Remove from process environment
    std::env::remove_var(&env_var);

    // Refresh auth detection
    state
        .kernel
        .model_catalog
        .write()
        .unwrap_or_else(|e| e.into_inner())
        .detect_auth();

    (
        StatusCode::OK,
        Json(serde_json::json!({"status": "removed", "provider": name})),
    )
}

/// POST /api/providers/{name}/test — Test a provider's connectivity.
pub async fn test_provider(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let (env_var, base_url, key_required) = {
        let catalog = state
            .kernel
            .model_catalog
            .read()
            .unwrap_or_else(|e| e.into_inner());
        match catalog.get_provider(&name) {
            Some(p) => (p.api_key_env.clone(), p.base_url.clone(), p.key_required),
            None => {
                return (
                    StatusCode::NOT_FOUND,
                    Json(serde_json::json!({"error": format!("Unknown provider '{}'", name)})),
                );
            }
        }
    };

    let api_key = std::env::var(&env_var).ok();
    // Only require API key for providers that need one (skip local providers like ollama/vllm/lmstudio)
    if key_required && api_key.is_none() && !env_var.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Provider API key not configured"})),
        );
    }

    // Attempt a lightweight connectivity test
    let start = std::time::Instant::now();
    let driver_config = skipper_runtime::llm_driver::DriverConfig {
        provider: name.clone(),
        api_key,
        base_url: if base_url.is_empty() {
            None
        } else {
            Some(base_url)
        },
    };

    match skipper_runtime::drivers::create_driver(&driver_config) {
        Ok(driver) => {
            // Send a minimal completion request to test connectivity
            let test_req = skipper_runtime::llm_driver::CompletionRequest {
                model: String::new(), // Driver will use default
                messages: vec![skipper_types::message::Message::user("Hi")],
                tools: vec![],
                max_tokens: 1,
                temperature: 0.0,
                system: None,
                thinking: None,
            };
            match driver.complete(test_req).await {
                Ok(_) => {
                    let latency_ms = start.elapsed().as_millis();
                    (
                        StatusCode::OK,
                        Json(serde_json::json!({
                            "status": "ok",
                            "provider": name,
                            "latency_ms": latency_ms,
                        })),
                    )
                }
                Err(e) => (
                    StatusCode::OK,
                    Json(serde_json::json!({
                        "status": "error",
                        "provider": name,
                        "error": format!("{e}"),
                    })),
                ),
            }
        }
        Err(e) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "error",
                "provider": name,
                "error": format!("Failed to create driver: {e}"),
            })),
        ),
    }
}

/// PUT /api/providers/{name}/url — Set a custom base URL for a provider.
pub async fn set_provider_url(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    // Validate provider exists
    let provider_exists = {
        let catalog = state
            .kernel
            .model_catalog
            .read()
            .unwrap_or_else(|e| e.into_inner());
        catalog.get_provider(&name).is_some()
    };
    if !provider_exists {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": format!("Unknown provider '{}'", name)})),
        );
    }

    let base_url = match body["base_url"].as_str() {
        Some(u) if !u.trim().is_empty() => u.trim().to_string(),
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Missing or empty 'base_url' field"})),
            );
        }
    };

    // Validate URL scheme
    if !base_url.starts_with("http://") && !base_url.starts_with("https://") {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "base_url must start with http:// or https://"})),
        );
    }

    // Update catalog in memory
    {
        let mut catalog = state
            .kernel
            .model_catalog
            .write()
            .unwrap_or_else(|e| e.into_inner());
        catalog.set_provider_url(&name, &base_url);
    }

    // Persist to config.toml [provider_urls] section
    let config_path = state.kernel.config.home_dir.join("config.toml");
    if let Err(e) = upsert_provider_url(&config_path, &name, &base_url) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Failed to save config: {e}")})),
        );
    }

    // Probe reachability at the new URL
    let probe =
        skipper_runtime::provider_health::probe_provider(&name, &base_url).await;

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "saved",
            "provider": name,
            "base_url": base_url,
            "reachable": probe.reachable,
            "latency_ms": probe.latency_ms,
        })),
    )
}

/// Upsert a provider URL in the `[provider_urls]` section of config.toml.
fn upsert_provider_url(
    config_path: &std::path::Path,
    provider: &str,
    url: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let content = if config_path.exists() {
        std::fs::read_to_string(config_path)?
    } else {
        String::new()
    };

    let mut doc: toml::Value = if content.trim().is_empty() {
        toml::Value::Table(toml::map::Map::new())
    } else {
        toml::from_str(&content)?
    };

    let root = doc.as_table_mut().ok_or("Config is not a TOML table")?;

    if !root.contains_key("provider_urls") {
        root.insert(
            "provider_urls".to_string(),
            toml::Value::Table(toml::map::Map::new()),
        );
    }
    let urls_table = root
        .get_mut("provider_urls")
        .and_then(|v| v.as_table_mut())
        .ok_or("provider_urls is not a table")?;

    urls_table.insert(provider.to_string(), toml::Value::String(url.to_string()));

    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    std::fs::write(config_path, toml::to_string_pretty(&doc)?)?;
    Ok(())
}

/// POST /api/skills/create — Create a local prompt-only skill.
pub async fn create_skill(
    State(state): State<Arc<AppState>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let name = match body["name"].as_str() {
        Some(n) if !n.trim().is_empty() => n.trim().to_string(),
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Missing or empty 'name' field"})),
            );
        }
    };

    // Validate name (alphanumeric + hyphens only)
    if !name
        .chars()
        .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(
                serde_json::json!({"error": "Skill name must contain only letters, numbers, hyphens, and underscores"}),
            ),
        );
    }

    let description = body["description"].as_str().unwrap_or("").to_string();
    let runtime = body["runtime"].as_str().unwrap_or("prompt_only");
    let prompt_context = body["prompt_context"].as_str().unwrap_or("").to_string();

    // Only allow prompt_only skills from the web UI for safety
    if runtime != "prompt_only" {
        return (
            StatusCode::BAD_REQUEST,
            Json(
                serde_json::json!({"error": "Only prompt_only skills can be created from the web UI"}),
            ),
        );
    }

    // Write skill.toml to ~/.skipper/skills/{name}/
    let skill_dir = state.kernel.config.home_dir.join("skills").join(&name);
    if skill_dir.exists() {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({"error": format!("Skill '{}' already exists", name)})),
        );
    }

    if let Err(e) = std::fs::create_dir_all(&skill_dir) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Failed to create skill directory: {e}")})),
        );
    }

    let toml_content = format!(
        "[skill]\nname = \"{}\"\ndescription = \"{}\"\nruntime = \"prompt_only\"\n\n[prompt]\ncontext = \"\"\"\n{}\n\"\"\"\n",
        name,
        description.replace('"', "\\\""),
        prompt_context
    );

    let toml_path = skill_dir.join("skill.toml");
    if let Err(e) = std::fs::write(&toml_path, &toml_content) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Failed to write skill.toml: {e}")})),
        );
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "created",
            "name": name,
            "note": "Restart the daemon to load the new skill, or it will be available on next boot."
        })),
    )
}

// ── Helper functions for secrets.env management ────────────────────────

/// Write or update a key in the secrets.env file.
/// File format: one `KEY=value` per line. Existing keys are overwritten.
fn write_secret_env(path: &std::path::Path, key: &str, value: &str) -> Result<(), std::io::Error> {
    let mut lines: Vec<String> = if path.exists() {
        std::fs::read_to_string(path)?
            .lines()
            .map(|l| l.to_string())
            .collect()
    } else {
        Vec::new()
    };

    // Remove existing line for this key
    lines.retain(|l| !l.starts_with(&format!("{key}=")));

    // Add new line
    lines.push(format!("{key}={value}"));

    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    std::fs::write(path, lines.join("\n") + "\n")?;

    // SECURITY: Restrict file permissions on Unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }

    Ok(())
}

/// Remove a key from the secrets.env file.
fn remove_secret_env(path: &std::path::Path, key: &str) -> Result<(), std::io::Error> {
    if !path.exists() {
        return Ok(());
    }

    let lines: Vec<String> = std::fs::read_to_string(path)?
        .lines()
        .filter(|l| !l.starts_with(&format!("{key}=")))
        .map(|l| l.to_string())
        .collect();

    std::fs::write(path, lines.join("\n") + "\n")?;

    Ok(())
}

// ── Config.toml channel management helpers ──────────────────────────

/// Upsert a `[channels.<name>]` section in config.toml with the given non-secret fields.
fn upsert_channel_config(
    config_path: &std::path::Path,
    channel_name: &str,
    fields: &HashMap<String, String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let content = if config_path.exists() {
        std::fs::read_to_string(config_path)?
    } else {
        String::new()
    };

    let mut doc: toml::Value = if content.trim().is_empty() {
        toml::Value::Table(toml::map::Map::new())
    } else {
        toml::from_str(&content)?
    };

    let root = doc.as_table_mut().ok_or("Config is not a TOML table")?;

    // Ensure [channels] table exists
    if !root.contains_key("channels") {
        root.insert(
            "channels".to_string(),
            toml::Value::Table(toml::map::Map::new()),
        );
    }
    let channels_table = root
        .get_mut("channels")
        .and_then(|v| v.as_table_mut())
        .ok_or("channels is not a table")?;

    // Build channel sub-table
    let mut ch_table = toml::map::Map::new();
    for (k, v) in fields {
        ch_table.insert(k.clone(), toml::Value::String(v.clone()));
    }
    channels_table.insert(channel_name.to_string(), toml::Value::Table(ch_table));

    // Ensure parent directory exists
    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    std::fs::write(config_path, toml::to_string_pretty(&doc)?)?;
    Ok(())
}

/// Remove a `[channels.<name>]` section from config.toml.
fn remove_channel_config(
    config_path: &std::path::Path,
    channel_name: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    if !config_path.exists() {
        return Ok(());
    }

    let content = std::fs::read_to_string(config_path)?;
    if content.trim().is_empty() {
        return Ok(());
    }

    let mut doc: toml::Value = toml::from_str(&content)?;

    if let Some(channels) = doc
        .as_table_mut()
        .and_then(|r| r.get_mut("channels"))
        .and_then(|c| c.as_table_mut())
    {
        channels.remove(channel_name);
    }

    std::fs::write(config_path, toml::to_string_pretty(&doc)?)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Integration management endpoints
// ---------------------------------------------------------------------------

/// GET /api/integrations — List installed integrations with status.
pub async fn list_integrations(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let registry = state
        .kernel
        .extension_registry
        .read()
        .unwrap_or_else(|e| e.into_inner());
    let health = &state.kernel.extension_health;

    let mut entries = Vec::new();
    for info in registry.list_all_info() {
        let h = health.get_health(&info.template.id);
        let status = match &info.installed {
            Some(inst) if !inst.enabled => "disabled",
            Some(_) => match h.as_ref().map(|h| &h.status) {
                Some(skipper_extensions::IntegrationStatus::Ready) => "ready",
                Some(skipper_extensions::IntegrationStatus::Error(_)) => "error",
                _ => "installed",
            },
            None => continue, // Only show installed
        };
        entries.push(serde_json::json!({
            "id": info.template.id,
            "name": info.template.name,
            "icon": info.template.icon,
            "category": info.template.category.to_string(),
            "status": status,
            "tool_count": h.as_ref().map(|h| h.tool_count).unwrap_or(0),
            "installed_at": info.installed.as_ref().map(|i| i.installed_at.to_rfc3339()),
        }));
    }

    Json(serde_json::json!({
        "installed": entries,
        "count": entries.len(),
    }))
}

/// GET /api/integrations/available — List all available templates.
pub async fn list_available_integrations(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let registry = state
        .kernel
        .extension_registry
        .read()
        .unwrap_or_else(|e| e.into_inner());
    let templates: Vec<serde_json::Value> = registry
        .list_templates()
        .iter()
        .map(|t| {
            let installed = registry.is_installed(&t.id);
            serde_json::json!({
                "id": t.id,
                "name": t.name,
                "description": t.description,
                "icon": t.icon,
                "category": t.category.to_string(),
                "installed": installed,
                "tags": t.tags,
                "required_env": t.required_env.iter().map(|e| serde_json::json!({
                    "name": e.name,
                    "label": e.label,
                    "help": e.help,
                    "is_secret": e.is_secret,
                    "get_url": e.get_url,
                })).collect::<Vec<_>>(),
                "has_oauth": t.oauth.is_some(),
                "setup_instructions": t.setup_instructions,
            })
        })
        .collect();

    Json(serde_json::json!({
        "integrations": templates,
        "count": templates.len(),
    }))
}

/// POST /api/integrations/add — Install an integration.
pub async fn add_integration(
    State(state): State<Arc<AppState>>,
    Json(req): Json<serde_json::Value>,
) -> impl IntoResponse {
    let id = match req.get("id").and_then(|v| v.as_str()) {
        Some(id) => id.to_string(),
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Missing 'id' field"})),
            );
        }
    };

    // Scope the write lock so it's dropped before any .await
    let install_err = {
        let mut registry = state
            .kernel
            .extension_registry
            .write()
            .unwrap_or_else(|e| e.into_inner());

        if registry.is_installed(&id) {
            Some((
                StatusCode::CONFLICT,
                format!("Integration '{}' already installed", id),
            ))
        } else if registry.get_template(&id).is_none() {
            Some((
                StatusCode::NOT_FOUND,
                format!("Unknown integration: '{}'", id),
            ))
        } else {
            let entry = skipper_extensions::InstalledIntegration {
                id: id.clone(),
                installed_at: chrono::Utc::now(),
                enabled: true,
                oauth_provider: None,
                config: std::collections::HashMap::new(),
            };
            match registry.install(entry) {
                Ok(_) => None,
                Err(e) => Some((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
            }
        }
    }; // write lock dropped here

    if let Some((status, error)) = install_err {
        return (status, Json(serde_json::json!({"error": error})));
    }

    state.kernel.extension_health.register(&id);

    // Hot-connect the new MCP server
    let connected = state.kernel.reload_extension_mcps().await.unwrap_or(0);

    (
        StatusCode::CREATED,
        Json(serde_json::json!({
            "id": id,
            "status": "installed",
            "connected": connected > 0,
            "message": format!("Integration '{}' installed", id),
        })),
    )
}

/// DELETE /api/integrations/:id — Remove an integration.
pub async fn remove_integration(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    // Scope the write lock
    let uninstall_err = {
        let mut registry = state
            .kernel
            .extension_registry
            .write()
            .unwrap_or_else(|e| e.into_inner());
        registry.uninstall(&id).err()
    };

    if let Some(e) = uninstall_err {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": e.to_string()})),
        );
    }

    state.kernel.extension_health.unregister(&id);

    // Hot-disconnect the removed MCP server
    let _ = state.kernel.reload_extension_mcps().await;

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "id": id,
            "status": "removed",
        })),
    )
}

/// POST /api/integrations/:id/reconnect — Reconnect an MCP server.
pub async fn reconnect_integration(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let is_installed = {
        let registry = state
            .kernel
            .extension_registry
            .read()
            .unwrap_or_else(|e| e.into_inner());
        registry.is_installed(&id)
    };

    if !is_installed {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": format!("Integration '{}' not installed", id)})),
        );
    }

    match state.kernel.reconnect_extension_mcp(&id).await {
        Ok(tool_count) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "id": id,
                "status": "connected",
                "tool_count": tool_count,
            })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "id": id,
                "status": "error",
                "error": e,
            })),
        ),
    }
}

/// GET /api/integrations/health — Health status for all integrations.
pub async fn integrations_health(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let health_entries = state.kernel.extension_health.all_health();
    let entries: Vec<serde_json::Value> = health_entries
        .iter()
        .map(|h| {
            serde_json::json!({
                "id": h.id,
                "status": h.status.to_string(),
                "tool_count": h.tool_count,
                "last_ok": h.last_ok.map(|t| t.to_rfc3339()),
                "last_error": h.last_error,
                "consecutive_failures": h.consecutive_failures,
                "reconnecting": h.reconnecting,
                "reconnect_attempts": h.reconnect_attempts,
                "connected_since": h.connected_since.map(|t| t.to_rfc3339()),
            })
        })
        .collect();

    Json(serde_json::json!({
        "health": entries,
        "count": entries.len(),
    }))
}

/// POST /api/integrations/reload — Hot-reload integration configs and reconnect MCP.
pub async fn reload_integrations(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    match state.kernel.reload_extension_mcps().await {
        Ok(connected) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "reloaded",
                "new_connections": connected,
            })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e})),
        ),
    }
}

// ---------------------------------------------------------------------------
// Scheduled Jobs (cron) endpoints
// ---------------------------------------------------------------------------

/// The well-known shared-memory agent ID used for cross-agent KV storage.
/// Must match the value in `skipper-kernel/src/kernel.rs::shared_memory_agent_id()`.
fn schedule_shared_agent_id() -> AgentId {
    AgentId(uuid::Uuid::from_bytes([
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x01,
    ]))
}

const SCHEDULES_KEY: &str = "__skipper_schedules";

/// GET /api/schedules — List all cron-based scheduled jobs.
pub async fn list_schedules(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let agent_id = schedule_shared_agent_id();
    match state.kernel.memory.structured_get(agent_id, SCHEDULES_KEY) {
        Ok(Some(serde_json::Value::Array(arr))) => {
            let total = arr.len();
            Json(serde_json::json!({"schedules": arr, "total": total}))
        }
        Ok(_) => Json(serde_json::json!({"schedules": [], "total": 0})),
        Err(e) => {
            tracing::warn!("Failed to load schedules: {e}");
            Json(serde_json::json!({"schedules": [], "total": 0, "error": format!("{e}")}))
        }
    }
}

/// POST /api/schedules — Create a new cron-based scheduled job.
pub async fn create_schedule(
    State(state): State<Arc<AppState>>,
    Json(req): Json<serde_json::Value>,
) -> impl IntoResponse {
    let name = match req["name"].as_str() {
        Some(n) if !n.is_empty() => n.to_string(),
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Missing 'name' field"})),
            );
        }
    };

    let cron = match req["cron"].as_str() {
        Some(c) if !c.is_empty() => c.to_string(),
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Missing 'cron' field"})),
            );
        }
    };

    // Validate cron expression: must be 5 space-separated fields
    let cron_parts: Vec<&str> = cron.split_whitespace().collect();
    if cron_parts.len() != 5 {
        return (
            StatusCode::BAD_REQUEST,
            Json(
                serde_json::json!({"error": "Invalid cron expression: must have 5 fields (min hour dom mon dow)"}),
            ),
        );
    }

    let agent_id_str = req["agent_id"].as_str().unwrap_or("").to_string();
    let message = req["message"].as_str().unwrap_or("").to_string();
    let enabled = req.get("enabled").and_then(|v| v.as_bool()).unwrap_or(true);

    let schedule_id = uuid::Uuid::new_v4().to_string();
    let entry = serde_json::json!({
        "id": schedule_id,
        "name": name,
        "cron": cron,
        "agent_id": agent_id_str,
        "message": message,
        "enabled": enabled,
        "created_at": chrono::Utc::now().to_rfc3339(),
        "last_run": null,
        "run_count": 0,
    });

    let shared_id = schedule_shared_agent_id();
    let mut schedules: Vec<serde_json::Value> =
        match state.kernel.memory.structured_get(shared_id, SCHEDULES_KEY) {
            Ok(Some(serde_json::Value::Array(arr))) => arr,
            _ => Vec::new(),
        };

    schedules.push(entry.clone());
    if let Err(e) = state.kernel.memory.structured_set(
        shared_id,
        SCHEDULES_KEY,
        serde_json::Value::Array(schedules),
    ) {
        tracing::warn!("Failed to save schedule: {e}");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Failed to save schedule: {e}")})),
        );
    }

    (StatusCode::CREATED, Json(entry))
}

/// PUT /api/schedules/:id — Update a scheduled job (toggle enabled, edit fields).
pub async fn update_schedule(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<serde_json::Value>,
) -> impl IntoResponse {
    let shared_id = schedule_shared_agent_id();
    let mut schedules: Vec<serde_json::Value> =
        match state.kernel.memory.structured_get(shared_id, SCHEDULES_KEY) {
            Ok(Some(serde_json::Value::Array(arr))) => arr,
            _ => Vec::new(),
        };

    let mut found = false;
    for s in schedules.iter_mut() {
        if s["id"].as_str() == Some(&id) {
            found = true;
            if let Some(enabled) = req.get("enabled").and_then(|v| v.as_bool()) {
                s["enabled"] = serde_json::Value::Bool(enabled);
            }
            if let Some(name) = req.get("name").and_then(|v| v.as_str()) {
                s["name"] = serde_json::Value::String(name.to_string());
            }
            if let Some(cron) = req.get("cron").and_then(|v| v.as_str()) {
                let cron_parts: Vec<&str> = cron.split_whitespace().collect();
                if cron_parts.len() != 5 {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(serde_json::json!({"error": "Invalid cron expression"})),
                    );
                }
                s["cron"] = serde_json::Value::String(cron.to_string());
            }
            if let Some(agent_id) = req.get("agent_id").and_then(|v| v.as_str()) {
                s["agent_id"] = serde_json::Value::String(agent_id.to_string());
            }
            if let Some(message) = req.get("message").and_then(|v| v.as_str()) {
                s["message"] = serde_json::Value::String(message.to_string());
            }
            break;
        }
    }

    if !found {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "Schedule not found"})),
        );
    }

    if let Err(e) = state.kernel.memory.structured_set(
        shared_id,
        SCHEDULES_KEY,
        serde_json::Value::Array(schedules),
    ) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Failed to update schedule: {e}")})),
        );
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({"status": "updated", "schedule_id": id})),
    )
}

/// DELETE /api/schedules/:id — Remove a scheduled job.
pub async fn delete_schedule(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let shared_id = schedule_shared_agent_id();
    let mut schedules: Vec<serde_json::Value> =
        match state.kernel.memory.structured_get(shared_id, SCHEDULES_KEY) {
            Ok(Some(serde_json::Value::Array(arr))) => arr,
            _ => Vec::new(),
        };

    let before = schedules.len();
    schedules.retain(|s| s["id"].as_str() != Some(&id));

    if schedules.len() == before {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "Schedule not found"})),
        );
    }

    if let Err(e) = state.kernel.memory.structured_set(
        shared_id,
        SCHEDULES_KEY,
        serde_json::Value::Array(schedules),
    ) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Failed to delete schedule: {e}")})),
        );
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({"status": "removed", "schedule_id": id})),
    )
}

/// POST /api/schedules/:id/run — Manually run a scheduled job now.
pub async fn run_schedule(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let shared_id = schedule_shared_agent_id();
    let schedules: Vec<serde_json::Value> =
        match state.kernel.memory.structured_get(shared_id, SCHEDULES_KEY) {
            Ok(Some(serde_json::Value::Array(arr))) => arr,
            _ => Vec::new(),
        };

    let schedule = match schedules.iter().find(|s| s["id"].as_str() == Some(&id)) {
        Some(s) => s.clone(),
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "Schedule not found"})),
            );
        }
    };

    let agent_id_str = schedule["agent_id"].as_str().unwrap_or("");
    let message = schedule["message"]
        .as_str()
        .unwrap_or("Scheduled task triggered manually.");
    let name = schedule["name"].as_str().unwrap_or("(unnamed)");

    // Find the target agent
    let target_agent = if !agent_id_str.is_empty() {
        if let Ok(aid) = agent_id_str.parse::<AgentId>() {
            Some(aid)
        } else {
            state
                .kernel
                .registry
                .list()
                .iter()
                .find(|a| a.name == agent_id_str)
                .map(|a| a.id)
        }
    } else {
        state.kernel.registry.list().first().map(|a| a.id)
    };

    let target_agent = match target_agent {
        Some(a) => a,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(
                    serde_json::json!({"error": "No target agent found. Specify an agent_id or start an agent first."}),
                ),
            );
        }
    };

    let run_message = if message.is_empty() {
        format!("[Scheduled task '{}' triggered manually]", name)
    } else {
        message.to_string()
    };

    // Update last_run and run_count
    let mut schedules_updated: Vec<serde_json::Value> =
        match state.kernel.memory.structured_get(shared_id, SCHEDULES_KEY) {
            Ok(Some(serde_json::Value::Array(arr))) => arr,
            _ => Vec::new(),
        };
    for s in schedules_updated.iter_mut() {
        if s["id"].as_str() == Some(&id) {
            s["last_run"] = serde_json::Value::String(chrono::Utc::now().to_rfc3339());
            let count = s["run_count"].as_u64().unwrap_or(0);
            s["run_count"] = serde_json::json!(count + 1);
            break;
        }
    }
    let _ = state.kernel.memory.structured_set(
        shared_id,
        SCHEDULES_KEY,
        serde_json::Value::Array(schedules_updated),
    );

    match state.kernel.send_message(target_agent, &run_message).await {
        Ok(result) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "completed",
                "schedule_id": id,
                "agent_id": target_agent.to_string(),
                "response": result.response,
            })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "status": "failed",
                "schedule_id": id,
                "error": format!("{e}"),
            })),
        ),
    }
}

// ---------------------------------------------------------------------------
// Agent Identity endpoint
// ---------------------------------------------------------------------------

/// Request body for updating agent visual identity.
#[derive(serde::Deserialize)]
pub struct UpdateIdentityRequest {
    pub emoji: Option<String>,
    pub avatar_url: Option<String>,
    pub color: Option<String>,
    #[serde(default)]
    pub archetype: Option<String>,
    #[serde(default)]
    pub vibe: Option<String>,
    #[serde(default)]
    pub greeting_style: Option<String>,
}

/// PATCH /api/agents/{id}/identity — Update an agent's visual identity.
pub async fn update_agent_identity(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<UpdateIdentityRequest>,
) -> impl IntoResponse {
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Invalid agent ID"})),
            );
        }
    };

    // Validate color format if provided
    if let Some(ref color) = req.color {
        if !color.is_empty() && !color.starts_with('#') {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Color must be a hex code starting with '#'"})),
            );
        }
    }

    // Validate avatar_url if provided
    if let Some(ref url) = req.avatar_url {
        if !url.is_empty()
            && !url.starts_with("http://")
            && !url.starts_with("https://")
            && !url.starts_with("data:")
        {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Avatar URL must be http/https or data URI"})),
            );
        }
    }

    let identity = AgentIdentity {
        emoji: req.emoji,
        avatar_url: req.avatar_url,
        color: req.color,
        archetype: req.archetype,
        vibe: req.vibe,
        greeting_style: req.greeting_style,
    };

    match state.kernel.registry.update_identity(agent_id, identity) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "ok", "agent_id": id})),
        ),
        Err(_) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "Agent not found"})),
        ),
    }
}

// ---------------------------------------------------------------------------
// Agent Config Hot-Update
// ---------------------------------------------------------------------------

/// Request body for patching agent config (name, description, prompt, identity).
#[derive(serde::Deserialize)]
pub struct PatchAgentConfigRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    pub system_prompt: Option<String>,
    pub emoji: Option<String>,
    pub avatar_url: Option<String>,
    pub color: Option<String>,
    pub archetype: Option<String>,
    pub vibe: Option<String>,
    pub greeting_style: Option<String>,
}

/// PATCH /api/agents/{id}/config — Hot-update agent name, description, system prompt, and identity.
pub async fn patch_agent_config(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<PatchAgentConfigRequest>,
) -> impl IntoResponse {
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Invalid agent ID"})),
            );
        }
    };

    // Input length limits
    const MAX_NAME_LEN: usize = 256;
    const MAX_DESC_LEN: usize = 4096;
    const MAX_PROMPT_LEN: usize = 65_536;

    if let Some(ref name) = req.name {
        if name.len() > MAX_NAME_LEN {
            return (
                StatusCode::PAYLOAD_TOO_LARGE,
                Json(serde_json::json!({"error": format!("Name exceeds max length ({MAX_NAME_LEN} chars)")})),
            );
        }
    }
    if let Some(ref desc) = req.description {
        if desc.len() > MAX_DESC_LEN {
            return (
                StatusCode::PAYLOAD_TOO_LARGE,
                Json(serde_json::json!({"error": format!("Description exceeds max length ({MAX_DESC_LEN} chars)")})),
            );
        }
    }
    if let Some(ref prompt) = req.system_prompt {
        if prompt.len() > MAX_PROMPT_LEN {
            return (
                StatusCode::PAYLOAD_TOO_LARGE,
                Json(serde_json::json!({"error": format!("System prompt exceeds max length ({MAX_PROMPT_LEN} chars)")})),
            );
        }
    }

    // Validate color format if provided
    if let Some(ref color) = req.color {
        if !color.is_empty() && !color.starts_with('#') {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Color must be a hex code starting with '#'"})),
            );
        }
    }

    // Validate avatar_url if provided
    if let Some(ref url) = req.avatar_url {
        if !url.is_empty()
            && !url.starts_with("http://")
            && !url.starts_with("https://")
            && !url.starts_with("data:")
        {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Avatar URL must be http/https or data URI"})),
            );
        }
    }

    // Update name
    if let Some(ref new_name) = req.name {
        if !new_name.is_empty() {
            if let Err(e) = state
                .kernel
                .registry
                .update_name(agent_id, new_name.clone())
            {
                return (
                    StatusCode::CONFLICT,
                    Json(serde_json::json!({"error": format!("{e}")})),
                );
            }
        }
    }

    // Update description
    if let Some(ref new_desc) = req.description {
        if state
            .kernel
            .registry
            .update_description(agent_id, new_desc.clone())
            .is_err()
        {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "Agent not found"})),
            );
        }
    }

    // Update system prompt (hot-swap — takes effect on next message)
    if let Some(ref new_prompt) = req.system_prompt {
        if state
            .kernel
            .registry
            .update_system_prompt(agent_id, new_prompt.clone())
            .is_err()
        {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "Agent not found"})),
            );
        }
    }

    // Update identity fields (merge — only overwrite provided fields)
    let has_identity_field = req.emoji.is_some()
        || req.avatar_url.is_some()
        || req.color.is_some()
        || req.archetype.is_some()
        || req.vibe.is_some()
        || req.greeting_style.is_some();

    if has_identity_field {
        // Read current identity, merge with provided fields
        let current = state
            .kernel
            .registry
            .get(agent_id)
            .map(|e| e.identity)
            .unwrap_or_default();
        let merged = AgentIdentity {
            emoji: req.emoji.or(current.emoji),
            avatar_url: req.avatar_url.or(current.avatar_url),
            color: req.color.or(current.color),
            archetype: req.archetype.or(current.archetype),
            vibe: req.vibe.or(current.vibe),
            greeting_style: req.greeting_style.or(current.greeting_style),
        };
        if state
            .kernel
            .registry
            .update_identity(agent_id, merged)
            .is_err()
        {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "Agent not found"})),
            );
        }
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({"status": "ok", "agent_id": id})),
    )
}

// ---------------------------------------------------------------------------
// Agent Cloning
// ---------------------------------------------------------------------------

/// Request body for cloning an agent.
#[derive(serde::Deserialize)]
pub struct CloneAgentRequest {
    pub new_name: String,
}

/// POST /api/agents/{id}/clone — Clone an agent with its workspace files.
pub async fn clone_agent(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<CloneAgentRequest>,
) -> impl IntoResponse {
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Invalid agent ID"})),
            );
        }
    };

    if req.new_name.len() > 256 {
        return (
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(serde_json::json!({"error": "Name exceeds max length (256 chars)"})),
        );
    }

    if req.new_name.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "new_name cannot be empty"})),
        );
    }

    let source = match state.kernel.registry.get(agent_id) {
        Some(e) => e,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "Agent not found"})),
            );
        }
    };

    // Deep-clone manifest with new name
    let mut cloned_manifest = source.manifest.clone();
    cloned_manifest.name = req.new_name.clone();
    cloned_manifest.workspace = None; // Let kernel assign a new workspace

    // Spawn the cloned agent
    let new_id = match state.kernel.spawn_agent(cloned_manifest) {
        Ok(id) => id,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": format!("Clone spawn failed: {e}")})),
            );
        }
    };

    // Copy workspace files from source to destination
    let new_entry = state.kernel.registry.get(new_id);
    if let (Some(ref src_ws), Some(ref new_entry)) = (source.manifest.workspace, new_entry) {
        if let Some(ref dst_ws) = new_entry.manifest.workspace {
            // Security: canonicalize both paths
            if let (Ok(src_can), Ok(dst_can)) = (src_ws.canonicalize(), dst_ws.canonicalize()) {
                for &fname in KNOWN_IDENTITY_FILES {
                    let src_file = src_can.join(fname);
                    let dst_file = dst_can.join(fname);
                    if src_file.exists() {
                        let _ = std::fs::copy(&src_file, &dst_file);
                    }
                }
            }
        }
    }

    // Copy identity from source
    let _ = state
        .kernel
        .registry
        .update_identity(new_id, source.identity.clone());

    (
        StatusCode::CREATED,
        Json(serde_json::json!({
            "agent_id": new_id.to_string(),
            "name": req.new_name,
        })),
    )
}

// ---------------------------------------------------------------------------
// Workspace File Editor endpoints
// ---------------------------------------------------------------------------

/// Whitelisted workspace identity files that can be read/written via API.
const KNOWN_IDENTITY_FILES: &[&str] = &[
    "SOUL.md",
    "IDENTITY.md",
    "USER.md",
    "TOOLS.md",
    "MEMORY.md",
    "AGENTS.md",
    "BOOTSTRAP.md",
    "HEARTBEAT.md",
];

/// GET /api/agents/{id}/files — List workspace identity files.
pub async fn list_agent_files(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Invalid agent ID"})),
            );
        }
    };

    let entry = match state.kernel.registry.get(agent_id) {
        Some(e) => e,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "Agent not found"})),
            );
        }
    };

    let workspace = match entry.manifest.workspace {
        Some(ref ws) => ws.clone(),
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "Agent has no workspace"})),
            );
        }
    };

    let mut files = Vec::new();
    for &name in KNOWN_IDENTITY_FILES {
        let path = workspace.join(name);
        let (exists, size_bytes) = if path.exists() {
            let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            (true, size)
        } else {
            (false, 0u64)
        };
        files.push(serde_json::json!({
            "name": name,
            "exists": exists,
            "size_bytes": size_bytes,
        }));
    }

    (StatusCode::OK, Json(serde_json::json!({ "files": files })))
}

/// GET /api/agents/{id}/files/{filename} — Read a workspace identity file.
pub async fn get_agent_file(
    State(state): State<Arc<AppState>>,
    Path((id, filename)): Path<(String, String)>,
) -> impl IntoResponse {
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Invalid agent ID"})),
            );
        }
    };

    // Validate filename whitelist
    if !KNOWN_IDENTITY_FILES.contains(&filename.as_str()) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "File not in whitelist"})),
        );
    }

    let entry = match state.kernel.registry.get(agent_id) {
        Some(e) => e,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "Agent not found"})),
            );
        }
    };

    let workspace = match entry.manifest.workspace {
        Some(ref ws) => ws.clone(),
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "Agent has no workspace"})),
            );
        }
    };

    // Security: canonicalize and verify stays inside workspace
    let file_path = workspace.join(&filename);
    let canonical = match file_path.canonicalize() {
        Ok(p) => p,
        Err(_) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "File not found"})),
            );
        }
    };
    let ws_canonical = match workspace.canonicalize() {
        Ok(p) => p,
        Err(_) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "Workspace path error"})),
            );
        }
    };
    if !canonical.starts_with(&ws_canonical) {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({"error": "Path traversal denied"})),
        );
    }

    let content = match std::fs::read_to_string(&canonical) {
        Ok(c) => c,
        Err(_) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "File not found"})),
            );
        }
    };

    let size_bytes = content.len();
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "name": filename,
            "content": content,
            "size_bytes": size_bytes,
        })),
    )
}

/// Request body for writing a workspace identity file.
#[derive(serde::Deserialize)]
pub struct SetAgentFileRequest {
    pub content: String,
}

/// PUT /api/agents/{id}/files/{filename} — Write a workspace identity file.
pub async fn set_agent_file(
    State(state): State<Arc<AppState>>,
    Path((id, filename)): Path<(String, String)>,
    Json(req): Json<SetAgentFileRequest>,
) -> impl IntoResponse {
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Invalid agent ID"})),
            );
        }
    };

    // Validate filename whitelist
    if !KNOWN_IDENTITY_FILES.contains(&filename.as_str()) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "File not in whitelist"})),
        );
    }

    // Max 32KB content
    const MAX_FILE_SIZE: usize = 32_768;
    if req.content.len() > MAX_FILE_SIZE {
        return (
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(serde_json::json!({"error": "File content too large (max 32KB)"})),
        );
    }

    let entry = match state.kernel.registry.get(agent_id) {
        Some(e) => e,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "Agent not found"})),
            );
        }
    };

    let workspace = match entry.manifest.workspace {
        Some(ref ws) => ws.clone(),
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "Agent has no workspace"})),
            );
        }
    };

    // Security: verify workspace path and target stays inside it
    let ws_canonical = match workspace.canonicalize() {
        Ok(p) => p,
        Err(_) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "Workspace path error"})),
            );
        }
    };

    let file_path = workspace.join(&filename);
    // For new files, check the parent directory instead
    let check_path = if file_path.exists() {
        file_path
            .canonicalize()
            .unwrap_or_else(|_| file_path.clone())
    } else {
        // Parent must be inside workspace
        file_path
            .parent()
            .and_then(|p| p.canonicalize().ok())
            .map(|p| p.join(&filename))
            .unwrap_or_else(|| file_path.clone())
    };
    if !check_path.starts_with(&ws_canonical) {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({"error": "Path traversal denied"})),
        );
    }

    // Atomic write: write to .tmp, then rename
    let tmp_path = workspace.join(format!(".{filename}.tmp"));
    if let Err(e) = std::fs::write(&tmp_path, &req.content) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Write failed: {e}")})),
        );
    }
    if let Err(e) = std::fs::rename(&tmp_path, &file_path) {
        let _ = std::fs::remove_file(&tmp_path);
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Rename failed: {e}")})),
        );
    }

    let size_bytes = req.content.len();
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "ok",
            "name": filename,
            "size_bytes": size_bytes,
        })),
    )
}

// ---------------------------------------------------------------------------
// File Upload endpoints
// ---------------------------------------------------------------------------

/// Response body for file uploads.
#[derive(serde::Serialize)]
struct UploadResponse {
    file_id: String,
    filename: String,
    content_type: String,
    size: usize,
    /// Transcription text for audio uploads (populated via Whisper STT).
    #[serde(skip_serializing_if = "Option::is_none")]
    transcription: Option<String>,
}

/// Metadata stored alongside uploaded files.
struct UploadMeta {
    #[allow(dead_code)]
    filename: String,
    content_type: String,
}

/// In-memory upload metadata registry.
static UPLOAD_REGISTRY: LazyLock<DashMap<String, UploadMeta>> = LazyLock::new(DashMap::new);

/// Maximum upload size: 10 MB.
const MAX_UPLOAD_SIZE: usize = 10 * 1024 * 1024;

/// Allowed content type prefixes for upload.
const ALLOWED_CONTENT_TYPES: &[&str] = &["image/", "text/", "application/pdf", "audio/"];

fn is_allowed_content_type(ct: &str) -> bool {
    ALLOWED_CONTENT_TYPES
        .iter()
        .any(|prefix| ct.starts_with(prefix))
}

/// POST /api/agents/{id}/upload — Upload a file attachment.
///
/// Accepts raw body bytes. The client must set:
/// - `Content-Type` header (e.g., `image/png`, `text/plain`, `application/pdf`)
/// - `X-Filename` header (original filename)
pub async fn upload_file(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
) -> impl IntoResponse {
    // Validate agent ID format
    let _agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Invalid agent ID"})),
            );
        }
    };

    // Extract content type
    let content_type = headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/octet-stream")
        .to_string();

    if !is_allowed_content_type(&content_type) {
        return (
            StatusCode::BAD_REQUEST,
            Json(
                serde_json::json!({"error": "Unsupported content type. Allowed: image/*, text/*, audio/*, application/pdf"}),
            ),
        );
    }

    // Extract filename from header
    let filename = headers
        .get("X-Filename")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("upload")
        .to_string();

    // Validate size
    if body.len() > MAX_UPLOAD_SIZE {
        return (
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(
                serde_json::json!({"error": format!("File too large (max {} MB)", MAX_UPLOAD_SIZE / (1024 * 1024))}),
            ),
        );
    }

    if body.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Empty file body"})),
        );
    }

    // Generate file ID and save
    let file_id = uuid::Uuid::new_v4().to_string();
    let upload_dir = std::env::temp_dir().join("skipper_uploads");
    if let Err(e) = std::fs::create_dir_all(&upload_dir) {
        tracing::warn!("Failed to create upload dir: {e}");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": "Failed to create upload directory"})),
        );
    }

    let file_path = upload_dir.join(&file_id);
    if let Err(e) = std::fs::write(&file_path, &body) {
        tracing::warn!("Failed to write upload: {e}");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": "Failed to save file"})),
        );
    }

    let size = body.len();
    UPLOAD_REGISTRY.insert(
        file_id.clone(),
        UploadMeta {
            filename: filename.clone(),
            content_type: content_type.clone(),
        },
    );

    // Auto-transcribe audio uploads using the media engine
    let transcription = if content_type.starts_with("audio/") {
        let attachment = skipper_types::media::MediaAttachment {
            media_type: skipper_types::media::MediaType::Audio,
            mime_type: content_type.clone(),
            source: skipper_types::media::MediaSource::FilePath {
                path: file_path.to_string_lossy().to_string(),
            },
            size_bytes: size as u64,
        };
        match state
            .kernel
            .media_engine
            .transcribe_audio(&attachment)
            .await
        {
            Ok(result) => {
                tracing::info!(chars = result.description.len(), provider = %result.provider, "Audio transcribed");
                Some(result.description)
            }
            Err(e) => {
                tracing::warn!("Audio transcription failed: {e}");
                None
            }
        }
    } else {
        None
    };

    (
        StatusCode::CREATED,
        Json(serde_json::json!(UploadResponse {
            file_id,
            filename,
            content_type,
            size,
            transcription,
        })),
    )
}

/// GET /api/uploads/{file_id} — Serve an uploaded file.
pub async fn serve_upload(Path(file_id): Path<String>) -> impl IntoResponse {
    // Validate file_id is a UUID to prevent path traversal
    if uuid::Uuid::parse_str(&file_id).is_err() {
        return (
            StatusCode::BAD_REQUEST,
            [(
                axum::http::header::CONTENT_TYPE,
                "application/json".to_string(),
            )],
            b"{\"error\":\"Invalid file ID\"}".to_vec(),
        );
    }

    let file_path = std::env::temp_dir().join("skipper_uploads").join(&file_id);

    // Look up metadata from registry; fall back to disk probe for generated images
    // (image_generate saves files without registering in UPLOAD_REGISTRY).
    let content_type = match UPLOAD_REGISTRY.get(&file_id) {
        Some(m) => m.content_type.clone(),
        None => {
            // Infer content type from file magic bytes
            if !file_path.exists() {
                return (
                    StatusCode::NOT_FOUND,
                    [(
                        axum::http::header::CONTENT_TYPE,
                        "application/json".to_string(),
                    )],
                    b"{\"error\":\"File not found\"}".to_vec(),
                );
            }
            "image/png".to_string()
        }
    };

    match std::fs::read(&file_path) {
        Ok(data) => (
            StatusCode::OK,
            [(axum::http::header::CONTENT_TYPE, content_type)],
            data,
        ),
        Err(_) => (
            StatusCode::NOT_FOUND,
            [(
                axum::http::header::CONTENT_TYPE,
                "application/json".to_string(),
            )],
            b"{\"error\":\"File not found on disk\"}".to_vec(),
        ),
    }
}

// ---------------------------------------------------------------------------
// Execution Approval System — backed by kernel.approval_manager
// ---------------------------------------------------------------------------

/// GET /api/approvals — List pending approval requests.
///
/// Transforms field names to match the dashboard template expectations:
/// `action_summary` → `action`, `agent_id` → `agent_name`, `requested_at` → `created_at`.
pub async fn list_approvals(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let pending = state.kernel.approval_manager.list_pending();
    let total = pending.len();

    // Resolve agent names for display
    let registry_agents = state.kernel.registry.list();

    let approvals: Vec<serde_json::Value> = pending
        .into_iter()
        .map(|a| {
            let agent_name = registry_agents
                .iter()
                .find(|ag| ag.id.to_string() == a.agent_id || ag.name == a.agent_id)
                .map(|ag| ag.name.as_str())
                .unwrap_or(&a.agent_id);
            serde_json::json!({
                "id": a.id,
                "agent_id": a.agent_id,
                "agent_name": agent_name,
                "tool_name": a.tool_name,
                "description": a.description,
                "action_summary": a.action_summary,
                "action": a.action_summary,
                "risk_level": a.risk_level,
                "requested_at": a.requested_at,
                "created_at": a.requested_at,
                "timeout_secs": a.timeout_secs,
                "status": "pending"
            })
        })
        .collect();

    Json(serde_json::json!({"approvals": approvals, "total": total}))
}

/// POST /api/approvals — Create a manual approval request (for external systems).
///
/// Note: Most approval requests are created automatically by the tool_runner
/// when an agent invokes a tool that requires approval. This endpoint exists
/// for external integrations that need to inject approval gates.
#[derive(serde::Deserialize)]
pub struct CreateApprovalRequest {
    pub agent_id: String,
    pub tool_name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub action_summary: String,
}

pub async fn create_approval(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateApprovalRequest>,
) -> impl IntoResponse {
    use skipper_types::approval::{ApprovalRequest, RiskLevel};

    let policy = state.kernel.approval_manager.policy();
    let id = uuid::Uuid::new_v4();
    let approval_req = ApprovalRequest {
        id,
        agent_id: req.agent_id,
        tool_name: req.tool_name.clone(),
        description: if req.description.is_empty() {
            format!("Manual approval request for {}", req.tool_name)
        } else {
            req.description
        },
        action_summary: if req.action_summary.is_empty() {
            req.tool_name.clone()
        } else {
            req.action_summary
        },
        risk_level: RiskLevel::High,
        requested_at: chrono::Utc::now(),
        timeout_secs: policy.timeout_secs,
    };

    // Spawn the request in the background (it will block until resolved or timed out)
    let kernel = Arc::clone(&state.kernel);
    tokio::spawn(async move {
        kernel.approval_manager.request_approval(approval_req).await;
    });

    (
        StatusCode::CREATED,
        Json(serde_json::json!({"id": id.to_string(), "status": "pending"})),
    )
}

/// POST /api/approvals/{id}/approve — Approve a pending request.
pub async fn approve_request(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let uuid = match uuid::Uuid::parse_str(&id) {
        Ok(u) => u,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Invalid approval ID"})),
            );
        }
    };

    match state.kernel.approval_manager.resolve(
        uuid,
        skipper_types::approval::ApprovalDecision::Approved,
        Some("api".to_string()),
    ) {
        Ok(resp) => (
            StatusCode::OK,
            Json(
                serde_json::json!({"id": id, "status": "approved", "decided_at": resp.decided_at.to_rfc3339()}),
            ),
        ),
        Err(e) => (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": e}))),
    }
}

/// POST /api/approvals/{id}/reject — Reject a pending request.
pub async fn reject_request(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let uuid = match uuid::Uuid::parse_str(&id) {
        Ok(u) => u,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Invalid approval ID"})),
            );
        }
    };

    match state.kernel.approval_manager.resolve(
        uuid,
        skipper_types::approval::ApprovalDecision::Denied,
        Some("api".to_string()),
    ) {
        Ok(resp) => (
            StatusCode::OK,
            Json(
                serde_json::json!({"id": id, "status": "rejected", "decided_at": resp.decided_at.to_rfc3339()}),
            ),
        ),
        Err(e) => (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": e}))),
    }
}

// ---------------------------------------------------------------------------
// Config Reload endpoint
// ---------------------------------------------------------------------------

/// POST /api/config/reload — Reload configuration from disk and apply hot-reloadable changes.
///
/// Reads the config file, diffs against current config, validates the new config,
/// and applies hot-reloadable actions (approval policy, cron limits, etc.).
/// Returns the reload plan showing what changed and what was applied.
pub async fn config_reload(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    // SECURITY: Record config reload in audit trail
    state.kernel.audit_log.record(
        "system",
        skipper_runtime::audit::AuditAction::ConfigChange,
        "config reload requested via API",
        "pending",
    );
    match state.kernel.reload_config() {
        Ok(plan) => {
            let status = if plan.restart_required {
                "partial"
            } else if plan.has_changes() {
                "applied"
            } else {
                "no_changes"
            };

            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "status": status,
                    "restart_required": plan.restart_required,
                    "restart_reasons": plan.restart_reasons,
                    "hot_actions_applied": plan.hot_actions.iter().map(|a| format!("{a:?}")).collect::<Vec<_>>(),
                    "noop_changes": plan.noop_changes,
                })),
            )
        }
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"status": "error", "error": e})),
        ),
    }
}

// ---------------------------------------------------------------------------
// Config Schema endpoint
// ---------------------------------------------------------------------------

/// GET /api/config/schema — Return a simplified JSON description of the config structure.
pub async fn config_schema() -> impl IntoResponse {
    Json(serde_json::json!({
        "sections": {
            "api": {
                "fields": {
                    "api_listen": "string",
                    "api_key": "string",
                    "log_level": "string"
                }
            },
            "default_model": {
                "fields": {
                    "provider": "string",
                    "model": "string",
                    "api_key_env": "string",
                    "base_url": "string"
                }
            },
            "memory": {
                "fields": {
                    "decay_rate": "number",
                    "vector_dims": "number"
                }
            },
            "web": {
                "fields": {
                    "provider": "string",
                    "timeout_secs": "number",
                    "max_results": "number"
                }
            },
            "browser": {
                "fields": {
                    "headless": "boolean",
                    "timeout_secs": "number",
                    "executable_path": "string"
                }
            },
            "network": {
                "fields": {
                    "enabled": "boolean",
                    "listen_addr": "string",
                    "shared_secret": "string"
                }
            },
            "extensions": {
                "fields": {
                    "auto_connect": "boolean",
                    "health_check_interval_secs": "number"
                }
            },
            "vault": {
                "fields": {
                    "path": "string"
                }
            },
            "a2a": {
                "fields": {
                    "enabled": "boolean",
                    "name": "string",
                    "description": "string",
                    "url": "string"
                }
            },
            "channels": {
                "fields": {
                    "telegram": "object",
                    "discord": "object",
                    "slack": "object",
                    "whatsapp": "object"
                }
            }
        }
    }))
}

// ---------------------------------------------------------------------------
// Config Set endpoint
// ---------------------------------------------------------------------------

/// POST /api/config/set — Set a single config value and persist to config.toml.
///
/// Accepts JSON `{ "path": "section.key", "value": "..." }`.
/// Writes the value to the TOML config file and triggers a reload.
pub async fn config_set(
    State(state): State<Arc<AppState>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let path = match body.get("path").and_then(|v| v.as_str()) {
        Some(p) => p.to_string(),
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"status": "error", "error": "missing 'path' field"})),
            );
        }
    };
    let value = match body.get("value") {
        Some(v) => v.clone(),
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"status": "error", "error": "missing 'value' field"})),
            );
        }
    };

    let config_path = state.kernel.config.home_dir.join("config.toml");

    // Read existing config as a TOML table, or start fresh
    let mut table: toml::value::Table = if config_path.exists() {
        match std::fs::read_to_string(&config_path) {
            Ok(content) => toml::from_str(&content).unwrap_or_default(),
            Err(_) => toml::value::Table::new(),
        }
    } else {
        toml::value::Table::new()
    };

    // Convert JSON value to TOML value
    let toml_val = json_to_toml_value(&value);

    // Parse "section.key" path and set value
    let parts: Vec<&str> = path.split('.').collect();
    match parts.len() {
        1 => {
            table.insert(parts[0].to_string(), toml_val);
        }
        2 => {
            let section = table
                .entry(parts[0].to_string())
                .or_insert_with(|| toml::Value::Table(toml::value::Table::new()));
            if let toml::Value::Table(ref mut t) = section {
                t.insert(parts[1].to_string(), toml_val);
            }
        }
        3 => {
            let section = table
                .entry(parts[0].to_string())
                .or_insert_with(|| toml::Value::Table(toml::value::Table::new()));
            if let toml::Value::Table(ref mut t) = section {
                let sub = t
                    .entry(parts[1].to_string())
                    .or_insert_with(|| toml::Value::Table(toml::value::Table::new()));
                if let toml::Value::Table(ref mut t2) = sub {
                    t2.insert(parts[2].to_string(), toml_val);
                }
            }
        }
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(
                    serde_json::json!({"status": "error", "error": "path too deep (max 3 levels)"}),
                ),
            );
        }
    }

    // Write back
    let toml_string = match toml::to_string_pretty(&table) {
        Ok(s) => s,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(
                    serde_json::json!({"status": "error", "error": format!("serialize failed: {e}")}),
                ),
            );
        }
    };
    if let Err(e) = std::fs::write(&config_path, &toml_string) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"status": "error", "error": format!("write failed: {e}")})),
        );
    }

    // Trigger reload
    let reload_status = match state.kernel.reload_config() {
        Ok(plan) => {
            if plan.restart_required {
                "applied_partial"
            } else {
                "applied"
            }
        }
        Err(_) => "saved_reload_failed",
    };

    state.kernel.audit_log.record(
        "system",
        skipper_runtime::audit::AuditAction::ConfigChange,
        format!("config set: {path}"),
        "completed",
    );

    (
        StatusCode::OK,
        Json(serde_json::json!({"status": reload_status, "path": path})),
    )
}

/// Convert a serde_json::Value to a toml::Value.
fn json_to_toml_value(value: &serde_json::Value) -> toml::Value {
    match value {
        serde_json::Value::String(s) => toml::Value::String(s.clone()),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                toml::Value::Integer(i)
            } else if let Some(f) = n.as_f64() {
                toml::Value::Float(f)
            } else {
                toml::Value::String(n.to_string())
            }
        }
        serde_json::Value::Bool(b) => toml::Value::Boolean(*b),
        _ => toml::Value::String(value.to_string()),
    }
}

// ---------------------------------------------------------------------------
// Delivery tracking endpoints
// ---------------------------------------------------------------------------

/// GET /api/agents/:id/deliveries — List recent delivery receipts for an agent.
pub async fn get_agent_deliveries(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            // Try name lookup
            match state.kernel.registry.find_by_name(&id) {
                Some(entry) => entry.id,
                None => {
                    return (
                        StatusCode::NOT_FOUND,
                        Json(serde_json::json!({"error": "Agent not found"})),
                    );
                }
            }
        }
    };

    let limit = params
        .get("limit")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(50)
        .min(500);

    let receipts = state.kernel.delivery_tracker.get_receipts(agent_id, limit);
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "agent_id": agent_id.to_string(),
            "count": receipts.len(),
            "receipts": receipts,
        })),
    )
}

// ---------------------------------------------------------------------------
// Webhook trigger endpoints
// ---------------------------------------------------------------------------

/// POST /hooks/wake — Inject a system event via webhook trigger.
///
/// Publishes a custom event through the kernel's event system, which can
/// trigger proactive agents that subscribe to the event type.
pub async fn webhook_wake(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    Json(body): Json<skipper_types::webhook::WakePayload>,
) -> impl IntoResponse {
    // Check if webhook triggers are enabled
    let wh_config = match &state.kernel.config.webhook_triggers {
        Some(c) if c.enabled => c,
        _ => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "Webhook triggers not enabled"})),
            );
        }
    };

    // Validate bearer token (constant-time comparison)
    if !validate_webhook_token(&headers, &wh_config.token_env) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"error": "Invalid or missing token"})),
        );
    }

    // Validate payload
    if let Err(e) = body.validate() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": e})),
        );
    }

    // Publish through the kernel's publish_event (KernelHandle trait), which
    // goes through the full event processing pipeline including trigger evaluation.
    let event_payload = serde_json::json!({
        "source": "webhook",
        "mode": body.mode,
        "text": body.text,
    });
    if let Err(e) =
        KernelHandle::publish_event(state.kernel.as_ref(), "webhook.wake", event_payload).await
    {
        tracing::warn!("Webhook wake event publish failed: {e}");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Event publish failed: {e}")})),
        );
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({"status": "accepted", "mode": body.mode})),
    )
}

/// POST /hooks/agent — Run an isolated agent turn via webhook.
///
/// Sends a message directly to the specified agent and returns the response.
/// This enables external systems (CI/CD, Slack, etc.) to trigger agent work.
pub async fn webhook_agent(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    Json(body): Json<skipper_types::webhook::AgentHookPayload>,
) -> impl IntoResponse {
    // Check if webhook triggers are enabled
    let wh_config = match &state.kernel.config.webhook_triggers {
        Some(c) if c.enabled => c,
        _ => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "Webhook triggers not enabled"})),
            );
        }
    };

    // Validate bearer token
    if !validate_webhook_token(&headers, &wh_config.token_env) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"error": "Invalid or missing token"})),
        );
    }

    // Validate payload
    if let Err(e) = body.validate() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": e})),
        );
    }

    // Resolve the agent by name or ID (if not specified, use the first running agent)
    let agent_id: AgentId = match &body.agent {
        Some(agent_ref) => match agent_ref.parse() {
            Ok(id) => id,
            Err(_) => {
                // Try name lookup
                match state.kernel.registry.find_by_name(agent_ref) {
                    Some(entry) => entry.id,
                    None => {
                        return (
                            StatusCode::NOT_FOUND,
                            Json(
                                serde_json::json!({"error": format!("Agent not found: {}", agent_ref)}),
                            ),
                        );
                    }
                }
            }
        },
        None => {
            // No agent specified — use the first available agent
            match state.kernel.registry.list().first() {
                Some(entry) => entry.id,
                None => {
                    return (
                        StatusCode::NOT_FOUND,
                        Json(serde_json::json!({"error": "No agents available"})),
                    );
                }
            }
        }
    };

    // Actually send the message to the agent and get the response
    match state.kernel.send_message(agent_id, &body.message).await {
        Ok(result) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "completed",
                "agent_id": agent_id.to_string(),
                "response": result.response,
                "usage": {
                    "input_tokens": result.total_usage.input_tokens,
                    "output_tokens": result.total_usage.output_tokens,
                },
            })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Agent execution failed: {e}")})),
        ),
    }
}

// ─── Agent Bindings API ────────────────────────────────────────────────

/// GET /api/bindings — List all agent bindings.
pub async fn list_bindings(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let bindings = state.kernel.list_bindings();
    (
        StatusCode::OK,
        Json(serde_json::json!({ "bindings": bindings })),
    )
}

/// POST /api/bindings — Add a new agent binding.
pub async fn add_binding(
    State(state): State<Arc<AppState>>,
    Json(binding): Json<skipper_types::config::AgentBinding>,
) -> impl IntoResponse {
    // Validate agent exists
    let agents = state.kernel.registry.list();
    let agent_exists = agents.iter().any(|e| e.name == binding.agent)
        || binding.agent.parse::<uuid::Uuid>().is_ok();
    if !agent_exists {
        tracing::warn!(agent = %binding.agent, "Binding references unknown agent");
    }

    state.kernel.add_binding(binding);
    (
        StatusCode::CREATED,
        Json(serde_json::json!({ "status": "created" })),
    )
}

/// DELETE /api/bindings/:index — Remove a binding by index.
pub async fn remove_binding(
    State(state): State<Arc<AppState>>,
    Path(index): Path<usize>,
) -> impl IntoResponse {
    match state.kernel.remove_binding(index) {
        Some(_) => (
            StatusCode::OK,
            Json(serde_json::json!({ "status": "removed" })),
        ),
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "Binding index out of range" })),
        ),
    }
}

// ─── Device Pairing endpoints ───────────────────────────────────────────

/// POST /api/pairing/request — Create a new pairing request (returns token + QR URI).
pub async fn pairing_request(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    if !state.kernel.config.pairing.enabled {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "Pairing not enabled"})),
        )
            .into_response();
    }
    match state.kernel.pairing.create_pairing_request() {
        Ok(req) => {
            let qr_uri = format!("skipper://pair?token={}", req.token);
            Json(serde_json::json!({
                "token": req.token,
                "qr_uri": qr_uri,
                "expires_at": req.expires_at.to_rfc3339(),
            }))
            .into_response()
        }
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": e})),
        )
            .into_response(),
    }
}

/// POST /api/pairing/complete — Complete pairing with token + device info.
pub async fn pairing_complete(
    State(state): State<Arc<AppState>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    if !state.kernel.config.pairing.enabled {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "Pairing not enabled"})),
        )
            .into_response();
    }
    let token = body.get("token").and_then(|v| v.as_str()).unwrap_or("");
    let display_name = body
        .get("display_name")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let platform = body
        .get("platform")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let push_token = body
        .get("push_token")
        .and_then(|v| v.as_str())
        .map(String::from);
    let device_info = skipper_kernel::pairing::PairedDevice {
        device_id: uuid::Uuid::new_v4().to_string(),
        display_name: display_name.to_string(),
        platform: platform.to_string(),
        paired_at: chrono::Utc::now(),
        last_seen: chrono::Utc::now(),
        push_token,
    };
    match state.kernel.pairing.complete_pairing(token, device_info) {
        Ok(device) => Json(serde_json::json!({
            "device_id": device.device_id,
            "display_name": device.display_name,
            "platform": device.platform,
            "paired_at": device.paired_at.to_rfc3339(),
        }))
        .into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": e})),
        )
            .into_response(),
    }
}

/// GET /api/pairing/devices — List paired devices.
pub async fn pairing_devices(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    if !state.kernel.config.pairing.enabled {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "Pairing not enabled"})),
        )
            .into_response();
    }
    let devices: Vec<_> = state
        .kernel
        .pairing
        .list_devices()
        .into_iter()
        .map(|d| {
            serde_json::json!({
                "device_id": d.device_id,
                "display_name": d.display_name,
                "platform": d.platform,
                "paired_at": d.paired_at.to_rfc3339(),
                "last_seen": d.last_seen.to_rfc3339(),
            })
        })
        .collect();
    Json(serde_json::json!({"devices": devices})).into_response()
}

/// DELETE /api/pairing/devices/{id} — Remove a paired device.
pub async fn pairing_remove_device(
    State(state): State<Arc<AppState>>,
    Path(device_id): Path<String>,
) -> impl IntoResponse {
    if !state.kernel.config.pairing.enabled {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "Pairing not enabled"})),
        )
            .into_response();
    }
    match state.kernel.pairing.remove_device(&device_id) {
        Ok(()) => Json(serde_json::json!({"ok": true})).into_response(),
        Err(e) => (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": e}))).into_response(),
    }
}

/// POST /api/pairing/notify — Push a notification to all paired devices.
pub async fn pairing_notify(
    State(state): State<Arc<AppState>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    if !state.kernel.config.pairing.enabled {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "Pairing not enabled"})),
        )
            .into_response();
    }
    let title = body
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("Skipper");
    let message = body.get("message").and_then(|v| v.as_str()).unwrap_or("");
    if message.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "message is required"})),
        )
            .into_response();
    }
    state.kernel.pairing.notify_devices(title, message).await;
    Json(serde_json::json!({"ok": true, "notified": state.kernel.pairing.list_devices().len()}))
        .into_response()
}

/// GET /api/commands — List available chat commands (for dynamic slash menu).
pub async fn list_commands(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let mut commands = vec![
        serde_json::json!({"cmd": "/help", "desc": "Show available commands"}),
        serde_json::json!({"cmd": "/new", "desc": "Reset session (clear history)"}),
        serde_json::json!({"cmd": "/compact", "desc": "Trigger LLM session compaction"}),
        serde_json::json!({"cmd": "/model", "desc": "Show or switch model (/model [name])"}),
        serde_json::json!({"cmd": "/stop", "desc": "Cancel current agent run"}),
        serde_json::json!({"cmd": "/usage", "desc": "Show session token usage & cost"}),
        serde_json::json!({"cmd": "/think", "desc": "Toggle extended thinking (/think [on|off|stream])"}),
        serde_json::json!({"cmd": "/context", "desc": "Show context window usage & pressure"}),
        serde_json::json!({"cmd": "/verbose", "desc": "Cycle tool detail level (/verbose [off|on|full])"}),
        serde_json::json!({"cmd": "/queue", "desc": "Check if agent is processing"}),
        serde_json::json!({"cmd": "/status", "desc": "Show system status"}),
        serde_json::json!({"cmd": "/clear", "desc": "Clear chat display"}),
        serde_json::json!({"cmd": "/exit", "desc": "Disconnect from agent"}),
    ];

    // Add skill-registered tool names as potential commands
    if let Ok(registry) = state.kernel.skill_registry.read() {
        for skill in registry.list() {
            let desc: String = skill.manifest.skill.description.chars().take(80).collect();
            commands.push(serde_json::json!({
                "cmd": format!("/{}", skill.manifest.skill.name),
                "desc": if desc.is_empty() { format!("Skill: {}", skill.manifest.skill.name) } else { desc },
                "source": "skill",
            }));
        }
    }

    Json(serde_json::json!({"commands": commands}))
}

/// SECURITY: Validate webhook bearer token using constant-time comparison.
fn validate_webhook_token(headers: &axum::http::HeaderMap, token_env: &str) -> bool {
    let expected = match std::env::var(token_env) {
        Ok(t) if t.len() >= 32 => t,
        _ => return false,
    };

    let provided = match headers.get("authorization") {
        Some(v) => match v.to_str() {
            Ok(s) if s.starts_with("Bearer ") => &s[7..],
            _ => return false,
        },
        None => return false,
    };

    use subtle::ConstantTimeEq;
    if provided.len() != expected.len() {
        return false;
    }
    provided.as_bytes().ct_eq(expected.as_bytes()).into()
}

// ══════════════════════════════════════════════════════════════════════
// GitHub Copilot OAuth Device Flow
// ══════════════════════════════════════════════════════════════════════

/// State for an in-progress device flow.
struct CopilotFlowState {
    device_code: String,
    interval: u64,
    expires_at: Instant,
}

/// Active device flows, keyed by poll_id. Auto-expire after the flow's TTL.
static COPILOT_FLOWS: LazyLock<DashMap<String, CopilotFlowState>> = LazyLock::new(DashMap::new);

/// POST /api/providers/github-copilot/oauth/start
///
/// Initiates a GitHub device flow for Copilot authentication.
/// Returns a user code and verification URI that the user visits in their browser.
pub async fn copilot_oauth_start() -> impl IntoResponse {
    // Clean up expired flows first
    COPILOT_FLOWS.retain(|_, state| state.expires_at > Instant::now());

    match skipper_runtime::copilot_oauth::start_device_flow().await {
        Ok(resp) => {
            let poll_id = uuid::Uuid::new_v4().to_string();

            COPILOT_FLOWS.insert(
                poll_id.clone(),
                CopilotFlowState {
                    device_code: resp.device_code,
                    interval: resp.interval,
                    expires_at: Instant::now()
                        + std::time::Duration::from_secs(resp.expires_in),
                },
            );

            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "user_code": resp.user_code,
                    "verification_uri": resp.verification_uri,
                    "poll_id": poll_id,
                    "expires_in": resp.expires_in,
                    "interval": resp.interval,
                })),
            )
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e })),
        ),
    }
}

/// GET /api/providers/github-copilot/oauth/poll/{poll_id}
///
/// Poll the status of a GitHub device flow.
/// Returns `pending`, `complete`, `expired`, `denied`, or `error`.
/// On `complete`, saves the token to secrets.env and sets GITHUB_TOKEN.
pub async fn copilot_oauth_poll(
    State(state): State<Arc<AppState>>,
    Path(poll_id): Path<String>,
) -> impl IntoResponse {
    let flow = match COPILOT_FLOWS.get(&poll_id) {
        Some(f) => f,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"status": "not_found", "error": "Unknown poll_id"})),
            )
        }
    };

    if flow.expires_at <= Instant::now() {
        drop(flow);
        COPILOT_FLOWS.remove(&poll_id);
        return (
            StatusCode::OK,
            Json(serde_json::json!({"status": "expired"})),
        );
    }

    let device_code = flow.device_code.clone();
    drop(flow);

    match skipper_runtime::copilot_oauth::poll_device_flow(&device_code).await {
        skipper_runtime::copilot_oauth::DeviceFlowStatus::Pending => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "pending"})),
        ),
        skipper_runtime::copilot_oauth::DeviceFlowStatus::Complete { access_token } => {
            // Save to secrets.env
            let secrets_path = state.kernel.config.home_dir.join("secrets.env");
            if let Err(e) = write_secret_env(&secrets_path, "GITHUB_TOKEN", &access_token) {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({"status": "error", "error": format!("Failed to save token: {e}")})),
                );
            }

            // Set in current process
            std::env::set_var("GITHUB_TOKEN", access_token.as_str());

            // Refresh auth detection
            state
                .kernel
                .model_catalog
                .write()
                .unwrap_or_else(|e| e.into_inner())
                .detect_auth();

            // Clean up flow state
            COPILOT_FLOWS.remove(&poll_id);

            (
                StatusCode::OK,
                Json(serde_json::json!({"status": "complete"})),
            )
        }
        skipper_runtime::copilot_oauth::DeviceFlowStatus::SlowDown { new_interval } => {
            // Update interval
            if let Some(mut f) = COPILOT_FLOWS.get_mut(&poll_id) {
                f.interval = new_interval;
            }
            (
                StatusCode::OK,
                Json(serde_json::json!({"status": "pending", "interval": new_interval})),
            )
        }
        skipper_runtime::copilot_oauth::DeviceFlowStatus::Expired => {
            COPILOT_FLOWS.remove(&poll_id);
            (
                StatusCode::OK,
                Json(serde_json::json!({"status": "expired"})),
            )
        }
        skipper_runtime::copilot_oauth::DeviceFlowStatus::AccessDenied => {
            COPILOT_FLOWS.remove(&poll_id);
            (
                StatusCode::OK,
                Json(serde_json::json!({"status": "denied"})),
            )
        }
        skipper_runtime::copilot_oauth::DeviceFlowStatus::Error(e) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "error", "error": e})),
        ),
    }
}
