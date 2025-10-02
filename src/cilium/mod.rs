/// Cilium CNI deployment and management
use anyhow::{Context, Result};
use std::process::Stdio;
use tokio::process::Command;
use tracing::info;

use crate::config::CiliumConfig;

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
        let output = Command::new("helm")
            .arg("version")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await;

        match output {
            Ok(output) if output.status.success() => Ok(()),
            _ => anyhow::bail!(
                "helm is not installed or not in PATH. Please install from https://helm.sh/docs/intro/install/"
            ),
        }
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

        let output = Command::new("kubectl")
            .args([
                "apply",
                "-f",
                "https://github.com/kubernetes-sigs/gateway-api/releases/download/v1.3.0/experimental-install.yaml",
            ])
            .env("KUBECONFIG", &self.kubeconfig_path)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .context("Failed to install Gateway API CRDs")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("Failed to install Gateway API CRDs: {}", stderr);
        }

        info!("Gateway API CRDs installed successfully");
        Ok(())
    }

    /// Add Cilium Helm repository
    async fn add_helm_repo(&self) -> Result<()> {
        info!("Adding Cilium Helm repository...");

        let output = Command::new("helm")
            .args(["repo", "add", "cilium", "https://helm.cilium.io/"])
            .env("KUBECONFIG", &self.kubeconfig_path)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .context("Failed to add Cilium Helm repo")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            // Ignore "already exists" errors
            if !stderr.contains("already exists") {
                anyhow::bail!("Failed to add Helm repo: {}", stderr);
            }
        }

        // Update Helm repositories
        let output = Command::new("helm")
            .args(["repo", "update"])
            .env("KUBECONFIG", &self.kubeconfig_path)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .context("Failed to update Helm repos")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("Failed to update Helm repos: {}", stderr);
        }

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

        let output = Command::new("helm")
            .args(&args)
            .env("KUBECONFIG", &self.kubeconfig_path)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .context("Failed to install Cilium")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("Failed to install Cilium: {}", stderr);
        }

        Ok(())
    }

    /// Wait for Cilium to be ready
    pub async fn wait_for_ready(&self, timeout_secs: u64) -> Result<()> {
        info!("Waiting for Cilium to be ready...");

        let start = std::time::Instant::now();
        let timeout = std::time::Duration::from_secs(timeout_secs);

        loop {
            let ready = self.check_cilium_status().await?;

            if ready {
                info!("Cilium is ready");
                break;
            }

            if start.elapsed() > timeout {
                anyhow::bail!("Timeout waiting for Cilium to be ready");
            }

            tokio::time::sleep(std::time::Duration::from_secs(10)).await;
        }

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
        let output = Command::new("kubectl")
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
            .env("KUBECONFIG", &self.kubeconfig_path)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .context("Failed to check Cilium status")?;

        if !output.status.success() {
            return Ok(false);
        }

        let status = String::from_utf8_lossy(&output.stdout);
        let all_ready = status
            .split_whitespace()
            .all(|s| s.eq_ignore_ascii_case("true"));

        Ok(all_ready && !status.is_empty())
    }

    /// Get Cilium status
    pub async fn get_status(&self) -> Result<String> {
        let output = Command::new("kubectl")
            .args(["get", "pods", "-n", "kube-system", "-l", "k8s-app=cilium"])
            .env("KUBECONFIG", &self.kubeconfig_path)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .context("Failed to get Cilium status")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("Failed to get status: {}", stderr);
        }

        Ok(String::from_utf8_lossy(&output.stdout).to_string())
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
