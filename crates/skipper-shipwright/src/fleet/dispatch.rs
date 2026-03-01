//! Worker pool and job dispatch.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Worker pool configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerPool {
    pub total_workers: u32,
    pub min_per_repo: u32,
    pub auto_scale: bool,
    pub max_workers: u32,
    pub min_workers: u32,
    pub worker_mem_gb: u32,
    pub cost_per_job_usd: f64,
}

impl Default for WorkerPool {
    fn default() -> Self {
        Self {
            total_workers: 4,
            min_per_repo: 1,
            auto_scale: false,
            max_workers: 8,
            min_workers: 1,
            worker_mem_gb: 4,
            cost_per_job_usd: 5.0,
        }
    }
}

impl WorkerPool {
    /// Check if pool has capacity for a repo.
    pub fn has_capacity(&self, allocated: u32) -> bool {
        allocated < self.total_workers
    }

    /// Get available worker count.
    pub fn available_workers(&self, allocated: u32) -> u32 {
        self.total_workers.saturating_sub(allocated)
    }

    /// Validate the pool configuration.
    pub fn validate(&self) -> Result<(), String> {
        if self.worker_mem_gb == 0 {
            return Err("worker_mem_gb must be greater than 0".to_string());
        }
        if self.min_workers > self.max_workers {
            return Err(format!(
                "min_workers ({}) must be <= max_workers ({})",
                self.min_workers, self.max_workers
            ));
        }
        Ok(())
    }

    /// Suggest scaled worker count based on system metrics.
    pub fn suggest_scaled_workers(
        &self,
        cpu_cores: u32,
        available_mem_gb: u32,
        remaining_budget_usd: f64,
    ) -> Result<u32, String> {
        // Validate config before calculations
        self.validate()?;

        if !self.auto_scale {
            return Ok(self.total_workers);
        }

        let cpu_based = (cpu_cores as f64 * 0.75) as u32;
        // Safe division: validated that worker_mem_gb > 0
        let mem_based = available_mem_gb / self.worker_mem_gb;
        let budget_based = ((remaining_budget_usd / self.cost_per_job_usd).floor() as u32).max(1);

        let suggested = cpu_based.min(mem_based).min(budget_based);
        // Safe clamp: validated that min_workers <= max_workers
        Ok(suggested.clamp(self.min_workers, self.max_workers))
    }
}

/// Job claim for a worker.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobClaim {
    pub job_id: String,
    pub repo: String,
    pub worker_id: String,
    pub claimed_at: u64,
}

impl JobClaim {
    /// Create a new job claim.
    pub fn new(job_id: String, repo: String, worker_id: String) -> Self {
        Self {
            job_id,
            repo,
            worker_id,
            claimed_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        }
    }
}

/// Dispatcher for managing job allocation.
#[derive(Debug, Clone)]
pub struct Dispatcher {
    pub pool: WorkerPool,
    pub allocated_per_repo: HashMap<String, u32>,
    pub active_claims: HashMap<String, JobClaim>,
}

impl Dispatcher {
    /// Create a new dispatcher.
    pub fn new(pool: WorkerPool) -> Self {
        Self {
            pool,
            allocated_per_repo: HashMap::new(),
            active_claims: HashMap::new(),
        }
    }

    /// Claim a worker for a job.
    pub fn claim(&mut self, repo: &str, job_id: &str) -> Result<String, String> {
        let current_allocated = self.allocated_per_repo.get(repo).copied().unwrap_or(0);
        let current_total: u32 = self.allocated_per_repo.values().sum();

        if current_total >= self.pool.total_workers {
            return Err("No available workers in pool".to_string());
        }

        let worker_id = uuid::Uuid::new_v4().to_string();
        self.allocated_per_repo.insert(repo.to_string(), current_allocated + 1);

        let claim = JobClaim::new(job_id.to_string(), repo.to_string(), worker_id.clone());
        self.active_claims.insert(job_id.to_string(), claim);

        Ok(worker_id)
    }

    /// Release a worker claim.
    pub fn release(&mut self, job_id: &str) -> Result<(), String> {
        if let Some(claim) = self.active_claims.remove(job_id) {
            if let Some(current) = self.allocated_per_repo.get_mut(&claim.repo) {
                *current = current.saturating_sub(1);
            }
            Ok(())
        } else {
            Err(format!("Job {} not found", job_id))
        }
    }

    /// Get allocated count for a repo.
    pub fn allocated_for_repo(&self, repo: &str) -> u32 {
        self.allocated_per_repo.get(repo).copied().unwrap_or(0)
    }

    /// Rebalance workers proportionally to queue depth.
    /// - Calculate total queue depth across all repos
    /// - Distribute workers proportionally: workers_for_repo = max(min_per_repo, total_workers * repo_depth / total_depth)
    /// - Respect pool.total_workers ceiling
    /// - Repos with 0 queue depth still get min_per_repo if they already have active claims
    /// - Only count repos with non-zero depth for proportional distribution
    pub fn rebalance(&mut self, queue_depths: &HashMap<String, u32>) {
        // Calculate total queue depth (only from repos with non-zero depth)
        let total_depth: u32 = queue_depths.values().filter(|&&d| d > 0).sum();

        if total_depth == 0 {
            // No work queued, nothing to rebalance
            return;
        }

        // New allocation map
        let mut new_allocation: HashMap<String, u32> = HashMap::new();

        // Proportional distribution
        for (repo, depth) in queue_depths {
            if *depth > 0 {
                // workers_for_repo = max(min_per_repo, (total_workers * repo_depth) / total_depth)
                let proportional = (self.pool.total_workers * depth) / total_depth;
                let allocated = proportional.max(self.pool.min_per_repo);
                new_allocation.insert(repo.clone(), allocated);
            } else if self.allocated_per_repo.contains_key(repo) {
                // Repos with 0 depth but existing claims keep min_per_repo
                new_allocation.insert(repo.clone(), self.pool.min_per_repo);
            }
        }

        // Enforce total ceiling: adjust down proportionally if exceeds total_workers
        let total_allocated: u32 = new_allocation.values().sum();
        if total_allocated > self.pool.total_workers {
            let scale_factor = self.pool.total_workers as f64 / total_allocated as f64;
            for workers in new_allocation.values_mut() {
                *workers = ((*workers as f64 * scale_factor).floor() as u32).max(1);
            }
        }

        self.allocated_per_repo = new_allocation;
    }
}

impl Default for Dispatcher {
    fn default() -> Self {
        Self::new(WorkerPool::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_worker_pool_default() {
        let pool = WorkerPool::default();
        assert_eq!(pool.total_workers, 4);
        assert!(!pool.auto_scale);
    }

    #[test]
    fn test_has_capacity() {
        let pool = WorkerPool::default();
        assert!(pool.has_capacity(3));
        assert!(!pool.has_capacity(4));
    }

    #[test]
    fn test_available_workers() {
        let pool = WorkerPool::default();
        assert_eq!(pool.available_workers(1), 3);
        assert_eq!(pool.available_workers(4), 0);
    }

    #[test]
    fn test_suggest_scaled_workers_disabled() {
        let pool = WorkerPool::default();
        let suggested = pool.suggest_scaled_workers(8, 32, 100.0).unwrap();
        assert_eq!(suggested, pool.total_workers);
    }

    #[test]
    fn test_suggest_scaled_workers_cpu_limited() {
        let pool = WorkerPool { auto_scale: true, ..Default::default() };
        let suggested = pool.suggest_scaled_workers(8, 32, 100.0).unwrap();
        assert_eq!(suggested, 6);
    }

    #[test]
    fn test_suggest_scaled_workers_memory_limited() {
        let pool = WorkerPool { auto_scale: true, worker_mem_gb: 4, ..Default::default() };
        let suggested = pool.suggest_scaled_workers(8, 8, 100.0).unwrap();
        assert_eq!(suggested, 2);
    }

    #[test]
    fn test_suggest_scaled_workers_budget_limited() {
        let pool = WorkerPool { auto_scale: true, cost_per_job_usd: 5.0, ..Default::default() };
        let suggested = pool.suggest_scaled_workers(8, 32, 10.0).unwrap();
        assert_eq!(suggested, 2);
    }

    #[test]
    fn test_worker_pool_validation_success() {
        let pool = WorkerPool::default();
        assert!(pool.validate().is_ok());
    }

    #[test]
    fn test_worker_pool_validation_zero_mem_gb() {
        let pool = WorkerPool { worker_mem_gb: 0, ..Default::default() };
        assert!(pool.validate().is_err());
    }

    #[test]
    fn test_worker_pool_validation_min_greater_than_max() {
        let pool = WorkerPool { min_workers: 10, max_workers: 5, ..Default::default() };
        assert!(pool.validate().is_err());
    }

    #[test]
    fn test_suggest_scaled_workers_rejects_zero_mem_gb() {
        let pool = WorkerPool { auto_scale: true, worker_mem_gb: 0, ..Default::default() };
        assert!(pool.suggest_scaled_workers(8, 32, 100.0).is_err());
    }

    #[test]
    fn test_suggest_scaled_workers_rejects_invalid_min_max() {
        let pool = WorkerPool { auto_scale: true, min_workers: 10, max_workers: 5, ..Default::default() };
        assert!(pool.suggest_scaled_workers(8, 32, 100.0).is_err());
    }

    #[test]
    fn test_job_claim_new() {
        let claim = JobClaim::new("job1".to_string(), "repo1".to_string(), "worker1".to_string());
        assert_eq!(claim.job_id, "job1");
        assert_eq!(claim.repo, "repo1");
        assert!(claim.claimed_at > 0);
    }

    #[test]
    fn test_dispatcher_new() {
        let dispatcher = Dispatcher::new(WorkerPool::default());
        assert_eq!(dispatcher.pool.total_workers, 4);
        assert!(dispatcher.active_claims.is_empty());
    }

    #[test]
    fn test_dispatcher_claim() {
        let mut dispatcher = Dispatcher::new(WorkerPool::default());
        let result = dispatcher.claim("repo1", "job1");
        assert!(result.is_ok());
        assert_eq!(dispatcher.allocated_for_repo("repo1"), 1);
        assert_eq!(dispatcher.active_claims.len(), 1);
    }

    #[test]
    fn test_dispatcher_claim_exceeds_capacity() {
        let mut dispatcher = Dispatcher::new(WorkerPool::default());
        for i in 0..4 {
            let _ = dispatcher.claim("repo1", &format!("job{}", i));
        }
        let result = dispatcher.claim("repo1", "job4");
        assert!(result.is_err());
    }

    #[test]
    fn test_dispatcher_release() {
        let mut dispatcher = Dispatcher::new(WorkerPool::default());
        let _ = dispatcher.claim("repo1", "job1");
        assert_eq!(dispatcher.allocated_for_repo("repo1"), 1);

        let result = dispatcher.release("job1");
        assert!(result.is_ok());
        assert_eq!(dispatcher.allocated_for_repo("repo1"), 0);
    }

    #[test]
    fn test_dispatcher_release_not_found() {
        let mut dispatcher = Dispatcher::new(WorkerPool::default());
        let result = dispatcher.release("nonexistent");
        assert!(result.is_err());
    }

    #[test]
    fn test_dispatcher_rebalance() {
        let mut dispatcher = Dispatcher::new(WorkerPool::default());
        let mut queue_depths = HashMap::new();
        queue_depths.insert("repo1".to_string(), 5);
        queue_depths.insert("repo2".to_string(), 3);

        dispatcher.rebalance(&queue_depths);
        assert!(dispatcher.allocated_for_repo("repo1") > 0);
        assert!(dispatcher.allocated_for_repo("repo2") > 0);
    }

    #[test]
    fn test_dispatcher_default() {
        let dispatcher = Dispatcher::default();
        assert_eq!(dispatcher.pool.total_workers, 4);
    }
}
