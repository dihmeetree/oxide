/// Prometheus monitoring stack deployment and management
use anyhow::{Context, Result};
use tracing::info;

use crate::config::PrometheusConfig;
use crate::utils::command::CommandBuilder;
use crate::utils::polling::PollingConfig;

/// Prometheus deployment manager
pub struct Prometheus {
    config: PrometheusConfig,
    kubeconfig_path: std::path::PathBuf,
}

impl Prometheus {
    /// Create a new Prometheus manager
    pub fn new(config: PrometheusConfig, kubeconfig_path: std::path::PathBuf) -> Self {
        Self {
            config,
            kubeconfig_path,
        }
    }

    /// Install Prometheus stack using Helm (kube-prometheus-stack)
    pub async fn install_stack(&self) -> Result<()> {
        if !self.config.enabled {
            info!("Prometheus is disabled in configuration, skipping installation");
            return Ok(());
        }

        info!(
            "Installing Prometheus stack (kube-prometheus-stack) version {}...",
            self.config.version
        );

        // Create namespace
        self.create_namespace().await?;

        // Add Prometheus community Helm repository
        self.add_helm_repo().await?;

        // Install kube-prometheus-stack chart
        self.install_prometheus_chart().await?;

        info!("Prometheus stack installed successfully");

        Ok(())
    }

    /// Create monitoring namespace
    async fn create_namespace(&self) -> Result<()> {
        info!("Creating namespace '{}'...", self.config.namespace);

        let output = CommandBuilder::new("kubectl")
            .args(["create", "namespace", &self.config.namespace])
            .kubeconfig(&self.kubeconfig_path)
            .context("Failed to create namespace")
            .output()
            .await?;

        if !output.success {
            // Ignore "already exists" errors
            if !output.stderr.contains("already exists") {
                anyhow::bail!("Failed to create namespace: {}", output.stderr);
            }
        }

        // Label namespace for privileged pod security (required for node-exporter)
        info!(
            "Labeling namespace '{}' for privileged pod security...",
            self.config.namespace
        );

        CommandBuilder::new("kubectl")
            .args([
                "label",
                "namespace",
                &self.config.namespace,
                "pod-security.kubernetes.io/enforce=privileged",
                "--overwrite",
            ])
            .kubeconfig(&self.kubeconfig_path)
            .context("Failed to label namespace")
            .run_silent()
            .await?;

        info!("Namespace '{}' created successfully", self.config.namespace);
        Ok(())
    }

    /// Add Prometheus community Helm repository
    async fn add_helm_repo(&self) -> Result<()> {
        info!("Adding Prometheus community Helm repository...");

        let output = CommandBuilder::new("helm")
            .args([
                "repo",
                "add",
                "prometheus-community",
                "https://prometheus-community.github.io/helm-charts",
            ])
            .kubeconfig(&self.kubeconfig_path)
            .context("Failed to add Prometheus Helm repo")
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

    /// Install kube-prometheus-stack Helm chart
    async fn install_prometheus_chart(&self) -> Result<()> {
        info!("Installing kube-prometheus-stack Helm chart...");

        // Pre-calculate formatted strings to avoid lifetime issues
        let retention_size_arg = format!(
            "prometheus.prometheusSpec.retentionSize={}",
            self.calculate_retention_size()
        );
        let storage_arg = format!(
            "prometheus.prometheusSpec.storageSpec.volumeClaimTemplate.spec.resources.requests.storage={}",
            self.config.storage_size
        );

        let mut args = vec![
            "install",
            "prometheus",
            "prometheus-community/kube-prometheus-stack",
            "--version",
            &self.config.version,
            "--namespace",
            &self.config.namespace,
            "--create-namespace",
        ];

        // Prometheus configuration
        args.extend_from_slice(&[
            "--set",
            "prometheus.prometheusSpec.retention=30d",
            "--set",
            &retention_size_arg,
        ]);

        // Storage configuration
        if self.config.enable_persistent_storage {
            args.extend_from_slice(&["--set", &storage_arg]);
        }

        // Grafana configuration
        if self.config.enable_grafana {
            args.extend_from_slice(&[
                "--set",
                "grafana.enabled=true",
                "--set",
                "grafana.adminPassword=admin",
            ]);

            if self.config.enable_persistent_storage {
                args.extend_from_slice(&[
                    "--set",
                    "grafana.persistence.enabled=true",
                    "--set",
                    "grafana.persistence.size=10Gi",
                ]);
            } else {
                args.extend_from_slice(&["--set", "grafana.persistence.enabled=false"]);
            }
        } else {
            args.extend_from_slice(&["--set", "grafana.enabled=false"]);
        }

        // AlertManager configuration
        if self.config.enable_alertmanager {
            args.extend_from_slice(&["--set", "alertmanager.enabled=true"]);

            if self.config.enable_persistent_storage {
                args.extend_from_slice(&[
                    "--set",
                    "alertmanager.alertmanagerSpec.storage.volumeClaimTemplate.spec.resources.requests.storage=10Gi",
                ]);
            }
        } else {
            args.extend_from_slice(&["--set", "alertmanager.enabled=false"]);
        }

        // Enable service monitors for common components
        args.extend_from_slice(&[
            "--set",
            "prometheus.prometheusSpec.serviceMonitorSelectorNilUsesHelmValues=false",
            "--set",
            "prometheus.prometheusSpec.podMonitorSelectorNilUsesHelmValues=false",
        ]);

        // Enable Cilium service monitors if available
        args.extend_from_slice(&[
            "--set",
            "prometheus.prometheusSpec.additionalScrapeConfigs[0].job_name=cilium-agent",
            "--set",
            "prometheus.prometheusSpec.additionalScrapeConfigs[0].kubernetes_sd_configs[0].role=pod",
            "--set",
            "prometheus.prometheusSpec.additionalScrapeConfigs[0].kubernetes_sd_configs[0].namespaces.names[0]=kube-system",
            "--set",
            "prometheus.prometheusSpec.additionalScrapeConfigs[0].relabel_configs[0].source_labels[0]=__meta_kubernetes_pod_label_k8s_app",
            "--set",
            "prometheus.prometheusSpec.additionalScrapeConfigs[0].relabel_configs[0].action=keep",
            "--set",
            "prometheus.prometheusSpec.additionalScrapeConfigs[0].relabel_configs[0].regex=cilium",
        ]);

        CommandBuilder::new("helm")
            .args(&args)
            .kubeconfig(&self.kubeconfig_path)
            .context("Failed to install Prometheus stack")
            .run_silent()
            .await?;

        Ok(())
    }

    /// Calculate retention size (90% of storage size)
    fn calculate_retention_size(&self) -> String {
        // Parse storage size (e.g., "50Gi" -> 50)
        let size_str = self.config.storage_size.trim_end_matches("Gi");
        if let Ok(size) = size_str.parse::<u32>() {
            let retention_size = (size as f32 * 0.9) as u32;
            format!("{}GB", retention_size)
        } else {
            "45GB".to_string() // Default fallback
        }
    }

    /// Wait for Prometheus stack to be ready
    pub async fn wait_for_ready(&self, timeout_secs: u64) -> Result<()> {
        if !self.config.enabled {
            return Ok(());
        }

        info!("Waiting for Prometheus stack to be ready...");

        let config =
            PollingConfig::new(timeout_secs, 10, "Waiting for Prometheus stack to be ready");

        config
            .poll_until(|| async { self.check_prometheus_status().await })
            .await?;

        info!("Prometheus stack is ready");

        Ok(())
    }

    /// Check if Prometheus pods are ready
    async fn check_prometheus_status(&self) -> Result<bool> {
        // Check Prometheus operator
        let operator_ready = self
            .check_pods_ready("app.kubernetes.io/name=kube-prometheus-stack-prometheus-operator")
            .await?;

        if !operator_ready {
            return Ok(false);
        }

        // Check Prometheus server
        let prometheus_ready = self
            .check_pods_ready("app.kubernetes.io/name=prometheus")
            .await?;

        if !prometheus_ready {
            return Ok(false);
        }

        // Check Grafana if enabled
        if self.config.enable_grafana {
            let grafana_ready = self
                .check_pods_ready("app.kubernetes.io/name=grafana")
                .await?;
            if !grafana_ready {
                return Ok(false);
            }
        }

        Ok(true)
    }

    /// Check if pods with given label selector are ready
    async fn check_pods_ready(&self, label_selector: &str) -> Result<bool> {
        let output = CommandBuilder::new("kubectl")
            .args([
                "get",
                "pods",
                "-n",
                &self.config.namespace,
                "-l",
                label_selector,
                "-o",
                "jsonpath={.items[*].status.conditions[?(@.type=='Ready')].status}",
            ])
            .kubeconfig(&self.kubeconfig_path)
            .context(format!("Failed to check pod status for {}", label_selector))
            .output()
            .await?;

        if !output.success {
            return Ok(false);
        }

        if output.stdout.is_empty() {
            return Ok(false);
        }

        let all_ready = output
            .stdout
            .split_whitespace()
            .all(|s| s.eq_ignore_ascii_case("true"));

        Ok(all_ready)
    }

    /// Get Prometheus stack status
    pub async fn get_status(&self) -> Result<String> {
        if !self.config.enabled {
            return Ok("Prometheus is disabled".to_string());
        }

        CommandBuilder::new("kubectl")
            .args(["get", "pods", "-n", &self.config.namespace])
            .kubeconfig(&self.kubeconfig_path)
            .context("Failed to get Prometheus status")
            .run()
            .await
    }

    /// Get Grafana access information
    pub async fn get_grafana_info(&self) -> Result<String> {
        if !self.config.enabled || !self.config.enable_grafana {
            return Ok("Grafana is disabled".to_string());
        }

        let mut info = String::new();
        info.push_str("Grafana Access Information:\n");
        info.push_str("  Username: admin\n");
        info.push_str("  Password: admin (change this after first login!)\n");
        info.push_str("\nTo access Grafana:\n");
        info.push_str(&format!(
            "  kubectl port-forward -n {} svc/prometheus-grafana 3000:80\n",
            self.config.namespace
        ));
        info.push_str("  Then open: http://localhost:3000\n");

        Ok(info)
    }

    /// Uninstall Prometheus stack
    pub async fn uninstall_stack(&self) -> Result<()> {
        info!("Uninstalling Prometheus stack...");

        CommandBuilder::new("helm")
            .args([
                "uninstall",
                "prometheus",
                "--namespace",
                &self.config.namespace,
            ])
            .kubeconfig(&self.kubeconfig_path)
            .context("Failed to uninstall Prometheus")
            .run_silent()
            .await?;

        info!("Prometheus stack uninstalled successfully");

        Ok(())
    }

    /// Install Prometheus
    pub async fn install(
        config_path: &std::path::Path,
        output_dir: &std::path::Path,
    ) -> Result<()> {
        use crate::config::ClusterConfig;

        info!("Installing Prometheus monitoring stack...");

        let config =
            ClusterConfig::from_file(config_path).context("Failed to load configuration")?;

        let prometheus_config = config.prometheus.ok_or_else(|| {
            anyhow::anyhow!("Prometheus configuration not found in cluster config")
        })?;

        let kubeconfig_path = output_dir.join("kubeconfig");
        if !kubeconfig_path.exists() {
            anyhow::bail!(
                "Kubeconfig not found at {}. Please create the cluster first.",
                kubeconfig_path.display()
            );
        }

        let prometheus = Self::new(prometheus_config.clone(), kubeconfig_path);

        prometheus.install_stack().await?;
        prometheus.wait_for_ready(600).await?;

        info!("✓ Prometheus monitoring stack installed successfully!");
        info!("");

        if prometheus_config.enable_grafana {
            let grafana_info = prometheus.get_grafana_info().await?;
            info!("{}", grafana_info);
        }

        info!("To check Prometheus status:");
        info!("  oxide prometheus-status");

        Ok(())
    }

    /// Show Prometheus status
    pub async fn status(config_path: &std::path::Path, output_dir: &std::path::Path) -> Result<()> {
        use crate::config::ClusterConfig;

        let config =
            ClusterConfig::from_file(config_path).context("Failed to load configuration")?;

        let prometheus_config = config.prometheus.ok_or_else(|| {
            anyhow::anyhow!("Prometheus configuration not found in cluster config")
        })?;

        let kubeconfig_path = output_dir.join("kubeconfig");
        if !kubeconfig_path.exists() {
            anyhow::bail!(
                "Kubeconfig not found at {}. Please create the cluster first.",
                kubeconfig_path.display()
            );
        }

        let prometheus = Self::new(prometheus_config.clone(), kubeconfig_path);

        let status = prometheus.get_status().await?;
        info!("Prometheus Status:");
        info!("{}", status);

        if prometheus_config.enable_grafana {
            info!("");
            let grafana_info = prometheus.get_grafana_info().await?;
            info!("{}", grafana_info);
        }

        Ok(())
    }

    /// Uninstall Prometheus
    pub async fn uninstall(
        config_path: &std::path::Path,
        output_dir: &std::path::Path,
    ) -> Result<()> {
        use crate::config::ClusterConfig;

        info!("Uninstalling Prometheus monitoring stack...");

        let config =
            ClusterConfig::from_file(config_path).context("Failed to load configuration")?;

        let prometheus_config = config.prometheus.ok_or_else(|| {
            anyhow::anyhow!("Prometheus configuration not found in cluster config")
        })?;

        let kubeconfig_path = output_dir.join("kubeconfig");
        if !kubeconfig_path.exists() {
            anyhow::bail!(
                "Kubeconfig not found at {}. Please create the cluster first.",
                kubeconfig_path.display()
            );
        }

        let prometheus = Self::new(prometheus_config, kubeconfig_path);
        prometheus.uninstall_stack().await?;

        info!("✓ Prometheus monitoring stack uninstalled successfully!");

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_calculate_retention_size() {
        let config = PrometheusConfig {
            version: "65.8.1".to_string(),
            enabled: true,
            namespace: "monitoring".to_string(),
            enable_grafana: true,
            enable_alertmanager: true,
            retention: "30d".to_string(),
            storage_size: "50Gi".to_string(),
            enable_persistent_storage: true,
            helm_values: serde_yaml::Value::Null,
        };

        let prometheus = Prometheus::new(config, std::path::PathBuf::from("test"));
        assert_eq!(prometheus.calculate_retention_size(), "45GB");
    }
}
