// Memory page — displays Shipwright learned patterns and failure data
Alpine.data("memoryPage", () => ({
  memory: {
    entries: [],
    total: 0,
  },
  loading: false,

  async loadMemory() {
    this.loading = true;
    try {
      const response = await fetch("/api/memory");
      if (!response.ok) throw new Error("Failed to load memory");
      const data = await response.json();
      this.memory = data || { entries: [], total: 0 };
    } catch (err) {
      console.error("Error loading memory:", err);
      this.memory = { entries: [], total: 0 };
    } finally {
      this.loading = false;
    }
  },

  getPatternBadgeClass(type) {
    const typeMap = {
      failure: "badge-error",
      security: "badge-warn",
      performance: "badge-info",
      optimization: "badge-success",
      architecture: "badge-muted",
    };
    return typeMap[type] || "badge-muted";
  },
}));
