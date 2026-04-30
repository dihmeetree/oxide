/// Cilium CNI deployment and management
use anyhow::{Context, Result};
use tracing::info;

use crate::config::CiliumConfig;
use crate::utils::command::CommandBuilder;
use crate::utils::polling::PollingConfig;

/// Cilium deployment manager
pub struct Cilium {
    config: CiliumConfig,
    kubeconfig_path: std::path::PathBuf,
    control_plane_count: u32,
}

impl Cilium {
    /// Create a new Cilium manager
    pub const fn new(
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

    /// Install Cilium CNI using Helm
    pub async fn install(&self) -> Result<()> {
        info!("Installing Cilium CNI version {}...", self.config.version);

        // Install Gateway API CRDs first
        self.install_gateway_api_crds().await?;

        // Add Cilium Helm repository
        self.add_helm_repo().await?;

        // Install Cilium
        self.install_cilium_chart().await?;

        // Configure CoreDNS to use public DNS servers instead of Talos hostDNS
        self.configure_coredns_public_dns().await?;

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
        let operator_replicas_arg = format!("operator.replicas={operator_replicas}");

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

    /// Configure CoreDNS to use Hetzner private DNS servers
    /// This is required because Cilium's VXLAN tunneling prevents pods from reaching Talos Host DNS (169.254.x.x)
    /// We use Hetzner's private DNS servers which are accessible from the pod network
    async fn configure_coredns_public_dns(&self) -> Result<()> {
        info!("Waiting for CoreDNS to be deployed...");

        // Wait for CoreDNS ConfigMap to exist (up to 5 minutes)
        let config = PollingConfig::new(300, 5, "Waiting for CoreDNS ConfigMap");
        config
            .poll_until(|| async {
                let result = CommandBuilder::new("kubectl")
                    .args(["get", "configmap", "coredns", "-n", "kube-system"])
                    .kubeconfig(&self.kubeconfig_path)
                    .run_silent()
                    .await;

                // Return true if ConfigMap exists, false if not found (keep polling)
                Ok(result.is_ok())
            })
            .await?;

        info!("Configuring CoreDNS to use Hetzner private DNS servers...");

        let coredns_config = r"
data:
  Corefile: |
    .:53 {
        errors
        health {
            lameduck 5s
        }
        ready
        log . {
            class error
        }
        prometheus :9153

        kubernetes cluster.local in-addr.arpa ip6.arpa {
            pods insecure
            fallthrough in-addr.arpa ip6.arpa
            ttl 30
        }
        forward . 185.12.64.1 185.12.64.2 {
           max_concurrent 1000
        }
        cache 30 {
           disable success cluster.local
           disable denial cluster.local
        }
        loop
        reload
        loadbalance
    }
";

        // Stage the patch in a unique temp file so concurrent runs cannot
        // clobber each other and a local attacker cannot pre-create a symlink
        // at a predictable path. The `NamedTempFile` is removed automatically
        // when it goes out of scope, even on failure.
        let temp_file = tempfile::Builder::new()
            .prefix("oxide-coredns-patch-")
            .suffix(".yaml")
            .tempfile()
            .context("Failed to create temp file for CoreDNS patch")?;
        tokio::fs::write(temp_file.path(), coredns_config).await?;

        // Convert path to string with error handling
        let temp_file_str = temp_file
            .path()
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("Temp file path contains invalid UTF-8"))?;

        // Apply patch to CoreDNS ConfigMap
        CommandBuilder::new("kubectl")
            .args([
                "patch",
                "configmap",
                "coredns",
                "-n",
                "kube-system",
                "--patch-file",
                temp_file_str,
            ])
            .kubeconfig(&self.kubeconfig_path)
            .context("Failed to patch CoreDNS ConfigMap")
            .run_silent()
            .await?;

        // Tempfile is closed/removed when `temp_file` goes out of scope.
        drop(temp_file);

        // Restart CoreDNS to apply changes
        info!("Restarting CoreDNS to apply configuration...");
        CommandBuilder::new("kubectl")
            .args([
                "rollout",
                "restart",
                "deployment",
                "coredns",
                "-n",
                "kube-system",
            ])
            .kubeconfig(&self.kubeconfig_path)
            .context("Failed to restart CoreDNS")
            .run_silent()
            .await?;

        // Wait for CoreDNS rollout to complete
        CommandBuilder::new("kubectl")
            .args([
                "rollout",
                "status",
                "deployment",
                "coredns",
                "-n",
                "kube-system",
                "--timeout=180s",
            ])
            .kubeconfig(&self.kubeconfig_path)
            .context("CoreDNS rollout timeout")
            .run_silent()
            .await?;

        info!("CoreDNS configured successfully");
        Ok(())
    }

    /// Configure Cilium monitoring by applying PodMonitor resources
    /// This enables Prometheus to scrape metrics from Cilium agent, operator, and Hubble
    pub async fn configure_monitoring(&self) -> Result<()> {
        info!("Configuring Cilium monitoring...");

        let manifest_dir = std::path::Path::new("manifests/cilium");

        if !manifest_dir.exists() {
            info!("Cilium monitoring manifests not found, skipping");
            return Ok(());
        }

        // Apply all Cilium monitoring manifests (PodMonitors and Grafana dashboards)
        for entry in std::fs::read_dir(manifest_dir)? {
            let entry = entry?;
            let path = entry.path();

            if path.extension().and_then(|s| s.to_str()) == Some("yaml") {
                crate::k8s::resources::Resources::apply_manifest(&self.kubeconfig_path, &path)
                    .await?;
            }
        }

        info!("Cilium monitoring configured successfully");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::helm::Helm;

    #[tokio::test]
    async fn test_check_tools() {
        // These tests check if helm is installed
        // They may fail in CI/test environments without these tools
        let _ = Helm::check_installed().await;
    }
}
