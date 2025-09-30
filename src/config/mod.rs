/// Configuration management for Oxide - Talos Kubernetes with Cilium
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Main cluster configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterConfig {
    /// Cluster name (used for resource naming)
    pub cluster_name: String,

    /// Hetzner Cloud configuration
    pub hcloud: HetznerCloudConfig,

    /// Talos configuration
    pub talos: TalosConfig,

    /// Cilium configuration
    pub cilium: CiliumConfig,

    /// Control plane nodes
    pub control_planes: Vec<NodeConfig>,

    /// Worker nodes
    pub workers: Vec<NodeConfig>,
}

/// Hetzner Cloud API and network configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HetznerCloudConfig {
    /// Hetzner Cloud API token (can also be set via HCLOUD_TOKEN env var)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,

    /// Hetzner Cloud region
    pub location: String,

    /// Private network configuration
    pub network: NetworkConfig,
}

/// Private network configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkConfig {
    /// Network CIDR (e.g., "10.0.0.0/16")
    pub cidr: String,

    /// Subnet CIDR for the cluster (e.g., "10.0.1.0/24")
    pub subnet_cidr: String,

    /// Network zone (e.g., "eu-central")
    pub zone: String,
}

/// Talos-specific configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TalosConfig {
    /// Talos version to use (e.g., "v1.7.0")
    pub version: String,

    /// Kubernetes version (e.g., "1.30.0")
    pub kubernetes_version: String,

    /// Cluster endpoint (will be set to first control plane IP if not specified)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cluster_endpoint: Option<String>,

    /// Hetzner Cloud snapshot ID containing Talos image
    /// If not provided, servers will be created with Ubuntu and require manual Talos installation
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hcloud_snapshot_id: Option<String>,

    /// Additional Talos machine config patches
    #[serde(default)]
    pub config_patches: Vec<String>,
}

/// Cilium CNI configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CiliumConfig {
    /// Cilium version (e.g., "1.15.0")
    pub version: String,

    /// Enable Hubble observability
    #[serde(default = "default_true")]
    pub enable_hubble: bool,

    /// Enable IPv6 support
    #[serde(default)]
    pub enable_ipv6: bool,

    /// Additional Cilium Helm values
    #[serde(default)]
    pub helm_values: serde_yaml::Value,
}

/// Node configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeConfig {
    /// Node name prefix
    pub name: String,

    /// Hetzner server type (e.g., "cx21", "cpx31")
    pub server_type: String,

    /// Number of nodes to create with this configuration
    #[serde(default = "default_one")]
    pub count: u32,

    /// Additional labels for the node
    #[serde(default)]
    pub labels: std::collections::HashMap<String, String>,
}

fn default_true() -> bool {
    true
}

fn default_one() -> u32 {
    1
}

impl ClusterConfig {
    /// Load configuration from a YAML file
    pub fn from_file<P: AsRef<Path>>(path: P) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let config: ClusterConfig = serde_yaml::from_str(&content)?;
        config.validate()?;
        Ok(config)
    }

    /// Validate the configuration
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.cluster_name.is_empty() {
            anyhow::bail!("cluster_name cannot be empty");
        }

        if self.control_planes.is_empty() {
            anyhow::bail!("at least one control plane node is required");
        }

        // Validate network CIDRs
        self.validate_cidr(&self.hcloud.network.cidr)?;
        self.validate_cidr(&self.hcloud.network.subnet_cidr)?;

        Ok(())
    }

    /// Validate CIDR notation
    fn validate_cidr(&self, cidr: &str) -> anyhow::Result<()> {
        if !cidr.contains('/') {
            anyhow::bail!("Invalid CIDR notation: {}", cidr);
        }
        Ok(())
    }

    /// Get Hetzner Cloud API token from config or environment
    pub fn get_hcloud_token(&self) -> anyhow::Result<String> {
        self.hcloud.token
            .clone()
            .or_else(|| std::env::var("HCLOUD_TOKEN").ok())
            .ok_or_else(|| anyhow::anyhow!(
                "Hetzner Cloud API token not found. Set HCLOUD_TOKEN environment variable or specify in config"
            ))
    }

    /// Generate an example configuration file
    pub fn example() -> Self {
        Self {
            cluster_name: "talos-cluster".to_string(),
            hcloud: HetznerCloudConfig {
                token: None,
                location: "nbg1".to_string(),
                network: NetworkConfig {
                    cidr: "10.0.0.0/16".to_string(),
                    subnet_cidr: "10.0.1.0/24".to_string(),
                    zone: "eu-central".to_string(),
                },
            },
            talos: TalosConfig {
                version: "v1.7.0".to_string(),
                kubernetes_version: "1.30.0".to_string(),
                cluster_endpoint: None,
                hcloud_snapshot_id: None,
                config_patches: vec![],
            },
            cilium: CiliumConfig {
                version: "1.15.0".to_string(),
                enable_hubble: true,
                enable_ipv6: false,
                helm_values: serde_yaml::Value::Null,
            },
            control_planes: vec![NodeConfig {
                name: "control-plane".to_string(),
                server_type: "cpx21".to_string(),
                count: 3,
                labels: std::collections::HashMap::new(),
            }],
            workers: vec![NodeConfig {
                name: "worker".to_string(),
                server_type: "cpx31".to_string(),
                count: 3,
                labels: std::collections::HashMap::new(),
            }],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_validation() {
        let mut config = ClusterConfig::example();
        assert!(config.validate().is_ok());

        config.cluster_name = String::new();
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_cidr_validation() {
        let config = ClusterConfig::example();
        assert!(config.validate_cidr("10.0.0.0/16").is_ok());
        assert!(config.validate_cidr("invalid").is_err());
    }
}
