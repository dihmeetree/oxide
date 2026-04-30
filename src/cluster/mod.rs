/// Cluster lifecycle management
///
/// This module orchestrates cluster operations across multiple components:
/// - Hetzner Cloud infrastructure (servers, networks, firewalls)
/// - Talos Linux configuration and bootstrapping
/// - Cilium CNI installation
/// - Kubernetes access and management
/// - Optional components (metrics server, prometheus, autoscaler)
use anyhow::{Context, Result};
use std::path::PathBuf;
use tracing::info;

use crate::autoscaler::Autoscaler;
use crate::cilium::Cilium;
use crate::config::ClusterConfig;
use crate::hcloud::network::NetworkManager;
use crate::hcloud::server::{ServerInfo, ServerManager};
use crate::hcloud::{FirewallManager, HetznerCloudClient, SSHKeyManager};
use crate::helm::Helm;
use crate::k8s::{KubernetesClient, NodeManager};
use crate::metrics_server::MetricsServer;
use crate::prometheus::Prometheus;
use crate::talos::{TalosClient, TalosConfigGenerator};

pub struct Cluster {
    config: ClusterConfig,
    output_dir: PathBuf,
}

/// Parameters for scaling up nodes
struct ScaleUpParams<'a> {
    hcloud_client: &'a HetznerCloudClient,
    pool_name: &'a str,
    pool_config: &'a crate::config::NodeConfig,
    role: crate::hcloud::server::NodeRole,
    nodes_to_add: u32,
    current_count: u32,
}

/// Parameters for scaling down nodes
struct ScaleDownParams<'a> {
    server_manager: &'a ServerManager,
    pool_servers: Vec<ServerInfo>,
    nodes_to_remove: u32,
    force: bool,
    timeout: u64,
}

/// Best-effort cleanup of servers that were freshly created during a scale-up
/// before a subsequent step failed. Logs (but swallows) deletion errors so the
/// caller can surface the original failure to the user.
async fn rollback_new_servers(server_manager: &ServerManager, server_ids: Vec<u64>) {
    if server_ids.is_empty() {
        return;
    }
    tracing::error!(
        "Scale-up failed; rolling back {} newly-created server(s)...",
        server_ids.len()
    );
    if let Err(e) = server_manager.delete_servers(server_ids).await {
        tracing::error!("Failed to roll back new servers (manual cleanup may be required): {e:#}");
    }
}

impl Cluster {
    pub fn new(config: ClusterConfig, output_dir: PathBuf) -> Self {
        Self { config, output_dir }
    }

    /// Create a new cluster
    ///
    /// Wraps `create_cluster_inner` with a best-effort rollback that tears
    /// down any infrastructure that was provisioned before the failure so
    /// orphaned servers, networks, firewalls or SSH keys are not left
    /// behind on the Hetzner Cloud account.
    pub async fn create_cluster(&self) -> Result<()> {
        match self.create_cluster_inner().await {
            Ok(()) => Ok(()),
            Err(err) => {
                tracing::error!(
                    "Cluster creation failed: {err:#}. Rolling back partially created resources..."
                );
                if let Err(rollback_err) = self.rollback_partial_creation().await {
                    tracing::error!(
                        "Rollback encountered errors (manual cleanup may be required): {rollback_err:#}"
                    );
                }
                Err(err)
            }
        }
    }

    /// Best-effort cleanup of any cluster resources that may have been created
    /// before a failure in [`create_cluster_inner`]. Each step is attempted
    /// independently so a failure in one does not prevent the others from
    /// running. Resources that do not exist are treated as success by the
    /// underlying managers.
    async fn rollback_partial_creation(&self) -> Result<()> {
        let hcloud_token = self.config.get_hcloud_token()?;
        let hcloud_client = HetznerCloudClient::new(hcloud_token)?;
        let cluster_name = &self.config.cluster_name;

        let mut errors: Vec<String> = Vec::new();

        let server_manager = ServerManager::new(hcloud_client.clone());
        if let Err(e) = server_manager.delete_cluster_servers(cluster_name).await {
            errors.push(format!("delete servers: {e:#}"));
        }

        let firewall_manager = FirewallManager::new(hcloud_client.clone());
        if let Err(e) = firewall_manager.delete_cluster_firewall(cluster_name).await {
            errors.push(format!("delete firewall: {e:#}"));
        }

        let ssh_key_manager = SSHKeyManager::new(hcloud_client.clone());
        if let Err(e) = ssh_key_manager.delete_cluster_ssh_key(cluster_name).await {
            errors.push(format!("delete ssh key: {e:#}"));
        }

        let network_manager = NetworkManager::new(hcloud_client);
        if let Err(e) = network_manager.delete_network(cluster_name).await {
            errors.push(format!("delete network: {e:#}"));
        }

        if errors.is_empty() {
            info!("Rollback completed successfully");
            Ok(())
        } else {
            Err(anyhow::anyhow!(
                "rollback finished with {} error(s): {}",
                errors.len(),
                errors.join("; ")
            ))
        }
    }

    /// Internal cluster creation routine. Any error returned here triggers a
    /// rollback in [`create_cluster`].
    async fn create_cluster_inner(&self) -> Result<()> {
        info!("Starting cluster creation...");

        // Ensure output directory exists
        tokio::fs::create_dir_all(&self.output_dir)
            .await
            .context("Failed to create output directory")?;

        // Check prerequisites
        TalosClient::check_talosctl_installed()
            .await
            .context("talosctl is required")?;
        KubernetesClient::check_kubectl_installed()
            .await
            .context("kubectl is required")?;
        Helm::check_installed().await.context("helm is required")?;

        info!("Cluster name: {}", self.config.cluster_name);

        // Create Hetzner Cloud client
        let hcloud_token = self.config.get_hcloud_token()?;
        let hcloud_client = HetznerCloudClient::new(hcloud_token)?;

        // Get current IP for firewall
        let current_ip = FirewallManager::get_current_ip().await?;
        info!("Detected current IP address: {}", current_ip);

        // Create firewall
        let firewall_manager = FirewallManager::new(hcloud_client.clone());
        let firewall = firewall_manager
            .create_cluster_firewall(&self.config.cluster_name, &current_ip)
            .await?;

        // Create network
        let network_manager = NetworkManager::new(hcloud_client.clone());
        let network = network_manager
            .ensure_network(&self.config.cluster_name, &self.config.hcloud.network)
            .await?;

        // Ensure SSH key exists for cluster
        let ssh_key_manager = SSHKeyManager::new(hcloud_client.clone());
        let (ssh_key, private_key) = ssh_key_manager
            .ensure_ssh_key(&self.config.cluster_name)
            .await?;

        // Save private key if it was newly generated
        if let Some(private_key_content) = private_key {
            self.save_ssh_key(private_key_content).await?;
        }

        // Generate Talos configuration
        let cluster_endpoint = self
            .config
            .talos
            .cluster_endpoint
            .clone()
            .unwrap_or_else(|| format!("https://{}:6443", "127.0.0.1"));

        info!(
            "Generating Talos configuration with endpoint: {}",
            cluster_endpoint
        );

        let config_generator =
            TalosConfigGenerator::new(self.config.cluster_name.clone(), self.config.talos.clone());

        let configs = config_generator
            .generate_configs(&cluster_endpoint, &self.output_dir)
            .await?;

        // Read generated configs as user_data
        let controlplane_user_data = tokio::fs::read_to_string(&configs.controlplane)
            .await
            .context("Failed to read controlplane config")?;
        let worker_user_data = tokio::fs::read_to_string(&configs.worker)
            .await
            .context("Failed to read worker config")?;

        // Create servers
        let server_manager = ServerManager::new(hcloud_client.clone());

        info!("Creating all servers with Talos configuration...");
        let (control_planes, workers) = tokio::join!(
            server_manager.create_control_planes(
                &self.config.cluster_name,
                &self.config.control_planes,
                &self.config.hcloud.location,
                &network,
                &self.config.talos.version,
                self.config.talos.hcloud_snapshot_id.as_deref(),
                Some(ssh_key.id),
                Some(controlplane_user_data),
            ),
            server_manager.create_workers(
                &self.config.cluster_name,
                &self.config.workers,
                &self.config.hcloud.location,
                &network,
                &self.config.talos.version,
                self.config.talos.hcloud_snapshot_id.as_deref(),
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
        let cluster_endpoint_ip = ServerManager::get_server_ip(&first_cp.server)
            .context("Control plane has no public IP")?;
        let actual_cluster_endpoint = self
            .config
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

        // Patch control plane nodes with actual endpoint if needed
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
        let kubeconfig_path = self.output_dir.join("kubeconfig");
        talos_client
            .generate_kubeconfig(&cluster_endpoint_ip, &kubeconfig_path)
            .await?;

        // Install Cilium
        info!("Installing Cilium CNI...");
        let control_plane_count = self.config.control_planes.iter().map(|cp| cp.count).sum();
        let cilium = Cilium::new(
            self.config.cilium.clone(),
            kubeconfig_path.clone(),
            control_plane_count,
        );
        cilium.install().await?;
        cilium.wait_for_ready(300).await?;

        info!("[OK] Cluster creation completed successfully!");
        info!("Cluster details:");
        info!("  Name: {}", self.config.cluster_name);
        info!("  Endpoint: {}", cluster_endpoint);
        info!("  Control planes: {}", control_planes.len());
        info!("  Workers: {}", workers.len());
        info!("Configuration files:");
        info!("  Talosconfig: {}", configs.talosconfig.display());
        info!("  Kubeconfig: {}", kubeconfig_path.display());

        // Install optional components
        self.install_optional_components().await?;

        info!("To access your cluster:");
        info!("  export KUBECONFIG={}", kubeconfig_path.display());
        info!("  kubectl get nodes");

        Ok(())
    }

    /// Save SSH private key with appropriate permissions
    async fn save_ssh_key(&self, private_key_content: String) -> Result<()> {
        let ssh_key_path = self.output_dir.join("id_ed25519");
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

        Ok(())
    }

    /// Install optional components based on configuration
    async fn install_optional_components(&self) -> Result<()> {
        info!("Installing optional cluster components...");

        let kubeconfig_path = self.output_dir.join("kubeconfig");

        // Install Metrics Server if enabled
        if let Some(metrics_config) = &self.config.metrics_server {
            if metrics_config.enabled {
                let metrics_server = MetricsServer::new(kubeconfig_path.clone());
                metrics_server.install_metrics_server().await?;
            }
        }

        // Install Prometheus if enabled
        if let Some(prometheus_config) = &self.config.prometheus {
            if prometheus_config.enabled {
                let prometheus =
                    Prometheus::new(prometheus_config.clone(), kubeconfig_path.clone());
                prometheus.install_stack().await?;

                // Configure Cilium monitoring after Prometheus is installed
                let control_plane_count =
                    self.config.control_planes.iter().map(|cp| cp.count).sum();
                let cilium = Cilium::new(
                    self.config.cilium.clone(),
                    kubeconfig_path.clone(),
                    control_plane_count,
                );
                cilium.configure_monitoring().await?;
            }
        }

        // Install Cluster Autoscaler if enabled
        if let Some(autoscaler_config) = &self.config.autoscaler {
            if autoscaler_config.enabled {
                let worker_config_path = self.output_dir.join("worker.yaml");
                let autoscaler = Autoscaler::new(kubeconfig_path);
                autoscaler
                    .install_autoscaler(&self.config, autoscaler_config, &worker_config_path)
                    .await?;
            }
        }

        Ok(())
    }

    /// Destroy the cluster
    pub async fn destroy_cluster(&self) -> Result<()> {
        info!("Destroying cluster: {}", self.config.cluster_name);

        let hcloud_token = self.config.get_hcloud_token()?;
        let hcloud_client = HetznerCloudClient::new(hcloud_token)?;

        // Delete servers
        let server_manager = ServerManager::new(hcloud_client.clone());
        server_manager
            .delete_cluster_servers(&self.config.cluster_name)
            .await?;

        // Delete firewall
        let firewall_manager = FirewallManager::new(hcloud_client.clone());
        firewall_manager
            .delete_cluster_firewall(&self.config.cluster_name)
            .await?;

        // Delete SSH key
        let ssh_key_manager = SSHKeyManager::new(hcloud_client.clone());
        ssh_key_manager
            .delete_cluster_ssh_key(&self.config.cluster_name)
            .await?;

        // Delete network
        let network_manager = NetworkManager::new(hcloud_client);
        network_manager
            .delete_network(&self.config.cluster_name)
            .await?;

        // Remove output directory
        if self.output_dir.exists() {
            info!("Removing output directory: {:?}", self.output_dir);
            tokio::fs::remove_dir_all(&self.output_dir)
                .await
                .context("Failed to remove output directory")?;
        }

        info!("[OK] Cluster destroyed successfully!");

        Ok(())
    }

    /// Display cluster status
    pub async fn show_status(&self) -> Result<()> {
        info!("Fetching cluster status for: {}", self.config.cluster_name);

        let hcloud_token = self.config.get_hcloud_token()?;
        let hcloud_client = HetznerCloudClient::new(hcloud_token)?;

        let server_manager = ServerManager::new(hcloud_client);
        let servers = server_manager
            .list_cluster_servers(&self.config.cluster_name)
            .await?;

        if servers.is_empty() {
            info!("No servers found for cluster: {}", self.config.cluster_name);
            return Ok(());
        }

        info!("Cluster: {}", self.config.cluster_name);

        // Display control plane node pools
        info!("Control Plane Pools:");
        for pool in &self.config.control_planes {
            let pool_servers = ServerManager::filter_by_role_and_pool(
                &servers,
                crate::hcloud::server::NodeRole::ControlPlane,
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

        info!("Worker Pools:");
        for pool in &self.config.workers {
            let pool_servers = ServerManager::filter_by_role_and_pool(
                &servers,
                crate::hcloud::server::NodeRole::Worker,
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

        // Try to show Cilium status if kubeconfig exists
        let kubeconfig_path = self.output_dir.join("kubeconfig");
        if kubeconfig_path.exists() {
            info!("Cilium Status:");
            let control_plane_count = self.config.control_planes.iter().map(|cp| cp.count).sum();
            let cilium = Cilium::new(
                self.config.cilium.clone(),
                kubeconfig_path,
                control_plane_count,
            );
            match cilium.get_status().await {
                Ok(status) => info!("{}", status),
                Err(e) => info!("Could not get Cilium status: {}", e),
            }
        }

        Ok(())
    }

    /// Scale a node pool to a target count
    pub async fn scale_cluster(
        &self,
        role: crate::hcloud::server::NodeRole,
        pool_name: Option<&str>,
        target_count: u32,
        force: bool,
        timeout: u64,
    ) -> Result<()> {
        info!("Starting cluster scaling...");
        info!("Cluster name: {}", self.config.cluster_name);

        let hcloud_token = self.config.get_hcloud_token()?;
        let hcloud_client = HetznerCloudClient::new(hcloud_token)?;

        // Get existing servers
        let server_manager = ServerManager::new(hcloud_client.clone());
        let all_servers = server_manager
            .list_cluster_servers(&self.config.cluster_name)
            .await?;

        // Determine pool configuration
        let pool_config = match role {
            crate::hcloud::server::NodeRole::ControlPlane => {
                if let Some(name) = pool_name {
                    self.config
                        .control_planes
                        .iter()
                        .find(|p| p.name == name)
                        .ok_or_else(|| anyhow::anyhow!("Control plane pool '{}' not found", name))?
                } else {
                    self.config
                        .control_planes
                        .first()
                        .ok_or_else(|| anyhow::anyhow!("No control plane pools configured"))?
                }
            }
            crate::hcloud::server::NodeRole::Worker => {
                if let Some(name) = pool_name {
                    self.config
                        .workers
                        .iter()
                        .find(|p| p.name == name)
                        .ok_or_else(|| anyhow::anyhow!("Worker pool '{}' not found", name))?
                } else {
                    self.config
                        .workers
                        .first()
                        .ok_or_else(|| anyhow::anyhow!("No worker pools configured"))?
                }
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
            self.scale_up(ScaleUpParams {
                hcloud_client: &hcloud_client,
                pool_name: &pool_config.name,
                pool_config,
                role,
                nodes_to_add,
                current_count,
            })
            .await?;
        } else {
            // Scale down
            let nodes_to_remove = current_count - target_count;
            info!("Scaling down: removing {} nodes", nodes_to_remove);

            if force {
                info!(
                    "WARNING:  FORCE mode enabled: nodes will be removed immediately without graceful drain"
                );
            }

            self.scale_down(ScaleDownParams {
                server_manager: &server_manager,
                pool_servers,
                nodes_to_remove,
                force,
                timeout,
            })
            .await?;
        }

        info!("[OK] Cluster scaling completed successfully!");

        Ok(())
    }

    /// Scale up by adding new nodes
    async fn scale_up(&self, params: ScaleUpParams<'_>) -> Result<()> {
        let ScaleUpParams {
            hcloud_client,
            pool_name,
            pool_config,
            role,
            nodes_to_add,
            current_count,
        } = params;
        // Get network
        let network_manager = NetworkManager::new(hcloud_client.clone());
        let network = network_manager
            .get_or_find_network(&self.config.cluster_name)
            .await?;

        // Get SSH key
        let ssh_key_manager = SSHKeyManager::new(hcloud_client.clone());
        let ssh_key = ssh_key_manager
            .ensure_ssh_key(&self.config.cluster_name)
            .await?
            .0;

        // Get firewall
        let firewall_manager = FirewallManager::new(hcloud_client.clone());
        let firewall = firewall_manager
            .get_cluster_firewall(&self.config.cluster_name)
            .await?;

        // Read existing Talos configuration files
        let config_path = if role == crate::hcloud::server::NodeRole::ControlPlane {
            self.output_dir.join("controlplane.yaml")
        } else {
            self.output_dir.join("worker.yaml")
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

        // Create new nodes. Track ids so we can roll them back if any
        // subsequent step (node-readiness wait, firewall application) fails;
        // otherwise we would leak servers in Hetzner Cloud while the cluster's
        // desired-state in code shows the scale-up as failed.
        let mut new_server_ids = Vec::new();
        for i in 0..nodes_to_add {
            let node_index = current_count + i + 1;
            let node_name = format!("{}-{}-{}", self.config.cluster_name, pool_name, node_index);

            match server_manager
                .create_single_node(crate::hcloud::server::CreateSingleNodeParams {
                    cluster_name: &self.config.cluster_name,
                    node_name: &node_name,
                    server_type: &pool_config.server_type,
                    location: &self.config.hcloud.location,
                    network_id: network.id,
                    role,
                    talos_version: &self.config.talos.version,
                    snapshot_id: self.config.talos.hcloud_snapshot_id.as_deref(),
                    ssh_key_id: Some(ssh_key.id),
                    user_data: Some(user_data.clone()),
                    labels: pool_config.labels.clone(),
                })
                .await
            {
                Ok(server_info) => {
                    new_server_ids.push(server_info.server.id);
                    info!("[OK] Node {} created successfully", node_name);
                }
                Err(e) => {
                    rollback_new_servers(&server_manager, std::mem::take(&mut new_server_ids))
                        .await;
                    return Err(e).context(format!("Failed to create node {node_name}"));
                }
            }
        }

        // Wait for new nodes to become Ready
        info!("Waiting for new nodes to become Ready...");
        let kubeconfig_path = self.output_dir.join("kubeconfig");

        for i in 0..nodes_to_add {
            let node_index = current_count + i + 1;
            let node_name = format!("{}-{}-{}", self.config.cluster_name, pool_name, node_index);
            if let Err(e) =
                NodeManager::wait_for_node_ready(&kubeconfig_path, &node_name, 300).await
            {
                rollback_new_servers(&server_manager, std::mem::take(&mut new_server_ids)).await;
                return Err(e).context(format!("Node {node_name} failed to become Ready"));
            }
        }

        // Apply firewall to new servers
        if let Some(fw) = firewall {
            if let Err(e) = firewall_manager
                .apply_to_servers(fw.id, new_server_ids.clone())
                .await
            {
                rollback_new_servers(&server_manager, std::mem::take(&mut new_server_ids)).await;
                return Err(e).context("Failed to apply firewall to new servers");
            }
        }

        info!("All new nodes created and configured");

        Ok(())
    }

    /// Scale down by removing nodes with parallel reset and validation
    async fn scale_down(&self, params: ScaleDownParams<'_>) -> Result<()> {
        let ScaleDownParams {
            server_manager,
            mut pool_servers,
            nodes_to_remove,
            force,
            timeout,
        } = params;
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

        // Initialize paths
        let talosconfig_path = self.output_dir.join("talosconfig");
        if !talosconfig_path.exists() {
            anyhow::bail!(
                "Talosconfig not found at {}. Cannot perform graceful node removal.",
                talosconfig_path.display()
            );
        }

        let kubeconfig_path = self.output_dir.join("kubeconfig");
        if !kubeconfig_path.exists() {
            anyhow::bail!(
                "Kubeconfig not found at {}. Cannot perform graceful node removal.",
                kubeconfig_path.display()
            );
        }

        // PRE-FLIGHT VALIDATION
        let node_names: Vec<String> = servers_to_remove
            .iter()
            .map(|s| s.server.name.clone())
            .collect();

        info!("Running pre-flight validation checks...");

        // Validate etcd quorum won't be broken
        NodeManager::validate_etcd_quorum(&kubeconfig_path, &node_names).await?;

        info!("[OK] Pre-flight validation passed");

        // PHASE 1: PARALLEL NODE RESET
        info!("Phase 1/3: Resetting nodes in parallel...");

        let mut reset_tasks = Vec::new();

        for server_info in &servers_to_remove {
            let node_name = server_info.server.name.clone();
            let node_ip = ServerManager::get_server_ip(&server_info.server);
            let talos_client_clone = TalosClient::new(talosconfig_path.clone());
            let kubeconfig_path_clone = kubeconfig_path.clone();

            let task = tokio::spawn(async move {
                if let Some(ip) = node_ip {
                    info!("Resetting node {} ({})...", node_name, ip);

                    let reset_result = talos_client_clone
                        .reset_node_with_timeout(&ip, &node_name, timeout, force, 2)
                        .await;

                    match reset_result {
                        Ok(_) => {
                            info!("[OK] Node {} reset completed", node_name);
                        }
                        Err(e) => {
                            let err_msg = e.to_string();
                            if err_msg.contains("connection closed")
                                || err_msg.contains("broken pipe")
                                || err_msg.contains("reset by peer")
                            {
                                info!(
                                    "[OK] Node {} powered down during reset (expected)",
                                    node_name
                                );
                            } else {
                                return Err(e);
                            }
                        }
                    }

                    // Monitor drain progress if not in force mode
                    if !force {
                        info!("Monitoring drain progress for {}...", node_name);
                        if let Err(e) = NodeManager::monitor_drain_progress(
                            &kubeconfig_path_clone,
                            &node_name,
                            timeout,
                        )
                        .await
                        {
                            info!(
                                "Warning: Failed to monitor drain progress for {}: {}",
                                node_name, e
                            );
                        }
                    }

                    Ok::<String, anyhow::Error>(node_name)
                } else {
                    info!(
                        "WARNING:  Warning: Node {} has no public IP, skipping reset",
                        node_name
                    );
                    Ok::<String, anyhow::Error>(node_name)
                }
            });

            reset_tasks.push(task);
        }

        // Wait for all resets to complete
        info!("Waiting for all node resets to complete...");
        let reset_results = futures::future::join_all(reset_tasks).await;

        let mut successfully_reset = Vec::new();
        let mut failed_resets = Vec::new();

        for (idx, result) in reset_results.into_iter().enumerate() {
            match result {
                Ok(Ok(node_name)) => {
                    successfully_reset.push(node_name);
                }
                Ok(Err(e)) => {
                    let node_name = &servers_to_remove[idx].server.name;
                    failed_resets.push(format!("{}: {}", node_name, e));
                }
                Err(e) => {
                    let node_name = &servers_to_remove[idx].server.name;
                    failed_resets.push(format!("{}: task join error: {}", node_name, e));
                }
            }
        }

        if !failed_resets.is_empty() {
            anyhow::bail!(
                "Failed to reset {} node(s):\n  {}",
                failed_resets.len(),
                failed_resets.join("\n  ")
            );
        }

        info!(
            "[OK] Phase 1 complete: {} nodes reset successfully",
            successfully_reset.len()
        );

        // PHASE 2: DELETE FROM KUBERNETES
        info!("Phase 2/3: Removing nodes from Kubernetes...");

        for node_name in &successfully_reset {
            if let Err(e) =
                NodeManager::wait_for_node_cordoned(&kubeconfig_path, node_name, 120).await
            {
                info!(
                    "WARNING:  Warning: Could not verify node {} cordon status: {}. Proceeding with deletion...",
                    node_name, e
                );
            }

            match NodeManager::delete_node(&kubeconfig_path, node_name).await {
                Ok(_) => {
                    info!("[OK] Node {} removed from Kubernetes", node_name);
                }
                Err(e) => {
                    info!(
                        "WARNING:  Warning: Failed to delete node {} from Kubernetes: {}",
                        node_name, e
                    );
                }
            }
        }

        info!("[OK] Phase 2 complete");

        // PHASE 3: DELETE FROM HETZNER CLOUD
        info!("Phase 3/3: Deleting servers from Hetzner Cloud...");

        let server_ids_to_delete: Vec<u64> =
            servers_to_remove.iter().map(|s| s.server.id).collect();

        server_manager.delete_servers(server_ids_to_delete).await?;

        info!("[OK] Phase 3 complete");
        info!(
            "[OK] All {} nodes removed successfully",
            servers_to_remove.len()
        );

        Ok(())
    }

    /// Upgrade cluster nodes to a new Talos version
    pub async fn upgrade_cluster(&self, options: &UpgradeOptions<'_>) -> Result<()> {
        use crate::talos::client::TalosClient;

        info!("Starting cluster upgrade to Talos {}...", options.version);
        info!("Cluster name: {}", self.config.cluster_name);
        info!(
            "WARNING:  Important: Nodes will be upgraded one at a time to maintain cluster availability"
        );
        if options.control_plane {
            info!("WARNING:  Control plane upgrades are protected - Talos will refuse upgrades that would break etcd quorum");
        }

        let hcloud_token = self.config.get_hcloud_token()?;
        let hcloud_client = crate::hcloud::client::HetznerCloudClient::new(hcloud_token)?;

        let talosconfig_path = self.output_dir.join("talosconfig");
        let talos_client = TalosClient::new(talosconfig_path);

        // Build the installer image from version
        let image = format!("ghcr.io/siderolabs/installer:{}", options.version);

        // Upgrade control plane nodes
        if options.control_plane {
            info!("Upgrading control plane nodes...");

            let all_servers = hcloud_client.list_servers().await?;
            let cp_prefix = format!("{}-control-plane", self.config.cluster_name);
            let servers: Vec<_> = all_servers
                .into_iter()
                .filter(|s| s.name.starts_with(&cp_prefix))
                .collect();

            for server in &servers {
                if let Some(private_ip) = &server.private_net.first().map(|net| &net.ip) {
                    info!(
                        "Upgrading control plane node: {} ({})",
                        server.name, private_ip
                    );

                    talos_client
                        .upgrade(
                            private_ip,
                            &image,
                            options.preserve,
                            options.wait,
                            options.stage,
                        )
                        .await?;

                    info!("[OK] Upgraded {}", server.name);
                }
            }

            info!("[OK] Control plane nodes upgraded successfully");
        }

        // Upgrade worker nodes
        if options.workers {
            info!("Upgrading worker nodes...");

            let all_servers = hcloud_client.list_servers().await?;
            let worker_prefix = format!("{}-worker", self.config.cluster_name);
            let servers: Vec<_> = all_servers
                .into_iter()
                .filter(|s| s.name.starts_with(&worker_prefix))
                .collect();

            for server in &servers {
                if let Some(private_ip) = &server.private_net.first().map(|net| &net.ip) {
                    info!("Upgrading worker node: {} ({})", server.name, private_ip);

                    talos_client
                        .upgrade(
                            private_ip,
                            &image,
                            options.preserve,
                            options.wait,
                            options.stage,
                        )
                        .await?;

                    info!("[OK] Upgraded {}", server.name);
                }
            }

            info!("[OK] Worker nodes upgraded successfully");
        }

        info!("[OK] Cluster upgrade completed successfully!");

        Ok(())
    }

    /// Create cluster (CLI entry point)
    pub async fn create(config_path: &std::path::Path, output_dir: &std::path::Path) -> Result<()> {
        use crate::config::ClusterConfig;
        let config =
            ClusterConfig::from_file(config_path).context("Failed to load configuration")?;
        let cluster = Self::new(config, output_dir.to_path_buf());
        cluster.create_cluster().await
    }

    /// Destroy cluster (CLI entry point)
    pub async fn destroy(
        config_path: &std::path::Path,
        output_dir: &std::path::Path,
    ) -> Result<()> {
        use crate::config::ClusterConfig;
        let config =
            ClusterConfig::from_file(config_path).context("Failed to load configuration")?;
        let cluster = Self::new(config, output_dir.to_path_buf());
        cluster.destroy_cluster().await
    }

    /// Show cluster status (CLI entry point)
    pub async fn status(config_path: &std::path::Path, output_dir: &std::path::Path) -> Result<()> {
        use crate::config::ClusterConfig;
        let config =
            ClusterConfig::from_file(config_path).context("Failed to load configuration")?;
        let cluster = Self::new(config, output_dir.to_path_buf());
        cluster.show_status().await
    }

    /// Scale cluster (CLI entry point)
    pub async fn scale(
        config_path: &std::path::Path,
        output_dir: &std::path::Path,
        node_type: crate::hcloud::server::NodeRole,
        pool_name: Option<&str>,
        target_count: u32,
        force: bool,
        timeout: u64,
    ) -> Result<()> {
        use crate::config::ClusterConfig;
        let config =
            ClusterConfig::from_file(config_path).context("Failed to load configuration")?;
        let cluster = Self::new(config, output_dir.to_path_buf());
        cluster
            .scale_cluster(node_type, pool_name, target_count, force, timeout)
            .await
    }

    /// Upgrade cluster (CLI entry point)
    pub async fn upgrade(params: UpgradeParams) -> Result<()> {
        use crate::config::ClusterConfig;

        let UpgradeParams {
            config_path,
            output_dir,
            version,
            preserve,
            control_plane,
            workers,
            wait,
            stage,
        } = params;

        if !control_plane && !workers {
            anyhow::bail!("At least one of --control-plane or --workers must be specified");
        }

        let config =
            ClusterConfig::from_file(&config_path).context("Failed to load configuration")?;
        let cluster = Self::new(config, output_dir);

        let options = UpgradeOptions {
            version: &version,
            preserve,
            control_plane,
            workers,
            wait,
            stage,
        };

        cluster.upgrade_cluster(&options).await
    }
}

/// Options for cluster upgrade
pub(crate) struct UpgradeOptions<'a> {
    version: &'a str,
    preserve: bool,
    control_plane: bool,
    workers: bool,
    wait: bool,
    stage: bool,
}

/// Parameters for Cluster::upgrade CLI entry point
pub struct UpgradeParams {
    pub config_path: std::path::PathBuf,
    pub output_dir: std::path::PathBuf,
    pub version: String,
    pub preserve: bool,
    pub control_plane: bool,
    pub workers: bool,
    pub wait: bool,
    pub stage: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::NodeConfig;
    use crate::hcloud::server::NodeRole;

    #[test]
    fn test_scale_up_params_construction() {
        use crate::hcloud::HetznerCloudClient;

        let token = "test-token".to_string();
        let client = HetznerCloudClient::new(token).unwrap();

        let pool_config = NodeConfig {
            name: "worker-pool".to_string(),
            server_type: "cx21".to_string(),
            count: 3,
            labels: std::collections::HashMap::new(),
        };

        let params = ScaleUpParams {
            hcloud_client: &client,
            pool_name: "worker-pool",
            pool_config: &pool_config,
            role: NodeRole::Worker,
            nodes_to_add: 2,
            current_count: 3,
        };

        assert_eq!(params.pool_name, "worker-pool");
        assert_eq!(params.nodes_to_add, 2);
        assert_eq!(params.current_count, 3);
        assert_eq!(params.role, NodeRole::Worker);
    }

    #[test]
    fn test_upgrade_params_construction() {
        let params = UpgradeParams {
            config_path: std::path::PathBuf::from("/path/to/config.yaml"),
            output_dir: std::path::PathBuf::from("/tmp/output"),
            version: "v1.8.0".to_string(),
            preserve: true,
            control_plane: true,
            workers: false,
            wait: true,
            stage: false,
        };

        assert_eq!(params.version, "v1.8.0");
        assert!(params.preserve);
        assert!(params.control_plane);
        assert!(!params.workers);
        assert!(params.wait);
        assert!(!params.stage);
    }

    #[test]
    fn test_upgrade_options_construction() {
        let options = UpgradeOptions {
            version: "v1.8.0",
            preserve: true,
            control_plane: true,
            workers: false,
            wait: true,
            stage: false,
        };

        assert_eq!(options.version, "v1.8.0");
        assert!(options.preserve);
        assert!(options.control_plane);
        assert!(!options.workers);
    }
}
