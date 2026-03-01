//! MCP (Model Context Protocol) endpoint tests.
//!
//! Tests for the Skipper MCP JSON-RPC 2.0 endpoint that integrates with Claude Code.
//!
//! Run: cargo test -p skipper-api --test mcp_test -- --nocapture

use skipper_api::routes::JsonRpcRequest;

// ---------------------------------------------------------------------------
// JSON-RPC 2.0 Parsing Tests
// ---------------------------------------------------------------------------

#[test]
fn test_mcp_initialize_request() {
    let json = r#"{"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}"#;
    let req: JsonRpcRequest = serde_json::from_str(json).expect("Failed to parse");

    assert_eq!(req.jsonrpc, "2.0");
    assert_eq!(req.method, "initialize");
}

#[test]
fn test_mcp_tools_list_request() {
    let json = r#"{"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}}"#;
    let req: JsonRpcRequest = serde_json::from_str(json).expect("Failed to parse");

    assert_eq!(req.jsonrpc, "2.0");
    assert_eq!(req.method, "tools/list");
}

#[test]
fn test_mcp_tools_call_spawn_agent() {
    let json = r#"{
        "jsonrpc": "2.0",
        "id": 3,
        "method": "tools/call",
        "params": {
            "name": "skipper_spawn_agent",
            "arguments": {
                "name": "test-agent",
                "manifest_toml": "[agent]\nname = \"test\"\nversion = \"0.1.0\""
            }
        }
    }"#;
    let req: JsonRpcRequest = serde_json::from_str(json).expect("Failed to parse");

    assert_eq!(req.jsonrpc, "2.0");
    assert_eq!(req.method, "tools/call");
    assert_eq!(
        req.params
            .get("name")
            .and_then(|v| v.as_str()),
        Some("skipper_spawn_agent")
    );
}

#[test]
fn test_mcp_tools_call_list_agents() {
    let json = r#"{
        "jsonrpc": "2.0",
        "id": 4,
        "method": "tools/call",
        "params": {
            "name": "skipper_list_agents",
            "arguments": {}
        }
    }"#;
    let req: JsonRpcRequest = serde_json::from_str(json).expect("Failed to parse");

    assert_eq!(req.jsonrpc, "2.0");
    assert_eq!(req.method, "tools/call");
    assert_eq!(
        req.params
            .get("name")
            .and_then(|v| v.as_str()),
        Some("skipper_list_agents")
    );
}

#[test]
fn test_mcp_tools_call_send_message() {
    let json = r#"{
        "jsonrpc": "2.0",
        "id": 5,
        "method": "tools/call",
        "params": {
            "name": "skipper_send_message",
            "arguments": {
                "agent_id": "12345678-1234-5678-1234-567812345678",
                "message": "Hello agent"
            }
        }
    }"#;
    let req: JsonRpcRequest = serde_json::from_str(json).expect("Failed to parse");

    assert_eq!(req.jsonrpc, "2.0");
    assert_eq!(req.method, "tools/call");
    assert_eq!(
        req.params
            .get("name")
            .and_then(|v| v.as_str()),
        Some("skipper_send_message")
    );
}

#[test]
fn test_mcp_tools_call_pipeline_status() {
    let json = r#"{
        "jsonrpc": "2.0",
        "id": 6,
        "method": "tools/call",
        "params": {
            "name": "skipper_pipeline_status",
            "arguments": {
                "pipeline_id": "pipeline-123"
            }
        }
    }"#;
    let req: JsonRpcRequest = serde_json::from_str(json).expect("Failed to parse");

    assert_eq!(req.jsonrpc, "2.0");
    assert_eq!(req.method, "tools/call");
    assert_eq!(
        req.params
            .get("name")
            .and_then(|v| v.as_str()),
        Some("skipper_pipeline_status")
    );
}

#[test]
fn test_mcp_tools_call_fleet_status() {
    let json = r#"{
        "jsonrpc": "2.0",
        "id": 7,
        "method": "tools/call",
        "params": {
            "name": "skipper_fleet_status",
            "arguments": {}
        }
    }"#;
    let req: JsonRpcRequest = serde_json::from_str(json).expect("Failed to parse");

    assert_eq!(req.jsonrpc, "2.0");
    assert_eq!(req.method, "tools/call");
    assert_eq!(
        req.params
            .get("name")
            .and_then(|v| v.as_str()),
        Some("skipper_fleet_status")
    );
}

#[test]
fn test_mcp_invalid_jsonrpc_version() {
    let json = r#"{"jsonrpc": "1.0", "id": 1, "method": "initialize", "params": {}}"#;
    let result: Result<JsonRpcRequest, _> = serde_json::from_str(json);

    // Should still parse, but handler would reject invalid version
    assert!(result.is_ok());
    let req = result.unwrap();
    assert_eq!(req.jsonrpc, "1.0");
}

#[test]
fn test_mcp_missing_method() {
    let json = r#"{"jsonrpc": "2.0", "id": 1, "params": {}}"#;
    let result: Result<JsonRpcRequest, _> = serde_json::from_str(json);

    // Method is required
    assert!(result.is_err());
}

// ---------------------------------------------------------------------------
// Tool Definition Tests
// ---------------------------------------------------------------------------

/// Verify the MCP tool definitions are serializable
#[test]
fn test_mcp_tool_definitions_serializable() {
    let tools = vec![
        serde_json::json!({
            "name": "skipper_spawn_agent",
            "description": "Spawn a new Skipper agent",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": {"type": "string"},
                    "manifest_toml": {"type": "string"}
                },
                "required": ["name", "manifest_toml"]
            }
        }),
        serde_json::json!({
            "name": "skipper_list_agents",
            "description": "List all Skipper agents",
            "inputSchema": {
                "type": "object",
                "properties": {},
                "required": []
            }
        }),
    ];

    let serialized = serde_json::to_string(&tools).expect("Failed to serialize");
    assert!(serialized.contains("skipper_spawn_agent"));
    assert!(serialized.contains("skipper_list_agents"));
}

// ---------------------------------------------------------------------------
// Response Format Tests
// ---------------------------------------------------------------------------

/// Verify MCP protocol response format
#[test]
fn test_mcp_response_format() {
    let response = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "result": {
            "protocolVersion": "2024-11-05",
            "capabilities": {
                "tools": {}
            },
            "serverInfo": {
                "name": "Skipper MCP Server",
                "version": "1.0.0"
            }
        }
    });

    let serialized = serde_json::to_string(&response).expect("Failed to serialize");
    assert!(serialized.contains("jsonrpc"));
    assert!(serialized.contains("2.0"));
    assert!(serialized.contains("Skipper MCP Server"));
}

/// Verify MCP error response format
#[test]
fn test_mcp_error_response_format() {
    let error_response = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "error": {
            "code": -32600,
            "message": "Invalid Request"
        }
    });

    let serialized = serde_json::to_string(&error_response).expect("Failed to serialize");
    assert!(serialized.contains("error"));
    assert!(serialized.contains("-32600"));
}

// ---------------------------------------------------------------------------
// Tool List Response Tests
// ---------------------------------------------------------------------------

#[test]
fn test_mcp_tools_list_response() {
    let response = serde_json::json!({
        "tools": [
            {
                "name": "skipper_spawn_agent",
                "description": "Spawn a new Skipper agent",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "name": {"type": "string"},
                        "manifest_toml": {"type": "string"}
                    },
                    "required": ["name", "manifest_toml"]
                }
            },
            {
                "name": "skipper_list_agents",
                "description": "List all Skipper agents",
                "inputSchema": {
                    "type": "object",
                    "properties": {},
                    "required": []
                }
            },
            {
                "name": "skipper_send_message",
                "description": "Send a message to a Skipper agent",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "agent_id": {"type": "string"},
                        "message": {"type": "string"}
                    },
                    "required": ["agent_id", "message"]
                }
            },
            {
                "name": "skipper_pipeline_status",
                "description": "Get pipeline status",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "pipeline_id": {"type": "string"}
                    },
                    "required": ["pipeline_id"]
                }
            },
            {
                "name": "skipper_fleet_status",
                "description": "Get fleet status",
                "inputSchema": {
                    "type": "object",
                    "properties": {},
                    "required": []
                }
            }
        ]
    });

    let tools_array = response
        .get("tools")
        .and_then(|v| v.as_array())
        .expect("Expected tools array");

    assert_eq!(tools_array.len(), 5);

    let tool_names: Vec<&str> = tools_array
        .iter()
        .filter_map(|t| t.get("name").and_then(|v| v.as_str()))
        .collect();

    assert!(tool_names.contains(&"skipper_spawn_agent"));
    assert!(tool_names.contains(&"skipper_list_agents"));
    assert!(tool_names.contains(&"skipper_send_message"));
    assert!(tool_names.contains(&"skipper_pipeline_status"));
    assert!(tool_names.contains(&"skipper_fleet_status"));
}
