//! Shipwright API routes for Axum server.
//!
//! HTTP REST API endpoints for pipeline management, decision engine,
//! fleet status, DORA metrics, and memory system.

use axum::{
    extract::{Path, Query},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};

/// Generic API response wrapper
#[derive(Debug, Serialize, Deserialize)]
pub struct ApiResponse<T> {
    pub data: T,
    pub status: String,
}

/// Memory search query parameters
#[derive(Debug, Deserialize)]
pub struct MemorySearchQuery {
    pub q: Option<String>,
    pub repo: Option<String>,
    pub limit: Option<usize>,
}

/// Pipeline execution status
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PipelineStatus {
    pub id: String,
    pub issue: u32,
    pub stage: String,
    pub status: String,
    pub cost: f64,
}

/// Fleet status snapshot
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct FleetStatusResponse {
    pub active_pipelines: u32,
    pub queued_issues: u32,
    pub allocated_workers: u32,
}

/// Decision candidate (API response DTO)
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CandidateResponse {
    pub id: String,
    pub signal: String,
    pub title: String,
    pub score: u32,
    pub tier: String,
}

// DORA metrics: use canonical type from intelligence::dora
use crate::intelligence::dora::DoraMetrics;

/// Create the Shipwright API router
pub fn router<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    Router::new()
        .route("/pipelines", get(list_pipelines).post(start_pipeline))
        .route("/pipelines/:id", get(get_pipeline))
        .route("/fleet/status", get(fleet_status))
        .route("/decide/run", post(run_decision))
        .route("/decide/candidates", get(list_candidates))
        .route("/dora/:repo", get(get_dora_metrics))
        .route("/memory/search", get(search_memory))
}

async fn list_pipelines() -> impl IntoResponse {
    let pipelines = vec![PipelineStatus {
        id: "pl-1".into(),
        issue: 123,
        stage: "build".into(),
        status: "running".into(),
        cost: 2.50,
    }];
    Json(ApiResponse {
        data: pipelines,
        status: "ok".into(),
    })
}

async fn start_pipeline() -> impl IntoResponse {
    (
        StatusCode::CREATED,
        Json(ApiResponse {
            data: PipelineStatus {
                id: "pl-new".into(),
                issue: 124,
                stage: "intake".into(),
                status: "running".into(),
                cost: 0.0,
            },
            status: "created".into(),
        }),
    )
}

async fn get_pipeline(Path(id): Path<String>) -> impl IntoResponse {
    let pipeline = PipelineStatus {
        id,
        issue: 123,
        stage: "test".into(),
        status: "running".into(),
        cost: 1.50,
    };
    Json(ApiResponse {
        data: Some(pipeline),
        status: "ok".into(),
    })
}

async fn fleet_status() -> impl IntoResponse {
    let status = FleetStatusResponse {
        active_pipelines: 3,
        queued_issues: 5,
        allocated_workers: 2,
    };
    Json(ApiResponse {
        data: status,
        status: "ok".into(),
    })
}

async fn run_decision() -> impl IntoResponse {
    Json(ApiResponse {
        data: serde_json::json!({"cycle_id": "dc-123", "candidates_found": 3}),
        status: "ok".into(),
    })
}

async fn list_candidates() -> impl IntoResponse {
    let candidates = vec![
        CandidateResponse {
            id: "c1".into(),
            signal: "security".into(),
            title: "Update vulnerable dependency".into(),
            score: 85,
            tier: "auto".into(),
        },
        CandidateResponse {
            id: "c2".into(),
            signal: "coverage".into(),
            title: "Improve test coverage".into(),
            score: 45,
            tier: "propose".into(),
        },
    ];
    Json(ApiResponse {
        data: candidates,
        status: "ok".into(),
    })
}

async fn get_dora_metrics(Path(_repo): Path<String>) -> impl IntoResponse {
    let metrics = DoraMetrics::new(2.3, 1.5, 0.18, 0.5);
    Json(ApiResponse {
        data: metrics,
        status: "ok".into(),
    })
}

async fn search_memory(Query(params): Query<MemorySearchQuery>) -> impl IntoResponse {
    // In a real implementation, this would query the memory store
    // For now, return matching failure patterns based on query params
    use crate::memory::FailurePattern;

    let _query_text = params.q.unwrap_or_default();
    let _repo = params.repo.unwrap_or_else(|| "default".to_string());
    let _limit = params.limit.unwrap_or(10);

    // Mock response with empty results (would be populated from actual memory)
    let patterns: Vec<FailurePattern> = vec![];
    let has_matches = !patterns.is_empty();

    Json(ApiResponse {
        data: patterns,
        status: if has_matches { "ok" } else { "no_matches" }.into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_api_response_structure() {
        let response: ApiResponse<String> = ApiResponse {
            data: "test".into(),
            status: "ok".into(),
        };
        assert_eq!(response.status, "ok");
    }

    #[test]
    fn test_pipeline_status() {
        let status = PipelineStatus {
            id: "pl-1".into(),
            issue: 123,
            stage: "build".into(),
            status: "running".into(),
            cost: 1.5,
        };
        assert_eq!(status.issue, 123);
    }

    #[test]
    fn test_fleet_status_response() {
        let status = FleetStatusResponse {
            active_pipelines: 2,
            queued_issues: 5,
            allocated_workers: 2,
        };
        assert_eq!(status.active_pipelines, 2);
    }

    #[test]
    fn test_dora_metrics() {
        let metrics = DoraMetrics::new(2.0, 1.0, 0.15, 1.0);
        assert!(metrics.lead_time_hours > 0.0);
    }

    #[test]
    fn test_candidate_structure() {
        let cand = CandidateResponse {
            id: "c1".into(),
            signal: "security".into(),
            title: "Test".into(),
            score: 80,
            tier: "auto".into(),
        };
        assert_eq!(cand.score, 80);
    }
}
