/// Configuration management for Oxide - Talos Kubernetes with Cilium
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Infrastructure provider used to host the cluster.
///
/// `Hcloud` provisions real virtual machines on Hetzner Cloud (the original,
/// production-oriented flow). `Docker` runs a Talos cluster locally as
/// containers via `talosctl cluster create --provisioner docker`, which is
/// useful for development, CI and quick experimentation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    #[default]
    Hcloud,
    Docker,
}

impl Provider {
    /// Returns true when this provider runs purely on the local machine and
    /// therefore does not need any cloud-specific infrastructure (network,
    /// firewall, SSH keys, ...).
    pub const fn is_local(self) -> bool {
        matches!(self, Provider::Docker)
    }
}

/// Main cluster configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterConfig {
    /// Cluster name (used for resource naming)
    pub cluster_name: String,

    /// Infrastructure provider. Defaults to `hcloud` for back-compat with
    /// existing configuration files that pre-date local-cluster support.
    #[serde(default)]
    pub provider: Provider,

    /// Hetzner Cloud configuration. Required when `provider == hcloud`,
    /// ignored otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hcloud: Option<HetznerCloudConfig>,

    /// Docker provisioner configuration. Only consulted when
    /// `provider == docker`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub docker: Option<DockerConfig>,

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

/// Docker provisioner configuration for local clusters.
///
/// Local clusters use Talos's built-in `talosctl cluster create
/// --provisioner docker` which provisions Talos nodes as Docker containers
/// on the developer's machine. The fields below are the most common knobs
/// that users may want to tweak; everything else relies on `talosctl`'s
/// sensible defaults.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DockerConfig {
    /// Optional override for the Talos image used by the docker provisioner
    /// (e.g. `ghcr.io/siderolabs/talos:v1.13.0`). When `None`, talosctl
    /// picks the image matching `talos.version`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,

    /// Optional fixed host port to expose the Kubernetes API on. When
    /// `None`, talosctl chooses a free port automatically.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_port: Option<u16>,

    /// Optional CIDR for the docker bridge network. Talos default is
    /// 10.5.0.0/24 which is fine for most users.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network_cidr: Option<String>,
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
    /// Talos version to use (e.g., "v1.13.0")
    pub version: String,

    /// Kubernetes version (e.g., "1.35.0")
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
    /// Cilium version (e.g., "1.19.3")
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
    /// kube-prometheus-stack Helm chart version (e.g., "84.4.0")
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
    "84.4.0".to_string()
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
    "v1.35.0".to_string()
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

        // Validate node pool counts and names so misconfiguration fails early
        // instead of at provisioning time. A pool with `count = 0` would create
        // no nodes and silently succeed; an empty name would produce malformed
        // server names like "<cluster>--1".
        for pool in self.control_planes.iter().chain(self.workers.iter()) {
            if pool.name.is_empty() {
                anyhow::bail!("node pool 'name' cannot be empty");
            }
            if pool.count == 0 {
                anyhow::bail!(
                    "node pool '{}' has count = 0; must be at least 1",
                    pool.name
                );
            }
        }

        // Provider-specific validation. The `hcloud` block is only required
        // for the Hetzner Cloud provider; for local/docker clusters there is
        // no cloud network or firewall to validate.
        match self.provider {
            Provider::Hcloud => {
                let hcloud = self.hcloud.as_ref().ok_or_else(|| {
                    anyhow::anyhow!(
                        "provider is 'hcloud' but the `hcloud` configuration section is missing"
                    )
                })?;
                self.validate_cidr(&hcloud.network.cidr)?;
                self.validate_cidr(&hcloud.network.subnet_cidr)?;
            }
            Provider::Docker => {
                if let Some(docker) = &self.docker {
                    if let Some(cidr) = &docker.network_cidr {
                        self.validate_cidr(cidr)?;
                    }
                }
                // Autoscaling depends on the Hetzner Cloud API and only makes
                // sense with the hcloud provider.
                if let Some(autoscaler) = &self.autoscaler {
                    if autoscaler.enabled {
                        anyhow::bail!(
                            "cluster autoscaler is only supported with provider = 'hcloud'"
                        );
                    }
                }
                // talosctl's docker provisioner only supports a single
                // control plane node — reject multi-CP configs at parse
                // time rather than failing later during `oxide create`.
                let cp_count: u32 = self.control_planes.iter().map(|p| p.count).sum();
                if cp_count > 1 {
                    anyhow::bail!(
                        "provider = 'docker' only supports a single control plane node \
                         (got {cp_count}); set control_planes[*].count to 1"
                    );
                }
                // The docker provisioner ignores `server_type`, so don't
                // require it to be set on individual node pools.
            }
        }

        // Validate autoscaler pool configuration if autoscaling is enabled.
        // Already short-circuited above for non-hcloud providers.
        if let Some(autoscaler) = &self.autoscaler {
            if autoscaler.enabled {
                if autoscaler.worker_pools.is_empty() {
                    anyhow::bail!("autoscaler is enabled but has no worker_pools configured");
                }

                let mut seen_names = std::collections::HashSet::new();
                for pool in &autoscaler.worker_pools {
                    if pool.name.is_empty() {
                        anyhow::bail!("autoscaler worker_pool 'name' cannot be empty");
                    }
                    if !seen_names.insert(pool.name.clone()) {
                        anyhow::bail!(
                            "autoscaler worker_pool '{}' is defined more than once",
                            pool.name
                        );
                    }
                    if pool.server_type.is_empty() {
                        anyhow::bail!(
                            "autoscaler worker_pool '{}' has empty server_type",
                            pool.name
                        );
                    }
                    if pool.location.is_empty() {
                        anyhow::bail!("autoscaler worker_pool '{}' has empty location", pool.name);
                    }
                    if pool.max_nodes == 0 {
                        anyhow::bail!(
                            "autoscaler worker_pool '{}' has max_nodes = 0; must be > 0",
                            pool.name
                        );
                    }
                    if pool.min_nodes > pool.max_nodes {
                        anyhow::bail!(
                            "autoscaler worker_pool '{}' has min_nodes ({}) > max_nodes ({})",
                            pool.name,
                            pool.min_nodes,
                            pool.max_nodes
                        );
                    }
                }
            }
        }

        Ok(())
    }

    /// Validate CIDR notation with proper IP and prefix length parsing
    fn validate_cidr(&self, cidr: &str) -> anyhow::Result<()> {
        use anyhow::Context;
        use std::net::IpAddr;

        let parts: Vec<&str> = cidr.split('/').collect();
        if parts.len() != 2 {
            anyhow::bail!(
                "Invalid CIDR notation '{}': must be in format IP/prefix (e.g., 10.0.0.0/16)",
                cidr
            );
        }

        // Validate IP address part
        let ip: IpAddr = parts[0].parse().context(format!(
            "Invalid IP address '{}' in CIDR notation",
            parts[0]
        ))?;

        // Validate prefix length
        let prefix: u8 = parts[1].parse().context(format!(
            "Invalid prefix length '{}' in CIDR notation",
            parts[1]
        ))?;

        // Check prefix length is within valid range based on IP version
        let max_prefix = match ip {
            IpAddr::V4(_) => 32,
            IpAddr::V6(_) => 128,
        };

        if prefix > max_prefix {
            anyhow::bail!(
                "Invalid CIDR prefix length: {} (must be 0-{} for {:?})",
                prefix,
                max_prefix,
                ip
            );
        }

        Ok(())
    }

    /// Get Hetzner Cloud API token from config or environment
    pub fn get_hcloud_token(&self) -> anyhow::Result<String> {
        let token = self
            .hcloud
            .as_ref()
            .and_then(|h| h.token.clone())
            .or_else(|| std::env::var("HCLOUD_TOKEN").ok())
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Hetzner Cloud API token not found. Set HCLOUD_TOKEN environment variable or specify in config"
                )
            })?;
        Ok(token)
    }

    /// Generate an example configuration file for the Hetzner Cloud provider.
    pub fn example() -> Self {
        Self {
            cluster_name: "talos-cluster".to_string(),
            provider: Provider::Hcloud,
            hcloud: Some(HetznerCloudConfig {
                token: None,
                location: "nbg1".to_string(),
                network: NetworkConfig {
                    cidr: "10.0.0.0/16".to_string(),
                    subnet_cidr: "10.0.1.0/24".to_string(),
                    zone: "eu-central".to_string(),
                },
            }),
            docker: None,
            talos: TalosConfig {
                version: "v1.13.0".to_string(),
                kubernetes_version: "1.35.0".to_string(),
                cluster_endpoint: None,
                hcloud_snapshot_id: None,
                config_patches: vec![],
            },
            cilium: CiliumConfig {
                version: "1.19.3".to_string(),
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

    /// Generate an example configuration file for the local Docker provider.
    ///
    /// Local clusters are intended for development and CI: a single control
    /// plane and a single worker by default, no Hetzner-specific fields, and
    /// no autoscaler (which only makes sense against a real cloud).
    pub fn example_local() -> Self {
        Self {
            cluster_name: "talos-local".to_string(),
            provider: Provider::Docker,
            hcloud: None,
            docker: Some(DockerConfig::default()),
            talos: TalosConfig {
                version: "v1.13.0".to_string(),
                kubernetes_version: "1.35.0".to_string(),
                cluster_endpoint: None,
                hcloud_snapshot_id: None,
                config_patches: vec![],
            },
            cilium: CiliumConfig {
                version: "1.19.3".to_string(),
                enable_hubble: true,
                enable_ipv6: false,
                helm_values: serde_yaml::Value::Null,
            },
            prometheus: Some(PrometheusConfig {
                // Local clusters have no default StorageClass; persistent
                // PVCs would stay Pending and stall `wait_for_ready`.
                enable_persistent_storage: false,
                ..PrometheusConfig::default()
            }),
            metrics_server: Some(MetricsServerConfig { enabled: true }),
            autoscaler: None,
            control_planes: vec![NodeConfig {
                name: "control-plane".to_string(),
                // server_type is unused by the docker provisioner but kept
                // for schema parity with the Hetzner config.
                server_type: String::new(),
                count: 1,
                labels: std::collections::HashMap::new(),
            }],
            workers: vec![NodeConfig {
                name: "worker".to_string(),
                server_type: String::new(),
                count: 1,
                labels: std::collections::HashMap::new(),
            }],
        }
    }

    /// Initialize example configuration file
    pub async fn init(config_path: &Path, provider: Provider) -> anyhow::Result<()> {
        use anyhow::Context;
        use tracing::info;

        if config_path.exists() {
            anyhow::bail!(
                "Configuration file already exists: {}",
                config_path.display()
            );
        }

        let example_config = match provider {
            Provider::Hcloud => Self::example(),
            Provider::Docker => Self::example_local(),
        };
        let yaml = serde_yaml::to_string(&example_config)?;

        tokio::fs::write(config_path, yaml)
            .await
            .context("Failed to write configuration file")?;

        info!("Example configuration created: {}", config_path.display());
        info!("Next steps:");
        info!("  1. Edit the configuration file to match your requirements");
        match provider {
            Provider::Hcloud => {
                info!("  2. Set your Hetzner Cloud API token:");
                info!("     export HCLOUD_TOKEN=your-token-here");
                info!("  3. Create the cluster:");
                info!("     oxide create");
            }
            Provider::Docker => {
                info!("  2. Make sure Docker and talosctl are installed locally");
                info!("  3. Create the cluster:");
                info!("     oxide create");
            }
        }

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

        // Valid IPv4 CIDRs
        assert!(config.validate_cidr("10.0.0.0/16").is_ok());
        assert!(config.validate_cidr("192.168.1.0/24").is_ok());
        assert!(config.validate_cidr("172.16.0.0/12").is_ok());
        assert!(config.validate_cidr("10.0.0.0/32").is_ok());
        assert!(config.validate_cidr("0.0.0.0/0").is_ok());

        // Valid IPv6 CIDRs
        assert!(config.validate_cidr("2001:db8::/32").is_ok());
        assert!(config.validate_cidr("fe80::/10").is_ok());
        assert!(config.validate_cidr("::1/128").is_ok());

        // Invalid CIDRs
        assert!(config.validate_cidr("invalid").is_err());
        assert!(config.validate_cidr("10.0.0.0").is_err()); // Missing prefix
        assert!(config.validate_cidr("10.0.0.0/").is_err()); // Empty prefix
        assert!(config.validate_cidr("10.0.0.0/33").is_err()); // Invalid prefix for IPv4
        assert!(config.validate_cidr("10.0.0.0/abc").is_err()); // Non-numeric prefix
        assert!(config.validate_cidr("999.0.0.0/16").is_err()); // Invalid IP
        assert!(config.validate_cidr("10.0.0/16").is_err()); // Incomplete IP
        assert!(config.validate_cidr("2001:db8::/129").is_err()); // Invalid prefix for IPv6
    }

    #[test]
    fn test_local_example_validates() {
        let config = ClusterConfig::example_local();
        assert_eq!(config.provider, Provider::Docker);
        assert!(config.hcloud.is_none());
        assert!(
            config.validate().is_ok(),
            "default local example must validate"
        );
    }

    #[test]
    fn test_local_rejects_autoscaler() {
        let mut config = ClusterConfig::example_local();
        config.autoscaler = Some(AutoscalerConfig {
            enabled: true,
            version: default_autoscaler_version(),
            worker_pools: vec![AutoscalePoolConfig {
                name: "worker".into(),
                server_type: "cpx21".into(),
                location: "nbg1".into(),
                min_nodes: 1,
                max_nodes: 3,
            }],
        });
        let err = config.validate().unwrap_err().to_string();
        assert!(err.contains("autoscaler"), "got: {err}");
    }

    #[test]
    fn test_hcloud_provider_requires_hcloud_section() {
        let mut config = ClusterConfig::example();
        config.hcloud = None;
        let err = config.validate().unwrap_err().to_string();
        assert!(err.contains("hcloud"), "got: {err}");
    }
}
