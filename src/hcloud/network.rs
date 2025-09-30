/// Network management for Hetzner Cloud
use anyhow::{Context, Result};
use tracing::info;

use super::client::{CreateNetworkRequest, HetznerCloudClient, SubnetRequest};
use super::models::Network;
use crate::config::NetworkConfig;

/// Network manager for handling Hetzner Cloud networks
pub struct NetworkManager {
    client: HetznerCloudClient,
}

impl NetworkManager {
    /// Create a new network manager
    pub fn new(client: HetznerCloudClient) -> Self {
        Self { client }
    }

    /// Create or get existing network for the cluster
    pub async fn ensure_network(
        &self,
        cluster_name: &str,
        config: &NetworkConfig,
    ) -> Result<Network> {
        // Check if network already exists
        let networks = self.client.list_networks().await?;
        if let Some(network) = networks
            .into_iter()
            .find(|n| n.name == format!("{}-network", cluster_name))
        {
            info!(
                "Found existing network: {} (ID: {})",
                network.name, network.id
            );
            return Ok(network);
        }

        info!("Creating new private network: {}-network", cluster_name);

        let request = CreateNetworkRequest {
            name: format!("{}-network", cluster_name),
            ip_range: config.cidr.clone(),
            subnets: Some(vec![SubnetRequest {
                ip_range: config.subnet_cidr.clone(),
                network_zone: config.zone.clone(),
                subnet_type: "cloud".to_string(),
            }]),
            routes: None,
            labels: Some(
                [
                    ("cluster".to_string(), cluster_name.to_string()),
                    ("managed-by".to_string(), "oxide".to_string()),
                ]
                .into_iter()
                .collect(),
            ),
        };

        let network = self
            .client
            .create_network(request)
            .await
            .context("Failed to create network")?;

        info!(
            "Network created successfully: {} (ID: {})",
            network.name, network.id
        );

        Ok(network)
    }

    /// Delete network by name
    pub async fn delete_network(&self, cluster_name: &str) -> Result<()> {
        let networks = self.client.list_networks().await?;

        if let Some(network) = networks
            .into_iter()
            .find(|n| n.name == format!("{}-network", cluster_name))
        {
            info!("Deleting network: {} (ID: {})", network.name, network.id);
            self.client
                .delete_network(network.id)
                .await
                .context("Failed to delete network")?;
            info!("Network deleted successfully");
        } else {
            info!("Network not found, nothing to delete");
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore] // Requires API token
    async fn test_network_manager() {
        let token = std::env::var("HCLOUD_TOKEN").expect("HCLOUD_TOKEN not set");
        let client = HetznerCloudClient::new(token).unwrap();
        let _manager = NetworkManager::new(client);

        // Test would create and delete a network
        // This is ignored by default to avoid API calls
    }
}
