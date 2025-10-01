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
use crate::hcloud::server::{NodeRole, ServerInfo, ServerManager};
use crate::hcloud::{FirewallManager, HetznerCloudClient, SSHKeyManager};
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
        } => scale_cluster(&cli, node_type.clone(), count, pool.clone()).await,
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

    // Ensure SSH key exists for cluster
    let ssh_key_manager = SSHKeyManager::new(hcloud_client.clone());
    let (ssh_key, private_key) = ssh_key_manager.ensure_ssh_key(&config.cluster_name).await?;

    // Save private key if it was newly generated
    if let Some(private_key_content) = private_key {
        let ssh_key_path = cli.output.join("id_ed25519");
        tokio::fs::write(&ssh_key_path, private_key_content)
            .await
            .context("Failed to save SSH private key")?;
        info!("SSH private key saved to: {}", ssh_key_path.display());

        // Set appropriate permissions (0600)
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = tokio::fs::metadata(&ssh_key_path)
                .await
                .context("Failed to get SSH key metadata")?
                .permissions();
            perms.set_mode(0o600);
            tokio::fs::set_permissions(&ssh_key_path, perms)
                .await
                .context("Failed to set SSH key permissions")?;
        }
    }

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
            Some(ssh_key.id),
            Some(controlplane_user_data),
        ),
        server_manager.create_workers(
            &config.cluster_name,
            &config.workers,
            &config.hcloud.location,
            &network,
            &config.talos.version,
            config.talos.hcloud_snapshot_id.as_deref(),
            Some(ssh_key.id),
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

    // Delete SSH key
    let ssh_key_manager = SSHKeyManager::new(hcloud_client.clone());
    ssh_key_manager
        .delete_cluster_ssh_key(&config.cluster_name)
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

    // Display control plane node pools
    info!("Control Plane Pools:");
    for pool in &config.control_planes {
        let pool_servers = ServerManager::filter_by_role_and_pool(
            &servers,
            NodeRole::ControlPlane,
            Some(&pool.name),
        );
        info!(
            "  {} - {} node(s) (server type: {})",
            pool.name,
            pool_servers.len(),
            pool.server_type
        );
        for server_info in pool_servers {
            let ip = ServerManager::get_server_ip(&server_info.server)
                .unwrap_or_else(|| "N/A".to_string());
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
    }

    info!("");
    info!("Worker Pools:");
    for pool in &config.workers {
        let pool_servers =
            ServerManager::filter_by_role_and_pool(&servers, NodeRole::Worker, Some(&pool.name));
        info!(
            "  {} - {} node(s) (server type: {})",
            pool.name,
            pool_servers.len(),
            pool.server_type
        );
        for server_info in pool_servers {
            let ip = ServerManager::get_server_ip(&server_info.server)
                .unwrap_or_else(|| "N/A".to_string());
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

/// Scale cluster nodes
async fn scale_cluster(
    cli: &Cli,
    node_type: NodeType,
    target_count: u32,
    pool_name: Option<String>,
) -> Result<()> {
    info!("Starting cluster scaling...");

    let config = ClusterConfig::from_file(&cli.config).context("Failed to load configuration")?;

    info!("Cluster name: {}", config.cluster_name);

    let hcloud_token = config.get_hcloud_token()?;
    let hcloud_client = HetznerCloudClient::new(hcloud_token)?;

    // Get existing servers
    let server_manager = ServerManager::new(hcloud_client.clone());
    let all_servers = server_manager
        .list_cluster_servers(&config.cluster_name)
        .await?;

    // Determine role and pool configuration
    let (role, pool_config) = match node_type {
        NodeType::ControlPlane => {
            let pool = if let Some(ref name) = pool_name {
                config
                    .control_planes
                    .iter()
                    .find(|p| &p.name == name)
                    .ok_or_else(|| anyhow::anyhow!("Control plane pool '{}' not found", name))?
            } else {
                config
                    .control_planes
                    .first()
                    .ok_or_else(|| anyhow::anyhow!("No control plane pools configured"))?
            };
            (NodeRole::ControlPlane, pool)
        }
        NodeType::Worker => {
            let pool = if let Some(ref name) = pool_name {
                config
                    .workers
                    .iter()
                    .find(|p| &p.name == name)
                    .ok_or_else(|| anyhow::anyhow!("Worker pool '{}' not found", name))?
            } else {
                config
                    .workers
                    .first()
                    .ok_or_else(|| anyhow::anyhow!("No worker pools configured"))?
            };
            (NodeRole::Worker, pool)
        }
    };

    // Filter servers by role and pool
    let pool_servers =
        ServerManager::filter_by_role_and_pool(&all_servers, role, Some(&pool_config.name));

    let current_count = pool_servers.len() as u32;

    info!(
        "Current {} count in pool '{}': {}",
        role, pool_config.name, current_count
    );
    info!("Target count: {}", target_count);

    if current_count == target_count {
        info!("Cluster is already at the target size");
        return Ok(());
    }

    if target_count > current_count {
        // Scale up
        let nodes_to_add = target_count - current_count;
        info!("Scaling up: adding {} nodes", nodes_to_add);

        scale_up(
            cli,
            &config,
            &hcloud_client,
            &pool_config.name,
            pool_config,
            role,
            nodes_to_add,
            current_count,
        )
        .await?;
    } else {
        // Scale down
        let nodes_to_remove = current_count - target_count;
        info!("Scaling down: removing {} nodes", nodes_to_remove);

        scale_down(cli, &server_manager, pool_servers, nodes_to_remove).await?;
    }

    info!("✓ Cluster scaling completed successfully!");

    Ok(())
}

/// Scale up by adding new nodes
#[allow(clippy::too_many_arguments)]
async fn scale_up(
    cli: &Cli,
    config: &ClusterConfig,
    hcloud_client: &HetznerCloudClient,
    pool_name: &str,
    pool_config: &crate::config::NodeConfig,
    role: NodeRole,
    nodes_to_add: u32,
    current_count: u32,
) -> Result<()> {
    // Get network
    let network_manager = NetworkManager::new(hcloud_client.clone());
    let network = network_manager
        .get_or_find_network(&config.cluster_name)
        .await?;

    // Get SSH key
    let ssh_key_manager = SSHKeyManager::new(hcloud_client.clone());
    let ssh_key = ssh_key_manager
        .ensure_ssh_key(&config.cluster_name)
        .await?
        .0;

    // Get firewall
    let firewall_manager = FirewallManager::new(hcloud_client.clone());
    let firewall = firewall_manager
        .get_cluster_firewall(&config.cluster_name)
        .await?;

    // Read existing Talos configuration files (cluster must already exist)
    let config_path = if role == NodeRole::ControlPlane {
        cli.output.join("controlplane.yaml")
    } else {
        cli.output.join("worker.yaml")
    };

    if !config_path.exists() {
        anyhow::bail!(
            "Talos configuration file not found: {}\n\
            Scaling requires an existing cluster. Please run 'oxide create' first.",
            config_path.display()
        );
    }

    info!(
        "Using existing {} configuration from {}",
        role,
        config_path.display()
    );

    let user_data = tokio::fs::read_to_string(&config_path)
        .await
        .context(format!(
            "Failed to read config from {}",
            config_path.display()
        ))?;

    let server_manager = ServerManager::new(hcloud_client.clone());

    // Create new nodes
    let mut new_server_ids = Vec::new();
    for i in 0..nodes_to_add {
        let node_index = current_count + i + 1;
        let node_name = format!("{}-{}-{}", config.cluster_name, pool_name, node_index);

        let server_info = server_manager
            .create_single_node(
                &config.cluster_name,
                &node_name,
                &pool_config.server_type,
                &config.hcloud.location,
                network.id,
                role,
                &config.talos.version,
                config.talos.hcloud_snapshot_id.as_deref(),
                Some(ssh_key.id),
                Some(user_data.clone()),
                pool_config.labels.clone(),
            )
            .await?;

        new_server_ids.push(server_info.server.id);
        info!("✓ Node {} created successfully", node_name);
    }

    // Wait for new nodes to become Ready
    info!("Waiting for new nodes to become Ready...");
    let kubeconfig_path = cli.output.join("kubeconfig");

    for i in 0..nodes_to_add {
        let node_index = current_count + i + 1;
        let node_name = format!("{}-{}-{}", config.cluster_name, pool_name, node_index);
        TalosClient::wait_for_node_ready(&kubeconfig_path, &node_name, 300).await?;
    }

    // Apply firewall to new servers
    if let Some(fw) = firewall {
        firewall_manager
            .apply_to_servers(fw.id, new_server_ids)
            .await?;
    }

    info!("All new nodes created and configured");

    Ok(())
}

/// Scale down by removing nodes
async fn scale_down(
    cli: &Cli,
    server_manager: &ServerManager,
    mut pool_servers: Vec<ServerInfo>,
    nodes_to_remove: u32,
) -> Result<()> {
    // Sort servers by index (highest first) to remove newest nodes first
    pool_servers.sort_by(|a, b| b.server.name.cmp(&a.server.name));

    let servers_to_remove: Vec<ServerInfo> = pool_servers
        .into_iter()
        .take(nodes_to_remove as usize)
        .collect();

    if servers_to_remove.is_empty() {
        info!("No servers to remove");
        return Ok(());
    }

    info!("Gracefully removing {} node(s)...", servers_to_remove.len());

    // Initialize Talos client
    let talosconfig_path = cli.output.join("talosconfig");
    if !talosconfig_path.exists() {
        anyhow::bail!(
            "Talosconfig not found at {}. Cannot perform graceful node removal.",
            talosconfig_path.display()
        );
    }
    let talos_client = TalosClient::new(talosconfig_path);

    // Kubeconfig for kubectl delete
    let kubeconfig_path = cli.output.join("kubeconfig");
    if !kubeconfig_path.exists() {
        anyhow::bail!(
            "Kubeconfig not found at {}. Cannot perform graceful node removal.",
            kubeconfig_path.display()
        );
    }

    let mut server_ids_to_delete = Vec::new();

    for server_info in servers_to_remove {
        let node_name = &server_info.server.name;
        let node_ip = ServerManager::get_server_ip(&server_info.server);

        info!(
            "Removing node: {} (ID: {})",
            node_name, server_info.server.id
        );

        // Step 1: Run talosctl reset --graceful --wait
        // This will cordon, drain, leave etcd, erase disks, and power down
        // The --wait flag means it will wait for the reset to complete or timeout
        if let Some(ip) = node_ip {
            info!(
                "Running talosctl reset --graceful on {} ({})...",
                node_name, ip
            );
            info!("This will cordon, drain workloads, and power down the node...");

            // First verify we can connect to Talos API before attempting reset
            match talos_client.get_cluster_info(&ip).await {
                Ok(_) => {
                    // Connection successful, proceed with reset
                    match talos_client.reset_node(&ip, node_name).await {
                        Ok(_) => {
                            info!("✓ Node {} reset completed and powered down", node_name);
                        }
                        Err(e) => {
                            // If reset fails after successful initial connection, it may have powered down
                            let err_msg = e.to_string();
                            if err_msg.contains("connection") || err_msg.contains("timeout") {
                                info!(
                                    "Node {} powered down during reset (expected behavior)",
                                    node_name
                                );
                            } else {
                                anyhow::bail!("Failed to reset node {}: {}", node_name, e);
                            }
                        }
                    }
                }
                Err(e) => {
                    anyhow::bail!(
                        "Cannot connect to Talos API on {} ({}). Check firewall rules and node status: {}",
                        node_name, ip, e
                    );
                }
            }
        } else {
            info!(
                "Warning: Node {} has no public IP, skipping talosctl reset",
                node_name
            );
        }

        // Step 2: Wait for node to be cordoned (SchedulingDisabled)
        info!("Waiting for node {} to be cordoned...", node_name);
        match TalosClient::wait_for_node_cordoned(&kubeconfig_path, node_name, 120).await {
            Ok(_) => {
                info!("✓ Node {} is cordoned and draining", node_name);
            }
            Err(e) => {
                info!(
                    "Warning: Could not verify node {} cordon status: {}. Continuing...",
                    node_name, e
                );
            }
        }

        // Step 3: Delete from Kubernetes
        info!("Deleting node {} from Kubernetes...", node_name);
        match TalosClient::delete_kubernetes_node(&kubeconfig_path, node_name).await {
            Ok(_) => {
                info!("✓ Node {} removed from Kubernetes", node_name);
            }
            Err(e) => {
                info!(
                    "Warning: Failed to delete node {} from Kubernetes: {}. Continuing...",
                    node_name, e
                );
            }
        }

        // Collect server ID for final cleanup
        server_ids_to_delete.push(server_info.server.id);
    }

    // Step 3: Delete servers from Hetzner Cloud
    info!("Deleting servers from Hetzner Cloud...");
    server_manager.delete_servers(server_ids_to_delete).await?;

    info!("✓ All nodes removed successfully");

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
