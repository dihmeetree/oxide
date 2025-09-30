/// Oxide - Talos Kubernetes with Cilium
///
/// A Rust-based tool for deploying Talos Linux Kubernetes clusters with Cilium CNI.
/// Currently supports Hetzner Cloud, with more providers coming soon.
mod cilium;
mod config;
mod hcloud;
mod talos;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use tracing::{error, info};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use crate::cilium::CiliumManager;
use crate::config::ClusterConfig;
use crate::hcloud::network::NetworkManager;
use crate::hcloud::server::{NodeRole, ServerManager};
use crate::hcloud::{FirewallManager, HetznerCloudClient};
use crate::talos::{TalosClient, TalosConfigGenerator};

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
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    // Initialize tracing
    let log_level = if cli.verbose { "debug" } else { "info" };
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| format!("talos_hcloud={}", log_level).into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    // Execute command
    let result = match cli.command {
        Commands::Create => create_cluster(&cli).await,
        Commands::Destroy => destroy_cluster(&cli).await,
        Commands::Status => show_status(&cli).await,
        Commands::Init => init_config(&cli).await,
        Commands::Upgrade {
            ref talos_version,
            ref kubernetes_version,
        } => upgrade_cluster(&cli, talos_version.clone(), kubernetes_version.clone()).await,
        Commands::DeployNginx => deploy_nginx(&cli).await,
    };

    if let Err(e) = result {
        error!("Error: {:#}", e);
        std::process::exit(1);
    }
}

/// Create a new Talos cluster
async fn create_cluster(cli: &Cli) -> Result<()> {
    info!("Starting cluster creation...");

    // Check prerequisites
    TalosClient::check_talosctl_installed()
        .await
        .context("talosctl is required")?;
    CiliumManager::check_kubectl_installed()
        .await
        .context("kubectl is required")?;
    CiliumManager::check_helm_installed()
        .await
        .context("helm is required")?;

    // Load configuration
    let config = ClusterConfig::from_file(&cli.config).context("Failed to load configuration")?;

    info!("Cluster name: {}", config.cluster_name);

    // Create Hetzner Cloud client
    let hcloud_token = config.get_hcloud_token()?;
    let hcloud_client = HetznerCloudClient::new(hcloud_token)?;

    // Get current IP for firewall
    let current_ip = FirewallManager::get_current_ip().await?;
    info!("Detected current IP address: {}", current_ip);

    // Create firewall
    let firewall_manager = FirewallManager::new(hcloud_client.clone());
    let firewall = firewall_manager
        .create_cluster_firewall(&config.cluster_name, &current_ip)
        .await?;

    // Create network
    let network_manager = NetworkManager::new(hcloud_client.clone());
    let network = network_manager
        .ensure_network(&config.cluster_name, &config.hcloud.network)
        .await?;

    // Generate Talos configuration first (using placeholder endpoint if needed)
    let cluster_endpoint = config
        .talos
        .cluster_endpoint
        .clone()
        .unwrap_or_else(|| format!("https://{}:6443", "127.0.0.1"));

    info!(
        "Generating Talos configuration with endpoint: {}",
        cluster_endpoint
    );

    let config_generator =
        TalosConfigGenerator::new(config.cluster_name.clone(), config.talos.clone());

    let configs = config_generator
        .generate_configs(&cluster_endpoint, &cli.output)
        .await?;

    // Read generated configs as user_data
    let controlplane_user_data = tokio::fs::read_to_string(&configs.controlplane)
        .await
        .context("Failed to read controlplane config")?;
    let worker_user_data = tokio::fs::read_to_string(&configs.worker)
        .await
        .context("Failed to read worker config")?;

    // Create servers (all in parallel) with user_data
    let server_manager = ServerManager::new(hcloud_client.clone());

    info!("Creating all servers with Talos configuration...");
    let (control_planes, workers) = tokio::join!(
        server_manager.create_control_planes(
            &config.cluster_name,
            &config.control_planes,
            &config.hcloud.location,
            &network,
            &config.talos.version,
            config.talos.hcloud_snapshot_id.as_deref(),
            None,
            Some(controlplane_user_data),
        ),
        server_manager.create_workers(
            &config.cluster_name,
            &config.workers,
            &config.hcloud.location,
            &network,
            &config.talos.version,
            config.talos.hcloud_snapshot_id.as_deref(),
            None,
            Some(worker_user_data),
        )
    );
    let control_planes = control_planes?;
    let workers = workers?;

    // Apply firewall to all servers
    let server_ids: Vec<u64> = control_planes
        .iter()
        .chain(workers.iter())
        .map(|s| s.server.id)
        .collect();
    firewall_manager
        .apply_to_servers(firewall.id, server_ids)
        .await?;

    // Get first control plane IP
    let first_cp = control_planes
        .first()
        .context("No control plane nodes created")?;
    let cluster_endpoint_ip =
        ServerManager::get_server_ip(&first_cp.server).context("Control plane has no public IP")?;
    let actual_cluster_endpoint = config
        .talos
        .cluster_endpoint
        .clone()
        .unwrap_or_else(|| format!("https://{}:6443", cluster_endpoint_ip));

    info!("Actual cluster endpoint: {}", actual_cluster_endpoint);

    // Configure talosconfig with control plane endpoints
    let talos_client = TalosClient::new(configs.talosconfig.clone());
    let control_plane_ips: Vec<String> = control_planes
        .iter()
        .filter_map(|cp| ServerManager::get_server_ip(&cp.server))
        .collect();
    talos_client.configure_endpoints(&control_plane_ips).await?;

    // Patch control plane nodes with actual endpoint if it differs from placeholder
    // Workers use private network and don't need endpoint patching
    if cluster_endpoint != actual_cluster_endpoint {
        info!("Waiting for Talos API and patching control plane with actual endpoint...");
        talos_client
            .patch_cluster_endpoint(&control_planes, &actual_cluster_endpoint)
            .await?;

        info!("Control plane patched successfully");
    } else {
        info!("Endpoint already correct, skipping patch");
    }

    // Bootstrap cluster
    talos_client.bootstrap(first_cp).await?;

    // Wait for API server
    talos_client
        .wait_for_api_server(&cluster_endpoint_ip, 300)
        .await?;

    // Generate kubeconfig
    let kubeconfig_path = cli.output.join("kubeconfig");
    talos_client
        .generate_kubeconfig(&cluster_endpoint_ip, &kubeconfig_path)
        .await?;

    // Install Cilium
    info!("Installing Cilium CNI...");
    let control_plane_count = config.control_planes.iter().map(|cp| cp.count).sum();
    let cilium_manager = CiliumManager::new(
        config.cilium.clone(),
        kubeconfig_path.clone(),
        control_plane_count,
    );
    cilium_manager.install().await?;
    cilium_manager.wait_for_ready(300).await?;

    info!("✓ Cluster creation completed successfully!");
    info!("");
    info!("Cluster details:");
    info!("  Name: {}", config.cluster_name);
    info!("  Endpoint: {}", cluster_endpoint);
    info!("  Control planes: {}", control_planes.len());
    info!("  Workers: {}", workers.len());
    info!("");
    info!("Configuration files:");
    info!("  Talosconfig: {}", configs.talosconfig.display());
    info!("  Kubeconfig: {}", kubeconfig_path.display());
    info!("");
    info!("To access your cluster:");
    info!("  export KUBECONFIG={}", kubeconfig_path.display());
    info!("  kubectl get nodes");

    Ok(())
}

/// Destroy an existing cluster
async fn destroy_cluster(cli: &Cli) -> Result<()> {
    info!("Starting cluster destruction...");

    let config = ClusterConfig::from_file(&cli.config).context("Failed to load configuration")?;

    info!("Cluster name: {}", config.cluster_name);

    let hcloud_token = config.get_hcloud_token()?;
    let hcloud_client = HetznerCloudClient::new(hcloud_token)?;

    // Delete servers
    let server_manager = ServerManager::new(hcloud_client.clone());
    server_manager
        .delete_cluster_servers(&config.cluster_name)
        .await?;

    // Delete firewall
    let firewall_manager = FirewallManager::new(hcloud_client.clone());
    firewall_manager
        .delete_cluster_firewall(&config.cluster_name)
        .await?;

    // Delete network
    let network_manager = NetworkManager::new(hcloud_client.clone());
    network_manager.delete_network(&config.cluster_name).await?;

    info!("✓ Cluster destroyed successfully");

    Ok(())
}

/// Show cluster status
async fn show_status(cli: &Cli) -> Result<()> {
    let config = ClusterConfig::from_file(&cli.config).context("Failed to load configuration")?;

    let hcloud_token = config.get_hcloud_token()?;
    let hcloud_client = HetznerCloudClient::new(hcloud_token)?;

    let server_manager = ServerManager::new(hcloud_client.clone());
    let servers = server_manager
        .list_cluster_servers(&config.cluster_name)
        .await?;

    if servers.is_empty() {
        info!("No servers found for cluster: {}", config.cluster_name);
        return Ok(());
    }

    info!("Cluster: {}", config.cluster_name);
    info!("");
    info!("Servers:");

    let mut control_planes: Vec<_> = servers
        .iter()
        .filter(|s| s.role == NodeRole::ControlPlane)
        .collect();
    control_planes.sort_by_key(|s| &s.server.name);

    let mut workers: Vec<_> = servers
        .iter()
        .filter(|s| s.role == NodeRole::Worker)
        .collect();
    workers.sort_by_key(|s| &s.server.name);

    info!("  Control Planes:");
    for server_info in control_planes {
        let ip =
            ServerManager::get_server_ip(&server_info.server).unwrap_or_else(|| "N/A".to_string());
        let private_ip = ServerManager::get_server_private_ip(&server_info.server)
            .unwrap_or_else(|| "N/A".to_string());
        info!(
            "    - {} (ID: {}, Status: {}, IP: {}, Private IP: {})",
            server_info.server.name,
            server_info.server.id,
            server_info.server.status,
            ip,
            private_ip
        );
    }

    info!("  Workers:");
    for server_info in workers {
        let ip =
            ServerManager::get_server_ip(&server_info.server).unwrap_or_else(|| "N/A".to_string());
        let private_ip = ServerManager::get_server_private_ip(&server_info.server)
            .unwrap_or_else(|| "N/A".to_string());
        info!(
            "    - {} (ID: {}, Status: {}, IP: {}, Private IP: {})",
            server_info.server.name,
            server_info.server.id,
            server_info.server.status,
            ip,
            private_ip
        );
    }

    // Try to show Cilium status if kubeconfig exists
    let kubeconfig_path = cli.output.join("kubeconfig");
    if kubeconfig_path.exists() {
        info!("");
        info!("Cilium Status:");
        let control_plane_count = config.control_planes.iter().map(|cp| cp.count).sum();
        let cilium_manager =
            CiliumManager::new(config.cilium.clone(), kubeconfig_path, control_plane_count);
        match cilium_manager.get_status().await {
            Ok(status) => info!("{}", status),
            Err(e) => info!("Could not get Cilium status: {}", e),
        }
    }

    Ok(())
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

    let config = ClusterConfig::from_file(&cli.config).context("Failed to load configuration")?;

    let kubeconfig_path = cli.output.join("kubeconfig");
    if !kubeconfig_path.exists() {
        anyhow::bail!(
            "Kubeconfig not found at {}. Please create the cluster first.",
            kubeconfig_path.display()
        );
    }

    let control_plane_count = config.control_planes.iter().map(|cp| cp.count).sum();
    let cilium_manager =
        CiliumManager::new(config.cilium.clone(), kubeconfig_path, control_plane_count);

    // Apply nginx deployment and service
    let nginx_deployment_path = std::path::Path::new("nginx-deployment.yaml");
    if !nginx_deployment_path.exists() {
        anyhow::bail!("nginx-deployment.yaml not found in current directory");
    }
    cilium_manager.apply_manifest(nginx_deployment_path).await?;

    // Apply Gateway and HTTPRoute
    let nginx_gateway_path = std::path::Path::new("nginx-gateway.yaml");
    if !nginx_gateway_path.exists() {
        anyhow::bail!("nginx-gateway.yaml not found in current directory");
    }
    cilium_manager.apply_manifest(nginx_gateway_path).await?;

    info!("✓ nginx deployed successfully with Gateway API!");
    info!("");
    info!("To check the status:");
    info!("  kubectl get pods");
    info!("  kubectl get gateway");
    info!("  kubectl get httproute");

    Ok(())
}
