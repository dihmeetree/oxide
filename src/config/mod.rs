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

    /// Prometheus configuration
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prometheus: Option<PrometheusConfig>,

    /// Cluster autoscaler configuration
    #[serde(skip_serializing_if = "Option::is_none")]
    pub autoscaler: Option<AutoscalerConfig>,

    /// Metrics server configuration
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metrics_server: Option<MetricsServerConfig>,

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

/// Prometheus monitoring configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrometheusConfig {
    /// kube-prometheus-stack Helm chart version (e.g., "65.8.1")
    #[serde(default = "default_prometheus_version")]
    pub version: String,

    /// Enable Prometheus
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Namespace to deploy Prometheus in
    #[serde(default = "default_prometheus_namespace")]
    pub namespace: String,

    /// Enable Grafana dashboards
    #[serde(default = "default_true")]
    pub enable_grafana: bool,

    /// Enable AlertManager
    #[serde(default = "default_true")]
    pub enable_alertmanager: bool,

    /// Prometheus retention period (e.g., "30d")
    #[serde(default = "default_prometheus_retention")]
    pub retention: String,

    /// Prometheus storage size (e.g., "50Gi")
    #[serde(default = "default_prometheus_storage")]
    pub storage_size: String,

    /// Enable persistent storage for Prometheus
    #[serde(default = "default_true")]
    pub enable_persistent_storage: bool,

    /// Additional Helm values
    #[serde(default)]
    pub helm_values: serde_yaml::Value,
}

/// Metrics server configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricsServerConfig {
    /// Enable metrics server
    #[serde(default = "default_true")]
    pub enabled: bool,
}

/// Cluster autoscaler configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutoscalerConfig {
    /// Enable cluster autoscaler
    #[serde(default)]
    pub enabled: bool,

    /// Cluster autoscaler version
    #[serde(default = "default_autoscaler_version")]
    pub version: String,

    /// Worker pools to autoscale with min/max limits
    #[serde(default)]
    pub worker_pools: Vec<AutoscalePoolConfig>,
}

/// Autoscale configuration for a specific worker pool
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutoscalePoolConfig {
    /// Worker pool name (must match a pool in workers)
    pub name: String,

    /// Server type (e.g., "cpx11", "cpx21")
    pub server_type: String,

    /// Location (e.g., "fsn1", "nbg1", "hel1")
    pub location: String,

    /// Minimum number of nodes (set to 0 to only manage autoscaled nodes)
    #[serde(default)]
    pub min_nodes: u32,

    /// Maximum number of nodes
    pub max_nodes: u32,
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

fn default_prometheus_version() -> String {
    "65.8.1".to_string()
}

fn default_prometheus_namespace() -> String {
    "monitoring".to_string()
}

fn default_prometheus_retention() -> String {
    "30d".to_string()
}

fn default_prometheus_storage() -> String {
    "50Gi".to_string()
}

fn default_autoscaler_version() -> String {
    "v1.34.0".to_string()
}

impl PrometheusConfig {
    /// Create default Prometheus configuration
    pub fn default() -> Self {
        Self {
            version: default_prometheus_version(),
            enabled: true,
            namespace: default_prometheus_namespace(),
            enable_grafana: true,
            enable_alertmanager: true,
            retention: default_prometheus_retention(),
            storage_size: default_prometheus_storage(),
            enable_persistent_storage: true,
            helm_values: serde_yaml::Value::Null,
        }
    }
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
                version: "v1.11.2".to_string(),
                kubernetes_version: "1.34.1".to_string(),
                cluster_endpoint: None,
                hcloud_snapshot_id: None,
                config_patches: vec![],
            },
            cilium: CiliumConfig {
                version: "1.17.8".to_string(),
                enable_hubble: true,
                enable_ipv6: false,
                helm_values: serde_yaml::Value::Null,
            },
            prometheus: Some(PrometheusConfig::default()),
            metrics_server: Some(MetricsServerConfig { enabled: true }),
            autoscaler: None,
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

    /// Initialize example configuration file
    pub async fn init(config_path: &Path) -> anyhow::Result<()> {
        use anyhow::Context;
        use tracing::info;

        if config_path.exists() {
            anyhow::bail!(
                "Configuration file already exists: {}",
                config_path.display()
            );
        }

        let example_config = Self::example();
        let yaml = serde_yaml::to_string(&example_config)?;

        tokio::fs::write(config_path, yaml)
            .await
            .context("Failed to write configuration file")?;

        info!("Example configuration created: {}", config_path.display());
        info!("");
        info!("Next steps:");
        info!("  1. Edit the configuration file to match your requirements");
        info!("  2. Set your Hetzner Cloud API token:");
        info!("     export HCLOUD_TOKEN=your-token-here");
        info!("  3. Create the cluster:");
        info!("     oxide create");

        Ok(())
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
