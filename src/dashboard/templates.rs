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
    pub insights_count: usize,
    pub warning_events_count: usize,
}

/// Clusters list page
#[derive(Template)]
#[template(path = "clusters.html")]
pub struct ClustersTemplate<'a> {
    pub clusters: &'a [ClusterInfo],
    pub active_page: String,
    pub version: String,
    pub firing_alerts_count: usize,
    pub insights_count: usize,
    pub warning_events_count: usize,
}

/// Create cluster form page
#[derive(Template)]
#[template(path = "create_cluster.html")]
pub struct CreateClusterTemplate {
    pub active_page: String,
    pub version: String,
    pub firing_alerts_count: usize,
    pub insights_count: usize,
    pub warning_events_count: usize,
}

/// Cluster detail page
#[derive(Template)]
#[template(path = "cluster_detail.html")]
pub struct ClusterDetailTemplate {
    pub cluster: ClusterDetail,
    pub active_page: String,
    pub version: String,
    pub firing_alerts_count: usize,
    pub insights_count: usize,
    pub warning_events_count: usize,
    pub metrics_json: String,
    pub node_names: Vec<String>,
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
    pub metrics_json: String,
    pub active_page: String,
    pub version: String,
    pub firing_alerts_count: usize,
    pub insights_count: usize,
    pub warning_events_count: usize,
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
    pub cpu_history: Vec<f64>,
    pub memory_history: Vec<f64>,
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
    pub insights_count: usize,
    pub warning_events_count: usize,
}

/// Cilium page
#[derive(Template)]
#[template(path = "cilium.html")]
pub struct CiliumTemplate<'a> {
    pub active_page: String,
    pub version: String,
    pub cilium_pods: &'a [CiliumPod],
    pub cilium_version: String,
    pub hubble_enabled: bool,
    pub ipv6_enabled: bool,
    pub metrics_json: String,
    pub pod_names: Vec<String>,
    pub firing_alerts_count: usize,
    pub insights_count: usize,
    pub warning_events_count: usize,
}

/// Alerts page
#[derive(Template)]
#[template(path = "alerts.html")]
pub struct AlertsTemplate<'a> {
    pub active_page: String,
    pub version: String,
    pub alerts: &'a [crate::prometheus::Alert],
    pub firing_count: usize,
    pub pending_count: usize,
    pub firing_alerts_count: usize,
    pub insights_count: usize,
    pub warning_events_count: usize,
}

/// Insights page
#[derive(Template)]
#[template(path = "insights.html")]
pub struct InsightsTemplate<'a> {
    pub active_page: String,
    pub version: String,
    pub insights: &'a [super::insights::Insight],
    pub high_count: usize,
    pub medium_count: usize,
    pub low_count: usize,
    pub firing_alerts_count: usize,
    pub insights_count: usize,
    pub warning_events_count: usize,
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

/// Envoy pod information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvoyPod {
    pub name: String,
    pub namespace: String,
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

/// Envoy page
#[derive(Template)]
#[template(path = "envoy.html")]
pub struct EnvoyTemplate<'a> {
    pub pods: &'a [EnvoyPod],
    pub envoy_version: String,
    pub active_page: String,
    pub version: String,
    pub metrics_json: String,
    pub pod_names: Vec<String>,
    pub firing_alerts_count: usize,
    pub insights_count: usize,
    pub warning_events_count: usize,
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
    pub insights_count: usize,
    pub warning_events_count: usize,
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
    pub insights_count: usize,
    pub warning_events_count: usize,
}

/// Pod logs viewing page
#[derive(Template)]
#[template(path = "pod_logs.html")]
pub struct PodLogsTemplate {
    pub pod: PodDetail,
    pub log_lines: Vec<LogLine>,
    pub error_message: Option<String>,
    pub selected_container: Option<String>,
    pub tail_lines: usize,
    pub active_page: String,
    pub version: String,
    pub firing_alerts_count: usize,
    pub insights_count: usize,
    pub warning_events_count: usize,
}

/// Individual log line with level detection
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogLine {
    pub content: String,
    pub level: LogLevel,
}

/// Log level enum for color-coding
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum LogLevel {
    Error,
    Warning,
    Info,
    Debug,
    Trace,
    Unknown,
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
    pub cpu_limit: String,
    pub cpu_request: String,
    pub memory_limit: String,
    pub memory_request: String,
    pub cpu_percent: String,
    pub memory_percent: String,
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
    pub insights_count: usize,
    pub warning_events_count: usize,
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

/// Services list page
#[derive(Template)]
#[template(path = "services.html")]
pub struct ServicesTemplate {
    pub services: Vec<ServiceInfo>,
    pub cluster_ip_count: usize,
    pub load_balancer_count: usize,
    pub node_port_count: usize,
    pub active_page: String,
    pub version: String,
    pub firing_alerts_count: usize,
    pub insights_count: usize,
    pub warning_events_count: usize,
}

/// Service information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceInfo {
    pub cluster_name: String,
    pub name: String,
    pub namespace: String,
    pub service_type: String,
    pub cluster_ip: String,
    pub external_ip: String,
    pub ports: String,
    pub age: String,
    pub selector: String,
}

/// Service detail page
#[derive(Template)]
#[template(path = "service_detail.html")]
pub struct ServiceDetailTemplate {
    pub service: ServiceDetail,
    pub active_page: String,
    pub version: String,
    pub firing_alerts_count: usize,
    pub insights_count: usize,
    pub warning_events_count: usize,
}

/// Detailed service information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceDetail {
    pub cluster_name: String,
    pub name: String,
    pub namespace: String,
    pub service_type: String,
    pub cluster_ip: String,
    pub external_ip: String,
    pub ports: Vec<ServicePort>,
    pub age: String,
    pub selector: Vec<(String, String)>,
    pub endpoints: Vec<String>,
    pub session_affinity: String,
    pub labels: Vec<(String, String)>,
}

/// Service port information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServicePort {
    pub name: String,
    pub protocol: String,
    pub port: u32,
    pub target_port: String,
    pub node_port: Option<u32>,
}

#[derive(Template)]
#[template(path = "events.html")]
pub struct EventsTemplate {
    pub events: Vec<EventInfo>,
    pub namespaces: Vec<String>,
    pub object_types: Vec<String>,
    pub warning_count: usize,
    pub normal_count: usize,
    pub active_page: String,
    pub version: String,
    pub firing_alerts_count: usize,
    pub insights_count: usize,
    pub warning_events_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventInfo {
    pub cluster_name: String,
    pub namespace: String,
    pub name: String,
    pub event_type: String,
    pub reason: String,
    pub message: String,
    pub object_kind: String,
    pub object_name: String,
    pub source: String,
    pub count: u32,
    pub first_seen: String,
    pub last_seen: String,
}

/// Deployments list page
#[derive(Template)]
#[template(path = "deployments.html")]
pub struct DeploymentsTemplate {
    pub deployments: Vec<DeploymentInfo>,
    pub available_count: usize,
    pub progressing_count: usize,
    pub unavailable_count: usize,
    pub active_page: String,
    pub version: String,
    pub firing_alerts_count: usize,
    pub insights_count: usize,
    pub warning_events_count: usize,
}

/// Deployment information for list view
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeploymentInfo {
    pub cluster_name: String,
    pub namespace: String,
    pub name: String,
    pub ready_replicas: u32,
    pub desired_replicas: u32,
    pub available_replicas: u32,
    pub unavailable_replicas: u32,
    pub status: String,
    pub age: String,
    pub strategy: String,
}

/// Deployment detail page
#[derive(Template)]
#[template(path = "deployment_detail.html")]
pub struct DeploymentDetailTemplate {
    pub deployment: DeploymentDetail,
    pub active_page: String,
    pub version: String,
    pub firing_alerts_count: usize,
    pub insights_count: usize,
    pub warning_events_count: usize,
}

/// Detailed deployment information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeploymentDetail {
    pub cluster_name: String,
    pub namespace: String,
    pub name: String,
    pub ready_replicas: u32,
    pub desired_replicas: u32,
    pub available_replicas: u32,
    pub unavailable_replicas: u32,
    pub updated_replicas: u32,
    pub status: String,
    pub age: String,
    pub strategy: String,
    pub max_surge: String,
    pub max_unavailable: String,
    pub labels: Vec<(String, String)>,
    pub selector: Vec<(String, String)>,
    pub pods: Vec<crate::k8s::client::PodInfo>,
    pub conditions: Vec<DeploymentCondition>,
}

/// Deployment condition information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeploymentCondition {
    pub condition_type: String,
    pub status: String,
    pub reason: String,
    pub message: String,
    pub last_update: String,
}
