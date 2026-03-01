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
use std::sync::Arc;
use std::time::Instant;

mod health;
pub use health::*;

mod budget;
pub use budget::*;

mod workflows;
pub use workflows::*;

mod channels;
pub use channels::*;

mod hands;
pub use hands::*;

mod skills;
pub use skills::*;

mod agents;
pub use agents::*;

mod network;
pub use network::*;

mod security;
pub use security::*;

mod settings;
pub use settings::*;

mod integrations;
pub use integrations::*;

mod misc;
pub use misc::*;

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

// ---------------------------------------------------------------------------
// Shared helper functions and constants
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
