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
    ClusterDetailTemplate, ClustersTemplate, CreateClusterTemplate, IndexTemplate,
    NodeDetailTemplate,
};
use crate::config::ClusterConfig;

/// Home page
pub async fn index(State(state): State<AppState>) -> impl IntoResponse {
    let cache_ready = state.cache.is_ready().await;
    let clusters = state.cache.get_clusters().await;
    let total_nodes: usize = clusters.iter().map(|c| c.nodes).sum();
    let template = IndexTemplate {
        cluster_count: clusters.len(),
        total_nodes,
        cache_ready,
        active_page: "dashboard".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    };
    Html(template.render().unwrap())
}

/// Clusters list page
pub async fn clusters_list(State(state): State<AppState>) -> impl IntoResponse {
    let cache_ready = state.cache.is_ready().await;
    let clusters = state.cache.get_clusters().await;
    let template = ClustersTemplate {
        clusters,
        cache_ready,
        active_page: "clusters".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    };
    Html(template.render().unwrap())
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
        None => {
            // Cluster not found or cache not ready, redirect to clusters list
            Redirect::to("/clusters").into_response()
        }
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
        None => {
            // Node not found or cache not ready, redirect to cluster detail
            Redirect::to(&format!("/clusters/{}", cluster_name)).into_response()
        }
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
            hcloud_snapshot_id: None,
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
