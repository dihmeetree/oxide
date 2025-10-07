/// CLI command definitions and handling
use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "oxide")]
#[command(about = "Deploy Talos Linux clusters on Hetzner Cloud", long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,

    /// Configuration file path
    #[arg(short, long, default_value = "cluster.yaml")]
    pub config: PathBuf,

    /// Output directory for generated files
    #[arg(short, long, default_value = "./output")]
    pub output: PathBuf,

    /// Enable verbose logging
    #[arg(short, long)]
    pub verbose: bool,
}

#[derive(Subcommand)]
pub enum Commands {
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
        /// Talos version (e.g., v1.11.0)
        #[arg(long)]
        version: String,

        /// Preserve node data during upgrade
        #[arg(long, default_value = "true")]
        preserve: bool,

        /// Upgrade control plane nodes
        #[arg(long)]
        control_plane: bool,

        /// Upgrade worker nodes
        #[arg(long)]
        workers: bool,

        /// Wait and observe the upgrade process for each node
        #[arg(long)]
        wait: bool,

        /// Stage the upgrade (useful if upgrade fails due to open files)
        #[arg(long)]
        stage: bool,
    },

    /// Deploy nginx with Gateway API
    DeployNginx,

    /// Install Prometheus monitoring stack
    InstallPrometheus,

    /// Show Prometheus status
    PrometheusStatus,

    /// Uninstall Prometheus monitoring stack
    UninstallPrometheus,

    /// Install cluster autoscaler
    InstallAutoscaler,

    /// Uninstall cluster autoscaler
    UninstallAutoscaler,

    /// Install Kubernetes Metrics Server
    InstallMetricsServer,

    /// Uninstall Kubernetes Metrics Server
    UninstallMetricsServer,
}

#[derive(Debug, Clone, clap::ValueEnum)]
pub enum NodeType {
    ControlPlane,
    Worker,
}
