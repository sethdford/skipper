// Fleet page — displays Shipwright fleet status and worker pool utilization
Alpine.data("fleetPage", () => ({
  fleetStatus: {
    status: "operational",
    active_pipelines: 0,
    completed_pipelines: 0,
    failed_pipelines: 0,
    total_cost_usd: 0.0,
    next_stage_time: null,
  },
  fleetDetail: {
    status: "operational",
    active_pipelines: 0,
    completed_pipelines: 0,
    failed_pipelines: 0,
    total_cost_usd: 0.0,
    repos: [],
    worker_pool_utilization: 0.0,
    next_stage_time: null,
  },
  loading: false,

  async loadFleet() {
    this.loading = true;
    try {
      // Load both status endpoints
      const [statusRes, detailRes] = await Promise.all([
        fetch("/api/fleet/status"),
        fetch("/api/fleet/detail"),
      ]);

      if (statusRes.ok) {
        const data = await statusRes.json();
        this.fleetStatus = data;
      }

      if (detailRes.ok) {
        const data = await detailRes.json();
        this.fleetDetail = data;
      }
    } catch (err) {
      console.error("Error loading fleet:", err);
    } finally {
      this.loading = false;
    }
  },

  getUtilizationColor(utilization) {
    if (utilization < 0.33) return "var(--success)";
    if (utilization < 0.66) return "var(--warning)";
    return "var(--error)";
  },
}));
