/// Hetzner Cloud API data models
use serde::{Deserialize, Serialize};

/// Hetzner Cloud server resource
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Server {
    pub id: u64,
    pub name: String,
    pub status: String,
    pub server_type: ServerType,
    pub datacenter: Datacenter,
    pub public_net: PublicNetwork,
    pub private_net: Vec<PrivateNetwork>,
    pub created: String,
    #[serde(default)]
    pub labels: std::collections::HashMap<String, String>,
}

/// Server type information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerType {
    pub id: u64,
    pub name: String,
    pub description: String,
    pub cores: u32,
    pub memory: f64,
    pub disk: u64,
}

/// Datacenter information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Datacenter {
    pub id: u64,
    pub name: String,
    pub description: String,
    pub location: Location,
}

/// Location information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Location {
    pub id: u64,
    pub name: String,
    pub description: String,
    pub country: String,
    pub city: String,
    pub latitude: f64,
    pub longitude: f64,
}

/// Public network configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublicNetwork {
    pub ipv4: Option<IPv4>,
    pub ipv6: Option<IPv6>,
    pub floating_ips: Vec<u64>,
}

/// IPv4 address information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IPv4 {
    pub ip: String,
    pub blocked: bool,
}

/// IPv6 address information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IPv6 {
    pub ip: String,
    pub blocked: bool,
}

/// Private network attachment
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrivateNetwork {
    pub network: u64,
    pub ip: String,
    pub alias_ips: Vec<String>,
    pub mac_address: String,
}

/// Network resource
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Network {
    pub id: u64,
    pub name: String,
    pub ip_range: String,
    pub subnets: Vec<Subnet>,
    pub routes: Vec<Route>,
    pub servers: Vec<u64>,
    pub created: String,
}

/// Network subnet
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Subnet {
    pub ip_range: String,
    pub network_zone: String,
    pub gateway: String,
    #[serde(rename = "type")]
    pub subnet_type: String,
}

/// Network route
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Route {
    pub destination: String,
    pub gateway: String,
}

/// SSH key resource
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SSHKey {
    pub id: u64,
    pub name: String,
    pub fingerprint: String,
    pub public_key: String,
    pub labels: std::collections::HashMap<String, String>,
    pub created: String,
}

/// Action represents an asynchronous operation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Action {
    pub id: u64,
    pub command: String,
    pub status: String,
    pub progress: u32,
    pub started: String,
    pub finished: Option<String>,
    pub error: Option<ActionError>,
}

/// Action error details
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionError {
    pub code: String,
    pub message: String,
}

/// Generic API response wrapper
#[allow(dead_code)]
#[derive(Debug, Serialize, Deserialize)]
pub struct ApiResponse<T> {
    #[serde(flatten)]
    pub data: T,
}

/// Server creation response
#[derive(Debug, Serialize, Deserialize)]
pub struct CreateServerResponse {
    pub server: Server,
    pub action: Action,
    pub root_password: Option<String>,
}

/// Network creation response
#[derive(Debug, Serialize, Deserialize)]
pub struct CreateNetworkResponse {
    pub network: Network,
}

/// SSH key creation response
#[allow(dead_code)]
#[derive(Debug, Serialize, Deserialize)]
pub struct CreateSSHKeyResponse {
    pub ssh_key: SSHKey,
}

/// Server list response
#[derive(Debug, Serialize, Deserialize)]
pub struct ServerListResponse {
    pub servers: Vec<Server>,
}

/// Network list response
#[derive(Debug, Serialize, Deserialize)]
pub struct NetworkListResponse {
    pub networks: Vec<Network>,
}

/// SSH key list response
#[allow(dead_code)]
#[derive(Debug, Serialize, Deserialize)]
pub struct SSHKeyListResponse {
    pub ssh_keys: Vec<SSHKey>,
}

/// Action response
#[derive(Debug, Serialize, Deserialize)]
pub struct ActionResponse {
    pub action: Action,
}

/// Error response from API
#[derive(Debug, Serialize, Deserialize)]
pub struct ErrorResponse {
    pub error: ApiError,
}

/// API error details
#[derive(Debug, Serialize, Deserialize)]
pub struct ApiError {
    pub code: String,
    pub message: String,
    pub details: Option<serde_json::Value>,
}

/// Firewall resource
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Firewall {
    pub id: u64,
    pub name: String,
    pub rules: Vec<FirewallRule>,
    pub applied_to: Vec<FirewallResource>,
    pub created: String,
    pub labels: std::collections::HashMap<String, String>,
}

/// Firewall rule
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FirewallRule {
    pub direction: String,
    pub source_ips: Vec<String>,
    pub destination_ips: Vec<String>,
    pub protocol: String,
    pub port: Option<String>,
}

/// Firewall resource attachment
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FirewallResource {
    #[serde(rename = "type")]
    pub resource_type: String,
    pub server: Option<FirewallServer>,
}

/// Firewall server reference
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FirewallServer {
    pub id: u64,
}

/// Firewall creation response
#[derive(Debug, Serialize, Deserialize)]
pub struct CreateFirewallResponse {
    pub firewall: Firewall,
    pub actions: Vec<Action>,
}

/// Firewall list response
#[derive(Debug, Serialize, Deserialize)]
pub struct FirewallListResponse {
    pub firewalls: Vec<Firewall>,
}
