/// Oxide - Talos Kubernetes with Cilium
///
/// A Rust-based tool for deploying Talos Linux Kubernetes clusters with Cilium CNI.
/// Currently supports Hetzner Cloud, with more providers coming soon.
mod autoscaler;
mod cilium;
mod cli;
mod cluster;
mod config;
mod examples;
mod hcloud;
mod helm;
mod k8s;
mod metrics_server;
mod prometheus;
mod talos;
mod utils;

use clap::Parser;
use tracing::error;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use crate::autoscaler::Autoscaler;
use crate::cli::{Cli, Commands, NodeType};
use crate::cluster::Cluster;
use crate::config::ClusterConfig;
use crate::examples::Examples;
use crate::hcloud::server::NodeRole;
use crate::metrics_server::MetricsServer;
use crate::prometheus::Prometheus;

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    // Initialize tracing
    let log_level = if cli.verbose { "debug" } else { "info" };
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| format!("oxide={}", log_level).into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    // Execute command
    let result = match cli.command {
        Commands::Create => Cluster::create(&cli.config, &cli.output).await,
        Commands::Destroy => Cluster::destroy(&cli.config, &cli.output).await,
        Commands::Status => Cluster::status(&cli.config, &cli.output).await,
        Commands::Init => ClusterConfig::init(&cli.config).await,
        Commands::Scale {
            ref node_type,
            count,
            ref pool,
            force,
            timeout,
        } => {
            let role = match node_type {
                NodeType::ControlPlane => NodeRole::ControlPlane,
                NodeType::Worker => NodeRole::Worker,
            };
            Cluster::scale(
                &cli.config,
                &cli.output,
                role,
                pool.as_deref(),
                count,
                force,
                timeout,
            )
            .await
        }
        Commands::Upgrade {
            ref version,
            preserve,
            control_plane,
            workers,
            wait,
            stage,
        } => {
            Cluster::upgrade(
                &cli.config,
                &cli.output,
                version.clone(),
                preserve,
                control_plane,
                workers,
                wait,
                stage,
            )
            .await
        }
        Commands::DeployNginx => Examples::deploy_nginx(&cli.output).await,
        Commands::InstallPrometheus => Prometheus::install(&cli.config, &cli.output).await,
        Commands::PrometheusStatus => Prometheus::status(&cli.config, &cli.output).await,
        Commands::UninstallPrometheus => Prometheus::uninstall(&cli.config, &cli.output).await,
        Commands::InstallAutoscaler => Autoscaler::install(&cli.config, &cli.output).await,
        Commands::UninstallAutoscaler => Autoscaler::uninstall(&cli.output).await,
        Commands::InstallMetricsServer => MetricsServer::install(&cli.output).await,
        Commands::UninstallMetricsServer => MetricsServer::uninstall(&cli.output).await,
    };

    if let Err(e) = result {
        error!("Error: {:#}", e);
        std::process::exit(1);
    }
}
