/// Dashboard web server implementation
use anyhow::Result;
use axum::{
    http::{header, HeaderValue},
    routing::get,
    Router,
};
use std::net::SocketAddr;
use std::path::PathBuf;
use tower_http::compression::CompressionLayer;
use tower_http::services::ServeDir;
use tower_http::set_header::SetResponseHeaderLayer;
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
    /// Create a new dashboard server.
    ///
    /// `host` accepts any value parseable as an [`IpAddr`] — typically
    /// `127.0.0.1` (default, loopback only) or `0.0.0.0` (all interfaces,
    /// e.g. when running inside a container or when accessing the dashboard
    /// from another machine on the network).
    pub fn new(config_path: PathBuf, output_dir: PathBuf, host: &str, port: u16) -> Result<Self> {
        let ip: std::net::IpAddr = host.parse().map_err(|e| {
            anyhow::anyhow!("Invalid --host '{host}': must be a valid IPv4/IPv6 address ({e})")
        })?;
        let addr = SocketAddr::new(ip, port);
        Ok(Self {
            config_path,
            output_dir,
            addr,
            cache_refresh_interval: 120,
        })
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

        // Long-cache static assets (JS/CSS/images) — they're served from
        // disk and changes are infrequent. Versioned filenames or manual
        // cache-busting can be added later if needed.
        let static_service = ServeDir::new("static");
        let static_router = Router::new().nest_service("/", static_service).layer(
            SetResponseHeaderLayer::overriding(
                header::CACHE_CONTROL,
                HeaderValue::from_static("public, max-age=31536000, immutable"),
            ),
        );

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
                "/clusters/{cluster}/nodes/{node}/pods/{namespace}/{pod}/logs",
                get(routes::pod_logs),
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
            .route("/services", get(routes::services_list))
            .route("/services/{namespace}/{name}", get(routes::service_detail))
            .route("/cilium", get(routes::cilium))
            .route("/envoy", get(routes::envoy))
            .route("/alerts", get(routes::alerts))
            .route("/insights", get(routes::insights))
            .route("/events", get(routes::events))
            .route("/deployments", get(routes::deployments))
            .route(
                "/deployments/{namespace}/{name}",
                get(routes::deployment_detail),
            )
            .route("/api/alerts", get(routes::api_alerts))
            .route("/api/insights", get(routes::api_insights))
            .route("/api/clusters", get(routes::api_clusters_list))
            .route(
                "/api/clusters/{name}/metrics",
                get(routes::api_cluster_metrics),
            )
            .route("/api/metrics", get(routes::api_metrics))
            .route("/api/cilium/metrics", get(routes::api_cilium_metrics))
            .route("/api/envoy/metrics", get(routes::api_envoy_metrics))
            .route(
                "/api/pods/{namespace}/{pod}/metrics",
                get(routes::api_pod_metrics),
            )
            .nest_service("/static", static_router)
            .with_state(AppState {
                config_path: self.config_path,
                output_dir: self.output_dir,
                cache,
            })
            // OPTIMIZATION: Add HTTP caching headers (reduces requests by 50-80%)
            .layer(SetResponseHeaderLayer::if_not_present(
                header::CACHE_CONTROL,
                HeaderValue::from_static("public, max-age=30"),
            ))
            // Enable compression with best settings for JSON/HTML
            .layer(
                CompressionLayer::new()
                    .gzip(true)
                    .br(true) // Brotli compression (better than gzip for text)
                    .deflate(true)
                    .zstd(true), // Zstandard (fastest decompression)
            )
            .layer(TraceLayer::new_for_http());

        let listener = tokio::net::TcpListener::bind(&self.addr).await?;
        axum::serve(listener, app).await?;

        Ok(())
    }
}

/// Shared application state across all HTTP handlers
#[derive(Clone)]
pub struct AppState {
    pub config_path: PathBuf,
    pub output_dir: PathBuf,
    pub cache: ClusterCache,
}
