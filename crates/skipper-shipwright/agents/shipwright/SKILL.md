# Shipwright Hand — Domain Expertise

## Overview

Shipwright is an autonomous software delivery agent. It transforms GitHub issues into pull requests and production deployments using a 12-stage pipeline with self-healing build loops, intelligent quality checks, and outcome-driven learning.

This document covers the technical conventions, best practices, and domain patterns that make Shipwright effective.

---

## Pipeline Architecture

### The 12 Stages

1. **Intake** — Parse issue, extract requirements, complexity scoring
2. **Plan** — Decompose into subtasks, architecture analysis, resource estimation
3. **Design** — High-level approach, API contracts, data model changes
4. **Build** — Code implementation with iterative refinement loop
5. **Test** — Execute test suite, capture failures, coverage analysis
6. **Review** — Architecture check, security scan, code style, performance
7. **Compound Quality** — Adversarial review, edge case discovery, regression check
8. **PR** — Create pull request, select reviewers, write description
9. **Merge** — Wait for approvals, auto-merge if enabled, close issue
10. **Deploy** — Create GitHub Deployment, run CI/CD, track metrics
11. **Validate** — Verify staging/production health, error rate, latency
12. **Monitor** — Track metrics over 24h, detect anomalies, record outcome

### Stage Gates & Approvals

Each stage has a gate:

- **auto**: proceed immediately when stage completes
- **propose**: emit a candidate but wait for human approval before advancing
- **manual**: always pause and require explicit user approval

The `PipelineTemplate` configuration controls which gates are auto vs manual.

---

## Build Loop Patterns

The build loop (Stage 4) is where code is generated and tests run. It's iterative and self-healing.

### Convergence Detection

After each test run, measure:

- **Error count**: number of test failures
- **Failure types**: group by error category

Convergence metric: `(error_count_prev - error_count_now) / error_count_prev`

- Convergence > 50% → we're making progress, keep iterating
- Convergence < 10% → plateau, consider different approach
- Divergence (negative) → we're making things worse, backtrack

### Backtracking & Restart

When diverging:

1. Save the current commit
2. `git checkout` the last good state
3. Try a different approach (different error pattern from memory)
4. Restart the test loop

If backtracking happens 2+ times → escalate to Stage 7 (Compound Quality) for adversarial review.

### Max Iterations

Default: 5-10 iterations per template (configurable).

If tests still don't pass:

- Create the PR anyway with test failure notes
- Add a comment: "Build loop exhausted after N iterations. See failed tests in the PR."
- Assign to code review for human intervention

### Session Restarts

If a build loop exhausts iterations:

1. Save `progress.md` with current state, recent commits, changed files
2. Restart a fresh Claude session
3. New session reads `progress.md` and resumes from the last good commit
4. This avoids context exhaustion on complex issues

---

## GitHub Workflow Conventions

### PR Description Format

```markdown
## Issue

Closes #123

## What Changed

- [List of code changes]
- [Impact on other modules]

## Acceptance Criteria ✅

- [ ] Requirement A addressed
- [ ] Requirement B addressed
- [ ] Tests passing (X/Y)
- [ ] No regressions in related modules
- [ ] Performance impact assessed
- [ ] Documentation updated

## Test Results

- Unit tests: N passed, 0 failed
- Integration tests: N passed, 0 failed
- Coverage: X%

## Architecture Notes

- Changes respect layer boundaries [if applicable]
- No circular dependencies introduced [if applicable]
- Uses existing patterns from [if applicable]

## Review Checklist

- Code style: follows project conventions
- Security: no secrets, no dangerous patterns
- Performance: no O(n²) loops, caching appropriate
- Tests: adequate coverage for changes
```

### Reviewer Selection

Priority order:

1. CODEOWNERS (read from file if exists)
2. Top 3 contributors to affected files (from GitHub GraphQL)
3. Last N reviewers who touched these files
4. Fallback: request from team/org leads

Limit to 3 reviewers max (too many is less effective).

### Label Semantics

Shipwright watches for labels:

- `shipwright` — enable pipeline for this issue
- `ready-to-build` — explicitly queue for building
- `priority-critical` → fast template + no review gates
- `priority-low` → standard template + extra review cycles
- `architecture-review` → route to specialist reviewer
- `security-review` → route to security-focused reviewer
- `shipwright:proposed` → human approval required before dispatch

---

## Test Strategy

### Fast Tests vs Full Suite

Template controls test strategy:

- **fast** template: Run only fast unit tests (< 5s)
  - Used for simple style/doc changes
  - Skips integration and e2e tests
- **standard**: Run full suite on every iteration
- **full**: Run full suite + coverage report + regression checks
- **autonomous**: Same as full, but never auto-passes (highest bar)

Switch strategy mid-loop with `--fast-test-interval N`:

- Run fast tests every iteration
- Run full suite every N iterations (default 5)
- Final iteration always runs full suite

### Coverage Requirements

- New code must have ≥ 80% line coverage
- Core modules (decision, pipeline, fleet) must have ≥ 90% coverage
- Don't decrease overall coverage percentage

If coverage drops:

1. Analyze which lines aren't covered
2. Add tests for critical paths
3. Suppress coverage for boilerplate/imports with `#[cfg(test)]`

### Test Failure Analysis

When tests fail:

1. Parse error output for error type + location
2. Search memory for similar failures + their fixes
3. Apply learned fix if found
4. If new error: generate minimal test case that reproduces it
5. Update code to pass that test
6. Re-run full suite to confirm no regressions

### Flaky Test Handling

If a test fails intermittently:

1. Re-run it 3 times to confirm it's flaky
2. Mark with `#[ignore]` and add a TODO comment
3. File an issue to fix the flakiness
4. Document the condition that causes flakiness

Don't try to fix flaky tests on the fly — escalate to human.

---

## Memory System Usage

### Failure Pattern Lookup

When tests fail, search memory:

```
memory_search("error message or symptom")
→ returns: [FailurePattern { root_cause, fix, confidence }, ...]
```

Each pattern has:

- **root_cause**: explanation of why this happens
- **fix**: code change or approach that resolved it
- **confidence**: 0.0-1.0, how often this fix works
- **language**: which language/framework it applies to
- **date_first_seen**: when we first encountered this pattern

### Architecture Rules

Load from memory:

- Layer definitions (API, service, data, external)
- Dependency rules ("api cannot import from data directly")
- Naming conventions (prefix patterns, module organization)
- Hotspots (frequently-changing files, risky areas)

Check PR against rules:

```
memory_search("architecture rules for [repo_name]")
→ returns: ArchitectureModel { layers, rules, hotspots }
→ validate changes against rules
```

### Learning Outcomes

After deployment, record:

```rust
Outcome {
    issue_id: "123",
    success: true,
    effort_actual_hours: 4.5,
    quality_score: 92,  // 0-100
    lead_time_hours: 2.3,
}
```

This trains the model to:

- Score future issues more accurately
- Adjust template selection (if success rate drops for "fast", shift to "standard")
- Recognize patterns that correlate with success/failure

---

## DORA Metrics Interpretation

Shipwright tracks four DORA metrics:

### Lead Time (Hours from First Commit to Deploy)

- **Elite**: < 1 hour
- **High**: 1–24 hours
- **Medium**: 1–7 days
- **Low**: > 1 month

**To improve**:

- Split complex issues into smaller PRs
- Automate more review steps
- Reduce deployment manual steps
- Improve test speed (parallel execution, fast subset)

### Deployment Frequency (Deploys Per Day)

- **Elite**: on-demand (> 1 per day)
- **High**: weekly
- **Medium**: monthly
- **Low**: < monthly

**To improve**:

- Smaller features, more frequent releases
- Automated CI/CD pipeline
- Feature flags for in-progress work
- Reduce approval cycles

### Change Failure Rate (% of Deploys That Fail)

- **Elite**: 0–15%
- **High**: 0–15%
- **Medium**: 15–45%
- **Low**: > 45%

**To improve**:

- Increase test coverage (especially high-risk areas)
- Use canary deployments
- Monitor in staging before prod
- Better architecture validation

### Mean Time to Recovery (Hours from Failure to Fix)

- **Elite**: < 1 hour
- **High**: < 1 day
- **Medium**: < 1 week
- **Low**: > 1 week

**To improve**:

- Faster rollback procedures
- Quick diagnosis tools (logs, metrics, traces)
- On-call rotation for fast response
- Automated health checks that detect failures early

---

## Intelligence Layer Patterns

### Risk Prediction

When prioritizing issues, Shipwright predicts risk (0–100):

Factors:

- **File hotspots**: files that change frequently → higher risk
- **Complexity**: cyclomatic complexity of changed functions
- **Test coverage**: lines without test → higher risk
- **Dependencies**: how many modules import from changed file
- **Author expertise**: who historically changes this code?

**Risk scores**:

- 0–30: low risk (style, docs, tests)
- 30–60: medium risk (features, bug fixes in stable code)
- 60–80: high risk (changes to core, multiple hotspots)
- 80–100: critical risk (security-sensitive, deployment infrastructure)

### Anomaly Detection

Each metric (lead time, CFR, MTTR) is tracked over a rolling window (30/90 days).

Anomaly = value > (mean + 3\*stdev)

When anomaly detected:

- Alert: "Deploy frequency anomaly: 0 deploys today vs 2.3 avg"
- Investigate: What changed? (config, team, scope of issues)
- Action: Escalate to human or trigger diagnostic run

### Self-Optimization

Every 7 days, Shipwright analyzes DORA metrics and suggests config changes:

```
DORA Metrics Last 7 Days:
- lead_time: 2.5h (UP from 1.8h avg)
  → Suggestion: increase max_workers from 2 to 3
  → Or: split large issues into smaller ones

- deploy_frequency: 1.2 per day (DOWN from 2.1)
  → Suggestion: reduce max_issues_per_day check (budget okay)
  → Or: increase build loop max_iterations (issues harder?)

- change_failure_rate: 22% (STEADY)
  → Suggestion: none, within normal range
```

Apply suggestion with `--auto-optimize` if enabled.

---

## Decision Engine Integration

### Signal Collectors

Shipwright's decision engine collects signals from multiple sources:

1. **Security**: npm audit, cargo audit, bandit → CVE candidates
2. **Dependencies**: outdated transitive deps → upgrade candidates
3. **Coverage**: files below threshold → coverage improvement candidates
4. **Dead Code**: unreachable code, unused imports → cleanup candidates
5. **DORA Regression**: metrics declining → investigation candidates
6. **Hand Signals**: Cross-pollination from Collector, Researcher, Predictor Hands

Each signal produces a `Candidate` with:

- `impact`: how important is this? (1–10)
- `urgency`: how soon does it need fixing? (1–10)
- `effort`: how hard is it? (1–10)
- `confidence`: how sure are we? (0.0–1.0)
- `risk`: how risky is the fix? (1–10)

### Scoring Formula

`score = (impact * 0.30) + (urgency * 0.25) + (effort * 0.20) + (confidence * 0.15) - (risk * 0.10)`

Scores 0–100. High scores = high priority.

### Autonomy Tiers

After scoring, resolve the autonomy tier:

- **Auto** (score > 80, low risk): Automatically create issue and queue for building
- **Propose** (score > 50): Create draft issue, wait for human approval
- **Draft** (score > 30): Create issue in draft state, human explicitly clicks "build"
- **Ignore** (score < 30): Don't create issue, but log for analysis

Tier is also configurable by category (security always → Auto, refactor always → Draft).

### Budget & Rate Limiting

Decision engine respects limits:

- `max_issues_per_day`: never process more than N issues in 24h
- `max_cost_per_day`: never spend more than $X on builds
- `cooldown_seconds`: don't process the same signal twice within N seconds
- `halt`: if true, stop processing all signals until `resume()` called

---

## Common Patterns

### Handling Test Flakiness

Sometimes tests pass/fail randomly. Strategy:

1. Detect flakiness: same test fails on run 1, passes on run 2
2. Mark with `#[ignore]` and note the flakiness condition
3. File an issue for human to fix
4. Continue with rest of test suite

**Don't try to fix flakiness algorithmically** — it usually indicates a real environmental issue.

### Circular Dependency Detection

When validating architecture:

```
Check for cycles in import graph:
  crate_a imports crate_b
  crate_b imports crate_a  ← CYCLE!
```

If found, reject the PR with:

```
Circular dependency introduced:
  decision → pipeline → decision

Fix: Move shared types to common module
```

### Hotspot Analysis

Files that change frequently are risky. Track with:

```rust
Hotspot {
    path: "src/lib.rs",
    changes_last_30d: 45,
    test_coverage: 82,
    risk_score: 78,  // high because frequent + complex
}
```

When a hotspot is changed:

- Require extra test coverage (target 90%+)
- Route to most experienced reviewers
- Run full test suite, not fast subset
- Consider splitting change into smaller PRs

---

## Error Recovery Examples

### Example 1: Tests Fail Due to Missing File

```
Error: FileNotFoundError: config.yaml
```

**Recovery**:

1. Analyze: some code reads a required config file
2. Check if other parts of codebase handle missing files gracefully
3. Add default config or check for existence before reading
4. Re-run tests

### Example 2: Coverage Regression

```
Coverage dropped from 85% to 79%
Changes: src/cache.rs added 240 lines, only 160 covered
```

**Recovery**:

1. Analyze which 80 lines aren't covered
2. Write tests for the uncovered code paths
3. Focus on critical/error paths first
4. Re-run coverage check

### Example 3: Flaky Test

```
Test passed: 2/3 runs
Test failed: 1/3 runs (timing-dependent)
```

**Recovery**:

1. Mark `#[ignore]` with explanation
2. File issue: "Fix flaky test in concurrent_queue_tests"
3. Continue with rest of suite
4. Don't spend iterations trying to fix it

---

## Best Practices

1. **Small, focused PRs**: One issue per PR, easier to review/revert
2. **Descriptive commit messages**: Helps future debugging via `git log`
3. **Test critical paths**: Don't rely on coverage % alone
4. **Respect architecture**: Layers exist for a reason
5. **Prefer boring code**: Maintainable > clever
6. **Document why not what**: Code shows what, PR shows why
7. **Fail fast**: Check for obvious issues early (linting, types)
8. **Learn from outcomes**: Record failures, improve over time

---

## When Things Go Wrong

If pipeline stalls:

1. Check memory for similar issues
2. Review recent commits in the repo
3. Check if tests themselves are broken
4. Escalate to human review if max iterations exceeded
5. Document the failure for learning

If auto-merge fails:

1. Check if all reviews are approved
2. Check if required status checks passed
3. Check if branch has conflicts
4. Create PR but don't merge, add comment explaining why

Never fabricate test results or skip actual testing.
