/// Askama HTML templates
use askama::Template;
use serde::{Deserialize, Serialize};

/// Index/home page
#[derive(Template)]
#[template(path = "index.html")]
pub struct IndexTemplate {
    pub cluster_count: usize,
    pub total_nodes: usize,
    pub active_page: String,
    pub version: String,
    pub firing_alerts_count: usize,
}

/// Clusters list page
#[derive(Template)]
#[template(path = "clusters.html")]
pub struct ClustersTemplate {
    pub clusters: Vec<ClusterInfo>,
    pub active_page: String,
    pub version: String,
    pub firing_alerts_count: usize,
}

/// Create cluster form page
#[derive(Template)]
#[template(path = "create_cluster.html")]
pub struct CreateClusterTemplate {
    pub active_page: String,
    pub version: String,
    pub firing_alerts_count: usize,
}

/// Cluster detail page
#[derive(Template)]
#[template(path = "cluster_detail.html")]
pub struct ClusterDetailTemplate {
    pub cluster: ClusterDetail,
    pub active_page: String,
    pub version: String,
    pub firing_alerts_count: usize,
}

/// Cluster information for display
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterInfo {
    pub name: String,
    pub status: String,
    pub nodes: usize,
    pub version: String,
    pub created: String,
}

/// Detailed cluster information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterDetail {
    pub name: String,
    pub status: String,
    pub version: String,
    pub created: String,
    pub nodes: Vec<NodeInfo>,
    pub endpoint: String,
    pub control_plane_count: usize,
    pub worker_count: usize,
}

/// Node information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeInfo {
    pub name: String,
    pub role: String,
    pub ip: String,
    pub private_ip: String,
    pub status: String,
    pub server_type: String,
    pub created: String,
    pub cpu_usage_percent: String,
    pub memory_usage_percent: String,
}

/// Node detail page
#[derive(Template)]
#[template(path = "node_detail.html")]
pub struct NodeDetailTemplate {
    pub node: NodeDetail,
    pub active_page: String,
    pub version: String,
    pub firing_alerts_count: usize,
}

/// Detailed node information with pods
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeDetail {
    pub cluster_name: String,
    pub name: String,
    pub role: String,
    pub ip: String,
    pub private_ip: String,
    pub status: String,
    pub server_type: String,
    pub created: String,
    pub pods: Vec<crate::k8s::client::PodInfo>,
    pub cpu_usage_percent: String,
    pub cpu_cores: u32,
    pub cpu_used_cores: String,
    pub memory_usage_percent: String,
    pub memory_used_gb: String,
    pub memory_total_gb: String,
}

/// Metrics page
#[derive(Template)]
#[template(path = "metrics.html")]
pub struct MetricsTemplate {
    pub active_page: String,
    pub version: String,
    pub has_data: bool,
    pub metrics_json: String,
    pub node_names: Vec<String>,
    pub pod_names: Vec<String>,
    pub firing_alerts_count: usize,
}

/// Cilium page
#[derive(Template)]
#[template(path = "cilium.html")]
pub struct CiliumTemplate {
    pub active_page: String,
    pub version: String,
    pub cilium_pods: Vec<CiliumPod>,
    pub cilium_version: String,
    pub hubble_enabled: bool,
    pub ipv6_enabled: bool,
    pub metrics_json: String,
    pub pod_names: Vec<String>,
    pub firing_alerts_count: usize,
}

/// Alerts page
#[derive(Template)]
#[template(path = "alerts.html")]
pub struct AlertsTemplate {
    pub active_page: String,
    pub version: String,
    pub alerts: Vec<crate::prometheus::Alert>,
    pub firing_count: usize,
    pub pending_count: usize,
    pub firing_alerts_count: usize,
}

/// Cilium pod information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CiliumPod {
    pub name: String,
    pub node: String,
    pub cluster_name: String,
    pub status: String,
    pub cpu: String,
    pub memory: String,
    pub cpu_request: f64,
    pub cpu_limit: f64,
    pub memory_request: f64,
    pub memory_limit: f64,
    pub restarts: u32,
    pub age: String,
}

/// Pods list page
#[derive(Template)]
#[template(path = "pods.html")]
pub struct PodsTemplate {
    pub pods: Vec<PodDetail>,
    pub running_count: usize,
    pub pending_count: usize,
    pub failed_count: usize,
    pub active_page: String,
    pub version: String,
    pub firing_alerts_count: usize,
}

/// Pod detail page
#[derive(Template)]
#[template(path = "pod_detail.html")]
pub struct PodDetailTemplate {
    pub pod: PodDetail,
    pub metrics_json: String,
    pub active_page: String,
    pub version: String,
    pub firing_alerts_count: usize,
}

/// Detailed pod information with metrics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PodDetail {
    pub cluster_name: String,
    pub node_name: String,
    pub name: String,
    pub namespace: String,
    pub status: String,
    pub restarts: u32,
    pub age: String,
    pub ip: String,
    pub cpu: String,
    pub memory: String,
    pub labels: Vec<(String, String)>,
    pub containers: Vec<ContainerInfo>,
}

/// Container information within a pod
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerInfo {
    pub name: String,
    pub image: String,
    pub cpu_request: String,
    pub cpu_limit: String,
    pub memory_request: String,
    pub memory_limit: String,
    pub ready: bool,
    pub restart_count: u32,
}

/// Nodes list page
#[derive(Template)]
#[template(path = "nodes.html")]
pub struct NodesTemplate {
    pub nodes: Vec<NodeInfoWithCluster>,
    pub control_plane_count: usize,
    pub worker_count: usize,
    pub running_count: usize,
    pub metrics_json: String,
    pub node_names: Vec<String>,
    pub active_page: String,
    pub version: String,
    pub firing_alerts_count: usize,
}

/// Node information with cluster name
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeInfoWithCluster {
    pub cluster_name: String,
    pub name: String,
    pub role: String,
    pub ip: String,
    pub private_ip: String,
    pub status: String,
    pub server_type: String,
    pub created: String,
    pub cpu_usage_percent: String,
    pub memory_usage_percent: String,
}
