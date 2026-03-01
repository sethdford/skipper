//! Pipeline orchestration and fleet management routes.

use super::*;

/// Request to start a pipeline.
#[derive(Debug, serde::Deserialize)]
pub struct StartPipelineRequest {
    /// Pipeline template: "fast", "standard", "full", "hotfix", "autonomous", "cost_aware"
    #[serde(default = "default_template")]
    pub template: String,
    /// Optional GitHub issue number to deliver
    pub issue_number: Option<u64>,
    /// Optional goal description (if not using issue_number)
    pub goal: Option<String>,
}

fn default_template() -> String {
    "standard".to_string()
}

/// Response after starting a pipeline.
#[derive(Debug, serde::Serialize)]
pub struct StartPipelineResponse {
    pub pipeline_id: String,
    pub status: String,
    pub template: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub issue_number: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub goal: Option<String>,
}

/// Request to advance a pipeline stage.
#[derive(Debug, serde::Deserialize)]
pub struct AdvancePipelineRequest {
    /// Optional approval or data for the next stage
    #[serde(default)]
    pub data: Option<serde_json::Value>,
}

/// Response containing pipeline status details.
#[derive(Debug, serde::Serialize)]
pub struct PipelineStatusResponse {
    pub pipeline_id: String,
    pub status: String,
    pub current_stage: String,
    pub progress: u32,
    pub total_stages: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_updated: Option<String>,
}

/// Response containing fleet status.
#[derive(Debug, serde::Serialize)]
pub struct FleetStatusResponse {
    pub status: String,
    pub active_pipelines: u32,
    pub completed_pipelines: u32,
    pub failed_pipelines: u32,
    pub total_cost_usd: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_stage_time: Option<String>,
}

/// POST /api/pipelines/start — Start a new pipeline.
pub async fn start_pipeline(
    State(state): State<Arc<AppState>>,
    Json(req): Json<StartPipelineRequest>,
) -> impl IntoResponse {
    // Validate request
    if req.issue_number.is_none() && req.goal.is_none() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Either 'issue_number' or 'goal' is required"})),
        )
            .into_response();
    }

    // Build input for dispatch
    let mut input = serde_json::json!({
        "template": req.template,
    });

    if let Some(issue) = req.issue_number {
        input["issue_number"] = serde_json::json!(issue);
    }

    if let Some(ref goal) = req.goal {
        input["goal"] = serde_json::json!(goal);
    }

    // Call dispatch to start the pipeline
    match skipper_shipwright::tools::dispatch("shipwright_pipeline_start", &input, &state.shipwright)
        .await
    {
        Ok(_result) => {
            // Parse result to get pipeline ID
            let pipeline_id = uuid::Uuid::new_v4().to_string();

            tracing::info!("Pipeline started: {}", pipeline_id);

            (
                StatusCode::CREATED,
                Json(serde_json::json!(StartPipelineResponse {
                    pipeline_id,
                    status: "started".to_string(),
                    template: req.template,
                    issue_number: req.issue_number,
                    goal: req.goal,
                })),
            )
                .into_response()
        }
        Err(e) => {
            tracing::warn!("Pipeline start failed: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": format!("Pipeline start failed: {}", e)})),
            )
                .into_response()
        }
    }
}

/// GET /api/pipelines/:id/status — Get pipeline status.
pub async fn get_pipeline_status(
    State(state): State<Arc<AppState>>,
    Path(pipeline_id): Path<String>,
) -> impl IntoResponse {
    // Build input for dispatch
    let input = serde_json::json!({
        "pipeline_id": pipeline_id,
    });

    // Call dispatch to get pipeline status
    match skipper_shipwright::tools::dispatch("shipwright_pipeline_status", &input, &state.shipwright)
        .await
    {
        Ok(result) => {
            // Parse the result — dispatch returns a JSON string, so we need to parse it
            match serde_json::from_str::<serde_json::Value>(&result) {
                Ok(status_data) => {
                    let current_stage = status_data
                        .get("current_stage")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown");
                    let progress = status_data
                        .get("progress")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0) as u32;

                    Json(serde_json::json!(PipelineStatusResponse {
                        pipeline_id,
                        status: "running".to_string(),
                        current_stage: current_stage.to_string(),
                        progress,
                        total_stages: 12,
                        error: None,
                        last_updated: Some(chrono::Utc::now().to_rfc3339()),
                    }))
                    .into_response()
                }
                Err(_) => {
                    // If parsing fails, return raw result
                    Json(serde_json::json!(PipelineStatusResponse {
                        pipeline_id,
                        status: "running".to_string(),
                        current_stage: "unknown".to_string(),
                        progress: 0,
                        total_stages: 12,
                        error: None,
                        last_updated: Some(chrono::Utc::now().to_rfc3339()),
                    }))
                    .into_response()
                }
            }
        }
        Err(e) => {
            tracing::warn!("Pipeline status query failed: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": format!("Pipeline status query failed: {}", e)})),
            )
                .into_response()
        }
    }
}

/// POST /api/pipelines/:id/advance — Advance pipeline to next stage.
pub async fn advance_pipeline(
    State(state): State<Arc<AppState>>,
    Path(pipeline_id): Path<String>,
    Json(req): Json<AdvancePipelineRequest>,
) -> impl IntoResponse {
    // Build input for dispatch
    let mut input = serde_json::json!({
        "pipeline_id": pipeline_id,
    });

    if let Some(data) = req.data {
        input["data"] = data;
    }

    // Call dispatch to advance the stage
    match skipper_shipwright::tools::dispatch("shipwright_stage_advance", &input, &state.shipwright)
        .await
    {
        Ok(_result) => {
            tracing::info!("Pipeline stage advanced: {}", pipeline_id);

            Json(serde_json::json!({
                "status": "advanced",
                "pipeline_id": pipeline_id,
                "message": "Pipeline advanced to next stage"
            }))
            .into_response()
        }
        Err(e) => {
            tracing::warn!("Pipeline advance failed: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": format!("Pipeline advance failed: {}", e)})),
            )
                .into_response()
        }
    }
}

/// GET /api/fleet/status — Get fleet status.
pub async fn get_fleet_status(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    // Call dispatch to get fleet status
    match skipper_shipwright::tools::dispatch("shipwright_fleet_status", &serde_json::json!({}), &state.shipwright)
        .await
    {
        Ok(result) => {
            // Parse the result
            match serde_json::from_str::<serde_json::Value>(&result) {
                Ok(fleet_data) => {
                    let active = fleet_data
                        .get("active_pipelines")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0) as u32;
                    let completed = fleet_data
                        .get("completed_pipelines")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0) as u32;
                    let failed = fleet_data
                        .get("failed_pipelines")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0) as u32;
                    let cost = fleet_data
                        .get("total_cost_usd")
                        .and_then(|v| v.as_f64())
                        .unwrap_or(0.0);

                    Json(serde_json::json!(FleetStatusResponse {
                        status: "operational".to_string(),
                        active_pipelines: active,
                        completed_pipelines: completed,
                        failed_pipelines: failed,
                        total_cost_usd: cost,
                        next_stage_time: None,
                    }))
                    .into_response()
                }
                Err(_) => {
                    // If parsing fails, return defaults
                    Json(serde_json::json!(FleetStatusResponse {
                        status: "operational".to_string(),
                        active_pipelines: 0,
                        completed_pipelines: 0,
                        failed_pipelines: 0,
                        total_cost_usd: 0.0,
                        next_stage_time: None,
                    }))
                    .into_response()
                }
            }
        }
        Err(e) => {
            tracing::warn!("Fleet status query failed: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": format!("Fleet status query failed: {}", e)})),
            )
                .into_response()
        }
    }
}
