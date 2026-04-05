// ========================================
// edit — Theme Picker
// ========================================

(function () {
  const themes = [
    { id: "midnight", label: "Midnight", color: "#ffffff" },
    { id: "ocean", label: "Ocean", color: "#38bdf8" },
    { id: "forest", label: "Forest", color: "#4ade80" },
    { id: "ember", label: "Ember", color: "#f97316" },
    { id: "violet", label: "Violet", color: "#a78bfa" },
  ];

  const saved = localStorage.getItem("edit-theme") || "midnight";

  function applyTheme(id) {
    if (id === "midnight") {
      document.documentElement.removeAttribute("data-theme");
    } else {
      document.documentElement.setAttribute("data-theme", id);
    }
    localStorage.setItem("edit-theme", id);

    document.querySelectorAll(".tp-dot").forEach((dot, i) => {
      dot.classList.toggle("active", themes[i].id === id);
    });
  }

  function mount(containerId) {
    const container = document.getElementById(containerId);
    if (!container) return;

    const wrapper = document.createElement("div");
    wrapper.className = "tp-wrapper";

    themes.forEach((t) => {
      const dot = document.createElement("button");
      dot.className = "tp-dot" + (t.id === saved ? " active" : "");
      dot.style.setProperty("--dot-color", t.color);
      dot.title = t.label;
      dot.setAttribute("aria-label", "Switch to " + t.label + " theme");
      dot.addEventListener("click", () => applyTheme(t.id));
      wrapper.appendChild(dot);
    });

    container.appendChild(wrapper);
  }

  // Apply saved theme immediately
  applyTheme(saved);

  // Mount when DOM is ready
  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", () => mount("theme-picker"));
  } else {
    mount("theme-picker");
  }
})();
