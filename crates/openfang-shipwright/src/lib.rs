//! OpenFang Shipwright — Software Engineering Hand
//!
//! Autonomous delivery pipelines: Issue → Plan → Build → Test → PR → Deploy.
//! Integrates with OpenFang kernel for scheduling, RBAC, metering,
//! and cross-Hand intelligence via Collector, Researcher, and Predictor.
//!
//! ## Overview
//!
//! Shipwright is an autonomous software delivery system implementing a 12-stage pipeline
//! with intelligent decision-making, self-healing build loops, vector-based memory,
//! fleet orchestration with auto-scaling, and cross-Hand collaboration via OpenFang's kernel.
//!
//! ### Core Components
//!
//! - **Pipeline** (`pipeline`): 12-stage delivery engine (Intake → Monitor) with state machine,
//!   templates (Fast/Standard/Full/Hotfix/Autonomous/CostAware), and convergence-driven
//!   self-healing build loops.
//!
//! - **Decision Engine** (`decision`): Autonomous what-to-build decisions using 10+ signal
//!   collectors (security, dependencies, coverage, etc.), EMA-adjusted scoring, and Hand
//!   cross-pollination to discover shared learning signals from other OpenFang Hands.
//!
//! - **Memory System** (`memory`): Vector-based failure pattern store with semantic search,
//!   architecture rule enforcement, and outcome learning that adjusts decision weights
//!   over time via exponential moving average.
//!
//! - **GitHub Integration** (`github`): REST and GraphQL API client for issue management,
//!   file change frequency analysis, blame data, security alerts, PR lifecycle, Checks API,
//!   and Deployments API for deploy tracking.
//!
//! - **Intelligence Layer** (`intelligence`): DORA metrics calculation (lead time, deploy
//!   frequency, CFR, MTTR), risk prediction using file hotspots, anomaly detection,
//!   and self-optimization that adjusts config based on DORA trends.
//!
//! - **Fleet Manager** (`fleet`): Worker pool orchestration with static and auto-scaling
//!   strategies, daemon polling for labeled issues, triage scoring, and patrol that feeds
//!   the decision engine with periodic signals.
//!
//! - **Hand Definition** (`hand`): Embedded HAND.toml, SKILL.md, and system_prompt.md
//!   bundled for OpenFang's hand registry.
//!
//! - **API Routes** (`api`): HTTP endpoints for pipeline management, decision cycles,
//!   fleet status, DORA metrics, and memory search.
//!
//! - **Dashboard** (`dashboard`): Alpine.js SPA page generators for 6 views:
//!   pipelines, fleet, decisions, memory, intelligence, cost.
//!
//! ### Configuration
//!
//! Configure Shipwright in `openfang.toml`:
//!
//! ```toml
//! [shipwright]
//! enabled = true
//! default_template = "standard"
//!
//! [shipwright.fleet]
//! auto_scale = true
//! max_workers = 8
//! poll_interval_seconds = 60
//!
//! [shipwright.decision]
//! enabled = true
//! max_issues_per_day = 15
//! max_cost_per_day_usd = 25.0
//!
//! [shipwright.intelligence]
//! prediction_enabled = true
//! ```
//!
//! ### Example: Starting a Pipeline
//!
//! ```no_run
//! use openfang_shipwright::{Pipeline, pipeline::PipelineTemplate};
//!
//! let template = PipelineTemplate::standard();
//! let pipeline = Pipeline::from_issue(123, "test goal".to_string(), template);
//! // Run pipeline stages: Intake → Plan → Build → Test → ...
//! ```
//!
//! ### Example: Running Decision Engine
//!
//! ```no_run
//! use openfang_shipwright::DecisionEngine;
//!
//! let engine = DecisionEngine::new();
//! // Collect signals from 10+ sources, score candidates, resolve tiers
//! ```

pub mod api;
pub mod config;
pub mod dashboard;
pub mod decision;
pub mod github;
pub mod hand;
pub mod intelligence;
pub mod memory;
pub mod pipeline;
pub mod fleet;

pub use config::ShipwrightConfig;
pub use decision::DecisionEngine;
pub use github::GitHubClient;
pub use hand::HandDefinition;
pub use intelligence::IntelligenceEngine;
pub use memory::ShipwrightMemory;
pub use pipeline::Pipeline;

// Canonical type exports (H4: consolidate duplicates)
pub use memory::learning::ScoringWeights;
pub use pipeline::ModelChoice;
pub use decision::signals::Candidate;
pub use intelligence::dora::DoraMetrics;
pub use github::Issue;
