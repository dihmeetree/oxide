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

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal valid server JSON fixture (no labels field → tests #[serde(default)])
    fn minimal_server_json(id: u64, name: &str) -> String {
        format!(
            r#"{{
                "id": {id},
                "name": "{name}",
                "status": "running",
                "server_type": {{"id": 1, "name": "cx11", "description": "CX11", "cores": 2, "memory": 4.0, "disk": 40}},
                "datacenter": {{
                    "id": 1, "name": "fsn1-dc14", "description": "Falkenstein 1",
                    "location": {{"id": 1, "name": "fsn1", "description": "Falkenstein DC Park 1",
                                  "country": "DE", "city": "Falkenstein", "latitude": 50.47612, "longitude": 12.370071}}
                }},
                "public_net": {{"ipv4": {{"ip": "1.2.3.4", "blocked": false}}, "ipv6": null, "floating_ips": []}},
                "private_net": [],
                "created": "2023-06-01T10:00:00+00:00"
            }}"#
        )
    }

    #[test]
    fn test_server_round_trip() {
        let json = minimal_server_json(123, "my-server");
        let server: Server = serde_json::from_str(&json).unwrap();
        assert_eq!(server.id, 123);
        assert_eq!(server.name, "my-server");
        assert_eq!(server.status, "running");
        assert_eq!(server.server_type.cores, 2);

        let serialized = serde_json::to_string(&server).unwrap();
        let deserialized: Server = serde_json::from_str(&serialized).unwrap();
        assert_eq!(deserialized.id, server.id);
        assert_eq!(deserialized.name, server.name);
        assert_eq!(deserialized.server_type.name, "cx11");
    }

    #[test]
    fn test_server_labels_default_when_missing() {
        // No "labels" field in JSON → #[serde(default)] should give empty HashMap
        let json = minimal_server_json(1, "no-labels");
        let server: Server = serde_json::from_str(&json).unwrap();
        assert!(server.labels.is_empty());
    }

    #[test]
    fn test_server_labels_present() {
        let mut json: serde_json::Value =
            serde_json::from_str(&minimal_server_json(2, "with-labels")).unwrap();
        json["labels"] = serde_json::json!({"talos-version": "v1.6.0", "cluster": "prod"});
        let server: Server = serde_json::from_value(json).unwrap();
        assert_eq!(
            server.labels.get("talos-version"),
            Some(&"v1.6.0".to_string())
        );
        assert_eq!(server.labels.get("cluster"), Some(&"prod".to_string()));
    }

    #[test]
    fn test_subnet_type_serde_rename() {
        // "type" in JSON must deserialize into field `subnet_type`
        let json = r#"{"ip_range": "10.0.1.0/24", "network_zone": "eu-central", "gateway": "10.0.0.1", "type": "cloud"}"#;
        let subnet: Subnet = serde_json::from_str(json).unwrap();
        assert_eq!(subnet.subnet_type, "cloud");

        // Serialized back it must use "type" key, not "subnet_type"
        let value = serde_json::to_value(&subnet).unwrap();
        assert_eq!(value["type"], "cloud");
        assert!(value.get("subnet_type").is_none());
    }

    #[test]
    fn test_network_round_trip() {
        let json = r#"{
            "id": 10, "name": "my-network", "ip_range": "10.0.0.0/16",
            "subnets": [{"ip_range": "10.0.1.0/24", "network_zone": "eu-central", "gateway": "10.0.0.1", "type": "cloud"}],
            "routes": [{"destination": "0.0.0.0/0", "gateway": "10.0.0.1"}],
            "servers": [123, 456],
            "created": "2023-01-01T00:00:00+00:00"
        }"#;
        let network: Network = serde_json::from_str(json).unwrap();
        assert_eq!(network.id, 10);
        assert_eq!(network.subnets.len(), 1);
        assert_eq!(network.subnets[0].subnet_type, "cloud");
        assert_eq!(network.servers, vec![123, 456]);
        assert_eq!(network.routes[0].destination, "0.0.0.0/0");

        let re = serde_json::to_string(&network).unwrap();
        let network2: Network = serde_json::from_str(&re).unwrap();
        assert_eq!(network2.id, network.id);
        assert_eq!(network2.subnets[0].subnet_type, "cloud");
    }

    #[test]
    fn test_firewall_round_trip() {
        let json = r#"{
            "id": 1, "name": "fw-1",
            "rules": [
                {"direction": "in", "source_ips": ["0.0.0.0/0", "::/0"], "destination_ips": [], "protocol": "tcp", "port": "80"},
                {"direction": "in", "source_ips": [], "destination_ips": [], "protocol": "icmp", "port": null}
            ],
            "applied_to": [{"type": "server", "server": {"id": 42}}],
            "created": "2023-01-01T00:00:00+00:00",
            "labels": {}
        }"#;
        let fw: Firewall = serde_json::from_str(json).unwrap();
        assert_eq!(fw.id, 1);
        assert_eq!(fw.rules.len(), 2);
        assert_eq!(fw.rules[0].port, Some("80".to_string()));
        assert!(fw.rules[1].port.is_none());
        assert_eq!(fw.applied_to[0].resource_type, "server");

        let re = serde_json::to_string(&fw).unwrap();
        let fw2: Firewall = serde_json::from_str(&re).unwrap();
        assert_eq!(fw2.rules[0].direction, "in");
        // "type" serde rename on FirewallResource
        let value = serde_json::to_value(&fw.applied_to[0]).unwrap();
        assert_eq!(value["type"], "server");
        assert!(value.get("resource_type").is_none());
    }

    #[test]
    fn test_action_success_round_trip() {
        let json = r#"{
            "id": 42, "command": "create_server", "status": "success", "progress": 100,
            "started": "2023-01-01T00:00:00+00:00", "finished": "2023-01-01T00:01:00+00:00", "error": null
        }"#;
        let action: Action = serde_json::from_str(json).unwrap();
        assert_eq!(action.id, 42);
        assert_eq!(action.status, "success");
        assert!(action.finished.is_some());
        assert!(action.error.is_none());

        let re = serde_json::to_string(&action).unwrap();
        let action2: Action = serde_json::from_str(&re).unwrap();
        assert_eq!(action2.id, action.id);
        assert_eq!(action2.command, "create_server");
    }

    #[test]
    fn test_action_with_error() {
        let json = r#"{
            "id": 1, "command": "create_server", "status": "error", "progress": 0,
            "started": "2023-01-01T00:00:00+00:00", "finished": null,
            "error": {"code": "limit_exceeded", "message": "Server limit exceeded"}
        }"#;
        let action: Action = serde_json::from_str(json).unwrap();
        assert_eq!(action.status, "error");
        let err = action.error.unwrap();
        assert_eq!(err.code, "limit_exceeded");
        assert_eq!(err.message, "Server limit exceeded");
    }

    #[test]
    fn test_create_server_response_round_trip() {
        let server_json = minimal_server_json(1, "new-server");
        let mut v: serde_json::Value = serde_json::from_str(&server_json).unwrap();
        v["labels"] = serde_json::json!({});
        let json = serde_json::json!({
            "server": v,
            "action": {"id": 1, "command": "create_server", "status": "running", "progress": 0,
                        "started": "2023-01-01T00:00:00+00:00", "finished": null, "error": null},
            "root_password": "s3cr3t"
        });
        let resp: CreateServerResponse = serde_json::from_value(json).unwrap();
        assert_eq!(resp.server.name, "new-server");
        assert_eq!(resp.action.command, "create_server");
        assert_eq!(resp.root_password, Some("s3cr3t".to_string()));

        let re = serde_json::to_string(&resp).unwrap();
        let resp2: CreateServerResponse = serde_json::from_str(&re).unwrap();
        assert_eq!(resp2.server.id, resp.server.id);
    }

    #[test]
    fn test_create_server_response_no_password() {
        let server_json = minimal_server_json(2, "key-auth-server");
        let mut sv: serde_json::Value = serde_json::from_str(&server_json).unwrap();
        sv["labels"] = serde_json::json!({});
        let json = serde_json::json!({
            "server": sv,
            "action": {"id": 2, "command": "create_server", "status": "running", "progress": 0,
                        "started": "2023-01-01T00:00:00+00:00", "finished": null, "error": null},
            "root_password": null
        });
        let resp: CreateServerResponse = serde_json::from_value(json).unwrap();
        assert!(resp.root_password.is_none());
    }

    #[test]
    fn test_server_list_response_empty() {
        let r: ServerListResponse = serde_json::from_str(r#"{"servers": []}"#).unwrap();
        assert!(r.servers.is_empty());
    }

    #[test]
    fn test_network_list_response_empty() {
        let r: NetworkListResponse = serde_json::from_str(r#"{"networks": []}"#).unwrap();
        assert!(r.networks.is_empty());
    }

    #[test]
    fn test_firewall_list_response_empty() {
        let r: FirewallListResponse = serde_json::from_str(r#"{"firewalls": []}"#).unwrap();
        assert!(r.firewalls.is_empty());
    }

    #[test]
    fn test_error_response_round_trip() {
        let json =
            r#"{"error": {"code": "not_found", "message": "Resource not found", "details": null}}"#;
        let resp: ErrorResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.error.code, "not_found");
        assert!(resp.error.details.is_none());

        let re = serde_json::to_string(&resp).unwrap();
        let resp2: ErrorResponse = serde_json::from_str(&re).unwrap();
        assert_eq!(resp2.error.code, resp.error.code);
    }

    #[test]
    fn test_error_response_with_details() {
        let json = r#"{"error": {"code": "invalid_input", "message": "Bad request", "details": {"field": "name", "reason": "required"}}}"#;
        let resp: ErrorResponse = serde_json::from_str(json).unwrap();
        assert!(resp.error.details.is_some());
        let details = resp.error.details.unwrap();
        assert_eq!(details["field"], "name");
    }

    #[test]
    fn test_private_network_in_server() {
        let json = r#"{
            "id": 5, "name": "private-net-server", "status": "running",
            "server_type": {"id": 1, "name": "cx21", "description": "CX21", "cores": 2, "memory": 4.0, "disk": 40},
            "datacenter": {"id": 1, "name": "fsn1-dc14", "description": "Falkenstein",
                            "location": {"id": 1, "name": "fsn1", "description": "FSN", "country": "DE",
                                          "city": "Falkenstein", "latitude": 50.4, "longitude": 12.3}},
            "public_net": {"ipv4": {"ip": "5.6.7.8", "blocked": false}, "ipv6": null, "floating_ips": [1, 2]},
            "private_net": [{"network": 10, "ip": "10.0.0.2", "alias_ips": [], "mac_address": "86:00:00:12:34:56"}],
            "created": "2023-06-01T00:00:00+00:00",
            "labels": {}
        }"#;
        let server: Server = serde_json::from_str(json).unwrap();
        assert_eq!(server.private_net.len(), 1);
        assert_eq!(server.private_net[0].ip, "10.0.0.2");
        assert_eq!(server.public_net.floating_ips, vec![1, 2]);
    }
}
