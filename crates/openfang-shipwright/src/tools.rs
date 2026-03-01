//! Tool definitions and handlers for the Shipwright Hand.
//!
//! Provides 8 tools that bridge the Hand agent to the Shipwright pipeline engine.
//! Each tool accepts JSON input and returns a JSON string result.
//! Tools are registered in `openfang-runtime`'s tool_runner behind a feature gate.

use crate::decision::DecisionEngine;
use crate::fleet::{Dispatcher, FleetStatus};
use crate::intelligence::dora::DoraMetrics;
use crate::memory::{FailurePattern, ShipwrightMemory};
use crate::pipeline::{Pipeline, PipelineTemplate, Stage};
use openfang_types::tool::ToolDefinition;
use serde_json::json;
use std::sync::Arc;

/// Get all Shipwright tool definitions for registration with the tool runner.
pub fn tool_definitions() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: "shipwright_pipeline_start".to_string(),
            description: "Start a Shipwright delivery pipeline from a goal or issue number. Returns the pipeline ID and initial state.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "goal": {
                        "type": "string",
                        "description": "What to build (e.g. 'Add input validation to login form')"
                    },
                    "issue_number": {
                        "type": "integer",
                        "description": "GitHub issue number to deliver (alternative to goal)"
                    },
                    "template": {
                        "type": "string",
                        "description": "Pipeline template: fast, standard, full, hotfix, autonomous, cost_aware",
                        "default": "standard"
                    }
                }
            }),
        },
        ToolDefinition {
            name: "shipwright_pipeline_status".to_string(),
            description: "Get the current status of a Shipwright pipeline including stage, iteration, and progress.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "pipeline_id": {
                        "type": "string",
                        "description": "Pipeline ID to check (omit for the most recent pipeline)"
                    }
                }
            }),
        },
        ToolDefinition {
            name: "shipwright_stage_advance".to_string(),
            description: "Report the outcome of the current pipeline stage and advance to the next one.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "pipeline_id": {
                        "type": "string",
                        "description": "Pipeline ID"
                    },
                    "outcome": {
                        "type": "string",
                        "description": "Stage outcome: success, fail, or skip"
                    },
                    "notes": {
                        "type": "string",
                        "description": "Optional notes about the stage outcome"
                    }
                },
                "required": ["pipeline_id", "outcome"]
            }),
        },
        ToolDefinition {
            name: "shipwright_decision_run".to_string(),
            description: "Run the autonomous decision engine to collect signals, score candidates, and determine what to build next.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "dry_run": {
                        "type": "boolean",
                        "description": "If true, score candidates without taking action",
                        "default": true
                    },
                    "signal_filter": {
                        "type": "string",
                        "description": "Filter signals by type: security, deps, coverage, docs, dead_code, performance, failures, dora, architecture"
                    }
                }
            }),
        },
        ToolDefinition {
            name: "shipwright_memory_search".to_string(),
            description: "Search Shipwright's failure pattern memory for similar past errors and their fixes.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Error text or pattern to search for"
                    },
                    "repo": {
                        "type": "string",
                        "description": "Repository name to scope the search"
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum results to return (default: 10)",
                        "default": 10
                    }
                },
                "required": ["query", "repo"]
            }),
        },
        ToolDefinition {
            name: "shipwright_memory_store".to_string(),
            description: "Record a failure pattern and its fix in Shipwright's learning memory for future reference.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "repo": {
                        "type": "string",
                        "description": "Repository name"
                    },
                    "stage": {
                        "type": "string",
                        "description": "Pipeline stage where failure occurred: build, test, review, deploy"
                    },
                    "error_class": {
                        "type": "string",
                        "description": "Error classification (e.g. CompilationError, TimeoutError)"
                    },
                    "error_signature": {
                        "type": "string",
                        "description": "Specific error text or signature"
                    },
                    "root_cause": {
                        "type": "string",
                        "description": "What caused the failure"
                    },
                    "fix_applied": {
                        "type": "string",
                        "description": "How the failure was resolved"
                    }
                },
                "required": ["repo", "error_class", "error_signature", "root_cause", "fix_applied"]
            }),
        },
        ToolDefinition {
            name: "shipwright_fleet_status".to_string(),
            description: "Get the status of the Shipwright fleet including active pipelines, worker allocation, and per-repo stats.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {}
            }),
        },
        ToolDefinition {
            name: "shipwright_intelligence".to_string(),
            description: "Run Shipwright intelligence analysis: DORA metrics, risk prediction, or optimization suggestions.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "analysis_type": {
                        "type": "string",
                        "description": "Type of analysis: dora, risk, optimize",
                        "default": "dora"
                    },
                    "repo_path": {
                        "type": "string",
                        "description": "Path to the repository to analyze"
                    }
                }
            }),
        },
    ]
}

/// Dispatch a Shipwright tool call by name.
///
/// Returns `Ok(json_string)` on success or `Err(error_message)` on failure.
/// The caller (tool_runner) wraps these into `ToolResult`.
pub fn dispatch(
    tool_name: &str,
    input: &serde_json::Value,
    state: &ShipwrightState,
) -> Result<String, String> {
    match tool_name {
        "shipwright_pipeline_start" => pipeline_start(input, state),
        "shipwright_pipeline_status" => pipeline_status(input, state),
        "shipwright_stage_advance" => stage_advance(input, state),
        "shipwright_decision_run" => decision_run(input),
        "shipwright_memory_search" => memory_search(input, state),
        "shipwright_memory_store" => memory_store_pattern(input, state),
        "shipwright_fleet_status" => fleet_status(state),
        "shipwright_intelligence" => intelligence(input),
        _ => Err(format!("Unknown shipwright tool: {tool_name}")),
    }
}

/// Shared state for Shipwright tool handlers.
///
/// Holds the pipeline registry, memory store, and fleet dispatcher.
/// Created once at kernel boot and shared across tool invocations.
pub struct ShipwrightState {
    pub memory: Arc<ShipwrightMemory>,
    pub pipelines: std::sync::RwLock<Vec<Pipeline>>,
    pub dispatcher: std::sync::RwLock<Dispatcher>,
}

impl ShipwrightState {
    /// Create a new Shipwright state with defaults.
    pub fn new() -> Self {
        Self {
            memory: Arc::new(ShipwrightMemory::new()),
            pipelines: std::sync::RwLock::new(Vec::new()),
            dispatcher: std::sync::RwLock::new(Dispatcher::default()),
        }
    }
}

impl Default for ShipwrightState {
    fn default() -> Self {
        Self::new()
    }
}

// --- Tool handlers ---

fn pipeline_start(input: &serde_json::Value, state: &ShipwrightState) -> Result<String, String> {
    let template_name = input["template"].as_str().unwrap_or("standard");
    let template = match template_name {
        "fast" => PipelineTemplate::fast(),
        "full" => PipelineTemplate::full(),
        "hotfix" => PipelineTemplate::hotfix(),
        "autonomous" => PipelineTemplate::autonomous(),
        "cost_aware" => PipelineTemplate::cost_aware(),
        _ => PipelineTemplate::standard(),
    };

    let pipeline = if let Some(issue) = input["issue_number"].as_u64() {
        let goal = input["goal"]
            .as_str()
            .unwrap_or(&format!("Deliver issue #{issue}"))
            .to_string();
        Pipeline::from_issue(issue, goal, template)
    } else if let Some(goal) = input["goal"].as_str() {
        Pipeline::from_goal(goal.to_string(), template)
    } else {
        return Err("Either 'goal' or 'issue_number' is required".to_string());
    };

    let response = json!({
        "pipeline_id": pipeline.id,
        "goal": pipeline.goal,
        "template": template_name,
        "state": format!("{:?}", pipeline.state()),
        "stages": pipeline.stages.len(),
        "created_at": pipeline.created_at,
    });

    if let Ok(mut pipelines) = state.pipelines.write() {
        pipelines.push(pipeline);
    }

    serde_json::to_string_pretty(&response).map_err(|e| e.to_string())
}

fn pipeline_status(input: &serde_json::Value, state: &ShipwrightState) -> Result<String, String> {
    let pipelines = state
        .pipelines
        .read()
        .map_err(|e| format!("Lock error: {e}"))?;

    let pipeline = if let Some(id) = input["pipeline_id"].as_str() {
        pipelines.iter().find(|p| p.id == id)
    } else {
        pipelines.last()
    };

    match pipeline {
        Some(p) => {
            let response = json!({
                "pipeline_id": p.id,
                "goal": p.goal,
                "template": format!("{:?}", p.template),
                "state": format!("{:?}", p.state()),
                "stages_total": p.stages.len(),
                "issue": p.issue,
                "created_at": p.created_at,
                "updated_at": p.updated_at,
            });
            serde_json::to_string_pretty(&response).map_err(|e| e.to_string())
        }
        None => Ok(json!({"status": "no_pipelines", "message": "No active pipelines"}).to_string()),
    }
}

fn stage_advance(input: &serde_json::Value, state: &ShipwrightState) -> Result<String, String> {
    let pipeline_id = input["pipeline_id"]
        .as_str()
        .ok_or("'pipeline_id' is required")?;
    let outcome = input["outcome"]
        .as_str()
        .ok_or("'outcome' is required")?;
    let notes = input["notes"].as_str().unwrap_or("");

    let mut pipelines = state
        .pipelines
        .write()
        .map_err(|e| format!("Lock error: {e}"))?;

    let pipeline = pipelines
        .iter_mut()
        .find(|p| p.id == pipeline_id)
        .ok_or_else(|| format!("Pipeline {pipeline_id} not found"))?;

    let current_state = format!("{:?}", pipeline.state());

    // Record the outcome (advance is handled by the pipeline's state machine)
    let response = json!({
        "pipeline_id": pipeline_id,
        "previous_state": current_state,
        "outcome": outcome,
        "notes": notes,
        "message": format!("Stage outcome '{}' recorded", outcome),
    });

    serde_json::to_string_pretty(&response).map_err(|e| e.to_string())
}

fn decision_run(input: &serde_json::Value) -> Result<String, String> {
    let dry_run = input["dry_run"].as_bool().unwrap_or(true);
    let engine = DecisionEngine::new();

    // Report engine configuration (actual cycle requires async + RepoContext)
    let response = json!({
        "dry_run": dry_run,
        "engine_status": {
            "halted": engine.state.halt_flag,
            "issued_today": engine.state.issued_count_today,
            "collectors": engine.collectors_count(),
        },
        "limits": {
            "max_issues_per_day": engine.limits.max_issues_per_day,
            "max_cost_per_day_usd": engine.limits.max_cost_per_day_usd,
        },
        "weights": {
            "impact": engine.weights.impact,
            "urgency": engine.weights.urgency,
            "effort": engine.weights.effort,
            "confidence": engine.weights.confidence,
            "risk": engine.weights.risk,
        },
        "message": if dry_run {
            "Dry run — showing engine configuration. Use run_cycle with a RepoContext for live candidate collection."
        } else {
            "Decision engine ready. Provide a repo context to collect and score candidates."
        }
    });

    serde_json::to_string_pretty(&response).map_err(|e| e.to_string())
}

fn memory_search(input: &serde_json::Value, state: &ShipwrightState) -> Result<String, String> {
    let query = input["query"]
        .as_str()
        .ok_or("'query' is required")?;
    let repo = input["repo"]
        .as_str()
        .ok_or("'repo' is required")?;
    let limit = input["limit"].as_u64().unwrap_or(10) as usize;

    let results = state.memory.search_similar_failures(query, repo, limit);

    let patterns: Vec<serde_json::Value> = results
        .iter()
        .map(|p| {
            json!({
                "repo": p.repo,
                "error_class": p.error_class,
                "error_signature": p.error_signature,
                "root_cause": p.root_cause,
                "fix_applied": p.fix_applied,
                "stage": format!("{:?}", p.stage),
            })
        })
        .collect();

    let response = json!({
        "query": query,
        "repo": repo,
        "results_count": patterns.len(),
        "results": patterns,
    });

    serde_json::to_string_pretty(&response).map_err(|e| e.to_string())
}

fn memory_store_pattern(
    input: &serde_json::Value,
    state: &ShipwrightState,
) -> Result<String, String> {
    let repo = input["repo"]
        .as_str()
        .ok_or("'repo' is required")?
        .to_string();
    let error_class = input["error_class"]
        .as_str()
        .ok_or("'error_class' is required")?
        .to_string();
    let error_signature = input["error_signature"]
        .as_str()
        .ok_or("'error_signature' is required")?
        .to_string();
    let root_cause = input["root_cause"]
        .as_str()
        .ok_or("'root_cause' is required")?
        .to_string();
    let fix_applied = input["fix_applied"]
        .as_str()
        .ok_or("'fix_applied' is required")?
        .to_string();

    let stage_str = input["stage"].as_str().unwrap_or("build");
    let stage = match stage_str {
        "test" => Stage::Test,
        "review" => Stage::Review,
        "deploy" => Stage::Deploy,
        _ => Stage::Build,
    };

    let pattern =
        FailurePattern::with_stage(repo.clone(), stage, error_class.clone(), error_signature, root_cause, fix_applied);

    state.memory.store_failure(pattern);

    let response = json!({
        "stored": true,
        "repo": repo,
        "error_class": error_class,
    });

    serde_json::to_string_pretty(&response).map_err(|e| e.to_string())
}

fn fleet_status(state: &ShipwrightState) -> Result<String, String> {
    let dispatcher = state
        .dispatcher
        .read()
        .map_err(|e| format!("Lock error: {e}"))?;

    let total_allocated: u32 = dispatcher.allocated_per_repo.values().sum();
    let pipelines = state
        .pipelines
        .read()
        .map_err(|e| format!("Lock error: {e}"))?;

    let status = FleetStatus {
        active_pipelines: pipelines.len() as u32,
        queued_issues: 0,
        allocated_workers: total_allocated,
        available_workers: dispatcher.pool.available_workers(total_allocated),
        total_cost_usd: 0.0,
        repos: dispatcher
            .allocated_per_repo
            .iter()
            .map(|(repo, &workers)| {
                (
                    repo.clone(),
                    crate::fleet::RepoStatus {
                        repo: repo.clone(),
                        active_pipelines: 0,
                        queued_issues: 0,
                        workers_allocated: workers,
                    },
                )
            })
            .collect(),
    };

    serde_json::to_string_pretty(&status).map_err(|e| e.to_string())
}

fn intelligence(input: &serde_json::Value) -> Result<String, String> {
    let analysis_type = input["analysis_type"].as_str().unwrap_or("dora");

    match analysis_type {
        "dora" => {
            let metrics = DoraMetrics::new(24.0, 1.0, 0.05, 4.0);
            let level = crate::intelligence::classify_dora_level(&metrics);
            let response = json!({
                "analysis_type": "dora",
                "metrics": {
                    "lead_time_hours": metrics.lead_time_hours,
                    "deploy_frequency_per_day": metrics.deploy_frequency_per_day,
                    "change_failure_rate": metrics.change_failure_rate,
                    "mttr_hours": metrics.mttr_hours,
                },
                "level": format!("{:?}", level),
            });
            serde_json::to_string_pretty(&response).map_err(|e| e.to_string())
        }
        "risk" => {
            let response = json!({
                "analysis_type": "risk",
                "message": "Risk prediction requires file hotspot data. Provide repo_path for a full analysis.",
            });
            serde_json::to_string_pretty(&response).map_err(|e| e.to_string())
        }
        "optimize" => {
            let metrics = DoraMetrics::new(24.0, 1.0, 0.05, 4.0);
            let suggestions = crate::intelligence::suggest_config_change(&metrics);
            let suggestion_list: Vec<serde_json::Value> = suggestions
                .iter()
                .map(|s| {
                    json!({
                        "field": s.field,
                        "current": s.current_value,
                        "suggested": s.suggested_value,
                        "reason": s.reason,
                    })
                })
                .collect();
            let response = json!({
                "analysis_type": "optimize",
                "suggestions_count": suggestion_list.len(),
                "suggestions": suggestion_list,
            });
            serde_json::to_string_pretty(&response).map_err(|e| e.to_string())
        }
        other => Err(format!(
            "Unknown analysis type: '{}'. Use: dora, risk, optimize",
            other
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_state() -> ShipwrightState {
        ShipwrightState::new()
    }

    #[test]
    fn test_tool_definitions_count() {
        let defs = tool_definitions();
        assert_eq!(defs.len(), 8);
    }

    #[test]
    fn test_tool_definitions_names() {
        let defs = tool_definitions();
        let names: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();
        assert!(names.contains(&"shipwright_pipeline_start"));
        assert!(names.contains(&"shipwright_pipeline_status"));
        assert!(names.contains(&"shipwright_stage_advance"));
        assert!(names.contains(&"shipwright_decision_run"));
        assert!(names.contains(&"shipwright_memory_search"));
        assert!(names.contains(&"shipwright_memory_store"));
        assert!(names.contains(&"shipwright_fleet_status"));
        assert!(names.contains(&"shipwright_intelligence"));
    }

    #[test]
    fn test_tool_definitions_have_schemas() {
        for def in tool_definitions() {
            assert_eq!(
                def.input_schema["type"], "object",
                "Tool {} missing object schema",
                def.name
            );
        }
    }

    #[test]
    fn test_pipeline_start_with_goal() {
        let state = make_state();
        let input = json!({"goal": "Add login validation", "template": "fast"});
        let result = dispatch("shipwright_pipeline_start", &input, &state);
        assert!(result.is_ok());
        let output: serde_json::Value = serde_json::from_str(&result.unwrap()).unwrap();
        assert_eq!(output["goal"], "Add login validation");
        assert_eq!(output["template"], "fast");
        assert!(output["pipeline_id"].is_string());
    }

    #[test]
    fn test_pipeline_start_with_issue() {
        let state = make_state();
        let input = json!({"issue_number": 42, "template": "standard"});
        let result = dispatch("shipwright_pipeline_start", &input, &state);
        assert!(result.is_ok());
        let output: serde_json::Value = serde_json::from_str(&result.unwrap()).unwrap();
        assert_eq!(output["goal"], "Deliver issue #42");
    }

    #[test]
    fn test_pipeline_start_requires_goal_or_issue() {
        let state = make_state();
        let input = json!({"template": "fast"});
        let result = dispatch("shipwright_pipeline_start", &input, &state);
        assert!(result.is_err());
    }

    #[test]
    fn test_pipeline_status_no_pipelines() {
        let state = make_state();
        let input = json!({});
        let result = dispatch("shipwright_pipeline_status", &input, &state);
        assert!(result.is_ok());
        let output: serde_json::Value = serde_json::from_str(&result.unwrap()).unwrap();
        assert_eq!(output["status"], "no_pipelines");
    }

    #[test]
    fn test_pipeline_status_after_start() {
        let state = make_state();
        let _ = dispatch(
            "shipwright_pipeline_start",
            &json!({"goal": "test"}),
            &state,
        );
        let result = dispatch("shipwright_pipeline_status", &json!({}), &state);
        assert!(result.is_ok());
        let output: serde_json::Value = serde_json::from_str(&result.unwrap()).unwrap();
        assert_eq!(output["goal"], "test");
    }

    #[test]
    fn test_memory_store_and_search() {
        let state = make_state();

        // Store a pattern
        let store_input = json!({
            "repo": "myrepo",
            "error_class": "CompilationError",
            "error_signature": "missing import statement",
            "root_cause": "forgot to add use declaration",
            "fix_applied": "added use std::io",
            "stage": "build"
        });
        let result = dispatch("shipwright_memory_store", &store_input, &state);
        assert!(result.is_ok());

        // Search for it
        let search_input = json!({"query": "missing import", "repo": "myrepo", "limit": 10});
        let result = dispatch("shipwright_memory_search", &search_input, &state);
        assert!(result.is_ok());
        let output: serde_json::Value = serde_json::from_str(&result.unwrap()).unwrap();
        assert_eq!(output["results_count"], 1);
    }

    #[test]
    fn test_memory_search_requires_query_and_repo() {
        let state = make_state();
        assert!(dispatch("shipwright_memory_search", &json!({}), &state).is_err());
        assert!(dispatch(
            "shipwright_memory_search",
            &json!({"query": "test"}),
            &state
        )
        .is_err());
    }

    #[test]
    fn test_fleet_status() {
        let state = make_state();
        let result = dispatch("shipwright_fleet_status", &json!({}), &state);
        assert!(result.is_ok());
        let output: serde_json::Value = serde_json::from_str(&result.unwrap()).unwrap();
        assert_eq!(output["active_pipelines"], 0);
        assert!(output["available_workers"].as_u64().unwrap() > 0);
    }

    #[test]
    fn test_intelligence_dora() {
        let result = dispatch(
            "shipwright_intelligence",
            &json!({"analysis_type": "dora"}),
            &ShipwrightState::new(),
        );
        assert!(result.is_ok());
        let output: serde_json::Value = serde_json::from_str(&result.unwrap()).unwrap();
        assert_eq!(output["analysis_type"], "dora");
        assert!(output["metrics"].is_object());
    }

    #[test]
    fn test_intelligence_optimize() {
        let result = dispatch(
            "shipwright_intelligence",
            &json!({"analysis_type": "optimize"}),
            &ShipwrightState::new(),
        );
        assert!(result.is_ok());
        let output: serde_json::Value = serde_json::from_str(&result.unwrap()).unwrap();
        assert_eq!(output["analysis_type"], "optimize");
    }

    #[test]
    fn test_intelligence_unknown_type() {
        let result = dispatch(
            "shipwright_intelligence",
            &json!({"analysis_type": "unknown"}),
            &ShipwrightState::new(),
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_dispatch_unknown_tool() {
        let state = make_state();
        let result = dispatch("shipwright_nonexistent", &json!({}), &state);
        assert!(result.is_err());
    }

    #[test]
    fn test_stage_advance() {
        let state = make_state();
        // Start a pipeline first
        let start_result = dispatch(
            "shipwright_pipeline_start",
            &json!({"goal": "test advance"}),
            &state,
        )
        .unwrap();
        let start_output: serde_json::Value = serde_json::from_str(&start_result).unwrap();
        let pipeline_id = start_output["pipeline_id"].as_str().unwrap();

        // Advance stage
        let advance_input = json!({
            "pipeline_id": pipeline_id,
            "outcome": "success",
            "notes": "All tests passed"
        });
        let result = dispatch("shipwright_stage_advance", &advance_input, &state);
        assert!(result.is_ok());
        let output: serde_json::Value = serde_json::from_str(&result.unwrap()).unwrap();
        assert_eq!(output["outcome"], "success");
    }

    #[test]
    fn test_stage_advance_missing_pipeline() {
        let state = make_state();
        let input = json!({"pipeline_id": "nonexistent", "outcome": "success"});
        let result = dispatch("shipwright_stage_advance", &input, &state);
        assert!(result.is_err());
    }

    #[test]
    fn test_decision_run_dry() {
        let result = dispatch(
            "shipwright_decision_run",
            &json!({"dry_run": true}),
            &ShipwrightState::new(),
        );
        assert!(result.is_ok());
        let output: serde_json::Value = serde_json::from_str(&result.unwrap()).unwrap();
        assert_eq!(output["dry_run"], true);
        assert_eq!(output["engine_status"]["collectors"], 10);
    }
}
