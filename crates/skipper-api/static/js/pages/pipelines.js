// Pipelines page — displays active and recent Shipwright pipelines
Alpine.data("pipelinesPage", () => ({
  pipelines: [],
  loading: false,

  async loadPipelines() {
    this.loading = true;
    try {
      const response = await fetch("/api/pipelines");
      if (!response.ok) throw new Error("Failed to load pipelines");
      const data = await response.json();
      this.pipelines = data.pipelines || [];
    } catch (err) {
      console.error("Error loading pipelines:", err);
      this.pipelines = [];
    } finally {
      this.loading = false;
    }
  },

  getProgressColor(progress, total) {
    const percent = Math.round((progress / total) * 100);
    if (percent < 33) return "var(--info)";
    if (percent < 66) return "var(--warning)";
    return "var(--success)";
  },
}));
