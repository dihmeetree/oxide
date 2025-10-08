/// Dashboard web server implementation
use anyhow::Result;
use axum::{routing::get, Router};
use std::net::SocketAddr;
use std::path::PathBuf;
use tower_http::compression::CompressionLayer;
use tower_http::services::ServeDir;
use tower_http::trace::TraceLayer;
use tracing::info;

use super::cache::ClusterCache;
use super::routes;

/// Dashboard server configuration
pub struct DashboardServer {
    config_path: PathBuf,
    output_dir: PathBuf,
    addr: SocketAddr,
    cache_refresh_interval: u64,
}

impl DashboardServer {
    /// Create a new dashboard server
    pub fn new(config_path: PathBuf, output_dir: PathBuf, port: u16) -> Self {
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        Self {
            config_path,
            output_dir,
            addr,
            cache_refresh_interval: 120,
        }
    }

    /// Start the dashboard server
    pub async fn serve(self) -> Result<()> {
        info!("Starting Oxide Dashboard on http://{}", self.addr);
        info!(
            "Cache refresh interval: {} seconds",
            self.cache_refresh_interval
        );
        info!("Press Ctrl+C to stop");

        // Initialize cache
        let cache = ClusterCache::new();

        // Start background refresh immediately (will load data in background)
        info!("Starting background data refresh...");
        cache.start_background_refresh(self.config_path.clone(), self.cache_refresh_interval);

        let app = Router::new()
            .route("/", get(routes::index))
            .route("/clusters", get(routes::clusters_list))
            .route("/clusters/create", get(routes::clusters_create_form))
            .route(
                "/clusters/create",
                axum::routing::post(routes::clusters_create),
            )
            .route("/clusters/{name}", get(routes::cluster_detail))
            .route("/clusters/{cluster}/nodes/{node}", get(routes::node_detail))
            .route(
                "/clusters/{cluster}/nodes/{node}/pods/{namespace}/{pod}",
                get(routes::pod_detail),
            )
            .route(
                "/clusters/{name}/scale",
                axum::routing::post(routes::cluster_scale),
            )
            .route(
                "/clusters/{name}/upgrade",
                axum::routing::post(routes::cluster_upgrade),
            )
            .route(
                "/clusters/{name}/delete",
                axum::routing::post(routes::cluster_delete),
            )
            .route("/metrics", get(routes::metrics))
            .route("/pods", get(routes::pods_list))
            .route("/nodes", get(routes::nodes_list))
            .route("/cilium", get(routes::cilium))
            .route("/alerts", get(routes::alerts))
            .route("/api/clusters", get(routes::api_clusters_list))
            .route("/api/metrics", get(routes::api_metrics))
            .route("/api/cilium/metrics", get(routes::api_cilium_metrics))
            .route(
                "/api/pods/{namespace}/{pod}/metrics",
                get(routes::api_pod_metrics),
            )
            .nest_service("/static", ServeDir::new("static"))
            .with_state(AppState {
                config_path: self.config_path,
                output_dir: self.output_dir,
                cache,
            })
            .layer(CompressionLayer::new())
            .layer(TraceLayer::new_for_http());

        let listener = tokio::net::TcpListener::bind(&self.addr).await?;
        axum::serve(listener, app).await?;

        Ok(())
    }
}

/// Shared application state
#[derive(Clone)]
pub struct AppState {
    #[allow(dead_code)]
    pub config_path: PathBuf,
    pub output_dir: PathBuf,
    pub cache: ClusterCache,
}
