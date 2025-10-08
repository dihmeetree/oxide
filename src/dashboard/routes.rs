/// HTTP route handlers
use askama::Template;
use axum::{
    extract::State,
    response::{Html, IntoResponse, Json, Redirect},
    Form,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::{error, info};

use super::server::AppState;
use super::templates::{
    CiliumPod, CiliumTemplate, ClusterDetailTemplate, ClustersTemplate, CreateClusterTemplate,
    IndexTemplate, MetricsTemplate, NodeDetailTemplate, PodDetailTemplate, PodsTemplate,
};
use crate::config::ClusterConfig;

/// Returns the preloader HTML page with auto-refresh
fn preloader_page() -> Html<String> {
    Html(r#"
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
                <h2 style="font-size: 1.75rem; font-weight: 600; margin-bottom: 12px; color: #E5E5E5;">Populating cache...</h2>
                <p style="color: #888888; font-size: 0.875rem;">Refreshing in <span id="countdown" style="display: inline-block; min-width: 1ch; text-align: center;">5</span> seconds...</p>
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
    "#.to_string())
}

/// Home page
pub async fn index(State(state): State<AppState>) -> impl IntoResponse {
    let cache_ready = state.cache.is_ready().await;

    if !cache_ready {
        return preloader_page().into_response();
    }

    let clusters = state.cache.get_clusters().await;
    let total_nodes: usize = clusters.iter().map(|c| c.nodes).sum();
    let template = IndexTemplate {
        cluster_count: clusters.len(),
        total_nodes,
        active_page: "dashboard".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    };
    Html(template.render().unwrap()).into_response()
}

/// Clusters list page
pub async fn clusters_list(State(state): State<AppState>) -> impl IntoResponse {
    let cache_ready = state.cache.is_ready().await;

    if !cache_ready {
        return preloader_page().into_response();
    }

    let clusters = state.cache.get_clusters().await;
    let template = ClustersTemplate {
        clusters,
        active_page: "clusters".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    };
    Html(template.render().unwrap()).into_response()
}

/// Create cluster form page
pub async fn clusters_create_form(State(_state): State<AppState>) -> impl IntoResponse {
    let template = CreateClusterTemplate {
        active_page: "clusters".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
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
            let template = ClusterDetailTemplate {
                cluster,
                active_page: "clusters".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
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
            let template = NodeDetailTemplate {
                node,
                active_page: "clusters".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
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

            // Build metrics JSON
            let mut all_timestamps = std::collections::BTreeSet::new();
            for (ts, _) in &metrics.cpu_history {
                all_timestamps.insert(*ts);
            }

            let timestamps: Vec<i64> = all_timestamps.iter().copied().collect();
            let cpu_history: Vec<f64> = metrics.cpu_history.iter().map(|(_, val)| *val).collect();
            let memory_history: Vec<f64> =
                metrics.memory_history.iter().map(|(_, val)| *val).collect();

            let metrics_json = serde_json::json!({
                "timestamps": timestamps,
                "cpu_history": cpu_history,
                "memory_history": memory_history,
            });

            let template = PodDetailTemplate {
                pod,
                metrics_json: serde_json::to_string(&metrics_json)
                    .unwrap_or_else(|_| "{}".to_string()),
                active_page: "pods".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
            };
            Html(template.render().unwrap()).into_response()
        }
        None => preloader_page().into_response(),
    }
}

/// Create cluster POST handler
pub async fn clusters_create(
    State(state): State<AppState>,
    Form(form): Form<CreateClusterForm>,
) -> impl IntoResponse {
    info!("Creating cluster: {}", form.cluster_name);

    // Validate cluster name
    if !form
        .cluster_name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        error!("Invalid cluster name: must be lowercase, numbers, and hyphens only");
        return Redirect::to("/clusters/create");
    }

    // Create cluster config from form data
    let config = create_cluster_config_from_form(&form);

    // Write config to temporary file
    let temp_config_path = std::env::temp_dir().join(format!("{}.yaml", form.cluster_name));
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
    tokio::spawn(async move {
        info!("Starting cluster creation for: {}", form.cluster_name);
        match crate::cluster::Cluster::create(&temp_config_path, &output_dir).await {
            Ok(_) => {
                info!("[OK] Cluster {} created successfully", form.cluster_name);
                // Clean up temp config
                let _ = tokio::fs::remove_file(&temp_config_path).await;
            }
            Err(e) => {
                error!("Failed to create cluster {}: {}", form.cluster_name, e);
            }
        }
    });

    info!("Cluster creation started in background, redirecting to clusters list");
    Redirect::to("/clusters")
}

/// API endpoint: List clusters as JSON
pub async fn api_clusters_list(State(state): State<AppState>) -> impl IntoResponse {
    let clusters = state.cache.get_clusters().await;
    Json(clusters)
}

/// API endpoint: Get pod metrics
pub async fn api_pod_metrics(
    State(state): State<AppState>,
    axum::extract::Path((namespace, pod_name)): axum::extract::Path<(String, String)>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> impl IntoResponse {
    let time_range = params.get("range").map(|s| s.as_str()).unwrap_or("1h");
    let kubeconfig = state.output_dir.join("kubeconfig");

    let history = crate::prometheus::query_pod_metrics_range(
        &namespace,
        &pod_name,
        &kubeconfig,
        time_range,
        "1m",
    )
    .await
    .unwrap_or_default();

    // Collect timestamps and values
    let mut all_timestamps = std::collections::BTreeSet::new();
    for (ts, _) in &history.cpu_history {
        all_timestamps.insert(*ts);
    }

    let timestamps: Vec<i64> = all_timestamps.iter().copied().collect();
    let cpu_history: Vec<f64> = history.cpu_history.iter().map(|(_, val)| *val).collect();
    let memory_history: Vec<f64> = history.memory_history.iter().map(|(_, val)| *val).collect();

    Json(serde_json::json!({
        "timestamps": timestamps,
        "cpu_history": cpu_history,
        "memory_history": memory_history,
    }))
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

    // Build metrics response
    let metrics_json = if has_data {
        let results: Vec<(String, crate::prometheus::NodeMetricsHistory)> =
            node_metrics_history.into_iter().collect();

        // Collect timestamps and build node metrics using helpers
        let timestamps = collect_timestamps(&results);
        let nodes = build_node_metrics(results);

        // Build pods data using helper
        let pods = build_pod_metrics(pod_metrics_history);

        let response = MetricsResponse {
            timestamps,
            nodes,
            pods,
        };
        serde_json::to_string(&response).unwrap_or_else(|_| "{}".to_string())
    } else {
        "{}".to_string()
    };

    let template = MetricsTemplate {
        active_page: "metrics".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        has_data,
        metrics_json,
    };
    Html(template.render().unwrap()).into_response()
}

/// API: Get metrics data for graphs
pub async fn api_metrics(
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> impl IntoResponse {
    // Get time range from query params (default to 1h)
    let time_range = params.get("range").map(|s| s.as_str()).unwrap_or("1h");

    // For ranges other than 1h, fetch fresh data from Prometheus
    let results: Vec<(String, crate::prometheus::NodeMetricsHistory)> = if time_range != "1h" {
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

        futures::future::join_all(fetch_tasks).await
    } else {
        // Use cached 1h data
        let metrics_history = state.cache.get_node_metrics_history().await;
        metrics_history.into_iter().collect()
    };

    // Collect timestamps and build node metrics using helpers
    let timestamps = collect_timestamps(&results);
    let nodes = build_node_metrics(results);

    // Get pod metrics history (fetch separately as node metrics may come from Prometheus)
    let pod_metrics_history = state.cache.get_pod_metrics_history().await;
    let pods = build_pod_metrics(pod_metrics_history);

    let response = MetricsResponse {
        timestamps,
        nodes,
        pods,
    };

    Json(response)
}

#[derive(Debug, Serialize)]
struct MetricsResponse {
    timestamps: Vec<i64>,
    nodes: Vec<MetricsNode>,
    pods: Vec<MetricsPod>,
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

    // Sort by CPU usage (highest to lowest)
    pods.sort_by(|a, b| {
        let cpu_a = a.cpu.trim_end_matches('m').parse::<f64>().unwrap_or(-1.0);
        let cpu_b = b.cpu.trim_end_matches('m').parse::<f64>().unwrap_or(-1.0);
        cpu_b
            .partial_cmp(&cpu_a)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let template = PodsTemplate {
        pods,
        running_count,
        pending_count,
        failed_count,
        active_page: "pods".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    };
    Html(template.render().unwrap()).into_response()
}

/// Build pod metrics from history data
fn build_pod_metrics(
    pod_metrics_history: std::collections::HashMap<String, crate::prometheus::NodeMetricsHistory>,
) -> Vec<MetricsPod> {
    pod_metrics_history
        .into_iter()
        .map(|(key, history)| {
            let parts: Vec<&str> = key.split('/').collect();
            let namespace = parts.first().unwrap_or(&"unknown").to_string();
            let name = parts.get(1).unwrap_or(&"unknown").to_string();

            // Convert CPU from percentage to millicores (multiply by 10)
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
        .collect()
}

/// Build node metrics from history data
fn build_node_metrics(
    results: Vec<(String, crate::prometheus::NodeMetricsHistory)>,
) -> Vec<MetricsNode> {
    results
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
        .collect()
}

/// Collect unique timestamps from node metrics history
fn collect_timestamps(results: &[(String, crate::prometheus::NodeMetricsHistory)]) -> Vec<i64> {
    let mut all_timestamps = std::collections::BTreeSet::new();
    for (_, history) in results {
        for (ts, _) in &history.cpu_history {
            all_timestamps.insert(*ts);
        }
    }
    all_timestamps.iter().copied().collect()
}

/// Build Cilium metrics JSON from cache data
fn build_cilium_metrics_json(
    pod_metrics_history: &std::collections::HashMap<String, crate::prometheus::NodeMetricsHistory>,
    cilium_pods: &[CiliumPod],
) -> serde_json::Value {
    // Filter for only Cilium pods (in kube-system namespace with cilium- prefix)
    let cilium_metrics: Vec<(&String, &crate::prometheus::NodeMetricsHistory)> =
        pod_metrics_history
            .iter()
            .filter(|(key, _)| key.starts_with("kube-system/cilium-"))
            .collect();

    if cilium_metrics.is_empty() {
        return serde_json::json!({
            "timestamps": [],
            "pods": []
        });
    }

    // Get timestamps from the first pod (all should have same timestamps)
    let timestamps: Vec<i64> = cilium_metrics
        .first()
        .map(|(_, history)| history.cpu_history.iter().map(|(ts, _)| *ts).collect())
        .unwrap_or_default();

    // Build pods metrics data
    let pods_metrics: Vec<serde_json::Value> = cilium_metrics
        .iter()
        .map(|(key, history)| {
            let parts: Vec<&str> = key.split('/').collect();
            let name = parts.get(1).unwrap_or(&"unknown");

            // Convert CPU from percentage to millicores (multiply by 10)
            let cpu_history: Vec<f64> = history
                .cpu_history
                .iter()
                .map(|(_, val)| val * 10.0)
                .collect();
            let memory_history: Vec<f64> =
                history.memory_history.iter().map(|(_, val)| *val).collect();

            // Find matching cilium_pod for request/limit data
            let cilium_pod = cilium_pods.iter().find(|p| p.name == *name);
            let cpu_request = cilium_pod.map(|p| p.cpu_request).unwrap_or(0.0);
            let cpu_limit = cilium_pod.map(|p| p.cpu_limit).unwrap_or(0.0);
            let memory_request = cilium_pod.map(|p| p.memory_request).unwrap_or(0.0);
            let memory_limit = cilium_pod.map(|p| p.memory_limit).unwrap_or(0.0);

            serde_json::json!({
                "name": name,
                "cpu_history": cpu_history,
                "memory_history": memory_history,
                "cpu_request": cpu_request,
                "cpu_limit": cpu_limit,
                "memory_request": memory_request,
                "memory_limit": memory_limit,
            })
        })
        .collect();

    serde_json::json!({
        "timestamps": timestamps,
        "pods": pods_metrics,
    })
}

/// Cilium page
pub async fn cilium(State(state): State<AppState>) -> impl IntoResponse {
    let cache_ready = state.cache.is_ready().await;

    if !cache_ready {
        return preloader_page().into_response();
    }

    // Build template using single lock - avoids cloning large HashMaps
    let template = state
        .cache
        .with_cilium_and_pod_metrics(
            |cilium_pods, cilium_version, hubble_enabled, ipv6_enabled, pod_metrics_history| {
                let metrics_json = build_cilium_metrics_json(pod_metrics_history, cilium_pods);

                CiliumTemplate {
                    active_page: "cilium".to_string(),
                    version: env!("CARGO_PKG_VERSION").to_string(),
                    cilium_pods: cilium_pods.to_vec(), // Still need to clone for template
                    cilium_version: cilium_version.to_string(),
                    hubble_enabled,
                    ipv6_enabled,
                    metrics_json: serde_json::to_string(&metrics_json)
                        .unwrap_or_else(|_| "{}".to_string()),
                }
            },
        )
        .await;

    Html(template.render().unwrap()).into_response()
}

/// API endpoint for Cilium metrics data
pub async fn api_cilium_metrics(State(state): State<AppState>) -> impl IntoResponse {
    let cache_ready = state.cache.is_ready().await;

    if !cache_ready {
        return Json(serde_json::json!({
            "error": "Cache not ready"
        }))
        .into_response();
    }

    // Build metrics JSON using single lock - avoids cloning large HashMaps
    let metrics_json = state
        .cache
        .with_cilium_and_pod_metrics(|cilium_pods, _, _, _, pod_metrics_history| {
            build_cilium_metrics_json(pod_metrics_history, cilium_pods)
        })
        .await;

    Json(metrics_json).into_response()
}
