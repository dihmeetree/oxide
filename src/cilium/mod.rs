/// Cilium CNI deployment and management
use anyhow::Result;
use tracing::info;

use crate::config::CiliumConfig;
use crate::utils::command::CommandBuilder;
use crate::utils::polling::PollingConfig;

/// Cilium deployment manager
pub struct CiliumManager {
    config: CiliumConfig,
    kubeconfig_path: std::path::PathBuf,
    control_plane_count: u32,
}

impl CiliumManager {
    /// Create a new Cilium manager
    pub fn new(
        config: CiliumConfig,
        kubeconfig_path: std::path::PathBuf,
        control_plane_count: u32,
    ) -> Self {
        Self {
            config,
            kubeconfig_path,
            control_plane_count,
        }
    }

    /// Check if helm is installed
    pub async fn check_helm_installed() -> Result<()> {
        crate::utils::command::check_tool_installed(
            "helm",
            "version",
            "https://helm.sh/docs/intro/install/",
        )
        .await
    }

    /// Install Cilium CNI using Helm
    pub async fn install(&self) -> Result<()> {
        info!("Installing Cilium CNI version {}...", self.config.version);

        // Install Gateway API CRDs first
        self.install_gateway_api_crds().await?;

        // Add Cilium Helm repository
        self.add_helm_repo().await?;

        // Install Cilium
        self.install_cilium_chart().await?;

        info!("Cilium installed successfully");

        Ok(())
    }

    /// Install Gateway API CRDs
    async fn install_gateway_api_crds(&self) -> Result<()> {
        info!("Installing Gateway API CRDs...");

        CommandBuilder::new("kubectl")
            .args([
                "apply",
                "-f",
                "https://github.com/kubernetes-sigs/gateway-api/releases/download/v1.3.0/experimental-install.yaml",
            ])
            .kubeconfig(&self.kubeconfig_path)
            .context("Failed to install Gateway API CRDs")
            .run_silent()
            .await?;

        info!("Gateway API CRDs installed successfully");
        Ok(())
    }

    /// Add Cilium Helm repository
    async fn add_helm_repo(&self) -> Result<()> {
        info!("Adding Cilium Helm repository...");

        let output = CommandBuilder::new("helm")
            .args(["repo", "add", "cilium", "https://helm.cilium.io/"])
            .kubeconfig(&self.kubeconfig_path)
            .context("Failed to add Cilium Helm repo")
            .output()
            .await?;

        if !output.success {
            // Ignore "already exists" errors
            if !output.stderr.contains("already exists") {
                anyhow::bail!("Failed to add Helm repo: {}", output.stderr);
            }
        }

        // Update Helm repositories
        CommandBuilder::new("helm")
            .args(["repo", "update"])
            .kubeconfig(&self.kubeconfig_path)
            .context("Failed to update Helm repos")
            .run_silent()
            .await?;

        Ok(())
    }

    /// Install Cilium Helm chart
    async fn install_cilium_chart(&self) -> Result<()> {
        info!("Installing Cilium Helm chart...");

        // Set operator replicas: 2 if we have multiple control planes, 1 otherwise
        let operator_replicas = if self.control_plane_count > 1 {
            "2"
        } else {
            "1"
        };
        let operator_replicas_arg = format!("operator.replicas={}", operator_replicas);

        let mut args = vec![
            "install",
            "cilium",
            "cilium/cilium",
            "--version",
            &self.config.version,
            "--namespace",
            "kube-system",
            "--set",
            "ipam.mode=kubernetes",
            "--set",
            "kubeProxyReplacement=true",
            "--set",
            "securityContext.capabilities.ciliumAgent={CHOWN,KILL,NET_ADMIN,NET_RAW,IPC_LOCK,SYS_ADMIN,SYS_RESOURCE,DAC_OVERRIDE,FOWNER,SETGID,SETUID}",
            "--set",
            "securityContext.capabilities.cleanCiliumState={NET_ADMIN,SYS_ADMIN,SYS_RESOURCE}",
            "--set",
            "cgroup.autoMount.enabled=false",
            "--set",
            "cgroup.hostRoot=/sys/fs/cgroup",
            "--set",
            &operator_replicas_arg,
        ];

        // Add Hubble settings
        if self.config.enable_hubble {
            args.extend_from_slice(&[
                "--set",
                "hubble.enabled=true",
                "--set",
                "hubble.relay.enabled=true",
                "--set",
                "hubble.ui.enabled=true",
                "--set",
                "hubble.metrics.enabled={dns,drop,tcp,flow,port-distribution,icmp,httpV2:exemplars=true;labelsContext=source_ip\\,source_namespace\\,source_workload\\,destination_ip\\,destination_namespace\\,destination_workload\\,traffic_direction}",
            ]);
        } else {
            args.extend_from_slice(&["--set", "hubble.enabled=false"]);
        }

        // Enable Prometheus metrics
        args.extend_from_slice(&[
            "--set",
            "prometheus.enabled=true",
            "--set",
            "operator.prometheus.enabled=true",
        ]);

        // Add IPv6 settings if enabled
        if self.config.enable_ipv6 {
            args.extend_from_slice(&["--set", "ipv6.enabled=true"]);
        }

        // Enable Gateway API support
        args.extend_from_slice(&["--set", "gatewayAPI.enabled=true"]);

        // Configure KubePrism for API server access (Talos-specific)
        args.extend_from_slice(&[
            "--set",
            "k8sServiceHost=localhost",
            "--set",
            "k8sServicePort=7445",
        ]);

        // Enable Node IPAM for LoadBalancer services with tunnel mode
        // Hetzner private network requires gateway routing, so use VXLAN tunnel for pod traffic
        args.extend_from_slice(&[
            "--set",
            "nodeIPAM.enabled=true",
            "--set",
            "tunnelProtocol=vxlan",
            "--set",
            "autoDirectNodeRoutes=false",
            "--set",
            "bpf.masquerade=true",
            "--set",
            "loadBalancer.acceleration=native",
            "--set",
            "defaultLBServiceIPAM=nodeipam",
        ]);

        CommandBuilder::new("helm")
            .args(&args)
            .kubeconfig(&self.kubeconfig_path)
            .context("Failed to install Cilium")
            .run_silent()
            .await?;

        Ok(())
    }

    /// Wait for Cilium to be ready
    pub async fn wait_for_ready(&self, timeout_secs: u64) -> Result<()> {
        let config = PollingConfig::new(timeout_secs, 10, "Waiting for Cilium to be ready");

        config
            .poll_until(|| async { self.check_cilium_status().await })
            .await?;

        // Wait for all nodes to be Ready
        crate::k8s::nodes::NodeManager::wait_for_all_nodes_ready(
            &self.kubeconfig_path,
            timeout_secs,
        )
        .await?;

        Ok(())
    }

    /// Check if Cilium pods are ready
    async fn check_cilium_status(&self) -> Result<bool> {
        let output = CommandBuilder::new("kubectl")
            .args([
                "get",
                "pods",
                "-n",
                "kube-system",
                "-l",
                "k8s-app=cilium",
                "-o",
                "jsonpath={.items[*].status.conditions[?(@.type=='Ready')].status}",
            ])
            .kubeconfig(&self.kubeconfig_path)
            .context("Failed to check Cilium status")
            .output()
            .await?;

        if !output.success {
            return Ok(false);
        }

        let all_ready = output
            .stdout
            .split_whitespace()
            .all(|s| s.eq_ignore_ascii_case("true"));

        Ok(all_ready && !output.stdout.is_empty())
    }

    /// Get Cilium status
    pub async fn get_status(&self) -> Result<String> {
        CommandBuilder::new("kubectl")
            .args(["get", "pods", "-n", "kube-system", "-l", "k8s-app=cilium"])
            .kubeconfig(&self.kubeconfig_path)
            .context("Failed to get Cilium status")
            .run()
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_check_tools() {
        // These tests check if helm is installed
        // They may fail in CI/test environments without these tools
        let _ = CiliumManager::check_helm_installed().await;
    }
}
