/// Oxide - Talos Kubernetes with Cilium
///
/// A Rust-based tool for deploying Talos Linux Kubernetes clusters with Cilium CNI.
/// Currently supports Hetzner Cloud, with more providers coming soon.
mod autoscaler;
mod cilium;
mod cluster;
mod config;
mod hcloud;
mod helm;
mod k8s;
mod metrics_server;
mod prometheus;
mod talos;
mod utils;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use tracing::{error, info};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use crate::autoscaler::Autoscaler;
use crate::cluster::Cluster;
use crate::config::ClusterConfig;
use crate::hcloud::server::NodeRole;
use crate::k8s::Resources;
use crate::metrics_server::MetricsServer;
use crate::prometheus::Prometheus;

#[derive(Parser)]
#[command(name = "oxide")]
#[command(about = "Deploy Talos Linux clusters on Hetzner Cloud", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Configuration file path
    #[arg(short, long, default_value = "cluster.yaml")]
    config: PathBuf,

    /// Output directory for generated files
    #[arg(short, long, default_value = "./output")]
    output: PathBuf,

    /// Enable verbose logging
    #[arg(short, long)]
    verbose: bool,
}

#[derive(Subcommand)]
enum Commands {
    /// Create a new Talos cluster
    Create,

    /// Destroy an existing cluster
    Destroy,

    /// Show cluster status
    Status,

    /// Generate example configuration file
    Init,

    /// Scale cluster nodes
    Scale {
        /// Node type to scale
        #[arg(value_enum)]
        node_type: NodeType,

        /// Target number of nodes
        #[arg(short, long)]
        count: u32,

        /// Node pool name (optional, uses first pool if not specified)
        #[arg(short, long)]
        pool: Option<String>,

        /// Force non-graceful scale down (skip drain, immediate removal)
        #[arg(long)]
        force: bool,

        /// Timeout in seconds for graceful node reset (default: 600)
        #[arg(long, default_value = "600")]
        timeout: u64,
    },

    /// Upgrade cluster
    Upgrade {
        /// New Talos version
        #[arg(long)]
        talos_version: Option<String>,

        /// New Kubernetes version
        #[arg(long)]
        kubernetes_version: Option<String>,
    },

    /// Deploy nginx with Gateway API
    DeployNginx,

    /// Install Prometheus monitoring stack
    InstallPrometheus,

    /// Show Prometheus status
    PrometheusStatus,

    /// Uninstall Prometheus monitoring stack
    UninstallPrometheus,

    /// Deploy cluster autoscaler
    DeployAutoscaler,

    /// Uninstall cluster autoscaler
    UninstallAutoscaler,

    /// Install Kubernetes Metrics Server
    InstallMetricsServer,

    /// Uninstall Kubernetes Metrics Server
    UninstallMetricsServer,
}

#[derive(Debug, Clone, clap::ValueEnum)]
enum NodeType {
    ControlPlane,
    Worker,
}

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
        Commands::Create => create_cluster(&cli).await,
        Commands::Destroy => destroy_cluster(&cli).await,
        Commands::Status => show_status(&cli).await,
        Commands::Init => init_config(&cli).await,
        Commands::Scale {
            ref node_type,
            count,
            ref pool,
            force,
            timeout,
        } => scale_cluster(&cli, node_type.clone(), count, pool.clone(), force, timeout).await,
        Commands::Upgrade {
            ref talos_version,
            ref kubernetes_version,
        } => upgrade_cluster(&cli, talos_version.clone(), kubernetes_version.clone()).await,
        Commands::DeployNginx => deploy_nginx(&cli).await,
        Commands::InstallPrometheus => install_prometheus(&cli).await,
        Commands::PrometheusStatus => prometheus_status(&cli).await,
        Commands::UninstallPrometheus => uninstall_prometheus(&cli).await,
        Commands::DeployAutoscaler => deploy_autoscaler(&cli).await,
        Commands::UninstallAutoscaler => uninstall_autoscaler(&cli).await,
        Commands::InstallMetricsServer => install_metrics_server(&cli).await,
        Commands::UninstallMetricsServer => uninstall_metrics_server(&cli).await,
    };

    if let Err(e) = result {
        error!("Error: {:#}", e);
        std::process::exit(1);
    }
}

/// Create a new Talos cluster
async fn create_cluster(cli: &Cli) -> Result<()> {
    let config = ClusterConfig::from_file(&cli.config).context("Failed to load configuration")?;
    let cluster = Cluster::new(config, cli.output.clone());
    cluster.create().await
}

/// Destroy an existing cluster
async fn destroy_cluster(cli: &Cli) -> Result<()> {
    let config = ClusterConfig::from_file(&cli.config).context("Failed to load configuration")?;
    let cluster = Cluster::new(config, cli.output.clone());
    cluster.destroy().await
}

/// Show cluster status
async fn show_status(cli: &Cli) -> Result<()> {
    let config = ClusterConfig::from_file(&cli.config).context("Failed to load configuration")?;
    let cluster = Cluster::new(config, cli.output.clone());
    cluster.status().await
}

/// Initialize example configuration file
async fn init_config(cli: &Cli) -> Result<()> {
    if cli.config.exists() {
        anyhow::bail!(
            "Configuration file already exists: {}",
            cli.config.display()
        );
    }

    let example_config = ClusterConfig::example();
    let yaml = serde_yaml::to_string(&example_config)?;

    tokio::fs::write(&cli.config, yaml)
        .await
        .context("Failed to write configuration file")?;

    info!("Example configuration created: {}", cli.config.display());
    info!("");
    info!("Next steps:");
    info!("  1. Edit the configuration file to match your requirements");
    info!("  2. Set your Hetzner Cloud API token:");
    info!("     export HCLOUD_TOKEN=your-token-here");
    info!("  3. Create the cluster:");
    info!("     oxide create");

    Ok(())
}

/// Scale cluster nodes
async fn scale_cluster(
    cli: &Cli,
    node_type: NodeType,
    target_count: u32,
    pool_name: Option<String>,
    force: bool,
    timeout: u64,
) -> Result<()> {
    let config = ClusterConfig::from_file(&cli.config).context("Failed to load configuration")?;

    let role = match node_type {
        NodeType::ControlPlane => NodeRole::ControlPlane,
        NodeType::Worker => NodeRole::Worker,
    };

    let cluster = Cluster::new(config, cli.output.clone());
    cluster
        .scale(role, pool_name.as_deref(), target_count, force, timeout)
        .await
}

/// Upgrade cluster
async fn upgrade_cluster(
    _cli: &Cli,
    _talos_version: Option<String>,
    _kubernetes_version: Option<String>,
) -> Result<()> {
    anyhow::bail!("Cluster upgrade is not yet implemented");
}

/// Deploy nginx with Gateway API
async fn deploy_nginx(cli: &Cli) -> Result<()> {
    info!("Deploying nginx with Gateway API...");

    let kubeconfig_path = cli.output.join("kubeconfig");
    if !kubeconfig_path.exists() {
        anyhow::bail!(
            "Kubeconfig not found at {}. Please create the cluster first.",
            kubeconfig_path.display()
        );
    }

    // Apply nginx deployment and service
    let nginx_deployment_path = std::path::Path::new("nginx-deployment.yaml");
    if !nginx_deployment_path.exists() {
        anyhow::bail!("nginx-deployment.yaml not found in current directory");
    }
    Resources::apply_manifest(&kubeconfig_path, nginx_deployment_path).await?;

    // Apply Gateway and HTTPRoute
    let nginx_gateway_path = std::path::Path::new("nginx-gateway.yaml");
    if !nginx_gateway_path.exists() {
        anyhow::bail!("nginx-gateway.yaml not found in current directory");
    }
    Resources::apply_manifest(&kubeconfig_path, nginx_gateway_path).await?;

    info!("✓ nginx deployed successfully with Gateway API!");
    info!("");
    info!("To check the status:");
    info!("  kubectl get pods");
    info!("  kubectl get gateway");
    info!("  kubectl get httproute");

    Ok(())
}

/// Install Prometheus monitoring stack
async fn install_prometheus(cli: &Cli) -> Result<()> {
    info!("Installing Prometheus monitoring stack...");

    let config = ClusterConfig::from_file(&cli.config).context("Failed to load configuration")?;

    let prometheus_config = config
        .prometheus
        .ok_or_else(|| anyhow::anyhow!("Prometheus configuration not found in cluster config"))?;

    let kubeconfig_path = cli.output.join("kubeconfig");
    if !kubeconfig_path.exists() {
        anyhow::bail!(
            "Kubeconfig not found at {}. Please create the cluster first.",
            kubeconfig_path.display()
        );
    }

    let prometheus = Prometheus::new(prometheus_config.clone(), kubeconfig_path);

    prometheus.install().await?;
    prometheus.wait_for_ready(600).await?;

    info!("✓ Prometheus monitoring stack installed successfully!");
    info!("");

    if prometheus_config.enable_grafana {
        let grafana_info = prometheus.get_grafana_info().await?;
        info!("{}", grafana_info);
    }

    info!("To check Prometheus status:");
    info!("  oxide prometheus-status");

    Ok(())
}

/// Show Prometheus status
async fn prometheus_status(cli: &Cli) -> Result<()> {
    let config = ClusterConfig::from_file(&cli.config).context("Failed to load configuration")?;

    let prometheus_config = config
        .prometheus
        .ok_or_else(|| anyhow::anyhow!("Prometheus configuration not found in cluster config"))?;

    let kubeconfig_path = cli.output.join("kubeconfig");
    if !kubeconfig_path.exists() {
        anyhow::bail!(
            "Kubeconfig not found at {}. Please create the cluster first.",
            kubeconfig_path.display()
        );
    }

    let prometheus = Prometheus::new(prometheus_config.clone(), kubeconfig_path);

    let status = prometheus.get_status().await?;
    info!("Prometheus Status:");
    info!("{}", status);

    if prometheus_config.enable_grafana {
        info!("");
        let grafana_info = prometheus.get_grafana_info().await?;
        info!("{}", grafana_info);
    }

    Ok(())
}

/// Uninstall Prometheus monitoring stack
async fn uninstall_prometheus(cli: &Cli) -> Result<()> {
    info!("Uninstalling Prometheus monitoring stack...");

    let config = ClusterConfig::from_file(&cli.config).context("Failed to load configuration")?;

    let prometheus_config = config
        .prometheus
        .ok_or_else(|| anyhow::anyhow!("Prometheus configuration not found in cluster config"))?;

    let kubeconfig_path = cli.output.join("kubeconfig");
    if !kubeconfig_path.exists() {
        anyhow::bail!(
            "Kubeconfig not found at {}. Please create the cluster first.",
            kubeconfig_path.display()
        );
    }

    let prometheus = Prometheus::new(prometheus_config, kubeconfig_path);
    prometheus.uninstall().await?;

    info!("✓ Prometheus monitoring stack uninstalled successfully!");

    Ok(())
}

/// Deploy cluster autoscaler
async fn deploy_autoscaler(cli: &Cli) -> Result<()> {
    let config = ClusterConfig::from_file(&cli.config).context("Failed to load configuration")?;

    let autoscaler_config = config
        .autoscaler
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Autoscaler not configured in cluster.yaml"))?;

    let kubeconfig_path = cli.output.join("kubeconfig");
    let worker_config_path = cli.output.join("worker.yaml");

    let autoscaler = Autoscaler::new(kubeconfig_path);
    autoscaler
        .deploy(&config, autoscaler_config, &worker_config_path)
        .await
}

/// Uninstall cluster autoscaler
async fn uninstall_autoscaler(cli: &Cli) -> Result<()> {
    let kubeconfig_path = cli.output.join("kubeconfig");
    let autoscaler = Autoscaler::new(kubeconfig_path);
    autoscaler.uninstall().await
}

/// Install Kubernetes Metrics Server
async fn install_metrics_server(cli: &Cli) -> Result<()> {
    let kubeconfig_path = cli.output.join("kubeconfig");
    let metrics_server = MetricsServer::new(kubeconfig_path);
    metrics_server.install().await
}

/// Uninstall Kubernetes Metrics Server
async fn uninstall_metrics_server(cli: &Cli) -> Result<()> {
    let kubeconfig_path = cli.output.join("kubeconfig");
    let metrics_server = MetricsServer::new(kubeconfig_path);
    metrics_server.uninstall().await
}
