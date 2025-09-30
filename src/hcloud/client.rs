/// Hetzner Cloud API client
use anyhow::{Context, Result};
use reqwest::{header, Client};
use serde::de::DeserializeOwned;
use serde::Serialize;
use tracing::{debug, warn};

use super::models::*;

const HCLOUD_API_BASE: &str = "https://api.hetzner.cloud/v1";

/// Main Hetzner Cloud API client
#[derive(Clone)]
pub struct HetznerCloudClient {
    client: Client,
    #[allow(dead_code)]
    api_token: String,
}

impl HetznerCloudClient {
    /// Create a new Hetzner Cloud API client
    pub fn new(api_token: String) -> Result<Self> {
        let mut headers = header::HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            header::HeaderValue::from_str(&format!("Bearer {}", api_token))
                .context("Invalid API token format")?,
        );
        headers.insert(
            header::CONTENT_TYPE,
            header::HeaderValue::from_static("application/json"),
        );

        let client = Client::builder()
            .default_headers(headers)
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .context("Failed to create HTTP client")?;

        Ok(Self { client, api_token })
    }

    /// Make a GET request to the API
    pub(crate) async fn get<T: DeserializeOwned>(&self, endpoint: &str) -> Result<T> {
        let url = format!("{}/{}", HCLOUD_API_BASE, endpoint);
        debug!("GET {}", url);

        let response = self
            .client
            .get(&url)
            .send()
            .await
            .context("Failed to send GET request")?;

        self.handle_response(response).await
    }

    /// Make a POST request to the API
    pub(crate) async fn post<T: Serialize, R: DeserializeOwned>(
        &self,
        endpoint: &str,
        body: &T,
    ) -> Result<R> {
        let url = format!("{}/{}", HCLOUD_API_BASE, endpoint);
        debug!("POST {}", url);

        let response = self
            .client
            .post(&url)
            .json(body)
            .send()
            .await
            .context("Failed to send POST request")?;

        self.handle_response(response).await
    }

    /// Make a DELETE request to the API
    pub(crate) async fn delete(&self, endpoint: &str) -> Result<()> {
        let url = format!("{}/{}", HCLOUD_API_BASE, endpoint);
        debug!("DELETE {}", url);

        let response = self
            .client
            .delete(&url)
            .send()
            .await
            .context("Failed to send DELETE request")?;

        if response.status().is_success() {
            Ok(())
        } else {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            anyhow::bail!("API request failed with status {}: {}", status, error_text)
        }
    }

    /// Handle API response, checking for errors
    async fn handle_response<T: DeserializeOwned>(&self, response: reqwest::Response) -> Result<T> {
        let status = response.status();

        if status.is_success() {
            response
                .json::<T>()
                .await
                .context("Failed to parse API response")
        } else {
            let error_text = response.text().await.unwrap_or_default();

            // Try to parse as error response
            if let Ok(error_response) = serde_json::from_str::<ErrorResponse>(&error_text) {
                anyhow::bail!(
                    "API error: {} - {}",
                    error_response.error.code,
                    error_response.error.message
                );
            }

            anyhow::bail!("API request failed with status {}: {}", status, error_text)
        }
    }

    /// List all servers
    pub async fn list_servers(&self) -> Result<Vec<Server>> {
        let response: ServerListResponse = self.get("servers").await?;
        Ok(response.servers)
    }

    /// Get server by ID
    pub async fn get_server(&self, server_id: u64) -> Result<Server> {
        #[derive(serde::Deserialize)]
        struct Response {
            server: Server,
        }
        let response: Response = self.get(&format!("servers/{}", server_id)).await?;
        Ok(response.server)
    }

    /// Create a new server
    pub async fn create_server(
        &self,
        request: CreateServerRequest,
    ) -> Result<CreateServerResponse> {
        self.post("servers", &request).await
    }

    /// Delete a server
    pub async fn delete_server(&self, server_id: u64) -> Result<()> {
        self.delete(&format!("servers/{}", server_id)).await
    }

    /// Power on a server
    #[allow(dead_code)]
    pub async fn power_on_server(&self, server_id: u64) -> Result<Action> {
        let response: ActionResponse = self
            .post(
                &format!("servers/{}/actions/poweron", server_id),
                &serde_json::json!({}),
            )
            .await?;
        Ok(response.action)
    }

    /// Wait for an action to complete
    pub async fn wait_for_action(&self, action_id: u64, timeout_secs: u64) -> Result<Action> {
        use tokio::time::{sleep, Duration};

        let start = std::time::Instant::now();
        let timeout = Duration::from_secs(timeout_secs);

        loop {
            let action = self.get_action(action_id).await?;

            match action.status.as_str() {
                "success" => return Ok(action),
                "error" => {
                    let error_msg = action
                        .error
                        .map(|e| format!("{}: {}", e.code, e.message))
                        .unwrap_or_else(|| "Unknown error".to_string());
                    anyhow::bail!("Action {} failed: {}", action_id, error_msg);
                }
                "running" => {
                    if start.elapsed() > timeout {
                        anyhow::bail!(
                            "Action {} timed out after {} seconds",
                            action_id,
                            timeout_secs
                        );
                    }
                    debug!("Action {} progress: {}%", action_id, action.progress);
                    sleep(Duration::from_secs(2)).await;
                }
                status => {
                    warn!("Unknown action status: {}", status);
                    sleep(Duration::from_secs(2)).await;
                }
            }
        }
    }

    /// Get action status
    pub async fn get_action(&self, action_id: u64) -> Result<Action> {
        let response: ActionResponse = self.get(&format!("actions/{}", action_id)).await?;
        Ok(response.action)
    }

    /// List all networks
    pub async fn list_networks(&self) -> Result<Vec<Network>> {
        let response: NetworkListResponse = self.get("networks").await?;
        Ok(response.networks)
    }

    /// Get network by ID
    #[allow(dead_code)]
    pub async fn get_network(&self, network_id: u64) -> Result<Network> {
        #[derive(serde::Deserialize)]
        struct Response {
            network: Network,
        }
        let response: Response = self.get(&format!("networks/{}", network_id)).await?;
        Ok(response.network)
    }

    /// Create a new network
    pub async fn create_network(&self, request: CreateNetworkRequest) -> Result<Network> {
        let response: CreateNetworkResponse = self.post("networks", &request).await?;
        Ok(response.network)
    }

    /// Delete a network
    pub async fn delete_network(&self, network_id: u64) -> Result<()> {
        self.delete(&format!("networks/{}", network_id)).await
    }

    /// Attach server to network
    #[allow(dead_code)]
    pub async fn attach_to_network(
        &self,
        server_id: u64,
        network_id: u64,
        ip: Option<String>,
    ) -> Result<Action> {
        #[derive(serde::Serialize)]
        struct Request {
            network: u64,
            #[serde(skip_serializing_if = "Option::is_none")]
            ip: Option<String>,
        }

        let request = Request {
            network: network_id,
            ip,
        };

        let response: ActionResponse = self
            .post(
                &format!("servers/{}/actions/attach_to_network", server_id),
                &request,
            )
            .await?;
        Ok(response.action)
    }

    /// List SSH keys
    #[allow(dead_code)]
    pub async fn list_ssh_keys(&self) -> Result<Vec<SSHKey>> {
        let response: SSHKeyListResponse = self.get("ssh_keys").await?;
        Ok(response.ssh_keys)
    }

    /// Create SSH key
    #[allow(dead_code)]
    pub async fn create_ssh_key(&self, name: String, public_key: String) -> Result<SSHKey> {
        #[derive(serde::Serialize)]
        struct Request {
            name: String,
            public_key: String,
        }

        let response: CreateSSHKeyResponse =
            self.post("ssh_keys", &Request { name, public_key }).await?;
        Ok(response.ssh_key)
    }

    /// Delete SSH key
    #[allow(dead_code)]
    pub async fn delete_ssh_key(&self, key_id: u64) -> Result<()> {
        self.delete(&format!("ssh_keys/{}", key_id)).await
    }
}

/// Request structure for creating a server
#[derive(Debug, Serialize)]
pub struct CreateServerRequest {
    pub name: String,
    pub server_type: String,
    pub location: String,
    pub image: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ssh_keys: Option<Vec<u64>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_data: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub networks: Option<Vec<u64>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub labels: Option<std::collections::HashMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub automount: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_after_create: Option<bool>,
}

/// Request structure for creating a network
#[derive(Debug, Serialize)]
pub struct CreateNetworkRequest {
    pub name: String,
    pub ip_range: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subnets: Option<Vec<SubnetRequest>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub routes: Option<Vec<RouteRequest>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub labels: Option<std::collections::HashMap<String, String>>,
}

/// Request structure for creating a subnet
#[derive(Debug, Serialize)]
pub struct SubnetRequest {
    pub ip_range: String,
    pub network_zone: String,
    #[serde(rename = "type")]
    pub subnet_type: String,
}

/// Request structure for creating a route
#[derive(Debug, Serialize)]
pub struct RouteRequest {
    pub destination: String,
    pub gateway: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_client_creation() {
        let result = HetznerCloudClient::new("test-token".to_string());
        assert!(result.is_ok());
    }
}
