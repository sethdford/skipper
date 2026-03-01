//! Health, status, version, and metrics route handlers.

use super::*;

/// GET /api/status — Agent registry and uptime.
pub async fn status(State(state): State<Arc<AppState>>) -> impl IntoResponse {
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
            })
        })
        .collect();

    let uptime = state.started_at.elapsed().as_secs();
    let agent_count = agents.len();

    Json(serde_json::json!({
        "status": "running",
        "agent_count": agent_count,
        "default_provider": state.kernel.config.default_model.provider,
        "default_model": state.kernel.config.default_model.model,
        "uptime_seconds": uptime,
        "agents": agents,
    }))
}

/// POST /api/shutdown — Graceful shutdown.
pub async fn shutdown(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    tracing::info!("Shutdown requested via API");
    // SECURITY: Record shutdown in audit trail
    state.kernel.audit_log.record(
        "system",
        skipper_runtime::audit::AuditAction::ConfigChange,
        "shutdown requested via API",
        "ok",
    );
    state.kernel.shutdown();
    // Signal the HTTP server to initiate graceful shutdown so the process exits.
    state.shutdown_notify.notify_one();
    Json(serde_json::json!({"status": "shutting_down"}))
}

/// GET /api/version — Version info.
pub async fn version() -> impl IntoResponse {
    Json(serde_json::json!({
        "name": "skipper",
        "version": env!("CARGO_PKG_VERSION"),
        "build_date": option_env!("BUILD_DATE").unwrap_or("dev"),
        "git_sha": option_env!("GIT_SHA").unwrap_or("unknown"),
        "rust_version": option_env!("RUSTC_VERSION").unwrap_or("unknown"),
        "platform": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
    }))
}

/// GET /api/health — Simple health check.
pub async fn health(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    // Check database connectivity
    let shared_id = skipper_types::agent::AgentId(uuid::Uuid::from_bytes([
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1,
    ]));
    let db_ok = state
        .kernel
        .memory
        .structured_get(shared_id, "__health_check__")
        .is_ok();

    let status = if db_ok { "ok" } else { "degraded" };

    Json(serde_json::json!({
        "status": status,
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

/// GET /api/health/detail — Full health diagnostics (requires auth).
pub async fn health_detail(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let health = state.kernel.supervisor.health();

    let shared_id = skipper_types::agent::AgentId(uuid::Uuid::from_bytes([
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1,
    ]));
    let db_ok = state
        .kernel
        .memory
        .structured_get(shared_id, "__health_check__")
        .is_ok();

    let config_warnings = state.kernel.config.validate();
    let status = if db_ok { "ok" } else { "degraded" };

    Json(serde_json::json!({
        "status": status,
        "version": env!("CARGO_PKG_VERSION"),
        "uptime_seconds": state.started_at.elapsed().as_secs(),
        "panic_count": health.panic_count,
        "restart_count": health.restart_count,
        "agent_count": state.kernel.registry.count(),
        "database": if db_ok { "connected" } else { "error" },
        "config_warnings": config_warnings,
    }))
}

/// GET /api/metrics — Prometheus text-format metrics.
///
/// Returns counters and gauges for monitoring Skipper in production:
/// - `skipper_agents_active` — number of active agents
/// - `skipper_uptime_seconds` — seconds since daemon started
/// - `skipper_tokens_total` — total tokens consumed (per agent)
/// - `skipper_tool_calls_total` — total tool calls (per agent)
/// - `skipper_panics_total` — supervisor panic count
/// - `skipper_restarts_total` — supervisor restart count
pub async fn prometheus_metrics(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let mut out = String::with_capacity(2048);

    // Uptime
    let uptime = state.started_at.elapsed().as_secs();
    out.push_str("# HELP skipper_uptime_seconds Time since daemon started.\n");
    out.push_str("# TYPE skipper_uptime_seconds gauge\n");
    out.push_str(&format!("skipper_uptime_seconds {uptime}\n\n"));

    // Active agents
    let agents = state.kernel.registry.list();
    let active = agents
        .iter()
        .filter(|a| matches!(a.state, skipper_types::agent::AgentState::Running))
        .count();
    out.push_str("# HELP skipper_agents_active Number of active agents.\n");
    out.push_str("# TYPE skipper_agents_active gauge\n");
    out.push_str(&format!("skipper_agents_active {active}\n"));
    out.push_str("# HELP skipper_agents_total Total number of registered agents.\n");
    out.push_str("# TYPE skipper_agents_total gauge\n");
    out.push_str(&format!("skipper_agents_total {}\n\n", agents.len()));

    // Per-agent token and tool usage
    out.push_str("# HELP skipper_tokens_total Total tokens consumed (rolling hourly window).\n");
    out.push_str("# TYPE skipper_tokens_total gauge\n");
    out.push_str("# HELP skipper_tool_calls_total Total tool calls (rolling hourly window).\n");
    out.push_str("# TYPE skipper_tool_calls_total gauge\n");
    for agent in &agents {
        let name = &agent.name;
        let provider = &agent.manifest.model.provider;
        let model = &agent.manifest.model.model;
        if let Some((tokens, tools)) = state.kernel.scheduler.get_usage(agent.id) {
            out.push_str(&format!(
                "skipper_tokens_total{{agent=\"{name}\",provider=\"{provider}\",model=\"{model}\"}} {tokens}\n"
            ));
            out.push_str(&format!(
                "skipper_tool_calls_total{{agent=\"{name}\"}} {tools}\n"
            ));
        }
    }
    out.push('\n');

    // Supervisor health
    let health = state.kernel.supervisor.health();
    out.push_str("# HELP skipper_panics_total Total supervisor panics since start.\n");
    out.push_str("# TYPE skipper_panics_total counter\n");
    out.push_str(&format!("skipper_panics_total {}\n", health.panic_count));
    out.push_str("# HELP skipper_restarts_total Total supervisor restarts since start.\n");
    out.push_str("# TYPE skipper_restarts_total counter\n");
    out.push_str(&format!(
        "skipper_restarts_total {}\n\n",
        health.restart_count
    ));

    // Version info
    out.push_str("# HELP skipper_info Skipper version and build info.\n");
    out.push_str("# TYPE skipper_info gauge\n");
    out.push_str(&format!(
        "skipper_info{{version=\"{}\"}} 1\n",
        env!("CARGO_PKG_VERSION")
    ));

    (
        StatusCode::OK,
        [(
            axum::http::header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )],
        out,
    )
}
