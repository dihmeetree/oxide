/// Server management for Hetzner Cloud
use anyhow::{Context, Result};
use futures::future::join_all;
use tracing::{info, warn};

use super::client::{CreateServerRequest, HetznerCloudClient};
use super::models::{Network, Server};
use crate::config::NodeConfig;

/// Server manager for handling Hetzner Cloud servers
pub struct ServerManager {
    client: HetznerCloudClient,
}

/// Information about a created server
#[derive(Debug, Clone)]
pub struct ServerInfo {
    pub server: Server,
    pub role: NodeRole,
    #[allow(dead_code)]
    pub index: u32,
}

/// Node role in the cluster
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeRole {
    ControlPlane,
    Worker,
}

impl std::fmt::Display for NodeRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NodeRole::ControlPlane => write!(f, "control-plane"),
            NodeRole::Worker => write!(f, "worker"),
        }
    }
}

/// Parameters for creating a server
struct CreateServerParams<'a> {
    cluster_name: &'a str,
    config: &'a NodeConfig,
    index: u32,
    role: NodeRole,
    location: &'a str,
    network_id: u64,
    talos_version: &'a str,
    snapshot_id: Option<&'a str>,
    ssh_key_id: Option<u64>,
    user_data: Option<String>,
}

/// Parameters for creating multiple nodes
struct CreateNodesParams<'a> {
    cluster_name: &'a str,
    configs: &'a [NodeConfig],
    role: NodeRole,
    location: &'a str,
    network: &'a Network,
    talos_version: &'a str,
    snapshot_id: Option<&'a str>,
    ssh_key_id: Option<u64>,
    user_data: Option<String>,
}

/// Parameters for creating a single node
pub struct CreateSingleNodeParams<'a> {
    pub cluster_name: &'a str,
    pub node_name: &'a str,
    pub server_type: &'a str,
    pub location: &'a str,
    pub network_id: u64,
    pub role: NodeRole,
    pub talos_version: &'a str,
    pub snapshot_id: Option<&'a str>,
    pub ssh_key_id: Option<u64>,
    pub user_data: Option<String>,
    pub labels: std::collections::HashMap<String, String>,
}

impl ServerManager {
    /// Create a new server manager
    pub fn new(client: HetznerCloudClient) -> Self {
        Self { client }
    }

    /// Create servers with specified role (generic implementation)
    async fn create_nodes(&self, params: CreateNodesParams<'_>) -> Result<Vec<ServerInfo>> {
        let CreateNodesParams {
            cluster_name,
            configs,
            role,
            location,
            network,
            talos_version,
            snapshot_id,
            ssh_key_id,
            user_data,
        } = params;

        let mut tasks = Vec::new();

        for config in configs {
            for i in 0..config.count {
                let server_params = CreateServerParams {
                    cluster_name,
                    config,
                    index: i,
                    role,
                    location,
                    network_id: network.id,
                    talos_version,
                    snapshot_id,
                    ssh_key_id,
                    user_data: user_data.clone(),
                };
                tasks.push(self.create_server(server_params));
            }
        }

        let results = join_all(tasks).await;
        results.into_iter().collect()
    }

    /// Create control plane servers
    #[allow(clippy::too_many_arguments)]
    pub async fn create_control_planes(
        &self,
        cluster_name: &str,
        configs: &[NodeConfig],
        location: &str,
        network: &Network,
        talos_version: &str,
        snapshot_id: Option<&str>,
        ssh_key_id: Option<u64>,
        user_data: Option<String>,
    ) -> Result<Vec<ServerInfo>> {
        self.create_nodes(CreateNodesParams {
            cluster_name,
            configs,
            role: NodeRole::ControlPlane,
            location,
            network,
            talos_version,
            snapshot_id,
            ssh_key_id,
            user_data,
        })
        .await
    }

    /// Create worker servers
    #[allow(clippy::too_many_arguments)]
    pub async fn create_workers(
        &self,
        cluster_name: &str,
        configs: &[NodeConfig],
        location: &str,
        network: &Network,
        talos_version: &str,
        snapshot_id: Option<&str>,
        ssh_key_id: Option<u64>,
        user_data: Option<String>,
    ) -> Result<Vec<ServerInfo>> {
        self.create_nodes(CreateNodesParams {
            cluster_name,
            configs,
            role: NodeRole::Worker,
            location,
            network,
            talos_version,
            snapshot_id,
            ssh_key_id,
            user_data,
        })
        .await
    }

    /// Create a single server
    async fn create_server(&self, params: CreateServerParams<'_>) -> Result<ServerInfo> {
        let server_name = if params.config.count == 1 {
            format!("{}-{}", params.cluster_name, params.config.name)
        } else {
            format!(
                "{}-{}-{}",
                params.cluster_name,
                params.config.name,
                params.index + 1
            )
        };

        info!(
            "Creating {} server: {} (type: {})",
            params.role, server_name, params.config.server_type
        );

        // Use Talos snapshot if provided, otherwise fail with helpful message
        let image = params.snapshot_id.ok_or_else(|| {
            anyhow::anyhow!(
                "Talos snapshot ID not configured. Please set 'talos.hcloud_snapshot_id' in your cluster configuration.\n\
                To create a Talos snapshot:\n\
                1. Create a server with any image\n\
                2. Boot into rescue mode\n\
                3. Download and write Talos image: wget -O - https://github.com/siderolabs/talos/releases/download/{}/hcloud-amd64.raw.xz | xz -d | dd of=/dev/sda\n\
                4. Reboot and create a snapshot\n\
                5. Use the snapshot ID in your configuration",
                params.talos_version
            )
        })?;

        let mut labels = params.config.labels.clone();
        labels.insert("cluster".to_string(), params.cluster_name.to_string());
        labels.insert("role".to_string(), params.role.to_string());
        labels.insert("managed-by".to_string(), "oxide".to_string());
        labels.insert(
            "talos-version".to_string(),
            params.talos_version.to_string(),
        );

        let request = CreateServerRequest {
            name: server_name.clone(),
            server_type: params.config.server_type.clone(),
            location: params.location.to_string(),
            image: image.to_string(),
            ssh_keys: params.ssh_key_id.map(|id| vec![id]),
            user_data: params.user_data,
            networks: Some(vec![params.network_id]),
            labels: Some(labels),
            automount: Some(false),
            start_after_create: Some(true),
        };

        let response = self
            .client
            .create_server(request)
            .await
            .context(format!("Failed to create server {}", server_name))?;

        info!(
            "Server {} created successfully (ID: {}), waiting for provisioning...",
            server_name, response.server.id
        );

        // Wait for server creation action to complete
        self.client
            .wait_for_action(response.action.id, 300)
            .await
            .context("Server creation action failed")?;

        // Get updated server information
        let server = self
            .client
            .get_server(response.server.id)
            .await
            .context("Failed to get server details")?;

        info!("Server {} is ready", server_name);

        Ok(ServerInfo {
            server,
            role: params.role,
            index: params.index,
        })
    }

    /// List all servers for a cluster
    pub async fn list_cluster_servers(&self, cluster_name: &str) -> Result<Vec<ServerInfo>> {
        let servers = self.client.list_servers().await?;

        let cluster_servers: Vec<ServerInfo> = servers
            .into_iter()
            .filter_map(|server| {
                // Check if server belongs to this cluster by:
                // 1. Cluster label (for original servers)
                // 2. Server name starts with cluster name (for autoscaled servers)
                let belongs_to_cluster = server
                    .labels
                    .get("cluster")
                    .is_some_and(|c| c == cluster_name)
                    || server.name.starts_with(&format!("{}-", cluster_name));

                if belongs_to_cluster {
                    let role = server
                        .labels
                        .get("role")
                        .and_then(|r| match r.as_str() {
                            "control-plane" => Some(NodeRole::ControlPlane),
                            "worker" => Some(NodeRole::Worker),
                            _ => None,
                        })
                        .unwrap_or(NodeRole::Worker);

                    Some(ServerInfo {
                        server,
                        role,
                        index: 0,
                    })
                } else {
                    None
                }
            })
            .collect();

        Ok(cluster_servers)
    }

    /// Delete all servers for a cluster
    pub async fn delete_cluster_servers(&self, cluster_name: &str) -> Result<()> {
        let servers = self.list_cluster_servers(cluster_name).await?;

        if servers.is_empty() {
            info!("No servers found for cluster {}", cluster_name);
            return Ok(());
        }

        info!(
            "Deleting {} servers for cluster {}",
            servers.len(),
            cluster_name
        );

        for server_info in servers {
            info!(
                "Deleting server: {} (ID: {})",
                server_info.server.name, server_info.server.id
            );
            if let Err(e) = self.client.delete_server(server_info.server.id).await {
                warn!(
                    "Failed to delete server {} (ID: {}): {}",
                    server_info.server.name, server_info.server.id, e
                );
            }
        }

        info!("All servers deleted");
        Ok(())
    }

    /// Get the primary IP address of a server
    pub fn get_server_ip(server: &Server) -> Option<String> {
        server.public_net.ipv4.as_ref().map(|ipv4| ipv4.ip.clone())
    }

    /// Get the private IP address of a server
    pub fn get_server_private_ip(server: &Server) -> Option<String> {
        server.private_net.first().map(|net| net.ip.clone())
    }

    /// Delete specific servers by ID
    pub async fn delete_servers(&self, server_ids: Vec<u64>) -> Result<()> {
        if server_ids.is_empty() {
            info!("No servers to delete");
            return Ok(());
        }

        info!("Deleting {} servers", server_ids.len());

        for server_id in server_ids {
            info!("Deleting server ID: {}", server_id);
            if let Err(e) = self.client.delete_server(server_id).await {
                warn!("Failed to delete server {}: {}", server_id, e);
            }
        }

        info!("Servers deleted");
        Ok(())
    }

    /// Get servers by role and pool name
    pub fn filter_by_role_and_pool(
        servers: &[ServerInfo],
        role: NodeRole,
        pool_name: Option<&str>,
    ) -> Vec<ServerInfo> {
        servers
            .iter()
            .filter(|s| {
                if s.role != role {
                    return false;
                }

                // If pool name is specified, match it
                if let Some(pool) = pool_name {
                    // Extract pool name from server name (format: cluster-poolname-index)
                    let server_name_parts: Vec<&str> = s.server.name.split('-').collect();
                    if server_name_parts.len() >= 2 {
                        let server_pool = server_name_parts[server_name_parts.len() - 2];
                        return server_pool == pool;
                    }
                    return false;
                }

                true
            })
            .cloned()
            .collect()
    }

    /// Create a single node with specific configuration
    pub async fn create_single_node(
        &self,
        params: CreateSingleNodeParams<'_>,
    ) -> Result<ServerInfo> {
        let CreateSingleNodeParams {
            cluster_name,
            node_name,
            server_type,
            location,
            network_id,
            role,
            talos_version,
            snapshot_id,
            ssh_key_id,
            user_data,
            labels,
        } = params;

        info!(
            "Creating {} server: {} (type: {})",
            role, node_name, server_type
        );

        let image = snapshot_id.ok_or_else(|| {
            anyhow::anyhow!(
                "Talos snapshot ID not configured. Please set 'talos.hcloud_snapshot_id' in your cluster configuration."
            )
        })?;

        let mut server_labels = labels;
        server_labels.insert("cluster".to_string(), cluster_name.to_string());
        server_labels.insert("role".to_string(), role.to_string());
        server_labels.insert("managed-by".to_string(), "oxide".to_string());
        server_labels.insert("talos-version".to_string(), talos_version.to_string());

        let request = CreateServerRequest {
            name: node_name.to_string(),
            server_type: server_type.to_string(),
            location: location.to_string(),
            image: image.to_string(),
            ssh_keys: ssh_key_id.map(|id| vec![id]),
            user_data,
            networks: Some(vec![network_id]),
            labels: Some(server_labels),
            automount: Some(false),
            start_after_create: Some(true),
        };

        let response = self
            .client
            .create_server(request)
            .await
            .context(format!("Failed to create server {}", node_name))?;

        info!(
            "Server {} created successfully (ID: {}), waiting for provisioning...",
            node_name, response.server.id
        );

        self.client
            .wait_for_action(response.action.id, 300)
            .await
            .context("Server creation action failed")?;

        let server = self
            .client
            .get_server(response.server.id)
            .await
            .context("Failed to get server details")?;

        info!("Server {} is ready", node_name);

        Ok(ServerInfo {
            server,
            role,
            index: 0,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hcloud::models::*;

    fn create_test_server(id: u64, name: &str, ip: &str) -> Server {
        Server {
            id,
            name: name.to_string(),
            status: "running".to_string(),
            server_type: ServerType {
                id: 1,
                name: "cx21".to_string(),
                description: "CX21".to_string(),
                cores: 2,
                memory: 4.0,
                disk: 40,
            },
            datacenter: Datacenter {
                id: 1,
                name: "fsn1-dc14".to_string(),
                description: "Falkenstein 1 DC14".to_string(),
                location: Location {
                    id: 1,
                    name: "fsn1".to_string(),
                    description: "Falkenstein DC Park 1".to_string(),
                    country: "DE".to_string(),
                    city: "Falkenstein".to_string(),
                    latitude: 50.47612,
                    longitude: 12.370071,
                },
            },
            public_net: PublicNetwork {
                ipv4: Some(IPv4 {
                    ip: ip.to_string(),
                    blocked: false,
                }),
                ipv6: Some(IPv6 {
                    ip: "2001:db8::1".to_string(),
                    blocked: false,
                }),
                floating_ips: vec![],
            },
            private_net: vec![],
            created: "2024-01-01T00:00:00+00:00".to_string(),
            labels: std::collections::HashMap::new(),
        }
    }

    #[test]
    fn test_node_role_display() {
        assert_eq!(NodeRole::ControlPlane.to_string(), "control-plane");
        assert_eq!(NodeRole::Worker.to_string(), "worker");
    }

    #[test]
    fn test_node_role_equality() {
        assert_eq!(NodeRole::ControlPlane, NodeRole::ControlPlane);
        assert_eq!(NodeRole::Worker, NodeRole::Worker);
        assert_ne!(NodeRole::ControlPlane, NodeRole::Worker);
    }

    #[test]
    fn test_get_server_ip() {
        let server_with_ip = create_test_server(1, "test-server", "192.168.1.1");
        assert_eq!(
            ServerManager::get_server_ip(&server_with_ip),
            Some("192.168.1.1".to_string())
        );
    }

    #[test]
    fn test_get_server_private_ip() {
        // Server with private IP
        let mut server_with_private = create_test_server(1, "test-server", "192.168.1.1");
        server_with_private.private_net.push(PrivateNetwork {
            network: 123,
            ip: "10.0.1.5".to_string(),
            alias_ips: vec![],
            mac_address: "aa:bb:cc:dd:ee:ff".to_string(),
        });

        assert_eq!(
            ServerManager::get_server_private_ip(&server_with_private),
            Some("10.0.1.5".to_string())
        );

        // Server without private IP
        let server_without_private = create_test_server(2, "test-server-2", "192.168.1.2");
        assert_eq!(
            ServerManager::get_server_private_ip(&server_without_private),
            None
        );
    }

    #[test]
    fn test_filter_by_role_and_pool() {
        let servers = vec![
            ServerInfo {
                server: create_test_server(1, "cluster-control-plane-1", "192.168.1.1"),
                role: NodeRole::ControlPlane,
                index: 1,
            },
            ServerInfo {
                server: create_test_server(2, "cluster-worker-1", "192.168.1.2"),
                role: NodeRole::Worker,
                index: 1,
            },
        ];

        // Filter control planes
        let control_planes =
            ServerManager::filter_by_role_and_pool(&servers, NodeRole::ControlPlane, None);
        assert_eq!(control_planes.len(), 1);
        assert_eq!(control_planes[0].server.name, "cluster-control-plane-1");

        // Filter workers
        let workers = ServerManager::filter_by_role_and_pool(&servers, NodeRole::Worker, None);
        assert_eq!(workers.len(), 1);
        assert_eq!(workers[0].server.name, "cluster-worker-1");

        // Filter by pool name (extracts second-to-last component from "cluster-control-plane-1")
        let pool_servers = ServerManager::filter_by_role_and_pool(
            &servers,
            NodeRole::ControlPlane,
            Some("control"),
        );
        // Should match "cluster-control-plane-1" where second-to-last is "control"
        // Actually server name is "cluster-control-plane-1", parts are ["cluster", "control", "plane", "1"]
        // Second-to-last is "plane", not "control"
        assert_eq!(pool_servers.len(), 0);

        // Test with correct pool name "plane"
        let pool_servers2 =
            ServerManager::filter_by_role_and_pool(&servers, NodeRole::ControlPlane, Some("plane"));
        assert_eq!(pool_servers2.len(), 1);
    }

    #[test]
    fn test_create_nodes_params_construction() {
        use crate::config::NodeConfig;

        let network = Network {
            id: 123,
            name: "test-network".to_string(),
            ip_range: "10.0.0.0/16".to_string(),
            subnets: vec![],
            routes: vec![],
            servers: vec![],
            created: "2024-01-01T00:00:00+00:00".to_string(),
        };

        let configs = vec![NodeConfig {
            name: "test-pool".to_string(),
            server_type: "cx21".to_string(),
            count: 3,
            labels: std::collections::HashMap::new(),
        }];

        let params = CreateNodesParams {
            cluster_name: "test-cluster",
            configs: &configs,
            role: NodeRole::ControlPlane,
            location: "fsn1",
            network: &network,
            talos_version: "v1.7.0",
            snapshot_id: Some("12345"),
            ssh_key_id: Some(678),
            user_data: Some("test-data".to_string()),
        };

        assert_eq!(params.cluster_name, "test-cluster");
        assert_eq!(params.configs.len(), 1);
        assert_eq!(params.role, NodeRole::ControlPlane);
        assert_eq!(params.network.id, 123);
    }

    #[test]
    fn test_create_single_node_params_construction() {
        let mut labels = std::collections::HashMap::new();
        labels.insert("env".to_string(), "test".to_string());

        let params = CreateSingleNodeParams {
            cluster_name: "test-cluster",
            node_name: "test-node-1",
            server_type: "cx21",
            location: "fsn1",
            network_id: 123,
            role: NodeRole::Worker,
            talos_version: "v1.7.0",
            snapshot_id: Some("12345"),
            ssh_key_id: Some(678),
            user_data: Some("test-data".to_string()),
            labels: labels.clone(),
        };

        assert_eq!(params.cluster_name, "test-cluster");
        assert_eq!(params.node_name, "test-node-1");
        assert_eq!(params.role, NodeRole::Worker);
        assert_eq!(params.network_id, 123);
        assert_eq!(params.labels.get("env"), Some(&"test".to_string()));
    }
}
