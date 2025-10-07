/// Askama HTML templates
use askama::Template;
use serde::{Deserialize, Serialize};

/// Index/home page
#[derive(Template)]
#[template(path = "index.html")]
pub struct IndexTemplate {
    pub cluster_count: usize,
    pub total_nodes: usize,
    pub cache_ready: bool,
    pub active_page: String,
    pub version: String,
}

/// Clusters list page
#[derive(Template)]
#[template(path = "clusters.html")]
pub struct ClustersTemplate {
    pub clusters: Vec<ClusterInfo>,
    pub cache_ready: bool,
    pub active_page: String,
    pub version: String,
}

/// Create cluster form page
#[derive(Template)]
#[template(path = "create_cluster.html")]
pub struct CreateClusterTemplate {
    pub active_page: String,
    pub version: String,
}

/// Cluster detail page
#[derive(Template)]
#[template(path = "cluster_detail.html")]
pub struct ClusterDetailTemplate {
    pub cluster: ClusterDetail,
    pub active_page: String,
    pub version: String,
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
}

/// Node detail page
#[derive(Template)]
#[template(path = "node_detail.html")]
pub struct NodeDetailTemplate {
    pub node: NodeDetail,
    pub active_page: String,
    pub version: String,
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
}
