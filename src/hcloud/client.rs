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
    base_url: String,
    #[allow(dead_code)]
    api_token: String,
}

impl HetznerCloudClient {
    /// Create a new Hetzner Cloud API client
    pub fn new(api_token: String) -> Result<Self> {
        Self::with_base_url(api_token, HCLOUD_API_BASE.to_string())
    }

    /// Create a new client targeting a custom base URL.
    ///
    /// Primarily intended for tests that point the client at a mock server.
    pub fn with_base_url(api_token: String, base_url: String) -> Result<Self> {
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

        Ok(Self {
            client,
            base_url: base_url.trim_end_matches('/').to_string(),
            api_token,
        })
    }

    /// Make a GET request to the API
    pub(crate) async fn get<T: DeserializeOwned>(&self, endpoint: &str) -> Result<T> {
        let url = format!("{}/{}", self.base_url, endpoint);
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
        let url = format!("{}/{}", self.base_url, endpoint);
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
        let url = format!("{}/{}", self.base_url, endpoint);
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
    use wiremock::matchers::{body_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn sample_server_json() -> serde_json::Value {
        serde_json::json!({
            "id": 42,
            "name": "node-1",
            "status": "running",
            "server_type": {
                "id": 1,
                "name": "cx22",
                "description": "CX22",
                "cores": 2,
                "memory": 4.0,
                "disk": 40
            },
            "datacenter": {
                "id": 1,
                "name": "fsn1-dc14",
                "description": "Falkenstein DC 14",
                "location": {
                    "id": 1,
                    "name": "fsn1",
                    "description": "Falkenstein",
                    "country": "DE",
                    "city": "Falkenstein",
                    "latitude": 50.0,
                    "longitude": 12.0
                }
            },
            "public_net": {
                "ipv4": {"ip": "1.2.3.4", "blocked": false},
                "ipv6": null,
                "floating_ips": []
            },
            "private_net": [],
            "created": "2024-01-01T00:00:00Z",
            "labels": {}
        })
    }

    fn sample_action_json(status: &str) -> serde_json::Value {
        serde_json::json!({
            "id": 100,
            "command": "create_server",
            "status": status,
            "progress": if status == "running" { 50 } else { 100 },
            "started": "2024-01-01T00:00:00Z",
            "finished": if status == "running" { serde_json::Value::Null } else { serde_json::Value::String("2024-01-01T00:01:00Z".into()) },
            "error": serde_json::Value::Null
        })
    }

    fn sample_network_json() -> serde_json::Value {
        serde_json::json!({
            "id": 7,
            "name": "test-net",
            "ip_range": "10.0.0.0/16",
            "subnets": [],
            "routes": [],
            "servers": [],
            "created": "2024-01-01T00:00:00Z"
        })
    }

    fn sample_ssh_key_json() -> serde_json::Value {
        serde_json::json!({
            "id": 5,
            "name": "key-1",
            "fingerprint": "aa:bb",
            "public_key": "ssh-ed25519 AAAA",
            "labels": {},
            "created": "2024-01-01T00:00:00Z"
        })
    }

    async fn client(server: &MockServer) -> HetznerCloudClient {
        HetznerCloudClient::with_base_url("test-token".into(), server.uri()).unwrap()
    }

    #[test]
    fn test_client_creation() {
        let result = HetznerCloudClient::new("test-token".to_string());
        assert!(result.is_ok());
    }

    #[test]
    fn test_with_base_url_strips_trailing_slash() {
        let c =
            HetznerCloudClient::with_base_url("t".into(), "https://api.example/".into()).unwrap();
        assert_eq!(c.base_url, "https://api.example");
    }

    #[test]
    fn test_invalid_token_rejected() {
        // Header values cannot contain newlines.
        let bad = HetznerCloudClient::new("invalid\ntoken".into());
        assert!(bad.is_err());
    }

    #[tokio::test]
    async fn test_list_servers_authenticated() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/servers"))
            .and(header("authorization", "Bearer test-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "servers": [sample_server_json()]
            })))
            .mount(&server)
            .await;

        let result = client(&server).await.list_servers().await.unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].id, 42);
        assert_eq!(result[0].public_net.ipv4.as_ref().unwrap().ip, "1.2.3.4");
    }

    #[tokio::test]
    async fn test_get_server() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/servers/42"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "server": sample_server_json()
            })))
            .mount(&server)
            .await;

        let s = client(&server).await.get_server(42).await.unwrap();
        assert_eq!(s.name, "node-1");
    }

    #[tokio::test]
    async fn test_create_server() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/servers"))
            .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
                "server": sample_server_json(),
                "action": sample_action_json("running"),
                "root_password": null
            })))
            .mount(&server)
            .await;

        let req = CreateServerRequest {
            name: "node-1".into(),
            server_type: "cx22".into(),
            location: "fsn1".into(),
            image: "ubuntu-24.04".into(),
            ssh_keys: None,
            user_data: None,
            networks: None,
            labels: None,
            automount: None,
            start_after_create: Some(true),
        };
        let resp = client(&server).await.create_server(req).await.unwrap();
        assert_eq!(resp.server.id, 42);
        assert_eq!(resp.action.status, "running");
    }

    #[tokio::test]
    async fn test_delete_server() {
        let server = MockServer::start().await;
        Mock::given(method("DELETE"))
            .and(path("/servers/42"))
            .respond_with(ResponseTemplate::new(204))
            .mount(&server)
            .await;
        client(&server).await.delete_server(42).await.unwrap();
    }

    #[tokio::test]
    async fn test_get_action_and_wait_success() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/actions/100"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "action": sample_action_json("success")
            })))
            .mount(&server)
            .await;

        let c = client(&server).await;
        assert_eq!(c.get_action(100).await.unwrap().status, "success");
        // wait_for_action returns immediately when status is success
        let action = c.wait_for_action(100, 5).await.unwrap();
        assert_eq!(action.id, 100);
    }

    #[tokio::test]
    async fn test_wait_for_action_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/actions/100"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "action": {
                    "id": 100,
                    "command": "create_server",
                    "status": "error",
                    "progress": 100,
                    "started": "2024-01-01T00:00:00Z",
                    "finished": "2024-01-01T00:01:00Z",
                    "error": {"code": "rate_limit", "message": "too many"}
                }
            })))
            .mount(&server)
            .await;

        let err = client(&server)
            .await
            .wait_for_action(100, 5)
            .await
            .unwrap_err();
        let s = format!("{err:#}");
        assert!(s.contains("rate_limit"), "{s}");
    }

    #[tokio::test]
    async fn test_list_networks() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/networks"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "networks": [sample_network_json()]
            })))
            .mount(&server)
            .await;
        let nets = client(&server).await.list_networks().await.unwrap();
        assert_eq!(nets.len(), 1);
        assert_eq!(nets[0].id, 7);
    }

    #[tokio::test]
    async fn test_get_network() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/networks/7"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "network": sample_network_json()
            })))
            .mount(&server)
            .await;
        let n = client(&server).await.get_network(7).await.unwrap();
        assert_eq!(n.name, "test-net");
    }

    #[tokio::test]
    async fn test_create_network_and_delete() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/networks"))
            .and(body_json(serde_json::json!({
                "name": "test-net",
                "ip_range": "10.0.0.0/16"
            })))
            .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
                "network": sample_network_json()
            })))
            .mount(&server)
            .await;
        Mock::given(method("DELETE"))
            .and(path("/networks/7"))
            .respond_with(ResponseTemplate::new(204))
            .mount(&server)
            .await;

        let c = client(&server).await;
        let req = CreateNetworkRequest {
            name: "test-net".into(),
            ip_range: "10.0.0.0/16".into(),
            subnets: None,
            routes: None,
            labels: None,
        };
        let n = c.create_network(req).await.unwrap();
        assert_eq!(n.id, 7);
        c.delete_network(7).await.unwrap();
    }

    #[tokio::test]
    async fn test_attach_to_network() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/servers/42/actions/attach_to_network"))
            .and(body_json(
                serde_json::json!({"network": 7, "ip": "10.0.0.5"}),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "action": sample_action_json("running")
            })))
            .mount(&server)
            .await;
        let action = client(&server)
            .await
            .attach_to_network(42, 7, Some("10.0.0.5".into()))
            .await
            .unwrap();
        assert_eq!(action.command, "create_server");
    }

    #[tokio::test]
    async fn test_power_on_server() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/servers/42/actions/poweron"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "action": sample_action_json("running")
            })))
            .mount(&server)
            .await;
        client(&server).await.power_on_server(42).await.unwrap();
    }

    #[tokio::test]
    async fn test_ssh_keys_crud() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/ssh_keys"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "ssh_keys": [sample_ssh_key_json()]
            })))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/ssh_keys"))
            .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
                "ssh_key": sample_ssh_key_json()
            })))
            .mount(&server)
            .await;
        Mock::given(method("DELETE"))
            .and(path("/ssh_keys/5"))
            .respond_with(ResponseTemplate::new(204))
            .mount(&server)
            .await;

        let c = client(&server).await;
        assert_eq!(c.list_ssh_keys().await.unwrap().len(), 1);
        let key = c
            .create_ssh_key("key-1".into(), "ssh-ed25519 AAAA".into())
            .await
            .unwrap();
        assert_eq!(key.id, 5);
        c.delete_ssh_key(5).await.unwrap();
    }

    #[tokio::test]
    async fn test_api_error_response_parsed() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/servers"))
            .respond_with(ResponseTemplate::new(401).set_body_json(serde_json::json!({
                "error": {"code": "unauthorized", "message": "bad token", "details": null}
            })))
            .mount(&server)
            .await;
        let err = client(&server).await.list_servers().await.unwrap_err();
        let s = format!("{err:#}");
        assert!(s.contains("unauthorized"), "{s}");
        assert!(s.contains("bad token"), "{s}");
    }

    #[tokio::test]
    async fn test_api_error_with_unparseable_body() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/servers"))
            .respond_with(ResponseTemplate::new(500).set_body_string("oops"))
            .mount(&server)
            .await;
        let err = client(&server).await.list_servers().await.unwrap_err();
        let s = format!("{err:#}");
        assert!(s.contains("500"), "{s}");
        assert!(s.contains("oops"), "{s}");
    }

    #[tokio::test]
    async fn test_delete_failure_includes_status() {
        let server = MockServer::start().await;
        Mock::given(method("DELETE"))
            .and(path("/servers/99"))
            .respond_with(ResponseTemplate::new(404).set_body_string("nope"))
            .mount(&server)
            .await;
        let err = client(&server).await.delete_server(99).await.unwrap_err();
        let s = format!("{err:#}");
        assert!(s.contains("404"), "{s}");
    }
}
