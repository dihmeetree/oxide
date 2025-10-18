/**
 * Shared Chart.js utilities for Oxide Dashboard
 *
 * This module provides reusable charting functions to eliminate code duplication
 * across multiple dashboard templates.
 */

// Color palette used consistently across all charts
const CHART_COLORS = [
  "#2776F3", // accent-blue
  "#10B981", // accent-green
  "#F59E0B", // orange
  "#8B5CF6", // purple
  "#EC4899", // pink
  "#14B8A6", // teal
  "#F97316", // orange-alt
  "#6366F1", // indigo
];

/**
 * Apply colors to server-rendered legend elements
 * @param {Chart} chart - Chart.js instance
 * @param {string} containerId - ID of the legend container element
 */
function applyLegendColors(chart, containerId) {
  const legendContainer = document.getElementById(containerId);
  if (!legendContainer) return;

  const colorSpans = legendContainer.querySelectorAll('[data-color-for]');
  colorSpans.forEach((span) => {
    const name = span.getAttribute('data-color-for');
    const dataset = chart.data.datasets.find(ds => ds.label === name);
    if (dataset) {
      span.style.backgroundColor = dataset.borderColor;
    }
  });

  // Update button states based on visibility
  const buttons = legendContainer.querySelectorAll('button');
  buttons.forEach((button, i) => {
    const isVisible = chart.isDatasetVisible(i);
    if (!isVisible) {
      button.classList.add('opacity-50');
      button.classList.remove('bg-dark-hover');
      button.classList.add('bg-dark-bg');
    }
  });
}

/**
 * Toggle dataset visibility in a chart
 * @param {Chart} chart - Chart.js instance
 * @param {string} containerId - ID of the legend container
 * @param {number} index - Index of the dataset to toggle
 */
function toggleChartDataset(chart, containerId, index) {
  const isCurrentlyVisible = chart.isDatasetVisible(index);

  // Count how many datasets are currently visible
  let visibleCount = 0;
  for (let i = 0; i < chart.data.datasets.length; i++) {
    if (chart.isDatasetVisible(i)) visibleCount++;
  }

  // If only one is visible and we click it, show all
  if (visibleCount === 1 && isCurrentlyVisible) {
    chart.data.datasets.forEach((dataset, i) => {
      chart.setDatasetVisibility(i, true);
    });
  } else {
    // Otherwise, show only the clicked one
    chart.data.datasets.forEach((dataset, i) => {
      chart.setDatasetVisibility(i, i === index);
    });
  }
  chart.update();

  // Update button styles
  const legendContainer = document.getElementById(containerId);
  if (!legendContainer) return;

  const buttons = legendContainer.querySelectorAll('button');
  buttons.forEach((button, i) => {
    const isVisible = chart.isDatasetVisible(i);
    if (isVisible) {
      button.classList.remove('opacity-50', 'bg-dark-bg');
      button.classList.add('bg-dark-hover');
    } else {
      button.classList.add('opacity-50', 'bg-dark-bg');
      button.classList.remove('bg-dark-hover');
    }
  });
}

/**
 * Assign consistent colors to items (nodes, pods, etc.)
 * @param {Array} items - Array of items that need color assignment
 * @param {Object} colorMap - Existing color map to update
 * @param {Function} keyFn - Function to extract key from item (defaults to .name)
 * @returns {Object} Updated color map
 */
function assignColors(items, colorMap, keyFn = (item) => item.name) {
  items.forEach((item) => {
    const key = keyFn(item);
    if (!colorMap[key]) {
      colorMap[key] = CHART_COLORS[Object.keys(colorMap).length % CHART_COLORS.length];
    }
  });
  return colorMap;
}

/**
 * Format Unix timestamps to local time strings
 * @param {Array<number>} timestamps - Array of Unix timestamps (seconds)
 * @param {Object} options - Formatting options
 * @param {boolean} options.includeSeconds - Include seconds in output (default: true)
 * @returns {Array<string>} Formatted time strings
 */
function formatTimestamps(timestamps, options = {}) {
  const { includeSeconds = true } = options;

  return timestamps.map((ts) => {
    const date = new Date(ts * 1000);
    return date.toLocaleTimeString([], {
      hour: "2-digit",
      minute: "2-digit",
      ...(includeSeconds && { second: "2-digit" }),
    });
  });
}

/**
 * Get common Chart.js configuration options
 * @param {Object} customOptions - Custom options to override defaults
 * @returns {Object} Chart.js options object
 */
function getCommonChartOptions(customOptions = {}) {
  const defaults = {
    responsive: true,
    maintainAspectRatio: false,
    animation: false,
    interaction: {
      mode: "nearest",
      axis: "x",
      intersect: false,
    },
    plugins: {
      legend: {
        display: false,
      },
      tooltip: {
        enabled: true,
        mode: "index",
        intersect: false,
        backgroundColor: "#1A1A1A",
        titleColor: "#FFFFFF",
        titleFont: {
          size: 14,
          weight: "600",
          family: "'Inter', -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif",
        },
        bodyColor: "#E5E5E5",
        bodyFont: {
          size: 13,
          weight: "500",
        },
        borderColor: "#404040",
        borderWidth: 1,
        padding: 16,
        displayColors: true,
        boxWidth: 10,
        boxHeight: 10,
        boxPadding: 6,
        cornerRadius: 8,
        caretSize: 6,
        caretPadding: 12,
      },
    },
    scales: {
      x: {
        display: true,
        grid: {
          color: "#313131",
        },
        ticks: {
          color: "#AAAAAA",
        },
      },
      y: {
        display: true,
        min: 0,
        grid: {
          color: "#313131",
        },
        ticks: {
          color: "#AAAAAA",
        },
      },
    },
  };

  // Deep merge custom options
  return mergeDeep(defaults, customOptions);
}

/**
 * Create a line chart with standard configuration
 * @param {string} canvasId - ID of the canvas element
 * @param {Object} data - Chart data object
 * @param {Object} options - Chart options
 * @returns {Chart} Chart.js instance
 */
function createLineChart(canvasId, data, options = {}) {
  const ctx = document.getElementById(canvasId);
  if (!ctx) {
    console.error(`Canvas element '${canvasId}' not found`);
    return null;
  }

  return new Chart(ctx.getContext("2d"), {
    type: "line",
    data: data,
    options: getCommonChartOptions(options),
  });
}

/**
 * Create dataset configuration for chart
 * @param {string} label - Dataset label
 * @param {Array} data - Data points
 * @param {string} color - Border color
 * @param {Object} options - Additional options
 * @returns {Object} Dataset configuration
 */
function createDataset(label, data, color, options = {}) {
  const {
    fill = false,
    stacked = false,
    borderWidth = 2,
    tension = 0.4,
    pointRadius = 0,
    pointHoverRadius = 5,
  } = options;

  return {
    label,
    data,
    borderColor: color,
    backgroundColor: color + (fill && stacked ? "80" : "20"),
    borderWidth,
    fill,
    tension,
    pointRadius,
    pointHoverRadius,
  };
}

/**
 * Update chart with new data while preserving visibility state
 * @param {Chart} chart - Chart.js instance
 * @param {Array} newLabels - New labels for x-axis
 * @param {Array} newDatasets - New datasets
 */
function updateChart(chart, newLabels, newDatasets) {
  if (!chart) return;

  // Capture current visibility state
  const visibilityState = chart.data.datasets.map((ds, i) => chart.isDatasetVisible(i));

  // Update chart data
  chart.data.labels = newLabels;
  chart.data.datasets = newDatasets.map((ds, i) => ({
    ...ds,
    hidden: i < visibilityState.length ? !visibilityState[i] : false,
  }));

  chart.update();
}

/**
 * Deep merge two objects
 * @param {Object} target - Target object
 * @param {Object} source - Source object
 * @returns {Object} Merged object
 */
function mergeDeep(target, source) {
  const output = Object.assign({}, target);
  if (isObject(target) && isObject(source)) {
    Object.keys(source).forEach(key => {
      if (isObject(source[key])) {
        if (!(key in target)) {
          Object.assign(output, { [key]: source[key] });
        } else {
          output[key] = mergeDeep(target[key], source[key]);
        }
      } else {
        Object.assign(output, { [key]: source[key] });
      }
    });
  }
  return output;
}

/**
 * Check if value is an object
 * @param {*} item - Value to check
 * @returns {boolean} True if object
 */
function isObject(item) {
  return item && typeof item === 'object' && !Array.isArray(item);
}

/**
 * Sort data items by name for consistent ordering
 * @param {Array} items - Array of items with name property
 * @param {Function} keyFn - Function to extract sort key (defaults to .name)
 * @returns {Array} Sorted array
 */
function sortByName(items, keyFn = (item) => item.name) {
  return items.sort((a, b) => keyFn(a).localeCompare(keyFn(b)));
}

// Export functions for use in templates
if (typeof module !== 'undefined' && module.exports) {
  // Node.js environment
  module.exports = {
    CHART_COLORS,
    applyLegendColors,
    toggleChartDataset,
    assignColors,
    formatTimestamps,
    getCommonChartOptions,
    createLineChart,
    createDataset,
    updateChart,
    sortByName,
  };
}
