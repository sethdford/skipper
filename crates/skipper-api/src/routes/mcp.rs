//! Model Context Protocol (MCP) server endpoint for Claude Code integration.
//!
//! This module implements a JSON-RPC 2.0 MCP endpoint that allows Claude Code
//! to interact with Skipper natively. The endpoint exposes tools for:
//! - Spawning and managing agents
//! - Listing agents
//! - Sending messages to agents
//! - Managing pipelines and fleets
//!
//! Reference: https://spec.modelcontextprotocol.io/

use super::*;
use serde_json::{json, Value};
use std::str::FromStr;

// ---------------------------------------------------------------------------
// MCP Tool Definitions
// ---------------------------------------------------------------------------

/// Defines a single MCP tool that Claude Code can call.
#[derive(Debug, Clone, serde::Serialize)]
struct McpTool {
    name: String,
    description: String,
    #[serde(rename = "inputSchema")]
    input_schema: Value,
}

/// Generate all Skipper MCP tools.
fn get_mcp_tools() -> Vec<McpTool> {
    vec![
        McpTool {
            name: "skipper_spawn_agent".to_string(),
            description: "Spawn a new Skipper agent with a given manifest".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "Agent name (must be unique)"
                    },
                    "manifest_toml": {
                        "type": "string",
                        "description": "TOML manifest defining agent configuration"
                    },
                    "model": {
                        "type": "string",
                        "description": "LLM model (e.g., 'groq/llama-3.3-70b', 'openai/gpt-4o')"
                    }
                },
                "required": ["name", "manifest_toml"]
            }),
        },
        McpTool {
            name: "skipper_list_agents".to_string(),
            description: "List all Skipper agents with their status".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
        },
        McpTool {
            name: "skipper_send_message".to_string(),
            description: "Send a message to a Skipper agent and get response".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "agent_id": {
                        "type": "string",
                        "description": "Agent ID to send message to"
                    },
                    "message": {
                        "type": "string",
                        "description": "Message content"
                    }
                },
                "required": ["agent_id", "message"]
            }),
        },
        McpTool {
            name: "skipper_pipeline_status".to_string(),
            description: "Get the status of a Skipper pipeline".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "pipeline_id": {
                        "type": "string",
                        "description": "Pipeline ID"
                    }
                },
                "required": ["pipeline_id"]
            }),
        },
        McpTool {
            name: "skipper_fleet_status".to_string(),
            description: "Get the status of the Skipper fleet (all pipelines)".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
        },
    ]
}

// ---------------------------------------------------------------------------
// MCP Request/Response Types (JSON-RPC 2.0)
// ---------------------------------------------------------------------------

/// JSON-RPC 2.0 request.
#[derive(Debug, serde::Deserialize)]
pub struct JsonRpcRequest {
    jsonrpc: String,
    id: serde_json::Value,
    method: String,
    #[serde(default)]
    params: serde_json::Value,
}

/// JSON-RPC 2.0 response.
#[derive(Debug, serde::Serialize)]
struct JsonRpcResponse {
    jsonrpc: String,
    id: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

/// JSON-RPC 2.0 error object.
#[derive(Debug, serde::Serialize)]
struct JsonRpcError {
    code: i32,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<Value>,
}

// ---------------------------------------------------------------------------
// MCP Protocol Handlers
// ---------------------------------------------------------------------------

/// POST /mcp — MCP JSON-RPC 2.0 endpoint.
///
/// Implements the Model Context Protocol for Claude Code integration.
/// Handles initialization, tool listing, and tool calling.
pub async fn mcp_handler(
    State(state): State<Arc<AppState>>,
    Json(req): Json<JsonRpcRequest>,
) -> impl IntoResponse {
    // Validate JSON-RPC version
    if req.jsonrpc != "2.0" {
        return Json(JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id: req.id,
            result: None,
            error: Some(JsonRpcError {
                code: -32600,
                message: "Invalid JSON-RPC version".to_string(),
                data: None,
            }),
        });
    }

    // Route the request to the appropriate handler
    let result = match req.method.as_str() {
        "initialize" => handle_initialize(),
        "tools/list" => handle_tools_list(),
        "tools/call" => handle_tools_call(&req.params, state).await,
        _ => {
            return Json(JsonRpcResponse {
                jsonrpc: "2.0".to_string(),
                id: req.id,
                result: None,
                error: Some(JsonRpcError {
                    code: -32601,
                    message: format!("Method not found: {}", req.method),
                    data: None,
                }),
            });
        }
    };

    match result {
        Ok(result) => Json(JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id: req.id,
            result: Some(result),
            error: None,
        }),
        Err(err) => Json(JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id: req.id,
            result: None,
            error: Some(err),
        }),
    }
}

/// Handle the MCP initialize request.
fn handle_initialize() -> Result<Value, JsonRpcError> {
    Ok(json!({
        "protocolVersion": "2024-11-05",
        "capabilities": {
            "tools": {}
        },
        "serverInfo": {
            "name": "Skipper MCP Server",
            "version": "1.0.0"
        }
    }))
}

/// Handle the MCP tools/list request.
fn handle_tools_list() -> Result<Value, JsonRpcError> {
    let tools = get_mcp_tools();
    Ok(json!({
        "tools": tools.iter().map(|t| json!({
            "name": t.name,
            "description": t.description,
            "inputSchema": t.input_schema,
        })).collect::<Vec<_>>()
    }))
}

/// Handle the MCP tools/call request.
async fn handle_tools_call(
    params: &Value,
    state: Arc<AppState>,
) -> Result<Value, JsonRpcError> {
    let tool_name = params
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or(JsonRpcError {
            code: -32602,
            message: "Missing 'name' parameter".to_string(),
            data: None,
        })?;

    let arguments = params.get("arguments").cloned().unwrap_or(json!({}));

    match tool_name {
        "skipper_spawn_agent" => call_spawn_agent(&arguments, state).await,
        "skipper_list_agents" => call_list_agents(state).await,
        "skipper_send_message" => call_send_message(&arguments, state).await,
        "skipper_pipeline_status" => call_pipeline_status(&arguments, state).await,
        "skipper_fleet_status" => call_fleet_status(state).await,
        _ => Err(JsonRpcError {
            code: -32602,
            message: format!("Unknown tool: {}", tool_name),
            data: None,
        }),
    }
}

// ---------------------------------------------------------------------------
// Tool Implementation Handlers
// ---------------------------------------------------------------------------

/// Call skipper_spawn_agent.
async fn call_spawn_agent(args: &Value, state: Arc<AppState>) -> Result<Value, JsonRpcError> {
    let manifest_toml = args
        .get("manifest_toml")
        .and_then(|v| v.as_str())
        .ok_or(JsonRpcError {
            code: -32602,
            message: "Missing 'manifest_toml' argument".to_string(),
            data: None,
        })?;

    let name = args
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or(JsonRpcError {
            code: -32602,
            message: "Missing 'name' argument".to_string(),
            data: None,
        })?;

    // Parse manifest
    let manifest: skipper_types::agent::AgentManifest =
        toml::from_str(manifest_toml).map_err(|e| JsonRpcError {
            code: -32602,
            message: format!("Invalid TOML manifest: {}", e),
            data: None,
        })?;

    // Spawn the agent
    match state.kernel.spawn_agent(manifest) {
        Ok(agent_id) => Ok(json!({
            "agent_id": agent_id.to_string(),
            "name": name,
            "status": "spawned"
        })),
        Err(e) => Err(JsonRpcError {
            code: -32603,
            message: format!("Failed to spawn agent: {}", e),
            data: None,
        }),
    }
}

/// Call skipper_list_agents.
async fn call_list_agents(state: Arc<AppState>) -> Result<Value, JsonRpcError> {
    let agents: Vec<Value> = state
        .kernel
        .registry
        .list()
        .into_iter()
        .map(|e| {
            json!({
                "id": e.id.to_string(),
                "name": e.name,
                "state": format!("{:?}", e.state),
                "model": {
                    "provider": e.manifest.model.provider,
                    "name": e.manifest.model.model,
                },
                "created_at": e.created_at.to_rfc3339(),
            })
        })
        .collect();

    Ok(json!({
        "agents": agents,
        "count": agents.len()
    }))
}

/// Call skipper_send_message.
async fn call_send_message(args: &Value, state: Arc<AppState>) -> Result<Value, JsonRpcError> {
    let agent_id_str = args
        .get("agent_id")
        .and_then(|v| v.as_str())
        .ok_or(JsonRpcError {
            code: -32602,
            message: "Missing 'agent_id' argument".to_string(),
            data: None,
        })?;

    let message = args
        .get("message")
        .and_then(|v| v.as_str())
        .ok_or(JsonRpcError {
            code: -32602,
            message: "Missing 'message' argument".to_string(),
            data: None,
        })?;

    // Parse agent ID
    let agent_id = skipper_types::agent::AgentId::from_str(agent_id_str).map_err(|e| {
        JsonRpcError {
            code: -32602,
            message: format!("Invalid agent ID: {}", e),
            data: None,
        }
    })?;

    // Send message
    match state.kernel.send_message(agent_id, message).await {
        Ok(response) => {
            Ok(json!({
                "response": response.response,
                "input_tokens": response.total_usage.input_tokens,
                "output_tokens": response.total_usage.output_tokens,
                "cost_usd": response.cost_usd,
            }))
        }
        Err(e) => Err(JsonRpcError {
            code: -32603,
            message: format!("Failed to send message: {}", e),
            data: None,
        }),
    }
}

/// Call skipper_pipeline_status.
async fn call_pipeline_status(args: &Value, state: Arc<AppState>) -> Result<Value, JsonRpcError> {
    let pipeline_id = args
        .get("pipeline_id")
        .and_then(|v| v.as_str())
        .ok_or(JsonRpcError {
            code: -32602,
            message: "Missing 'pipeline_id' argument".to_string(),
            data: None,
        })?;

    // Delegate to the existing shipwright pipeline status tool
    let input = json!({
        "pipeline_id": pipeline_id,
    });

    match skipper_shipwright::tools::dispatch("shipwright_pipeline_status", &input, &state.shipwright)
        .await
    {
        Ok(result) => {
            // Parse the result
            match serde_json::from_str::<Value>(&result) {
                Ok(status_data) => Ok(json!({
                    "pipeline_id": pipeline_id,
                    "status": status_data.get("status").unwrap_or(&json!("unknown")),
                    "current_stage": status_data.get("current_stage").unwrap_or(&json!("unknown")),
                    "progress": status_data.get("progress").unwrap_or(&json!(0)),
                })),
                Err(_) => Ok(json!({
                    "pipeline_id": pipeline_id,
                    "status": "unknown",
                })),
            }
        }
        Err(e) => Err(JsonRpcError {
            code: -32603,
            message: format!("Failed to get pipeline status: {}", e),
            data: None,
        }),
    }
}

/// Call skipper_fleet_status.
async fn call_fleet_status(state: Arc<AppState>) -> Result<Value, JsonRpcError> {
    // Delegate to the existing shipwright fleet status tool
    match skipper_shipwright::tools::dispatch("shipwright_fleet_status", &json!({}), &state.shipwright)
        .await
    {
        Ok(result) => {
            // Parse the result
            match serde_json::from_str::<Value>(&result) {
                Ok(fleet_data) => Ok(json!({
                    "status": "operational",
                    "active_pipelines": fleet_data.get("active_pipelines").unwrap_or(&json!(0)),
                    "completed_pipelines": fleet_data.get("completed_pipelines").unwrap_or(&json!(0)),
                    "failed_pipelines": fleet_data.get("failed_pipelines").unwrap_or(&json!(0)),
                    "total_cost_usd": fleet_data.get("total_cost_usd").unwrap_or(&json!(0.0)),
                })),
                Err(_) => Ok(json!({
                    "status": "operational",
                    "active_pipelines": 0,
                    "completed_pipelines": 0,
                    "failed_pipelines": 0,
                    "total_cost_usd": 0.0,
                })),
            }
        }
        Err(e) => Err(JsonRpcError {
            code: -32603,
            message: format!("Failed to get fleet status: {}", e),
            data: None,
        }),
    }
}
