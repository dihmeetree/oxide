/// Cluster data cache with background refresh
use anyhow::Result;
use dashmap::DashMap;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tracing::{error, info};

use crate::dashboard::templates::CiliumPod;

// Constants for metrics and caching
const METRICS_HISTORY_MAX_AGE_SECS: i64 = 7200; // 2 hours max history
const TIMESTAMP_ROUNDING_SECS: i64 = 60; // Round timestamps to nearest minute
const TIMESTAMP_TOLERANCE_SECS: i64 = 60; // Tolerance for timestamp matching
const CPU_TO_MILLICORES_MULTIPLIER: f64 = 10.0; // Prometheus CPU to millicores conversion
const KUBERNETES_REFRESH_INTERVAL_SECS: u64 = 30; // Fast refresh for K8s data
const KUBERNETES_REFRESH_INITIAL_DELAY_SECS: u64 = 5; // Wait before starting K8s refresh

/// Calculate human-readable age from ISO 8601 timestamp
fn calculate_age(timestamp: &str) -> String {
    use chrono::{DateTime, Utc};

    let created = match DateTime::parse_from_rfc3339(timestamp) {
        Ok(dt) => dt.with_timezone(&Utc),
        Err(_) => return "N/A".to_string(),
    };

    let now = Utc::now();
    let duration = now.signed_duration_since(created);

    let days = duration.num_days();
    let hours = duration.num_hours() % 24;
    let minutes = duration.num_minutes() % 60;

    if days > 0 {
        format!("{}d", days)
    } else if hours > 0 {
        format!("{}h", hours)
    } else {
        format!("{}m", minutes)
    }
}

/// Parse CPU resource string to millicores
fn parse_cpu_resource(value: &str) -> Option<f64> {
    if value.is_empty() || value == "-" {
        return None;
    }

    if let Some(stripped) = value.strip_suffix('m') {
        // Already in millicores (e.g., "100m")
        stripped.parse::<f64>().ok()
    } else {
        // Cores to millicores (e.g., "1" = 1000m)
        value.parse::<f64>().ok().map(|v| v * 1000.0)
    }
}

/// Parse memory resource string to MiB
fn parse_memory_resource(value: &str) -> Option<f64> {
    if value.is_empty() || value == "-" {
        return None;
    }

    if let Some(stripped) = value.strip_suffix("Ki") {
        // Kibibytes to MiB
        stripped.parse::<f64>().ok().map(|v| v / 1024.0)
    } else if let Some(stripped) = value.strip_suffix("Mi") {
        // Already in MiB
        stripped.parse::<f64>().ok()
    } else if let Some(stripped) = value.strip_suffix("Gi") {
        // Gibibytes to MiB
        stripped.parse::<f64>().ok().map(|v| v * 1024.0)
    } else if let Some(stripped) = value.strip_suffix('K') {
        // Kilobytes to MiB
        stripped.parse::<f64>().ok().map(|v| v / 1024.0)
    } else if let Some(stripped) = value.strip_suffix('M') {
        // Megabytes to MiB
        stripped.parse::<f64>().ok()
    } else if let Some(stripped) = value.strip_suffix('G') {
        // Gigabytes to MiB
        stripped.parse::<f64>().ok().map(|v| v * 1024.0)
    } else {
        // Bytes to MiB
        value.parse::<f64>().ok().map(|v| v / (1024.0 * 1024.0))
    }
}

/// Update pod details with metrics from history (consolidated function to avoid duplication)
fn update_pod_details_with_metrics(
    pod_details: &mut HashMap<String, super::templates::PodDetail>,
    pod_metrics_history: &HashMap<String, crate::prometheus::NodeMetricsHistory>,
) {
    for (key, history) in pod_metrics_history {
        if let Some(pod_detail) = pod_details.get_mut(key) {
            // Calculate total limits and requests from containers
            let mut total_cpu_limit = 0.0;
            let mut total_cpu_request = 0.0;
            let mut total_memory_limit = 0.0;
            let mut total_memory_request = 0.0;

            for container in &pod_detail.containers {
                // Parse CPU limit/request (handle "m" suffix and plain numbers)
                if let Ok(limit) = container.cpu_limit.trim_end_matches('m').parse::<f64>() {
                    total_cpu_limit += limit;
                }
                if let Ok(request) = container.cpu_request.trim_end_matches('m').parse::<f64>() {
                    total_cpu_request += request;
                }

                // Parse memory limit/request (handle "Mi" and "Gi" suffixes)
                if let Some(val) = container.memory_limit.strip_suffix("Gi") {
                    if let Ok(gib) = val.parse::<f64>() {
                        total_memory_limit += gib * 1024.0;
                    }
                } else if let Some(val) = container.memory_limit.strip_suffix("Mi") {
                    if let Ok(mib) = val.parse::<f64>() {
                        total_memory_limit += mib;
                    }
                }

                if let Some(val) = container.memory_request.strip_suffix("Gi") {
                    if let Ok(gib) = val.parse::<f64>() {
                        total_memory_request += gib * 1024.0;
                    }
                } else if let Some(val) = container.memory_request.strip_suffix("Mi") {
                    if let Ok(mib) = val.parse::<f64>() {
                        total_memory_request += mib;
                    }
                }
            }

            // Update CPU usage and calculate percentage
            if let Some(&(_, cpu_value)) = history.cpu_history.last() {
                let cpu_usage_m = (cpu_value * CPU_TO_MILLICORES_MULTIPLIER) as u64;
                pod_detail.cpu = format!("{}m", cpu_usage_m);

                // Calculate percentage based on limit (fallback to request if no limit)
                let cpu_percent = if total_cpu_limit > 0.0 {
                    (cpu_usage_m as f64 / total_cpu_limit * 100.0) as u64
                } else if total_cpu_request > 0.0 {
                    (cpu_usage_m as f64 / total_cpu_request * 100.0) as u64
                } else {
                    0
                };
                pod_detail.cpu_percent = format!("{}%", cpu_percent);
            }

            // Update memory usage and calculate percentage
            if let Some(&(_, mem_value)) = history.memory_history.last() {
                let mem_usage_mi = mem_value as u64;
                pod_detail.memory = format!("{}Mi", mem_usage_mi);

                // Calculate percentage based on limit (fallback to request if no limit)
                let memory_percent = if total_memory_limit > 0.0 {
                    (mem_usage_mi as f64 / total_memory_limit * 100.0) as u64
                } else if total_memory_request > 0.0 {
                    (mem_usage_mi as f64 / total_memory_request * 100.0) as u64
                } else {
                    0
                };
                pod_detail.memory_percent = format!("{}%", memory_percent);
            }

            // Set limit/request strings
            pod_detail.cpu_limit = if total_cpu_limit > 0.0 {
                format!("{}m", total_cpu_limit as u64)
            } else {
                "N/A".to_string()
            };
            pod_detail.cpu_request = if total_cpu_request > 0.0 {
                format!("{}m", total_cpu_request as u64)
            } else {
                "N/A".to_string()
            };
            pod_detail.memory_limit = if total_memory_limit > 0.0 {
                format!("{}Mi", total_memory_limit as u64)
            } else {
                "N/A".to_string()
            };
            pod_detail.memory_request = if total_memory_request > 0.0 {
                format!("{}Mi", total_memory_request as u64)
            } else {
                "N/A".to_string()
            };
        }
    }
}

/// Set limit/request for pods without metrics history
fn set_pod_resource_limits(pod_details: &mut HashMap<String, super::templates::PodDetail>) {
    for pod_detail in pod_details.values_mut() {
        // Skip if already updated (cpu_limit won't be "N/A")
        if pod_detail.cpu_limit != "N/A" {
            continue;
        }

        // Calculate total limits and requests from containers
        let mut total_cpu_limit = 0.0;
        let mut total_cpu_request = 0.0;
        let mut total_memory_limit = 0.0;
        let mut total_memory_request = 0.0;

        for container in &pod_detail.containers {
            // Parse CPU limit/request
            if let Ok(limit) = container.cpu_limit.trim_end_matches('m').parse::<f64>() {
                total_cpu_limit += limit;
            }
            if let Ok(request) = container.cpu_request.trim_end_matches('m').parse::<f64>() {
                total_cpu_request += request;
            }

            // Parse memory limit/request
            if let Some(val) = container.memory_limit.strip_suffix("Gi") {
                if let Ok(gib) = val.parse::<f64>() {
                    total_memory_limit += gib * 1024.0;
                }
            } else if let Some(val) = container.memory_limit.strip_suffix("Mi") {
                if let Ok(mib) = val.parse::<f64>() {
                    total_memory_limit += mib;
                }
            }

            if let Some(val) = container.memory_request.strip_suffix("Gi") {
                if let Ok(gib) = val.parse::<f64>() {
                    total_memory_request += gib * 1024.0;
                }
            } else if let Some(val) = container.memory_request.strip_suffix("Mi") {
                if let Ok(mib) = val.parse::<f64>() {
                    total_memory_request += mib;
                }
            }
        }

        // Set limit/request strings
        pod_detail.cpu_limit = if total_cpu_limit > 0.0 {
            format!("{}m", total_cpu_limit as u64)
        } else {
            "N/A".to_string()
        };
        pod_detail.cpu_request = if total_cpu_request > 0.0 {
            format!("{}m", total_cpu_request as u64)
        } else {
            "N/A".to_string()
        };
        pod_detail.memory_limit = if total_memory_limit > 0.0 {
            format!("{}Mi", total_memory_limit as u64)
        } else {
            "N/A".to_string()
        };
        pod_detail.memory_request = if total_memory_request > 0.0 {
            format!("{}Mi", total_memory_request as u64)
        } else {
            "N/A".to_string()
        };
    }
}

/// Update Envoy pod resources from pod details
fn update_envoy_pod_resources(
    envoy_pods: &mut [super::templates::EnvoyPod],
    pod_details: &HashMap<String, super::templates::PodDetail>,
) {
    for envoy_pod in envoy_pods {
        let key = format!("{}/{}", envoy_pod.namespace, envoy_pod.name);
        if let Some(pod_detail) = pod_details.get(&key) {
            envoy_pod.cpu = pod_detail.cpu.clone();
            envoy_pod.memory = pod_detail.memory.clone();

            // Calculate total requests and limits from all containers
            let mut cpu_req_total = 0.0;
            let mut cpu_lim_total = 0.0;
            let mut mem_req_total = 0.0;
            let mut mem_lim_total = 0.0;

            for container in &pod_detail.containers {
                if let Some(val) = parse_cpu_resource(&container.cpu_request) {
                    cpu_req_total += val;
                }
                if let Some(val) = parse_cpu_resource(&container.cpu_limit) {
                    cpu_lim_total += val;
                }
                if let Some(val) = parse_memory_resource(&container.memory_request) {
                    mem_req_total += val;
                }
                if let Some(val) = parse_memory_resource(&container.memory_limit) {
                    mem_lim_total += val;
                }
            }

            envoy_pod.cpu_request = cpu_req_total;
            envoy_pod.cpu_limit = cpu_lim_total;
            envoy_pod.memory_request = mem_req_total;
            envoy_pod.memory_limit = mem_lim_total;
        }
    }
}

/// Update Cilium pod resources from pod details
fn update_cilium_pod_resources(
    cilium_pods: &mut [CiliumPod],
    pod_details: &HashMap<String, super::templates::PodDetail>,
) {
    for cilium_pod in cilium_pods {
        let key = format!("kube-system/{}", cilium_pod.name);
        if let Some(pod_detail) = pod_details.get(&key) {
            cilium_pod.cpu = pod_detail.cpu.clone();
            cilium_pod.memory = pod_detail.memory.clone();

            // Calculate total requests and limits from all containers
            let mut cpu_req_total = 0.0;
            let mut cpu_lim_total = 0.0;
            let mut mem_req_total = 0.0;
            let mut mem_lim_total = 0.0;

            for container in &pod_detail.containers {
                if let Some(val) = parse_cpu_resource(&container.cpu_request) {
                    cpu_req_total += val;
                }
                if let Some(val) = parse_cpu_resource(&container.cpu_limit) {
                    cpu_lim_total += val;
                }
                if let Some(val) = parse_memory_resource(&container.memory_request) {
                    mem_req_total += val;
                }
                if let Some(val) = parse_memory_resource(&container.memory_limit) {
                    mem_lim_total += val;
                }
            }

            cilium_pod.cpu_request = cpu_req_total;
            cilium_pod.cpu_limit = cpu_lim_total;
            cilium_pod.memory_request = mem_req_total;
            cilium_pod.memory_limit = mem_lim_total;
        }
    }
}

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
    clusters: Arc<[ClusterInfo]>,
    servers: Arc<[Server]>,
    node_details: Arc<DashMap<String, NodeDetail>>,
    node_metrics_history:
        Arc<std::collections::HashMap<String, crate::prometheus::NodeMetricsHistory>>,
    pod_details: Arc<DashMap<String, super::templates::PodDetail>>,
    pod_metrics_history:
        Arc<std::collections::HashMap<String, crate::prometheus::NodeMetricsHistory>>,
    cilium_pods: Arc<[super::templates::CiliumPod]>,
    cilium_version: Arc<str>,
    hubble_enabled: bool,
    ipv6_enabled: bool,
    envoy_pods: Arc<[super::templates::EnvoyPod]>,
    envoy_version: Arc<str>,
    envoy_metrics_history: Arc<crate::prometheus::EnvoyMetricsHistory>,
    alerts: Arc<[crate::prometheus::Alert]>,
    insights: Arc<[crate::prometheus::Insight]>,
    last_update: Instant,
    is_ready: bool,
    metrics_json_cache: Arc<str>,
    cilium_metrics_json_cache: Arc<str>,
    envoy_metrics_json_cache: Arc<str>,
}

impl ClusterCache {
    /// Create a new empty cache
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(CacheData {
                clusters: Arc::from([]),
                servers: Arc::from([]),
                node_details: Arc::new(DashMap::new()),
                node_metrics_history: Arc::new(std::collections::HashMap::new()),
                pod_details: Arc::new(DashMap::new()),
                pod_metrics_history: Arc::new(std::collections::HashMap::new()),
                cilium_pods: Arc::from([]),
                cilium_version: Arc::from("N/A"),
                hubble_enabled: false,
                ipv6_enabled: false,
                envoy_pods: Arc::from([]),
                envoy_version: Arc::from("N/A"),
                envoy_metrics_history: Arc::new(crate::prometheus::EnvoyMetricsHistory::default()),
                alerts: Arc::from([]),
                insights: Arc::from([]),
                last_update: Instant::now(),
                is_ready: false,
                metrics_json_cache: Arc::from("{}"),
                cilium_metrics_json_cache: Arc::from("{}"),
                envoy_metrics_json_cache: Arc::from("{}"),
            })),
        }
    }

    /// Get all clusters from cache
    #[inline]
    pub async fn get_clusters(&self) -> Arc<[ClusterInfo]> {
        let data = self.inner.read().await;
        Arc::clone(&data.clusters)
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
        data.node_details.get(node_name).map(|v| v.clone())
    }

    /// Get detailed pod info from cache
    pub async fn get_pod_detail(
        &self,
        _cluster_name: &str,
        _node_name: &str,
        namespace: &str,
        pod_name: &str,
    ) -> Option<super::templates::PodDetail> {
        let data = self.inner.read().await;
        let key = format!("{}/{}", namespace, pod_name);
        data.pod_details.get(&key).map(|v| v.clone())
    }

    /// Get pod metrics history from cache
    pub async fn get_pod_metrics(
        &self,
        namespace: &str,
        pod_name: &str,
    ) -> Option<crate::prometheus::NodeMetricsHistory> {
        let data = self.inner.read().await;
        let key = format!("{}/{}", namespace, pod_name);
        data.pod_metrics_history.get(&key).cloned()
    }

    /// Get all node metrics history from cache
    pub async fn get_node_metrics_history(
        &self,
    ) -> Arc<std::collections::HashMap<String, crate::prometheus::NodeMetricsHistory>> {
        let data = self.inner.read().await;
        Arc::clone(&data.node_metrics_history)
    }

    /// Process node metrics history with a closure to avoid cloning
    #[allow(dead_code)]
    pub async fn with_node_metrics_history<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&std::collections::HashMap<String, crate::prometheus::NodeMetricsHistory>) -> R,
    {
        let data = self.inner.read().await;
        f(&data.node_metrics_history)
    }

    /// Get all node details map from cache
    pub async fn get_node_details_map(&self) -> std::collections::HashMap<String, NodeDetail> {
        let data = self.inner.read().await;
        data.node_details
            .iter()
            .map(|entry| (entry.key().clone(), entry.value().clone()))
            .collect()
    }

    /// Get all pods from cache
    pub async fn get_all_pods(&self) -> Vec<super::templates::PodDetail> {
        let data = self.inner.read().await;
        data.pod_details
            .iter()
            .map(|entry| entry.value().clone())
            .collect()
    }

    /// Get all node details from cache
    pub async fn get_all_node_details(&self) -> Vec<super::templates::NodeDetail> {
        let data = self.inner.read().await;
        data.node_details
            .iter()
            .map(|entry| entry.value().clone())
            .collect()
    }

    /// Get all pod metrics history from cache
    pub async fn get_pod_metrics_history(
        &self,
    ) -> Arc<std::collections::HashMap<String, crate::prometheus::NodeMetricsHistory>> {
        let data = self.inner.read().await;
        Arc::clone(&data.pod_metrics_history)
    }

    /// Process pod metrics history with a closure to avoid cloning
    #[allow(dead_code)]
    pub async fn with_pod_metrics_history<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&std::collections::HashMap<String, crate::prometheus::NodeMetricsHistory>) -> R,
    {
        let data = self.inner.read().await;
        f(&data.pod_metrics_history)
    }

    /// Get Cilium pod information from cache
    #[allow(dead_code)]
    #[inline]
    pub async fn get_cilium_data(
        &self,
    ) -> (Arc<[super::templates::CiliumPod]>, Arc<str>, bool, bool) {
        let data = self.inner.read().await;
        (
            Arc::clone(&data.cilium_pods),
            Arc::clone(&data.cilium_version),
            data.hubble_enabled,
            data.ipv6_enabled,
        )
    }

    /// Get all alerts from cache
    #[inline]
    pub async fn get_alerts(&self) -> Arc<[crate::prometheus::Alert]> {
        let data = self.inner.read().await;
        Arc::clone(&data.alerts)
    }

    /// Get all insights from cache
    #[inline]
    pub async fn get_insights(&self) -> Arc<[crate::prometheus::Insight]> {
        let data = self.inner.read().await;
        Arc::clone(&data.insights)
    }

    /// Process Cilium data with a closure to avoid cloning
    #[allow(dead_code)]
    pub async fn with_cilium_data<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&[super::templates::CiliumPod], &str, bool, bool) -> R,
    {
        let data = self.inner.read().await;
        f(
            &data.cilium_pods,
            &data.cilium_version,
            data.hubble_enabled,
            data.ipv6_enabled,
        )
    }

    /// Process both Cilium data and pod metrics history together to avoid multiple locks
    #[allow(dead_code)]
    pub async fn with_cilium_and_pod_metrics<F, R>(&self, f: F) -> R
    where
        F: FnOnce(
            &[super::templates::CiliumPod],
            &str,
            bool,
            bool,
            &std::collections::HashMap<String, crate::prometheus::NodeMetricsHistory>,
        ) -> R,
    {
        let data = self.inner.read().await;
        f(
            &data.cilium_pods,
            &data.cilium_version,
            data.hubble_enabled,
            data.ipv6_enabled,
            &data.pod_metrics_history,
        )
    }

    /// Get both node and pod metrics history together in one lock
    pub async fn get_all_metrics_history(
        &self,
    ) -> (
        Arc<std::collections::HashMap<String, crate::prometheus::NodeMetricsHistory>>,
        Arc<std::collections::HashMap<String, crate::prometheus::NodeMetricsHistory>>,
    ) {
        let data = self.inner.read().await;
        (
            Arc::clone(&data.node_metrics_history),
            Arc::clone(&data.pod_metrics_history),
        )
    }

    /// Get pre-serialized metrics JSON from cache
    pub async fn get_metrics_json_cache(&self) -> Arc<str> {
        let data = self.inner.read().await;
        Arc::clone(&data.metrics_json_cache)
    }

    /// Get pre-serialized Cilium metrics JSON from cache
    pub async fn get_cilium_metrics_json_cache(&self) -> Arc<str> {
        let data = self.inner.read().await;
        Arc::clone(&data.cilium_metrics_json_cache)
    }

    /// Get Envoy data from cache
    #[inline]
    pub async fn get_envoy_data(&self) -> (Arc<[super::templates::EnvoyPod]>, Arc<str>) {
        let data = self.inner.read().await;
        (
            Arc::clone(&data.envoy_pods),
            Arc::clone(&data.envoy_version),
        )
    }

    /// Get pre-serialized Envoy metrics JSON from cache
    pub async fn get_envoy_metrics_json_cache(&self) -> Arc<str> {
        let data = self.inner.read().await;
        Arc::clone(&data.envoy_metrics_json_cache)
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
        info!("Starting full cache refresh (Hetzner API + Kubernetes)...");

        // Load config
        let config_str = tokio::fs::read_to_string(config_path).await?;
        let config: ClusterConfig = serde_yaml::from_str(&config_str)?;
        let hcloud_token = config.get_hcloud_token()?;

        // Fetch servers from Hetzner API
        info!("Fetching servers from Hetzner Cloud API...");
        let client = HetznerCloudClient::new(hcloud_token)?;
        let servers = client.list_servers().await?;
        info!("Retrieved {} servers from Hetzner", servers.len());

        // Group by cluster name
        let clusters = group_servers_into_clusters(&servers);

        // Fetch all Kubernetes/Prometheus data in parallel
        info!("Fetching Kubernetes and Prometheus data in parallel...");
        let (
            mut node_details,
            mut pod_details,
            pod_metrics_history,
            node_metrics_history,
            cilium_data,
            envoy_data,
            alerts,
            insights,
        ) = tokio::join!(
            fetch_all_node_details(&servers, config_path),
            fetch_all_pod_details(config_path, &config.cluster_name),
            fetch_all_pod_metrics_history(config_path),
            fetch_all_node_metrics_history(&servers, config_path),
            fetch_cilium_data(config_path, &config, &config.cluster_name),
            fetch_envoy_data(config_path, &config.cluster_name),
            fetch_alerts(config_path),
            fetch_insights(config_path)
        );

        // Unpack Cilium data
        let (mut cilium_pods, cilium_version, hubble_enabled, ipv6_enabled) = cilium_data;

        // Unpack Envoy data
        let (mut envoy_pods, envoy_version, envoy_metrics_history) = envoy_data;

        // Update pod details with latest metrics from history (consolidated function)
        update_pod_details_with_metrics(&mut pod_details, &pod_metrics_history);

        // Update pods that don't have metrics history yet (set limits/requests from containers)
        set_pod_resource_limits(&mut pod_details);

        // Update node details with metrics history
        for (node_name, history) in &node_metrics_history {
            if let Some(node_detail) = node_details.get_mut(node_name) {
                // Extract just the values (not timestamps) and convert to percentages
                node_detail.cpu_history = history
                    .cpu_history
                    .iter()
                    .map(|(_, value)| *value)
                    .collect();
                node_detail.memory_history = history
                    .memory_history
                    .iter()
                    .map(|(_, value)| *value)
                    .collect();
            }
        }

        // Update Cilium pods with CPU and memory from pod_details
        update_cilium_pod_resources(&mut cilium_pods, &pod_details);

        // Update Envoy pods with CPU and memory from pod_details
        update_envoy_pod_resources(&mut envoy_pods, &pod_details);

        // Pre-serialize metrics JSON for API responses (outside of write lock)
        let metrics_json_cache =
            build_metrics_json_cache(&node_metrics_history, &pod_metrics_history);

        // Pre-serialize Cilium metrics JSON
        let cilium_metrics_json_cache =
            build_cilium_metrics_json_cache(&pod_metrics_history, &cilium_pods);

        // Pre-serialize Envoy metrics JSON
        let envoy_metrics_json_cache = build_envoy_metrics_json_cache(&envoy_metrics_history);

        // Update cache
        let mut data = self.inner.write().await;
        data.clusters = Arc::from(clusters.into_boxed_slice());
        data.servers = Arc::from(servers.into_boxed_slice());
        data.node_details = Arc::new(node_details.into_iter().collect());
        data.pod_details = Arc::new(pod_details.into_iter().collect());
        data.pod_metrics_history = Arc::new(pod_metrics_history);
        data.node_metrics_history = Arc::new(node_metrics_history);
        data.cilium_pods = Arc::from(cilium_pods.into_boxed_slice());
        data.cilium_version = Arc::from(cilium_version.as_str());
        data.hubble_enabled = hubble_enabled;
        data.ipv6_enabled = ipv6_enabled;
        data.envoy_pods = Arc::from(envoy_pods.into_boxed_slice());
        data.envoy_version = Arc::from(envoy_version.as_str());
        data.envoy_metrics_history = Arc::new(envoy_metrics_history);
        data.alerts = Arc::from(alerts.into_boxed_slice());
        data.insights = Arc::from(insights.into_boxed_slice());
        data.metrics_json_cache = metrics_json_cache;
        data.cilium_metrics_json_cache = cilium_metrics_json_cache;
        data.envoy_metrics_json_cache = envoy_metrics_json_cache;
        data.last_update = Instant::now();
        data.is_ready = true;

        info!(
            "Full cache refresh completed - {} clusters, {} nodes, {} pods, {} alerts, {} insights",
            data.clusters.len(),
            data.node_details.len(),
            data.pod_details.len(),
            data.alerts.len(),
            data.insights.len()
        );
        Ok(())
    }

    /// Refresh Kubernetes/Prometheus data only (no Hetzner API calls)
    async fn refresh_kubernetes_data(&self, config_path: &std::path::Path) -> Result<()> {
        info!("Starting Kubernetes/Prometheus data refresh...");

        // Read current state
        let data = self.inner.read().await;
        let servers = data.servers.clone();
        let cluster_name = data.clusters.first().map(|c| c.name.clone());
        drop(data); // Release read lock

        if servers.is_empty() {
            info!("Skipping Kubernetes refresh - no servers available yet");
            return Ok(());
        }

        let Some(cluster_name) = cluster_name else {
            info!("Skipping Kubernetes refresh - no cluster name available yet");
            return Ok(());
        };

        // Load config for Cilium settings
        let config_str = tokio::fs::read_to_string(config_path).await?;
        let config: ClusterConfig = serde_yaml::from_str(&config_str)?;

        // Fetch all Kubernetes/Prometheus data in parallel
        info!("Fetching pods, metrics, Cilium data, Envoy data, alerts, and insights...");
        let (
            mut pod_details,
            pod_metrics_history,
            node_metrics_history,
            cilium_data,
            envoy_data,
            alerts,
            insights,
        ) = tokio::join!(
            fetch_all_pod_details(config_path, &cluster_name),
            fetch_all_pod_metrics_history(config_path),
            fetch_all_node_metrics_history(&servers, config_path),
            fetch_cilium_data(config_path, &config, &cluster_name),
            fetch_envoy_data(config_path, &cluster_name),
            fetch_alerts(config_path),
            fetch_insights(config_path)
        );

        // Unpack Cilium data
        let (mut cilium_pods, cilium_version, hubble_enabled, ipv6_enabled) = cilium_data;

        // Unpack Envoy data
        let (mut envoy_pods, envoy_version, envoy_metrics_history) = envoy_data;

        // Update pod details with latest metrics from history (consolidated function)
        update_pod_details_with_metrics(&mut pod_details, &pod_metrics_history);

        // Get existing node details from cache and update with metrics history
        let mut node_details = self.get_node_details_map().await;
        for (node_name, history) in &node_metrics_history {
            if let Some(node_detail) = node_details.get_mut(node_name) {
                // Extract just the values (not timestamps) and convert to percentages
                node_detail.cpu_history = history
                    .cpu_history
                    .iter()
                    .map(|(_, value)| *value)
                    .collect();
                node_detail.memory_history = history
                    .memory_history
                    .iter()
                    .map(|(_, value)| *value)
                    .collect();
            }
        }

        // Update Cilium pods with CPU and memory from pod_details
        update_cilium_pod_resources(&mut cilium_pods, &pod_details);

        // Update Envoy pods with CPU and memory from pod_details
        update_envoy_pod_resources(&mut envoy_pods, &pod_details);

        // Build JSON caches before updating data
        let metrics_json_cache =
            build_metrics_json_cache(&node_metrics_history, &pod_metrics_history);
        let cilium_metrics_json_cache =
            build_cilium_metrics_json_cache(&pod_metrics_history, &cilium_pods);
        let envoy_metrics_json_cache = build_envoy_metrics_json_cache(&envoy_metrics_history);

        // Update cache with Kubernetes/Prometheus data
        let mut data = self.inner.write().await;
        data.pod_details = Arc::new(pod_details.into_iter().collect());
        data.pod_metrics_history = Arc::new(pod_metrics_history);
        data.node_metrics_history = Arc::new(node_metrics_history);
        data.cilium_pods = Arc::from(cilium_pods.into_boxed_slice());
        data.cilium_version = Arc::from(cilium_version.as_str());
        data.hubble_enabled = hubble_enabled;
        data.ipv6_enabled = ipv6_enabled;
        data.envoy_pods = Arc::from(envoy_pods.into_boxed_slice());
        data.envoy_version = Arc::from(envoy_version.as_str());
        data.envoy_metrics_history = Arc::new(envoy_metrics_history);
        data.alerts = Arc::from(alerts.into_boxed_slice());
        data.insights = Arc::from(insights.into_boxed_slice());
        data.metrics_json_cache = metrics_json_cache;
        data.cilium_metrics_json_cache = cilium_metrics_json_cache;
        data.envoy_metrics_json_cache = envoy_metrics_json_cache;
        data.last_update = Instant::now();

        info!(
            "Kubernetes/Prometheus refresh completed - {} pods, {} alerts, {} insights",
            data.pod_details.len(),
            data.alerts.len(),
            data.insights.len()
        );

        Ok(())
    }

    /// Start background refresh task
    pub fn start_background_refresh(&self, config_path: std::path::PathBuf, interval_secs: u64) {
        let cache = self.clone();
        let config_path_clone = config_path.clone();

        // Start full cluster data refresh (Hetzner API + Kubernetes)
        // This runs at interval_secs (120 seconds = 2 minutes)
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
                    error!("Failed to refresh full cache: {}", e);
                }
            }
        });

        // Start Kubernetes/Prometheus-only refresh (faster refresh for K8s data)
        // This skips Hetzner API calls and only updates pods, metrics, alerts, etc.
        let cache_kubernetes = self.clone();
        tokio::spawn(async move {
            // Wait for first cluster refresh to complete
            tokio::time::sleep(Duration::from_secs(KUBERNETES_REFRESH_INITIAL_DELAY_SECS)).await;

            let mut interval =
                tokio::time::interval(Duration::from_secs(KUBERNETES_REFRESH_INTERVAL_SECS));
            loop {
                interval.tick().await;
                if let Err(e) = cache_kubernetes
                    .refresh_kubernetes_data(&config_path_clone)
                    .await
                {
                    error!("Failed to refresh Kubernetes data: {}", e);
                }
            }
        });
    }
}

/// Build metrics JSON cache from node and pod metrics history
#[inline]
fn build_metrics_json_cache(
    node_metrics_history: &HashMap<String, crate::prometheus::NodeMetricsHistory>,
    pod_metrics_history: &HashMap<String, crate::prometheus::NodeMetricsHistory>,
) -> Arc<str> {
    use serde::Serialize;

    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct MetricsResponse {
        timestamps: Vec<i64>,
        nodes: Vec<MetricsNode>,
        pods: Vec<MetricsPod>,
    }

    #[derive(Serialize)]
    struct MetricsNode {
        name: String,
        cpu_history: Vec<f64>,
        memory_history: Vec<f64>,
    }

    #[derive(Serialize)]
    struct MetricsPod {
        name: String,
        namespace: String,
        cpu_history: Vec<f64>,
        memory_history: Vec<f64>,
    }

    if !node_metrics_history.is_empty() {
        let results: Vec<_> = node_metrics_history.iter().collect();

        // Collect all unique timestamps, rounded to nearest minute to avoid duplicates
        let mut all_timestamps = std::collections::BTreeSet::new();
        for (_, history) in &results {
            for (ts, _) in &history.cpu_history {
                let rounded = (*ts / TIMESTAMP_ROUNDING_SECS) * TIMESTAMP_ROUNDING_SECS;
                all_timestamps.insert(rounded);
            }
            for (ts, _) in &history.memory_history {
                let rounded = (*ts / TIMESTAMP_ROUNDING_SECS) * TIMESTAMP_ROUNDING_SECS;
                all_timestamps.insert(rounded);
            }
        }

        // Filter timestamps to only include those where ALL nodes have actual data
        // This prevents 0.0 values from appearing during initial collection
        let valid_timestamps: Vec<i64> =
            all_timestamps
                .into_iter()
                .filter(|ts| {
                    // Check if ALL nodes have data for this timestamp (both CPU and memory)
                    results.iter().all(|(_, history)| {
                        let has_cpu = history.cpu_history.iter().any(|(t, val)| {
                            (*t - *ts).abs() <= TIMESTAMP_TOLERANCE_SECS && *val > 0.0
                        });
                        let has_memory = history.memory_history.iter().any(|(t, val)| {
                            (*t - *ts).abs() <= TIMESTAMP_TOLERANCE_SECS && *val > 0.0
                        });
                        has_cpu && has_memory
                    })
                })
                .collect();

        // Build nodes with only valid timestamps
        let nodes: Vec<MetricsNode> = results
            .iter()
            .map(|(name, history)| {
                let cpu_history: Vec<f64> = valid_timestamps
                    .iter()
                    .filter_map(|ts| {
                        history
                            .cpu_history
                            .iter()
                            .filter(|(t, _)| (*t - *ts).abs() <= TIMESTAMP_TOLERANCE_SECS)
                            .min_by_key(|(t, _)| (*t - *ts).abs())
                            .map(|(_, val)| *val)
                    })
                    .collect();

                let memory_history: Vec<f64> = valid_timestamps
                    .iter()
                    .filter_map(|ts| {
                        history
                            .memory_history
                            .iter()
                            .filter(|(t, _)| (*t - *ts).abs() <= TIMESTAMP_TOLERANCE_SECS)
                            .min_by_key(|(t, _)| (*t - *ts).abs())
                            .map(|(_, val)| *val)
                    })
                    .collect();

                MetricsNode {
                    name: (*name).clone(),
                    cpu_history,
                    memory_history,
                }
            })
            .collect();

        // Build pods
        let pods: Vec<MetricsPod> = pod_metrics_history
            .iter()
            .map(|(key, history)| {
                let parts: Vec<&str> = key.split('/').collect();
                let namespace = parts.first().unwrap_or(&"unknown").to_string();
                let name = parts.get(1).unwrap_or(&"unknown").to_string();

                let cpu_history: Vec<f64> = history
                    .cpu_history
                    .iter()
                    .map(|(_, val)| val * CPU_TO_MILLICORES_MULTIPLIER)
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
            timestamps: valid_timestamps,
            nodes,
            pods,
        };
        Arc::from(
            serde_json::to_string(&response)
                .unwrap_or_else(|_| "{}".to_string())
                .as_str(),
        )
    } else {
        Arc::from("{}")
    }
}

/// Build Cilium metrics JSON cache
#[inline]
fn build_cilium_metrics_json_cache(
    pod_metrics_history: &HashMap<String, crate::prometheus::NodeMetricsHistory>,
    cilium_pods: &[CiliumPod],
) -> Arc<str> {
    use serde::Serialize;

    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct CiliumMetrics {
        timestamps: Vec<i64>,
        pods: Vec<CiliumPodMetrics>,
    }

    #[derive(Serialize)]
    struct CiliumPodMetrics {
        name: String,
        cpu_history: Vec<f64>,
        memory_history: Vec<f64>,
        #[serde(rename = "cpuRequest")]
        cpu_request: f64,
        #[serde(rename = "cpuLimit")]
        cpu_limit: f64,
        #[serde(rename = "memoryRequest")]
        memory_request: f64,
        #[serde(rename = "memoryLimit")]
        memory_limit: f64,
    }

    if !pod_metrics_history.is_empty() {
        // Collect all unique timestamps, rounded to nearest minute to avoid duplicates
        let mut all_timestamps = std::collections::BTreeSet::new();
        for history in pod_metrics_history.values() {
            for (ts, _) in &history.cpu_history {
                let rounded = (*ts / TIMESTAMP_ROUNDING_SECS) * TIMESTAMP_ROUNDING_SECS;
                all_timestamps.insert(rounded);
            }
            for (ts, _) in &history.memory_history {
                let rounded = (*ts / TIMESTAMP_ROUNDING_SECS) * TIMESTAMP_ROUNDING_SECS;
                all_timestamps.insert(rounded);
            }
        }

        // Filter timestamps to only include those where ALL Cilium pods have actual data
        // This prevents 0.0 values from appearing during initial collection
        let valid_timestamps: Vec<i64> = all_timestamps
            .into_iter()
            .filter(|ts| {
                // Check if all Cilium pods have data for this timestamp (both CPU and memory)
                cilium_pods.iter().all(|cilium_pod| {
                    let key = format!("kube-system/{}", cilium_pod.name);
                    if let Some(history) = pod_metrics_history.get(&key) {
                        let has_cpu = history.cpu_history.iter().any(|(t, val)| {
                            (*t - *ts).abs() <= TIMESTAMP_TOLERANCE_SECS && *val > 0.0
                        });
                        let has_memory = history.memory_history.iter().any(|(t, val)| {
                            (*t - *ts).abs() <= TIMESTAMP_TOLERANCE_SECS && *val > 0.0
                        });
                        has_cpu && has_memory
                    } else {
                        false
                    }
                })
            })
            .collect();

        // Build pods metrics
        let pods_metrics: Vec<CiliumPodMetrics> = cilium_pods
            .iter()
            .map(|cilium_pod| {
                let key = format!("kube-system/{}", cilium_pod.name);
                let history = pod_metrics_history.get(&key);

                let (cpu_history, memory_history) = if let Some(hist) = history {
                    // Align CPU data to valid timestamps (find nearest within tolerance)
                    let cpu_aligned: Vec<f64> = valid_timestamps
                        .iter()
                        .filter_map(|ts| {
                            hist.cpu_history
                                .iter()
                                .filter(|(t, _)| (*t - *ts).abs() <= TIMESTAMP_TOLERANCE_SECS)
                                .min_by_key(|(t, _)| (*t - *ts).abs())
                                .map(|(_, val)| val * CPU_TO_MILLICORES_MULTIPLIER)
                        })
                        .collect();

                    // Align memory data to valid timestamps (find nearest within tolerance)
                    let memory_aligned: Vec<f64> = valid_timestamps
                        .iter()
                        .filter_map(|ts| {
                            hist.memory_history
                                .iter()
                                .filter(|(t, _)| (*t - *ts).abs() <= TIMESTAMP_TOLERANCE_SECS)
                                .min_by_key(|(t, _)| (*t - *ts).abs())
                                .map(|(_, val)| *val)
                        })
                        .collect();

                    (cpu_aligned, memory_aligned)
                } else {
                    (vec![], vec![])
                };

                CiliumPodMetrics {
                    name: cilium_pod.name.clone(),
                    cpu_history,
                    memory_history,
                    cpu_request: cilium_pod.cpu_request,
                    cpu_limit: cilium_pod.cpu_limit,
                    memory_request: cilium_pod.memory_request,
                    memory_limit: cilium_pod.memory_limit,
                }
            })
            .collect();

        let response = CiliumMetrics {
            timestamps: valid_timestamps,
            pods: pods_metrics,
        };
        Arc::from(
            serde_json::to_string(&response)
                .unwrap_or_else(|_| "{}".to_string())
                .as_str(),
        )
    } else {
        Arc::from("{}")
    }
}

/// Build Envoy metrics JSON cache for API responses
#[inline]
fn build_envoy_metrics_json_cache(
    envoy_metrics_history: &crate::prometheus::EnvoyMetricsHistory,
) -> Arc<str> {
    use serde::Serialize;

    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct EnvoyMetrics {
        timestamps: Vec<i64>,
        rps_history: Vec<f64>,
        status_2xx_history: Vec<f64>,
        status_3xx_history: Vec<f64>,
        status_4xx_history: Vec<f64>,
        status_5xx_history: Vec<f64>,
    }

    if !envoy_metrics_history.rps_history.is_empty() {
        // Collect all unique timestamps, rounded to nearest minute
        let mut all_timestamps = std::collections::BTreeSet::new();
        for (ts, _) in &envoy_metrics_history.rps_history {
            let rounded = (*ts / TIMESTAMP_ROUNDING_SECS) * TIMESTAMP_ROUNDING_SECS;
            all_timestamps.insert(rounded);
        }
        for (ts, _) in &envoy_metrics_history.status_2xx_history {
            let rounded = (*ts / TIMESTAMP_ROUNDING_SECS) * TIMESTAMP_ROUNDING_SECS;
            all_timestamps.insert(rounded);
        }
        for (ts, _) in &envoy_metrics_history.status_3xx_history {
            let rounded = (*ts / TIMESTAMP_ROUNDING_SECS) * TIMESTAMP_ROUNDING_SECS;
            all_timestamps.insert(rounded);
        }
        for (ts, _) in &envoy_metrics_history.status_4xx_history {
            let rounded = (*ts / TIMESTAMP_ROUNDING_SECS) * TIMESTAMP_ROUNDING_SECS;
            all_timestamps.insert(rounded);
        }
        for (ts, _) in &envoy_metrics_history.status_5xx_history {
            let rounded = (*ts / TIMESTAMP_ROUNDING_SECS) * TIMESTAMP_ROUNDING_SECS;
            all_timestamps.insert(rounded);
        }
        let timestamps: Vec<i64> = all_timestamps.into_iter().collect();

        // Align RPS data to rounded timestamps
        let rps_aligned: Vec<f64> = timestamps
            .iter()
            .map(|ts| {
                envoy_metrics_history
                    .rps_history
                    .iter()
                    .filter(|(t, _)| (*t - *ts).abs() <= TIMESTAMP_TOLERANCE_SECS)
                    .min_by_key(|(t, _)| (*t - *ts).abs())
                    .map(|(_, val)| *val)
                    .unwrap_or(0.0)
            })
            .collect();

        // Align 2xx data
        let status_2xx_aligned: Vec<f64> = timestamps
            .iter()
            .map(|ts| {
                envoy_metrics_history
                    .status_2xx_history
                    .iter()
                    .filter(|(t, _)| (*t - *ts).abs() <= TIMESTAMP_TOLERANCE_SECS)
                    .min_by_key(|(t, _)| (*t - *ts).abs())
                    .map(|(_, val)| *val)
                    .unwrap_or(0.0)
            })
            .collect();

        // Align 3xx data
        let status_3xx_aligned: Vec<f64> = timestamps
            .iter()
            .map(|ts| {
                envoy_metrics_history
                    .status_3xx_history
                    .iter()
                    .filter(|(t, _)| (*t - *ts).abs() <= TIMESTAMP_TOLERANCE_SECS)
                    .min_by_key(|(t, _)| (*t - *ts).abs())
                    .map(|(_, val)| *val)
                    .unwrap_or(0.0)
            })
            .collect();

        // Align 4xx data
        let status_4xx_aligned: Vec<f64> = timestamps
            .iter()
            .map(|ts| {
                envoy_metrics_history
                    .status_4xx_history
                    .iter()
                    .filter(|(t, _)| (*t - *ts).abs() <= TIMESTAMP_TOLERANCE_SECS)
                    .min_by_key(|(t, _)| (*t - *ts).abs())
                    .map(|(_, val)| *val)
                    .unwrap_or(0.0)
            })
            .collect();

        // Align 5xx data
        let status_5xx_aligned: Vec<f64> = timestamps
            .iter()
            .map(|ts| {
                envoy_metrics_history
                    .status_5xx_history
                    .iter()
                    .filter(|(t, _)| (*t - *ts).abs() <= TIMESTAMP_TOLERANCE_SECS)
                    .min_by_key(|(t, _)| (*t - *ts).abs())
                    .map(|(_, val)| *val)
                    .unwrap_or(0.0)
            })
            .collect();

        let response = EnvoyMetrics {
            timestamps,
            rps_history: rps_aligned,
            status_2xx_history: status_2xx_aligned,
            status_3xx_history: status_3xx_aligned,
            status_4xx_history: status_4xx_aligned,
            status_5xx_history: status_5xx_aligned,
        };
        Arc::from(
            serde_json::to_string(&response)
                .unwrap_or_else(|_| "{}".to_string())
                .as_str(),
        )
    } else {
        Arc::from("{}")
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
    use std::sync::Arc;

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
        return HashMap::new();
    }

    // Wrap kubeconfig in Arc to avoid cloning PathBuf for each task
    let kubeconfig = std::sync::Arc::new(kubeconfig);

    // Fetch pods for all nodes in parallel
    let fetch_tasks: Vec<_> = servers
        .iter()
        .map(|server| {
            let kubeconfig = Arc::clone(&kubeconfig);
            // Extract only the fields we need instead of cloning entire server
            let server_name = server.name.clone();
            let server_status = server.status.clone();
            let server_type_name = server.server_type.name.clone();
            let server_cores = server.server_type.cores;
            let server_created = server.created.clone();
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

            async move {
                let cluster_name = server_name
                    .split('-')
                    .next()
                    .unwrap_or("unknown")
                    .to_string();

                let role = if server_name.contains("control-plane") {
                    "Control Plane".to_string()
                } else {
                    "Worker".to_string()
                };

                // Get pods from Kubernetes
                let mut pods = KubernetesClient::get_pods_on_node(&kubeconfig, &server_name)
                    .await
                    .unwrap_or_else(|e| {
                        error!("Failed to get pods for node {}: {}", server_name, e);
                        Vec::new()
                    });

                // Sort pods by CPU usage (highest to lowest)
                pods.sort_by(|a, b| {
                    let cpu_a = a.cpu.trim_end_matches('m').parse::<f64>().unwrap_or(-1.0);
                    let cpu_b = b.cpu.trim_end_matches('m').parse::<f64>().unwrap_or(-1.0);
                    cpu_b
                        .partial_cmp(&cpu_a)
                        .unwrap_or(std::cmp::Ordering::Equal)
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

                let cpu_used_cores = if metrics.cpu_usage_percent > 0.0 {
                    let used = (metrics.cpu_usage_percent / 100.0) * server_cores as f64;
                    format!("{:.2}", used)
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
                    server_name.clone(),
                    NodeDetail {
                        cluster_name,
                        name: server_name,
                        role,
                        ip,
                        private_ip,
                        status: server_status,
                        server_type: server_type_name,
                        created: server_created
                            .split('T')
                            .next()
                            .unwrap_or("Unknown")
                            .to_string(),
                        pods,
                        cpu_usage_percent,
                        cpu_cores: server_cores,
                        cpu_used_cores,
                        memory_usage_percent,
                        memory_used_gb,
                        memory_total_gb,
                        cpu_history: Vec::new(),
                        memory_history: Vec::new(),
                    },
                )
            }
        })
        .collect();

    // Execute all fetch tasks in parallel and collect directly into HashMap
    futures::future::join_all(fetch_tasks)
        .await
        .into_iter()
        .collect()
}

/// Trim old metrics from history to prevent unbounded memory growth
fn trim_metrics_history(history: &mut crate::prometheus::NodeMetricsHistory) {
    use chrono::Utc;

    let now = Utc::now().timestamp();
    let cutoff = now - METRICS_HISTORY_MAX_AGE_SECS;

    // Remove entries older than max age
    history.cpu_history.retain(|(ts, _)| *ts >= cutoff);
    history.memory_history.retain(|(ts, _)| *ts >= cutoff);
}

/// Fetch historical metrics for all nodes
async fn fetch_all_node_metrics_history(
    servers: &[Server],
    config_path: &std::path::Path,
) -> std::collections::HashMap<String, crate::prometheus::NodeMetricsHistory> {
    use std::sync::Arc;

    let output_dir = config_path
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.join("output"))
        .unwrap_or_else(|| std::path::PathBuf::from("output"));

    let kubeconfig = output_dir.join("kubeconfig");

    // Check if kubeconfig exists
    if !kubeconfig.exists() {
        info!("Kubeconfig not found, skipping metrics history fetch");
        return std::collections::HashMap::new();
    }

    // Wrap kubeconfig in Arc to avoid cloning PathBuf for each task
    let kubeconfig = Arc::new(kubeconfig);

    // Fetch metrics history for all nodes in parallel
    let fetch_tasks: Vec<_> = servers
        .iter()
        .map(|server| {
            let kubeconfig = Arc::clone(&kubeconfig);
            // Extract only needed fields instead of cloning entire server
            let server_name = server.name.clone();
            let private_ip = server
                .private_net
                .first()
                .map(|net| net.ip.clone())
                .unwrap_or_else(|| "N/A".to_string());

            async move {
                let mut history = crate::prometheus::query_node_metrics_range(
                    &private_ip,
                    &kubeconfig,
                    "1h",
                    "1m",
                )
                .await
                .unwrap_or_default();

                // Trim old data to prevent memory leaks
                trim_metrics_history(&mut history);

                (server_name, history)
            }
        })
        .collect();

    // Execute all fetch tasks in parallel and collect directly into HashMap
    futures::future::join_all(fetch_tasks)
        .await
        .into_iter()
        .collect()
}

/// Build detailed cluster info
fn build_cluster_detail(
    cluster_name: &str,
    cluster_servers: &[&Server],
    node_details: &DashMap<String, NodeDetail>,
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

    let mut nodes: Vec<NodeInfo> = cluster_servers
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

    // Sort nodes by CPU usage (highest to lowest)
    nodes.sort_by(|a, b| {
        let cpu_a = a
            .cpu_usage_percent
            .trim_end_matches('%')
            .parse::<f64>()
            .unwrap_or(-1.0);
        let cpu_b = b
            .cpu_usage_percent
            .trim_end_matches('%')
            .parse::<f64>()
            .unwrap_or(-1.0);
        cpu_b
            .partial_cmp(&cpu_a)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

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

/// Fetch Cilium pod information
async fn fetch_cilium_data(
    config_path: &std::path::Path,
    config: &ClusterConfig,
    cluster_name: &str,
) -> (Vec<super::templates::CiliumPod>, String, bool, bool) {
    use tokio::process::Command;

    let output_dir = config_path
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.join("output"))
        .unwrap_or_else(|| std::path::PathBuf::from("output"));

    let kubeconfig = output_dir.join("kubeconfig");

    // Get configuration from cluster config
    let cilium_version = config.cilium.version.clone();
    let hubble_enabled = config.cilium.enable_hubble;
    let ipv6_enabled = config.cilium.enable_ipv6;

    // Check if kubeconfig exists
    if !kubeconfig.exists() {
        info!("Kubeconfig not found, skipping Cilium data fetch");
        return (vec![], cilium_version, hubble_enabled, ipv6_enabled);
    }

    // Get Cilium pods using kubectl
    let output = match Command::new("kubectl")
        .arg("--kubeconfig")
        .arg(&kubeconfig)
        .arg("get")
        .arg("pods")
        .arg("-n")
        .arg("kube-system")
        .arg("-l")
        .arg("k8s-app=cilium")
        .arg("-o")
        .arg("json")
        .output()
        .await
    {
        Ok(output) => output,
        Err(e) => {
            info!("Failed to query Cilium pods: {}", e);
            return (vec![], cilium_version, hubble_enabled, ipv6_enabled);
        }
    };

    if !output.status.success() {
        info!(
            "kubectl get pods failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        return (vec![], cilium_version, hubble_enabled, ipv6_enabled);
    }

    let pods_json: serde_json::Value = match serde_json::from_slice(&output.stdout) {
        Ok(json) => json,
        Err(e) => {
            info!("Failed to parse Cilium pods JSON: {}", e);
            return (vec![], cilium_version, hubble_enabled, ipv6_enabled);
        }
    };

    let mut cilium_pods = Vec::new();

    if let Some(items) = pods_json["items"].as_array() {
        for pod in items {
            let name = pod["metadata"]["name"]
                .as_str()
                .unwrap_or("unknown")
                .to_string();
            let node = pod["spec"]["nodeName"]
                .as_str()
                .unwrap_or("N/A")
                .to_string();
            let status = pod["status"]["phase"]
                .as_str()
                .unwrap_or("Unknown")
                .to_string();

            // Count restarts
            let mut restarts = 0u32;
            if let Some(container_statuses) = pod["status"]["containerStatuses"].as_array() {
                for container in container_statuses {
                    if let Some(restart_count) = container["restartCount"].as_u64() {
                        restarts += restart_count as u32;
                    }
                }
            }

            // Calculate age from creationTimestamp
            let age = if let Some(created_str) = pod["metadata"]["creationTimestamp"].as_str() {
                calculate_age(created_str)
            } else {
                "N/A".to_string()
            };

            cilium_pods.push(super::templates::CiliumPod {
                name,
                node,
                cluster_name: cluster_name.to_string(),
                status,
                cpu: "0m".to_string(),
                memory: "0Mi".to_string(),
                cpu_request: 0.0,
                cpu_limit: 0.0,
                memory_request: 0.0,
                memory_limit: 0.0,
                restarts,
                age,
            });
        }
    }

    (cilium_pods, cilium_version, hubble_enabled, ipv6_enabled)
}

/// Fetch Envoy pod information and metrics
async fn fetch_envoy_data(
    config_path: &std::path::Path,
    cluster_name: &str,
) -> (
    Vec<super::templates::EnvoyPod>,
    String,
    crate::prometheus::EnvoyMetricsHistory,
) {
    use tokio::process::Command;

    let output_dir = config_path
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.join("output"))
        .unwrap_or_else(|| std::path::PathBuf::from("output"));

    let kubeconfig = output_dir.join("kubeconfig");

    // Check if kubeconfig exists
    if !kubeconfig.exists() {
        info!("Kubeconfig not found, skipping Envoy data fetch");
        return (
            vec![],
            "N/A".to_string(),
            crate::prometheus::EnvoyMetricsHistory::default(),
        );
    }

    // Get Envoy pods using kubectl (looking for pods with envoy in the name across all namespaces)
    let output = match Command::new("kubectl")
        .arg("--kubeconfig")
        .arg(&kubeconfig)
        .arg("get")
        .arg("pods")
        .arg("--all-namespaces")
        .arg("-o")
        .arg("json")
        .output()
        .await
    {
        Ok(o) => o,
        Err(e) => {
            error!("Failed to get Envoy pods: {}", e);
            return (
                vec![],
                "N/A".to_string(),
                crate::prometheus::EnvoyMetricsHistory::default(),
            );
        }
    };

    if !output.status.success() {
        error!("kubectl get pods failed for Envoy");
        return (
            vec![],
            "N/A".to_string(),
            crate::prometheus::EnvoyMetricsHistory::default(),
        );
    }

    let mut envoy_pods = Vec::new();
    let mut envoy_version = "N/A".to_string();

    // Parse pod list
    if let Ok(pod_list) = serde_json::from_slice::<serde_json::Value>(&output.stdout) {
        if let Some(items) = pod_list["items"].as_array() {
            for pod in items {
                let name = pod["metadata"]["name"]
                    .as_str()
                    .unwrap_or("unknown")
                    .to_string();

                // Only include pods with "envoy" in the name
                if !name.contains("envoy") {
                    continue;
                }

                let namespace = pod["metadata"]["namespace"]
                    .as_str()
                    .unwrap_or("default")
                    .to_string();
                let node = pod["spec"]["nodeName"]
                    .as_str()
                    .unwrap_or("N/A")
                    .to_string();
                let status = pod["status"]["phase"]
                    .as_str()
                    .unwrap_or("Unknown")
                    .to_string();
                let restarts = pod["status"]["containerStatuses"]
                    .as_array()
                    .and_then(|containers| containers.first())
                    .and_then(|container| container["restartCount"].as_u64())
                    .unwrap_or(0) as u32;

                let created_at = pod["metadata"]["creationTimestamp"]
                    .as_str()
                    .unwrap_or_default();
                let age = if !created_at.is_empty() {
                    calculate_age(created_at)
                } else {
                    "N/A".to_string()
                };

                envoy_pods.push(super::templates::EnvoyPod {
                    name,
                    namespace,
                    node,
                    cluster_name: cluster_name.to_string(),
                    status,
                    cpu: "0m".to_string(),
                    memory: "0Mi".to_string(),
                    cpu_request: 0.0,
                    cpu_limit: 0.0,
                    memory_request: 0.0,
                    memory_limit: 0.0,
                    restarts,
                    age,
                });

                // Extract version from first pod's container image
                if envoy_version == "N/A" {
                    if let Some(containers) = pod["spec"]["containers"].as_array() {
                        if let Some(first_container) = containers.first() {
                            if let Some(image) = first_container["image"].as_str() {
                                // Extract version from image (e.g., "envoyproxy/envoy:v1.28.0@sha256:..." -> "v1.28.0")
                                // First remove the @sha256 part if present
                                let image_without_sha = image.split('@').next().unwrap_or(image);
                                // Then extract the tag after the last colon
                                if let Some(version_part) = image_without_sha.split(':').next_back()
                                {
                                    // Extract just the semantic version part (v1.33.9 from v1.33.9-1757932127-3c04e8f2f1027d106b96f8ef4a0215e81dbaaece)
                                    let clean_version =
                                        version_part.split('-').next().unwrap_or(version_part);
                                    envoy_version = clean_version.to_string();
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Fetch Envoy metrics history from Prometheus
    let metrics_history = crate::prometheus::query_envoy_metrics_range(&kubeconfig, "1h", "1m")
        .await
        .unwrap_or_else(|e| {
            error!("Failed to fetch Envoy metrics: {}", e);
            crate::prometheus::EnvoyMetricsHistory::default()
        });

    (envoy_pods, envoy_version, metrics_history)
}

/// Fetch Prometheus alerts
async fn fetch_alerts(config_path: &std::path::Path) -> Vec<crate::prometheus::Alert> {
    // Get the output directory from config path
    let output_dir = config_path
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.join("output"))
        .unwrap_or_else(|| std::path::PathBuf::from("output"));

    let kubeconfig = output_dir.join("kubeconfig");

    crate::prometheus::query_alerts(&kubeconfig)
        .await
        .unwrap_or_else(|e| {
            error!("Failed to fetch alerts: {}", e);
            Vec::new()
        })
}

/// Fetch cluster insights
async fn fetch_insights(config_path: &std::path::Path) -> Vec<crate::prometheus::Insight> {
    // Get the output directory from config path
    let output_dir = config_path
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.join("output"))
        .unwrap_or_else(|| std::path::PathBuf::from("output"));

    let kubeconfig = output_dir.join("kubeconfig");

    crate::prometheus::collect_insights(&kubeconfig)
        .await
        .unwrap_or_else(|e| {
            error!("Failed to fetch insights: {}", e);
            Vec::new()
        })
}

/// Fetch detailed information for all pods
async fn fetch_all_pod_details(
    config_path: &std::path::Path,
    cluster_name: &str,
) -> std::collections::HashMap<String, super::templates::PodDetail> {
    use tokio::process::Command;

    let mut pod_details = std::collections::HashMap::new();

    let output_dir = config_path
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.join("output"))
        .unwrap_or_else(|| std::path::PathBuf::from("output"));

    let kubeconfig = output_dir.join("kubeconfig");

    // Check if kubeconfig exists
    if !kubeconfig.exists() {
        info!("Kubeconfig not found, skipping pod details fetch");
        return pod_details;
    }

    // Get all pods from all namespaces
    let output = match Command::new("kubectl")
        .arg("--kubeconfig")
        .arg(&kubeconfig)
        .arg("get")
        .arg("pods")
        .arg("--all-namespaces")
        .arg("-o")
        .arg("json")
        .output()
        .await
    {
        Ok(output) => output,
        Err(e) => {
            info!("Failed to query pods: {}", e);
            return pod_details;
        }
    };

    if !output.status.success() {
        info!(
            "kubectl get pods failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        return pod_details;
    }

    let pods_json: serde_json::Value = match serde_json::from_slice(&output.stdout) {
        Ok(json) => json,
        Err(e) => {
            info!("Failed to parse pods JSON: {}", e);
            return pod_details;
        }
    };

    if let Some(items) = pods_json["items"].as_array() {
        for pod in items {
            let namespace = pod["metadata"]["namespace"]
                .as_str()
                .unwrap_or("unknown")
                .to_string();
            let pod_name = pod["metadata"]["name"]
                .as_str()
                .unwrap_or("unknown")
                .to_string();
            let node_name = pod["spec"]["nodeName"]
                .as_str()
                .unwrap_or("N/A")
                .to_string();
            let status = pod["status"]["phase"]
                .as_str()
                .unwrap_or("Unknown")
                .to_string();
            let pod_ip = pod["status"]["podIP"].as_str().unwrap_or("N/A").to_string();

            // Calculate age
            let age = if let Some(created_str) = pod["metadata"]["creationTimestamp"].as_str() {
                calculate_age(created_str)
            } else {
                "N/A".to_string()
            };

            // Count restarts
            let mut restarts = 0u32;
            if let Some(container_statuses) = pod["status"]["containerStatuses"].as_array() {
                for container in container_statuses {
                    if let Some(restart_count) = container["restartCount"].as_u64() {
                        restarts += restart_count as u32;
                    }
                }
            }

            // Extract labels
            let mut labels = Vec::new();
            if let Some(labels_obj) = pod["metadata"]["labels"].as_object() {
                for (k, v) in labels_obj {
                    if let Some(v_str) = v.as_str() {
                        labels.push((k.clone(), v_str.to_string()));
                    }
                }
            }

            // Extract container information
            let mut containers = Vec::new();
            if let Some(container_specs) = pod["spec"]["containers"].as_array() {
                for (idx, container_spec) in container_specs.iter().enumerate() {
                    let container_name = container_spec["name"]
                        .as_str()
                        .unwrap_or("unknown")
                        .to_string();
                    let image = container_spec["image"]
                        .as_str()
                        .unwrap_or("N/A")
                        .to_string();

                    // Get resource requests and limits (handle both missing and present)
                    let cpu_request = container_spec
                        .get("resources")
                        .and_then(|r| r.get("requests"))
                        .and_then(|req| req.get("cpu"))
                        .and_then(|c| c.as_str())
                        .unwrap_or("-")
                        .to_string();
                    let cpu_limit = container_spec
                        .get("resources")
                        .and_then(|r| r.get("limits"))
                        .and_then(|lim| lim.get("cpu"))
                        .and_then(|c| c.as_str())
                        .unwrap_or("-")
                        .to_string();
                    let memory_request = container_spec
                        .get("resources")
                        .and_then(|r| r.get("requests"))
                        .and_then(|req| req.get("memory"))
                        .and_then(|m| m.as_str())
                        .unwrap_or("-")
                        .to_string();
                    let memory_limit = container_spec
                        .get("resources")
                        .and_then(|r| r.get("limits"))
                        .and_then(|lim| lim.get("memory"))
                        .and_then(|m| m.as_str())
                        .unwrap_or("-")
                        .to_string();

                    // Get container status
                    let (ready, restart_count) = if let Some(container_statuses) =
                        pod["status"]["containerStatuses"].as_array()
                    {
                        if let Some(container_status) = container_statuses.get(idx) {
                            let ready = container_status["ready"].as_bool().unwrap_or(false);
                            let restart_count =
                                container_status["restartCount"].as_u64().unwrap_or(0) as u32;
                            (ready, restart_count)
                        } else {
                            (false, 0)
                        }
                    } else {
                        (false, 0)
                    };

                    containers.push(super::templates::ContainerInfo {
                        name: container_name,
                        image,
                        cpu_request,
                        cpu_limit,
                        memory_request,
                        memory_limit,
                        ready,
                        restart_count,
                    });
                }
            }

            let pod_detail = super::templates::PodDetail {
                cluster_name: cluster_name.to_string(),
                node_name,
                name: pod_name.clone(),
                namespace: namespace.clone(),
                status,
                restarts,
                age,
                ip: pod_ip,
                cpu: "N/A".to_string(),
                memory: "N/A".to_string(),
                cpu_limit: "N/A".to_string(),
                cpu_request: "N/A".to_string(),
                memory_limit: "N/A".to_string(),
                memory_request: "N/A".to_string(),
                cpu_percent: "0%".to_string(),
                memory_percent: "0%".to_string(),
                labels,
                containers,
            };

            let key = format!("{}/{}", namespace, pod_name);
            pod_details.insert(key, pod_detail);
        }
    }

    pod_details
}

/// Fetch metrics history for all pods
async fn fetch_all_pod_metrics_history(
    config_path: &std::path::Path,
) -> std::collections::HashMap<String, crate::prometheus::NodeMetricsHistory> {
    use std::sync::Arc;

    let output_dir = config_path
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.join("output"))
        .unwrap_or_else(|| std::path::PathBuf::from("output"));

    let kubeconfig = output_dir.join("kubeconfig");

    // Check if kubeconfig exists
    if !kubeconfig.exists() {
        info!("Kubeconfig not found, skipping pod metrics history fetch");
        return std::collections::HashMap::new();
    }

    // Get all pods to fetch metrics for
    let output = match tokio::process::Command::new("kubectl")
        .arg("--kubeconfig")
        .arg(&kubeconfig)
        .arg("get")
        .arg("pods")
        .arg("--all-namespaces")
        .arg("-o")
        .arg("json")
        .output()
        .await
    {
        Ok(output) => output,
        Err(e) => {
            info!("Failed to query pods for metrics: {}", e);
            return std::collections::HashMap::new();
        }
    };

    if !output.status.success() {
        return std::collections::HashMap::new();
    }

    let pods_json: serde_json::Value = match serde_json::from_slice(&output.stdout) {
        Ok(json) => json,
        Err(_) => return std::collections::HashMap::new(),
    };

    let items = match pods_json["items"].as_array() {
        Some(items) => items,
        None => return std::collections::HashMap::new(),
    };

    // Wrap kubeconfig in Arc to avoid cloning PathBuf for each task
    let kubeconfig = Arc::new(kubeconfig);

    // Fetch metrics for all pods in parallel
    let fetch_tasks: Vec<_> = items
        .iter()
        .map(|pod| {
            let namespace = pod["metadata"]["namespace"]
                .as_str()
                .unwrap_or("unknown")
                .to_string();
            let pod_name = pod["metadata"]["name"]
                .as_str()
                .unwrap_or("unknown")
                .to_string();
            let kubeconfig = Arc::clone(&kubeconfig);

            async move {
                let mut history = crate::prometheus::query_pod_metrics_range(
                    &namespace,
                    &pod_name,
                    &kubeconfig,
                    "1h",
                    "1m",
                )
                .await
                .unwrap_or_default();

                // Trim old data to prevent memory leaks
                trim_metrics_history(&mut history);

                let key = format!("{}/{}", namespace, pod_name);
                (key, history)
            }
        })
        .collect();

    // Execute all fetch tasks in parallel and collect directly into HashMap
    futures::future::join_all(fetch_tasks)
        .await
        .into_iter()
        .collect()
}
