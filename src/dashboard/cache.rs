/// Cluster data cache with background refresh
use anyhow::Result;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tracing::{error, info};

use super::templates::{ClusterDetail, ClusterInfo, NodeDetail};
use crate::config::ClusterConfig;
use crate::hcloud::client::HetznerCloudClient;
use crate::hcloud::models::Server;
use crate::k8s::client::KubernetesClient;

/// Cache for cluster data
#[derive(Clone)]
pub struct ClusterCache {
    inner: Arc<RwLock<CacheData>>,
}

struct CacheData {
    clusters: Vec<ClusterInfo>,
    servers: Vec<Server>,
    node_details: std::collections::HashMap<String, NodeDetail>,
    node_metrics_history: std::collections::HashMap<String, crate::prometheus::NodeMetricsHistory>,
    last_update: Instant,
    is_ready: bool,
}

impl ClusterCache {
    /// Create a new empty cache
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(CacheData {
                clusters: Vec::new(),
                servers: Vec::new(),
                node_details: std::collections::HashMap::new(),
                node_metrics_history: std::collections::HashMap::new(),
                last_update: Instant::now(),
                is_ready: false,
            })),
        }
    }

    /// Get all clusters from cache
    pub async fn get_clusters(&self) -> Vec<ClusterInfo> {
        let data = self.inner.read().await;
        data.clusters.clone()
    }

    /// Get detailed cluster info from cache
    pub async fn get_cluster_detail(&self, cluster_name: &str) -> Option<ClusterDetail> {
        let data = self.inner.read().await;

        // Filter servers by cluster name
        let cluster_servers: Vec<&Server> = data
            .servers
            .iter()
            .filter(|s| {
                let parts: Vec<&str> = s.name.split('-').collect();
                parts.first() == Some(&cluster_name)
            })
            .collect();

        if cluster_servers.is_empty() {
            return None;
        }

        // Build ClusterDetail from cached servers and node details
        Some(build_cluster_detail(
            cluster_name,
            &cluster_servers,
            &data.node_details,
        ))
    }

    /// Get detailed node info with pods from cache
    pub async fn get_node_detail(
        &self,
        _cluster_name: &str,
        node_name: &str,
    ) -> Option<NodeDetail> {
        let data = self.inner.read().await;
        data.node_details.get(node_name).cloned()
    }

    /// Get all node metrics history from cache
    pub async fn get_node_metrics_history(
        &self,
    ) -> std::collections::HashMap<String, crate::prometheus::NodeMetricsHistory> {
        let data = self.inner.read().await;
        data.node_metrics_history.clone()
    }

    /// Get all node details map from cache
    pub async fn get_node_details_map(&self) -> std::collections::HashMap<String, NodeDetail> {
        let data = self.inner.read().await;
        data.node_details.clone()
    }

    /// Get cache age
    #[allow(dead_code)]
    pub async fn cache_age(&self) -> Duration {
        let data = self.inner.read().await;
        data.last_update.elapsed()
    }

    /// Check if cache has been populated at least once
    pub async fn is_ready(&self) -> bool {
        let data = self.inner.read().await;
        data.is_ready
    }

    /// Refresh cache with new data
    pub async fn refresh(&self, config_path: &std::path::Path) -> Result<()> {
        info!("Refreshing cluster cache...");

        // Load config
        let config_str = tokio::fs::read_to_string(config_path).await?;
        let config: ClusterConfig = serde_yaml::from_str(&config_str)?;
        let hcloud_token = config.get_hcloud_token()?;

        // Fetch servers from Hetzner API
        let client = HetznerCloudClient::new(hcloud_token)?;
        let servers = client.list_servers().await?;

        // Group by cluster name
        let clusters = group_servers_into_clusters(&servers);

        // Fetch node details with pods for all nodes
        let node_details = fetch_all_node_details(&servers, config_path).await;

        // Fetch historical metrics for all nodes
        let node_metrics_history = fetch_all_node_metrics_history(&servers, config_path).await;

        // Update cache
        let mut data = self.inner.write().await;
        data.clusters = clusters;
        data.servers = servers;
        data.node_details = node_details;
        data.node_metrics_history = node_metrics_history;
        data.last_update = Instant::now();
        data.is_ready = true;

        info!("Cache refreshed successfully");
        Ok(())
    }

    /// Refresh only metrics history (faster update)
    async fn refresh_metrics_only(&self, config_path: &std::path::Path) -> Result<()> {
        let data = self.inner.read().await;
        let servers = data.servers.clone();
        drop(data); // Release read lock

        if servers.is_empty() {
            return Ok(());
        }

        // Fetch historical metrics for all nodes
        let node_metrics_history = fetch_all_node_metrics_history(&servers, config_path).await;

        // Update only metrics in cache
        let mut data = self.inner.write().await;
        data.node_metrics_history = node_metrics_history;

        Ok(())
    }

    /// Start background refresh task
    pub fn start_background_refresh(&self, config_path: std::path::PathBuf, interval_secs: u64) {
        let cache = self.clone();
        let config_path_clone = config_path.clone();

        // Start cluster data refresh (every 30s by default)
        tokio::spawn(async move {
            // Do initial refresh immediately
            if let Err(e) = cache.refresh(&config_path).await {
                info!(
                    "Initial data load failed (this is OK if no clusters exist yet): {}",
                    e
                );
            }

            let mut interval = tokio::time::interval(Duration::from_secs(interval_secs));
            // Skip the first tick since we just did an initial refresh
            interval.tick().await;

            loop {
                interval.tick().await;
                if let Err(e) = cache.refresh(&config_path).await {
                    error!("Failed to refresh cache: {}", e);
                }
            }
        });

        // Start metrics-only refresh (every 10s)
        let cache_metrics = self.clone();
        tokio::spawn(async move {
            // Wait for first cluster refresh to complete
            tokio::time::sleep(Duration::from_secs(5)).await;

            let mut interval = tokio::time::interval(Duration::from_secs(10));
            loop {
                interval.tick().await;
                if let Err(e) = cache_metrics.refresh_metrics_only(&config_path_clone).await {
                    error!("Failed to refresh metrics: {}", e);
                }
            }
        });
    }
}

/// Group servers into cluster info
fn group_servers_into_clusters(servers: &[Server]) -> Vec<ClusterInfo> {
    use std::collections::HashMap;

    let mut clusters: HashMap<String, Vec<&Server>> = HashMap::new();
    for server in servers {
        let parts: Vec<&str> = server.name.split('-').collect();
        if parts.len() >= 2 {
            let cluster_name = parts[0].to_string();
            clusters.entry(cluster_name).or_default().push(server);
        }
    }

    let mut cluster_infos: Vec<ClusterInfo> = clusters
        .into_iter()
        .map(|(name, servers)| {
            let status = if servers.iter().all(|s| s.status == "running") {
                "Running".to_string()
            } else {
                "Partial".to_string()
            };

            let version = servers
                .first()
                .and_then(|s| s.labels.get("talos-version"))
                .cloned()
                .unwrap_or_else(|| "Unknown".to_string());

            let created = servers
                .iter()
                .map(|s| &s.created)
                .min()
                .map(|s| s.split('T').next().unwrap_or("Unknown"))
                .unwrap_or("Unknown")
                .to_string();

            ClusterInfo {
                name,
                status,
                nodes: servers.len(),
                version,
                created,
            }
        })
        .collect();

    cluster_infos.sort_by(|a, b| a.name.cmp(&b.name));
    cluster_infos
}

/// Fetch node details with pods for all servers
async fn fetch_all_node_details(
    servers: &[Server],
    config_path: &std::path::Path,
) -> std::collections::HashMap<String, NodeDetail> {
    use std::collections::HashMap;

    let mut node_details = HashMap::new();

    // Get the output directory from config path
    let output_dir = config_path
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.join("output"))
        .unwrap_or_else(|| std::path::PathBuf::from("output"));

    let kubeconfig = output_dir.join("kubeconfig");

    // Check if kubeconfig exists
    if !kubeconfig.exists() {
        info!("Kubeconfig not found, skipping pod data fetch");
        return node_details;
    }

    // Fetch pods for all nodes in parallel
    let fetch_tasks: Vec<_> = servers
        .iter()
        .map(|server| {
            let kubeconfig = kubeconfig.clone();
            let server = server.clone();
            async move {
                let cluster_name = server
                    .name
                    .split('-')
                    .next()
                    .unwrap_or("unknown")
                    .to_string();

                let role = if server.name.contains("control-plane") {
                    "Control Plane".to_string()
                } else {
                    "Worker".to_string()
                };

                let ip = server
                    .public_net
                    .ipv4
                    .as_ref()
                    .map(|ipv4| ipv4.ip.clone())
                    .unwrap_or_else(|| "N/A".to_string());

                let private_ip = server
                    .private_net
                    .first()
                    .map(|net| net.ip.clone())
                    .unwrap_or_else(|| "N/A".to_string());

                // Get pods from Kubernetes
                let pods = KubernetesClient::get_pods_on_node(&kubeconfig, &server.name)
                    .await
                    .unwrap_or_else(|e| {
                        error!("Failed to get pods for node {}: {}", server.name, e);
                        Vec::new()
                    });

                // Get metrics from Prometheus using private IP
                let metrics = crate::prometheus::query_node_metrics(&private_ip, &kubeconfig)
                    .await
                    .unwrap_or_default();

                let cpu_usage_percent = if metrics.cpu_usage_percent > 0.0 {
                    format!("{:.1}%", metrics.cpu_usage_percent)
                } else {
                    "N/A".to_string()
                };

                let memory_usage_percent = if metrics.memory_usage_percent > 0.0 {
                    format!("{:.1}%", metrics.memory_usage_percent)
                } else {
                    "N/A".to_string()
                };

                let memory_used_gb = if metrics.memory_used_bytes > 0 {
                    format!(
                        "{:.2}",
                        metrics.memory_used_bytes as f64 / 1024.0 / 1024.0 / 1024.0
                    )
                } else {
                    "N/A".to_string()
                };

                let memory_total_gb = if metrics.memory_total_bytes > 0 {
                    format!(
                        "{:.2}",
                        metrics.memory_total_bytes as f64 / 1024.0 / 1024.0 / 1024.0
                    )
                } else {
                    "N/A".to_string()
                };

                (
                    server.name.clone(),
                    NodeDetail {
                        cluster_name,
                        name: server.name.clone(),
                        role,
                        ip,
                        private_ip,
                        status: server.status.clone(),
                        server_type: server.server_type.name.clone(),
                        created: server
                            .created
                            .split('T')
                            .next()
                            .unwrap_or("Unknown")
                            .to_string(),
                        pods,
                        cpu_usage_percent,
                        memory_usage_percent,
                        memory_used_gb,
                        memory_total_gb,
                    },
                )
            }
        })
        .collect();

    // Execute all fetch tasks in parallel
    let results = futures::future::join_all(fetch_tasks).await;

    // Collect results into HashMap
    for (name, detail) in results {
        node_details.insert(name, detail);
    }

    node_details
}

/// Fetch historical metrics for all nodes
async fn fetch_all_node_metrics_history(
    servers: &[Server],
    config_path: &std::path::Path,
) -> std::collections::HashMap<String, crate::prometheus::NodeMetricsHistory> {
    let mut metrics_history = std::collections::HashMap::new();

    let output_dir = config_path
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.join("output"))
        .unwrap_or_else(|| std::path::PathBuf::from("output"));

    let kubeconfig = output_dir.join("kubeconfig");

    // Check if kubeconfig exists
    if !kubeconfig.exists() {
        info!("Kubeconfig not found, skipping metrics history fetch");
        return metrics_history;
    }

    // Fetch metrics history for all nodes in parallel
    let fetch_tasks: Vec<_> = servers
        .iter()
        .map(|server| {
            let kubeconfig = kubeconfig.clone();
            let server = server.clone();
            async move {
                let private_ip = server
                    .private_net
                    .first()
                    .map(|net| net.ip.clone())
                    .unwrap_or_else(|| "N/A".to_string());

                let history = crate::prometheus::query_node_metrics_range(
                    &private_ip,
                    &kubeconfig,
                    "1h",
                    "1m",
                )
                .await
                .unwrap_or_default();

                (server.name.clone(), history)
            }
        })
        .collect();

    // Execute all fetch tasks in parallel
    let results = futures::future::join_all(fetch_tasks).await;

    // Collect results into HashMap
    for (name, history) in results {
        metrics_history.insert(name, history);
    }

    metrics_history
}

/// Build detailed cluster info
fn build_cluster_detail(
    cluster_name: &str,
    cluster_servers: &[&Server],
    node_details: &std::collections::HashMap<String, NodeDetail>,
) -> ClusterDetail {
    use super::templates::NodeInfo;

    let status = if cluster_servers.iter().all(|s| s.status == "running") {
        "Running".to_string()
    } else {
        "Partial".to_string()
    };

    let version = cluster_servers
        .first()
        .and_then(|s| s.labels.get("talos-version"))
        .cloned()
        .unwrap_or_else(|| "Unknown".to_string());

    let created = cluster_servers
        .iter()
        .map(|s| &s.created)
        .min()
        .map(|s| s.split('T').next().unwrap_or("Unknown"))
        .unwrap_or("Unknown")
        .to_string();

    let endpoint = cluster_servers
        .iter()
        .find(|s| s.name.contains("control-plane"))
        .and_then(|s| s.public_net.ipv4.as_ref())
        .map(|ipv4| format!("https://{}:6443", ipv4.ip))
        .unwrap_or_else(|| "N/A".to_string());

    let nodes: Vec<NodeInfo> = cluster_servers
        .iter()
        .map(|server| {
            let role = if server.name.contains("control-plane") {
                "Control Plane".to_string()
            } else {
                "Worker".to_string()
            };

            let ip = server
                .public_net
                .ipv4
                .as_ref()
                .map(|ipv4| ipv4.ip.clone())
                .unwrap_or_else(|| "N/A".to_string());

            let private_ip = server
                .private_net
                .first()
                .map(|net| net.ip.clone())
                .unwrap_or_else(|| "N/A".to_string());

            // Get metrics from node_details if available
            let (cpu_usage_percent, memory_usage_percent) =
                if let Some(detail) = node_details.get(&server.name) {
                    (
                        detail.cpu_usage_percent.clone(),
                        detail.memory_usage_percent.clone(),
                    )
                } else {
                    ("N/A".to_string(), "N/A".to_string())
                };

            NodeInfo {
                name: server.name.clone(),
                role,
                ip,
                private_ip,
                status: server.status.clone(),
                server_type: server.server_type.name.clone(),
                created: server
                    .created
                    .split('T')
                    .next()
                    .unwrap_or("Unknown")
                    .to_string(),
                cpu_usage_percent,
                memory_usage_percent,
            }
        })
        .collect();

    let control_plane_count = nodes.iter().filter(|n| n.role == "Control Plane").count();
    let worker_count = nodes.iter().filter(|n| n.role == "Worker").count();

    ClusterDetail {
        name: cluster_name.to_string(),
        status,
        version,
        created,
        nodes,
        endpoint,
        control_plane_count,
        worker_count,
    }
}
