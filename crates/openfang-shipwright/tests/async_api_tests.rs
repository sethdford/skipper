//! Async API handler tests using axum test utilities.

#[cfg(test)]
mod tests {
    use openfang_shipwright::api::{
        ApiResponse, CandidateResponse, FleetStatusResponse, PipelineStatus,
    };

    #[tokio::test]
    async fn test_api_response_serialization() {
        let response: ApiResponse<String> = ApiResponse {
            data: "test".to_string(),
            status: "ok".to_string(),
        };
        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("\"data\":\"test\""));
        assert!(json.contains("\"status\":\"ok\""));
    }

    #[tokio::test]
    async fn test_pipeline_status_fields() {
        let status = PipelineStatus {
            id: "pl-1".to_string(),
            issue: 123,
            stage: "build".to_string(),
            status: "running".to_string(),
            cost: 2.50,
        };

        assert_eq!(status.id, "pl-1");
        assert_eq!(status.issue, 123);
        assert_eq!(status.stage, "build");
        assert_eq!(status.status, "running");
        assert!(status.cost > 0.0);
    }

    #[tokio::test]
    async fn test_fleet_status_response_fields() {
        let status = FleetStatusResponse {
            active_pipelines: 3,
            queued_issues: 5,
            allocated_workers: 2,
        };

        assert_eq!(status.active_pipelines, 3);
        assert_eq!(status.queued_issues, 5);
        assert_eq!(status.allocated_workers, 2);
    }

    #[tokio::test]
    async fn test_candidate_response_structure() {
        let candidate = CandidateResponse {
            id: "c1".to_string(),
            signal: "security".to_string(),
            title: "Update vulnerable dependency".to_string(),
            score: 85,
            tier: "auto".to_string(),
        };

        assert_eq!(candidate.id, "c1");
        assert_eq!(candidate.signal, "security");
        assert_eq!(candidate.title, "Update vulnerable dependency");
        assert_eq!(candidate.score, 85);
        assert_eq!(candidate.tier, "auto");
    }

    #[tokio::test]
    async fn test_api_response_generic() {
        let response: ApiResponse<Vec<String>> = ApiResponse {
            data: vec!["a".to_string(), "b".to_string()],
            status: "ok".to_string(),
        };
        assert_eq!(response.data.len(), 2);
    }
}
