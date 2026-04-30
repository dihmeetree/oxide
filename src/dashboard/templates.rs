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
    pub object_node: Option<String>,
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

#[cfg(test)]
mod tests {
    use super::*;

    // --- ClusterInfo round-trip ---

    #[test]
    fn test_cluster_info_round_trip() {
        let info = ClusterInfo {
            name: "prod".to_string(),
            status: "Running".to_string(),
            nodes: 3,
            version: "v1.7.0".to_string(),
            created: "2023-06-01".to_string(),
        };
        let json = serde_json::to_string(&info).unwrap();
        let info2: ClusterInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(info2.name, info.name);
        assert_eq!(info2.nodes, 3);
        assert_eq!(info2.status, "Running");
    }

    // --- ContainerInfo round-trip ---

    #[test]
    fn test_container_info_round_trip() {
        let c = ContainerInfo {
            name: "nginx".to_string(),
            image: "nginx:1.25".to_string(),
            cpu_request: "100m".to_string(),
            cpu_limit: "500m".to_string(),
            memory_request: "128Mi".to_string(),
            memory_limit: "512Mi".to_string(),
            ready: true,
            restart_count: 2,
        };
        let json = serde_json::to_string(&c).unwrap();
        let c2: ContainerInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(c2.name, "nginx");
        assert!(c2.ready);
        assert_eq!(c2.restart_count, 2);
    }

    // --- PodDetail with nested ContainerInfo ---

    #[test]
    fn test_pod_detail_round_trip() {
        let pod = PodDetail {
            cluster_name: "prod".to_string(),
            node_name: "prod-worker-1".to_string(),
            name: "nginx-abc123".to_string(),
            namespace: "default".to_string(),
            status: "Running".to_string(),
            restarts: 0,
            age: "2d".to_string(),
            ip: "10.0.0.5".to_string(),
            cpu: "50m".to_string(),
            memory: "64Mi".to_string(),
            cpu_limit: "500m".to_string(),
            cpu_request: "100m".to_string(),
            memory_limit: "512Mi".to_string(),
            memory_request: "128Mi".to_string(),
            cpu_percent: "10%".to_string(),
            memory_percent: "12%".to_string(),
            labels: vec![("app".to_string(), "nginx".to_string())],
            containers: vec![ContainerInfo {
                name: "nginx".to_string(),
                image: "nginx:latest".to_string(),
                cpu_request: "100m".to_string(),
                cpu_limit: "500m".to_string(),
                memory_request: "128Mi".to_string(),
                memory_limit: "512Mi".to_string(),
                ready: true,
                restart_count: 0,
            }],
        };
        let json = serde_json::to_string(&pod).unwrap();
        let pod2: PodDetail = serde_json::from_str(&json).unwrap();
        assert_eq!(pod2.name, "nginx-abc123");
        assert_eq!(pod2.containers.len(), 1);
        assert_eq!(pod2.labels[0], ("app".to_string(), "nginx".to_string()));
    }

    // --- ServicePort with optional node_port ---

    #[test]
    fn test_service_port_with_node_port() {
        let port = ServicePort {
            name: "http".to_string(),
            protocol: "TCP".to_string(),
            port: 80,
            target_port: "8080".to_string(),
            node_port: Some(30080),
        };
        let json = serde_json::to_string(&port).unwrap();
        let port2: ServicePort = serde_json::from_str(&json).unwrap();
        assert_eq!(port2.node_port, Some(30080));
    }

    #[test]
    fn test_service_port_without_node_port() {
        let port = ServicePort {
            name: "http".to_string(),
            protocol: "TCP".to_string(),
            port: 443,
            target_port: "8443".to_string(),
            node_port: None,
        };
        let json = serde_json::to_string(&port).unwrap();
        let port2: ServicePort = serde_json::from_str(&json).unwrap();
        assert!(port2.node_port.is_none());
    }

    // --- EventInfo with optional object_node ---

    #[test]
    fn test_event_info_with_node() {
        let ev = EventInfo {
            cluster_name: "prod".to_string(),
            namespace: "default".to_string(),
            name: "pod-crash".to_string(),
            event_type: "Warning".to_string(),
            reason: "OOMKilled".to_string(),
            message: "Container was OOM killed".to_string(),
            object_kind: "Pod".to_string(),
            object_name: "my-pod".to_string(),
            object_node: Some("worker-1".to_string()),
            source: "kubelet".to_string(),
            count: 3,
            first_seen: "2023-01-01T00:00:00Z".to_string(),
            last_seen: "2023-01-01T01:00:00Z".to_string(),
        };
        let json = serde_json::to_string(&ev).unwrap();
        let ev2: EventInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(ev2.object_node, Some("worker-1".to_string()));
        assert_eq!(ev2.count, 3);
    }

    #[test]
    fn test_event_info_no_node() {
        let ev = EventInfo {
            cluster_name: "prod".to_string(),
            namespace: "kube-system".to_string(),
            name: "sched-event".to_string(),
            event_type: "Normal".to_string(),
            reason: "Scheduled".to_string(),
            message: "Pod scheduled".to_string(),
            object_kind: "Pod".to_string(),
            object_name: "coredns-xyz".to_string(),
            object_node: None,
            source: "scheduler".to_string(),
            count: 1,
            first_seen: "2023-01-01T00:00:00Z".to_string(),
            last_seen: "2023-01-01T00:00:00Z".to_string(),
        };
        let json = serde_json::to_string(&ev).unwrap();
        let ev2: EventInfo = serde_json::from_str(&json).unwrap();
        assert!(ev2.object_node.is_none());
    }

    // --- LogLevel serde ---

    #[test]
    fn test_log_level_serde_round_trip() {
        for level in [
            LogLevel::Error,
            LogLevel::Warning,
            LogLevel::Info,
            LogLevel::Debug,
            LogLevel::Trace,
            LogLevel::Unknown,
        ] {
            let json = serde_json::to_string(&level).unwrap();
            let level2: LogLevel = serde_json::from_str(&json).unwrap();
            assert_eq!(level, level2);
        }
    }

    // --- DeploymentInfo round-trip ---

    #[test]
    fn test_deployment_info_round_trip() {
        let d = DeploymentInfo {
            cluster_name: "prod".to_string(),
            namespace: "default".to_string(),
            name: "my-deployment".to_string(),
            ready_replicas: 3,
            desired_replicas: 3,
            available_replicas: 3,
            unavailable_replicas: 0,
            status: "Available".to_string(),
            age: "5d".to_string(),
            strategy: "RollingUpdate".to_string(),
        };
        let json = serde_json::to_string(&d).unwrap();
        let d2: DeploymentInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(d2.name, "my-deployment");
        assert_eq!(d2.ready_replicas, 3);
        assert_eq!(d2.unavailable_replicas, 0);
    }
}
