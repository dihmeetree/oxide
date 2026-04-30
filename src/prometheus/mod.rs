/// Prometheus monitoring stack deployment and management
use anyhow::{Context, Result};
use serde::Deserialize;
use tracing::info;

use crate::config::PrometheusConfig;
use crate::utils::command::CommandBuilder;
use crate::utils::polling::PollingConfig;

pub mod client;
pub use client::shared_client;

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

        // Generate a strong random Grafana admin password (or reuse the one we
        // stored on a previous install) so we never deploy with the well-known
        // default. The password is persisted alongside the kubeconfig with 0600
        // permissions so operators can retrieve it without scraping logs.
        let grafana_password_arg = if self.config.enable_grafana {
            let password = self.ensure_grafana_password().await?;
            Some(format!("grafana.adminPassword={}", password))
        } else {
            None
        };

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
            let pwd_arg = grafana_password_arg
                .as_deref()
                .expect("password generated when grafana enabled");
            args.extend_from_slice(&["--set", "grafana.enabled=true", "--set", pwd_arg]);

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

    /// Calculate retention size (90% of storage size).
    ///
    /// Helm rejects a `retentionSize=0GB` setting, so we always clamp to a
    /// minimum of 1GB even for tiny storage volumes.
    fn calculate_retention_size(&self) -> String {
        // Parse storage size (e.g., "50Gi" -> 50)
        let size_str = self.config.storage_size.trim_end_matches("Gi");
        if let Ok(size) = size_str.parse::<u32>() {
            let retention_size = ((size as f32) * 0.9) as u32;
            let retention_size = retention_size.max(1);
            format!("{}GB", retention_size)
        } else {
            "45GB".to_string() // Default fallback
        }
    }

    /// Path where the generated Grafana admin password is persisted.
    fn grafana_password_path(&self) -> std::path::PathBuf {
        let parent = self
            .kubeconfig_path
            .parent()
            .map(std::path::Path::to_path_buf)
            .unwrap_or_else(|| std::path::PathBuf::from("."));
        parent.join("grafana-admin-password")
    }

    /// Read an existing Grafana admin password or generate and persist a new one.
    ///
    /// The password is 24 random URL-safe base64 characters and is written with
    /// mode 0600 so it cannot be read by other users on the host.
    async fn ensure_grafana_password(&self) -> Result<String> {
        let path = self.grafana_password_path();

        if path.exists() {
            let existing = tokio::fs::read_to_string(&path)
                .await
                .with_context(|| format!("Failed to read {}", path.display()))?;
            let trimmed = existing.trim().to_string();
            if !trimmed.is_empty() {
                return Ok(trimmed);
            }
        }

        // 18 random bytes -> 24-char URL-safe base64 string. We use the OS RNG
        // (already a project dependency via rand_core/getrandom) to avoid pulling
        // in a new crate.
        use base64::Engine;
        use rand_core::RngCore;
        let mut bytes = [0u8; 18];
        rand_core::OsRng.fill_bytes(&mut bytes);
        let password = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes);

        tokio::fs::write(&path, &password)
            .await
            .with_context(|| format!("Failed to write Grafana password to {}", path.display()))?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = tokio::fs::metadata(&path).await?.permissions();
            perms.set_mode(0o600);
            tokio::fs::set_permissions(&path, perms).await?;
        }

        info!(
            "Generated random Grafana admin password and stored it at {}",
            path.display()
        );

        Ok(password)
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

        let password_path = self.grafana_password_path();
        let password_hint = if password_path.exists() {
            format!(
                "  Password: stored at {} (mode 0600)\n",
                password_path.display()
            )
        } else {
            "  Password: not yet generated (run install first)\n".to_string()
        };

        let mut info = String::new();
        info.push_str("Grafana Access Information:\n");
        info.push_str("  Username: admin\n");
        info.push_str(&password_hint);
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

        info!("[OK] Prometheus monitoring stack installed successfully!");

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

        info!("[OK] Prometheus monitoring stack uninstalled successfully!");

        Ok(())
    }
}

/// Prometheus API response for instant queries
#[derive(Debug, Deserialize)]
pub struct PrometheusResponse {
    pub status: String,
    pub data: PrometheusData,
}

#[derive(Debug, Deserialize)]
pub struct PrometheusData {
    #[serde(rename = "resultType")]
    #[allow(dead_code)]
    pub result_type: String,
    pub result: Vec<PrometheusResult>,
}

#[derive(Debug, Deserialize)]
pub struct PrometheusResult {
    #[allow(dead_code)]
    pub metric: std::collections::HashMap<String, String>,
    pub value: (f64, String),
}

/// Node metrics from Prometheus
#[derive(Debug, Clone)]
pub struct NodeMetrics {
    pub cpu_usage_percent: f64,
    pub memory_usage_percent: f64,
    pub memory_used_bytes: u64,
    pub memory_total_bytes: u64,
}

impl Default for NodeMetrics {
    fn default() -> Self {
        Self {
            cpu_usage_percent: 0.0,
            memory_usage_percent: 0.0,
            memory_used_bytes: 0,
            memory_total_bytes: 0,
        }
    }
}

/// Query Prometheus for node metrics using the node's private IP
pub async fn query_node_metrics(
    node_private_ip: &str,
    kubeconfig_path: &std::path::Path,
) -> Result<NodeMetrics> {
    let client = shared_client(kubeconfig_path).await;

    let cpu_query = format!(
        "100 * (1 - avg(rate(node_cpu_seconds_total{{mode=\"idle\",instance=~\"{}:.*\"}}[5m])))",
        node_private_ip
    );
    let mem_used_query = format!(
        "node_memory_MemTotal_bytes{{instance=~\"{}:.*\"}} - node_memory_MemAvailable_bytes{{instance=~\"{}:.*\"}}",
        node_private_ip, node_private_ip
    );
    let mem_total_query = format!(
        "node_memory_MemTotal_bytes{{instance=~\"{}:.*\"}}",
        node_private_ip
    );

    let (cpu_result, mem_used_result, mem_total_result) = tokio::join!(
        client.instant_scalar(&cpu_query),
        client.instant_scalar(&mem_used_query),
        client.instant_scalar(&mem_total_query)
    );

    let cpu_usage = cpu_result?.unwrap_or(0.0);
    let memory_used_bytes = mem_used_result?.unwrap_or(0.0) as u64;
    let memory_total_bytes = mem_total_result?.unwrap_or(0.0) as u64;

    let memory_usage_percent = if memory_total_bytes > 0 {
        (memory_used_bytes as f64 / memory_total_bytes as f64) * 100.0
    } else {
        0.0
    };

    Ok(NodeMetrics {
        cpu_usage_percent: cpu_usage,
        memory_usage_percent,
        memory_used_bytes,
        memory_total_bytes,
    })
}

/// Query Prometheus for historical metrics (range query)
pub async fn query_node_metrics_range(
    node_private_ip: &str,
    kubeconfig_path: &std::path::Path,
    duration: &str, // e.g., "1h"
    step: &str,     // e.g., "1m"
) -> Result<NodeMetricsHistory> {
    let client = shared_client(kubeconfig_path).await;
    let duration_secs = parse_duration(duration)?;

    let cpu_query = format!(
        "100 * (1 - avg(rate(node_cpu_seconds_total{{mode=\"idle\",instance=~\"{}:.*\"}}[5m])))",
        node_private_ip
    );
    let mem_query = format!(
        "100 * (1 - (node_memory_MemAvailable_bytes{{instance=~\"{}:.*\"}} / node_memory_MemTotal_bytes{{instance=~\"{}:.*\"}}))",
        node_private_ip, node_private_ip
    );

    let (cpu_history, memory_history) = tokio::join!(
        client.range_single(&cpu_query, duration_secs, step),
        client.range_single(&mem_query, duration_secs, step)
    );

    Ok(NodeMetricsHistory {
        cpu_history: cpu_history.unwrap_or_default(),
        memory_history: memory_history.unwrap_or_default(),
    })
}

/// Node metrics history
#[derive(Debug, Clone, Default)]
pub struct NodeMetricsHistory {
    pub cpu_history: Vec<(i64, f64)>,    // (timestamp, value)
    pub memory_history: Vec<(i64, f64)>, // (timestamp, value)
}

/// Prometheus range query response
#[derive(Debug, Deserialize)]
pub struct PrometheusRangeResponse {
    pub status: String,
    pub data: PrometheusRangeData,
}

#[derive(Debug, Deserialize)]
pub struct PrometheusRangeData {
    #[serde(rename = "resultType")]
    #[allow(dead_code)]
    pub result_type: String,
    pub result: Vec<PrometheusRangeResult>,
}

#[derive(Debug, Deserialize)]
pub struct PrometheusRangeResult {
    #[allow(dead_code)]
    pub metric: std::collections::HashMap<String, String>,
    pub values: Vec<(f64, String)>,
}

/// Parse duration string like "1h", "30m", "1d" to seconds
pub(crate) fn parse_duration(duration: &str) -> Result<u64> {
    let duration = duration.trim();
    if duration.is_empty() {
        anyhow::bail!("Empty duration string");
    }

    let (num_str, unit) = duration.split_at(duration.len() - 1);
    let num: u64 = num_str.parse().context("Invalid duration number")?;

    let multiplier = match unit {
        "s" => 1,
        "m" => 60,
        "h" => 3600,
        "d" => 86400,
        _ => anyhow::bail!("Invalid duration unit: {}", unit),
    };

    Ok(num * multiplier)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_calculate_retention_size() {
        let config = PrometheusConfig {
            version: "84.4.0".to_string(),
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

/// Batch-fetch CPU + memory range histories for **every** pod in one pair of
/// queries, keyed by `"namespace/pod"`. Replaces the previous per-pod fan-out
/// (N pods × 2 wget-through-kubectl calls) with exactly two HTTP requests.
pub async fn query_all_pod_metrics_range(
    kubeconfig_path: &std::path::Path,
    duration: &str,
    step: &str,
) -> Result<std::collections::HashMap<String, NodeMetricsHistory>> {
    let client = shared_client(kubeconfig_path).await;
    let duration_secs = parse_duration(duration)?;

    // `sum by (namespace, pod) (...)` collapses per-container series into one
    // series per pod with `namespace` and `pod` labels intact for keying.
    let cpu_query = "sum by (namespace, pod) (rate(container_cpu_usage_seconds_total{container!=\"\",pod!=\"\"}[5m])) * 100";
    let mem_query = "sum by (namespace, pod) (container_memory_working_set_bytes{container!=\"\",pod!=\"\"}) / 1024 / 1024";

    let (cpu_series, mem_series) = tokio::join!(
        client.range_multi(cpu_query, duration_secs, step),
        client.range_multi(mem_query, duration_secs, step)
    );

    let mut out: std::collections::HashMap<String, NodeMetricsHistory> =
        std::collections::HashMap::new();

    for (labels, values) in cpu_series.unwrap_or_default() {
        if let (Some(ns), Some(pod)) = (labels.get("namespace"), labels.get("pod")) {
            let key = format!("{}/{}", ns, pod);
            out.entry(key).or_default().cpu_history = values;
        }
    }
    for (labels, values) in mem_series.unwrap_or_default() {
        if let (Some(ns), Some(pod)) = (labels.get("namespace"), labels.get("pod")) {
            let key = format!("{}/{}", ns, pod);
            out.entry(key).or_default().memory_history = values;
        }
    }
    Ok(out)
}

/// Batch-fetch CPU + memory range histories for **every** node in one pair of
/// queries, keyed by node IP (the `instance` label without port suffix).
pub async fn query_all_node_metrics_range(
    kubeconfig_path: &std::path::Path,
    duration: &str,
    step: &str,
) -> Result<std::collections::HashMap<String, NodeMetricsHistory>> {
    let client = shared_client(kubeconfig_path).await;
    let duration_secs = parse_duration(duration)?;

    let cpu_query =
        "100 * (1 - avg by (instance) (rate(node_cpu_seconds_total{mode=\"idle\"}[5m])))";
    let mem_query = "100 * (1 - (node_memory_MemAvailable_bytes / node_memory_MemTotal_bytes))";

    let (cpu_series, mem_series) = tokio::join!(
        client.range_multi(cpu_query, duration_secs, step),
        client.range_multi(mem_query, duration_secs, step)
    );

    let mut out: std::collections::HashMap<String, NodeMetricsHistory> =
        std::collections::HashMap::new();

    for (labels, values) in cpu_series.unwrap_or_default() {
        if let Some(instance) = labels.get("instance") {
            let key = strip_port(instance).to_string();
            out.entry(key).or_default().cpu_history = values;
        }
    }
    for (labels, values) in mem_series.unwrap_or_default() {
        if let Some(instance) = labels.get("instance") {
            let key = strip_port(instance).to_string();
            out.entry(key).or_default().memory_history = values;
        }
    }
    Ok(out)
}

fn strip_port(instance: &str) -> &str {
    instance.split(':').next().unwrap_or(instance)
}

/// Alert information from Prometheus
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Alert {
    pub name: String,
    pub state: String, // firing, pending, inactive
    pub severity: String,
    pub description: String,
    pub labels: Vec<(String, String)>,
    pub active_at: Option<String>,
    pub value: Option<String>,
}

/// Query all alerts from Prometheus
pub async fn query_alerts(kubeconfig_path: &std::path::Path) -> Result<Vec<Alert>> {
    let client = shared_client(kubeconfig_path).await;
    let body = match client.get_json("/api/v1/alerts").await {
        Ok(b) => b,
        Err(e) => {
            tracing::debug!("Prometheus alerts query failed: {}", e);
            return Ok(Vec::new());
        }
    };

    #[derive(Deserialize)]
    struct AlertsResponse {
        data: AlertsData,
    }

    #[derive(Deserialize)]
    struct AlertsData {
        alerts: Vec<PrometheusAlert>,
    }

    #[derive(Deserialize)]
    struct PrometheusAlert {
        labels: std::collections::HashMap<String, String>,
        annotations: std::collections::HashMap<String, String>,
        state: String,
        #[serde(rename = "activeAt")]
        active_at: Option<String>,
        value: Option<String>,
    }

    let response: AlertsResponse =
        serde_json::from_str(&body).context("Failed to parse alerts response")?;

    let alerts: Vec<Alert> = response
        .data
        .alerts
        .into_iter()
        .map(|a| {
            let name = a
                .labels
                .get("alertname")
                .cloned()
                .unwrap_or_else(|| "Unknown".to_string());
            let severity = a
                .labels
                .get("severity")
                .cloned()
                .unwrap_or_else(|| "none".to_string());
            let description = a
                .annotations
                .get("description")
                .or_else(|| a.annotations.get("summary"))
                .cloned()
                .unwrap_or_else(|| "No description".to_string());

            // Convert HashMap to Vec of tuples for template rendering
            let labels: Vec<(String, String)> = a
                .labels
                .into_iter()
                .filter(|(k, _)| k != "alertname" && k != "severity") // Exclude already displayed fields
                .collect();

            // Format value to avoid scientific notation
            let value = a.value.and_then(|v| {
                v.parse::<f64>().ok().map(|num| {
                    if num.abs() < 0.01 {
                        format!("{:.6}", num)
                    } else if num.abs() < 1.0 {
                        format!("{:.4}", num)
                    } else if num.abs() < 100.0 {
                        format!("{:.2}", num)
                    } else {
                        format!("{:.0}", num)
                    }
                })
            });

            // Format active_at timestamp to human-readable format
            let active_at = a.active_at.and_then(|ts| {
                use chrono::{DateTime, Utc};
                DateTime::parse_from_rfc3339(&ts).ok().map(|dt| {
                    let utc = dt.with_timezone(&Utc);
                    let now = Utc::now();
                    let duration = now.signed_duration_since(utc);

                    let days = duration.num_days();
                    let hours = duration.num_hours();
                    let minutes = duration.num_minutes();
                    let seconds = duration.num_seconds();

                    let relative = if days > 0 {
                        format!("{}d {}h ago", days, hours % 24)
                    } else if hours > 0 {
                        format!("{}h {}m ago", hours, minutes % 60)
                    } else if minutes > 0 {
                        format!("{}m {}s ago", minutes, seconds % 60)
                    } else {
                        format!("{}s ago", seconds)
                    };

                    format!("{} ({})", utc.format("%Y-%m-%d %H:%M:%S UTC"), relative)
                })
            });

            Alert {
                name,
                state: a.state,
                severity,
                description,
                labels,
                active_at,
                value,
            }
        })
        .collect();

    Ok(alerts)
}

/// Envoy metrics history for L7 request metrics
#[derive(Debug, Clone, Default)]
pub struct EnvoyMetricsHistory {
    pub rps_history: Vec<(i64, f64)>, // (timestamp, requests per second)
    pub status_2xx_history: Vec<(i64, f64)>, // (timestamp, 2xx count)
    pub status_3xx_history: Vec<(i64, f64)>, // (timestamp, 3xx count)
    pub status_4xx_history: Vec<(i64, f64)>, // (timestamp, 4xx count)
    pub status_5xx_history: Vec<(i64, f64)>, // (timestamp, 5xx count)
}

/// Query Envoy metrics for a time range
pub async fn query_envoy_metrics_range(
    kubeconfig_path: &std::path::Path,
    duration: &str, // e.g., "1h"
    step: &str,     // e.g., "1m"
) -> Result<EnvoyMetricsHistory> {
    let client = shared_client(kubeconfig_path).await;
    let duration_secs = parse_duration(duration)?;

    let rps_query = "sum(rate(hubble_http_requests_total[5m]))";
    let s2xx_query = "sum(rate(hubble_http_requests_total{status=~\"2..\"}[5m]))";
    let s3xx_query = "sum(rate(hubble_http_requests_total{status=~\"3..\"}[5m]))";
    let s4xx_query = "sum(rate(hubble_http_requests_total{status=~\"4..\"}[5m]))";
    let s5xx_query = "sum(rate(hubble_http_requests_total{status=~\"5..\"}[5m]))";

    // Fire all five range queries concurrently against the shared port-forward.
    let (rps, s2xx, s3xx, s4xx, s5xx) = tokio::join!(
        client.range_single(rps_query, duration_secs, step),
        client.range_single(s2xx_query, duration_secs, step),
        client.range_single(s3xx_query, duration_secs, step),
        client.range_single(s4xx_query, duration_secs, step),
        client.range_single(s5xx_query, duration_secs, step),
    );

    Ok(EnvoyMetricsHistory {
        rps_history: rps.unwrap_or_default(),
        status_2xx_history: s2xx.unwrap_or_default(),
        status_3xx_history: s3xx.unwrap_or_default(),
        status_4xx_history: s4xx.unwrap_or_default(),
        status_5xx_history: s5xx.unwrap_or_default(),
    })
}
