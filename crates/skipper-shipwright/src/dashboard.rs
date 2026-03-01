//! Shipwright Dashboard — HTML generation for 6 SPA pages
//!
//! Generates Alpine.js templates for:
//! - Pipeline status (active pipelines, stage progress, build loop status)
//! - Fleet overview (worker pool, repo allocation, auto-scaling)
//! - Decisions log (recent decisions, candidates, tier breakdown)
//! - Memory explorer (failure patterns, architecture rules, outcomes)
//! - Intelligence dashboard (DORA metrics, risk predictions, optimization suggestions)
//! - Cost tracker (token usage, budget remaining, per-pipeline cost)

/// Generate the pipeline status page
pub fn pipeline_status_page() -> String {
    r#"
    <div class="page-section" x-show="page === 'shipwright-pipelines'" x-transition>
      <div class="page-header">
        <h1>Pipeline Status</h1>
        <p class="subtitle">Active pipelines, stage progress, build loop iterations</p>
      </div>

      <div class="card">
        <div class="card-header">
          <h2>Active Pipelines</h2>
          <button @click="refreshPipelines()" class="btn-secondary">Refresh</button>
        </div>

        <table class="table">
          <thead>
            <tr>
              <th>ID</th>
              <th>Issue</th>
              <th>Template</th>
              <th>Stage Progress</th>
              <th>Status</th>
              <th>Cost</th>
              <th>Duration</th>
            </tr>
          </thead>
          <tbody>
            <template x-for="pipeline in pipelines" :key="pipeline.id">
              <tr>
                <td><code x-text="pipeline.id"></code></td>
                <td x-text="'#' + pipeline.issue"></td>
                <td x-text="pipeline.template"></td>
                <td>
                  <div class="stage-progress">
                    <div class="stage-bar" :style="'width:' + pipeline.progress + '%'"></div>
                    <span class="stage-label" x-text="pipeline.stage"></span>
                  </div>
                </td>
                <td>
                  <span class="badge" :class="pipeline.status" x-text="pipeline.status"></span>
                </td>
                <td>${{ pipeline.cost.toFixed(2) }}</td>
                <td x-text="pipeline.duration_minutes + ' min'"></td>
              </tr>
            </template>
          </tbody>
        </table>

        <div x-show="pipelines.length === 0" class="empty-state">
          <p>No active pipelines</p>
        </div>
      </div>

      <div class="card" x-show="currentBuildLoop">
        <div class="card-header">
          <h2>Current Build Loop</h2>
        </div>
        <div class="build-loop-view">
          <div class="metric">
            <label>Iteration</label>
            <span x-text="currentBuildLoop.iteration + ' / ' + currentBuildLoop.max_iterations"></span>
          </div>
          <div class="metric">
            <label>Convergence</label>
            <span x-text="currentBuildLoop.convergence_status"></span>
          </div>
          <div class="metric">
            <label>Error Reduction</label>
            <span x-text="currentBuildLoop.error_reduction_percent.toFixed(1) + '%'"></span>
          </div>
          <div class="metric">
            <label>Recent Errors</label>
            <span x-text="currentBuildLoop.error_count"></span>
          </div>
        </div>
      </div>
    </div>
    "#.to_string()
}

/// Generate the fleet overview page
pub fn fleet_overview_page() -> String {
    r#"
    <div class="page-section" x-show="page === 'shipwright-fleet'" x-transition>
      <div class="page-header">
        <h1>Fleet Overview</h1>
        <p class="subtitle">Worker pool allocation, auto-scaling, per-repo metrics</p>
      </div>

      <div class="card">
        <div class="card-header">
          <h2>Worker Pool</h2>
        </div>
        <div class="worker-pool-view">
          <div class="metric-card">
            <div class="metric-label">Active Workers</div>
            <div class="metric-value" x-text="fleetStatus.active_workers"></div>
            <div class="metric-subtext" x-text="fleetStatus.max_workers + ' max'"></div>
          </div>
          <div class="metric-card">
            <div class="metric-label">Queued Issues</div>
            <div class="metric-value" x-text="fleetStatus.queued_issues"></div>
          </div>
          <div class="metric-card">
            <div class="metric-label">Active Pipelines</div>
            <div class="metric-value" x-text="fleetStatus.active_pipelines"></div>
          </div>
          <div class="metric-card">
            <div class="metric-label">CPU Usage</div>
            <div class="metric-value" x-text="fleetStatus.cpu_percent.toFixed(0) + '%'"></div>
          </div>
        </div>

        <div class="worker-visualization" style="margin-top: 1.5rem">
          <h3>Worker Allocation by Repo</h3>
          <template x-for="repo in fleetStatus.repos" :key="repo.path">
            <div class="repo-allocation">
              <span class="repo-name" x-text="repo.name"></span>
              <div class="allocation-bar">
                <div class="allocation-fill" :style="'width: ' + (repo.allocated_workers / fleetStatus.max_workers * 100) + '%'"></div>
              </div>
              <span class="allocation-count" x-text="repo.allocated_workers + ' / ' + repo.max_parallel"></span>
            </div>
          </template>
        </div>
      </div>

      <div class="card">
        <div class="card-header">
          <h2>Auto-Scaling Metrics</h2>
        </div>
        <div class="auto-scaling-view">
          <div class="metric">
            <label>CPU Available</label>
            <span x-text="(fleetStatus.cpu_available * 0.75).toFixed(1) + ' cores'"></span>
          </div>
          <div class="metric">
            <label>Memory Available</label>
            <span x-text="(fleetStatus.memory_gb_available / fleetStatus.worker_mem_gb).toFixed(0) + ' workers'"></span>
          </div>
          <div class="metric">
            <label>Daily Budget Remaining</label>
            <span x-text="'$' + fleetStatus.budget_remaining.toFixed(2)"></span>
          </div>
          <div class="metric">
            <label>Auto-Scaling</label>
            <span x-text="fleetStatus.auto_scaling_enabled ? 'Enabled' : 'Disabled'"></span>
          </div>
        </div>
      </div>
    </div>
    "#.to_string()
}

/// Generate the decisions log page
pub fn decisions_log_page() -> String {
    r#"
    <div class="page-section" x-show="page === 'shipwright-decisions'" x-transition>
      <div class="page-header">
        <h1>Decisions Log</h1>
        <p class="subtitle">Recent decisions, candidates, tier breakdown</p>
      </div>

      <div class="card">
        <div class="card-header">
          <h2>Active Candidates</h2>
          <button @click="runDecisionCycle()" class="btn-secondary">Run Cycle</button>
        </div>

        <table class="table">
          <thead>
            <tr>
              <th>Signal</th>
              <th>Title</th>
              <th>Score</th>
              <th>Breakdown</th>
              <th>Tier</th>
              <th>Status</th>
            </tr>
          </thead>
          <tbody>
            <template x-for="candidate in candidates" :key="candidate.id">
              <tr>
                <td x-text="candidate.signal"></td>
                <td x-text="candidate.title"></td>
                <td>
                  <span class="score-badge" x-text="candidate.score"></span>
                </td>
                <td>
                  <div class="score-breakdown">
                    <span class="breakdown-item" title="Impact">I: {{ candidate.breakdown.impact }}</span>
                    <span class="breakdown-item" title="Urgency">U: {{ candidate.breakdown.urgency }}</span>
                    <span class="breakdown-item" title="Effort">E: {{ candidate.breakdown.effort }}</span>
                    <span class="breakdown-item" title="Confidence">C: {{ candidate.breakdown.confidence }}</span>
                  </div>
                </td>
                <td>
                  <span class="tier-badge" :class="candidate.tier" x-text="candidate.tier"></span>
                </td>
                <td x-text="candidate.status"></td>
              </tr>
            </template>
          </tbody>
        </table>

        <div x-show="candidates.length === 0" class="empty-state">
          <p>No active candidates</p>
        </div>
      </div>

      <div class="card">
        <div class="card-header">
          <h2>Tier Breakdown (Last 30 Days)</h2>
        </div>
        <div class="tier-breakdown-chart">
          <template x-for="tier in tierBreakdown" :key="tier.name">
            <div class="tier-row">
              <span class="tier-name" x-text="tier.name"></span>
              <div class="tier-bar" :style="'width:' + tier.percentage + '%'"></div>
              <span class="tier-count" x-text="tier.count"></span>
            </div>
          </template>
        </div>
      </div>

      <div class="card">
        <div class="card-header">
          <h2>Recent Decisions</h2>
        </div>
        <div class="recent-decisions-list">
          <template x-for="decision in recentDecisions" :key="decision.id">
            <div class="decision-item">
              <div class="decision-header">
                <span x-text="decision.title"></span>
                <span class="decision-tier" :class="decision.tier" x-text="decision.tier"></span>
              </div>
              <p class="decision-reasoning" x-text="decision.reasoning"></p>
              <div class="decision-meta">
                <span x-text="decision.timestamp"></span>
                <span x-text="decision.outcome || 'pending'"></span>
              </div>
            </div>
          </template>
        </div>
      </div>
    </div>
    "#.to_string()
}

/// Generate the memory explorer page
pub fn memory_explorer_page() -> String {
    r#"
    <div class="page-section" x-show="page === 'shipwright-memory'" x-transition>
      <div class="page-header">
        <h1>Memory Explorer</h1>
        <p class="subtitle">Failure patterns, architecture rules, learning outcomes</p>
      </div>

      <div class="card">
        <div class="card-header">
          <h2>Search Failure Patterns</h2>
        </div>
        <div class="search-area">
          <input type="text" x-model="memorySearchQuery" placeholder="Search failure patterns..." @input="searchMemory()" class="search-input">
          <span class="search-results-count" x-text="memoryResults.length + ' patterns'"></span>
        </div>

        <div x-show="memoryResults.length > 0" class="memory-results">
          <template x-for="pattern in memoryResults" :key="pattern.id">
            <div class="pattern-card">
              <div class="pattern-header">
                <h3 x-text="pattern.error_signature"></h3>
                <span class="similarity-score" x-text="(pattern.similarity * 100).toFixed(0) + '%'"></span>
              </div>
              <div class="pattern-body">
                <div class="pattern-field">
                  <label>Root Cause</label>
                  <p x-text="pattern.root_cause"></p>
                </div>
                <div class="pattern-field">
                  <label>Fix Applied</label>
                  <p x-text="pattern.fix_applied"></p>
                </div>
                <div class="pattern-field">
                  <label>Recurrence</label>
                  <span x-text="pattern.recurrence_count + ' times'"></span>
                </div>
              </div>
            </div>
          </template>
        </div>

        <div x-show="memoryResults.length === 0 && memorySearchQuery" class="empty-state">
          <p>No matching failure patterns</p>
        </div>
      </div>

      <div class="card">
        <div class="card-header">
          <h2>Architecture Rules</h2>
        </div>
        <div class="architecture-rules">
          <template x-for="rule in architectureRules" :key="rule.id">
            <div class="rule-card">
              <h3 x-text="rule.name"></h3>
              <div class="rule-details">
                <div class="rule-layers">
                  <label>Layers:</label>
                  <span x-text="rule.layers.join(', ')"></span>
                </div>
                <div class="rule-violations" x-show="rule.violations.length > 0">
                  <label>Recent Violations:</label>
                  <ul>
                    <template x-for="violation in rule.violations.slice(0, 3)" :key="violation">
                      <li x-text="violation"></li>
                    </template>
                  </ul>
                </div>
              </div>
            </div>
          </template>
        </div>
      </div>

      <div class="card">
        <div class="card-header">
          <h2>Learning Outcomes</h2>
        </div>
        <div class="learning-outcomes">
          <div class="outcome-summary">
            <div class="outcome-metric">
              <label>Total Patterns Learned</label>
              <span x-text="learningOutcomes.total_patterns"></span>
            </div>
            <div class="outcome-metric">
              <label>Success Rate</label>
              <span x-text="(learningOutcomes.success_rate * 100).toFixed(1) + '%'"></span>
            </div>
            <div class="outcome-metric">
              <label>Weight Updates</label>
              <span x-text="learningOutcomes.weight_adjustments"></span>
            </div>
          </div>
        </div>
      </div>
    </div>
    "#.to_string()
}

/// Generate the intelligence dashboard page
pub fn intelligence_dashboard_page() -> String {
    r#"
    <div class="page-section" x-show="page === 'shipwright-intelligence'" x-transition>
      <div class="page-header">
        <h1>Intelligence Dashboard</h1>
        <p class="subtitle">DORA metrics, risk predictions, optimization suggestions</p>
      </div>

      <div class="card">
        <div class="card-header">
          <h2>DORA Metrics</h2>
        </div>
        <div class="dora-metrics">
          <div class="metric-card dora-card">
            <div class="metric-label">Lead Time for Changes</div>
            <div class="metric-value" x-text="doraMetrics.lead_time_hours.toFixed(1) + 'h'"></div>
            <div class="metric-subtext" x-text="doraMetrics.lead_time_trend"></div>
          </div>
          <div class="metric-card dora-card">
            <div class="metric-label">Deployment Frequency</div>
            <div class="metric-value" x-text="doraMetrics.deploy_frequency_per_day.toFixed(2) + '/d'"></div>
            <div class="metric-subtext" x-text="doraMetrics.deploy_frequency_trend"></div>
          </div>
          <div class="metric-card dora-card">
            <div class="metric-label">Change Failure Rate</div>
            <div class="metric-value" x-text="(doraMetrics.change_failure_rate * 100).toFixed(1) + '%'"></div>
            <div class="metric-subtext" x-text="doraMetrics.cfr_trend"></div>
          </div>
          <div class="metric-card dora-card">
            <div class="metric-label">MTTR</div>
            <div class="metric-value" x-text="doraMetrics.mttr_minutes.toFixed(0) + 'm'"></div>
            <div class="metric-subtext" x-text="doraMetrics.mttr_trend"></div>
          </div>
        </div>

        <div class="dora-level-badge" :class="doraMetrics.level" x-text="doraMetrics.level + ' Performer'"></div>
      </div>

      <div class="card">
        <div class="card-header">
          <h2>Risk Predictions</h2>
        </div>
        <div class="risk-heatmap">
          <h3>File Risk Scores (Next 30 Days)</h3>
          <template x-for="file in riskPredictions.files" :key="file.path">
            <div class="risk-row">
              <span class="file-path" x-text="file.path"></span>
              <div class="risk-bar" :style="'background: linear-gradient(to right, #4ade80, #eab308, #ef4444); width: 200px; height: 20px; border-radius: 4px'">
                <div class="risk-fill" :style="'width: ' + file.risk_score + '%; height: 100%; background: rgba(0,0,0,0.1)'"></div>
              </div>
              <span class="risk-score" x-text="file.risk_score.toFixed(0)"></span>
            </div>
          </template>
        </div>

        <div class="anomalies" style="margin-top: 1.5rem">
          <h3>Detected Anomalies</h3>
          <template x-for="anomaly in riskPredictions.anomalies" :key="anomaly.id">
            <div class="anomaly-alert" :class="anomaly.severity">
              <span class="anomaly-label" x-text="anomaly.label"></span>
              <span class="anomaly-value" x-text="anomaly.message"></span>
            </div>
          </template>
          <div x-show="riskPredictions.anomalies.length === 0" class="empty-state">
            <p>No anomalies detected</p>
          </div>
        </div>
      </div>

      <div class="card">
        <div class="card-header">
          <h2>Optimization Suggestions</h2>
        </div>
        <div class="suggestions-list">
          <template x-for="suggestion in optimizationSuggestions" :key="suggestion.id">
            <div class="suggestion-card" :class="suggestion.priority">
              <div class="suggestion-header">
                <h3 x-text="suggestion.title"></h3>
                <span class="priority-badge" :class="suggestion.priority" x-text="suggestion.priority"></span>
              </div>
              <p class="suggestion-description" x-text="suggestion.description"></p>
              <div class="suggestion-impact">
                <span>Est. Impact: <strong x-text="suggestion.estimated_impact + '%'"></strong></span>
              </div>
            </div>
          </template>
        </div>
      </div>
    </div>
    "#.to_string()
}

/// Generate the cost tracker page
pub fn cost_tracker_page() -> String {
    r#"
    <div class="page-section" x-show="page === 'shipwright-cost'" x-transition>
      <div class="page-header">
        <h1>Cost Tracker</h1>
        <p class="subtitle">Token usage, budget remaining, per-pipeline costs</p>
      </div>

      <div class="card">
        <div class="card-header">
          <h2>Daily Budget Status</h2>
        </div>
        <div class="budget-overview">
          <div class="metric-card">
            <div class="metric-label">Daily Limit</div>
            <div class="metric-value" x-text="'$' + costData.daily_limit.toFixed(2)"></div>
          </div>
          <div class="metric-card">
            <div class="metric-label">Spent Today</div>
            <div class="metric-value" x-text="'$' + costData.spent_today.toFixed(2)"></div>
          </div>
          <div class="metric-card">
            <div class="metric-label">Remaining</div>
            <div class="metric-value" x-text="'$' + costData.remaining.toFixed(2)"></div>
          </div>
          <div class="metric-card">
            <div class="metric-label">Burn Rate</div>
            <div class="metric-value" x-text="'$' + costData.burn_rate.toFixed(2) + '/hr'"></div>
          </div>
        </div>

        <div class="budget-progress">
          <div class="budget-bar">
            <div class="budget-fill" :style="'width: ' + (costData.spent_today / costData.daily_limit * 100) + '%'"></div>
          </div>
          <p class="budget-status" x-text="(costData.spent_today / costData.daily_limit * 100).toFixed(1) + '% of daily budget used'"></p>
        </div>
      </div>

      <div class="card">
        <div class="card-header">
          <h2>Token Usage Summary</h2>
        </div>
        <div class="token-usage">
          <div class="usage-row">
            <span>Input Tokens</span>
            <span x-text="costData.input_tokens.toLocaleString()"></span>
            <span x-text="'$' + costData.input_cost.toFixed(4)"></span>
          </div>
          <div class="usage-row">
            <span>Output Tokens</span>
            <span x-text="costData.output_tokens.toLocaleString()"></span>
            <span x-text="'$' + costData.output_cost.toFixed(4)"></span>
          </div>
          <div class="usage-row total">
            <span><strong>Total Tokens</strong></span>
            <span x-text="(costData.input_tokens + costData.output_tokens).toLocaleString()"></span>
            <span x-text="'$' + (costData.input_cost + costData.output_cost).toFixed(2)"></span>
          </div>
        </div>
      </div>

      <div class="card">
        <div class="card-header">
          <h2>Cost by Pipeline</h2>
        </div>
        <table class="table">
          <thead>
            <tr>
              <th>Pipeline ID</th>
              <th>Issue</th>
              <th>Input Tokens</th>
              <th>Output Tokens</th>
              <th>Cost</th>
              <th>Duration</th>
            </tr>
          </thead>
          <tbody>
            <template x-for="pipeline in costData.pipelines" :key="pipeline.id">
              <tr>
                <td><code x-text="pipeline.id"></code></td>
                <td x-text="'#' + pipeline.issue"></td>
                <td x-text="pipeline.input_tokens.toLocaleString()"></td>
                <td x-text="pipeline.output_tokens.toLocaleString()"></td>
                <td x-text="'$' + pipeline.cost.toFixed(2)"></td>
                <td x-text="pipeline.duration_minutes + ' min'"></td>
              </tr>
            </template>
          </tbody>
        </table>
      </div>

      <div class="card">
        <div class="card-header">
          <h2>Cost Trends</h2>
        </div>
        <div class="cost-trend-chart">
          <p style="color: var(--text-dim); font-size: 0.85rem">Daily costs over last 30 days</p>
          <div class="chart-placeholder">
            <p>Cost trend visualization</p>
          </div>
        </div>
      </div>
    </div>
    "#.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pipeline_status_page_contains_required_elements() {
        let html = pipeline_status_page();
        assert!(html.contains("Pipeline Status"));
        assert!(html.contains("Active Pipelines"));
        assert!(html.contains("Current Build Loop"));
        assert!(html.contains("shipwright-pipelines"));
        assert!(html.contains("stage-progress"));
    }

    #[test]
    fn test_fleet_overview_page_contains_required_elements() {
        let html = fleet_overview_page();
        assert!(html.contains("Fleet Overview"));
        assert!(html.contains("Worker Pool"));
        assert!(html.contains("Auto-Scaling Metrics"));
        assert!(html.contains("shipwright-fleet"));
        assert!(html.contains("repo-allocation"));
    }

    #[test]
    fn test_decisions_log_page_contains_required_elements() {
        let html = decisions_log_page();
        assert!(html.contains("Decisions Log"));
        assert!(html.contains("Active Candidates"));
        assert!(html.contains("Tier Breakdown"));
        assert!(html.contains("Recent Decisions"));
        assert!(html.contains("shipwright-decisions"));
        assert!(html.contains("score-breakdown"));
    }

    #[test]
    fn test_memory_explorer_page_contains_required_elements() {
        let html = memory_explorer_page();
        assert!(html.contains("Memory Explorer"));
        assert!(html.contains("Failure Patterns"));
        assert!(html.contains("Architecture Rules"));
        assert!(html.contains("Learning Outcomes"));
        assert!(html.contains("shipwright-memory"));
        assert!(html.contains("similarity-score"));
    }

    #[test]
    fn test_intelligence_dashboard_page_contains_required_elements() {
        let html = intelligence_dashboard_page();
        assert!(html.contains("Intelligence Dashboard"));
        assert!(html.contains("DORA Metrics"));
        assert!(html.contains("Risk Predictions"));
        assert!(html.contains("Optimization Suggestions"));
        assert!(html.contains("Lead Time for Changes"));
        assert!(html.contains("Deployment Frequency"));
    }

    #[test]
    fn test_cost_tracker_page_contains_required_elements() {
        let html = cost_tracker_page();
        assert!(html.contains("Cost Tracker"));
        assert!(html.contains("Daily Budget Status"));
        assert!(html.contains("Token Usage Summary"));
        assert!(html.contains("Cost by Pipeline"));
        assert!(html.contains("Cost Trends"));
        assert!(html.contains("shipwright-cost"));
    }

    #[test]
    fn test_all_pages_return_valid_html() {
        let pages = vec![
            pipeline_status_page(),
            fleet_overview_page(),
            decisions_log_page(),
            memory_explorer_page(),
            intelligence_dashboard_page(),
            cost_tracker_page(),
        ];

        for page in pages {
            // Each page should have at least one div with page-section
            assert!(page.contains("page-section"));
            // Each page should be valid Alpine.js template
            assert!(page.contains("x-show"));
        }
    }

    #[test]
    fn test_all_pages_have_unique_identifiers() {
        let pages = vec![
            ("shipwright-pipelines", pipeline_status_page()),
            ("shipwright-fleet", fleet_overview_page()),
            ("shipwright-decisions", decisions_log_page()),
            ("shipwright-memory", memory_explorer_page()),
            ("shipwright-intelligence", intelligence_dashboard_page()),
            ("shipwright-cost", cost_tracker_page()),
        ];

        for (expected_id, page) in pages {
            assert!(
                page.contains(expected_id),
                "Page should contain identifier: {}",
                expected_id
            );
        }
    }

    #[test]
    fn test_pages_contain_alpine_data_bindings() {
        let pages = vec![
            pipeline_status_page(),
            fleet_overview_page(),
            decisions_log_page(),
            memory_explorer_page(),
            intelligence_dashboard_page(),
            cost_tracker_page(),
        ];

        for page in pages {
            // Should use Alpine.js directives
            assert!(page.contains("x-for") || page.contains("x-show") || page.contains("x-model"));
        }
    }
}
