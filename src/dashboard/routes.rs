/// HTTP route handlers
use askama::Template;
use axum::{
    extract::State,
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{Html, IntoResponse, Json, Redirect},
    Form,
};
use serde::{Deserialize, Serialize};
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use tracing::{error, info};

use super::server::AppState;
use super::templates::{
    AlertsTemplate, CiliumTemplate, ClusterDetailTemplate, ClustersTemplate, CreateClusterTemplate,
    DeploymentDetailTemplate, DeploymentsTemplate, EnvoyTemplate, EventsTemplate, IndexTemplate,
    InsightsTemplate, LogLevel, LogLine, MetricsTemplate, NodeDetailTemplate, NodeInfoWithCluster,
    NodesTemplate, PodDetailTemplate, PodsTemplate, ServicesTemplate,
};
use crate::config::ClusterConfig;

/// Parse log level from a log line
/// Supports common log formats: level=, "level":, [LEVEL], LEVEL:, etc.
/// Prioritizes structured log formats (level=) over keyword matching to avoid false positives
fn parse_log_level(line: &str) -> LogLevel {
    let line_lower = line.to_lowercase();

    // First, check for structured log formats (most reliable)
    // These are checked first to avoid false matches in file paths or messages
    if line_lower.contains("level=error") || line_lower.contains("\"level\":\"error\"") {
        return LogLevel::Error;
    } else if line_lower.contains("level=warn") || line_lower.contains("\"level\":\"warn\"") {
        return LogLevel::Warning;
    } else if line_lower.contains("level=info") || line_lower.contains("\"level\":\"info\"") {
        return LogLevel::Info;
    } else if line_lower.contains("level=debug") || line_lower.contains("\"level\":\"debug\"") {
        return LogLevel::Debug;
    } else if line_lower.contains("level=trace") || line_lower.contains("\"level\":\"trace\"") {
        return LogLevel::Trace;
    }

    // Then check for bracketed formats [LEVEL]
    if line_lower.contains("[error]") {
        return LogLevel::Error;
    } else if line_lower.contains("[warn]") || line_lower.contains("[warning]") {
        return LogLevel::Warning;
    } else if line_lower.contains("[info]") {
        return LogLevel::Info;
    } else if line_lower.contains("[debug]") {
        return LogLevel::Debug;
    } else if line_lower.contains("[trace]") {
        return LogLevel::Trace;
    }

    // Check for level at start of line with colon (e.g., "ERROR: message")
    if line_lower.starts_with("error:") || line_lower.starts_with("fatal:") {
        return LogLevel::Error;
    } else if line_lower.starts_with("warn:") || line_lower.starts_with("warning:") {
        return LogLevel::Warning;
    } else if line_lower.starts_with("info:") {
        return LogLevel::Info;
    } else if line_lower.starts_with("debug:") {
        return LogLevel::Debug;
    } else if line_lower.starts_with("trace:") {
        return LogLevel::Trace;
    }

    // Check for standalone level keywords at word boundaries (last resort)
    // Use regex-like pattern to match only complete words
    if line_lower.contains(" fatal ") || line_lower.contains(" panic ") {
        return LogLevel::Error;
    }

    // Default to Unknown if no pattern matches
    LogLevel::Unknown
}

/// Get both firing alerts count and insights count from cache (optimized - uses pre-calculated counts)
async fn get_alerts_and_insights_counts(
    cache: &super::cache::ClusterCache,
) -> (usize, usize, usize) {
    let data = cache.inner.read().await;
    (
        data.firing_alerts_count,
        data.insights_count,
        data.warning_events_count,
    )
}

/// Collect unique timestamps from metrics history
fn collect_unique_timestamps(history: &crate::prometheus::NodeMetricsHistory) -> Vec<i64> {
    let mut all_timestamps = std::collections::BTreeSet::new();
    for (ts, _) in &history.cpu_history {
        all_timestamps.insert(*ts);
    }
    all_timestamps.iter().copied().collect()
}

/// Align metric values to unique timestamps with tolerance
fn align_metric_to_timestamps(
    timestamps: &[i64],
    metric_history: &[(i64, f64)],
    tolerance_secs: i64,
) -> Vec<f64> {
    timestamps
        .iter()
        .map(|ts| {
            // Try exact match first
            if let Some((_, val)) = metric_history.iter().find(|(t, _)| t == ts) {
                return *val;
            }
            // If no exact match, find nearest timestamp within tolerance
            metric_history
                .iter()
                .filter(|(t, _)| (*t - *ts).abs() <= tolerance_secs)
                .min_by_key(|(t, _)| (*t - *ts).abs())
                .map(|(_, val)| *val)
                .unwrap_or(0.0)
        })
        .collect()
}

/// Build pod metrics JSON from history
fn build_pod_metrics_json(history: &crate::prometheus::NodeMetricsHistory) -> serde_json::Value {
    let timestamps = collect_unique_timestamps(history);

    // Align CPU data to unique timestamps
    let cpu_history: Vec<f64> = timestamps
        .iter()
        .map(|ts| {
            history
                .cpu_history
                .iter()
                .find(|(t, _)| t == ts)
                .map(|(_, val)| *val)
                .unwrap_or(0.0)
        })
        .collect();

    // Align memory data to unique timestamps (with 2-second tolerance)
    let memory_history = align_metric_to_timestamps(&timestamps, &history.memory_history, 2);

    serde_json::json!({
        "timestamps": timestamps,
        "cpu_history": cpu_history,
        "memory_history": memory_history,
    })
}

/// Generate ETag from content string (fast FNV-1a hash)
fn generate_etag(content: &str) -> String {
    let mut hasher = DefaultHasher::new();
    content.hash(&mut hasher);
    format!("\"{}\"", hasher.finish())
}

/// Check if ETag matches If-None-Match header
fn check_etag(headers: &HeaderMap, etag: &str) -> bool {
    headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        .map(|v| v == etag)
        .unwrap_or(false)
}

/// Returns the preloader HTML page with auto-refresh
fn preloader_page() -> impl IntoResponse {
    let html = r#"
        <!DOCTYPE html>
        <html>
        <head>
            <meta http-equiv="refresh" content="5">
            <title>Loading - Oxide</title>
            <style>
                body {
                    background: #1e1e1e;
                    color: #F2F2F2;
                    font-family: system-ui;
                    display: flex;
                    align-items: center;
                    justify-content: center;
                    height: 100vh;
                    margin: 0;
                }
                .loader { text-align: center; }
                .logo-container {
                    display: flex;
                    align-items: center;
                    justify-content: center;
                    gap: 12px;
                    margin-bottom: 12px;
                }
                .logo { width: 64px; height: 64px; object-fit: contain; }
                .brand { font-size: 2.5rem; font-weight: 600; color: #F2F2F2; margin: 0; }
                @keyframes spin-slow {
                    from { transform: rotate(0deg); }
                    to { transform: rotate(360deg); }
                }
                .animate-spin-slow {
                    animation: spin-slow 2s linear infinite;
                }
                .spinner-container {
                    width: 32px;
                    height: 32px;
                    margin: 0 auto 8px;
                }
                .spinner-ring {
                    width: 32px;
                    height: 32px;
                    border: 4px solid rgba(39, 118, 243, 0.2);
                    border-top-color: rgba(39, 118, 243, 1);
                    border-radius: 50%;
                }
                .status-text {
                    color: #B8B8B8;
                    font-size: 1rem;
                    line-height: 1.6;
                    margin: 20px 0 8px 0;
                    max-width: 520px;
                }
                .countdown-text {
                    color: #888888;
                    font-size: 0.9375rem;
                    margin: 0;
                }
            </style>
        </head>
        <body>
            <div class="loader">
                <div class="logo-container">
                    <img src="/static/logo.png" alt="Oxide Logo" class="logo">
                    <h1 class="brand">Oxide</h1>
                </div>
                <div class="spinner-container">
                    <div class="spinner-ring animate-spin-slow"></div>
                </div>
                <h2 style="font-size: 1.75rem; font-weight: 600; margin-bottom: 4px; color: #E5E5E5;">Fetching Resources...</h2>
                <p class="status-text">Collecting cluster data from the Kubernetes API and Prometheus metrics.</p>
                <p class="countdown-text">Refreshing in <span id="countdown" style="display: inline-block; min-width: 1ch; text-align: center;">5</span> seconds...</p>
            </div>
            <script>
                let seconds = 5;
                const countdownEl = document.getElementById('countdown');
                setInterval(function() {
                    seconds--;
                    if (seconds > 0) {
                        countdownEl.textContent = seconds;
                    }
                }, 1000);
            </script>
        </body>
        </html>
    "#;

    let mut headers = HeaderMap::new();
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("no-cache, no-store, must-revalidate"),
    );
    headers.insert(header::PRAGMA, HeaderValue::from_static("no-cache"));
    headers.insert(header::EXPIRES, HeaderValue::from_static("0"));

    (headers, Html(html))
}

/// Render the home/index page with cluster overview
pub async fn index(State(state): State<AppState>) -> impl IntoResponse {
    let cache_ready = state.cache.is_ready().await;

    if !cache_ready {
        return preloader_page().into_response();
    }

    let clusters = state.cache.get_clusters().await;
    let total_nodes: usize = clusters.iter().map(|c| c.nodes).sum();
    let (firing_alerts_count, insights_count, warning_events_count) =
        get_alerts_and_insights_counts(&state.cache).await;
    let template = IndexTemplate {
        cluster_count: clusters.len(),
        total_nodes,
        active_page: "dashboard".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        firing_alerts_count,
        insights_count,
        warning_events_count,
    };
    Html(template.render().unwrap()).into_response()
}

/// Render the clusters list page
pub async fn clusters_list(State(state): State<AppState>) -> impl IntoResponse {
    let cache_ready = state.cache.is_ready().await;

    if !cache_ready {
        return preloader_page().into_response();
    }

    let clusters = state.cache.get_clusters().await;
    let (firing_alerts_count, insights_count, warning_events_count) =
        get_alerts_and_insights_counts(&state.cache).await;
    let template = ClustersTemplate {
        clusters: clusters.as_ref(),
        active_page: "clusters".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        firing_alerts_count,
        insights_count,
        warning_events_count,
    };
    Html(template.render().unwrap()).into_response()
}

/// Create cluster form page
pub async fn clusters_create_form(State(state): State<AppState>) -> impl IntoResponse {
    let (firing_alerts_count, insights_count, warning_events_count) =
        get_alerts_and_insights_counts(&state.cache).await;
    let template = CreateClusterTemplate {
        active_page: "clusters".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        firing_alerts_count,
        insights_count,
        warning_events_count,
    };
    Html(template.render().unwrap())
}

/// Cluster detail page
pub async fn cluster_detail(
    State(state): State<AppState>,
    axum::extract::Path(name): axum::extract::Path<String>,
) -> impl IntoResponse {
    match state.cache.get_cluster_detail(&name).await {
        Some(cluster) => {
            // Use pre-cached metrics JSON (BLAZING FAST!)
            let metrics_json = state
                .cache
                .get_cluster_metrics_json_cache(&name)
                .await
                .map(|arc| arc.to_string())
                .unwrap_or_else(|| "{}".to_string());

            // Build node names list for legend from cluster nodes
            let mut node_names: Vec<String> =
                cluster.nodes.iter().map(|n| n.name.clone()).collect();
            node_names.sort();

            // Get alerts and insights counts using helper
            let (firing_alerts_count, insights_count, warning_events_count) =
                get_alerts_and_insights_counts(&state.cache).await;

            let template = ClusterDetailTemplate {
                cluster,
                active_page: "clusters".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
                firing_alerts_count,
                insights_count,
                warning_events_count,
                metrics_json,
                node_names,
            };
            Html(template.render().unwrap()).into_response()
        }
        None => preloader_page().into_response(),
    }
}

/// Node detail page
pub async fn node_detail(
    State(state): State<AppState>,
    axum::extract::Path((cluster_name, node_name)): axum::extract::Path<(String, String)>,
) -> impl IntoResponse {
    match state.cache.get_node_detail(&cluster_name, &node_name).await {
        Some(node) => {
            // Get metrics history from cache to build JSON
            let node_metrics_history = state.cache.get_node_metrics_history().await;
            let metrics_json = if let Some(history) = node_metrics_history.get(&node_name) {
                let timestamps: Vec<i64> = history.cpu_history.iter().map(|(ts, _)| *ts).collect();
                let cpu_history: Vec<f64> =
                    history.cpu_history.iter().map(|(_, val)| *val).collect();
                let memory_history: Vec<f64> =
                    history.memory_history.iter().map(|(_, val)| *val).collect();

                serde_json::json!({
                    "timestamps": timestamps,
                    "cpu_history": cpu_history,
                    "memory_history": memory_history,
                })
                .to_string()
            } else {
                "{}".to_string()
            };

            let (firing_alerts_count, insights_count, warning_events_count) =
                get_alerts_and_insights_counts(&state.cache).await;
            let template = NodeDetailTemplate {
                node,
                metrics_json,
                active_page: "nodes".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
                firing_alerts_count,
                insights_count,
                warning_events_count,
            };
            Html(template.render().unwrap()).into_response()
        }
        None => preloader_page().into_response(),
    }
}

/// Pod detail page
pub async fn pod_detail(
    State(state): State<AppState>,
    axum::extract::Path((cluster_name, node_name, namespace, pod_name)): axum::extract::Path<(
        String,
        String,
        String,
        String,
    )>,
) -> impl IntoResponse {
    match state
        .cache
        .get_pod_detail(&cluster_name, &node_name, &namespace, &pod_name)
        .await
    {
        Some(pod) => {
            // Get metrics from cache
            let metrics = state
                .cache
                .get_pod_metrics(&namespace, &pod_name)
                .await
                .unwrap_or_default();

            // Build metrics JSON using helper function
            let metrics_json = build_pod_metrics_json(&metrics);

            let (firing_alerts_count, insights_count, warning_events_count) =
                get_alerts_and_insights_counts(&state.cache).await;
            let template = PodDetailTemplate {
                pod,
                metrics_json: serde_json::to_string(&metrics_json)
                    .unwrap_or_else(|_| "{}".to_string()),
                active_page: "pods".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
                firing_alerts_count,
                insights_count,
                warning_events_count,
            };
            Html(template.render().unwrap()).into_response()
        }
        None => preloader_page().into_response(),
    }
}

/// Pod logs viewing page
pub async fn pod_logs(
    State(state): State<AppState>,
    axum::extract::Path((cluster_name, node_name, namespace, pod_name)): axum::extract::Path<(
        String,
        String,
        String,
        String,
    )>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> impl IntoResponse {
    // Extract query parameters
    let container = params.get("container").map(|s| s.as_str());
    let tail = params
        .get("tail")
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(100); // Default to 100 lines
    let follow = params.get("follow").map(|s| s == "true").unwrap_or(false);

    // Get pod detail to display pod info
    let pod = state
        .cache
        .get_pod_detail(&cluster_name, &node_name, &namespace, &pod_name)
        .await;

    if pod.is_none() {
        return preloader_page().into_response();
    }

    let pod = pod.unwrap();

    // Fetch logs from kubectl
    let logs_result = state
        .cache
        .get_pod_logs(
            &cluster_name,
            &namespace,
            &pod_name,
            container,
            Some(tail),
            follow,
        )
        .await;

    let (log_lines, error_message) = match logs_result {
        Ok(logs) => {
            // Split logs into lines, parse log level, reverse to show newest first
            let lines: Vec<LogLine> = logs
                .lines()
                .rev()
                .map(|line| LogLine {
                    content: line.to_string(),
                    level: parse_log_level(line),
                })
                .collect();
            (lines, None)
        }
        Err(e) => (Vec::new(), Some(e)),
    };

    // Get alerts and insights counts
    let (firing_alerts_count, insights_count, warning_events_count) =
        get_alerts_and_insights_counts(&state.cache).await;

    let template = super::templates::PodLogsTemplate {
        pod,
        log_lines,
        error_message,
        selected_container: container.map(|s| s.to_string()),
        tail_lines: tail,
        active_page: "pods".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        firing_alerts_count,
        insights_count,
        warning_events_count,
    };

    Html(template.render().unwrap()).into_response()
}

/// Create cluster POST handler
pub async fn clusters_create(
    State(state): State<AppState>,
    Form(form): Form<CreateClusterForm>,
) -> impl IntoResponse {
    info!("Creating cluster: {}", form.cluster_name);

    // Validate cluster name (strict validation to prevent path traversal)
    if form.cluster_name.is_empty() || form.cluster_name.len() > 63 {
        error!("Invalid cluster name: must be between 1-63 characters");
        return Redirect::to("/clusters/create");
    }

    if !form
        .cluster_name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        error!("Invalid cluster name: must be lowercase, numbers, and hyphens only");
        return Redirect::to("/clusters/create");
    }

    if form.cluster_name.starts_with('-') || form.cluster_name.ends_with('-') {
        error!("Invalid cluster name: cannot start or end with hyphen");
        return Redirect::to("/clusters/create");
    }

    if form.cluster_name.contains("..")
        || form.cluster_name.contains('/')
        || form.cluster_name.contains('\\')
    {
        error!("Invalid cluster name: contains invalid characters");
        return Redirect::to("/clusters/create");
    }

    // Create cluster config from form data
    let config = create_cluster_config_from_form(&form);

    // Write config to secure temporary file with random suffix to avoid race conditions
    use std::time::{SystemTime, UNIX_EPOCH};
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let temp_filename = format!("oxide-{}-{}.yaml", form.cluster_name, timestamp);
    let temp_config_path = std::env::temp_dir().join(temp_filename);

    let config_yaml = match serde_yaml::to_string(&config) {
        Ok(yaml) => yaml,
        Err(e) => {
            error!("Failed to serialize config: {}", e);
            return Redirect::to("/clusters/create");
        }
    };

    if let Err(e) = tokio::fs::write(&temp_config_path, config_yaml).await {
        error!("Failed to write config file: {}", e);
        return Redirect::to("/clusters/create");
    }

    info!("Config written to {:?}", temp_config_path);

    // Spawn cluster creation in background
    let output_dir = state.output_dir.clone();
    let cluster_name = form.cluster_name.clone();
    tokio::spawn(async move {
        info!("Starting cluster creation for: {}", cluster_name);
        match crate::cluster::Cluster::create(&temp_config_path, &output_dir).await {
            Ok(_) => {
                info!("[OK] Cluster {} created successfully", cluster_name);
                // Clean up temp config
                let _ = tokio::fs::remove_file(&temp_config_path).await;
            }
            Err(e) => {
                error!("Failed to create cluster {}: {}", cluster_name, e);
                // Clean up temp config even on failure
                let _ = tokio::fs::remove_file(&temp_config_path).await;
            }
        }
    });

    info!("Cluster creation started in background, redirecting to clusters list");
    Redirect::to("/clusters")
}

/// API endpoint to list all clusters as JSON
pub async fn api_clusters_list(State(state): State<AppState>) -> impl IntoResponse {
    let clusters = state.cache.get_clusters().await;
    // Clone needed for JSON serialization (Arc<[T]> doesn't implement Serialize)
    Json(clusters.to_vec())
}

/// API endpoint to retrieve metrics for a specific cluster's nodes
pub async fn api_cluster_metrics(
    State(state): State<AppState>,
    axum::extract::Path(cluster_name): axum::extract::Path<String>,
) -> impl IntoResponse {
    // Get cluster detail to find node names
    match state.cache.get_cluster_detail(&cluster_name).await {
        Some(cluster) => {
            // Get node metrics for this cluster's nodes only
            let node_metrics_history = state.cache.get_node_metrics_history().await;

            // Filter metrics to only include nodes from this cluster
            let cluster_node_names: Vec<String> =
                cluster.nodes.iter().map(|n| n.name.clone()).collect();
            let cluster_metrics: HashMap<String, crate::prometheus::NodeMetricsHistory> =
                node_metrics_history
                    .iter()
                    .filter(|(name, _)| cluster_node_names.contains(name))
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect();

            // Build metrics JSON
            if !cluster_metrics.is_empty() {
                let mut all_timestamps = std::collections::BTreeSet::new();
                for history in cluster_metrics.values() {
                    for (ts, _) in &history.cpu_history {
                        all_timestamps.insert(*ts);
                    }
                }
                let timestamps: Vec<i64> = all_timestamps.into_iter().collect();

                let metrics_nodes: Vec<MetricsNode> = cluster_metrics
                    .iter()
                    .map(|(name, history)| {
                        let cpu_history: Vec<f64> =
                            history.cpu_history.iter().map(|(_, val)| *val).collect();
                        let memory_history: Vec<f64> =
                            history.memory_history.iter().map(|(_, val)| *val).collect();
                        MetricsNode {
                            name: name.clone(),
                            cpu_history,
                            memory_history,
                        }
                    })
                    .collect();

                let response = NodeMetricsResponse {
                    timestamps,
                    nodes: metrics_nodes,
                };
                Json(response).into_response()
            } else {
                Json(NodeMetricsResponse {
                    timestamps: vec![],
                    nodes: vec![],
                })
                .into_response()
            }
        }
        None => Json(NodeMetricsResponse {
            timestamps: vec![],
            nodes: vec![],
        })
        .into_response(),
    }
}

/// API endpoint to retrieve historical metrics for a specific pod
pub async fn api_pod_metrics(
    State(state): State<AppState>,
    axum::extract::Path((namespace, pod_name)): axum::extract::Path<(String, String)>,
    axum::extract::Query(_params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> impl IntoResponse {
    // Get metrics from cache only (no live Prometheus queries)
    let history = state
        .cache
        .get_pod_metrics(&namespace, &pod_name)
        .await
        .unwrap_or_default();

    // Build metrics JSON using helper function
    Json(build_pod_metrics_json(&history))
}

/// Scale cluster POST handler
pub async fn cluster_scale(
    State(state): State<AppState>,
    axum::extract::Path(cluster_name): axum::extract::Path<String>,
    Form(form): Form<ScaleClusterForm>,
) -> impl IntoResponse {
    info!(
        "Scaling cluster {}: {} {} to {}",
        cluster_name, form.node_type, form.pool_name, form.count
    );

    // Load cluster config
    let config_path = state.config_path.clone();
    let output_dir = state.output_dir.clone();
    let cluster_name_clone = cluster_name.clone();

    // Spawn scaling in background
    tokio::spawn(async move {
        info!("Starting cluster scaling for: {}", cluster_name_clone);

        let role = if form.node_type == "control-plane" {
            crate::hcloud::server::NodeRole::ControlPlane
        } else {
            crate::hcloud::server::NodeRole::Worker
        };

        match crate::cluster::Cluster::scale(
            &config_path,
            &output_dir,
            role,
            Some(&form.pool_name),
            form.count,
            form.force,
            form.timeout,
        )
        .await
        {
            Ok(_) => info!("[OK] Cluster {} scaled successfully", cluster_name_clone),
            Err(e) => error!("Failed to scale cluster {}: {}", cluster_name_clone, e),
        }
    });

    Redirect::to(&format!("/clusters/{}", cluster_name))
}

/// Upgrade cluster POST handler
pub async fn cluster_upgrade(
    State(state): State<AppState>,
    axum::extract::Path(cluster_name): axum::extract::Path<String>,
    Form(form): Form<UpgradeClusterForm>,
) -> impl IntoResponse {
    info!(
        "Upgrading cluster {} to version {}",
        cluster_name, form.version
    );

    let config_path = state.config_path.clone();
    let output_dir = state.output_dir.clone();
    let cluster_name_clone = cluster_name.clone();

    // Spawn upgrade in background
    tokio::spawn(async move {
        info!("Starting cluster upgrade for: {}", cluster_name_clone);

        match crate::cluster::Cluster::upgrade(crate::cluster::UpgradeParams {
            config_path,
            output_dir,
            version: form.version.clone(),
            preserve: form.preserve,
            control_plane: form.control_plane,
            workers: form.workers,
            wait: form.wait,
            stage: form.stage,
        })
        .await
        {
            Ok(_) => info!("[OK] Cluster {} upgraded successfully", cluster_name_clone),
            Err(e) => error!("Failed to upgrade cluster {}: {}", cluster_name_clone, e),
        }
    });

    Redirect::to(&format!("/clusters/{}", cluster_name))
}

/// Delete cluster POST handler
pub async fn cluster_delete(
    State(state): State<AppState>,
    axum::extract::Path(cluster_name): axum::extract::Path<String>,
) -> impl IntoResponse {
    info!("Deleting cluster {}", cluster_name);

    let config_path = state.config_path.clone();
    let output_dir = state.output_dir.clone();

    // Spawn deletion in background
    tokio::spawn(async move {
        info!("Starting cluster deletion for: {}", cluster_name);

        match crate::cluster::Cluster::destroy(&config_path, &output_dir).await {
            Ok(_) => info!("[OK] Cluster {} deleted successfully", cluster_name),
            Err(e) => error!("Failed to delete cluster {}: {}", cluster_name, e),
        }
    });

    Redirect::to("/clusters")
}

/// Create cluster config from form data
fn create_cluster_config_from_form(form: &CreateClusterForm) -> ClusterConfig {
    use crate::config::*;

    ClusterConfig {
        cluster_name: form.cluster_name.clone(),
        hcloud: HetznerCloudConfig {
            token: Some(form.hcloud_token.clone()),
            location: form.location.clone(),
            network: NetworkConfig {
                cidr: "10.0.0.0/16".to_string(),
                subnet_cidr: "10.0.1.0/24".to_string(),
                zone: "eu-central".to_string(),
            },
        },
        talos: TalosConfig {
            version: form.talos_version.clone(),
            kubernetes_version: "1.30.0".to_string(),
            cluster_endpoint: None,
            hcloud_snapshot_id: Some(form.hcloud_snapshot_id.clone()),
            config_patches: vec![],
        },
        cilium: CiliumConfig {
            version: "1.16.5".to_string(),
            enable_hubble: true,
            enable_ipv6: false,
            helm_values: serde_yaml::Value::Null,
        },
        prometheus: None,
        autoscaler: None,
        metrics_server: None,
        control_planes: vec![NodeConfig {
            name: "control-plane".to_string(),
            count: form.control_plane_count,
            server_type: form.server_type.clone(),
            labels: HashMap::new(),
        }],
        workers: if form.worker_count > 0 {
            vec![NodeConfig {
                name: "worker".to_string(),
                count: form.worker_count,
                server_type: form.server_type.clone(),
                labels: HashMap::new(),
            }]
        } else {
            vec![]
        },
    }
}

/// Create cluster form data
#[derive(Debug, Deserialize, Serialize)]
pub struct CreateClusterForm {
    pub cluster_name: String,
    pub hcloud_token: String,
    pub talos_version: String,
    pub hcloud_snapshot_id: String,
    pub control_plane_count: u32,
    pub worker_count: u32,
    pub server_type: String,
    pub location: String,
}

/// Scale cluster form data
#[derive(Debug, Deserialize, Serialize)]
pub struct ScaleClusterForm {
    pub node_type: String, // "control-plane" or "worker"
    pub pool_name: String,
    pub count: u32,
    #[serde(default)]
    pub force: bool,
    #[serde(default = "default_timeout")]
    pub timeout: u64,
}

fn default_timeout() -> u64 {
    600
}

/// Upgrade cluster form data
#[derive(Debug, Deserialize, Serialize)]
pub struct UpgradeClusterForm {
    pub version: String,
    #[serde(default = "default_true")]
    pub preserve: bool,
    #[serde(default)]
    pub control_plane: bool,
    #[serde(default)]
    pub workers: bool,
    #[serde(default)]
    pub wait: bool,
    #[serde(default)]
    pub stage: bool,
}

fn default_true() -> bool {
    true
}

/// Metrics page
pub async fn metrics(State(state): State<AppState>) -> impl IntoResponse {
    let cache_ready = state.cache.is_ready().await;

    if !cache_ready {
        return preloader_page().into_response();
    }

    // Check if we have clusters
    let clusters = state.cache.get_clusters().await;
    if clusters.is_empty() {
        // No clusters found, redirect to clusters page
        return Redirect::to("/clusters").into_response();
    }

    // Get both metrics histories in one lock to avoid multiple lock acquisitions
    let (node_metrics_history, pod_metrics_history) = state.cache.get_all_metrics_history().await;
    let has_data = !node_metrics_history.is_empty();

    // Extract node and pod names for server-side legend rendering
    let mut node_names: Vec<String> = node_metrics_history.keys().cloned().collect();
    node_names.sort();

    let mut pod_names: Vec<String> = pod_metrics_history.keys().cloned().collect();
    pod_names.sort();

    // Build metrics response - use pre-serialized JSON from cache for better performance
    let metrics_json = if has_data {
        state.cache.get_metrics_json_cache().await.to_string()
    } else {
        "{}".to_string()
    };

    let (firing_alerts_count, insights_count, warning_events_count) =
        get_alerts_and_insights_counts(&state.cache).await;
    let template = MetricsTemplate {
        active_page: "metrics".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        has_data,
        metrics_json,
        node_names,
        pod_names,
        firing_alerts_count,
        insights_count,
        warning_events_count,
    };
    Html(template.render().unwrap()).into_response()
}

/// API endpoint to retrieve metrics data for rendering graphs
pub async fn api_metrics(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> impl IntoResponse {
    // Get time range from query params (default to 1h)
    let time_range = params.get("range").map(|s| s.as_str()).unwrap_or("1h");

    // For default 1h range, use pre-serialized JSON cache (BLAZING FAST!)
    if time_range == "1h" {
        let json_cache = state.cache.get_metrics_json_cache().await;
        let json_string = json_cache.as_ref();

        // OPTIMIZATION: Add ETag support for cache validation
        let etag = generate_etag(json_string);
        if check_etag(&headers, &etag) {
            return (StatusCode::NOT_MODIFIED, "").into_response();
        }

        return (
            [
                (
                    header::CONTENT_TYPE,
                    HeaderValue::from_static("application/json"),
                ),
                (header::ETAG, HeaderValue::from_str(&etag).unwrap()),
            ],
            json_string.to_string(),
        )
            .into_response();
    }

    // For custom time ranges, build fresh response from Prometheus
    // For now, we only support 1h range via cache for optimal performance
    // Custom ranges would require fetching fresh data from Prometheus
    let node_details = state.cache.get_node_details_map().await;
    let kubeconfig = state.output_dir.join("kubeconfig");

    let fetch_tasks: Vec<_> = node_details
        .iter()
        .map(|(name, detail)| {
            let private_ip = detail.private_ip.clone();
            let name = name.clone();
            let kubeconfig = kubeconfig.clone();
            let time_range = time_range.to_string();
            async move {
                let history = crate::prometheus::query_node_metrics_range(
                    &private_ip,
                    &kubeconfig,
                    &time_range,
                    "1m",
                )
                .await
                .unwrap_or_default();

                (name, history)
            }
        })
        .collect();

    let node_results: Vec<(String, crate::prometheus::NodeMetricsHistory)> =
        futures::future::join_all(fetch_tasks).await;

    // Build timestamps from node results
    let mut all_timestamps = std::collections::BTreeSet::new();
    for (_, history) in &node_results {
        for (ts, _) in &history.cpu_history {
            all_timestamps.insert(*ts);
        }
    }
    let timestamps: Vec<i64> = all_timestamps.into_iter().collect();

    // Build node metrics
    let nodes: Vec<MetricsNode> = node_results
        .into_iter()
        .map(|(name, history)| {
            let cpu_history: Vec<f64> = history.cpu_history.iter().map(|(_, val)| *val).collect();
            let memory_history: Vec<f64> =
                history.memory_history.iter().map(|(_, val)| *val).collect();
            MetricsNode {
                name,
                cpu_history,
                memory_history,
            }
        })
        .collect();

    // Get pod metrics from cache (always use 1h for pods)
    let pod_metrics_history = state.cache.get_pod_metrics_history().await;
    let pods: Vec<MetricsPod> = pod_metrics_history
        .iter()
        .map(|(key, history)| {
            let parts: Vec<&str> = key.split('/').collect();
            let namespace = parts.first().unwrap_or(&"unknown").to_string();
            let name = parts.get(1).unwrap_or(&"unknown").to_string();
            let cpu_history: Vec<f64> = history
                .cpu_history
                .iter()
                .map(|(_, val)| val * 10.0)
                .collect();
            let memory_history: Vec<f64> =
                history.memory_history.iter().map(|(_, val)| *val).collect();
            MetricsPod {
                name,
                namespace,
                cpu_history,
                memory_history,
            }
        })
        .collect();

    let response = MetricsResponse {
        timestamps,
        nodes,
        pods,
    };

    Json(response).into_response()
}

#[derive(Debug, Serialize)]
struct MetricsResponse {
    timestamps: Vec<i64>,
    nodes: Vec<MetricsNode>,
    pods: Vec<MetricsPod>,
}

#[derive(Debug, Serialize)]
struct NodeMetricsResponse {
    timestamps: Vec<i64>,
    nodes: Vec<MetricsNode>,
}

#[derive(Debug, Serialize)]
struct MetricsNode {
    name: String,
    cpu_history: Vec<f64>,
    memory_history: Vec<f64>,
}

#[derive(Debug, Serialize)]
struct MetricsPod {
    name: String,
    namespace: String,
    cpu_history: Vec<f64>,
    memory_history: Vec<f64>,
}

/// Pods list page
pub async fn pods_list(State(state): State<AppState>) -> impl IntoResponse {
    let cache_ready = state.cache.is_ready().await;

    if !cache_ready {
        return preloader_page().into_response();
    }

    let mut pods = state.cache.get_all_pods().await;

    // Calculate counts
    let running_count = pods.iter().filter(|p| p.status == "Running").count();
    let pending_count = pods.iter().filter(|p| p.status == "Pending").count();
    let failed_count = pods
        .iter()
        .filter(|p| p.status == "Failed" || p.status == "Error" || p.status == "CrashLoopBackOff")
        .count();

    // Sort by status priority (Pending first, then Running, then others), then by CPU usage
    pods.sort_by(|a, b| {
        // Status priority: Pending (0), Running (1), others (2)
        let priority_a = match a.status.as_str() {
            "Pending" => 0,
            "Running" => 1,
            _ => 2,
        };
        let priority_b = match b.status.as_str() {
            "Pending" => 0,
            "Running" => 1,
            _ => 2,
        };

        // First compare by priority
        match priority_a.cmp(&priority_b) {
            std::cmp::Ordering::Equal => {
                // If same priority, sort by CPU usage (highest to lowest)
                let cpu_a = a.cpu.trim_end_matches('m').parse::<f64>().unwrap_or(-1.0);
                let cpu_b = b.cpu.trim_end_matches('m').parse::<f64>().unwrap_or(-1.0);
                cpu_b
                    .partial_cmp(&cpu_a)
                    .unwrap_or(std::cmp::Ordering::Equal)
            }
            other => other,
        }
    });

    let (firing_alerts_count, insights_count, warning_events_count) =
        get_alerts_and_insights_counts(&state.cache).await;
    let template = PodsTemplate {
        pods,
        running_count,
        pending_count,
        failed_count,
        active_page: "pods".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        firing_alerts_count,
        insights_count,
        warning_events_count,
    };
    Html(template.render().unwrap()).into_response()
}

/// Nodes list page
pub async fn nodes_list(State(state): State<AppState>) -> impl IntoResponse {
    let cache_ready = state.cache.is_ready().await;

    if !cache_ready {
        return preloader_page().into_response();
    }

    let node_details = state.cache.get_all_node_details().await;

    // Convert NodeDetail to NodeInfoWithCluster
    let mut nodes: Vec<NodeInfoWithCluster> = node_details
        .into_iter()
        .map(|node| NodeInfoWithCluster {
            cluster_name: node.cluster_name,
            name: node.name,
            role: node.role,
            ip: node.ip,
            private_ip: node.private_ip,
            status: node.status,
            server_type: node.server_type,
            created: node.created,
            cpu_usage_percent: node.cpu_usage_percent,
            memory_usage_percent: node.memory_usage_percent,
        })
        .collect();

    // Calculate counts
    let control_plane_count = nodes.iter().filter(|n| n.role == "Control Plane").count();
    let worker_count = nodes.iter().filter(|n| n.role == "Worker").count();
    let running_count = nodes.iter().filter(|n| n.status == "running").count();

    // Sort by cluster name, then by role (control plane first), then by name
    nodes.sort_by(|a, b| {
        a.cluster_name
            .cmp(&b.cluster_name)
            .then_with(|| {
                // Control Plane before Worker
                match (&a.role as &str, &b.role as &str) {
                    ("Control Plane", "Worker") => std::cmp::Ordering::Less,
                    ("Worker", "Control Plane") => std::cmp::Ordering::Greater,
                    _ => std::cmp::Ordering::Equal,
                }
            })
            .then_with(|| a.name.cmp(&b.name))
    });

    // Use pre-cached node metrics JSON (BLAZING FAST!)
    let metrics_json = state.cache.get_node_metrics_json_cache().await.to_string();

    // Extract node names for server-side legend rendering
    let node_metrics_history = state.cache.get_node_metrics_history().await;
    let mut node_names: Vec<String> = node_metrics_history.keys().cloned().collect();
    node_names.sort();

    // Get alerts and insights counts using helper
    let (firing_alerts_count, insights_count, warning_events_count) =
        get_alerts_and_insights_counts(&state.cache).await;
    let template = NodesTemplate {
        nodes,
        control_plane_count,
        worker_count,
        running_count,
        metrics_json,
        node_names,
        active_page: "nodes".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        firing_alerts_count,
        insights_count,
        warning_events_count,
    };
    Html(template.render().unwrap()).into_response()
}

/// Services list page
pub async fn services_list(State(state): State<AppState>) -> impl IntoResponse {
    let cache_ready = state.cache.is_ready().await;

    if !cache_ready {
        return preloader_page().into_response();
    }

    let mut services = state.cache.get_all_services().await;

    // Sort services: LoadBalancer first, then NodePort, then ClusterIP
    // Within each type, sort alphabetically by name
    services.sort_by(|a, b| {
        // Define priority: LoadBalancer = 0, NodePort = 1, ClusterIP = 2, others = 3
        let type_priority = |service_type: &str| match service_type {
            "LoadBalancer" => 0,
            "NodePort" => 1,
            "ClusterIP" => 2,
            _ => 3,
        };

        let a_priority = type_priority(&a.service_type);
        let b_priority = type_priority(&b.service_type);

        // First sort by type priority, then by name
        a_priority
            .cmp(&b_priority)
            .then_with(|| a.name.cmp(&b.name))
    });

    // Calculate counts by service type
    let cluster_ip_count = services
        .iter()
        .filter(|s| s.service_type == "ClusterIP")
        .count();
    let load_balancer_count = services
        .iter()
        .filter(|s| s.service_type == "LoadBalancer")
        .count();
    let node_port_count = services
        .iter()
        .filter(|s| s.service_type == "NodePort")
        .count();

    let (firing_alerts_count, insights_count, warning_events_count) =
        get_alerts_and_insights_counts(&state.cache).await;

    let template = ServicesTemplate {
        services,
        cluster_ip_count,
        load_balancer_count,
        node_port_count,
        active_page: "services".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        firing_alerts_count,
        insights_count,
        warning_events_count,
    };
    Html(template.render().unwrap()).into_response()
}

/// Service detail page
pub async fn service_detail(
    State(state): State<AppState>,
    axum::extract::Path((namespace, name)): axum::extract::Path<(String, String)>,
) -> impl IntoResponse {
    match state.cache.get_service_detail(&namespace, &name).await {
        Some(service) => {
            let (firing_alerts_count, insights_count, warning_events_count) =
                get_alerts_and_insights_counts(&state.cache).await;

            let template = super::templates::ServiceDetailTemplate {
                service,
                active_page: "services".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
                firing_alerts_count,
                insights_count,
                warning_events_count,
            };

            Html(template.render().unwrap()).into_response()
        }
        None => Html(format!("Service {}/{} not found", namespace, name)).into_response(),
    }
}

/// Cilium page
pub async fn cilium(State(state): State<AppState>) -> impl IntoResponse {
    let cache_ready = state.cache.is_ready().await;

    if !cache_ready {
        return preloader_page().into_response();
    }

    let (firing_alerts_count, insights_count, warning_events_count) =
        get_alerts_and_insights_counts(&state.cache).await;

    // Get pre-serialized metrics JSON from cache (same as API endpoint)
    let metrics_json = state
        .cache
        .get_cilium_metrics_json_cache()
        .await
        .to_string();

    // Get Cilium pod data for table display
    let (cilium_pods, cilium_version, hubble_enabled, ipv6_enabled) =
        state.cache.get_cilium_data().await;

    // Extract pod names for server-side legend rendering
    let mut pod_names: Vec<String> = cilium_pods.iter().map(|p| p.name.clone()).collect();
    pod_names.sort();

    let template = CiliumTemplate {
        active_page: "cilium".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        cilium_pods: cilium_pods.as_ref(),
        cilium_version: cilium_version.to_string(),
        hubble_enabled,
        ipv6_enabled,
        metrics_json,
        pod_names,
        firing_alerts_count,
        insights_count,
        warning_events_count,
    };

    Html(template.render().unwrap()).into_response()
}

/// Envoy page - shows Envoy L7 metrics
pub async fn envoy(State(state): State<AppState>) -> impl IntoResponse {
    let cache_ready = state.cache.is_ready().await;

    if !cache_ready {
        return preloader_page().into_response();
    }

    let (firing_alerts_count, insights_count, warning_events_count) =
        get_alerts_and_insights_counts(&state.cache).await;

    // Get pre-serialized metrics JSON from cache
    let metrics_json = state.cache.get_envoy_metrics_json_cache().await.to_string();

    // Get Envoy pod data for table display
    let (envoy_pods, envoy_version) = state.cache.get_envoy_data().await;

    let template = EnvoyTemplate {
        active_page: "envoy".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        pods: envoy_pods.as_ref(),
        envoy_version: envoy_version.to_string(),
        metrics_json,
        firing_alerts_count,
        insights_count,
        warning_events_count,
    };

    Html(template.render().unwrap()).into_response()
}

/// API endpoint to get Envoy metrics JSON
pub async fn api_envoy_metrics(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let cache_ready = state.cache.is_ready().await;

    if !cache_ready {
        return Json(serde_json::json!({
            "error": "Cache not ready"
        }))
        .into_response();
    }

    // Use pre-serialized JSON cache
    let json_cache = state.cache.get_envoy_metrics_json_cache().await;
    let json_string = json_cache.as_ref();

    // OPTIMIZATION: Add ETag support for cache validation
    let etag = generate_etag(json_string);
    if check_etag(&headers, &etag) {
        return (StatusCode::NOT_MODIFIED, "").into_response();
    }

    (
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/json"),
            ),
            (header::ETAG, HeaderValue::from_str(&etag).unwrap()),
        ],
        json_cache.to_string(),
    )
        .into_response()
}

/// Alerts page - shows all Prometheus alerts
pub async fn alerts(State(state): State<AppState>) -> impl IntoResponse {
    let cache_ready = state.cache.is_ready().await;

    if !cache_ready {
        return preloader_page().into_response();
    }

    // Get pre-sorted alerts from cache
    let alerts = state.cache.get_alerts().await;

    // Count alert states
    let firing_count = alerts.iter().filter(|a| a.state == "firing").count();
    let pending_count = alerts.iter().filter(|a| a.state == "pending").count();

    let insights_count = state.cache.get_insights().await.len();
    let (_, warning_events_count, _) = state.cache.get_events().await;
    let template = AlertsTemplate {
        active_page: "alerts".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        alerts: &alerts,
        firing_count,
        pending_count,
        firing_alerts_count: firing_count,
        insights_count,
        warning_events_count,
    };

    Html(template.render().unwrap()).into_response()
}

/// Insights page - shows cluster best practice recommendations
pub async fn insights(State(state): State<AppState>) -> impl IntoResponse {
    let cache_ready = state.cache.is_ready().await;

    if !cache_ready {
        return preloader_page().into_response();
    }

    // Get pre-sorted insights from cache
    let insights = state.cache.get_insights().await;

    // Count severity levels
    let high_count = insights.iter().filter(|i| i.severity == "high").count();
    let medium_count = insights.iter().filter(|i| i.severity == "medium").count();
    let low_count = insights.iter().filter(|i| i.severity == "low").count();

    let firing_alerts_count = state
        .cache
        .get_alerts()
        .await
        .iter()
        .filter(|a| a.state == "firing")
        .count();
    let insights_count = insights.len();
    let (_, warning_events_count, _) = state.cache.get_events().await;
    let template = InsightsTemplate {
        active_page: "insights".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        insights: &insights,
        high_count,
        medium_count,
        low_count,
        firing_alerts_count,
        insights_count,
        warning_events_count,
    };

    Html(template.render().unwrap()).into_response()
}

/// Events page
pub async fn events(State(state): State<AppState>) -> impl IntoResponse {
    let cache_ready = state.cache.is_ready().await;

    if !cache_ready {
        return preloader_page().into_response();
    }

    // Get events and counts from cache
    let (events, warning_count, normal_count) = state.cache.get_events().await;

    let (firing_alerts_count, insights_count, warning_events_count) =
        get_alerts_and_insights_counts(&state.cache).await;

    let template = EventsTemplate {
        active_page: "events".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        events: events.to_vec(),
        warning_count,
        normal_count,
        firing_alerts_count,
        insights_count,
        warning_events_count,
    };

    Html(template.render().unwrap()).into_response()
}

/// Render the deployments list page
pub async fn deployments(State(state): State<AppState>) -> impl IntoResponse {
    let cache_ready = state.cache.is_ready().await;

    if !cache_ready {
        return preloader_page().into_response();
    }

    // Get deployments and counts from cache
    let (deployments, available_count, progressing_count, unavailable_count) =
        state.cache.get_deployments().await;

    let (firing_alerts_count, insights_count, warning_events_count) =
        get_alerts_and_insights_counts(&state.cache).await;

    let template = DeploymentsTemplate {
        active_page: "deployments".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        deployments: deployments.to_vec(),
        available_count,
        progressing_count,
        unavailable_count,
        firing_alerts_count,
        insights_count,
        warning_events_count,
    };

    Html(template.render().unwrap()).into_response()
}

/// Render the deployment detail page
pub async fn deployment_detail(
    State(state): State<AppState>,
    axum::extract::Path((namespace, name)): axum::extract::Path<(String, String)>,
) -> impl IntoResponse {
    match state.cache.get_deployment_detail(&namespace, &name).await {
        Some(deployment) => {
            let (firing_alerts_count, insights_count, warning_events_count) =
                get_alerts_and_insights_counts(&state.cache).await;

            let template = DeploymentDetailTemplate {
                deployment,
                active_page: "deployments".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
                firing_alerts_count,
                insights_count,
                warning_events_count,
            };

            Html(template.render().unwrap()).into_response()
        }
        None => Html(format!("Deployment {}/{} not found", namespace, name)).into_response(),
    }
}

/// API endpoint for insights data
pub async fn api_insights(State(state): State<AppState>) -> impl IntoResponse {
    let cache_ready = state.cache.is_ready().await;

    if !cache_ready {
        return Json(serde_json::json!({
            "error": "Cache not ready"
        }))
        .into_response();
    }

    // Get pre-sorted insights from cache
    let insights = state.cache.get_insights().await;

    // Count severity levels
    let high_count = insights.iter().filter(|i| i.severity == "high").count();
    let medium_count = insights.iter().filter(|i| i.severity == "medium").count();
    let low_count = insights.iter().filter(|i| i.severity == "low").count();

    Json(serde_json::json!({
        "insights": insights.as_ref(),
        "high_count": high_count,
        "medium_count": medium_count,
        "low_count": low_count,
        "total_count": insights.len()
    }))
    .into_response()
}

/// API endpoint for alerts data
pub async fn api_alerts(State(state): State<AppState>) -> impl IntoResponse {
    let cache_ready = state.cache.is_ready().await;

    if !cache_ready {
        return Json(serde_json::json!({
            "error": "Cache not ready"
        }))
        .into_response();
    }

    // Get pre-sorted alerts from cache
    let alerts = state.cache.get_alerts().await;

    // Count alert states
    let firing_count = alerts.iter().filter(|a| a.state == "firing").count();
    let pending_count = alerts.iter().filter(|a| a.state == "pending").count();

    Json(serde_json::json!({
        "alerts": alerts.as_ref(),
        "firing_count": firing_count,
        "pending_count": pending_count,
        "total_count": alerts.len()
    }))
    .into_response()
}

/// API endpoint for Cilium metrics data
pub async fn api_cilium_metrics(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let cache_ready = state.cache.is_ready().await;

    if !cache_ready {
        return Json(serde_json::json!({
            "error": "Cache not ready"
        }))
        .into_response();
    }

    // Use pre-serialized JSON cache (BLAZING FAST!)
    let json_cache = state.cache.get_cilium_metrics_json_cache().await;
    let json_string = json_cache.as_ref();

    // OPTIMIZATION: Add ETag support for cache validation
    let etag = generate_etag(json_string);
    if check_etag(&headers, &etag) {
        return (StatusCode::NOT_MODIFIED, "").into_response();
    }

    (
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/json"),
            ),
            (header::ETAG, HeaderValue::from_str(&etag).unwrap()),
        ],
        json_cache.to_string(),
    )
        .into_response()
}
