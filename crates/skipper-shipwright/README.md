# Skipper Shipwright — Autonomous Software Delivery Hand

Shipwright is an Skipper Hand that automates the entire software delivery pipeline: from GitHub issue detection through testing and PR creation to production deployment. It implements 12-stage pipelines with intelligent decision-making, self-healing build loops, vector-based memory, and fleet orchestration.

## Features

### 12-Stage Pipeline Engine

- **Stages**: Intake → Plan → Design → Build → Test → Review → CompoundQuality → PR → Merge → Deploy → Validate → Monitor
- **Templates**: Fast (4 stages), Standard (with review), Full (all stages), Hotfix, Autonomous, CostAware
- **State Machine**: Prevents invalid transitions; tracks iteration count, stage progress, error history
- **Self-Healing**: Convergence detection via error reduction; backtracking on divergence; configurable max iterations

### Decision Engine

- **10+ Signal Collectors**: Security (CVE parsing), Dependency updates, Coverage regression, Dead code, Test flakiness, Architecture violations, and Hand signals from other Skipper Hands
- **Scoring Formula**: `value = (impact × 0.30) + (urgency × 0.25) + (effort × 0.20) + (confidence × 0.15) - (risk × 0.10)`
- **Autonomy Tiers**: Auto (execute immediately), Propose (create issue), Draft (comment), Manual (approval required)
- **Deduplication**: Against open GitHub issues and recent decisions (7-day window)
- **Learning**: EMA-adjusted weights from outcome history refine scoring over time

### Vector Memory System

- **Failure Patterns**: Store root cause + fix + recurrence count with vector embeddings
- **Semantic Search**: Find similar patterns using cosine similarity, not just keyword matching
- **Architecture Rules**: Define layers and dependency constraints; detect violations
- **Learning Outcomes**: Record decision success/failure; adjust scoring weights
- **Persistence**: SQLite with sqlite-vec for embeddings

### GitHub Integration

- **REST API**: List issues, create issues, add labels
- **GraphQL**: File change frequency, blame data, security alerts, CODEOWNERS, similar issues, commit history
- **Checks API**: Create/update GitHub Check Runs per pipeline stage
- **Deployments API**: Track deployments per environment, enable rollback
- **PR Lifecycle**: Create PRs, select reviewers (CODEOWNERS first, then top contributors), auto-merge

### DORA Metrics & Intelligence

- **Lead Time**: Hours from first commit to deploy
- **Deployment Frequency**: Deploys per day
- **Change Failure Rate**: Ratio of failed/total deploys
- **MTTR**: Time from failure detection to recovery
- **Performer Level**: Elite/High/Medium/Low classification
- **Risk Prediction**: File hotspot analysis; anomaly detection (z-score)
- **Self-Optimization**: Recommends config changes based on DORA trends

### Fleet Orchestration

- **Worker Pool**: Static and auto-scaling strategies
- **Auto-Scaling**: Min(75% CPU cores, available memory ÷ worker_mem_gb, remaining daily budget ÷ cost_per_job)
- **Daemon Polling**: Watches GitHub for labeled issues; dispatches up to max_parallel
- **Triage Scoring**: Prioritizes by issue labels (p0 > p1 > unlabeled)
- **Patrol**: Runs periodic checks (outdated dependencies, security vulnerabilities, coverage regression, DORA degradation)

### Dashboard

Six Alpine.js SPA pages:

1. **Pipelines**: Active pipelines, stage progress bar, build loop status
2. **Fleet**: Worker allocation, auto-scaling metrics, per-repo queue depth
3. **Decisions**: Active candidates with score breakdown, tier breakdown, recent decision history
4. **Memory**: Failure pattern search, architecture rule viewer, learning outcome metrics
5. **Intelligence**: DORA metric cards with trends, risk heatmap by file, anomaly alerts, optimization suggestions
6. **Cost**: Daily budget status, token usage breakdown, cost by pipeline, 30-day trends

### CLI & API

**CLI Subcommands**:

```bash
skipper shipwright pipeline start --issue 123
skipper shipwright fleet status
skipper shipwright decide run
skipper shipwright memory search "undefined property"
skipper shipwright dora --repo myorg/myrepo
skipper shipwright cost show
skipper shipwright doctor
```

**HTTP Endpoints**:

- `GET /api/shipwright/pipelines` — List active/completed pipelines
- `POST /api/shipwright/pipelines` — Start new pipeline
- `GET /api/shipwright/pipelines/:id` — Pipeline detail with stage status
- `GET /api/shipwright/pipelines/:id/ws` — WebSocket for real-time updates
- `POST /api/shipwright/decide/run` — Trigger decision cycle
- `GET /api/shipwright/decide/candidates` — List current candidates
- `GET /api/shipwright/fleet/status` — Fleet overview
- `GET /api/shipwright/dora/:repo` — DORA metrics
- `GET /api/shipwright/memory/search` — Semantic memory search

## Configuration

### Full Example (skipper.toml)

```toml
[shipwright]
enabled = true
default_template = "standard"

[shipwright.fleet]
poll_interval_seconds = 60
auto_scale = true
max_workers = 8
min_workers = 1
worker_mem_gb = 4
cost_per_job_usd = 5.0

[shipwright.decision]
enabled = true
cycle_interval_seconds = 1800
max_issues_per_day = 15
max_cost_per_day_usd = 25.0
cooldown_seconds = 300
halt_after_failures = 3
outcome_learning = true

[shipwright.intelligence]
cache_ttl_seconds = 3600
prediction_enabled = true
adversarial_enabled = false
architecture_enabled = false

[shipwright.github]
watch_labels = ["shipwright", "ready-to-build"]
auto_merge = false
check_runs_enabled = true
deployment_tracking = true

[[shipwright.repos]]
path = "/home/user/myorg/myrepo"
owner = "myorg"
repo = "myrepo"
template = "standard"
max_parallel = 2
auto_merge = false
```

## Quick Start

### 1. Enable Shipwright

```toml
[shipwright]
enabled = true
default_template = "standard"
```

### 2. Set GitHub Token

```bash
export GITHUB_TOKEN=ghp_...
```

### 3. Start Pipeline for an Issue

```bash
skipper shipwright pipeline start --issue 123
```

### 4. Monitor Dashboard

Open `http://localhost:4200/shipwright/pipelines` to watch stage progress.

## Architecture

### Module Organization

```
src/
├── lib.rs              # Crate exports
├── config.rs           # ShipwrightConfig, templates, per-repo config
├── pipeline/           # 12-stage engine
│   ├── mod.rs          # Pipeline struct, state machine
│   ├── stages.rs       # Stage enum, state transitions
│   ├── templates.rs    # Pipeline templates (Fast/Standard/Full/etc)
│   ├── self_healing.rs # Convergence detection, build loop
│   └── composer.rs     # Dynamic template composition
├── decision/           # Decision engine
│   ├── mod.rs          # DecisionEngine orchestrator
│   ├── signals.rs      # SignalCollector trait, 10+ implementations
│   ├── scoring.rs      # Score formula, EMA weight adjustment
│   └── autonomy.rs     # Tier resolution, budget/rate limiting
├── memory/             # Vector memory system
│   ├── mod.rs          # ShipwrightMemory wrapper
│   ├── patterns.rs     # FailurePattern with vector search
│   ├── architecture.rs # ArchitectureRule enforcement
│   └── learning.rs     # Outcome tracking, weight adjustment
├── github/             # GitHub integration
│   ├── mod.rs          # GitHubClient, Issue type
│   ├── graphql.rs      # GraphQL queries
│   ├── checks.rs       # Checks API
│   ├── deployments.rs  # Deployments API
│   └── pr.rs           # PR lifecycle
├── intelligence/       # Intelligence layer
│   ├── mod.rs          # IntelligenceEngine
│   ├── dora.rs         # DORA metrics (lead time, freq, CFR, MTTR)
│   ├── prediction.rs   # Risk scoring, anomaly detection
│   └── optimization.rs # Self-tuning, config recommendations
├── fleet/              # Fleet orchestration
│   ├── mod.rs          # FleetManager
│   ├── daemon.rs       # Polling loop, issue dispatch
│   ├── dispatch.rs     # Worker pool, auto-scaling
│   └── patrol.rs       # Periodic checks → decision engine
├── hand.rs             # Hand definition (HAND.toml, SKILL.md, system_prompt)
├── api.rs              # HTTP routes
├── dashboard.rs        # HTML page generators
└── cli.rs              # CLI subcommand dispatcher
```

### Integration with Skipper

Shipwright integrates with Skipper's kernel via:

- **KernelHandle**: For scheduling, RBAC, metering, agent spawning
- **Memory Store**: skipper-memory for vector embeddings
- **Collector/Researcher/Predictor Hands**: Cross-pollination signals
- **Event Bus**: Real-time notifications
- **Hand Registry**: Bundled HAND.toml, SKILL.md, system_prompt

## Testing

### Run All Tests

```bash
cargo test -p skipper-shipwright
```

### Test Coverage

- **Unit Tests** (~280): Config, pipeline state machine, decision scoring, memory search, DORA calculations
- **Integration Tests** (~40): Full pipeline lifecycle, decision engine with collectors, fleet dispatch
- **Edge Cases**: NaN/Infinity handling, concurrent access, schema migrations

### Live Integration Test

```bash
# Start daemon
GITHUB_TOKEN=ghp_... cargo run --release --bin skipper start &
sleep 6

# Test endpoints
curl http://localhost:4200/api/shipwright/pipelines
curl http://localhost:4200/api/shipwright/fleet/status

# Cleanup
pkill -f "skipper start"
```

## Examples

### Start a Pipeline from an Issue

```rust
use skipper_shipwright::Pipeline;

let pipeline = Pipeline::from_issue(123, "standard");
// Stages advance automatically via daemon polling
// Or manually via: pipeline.advance_stage()
```

### Run Decision Engine Cycle

```rust
use skipper_shipwright::DecisionEngine;

let engine = DecisionEngine::new(Default::default());
let candidates = engine.run_cycle().await?;
// candidates include security patches, dependency updates, coverage improvements
```

### Search Memory for Similar Failures

```rust
use skipper_shipwright::ShipwrightMemory;

let memory = ShipwrightMemory::new();
let patterns = memory.search_failures("undefined property", 5)?;
// Returns top 5 failure patterns by similarity
```

### Get DORA Metrics

```rust
use skipper_shipwright::IntelligenceEngine;

let engine = IntelligenceEngine::new(Default::default());
let metrics = engine.calculate_dora("myorg/myrepo").await?;
println!("Lead time: {}h", metrics.lead_time_hours);
println!("Level: {}", metrics.level); // Elite/High/Medium/Low
```

## Performance Characteristics

- **Pipeline State Transitions**: O(1) — state machine lookup
- **Decision Scoring**: O(n) where n = number of candidates (~10-50)
- **Memory Search**: O(1) with DashMap + semantic similarity in skipper-memory
- **DORA Calculation**: O(n) where n = deployments in window (usually < 100)
- **Fleet Auto-Scaling**: O(m) where m = number of repos (usually < 20)

## Troubleshooting

### Pipeline Stuck in Build Loop

Check `progress.md` in pipeline artifacts for iteration count and last error:

```bash
cat .claude/pipeline-artifacts/progress.md
```

Increase max iterations or check error log:

```bash
cat .claude/pipeline-artifacts/error-log.jsonl
```

### Decision Engine Not Finding Candidates

1. Verify collectors are enabled in config
2. Check GitHub token and repo access
3. Review decision cycle logs via dashboard or:
   ```bash
   skipper shipwright decide run
   ```

### Memory Search Returns No Results

1. Store failure patterns first:
   ```bash
   skipper shipwright memory import-jsonl path/to/failures.jsonl
   ```
2. Verify embeddings are generated
3. Try keyword search via GitHub issues

### DORA Metrics Show Zeros

1. Ensure deployment tracking is enabled: `deployment_tracking = true`
2. Check GitHub Deployments API access
3. Verify at least one deployment in the window

## Contributing

Contributions welcome! Areas for enhancement:

- Additional signal collectors (e.g., Slack integration, log anomalies)
- Advanced models for risk prediction (e.g., gradient boosting)
- Dashboard real-time updates via WebSocket
- Integration with more version control systems

## License

Same as Skipper (see root LICENSE)
