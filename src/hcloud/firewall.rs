/// Firewall management for Hetzner Cloud
use anyhow::{Context, Result};
use tracing::info;

use super::client::HetznerCloudClient;
use super::models::{Firewall, FirewallRule};

/// Parse and validate an allowed IP/CIDR before passing it to the Hetzner API.
///
/// Accepts either a bare IPv4/IPv6 address (which is normalised to `/32` or
/// `/128` respectively) or an explicit CIDR. Anything else is rejected so
/// malformed input cannot produce an unusable or overbroad firewall rule.
fn parse_allowed_ip_cidr(allowed_ip: &str) -> Result<String> {
    use std::net::IpAddr;

    let trimmed = allowed_ip.trim();
    if trimmed.is_empty() {
        anyhow::bail!("allowed IP must not be empty");
    }

    if let Some((addr_str, prefix_str)) = trimmed.split_once('/') {
        let addr: IpAddr = addr_str
            .parse()
            .with_context(|| format!("Invalid IP address in CIDR: {}", trimmed))?;
        let prefix: u8 = prefix_str
            .parse()
            .with_context(|| format!("Invalid prefix length in CIDR: {}", trimmed))?;
        let max_prefix = match addr {
            IpAddr::V4(_) => 32,
            IpAddr::V6(_) => 128,
        };
        if prefix > max_prefix {
            anyhow::bail!(
                "CIDR prefix /{} exceeds maximum /{} for {}",
                prefix,
                max_prefix,
                addr
            );
        }
        Ok(format!("{}/{}", addr, prefix))
    } else {
        let addr: IpAddr = trimmed
            .parse()
            .with_context(|| format!("Invalid IP address: {:?}", allowed_ip))?;
        let prefix = match addr {
            IpAddr::V4(_) => 32,
            IpAddr::V6(_) => 128,
        };
        Ok(format!("{}/{}", addr, prefix))
    }
}

/// Firewall manager
pub struct FirewallManager {
    client: HetznerCloudClient,
}

impl FirewallManager {
    /// Create a new firewall manager
    pub fn new(client: HetznerCloudClient) -> Self {
        Self { client }
    }

    /// Get current public IP address
    pub async fn get_current_ip() -> Result<String> {
        use std::net::IpAddr;

        let client = reqwest::Client::new();
        let response = client
            .get("https://ipv4.icanhazip.com")
            .send()
            .await
            .context("Failed to get current IP address")?;

        let body = response
            .text()
            .await
            .context("Failed to read IP address response")?;

        let trimmed = body.trim();

        // Validate the response is actually an IP address before returning. The
        // upstream service has, on occasion, returned HTML/error bodies; if we
        // forwarded that string into firewall rules we would either fail or, worse,
        // produce a malformed/overbroad rule.
        let ip: IpAddr = trimmed.parse().with_context(|| {
            format!(
                "External IP service returned an invalid IP address: {:?}",
                trimmed
            )
        })?;

        Ok(ip.to_string())
    }

    /// Create firewall with Talos/Cilium ports
    pub async fn create_cluster_firewall(
        &self,
        cluster_name: &str,
        allowed_ip: &str,
    ) -> Result<Firewall> {
        info!(
            "Creating firewall for cluster with allowed IP: {}",
            allowed_ip
        );

        let firewall_name = format!("{}-firewall", cluster_name);

        // Check if firewall already exists
        let firewalls = self.list_firewalls().await?;
        if let Some(firewall) = firewalls.into_iter().find(|f| f.name == firewall_name) {
            info!(
                "Found existing firewall: {} (ID: {})",
                firewall.name, firewall.id
            );
            return Ok(firewall);
        }

        let allowed_ip_cidr = parse_allowed_ip_cidr(allowed_ip)?;

        // Define firewall rules for external access only
        // Note: Internal cluster communication (10.0.0.0/16) is not affected by Hetzner Cloud firewalls
        let rules = vec![
            // Talos API (apid) - port 50000
            FirewallRule {
                direction: "in".to_string(),
                source_ips: vec![allowed_ip_cidr.clone()],
                destination_ips: vec![],
                protocol: "tcp".to_string(),
                port: Some("50000".to_string()),
            },
            // Kubernetes API - port 6443
            FirewallRule {
                direction: "in".to_string(),
                source_ips: vec![allowed_ip_cidr.clone()],
                destination_ips: vec![],
                protocol: "tcp".to_string(),
                port: Some("6443".to_string()),
            },
            // HTTP - port 80
            FirewallRule {
                direction: "in".to_string(),
                source_ips: vec!["0.0.0.0/0".to_string(), "::/0".to_string()],
                destination_ips: vec![],
                protocol: "tcp".to_string(),
                port: Some("80".to_string()),
            },
            // HTTPS - port 443
            FirewallRule {
                direction: "in".to_string(),
                source_ips: vec!["0.0.0.0/0".to_string(), "::/0".to_string()],
                destination_ips: vec![],
                protocol: "tcp".to_string(),
                port: Some("443".to_string()),
            },
        ];

        #[derive(serde::Serialize)]
        struct CreateFirewallRequest {
            name: String,
            rules: Vec<FirewallRule>,
            labels: std::collections::HashMap<String, String>,
        }

        let request = CreateFirewallRequest {
            name: firewall_name,
            rules,
            labels: [
                ("cluster".to_string(), cluster_name.to_string()),
                ("managed-by".to_string(), "oxide".to_string()),
            ]
            .into_iter()
            .collect(),
        };

        let firewall = self
            .create_firewall(request)
            .await
            .context("Failed to create firewall")?;

        info!(
            "Firewall created successfully: {} (ID: {})",
            firewall.name, firewall.id
        );

        Ok(firewall)
    }

    /// Apply firewall to servers
    pub async fn apply_to_servers(&self, firewall_id: u64, server_ids: Vec<u64>) -> Result<()> {
        info!("Applying firewall to {} servers", server_ids.len());

        #[derive(serde::Serialize)]
        struct ApplyToResourcesRequest {
            apply_to: Vec<ApplyToResource>,
        }

        #[derive(serde::Serialize)]
        struct ApplyToResource {
            #[serde(rename = "type")]
            resource_type: String,
            server: ServerReference,
        }

        #[derive(serde::Serialize)]
        struct ServerReference {
            id: u64,
        }

        let request = ApplyToResourcesRequest {
            apply_to: server_ids
                .into_iter()
                .map(|id| ApplyToResource {
                    resource_type: "server".to_string(),
                    server: ServerReference { id },
                })
                .collect(),
        };

        let _: serde_json::Value = self
            .client
            .post(
                &format!("firewalls/{}/actions/apply_to_resources", firewall_id),
                &request,
            )
            .await
            .context("Failed to apply firewall to servers")?;

        info!("Firewall applied successfully");

        Ok(())
    }

    /// List all firewalls
    async fn list_firewalls(&self) -> Result<Vec<Firewall>> {
        use super::models::FirewallListResponse;
        let response: FirewallListResponse = self.client.get("firewalls").await?;
        Ok(response.firewalls)
    }

    /// Create firewall
    async fn create_firewall<T: serde::Serialize>(&self, request: T) -> Result<Firewall> {
        use super::models::CreateFirewallResponse;
        let response: CreateFirewallResponse = self.client.post("firewalls", &request).await?;
        Ok(response.firewall)
    }

    /// Get firewall for cluster
    pub async fn get_cluster_firewall(&self, cluster_name: &str) -> Result<Option<Firewall>> {
        let firewalls = self.list_firewalls().await?;

        Ok(firewalls
            .into_iter()
            .find(|f| f.name == format!("{}-firewall", cluster_name)))
    }

    /// Delete firewall
    pub async fn delete_cluster_firewall(&self, cluster_name: &str) -> Result<()> {
        use tokio::time::{sleep, Duration};

        let firewalls = self.list_firewalls().await?;

        if let Some(firewall) = firewalls
            .into_iter()
            .find(|f| f.name == format!("{}-firewall", cluster_name))
        {
            info!("Deleting firewall: {} (ID: {})", firewall.name, firewall.id);

            // Retry deletion if firewall is still in use (servers may still be deleting)
            for attempt in 1..=12 {
                match self
                    .client
                    .delete(&format!("firewalls/{}", firewall.id))
                    .await
                {
                    Ok(_) => {
                        info!("Firewall deleted successfully");
                        return Ok(());
                    }
                    Err(e) => {
                        let err_msg = e.to_string();
                        if err_msg.contains("resource_in_use") || err_msg.contains("still in use") {
                            if attempt < 12 {
                                info!(
                                    "Firewall still in use, waiting for servers to be deleted (attempt {}/12)...",
                                    attempt
                                );
                                sleep(Duration::from_secs(5)).await;
                            } else {
                                return Err(e).context(
                                    "Failed to delete firewall after waiting for servers",
                                );
                            }
                        } else {
                            return Err(e).context("Failed to delete firewall");
                        }
                    }
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_get_current_ip() {
        let result = FirewallManager::get_current_ip().await;
        // Should get an IP address
        if let Ok(ip) = result {
            assert!(!ip.is_empty());
            println!("Current IP: {}", ip);
        }
    }

    #[tokio::test]
    #[ignore] // Requires API token
    async fn test_firewall_manager() {
        let token = std::env::var("HCLOUD_TOKEN").expect("HCLOUD_TOKEN not set");
        let client = HetznerCloudClient::new(token).unwrap();
        let _manager = FirewallManager::new(client);

        // Test would create and delete a firewall
        // This is ignored by default to avoid API calls
    }
}
