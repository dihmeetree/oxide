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

impl ServerManager {
    /// Create a new server manager
    pub fn new(client: HetznerCloudClient) -> Self {
        Self { client }
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
        let mut tasks = Vec::new();

        for config in configs {
            for i in 0..config.count {
                let params = CreateServerParams {
                    cluster_name,
                    config,
                    index: i,
                    role: NodeRole::ControlPlane,
                    location,
                    network_id: network.id,
                    talos_version,
                    snapshot_id,
                    ssh_key_id,
                    user_data: user_data.clone(),
                };
                tasks.push(self.create_server(params));
            }
        }

        let results = join_all(tasks).await;
        let mut servers = Vec::new();
        for result in results {
            servers.push(result?);
        }

        Ok(servers)
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
        let mut tasks = Vec::new();

        for config in configs {
            for i in 0..config.count {
                let params = CreateServerParams {
                    cluster_name,
                    config,
                    index: i,
                    role: NodeRole::Worker,
                    location,
                    network_id: network.id,
                    talos_version,
                    snapshot_id,
                    ssh_key_id,
                    user_data: user_data.clone(),
                };
                tasks.push(self.create_server(params));
            }
        }

        let results = join_all(tasks).await;
        let mut servers = Vec::new();
        for result in results {
            servers.push(result?);
        }

        Ok(servers)
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
                // Check if server belongs to this cluster
                if let Some(cluster) = server.labels.get("cluster") {
                    if cluster == cluster_name {
                        let role = server
                            .labels
                            .get("role")
                            .and_then(|r| match r.as_str() {
                                "control-plane" => Some(NodeRole::ControlPlane),
                                "worker" => Some(NodeRole::Worker),
                                _ => None,
                            })
                            .unwrap_or(NodeRole::Worker);

                        return Some(ServerInfo {
                            server,
                            role,
                            index: 0,
                        });
                    }
                }
                None
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
    #[allow(clippy::too_many_arguments)]
    pub async fn create_single_node(
        &self,
        cluster_name: &str,
        node_name: &str,
        server_type: &str,
        location: &str,
        network_id: u64,
        role: NodeRole,
        talos_version: &str,
        snapshot_id: Option<&str>,
        ssh_key_id: Option<u64>,
        user_data: Option<String>,
        labels: std::collections::HashMap<String, String>,
    ) -> Result<ServerInfo> {
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

    #[test]
    fn test_node_role_display() {
        assert_eq!(NodeRole::ControlPlane.to_string(), "control-plane");
        assert_eq!(NodeRole::Worker.to_string(), "worker");
    }
}
