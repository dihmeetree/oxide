/// Prometheus monitoring stack deployment and management
use anyhow::{Context, Result};
use serde::Deserialize;
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
    // Get the Prometheus pod name
    let output = CommandBuilder::new("kubectl")
        .args([
            "get",
            "pods",
            "-n",
            "monitoring",
            "-l",
            "app.kubernetes.io/name=prometheus",
            "-o",
            "jsonpath={.items[0].metadata.name}",
        ])
        .kubeconfig(kubeconfig_path)
        .context("Failed to get Prometheus pod name")
        .output()
        .await?;

    if !output.success || output.stdout.is_empty() {
        return Ok(NodeMetrics::default());
    }

    let pod_name = output.stdout.trim();

    // Query CPU usage (percentage of allocatable CPU)
    let cpu_query = format!(
        "100 * (1 - avg(rate(node_cpu_seconds_total{{mode=\"idle\",instance=~\"{}:.*\"}}[5m])))",
        node_private_ip
    );

    // OPTIMIZATION: Query all metrics in parallel (3× faster!)
    let mem_used_query = format!(
        "node_memory_MemTotal_bytes{{instance=~\"{}:.*\"}} - node_memory_MemAvailable_bytes{{instance=~\"{}:.*\"}}",
        node_private_ip, node_private_ip
    );
    let mem_total_query = format!(
        "node_memory_MemTotal_bytes{{instance=~\"{}:.*\"}}",
        node_private_ip
    );

    let (cpu_result, mem_used_result, mem_total_result) = tokio::join!(
        query_prometheus(pod_name, &cpu_query, kubeconfig_path),
        query_prometheus(pod_name, &mem_used_query, kubeconfig_path),
        query_prometheus(pod_name, &mem_total_query, kubeconfig_path)
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

/// Execute a Prometheus query
async fn query_prometheus(
    pod_name: &str,
    query: &str,
    kubeconfig_path: &std::path::Path,
) -> Result<Option<f64>> {
    let url = format!(
        "http://localhost:9090/api/v1/query?query={}",
        urlencoding::encode(query)
    );

    let output = CommandBuilder::new("kubectl")
        .args([
            "exec",
            "-n",
            "monitoring",
            pod_name,
            "-c",
            "prometheus",
            "--",
            "wget",
            "-qO-",
            &url,
        ])
        .kubeconfig(kubeconfig_path)
        .context("Failed to query Prometheus")
        .output()
        .await?;

    if !output.success {
        return Ok(None);
    }

    let response: PrometheusResponse =
        serde_json::from_str(&output.stdout).context("Failed to parse Prometheus response")?;

    if response.status != "success" || response.data.result.is_empty() {
        return Ok(None);
    }

    let value = response.data.result[0]
        .value
        .1
        .parse::<f64>()
        .unwrap_or(0.0);

    Ok(Some(value))
}

/// Query Prometheus for historical metrics (range query)
pub async fn query_node_metrics_range(
    node_private_ip: &str,
    kubeconfig_path: &std::path::Path,
    duration: &str, // e.g., "1h"
    step: &str,     // e.g., "1m"
) -> Result<NodeMetricsHistory> {
    // Get the Prometheus pod name
    let output = CommandBuilder::new("kubectl")
        .args([
            "get",
            "pods",
            "-n",
            "monitoring",
            "-l",
            "app.kubernetes.io/name=prometheus",
            "-o",
            "jsonpath={.items[0].metadata.name}",
        ])
        .kubeconfig(kubeconfig_path)
        .context("Failed to get Prometheus pod name")
        .output()
        .await?;

    if !output.success || output.stdout.is_empty() {
        return Ok(NodeMetricsHistory::default());
    }

    let pod_name = output.stdout.trim();

    // Query CPU usage history
    let cpu_query = format!(
        "100 * (1 - avg(rate(node_cpu_seconds_total{{mode=\"idle\",instance=~\"{}:.*\"}}[5m])))",
        node_private_ip
    );

    let cpu_history =
        query_prometheus_range(pod_name, &cpu_query, duration, step, kubeconfig_path).await?;

    // Query memory usage history
    let mem_query = format!(
        "100 * (1 - (node_memory_MemAvailable_bytes{{instance=~\"{}:.*\"}} / node_memory_MemTotal_bytes{{instance=~\"{}:.*\"}}))",
        node_private_ip, node_private_ip
    );

    let memory_history =
        query_prometheus_range(pod_name, &mem_query, duration, step, kubeconfig_path).await?;

    Ok(NodeMetricsHistory {
        cpu_history,
        memory_history,
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

/// Execute a Prometheus range query
async fn query_prometheus_range(
    pod_name: &str,
    query: &str,
    duration: &str,
    step: &str,
    kubeconfig_path: &std::path::Path,
) -> Result<Vec<(i64, f64)>> {
    // Calculate start and end times (end=now, start=now-duration)
    let end = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let duration_secs = parse_duration(duration)?;
    let start = end - duration_secs;

    let url = format!(
        "http://localhost:9090/api/v1/query_range?query={}&start={}&end={}&step={}",
        urlencoding::encode(query),
        start,
        end,
        step
    );

    let output = CommandBuilder::new("kubectl")
        .args([
            "exec",
            "-n",
            "monitoring",
            pod_name,
            "-c",
            "prometheus",
            "--",
            "wget",
            "-qO-",
            &url,
        ])
        .kubeconfig(kubeconfig_path)
        .context("Failed to query Prometheus range")
        .output()
        .await?;

    if !output.success {
        return Ok(vec![]);
    }

    let response: PrometheusRangeResponse = serde_json::from_str(&output.stdout)
        .context("Failed to parse Prometheus range response")?;

    if response.status != "success" || response.data.result.is_empty() {
        return Ok(vec![]);
    }

    let values: Vec<(i64, f64)> = response.data.result[0]
        .values
        .iter()
        .map(|(timestamp, value)| {
            let ts = *timestamp as i64;
            let val = value.parse::<f64>().unwrap_or(0.0);
            (ts, val)
        })
        .collect();

    Ok(values)
}

/// Parse duration string like "1h", "30m", "1d" to seconds
fn parse_duration(duration: &str) -> Result<u64> {
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

/// Query pod metrics history from Prometheus
pub async fn query_pod_metrics_range(
    namespace: &str,
    pod_name: &str,
    kubeconfig_path: &std::path::Path,
    duration: &str, // e.g., "1h"
    step: &str,     // e.g., "1m"
) -> Result<NodeMetricsHistory> {
    // Get the Prometheus pod name
    let output = CommandBuilder::new("kubectl")
        .args([
            "get",
            "pods",
            "-n",
            "monitoring",
            "-l",
            "app.kubernetes.io/name=prometheus",
            "-o",
            "jsonpath={.items[0].metadata.name}",
        ])
        .kubeconfig(kubeconfig_path)
        .context("Failed to get Prometheus pod name")
        .output()
        .await?;

    if !output.success || output.stdout.is_empty() {
        return Ok(NodeMetricsHistory::default());
    }

    let prom_pod_name = output.stdout.trim();

    // Query CPU usage history (rate of cpu usage for the pod)
    let cpu_query = format!(
        "sum(rate(container_cpu_usage_seconds_total{{namespace=\"{}\",pod=\"{}\",container!=\"\"}}[5m])) * 100",
        namespace, pod_name
    );

    let cpu_history =
        query_prometheus_range(prom_pod_name, &cpu_query, duration, step, kubeconfig_path).await?;

    // Query memory usage history (in bytes)
    let memory_query = format!(
        "sum(container_memory_working_set_bytes{{namespace=\"{}\",pod=\"{}\",container!=\"\"}}) / 1024 / 1024",
        namespace, pod_name
    );

    let memory_history = query_prometheus_range(
        prom_pod_name,
        &memory_query,
        duration,
        step,
        kubeconfig_path,
    )
    .await?;

    Ok(NodeMetricsHistory {
        cpu_history,
        memory_history,
    })
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
    // Get the Prometheus pod name
    let output = CommandBuilder::new("kubectl")
        .args([
            "get",
            "pods",
            "-n",
            "monitoring",
            "-l",
            "app.kubernetes.io/name=prometheus",
            "-o",
            "jsonpath={.items[0].metadata.name}",
        ])
        .kubeconfig(kubeconfig_path)
        .context("Failed to get Prometheus pod name")
        .output()
        .await?;

    if !output.success || output.stdout.is_empty() {
        return Ok(Vec::new());
    }

    let prom_pod_name = output.stdout.trim();

    // Query alerts API
    let alert_output = CommandBuilder::new("kubectl")
        .args([
            "exec",
            "-n",
            "monitoring",
            prom_pod_name,
            "--",
            "wget",
            "-q",
            "-O-",
            "http://localhost:9090/api/v1/alerts",
        ])
        .kubeconfig(kubeconfig_path)
        .context("Failed to query Prometheus alerts")
        .output()
        .await?;

    if !alert_output.success {
        tracing::debug!(
            "Prometheus alerts query failed: {}",
            alert_output.stderr.trim()
        );
        return Ok(Vec::new());
    }

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
        serde_json::from_str(&alert_output.stdout).context("Failed to parse alerts response")?;

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
    // Get the Prometheus pod name
    let output = CommandBuilder::new("kubectl")
        .args([
            "get",
            "pods",
            "-n",
            "monitoring",
            "-l",
            "app.kubernetes.io/name=prometheus",
            "-o",
            "jsonpath={.items[0].metadata.name}",
        ])
        .kubeconfig(kubeconfig_path)
        .context("Failed to get Prometheus pod name")
        .output()
        .await?;

    if !output.success || output.stdout.is_empty() {
        return Ok(EnvoyMetricsHistory::default());
    }

    let pod_name = output.stdout.trim();

    // Query total requests per second using Hubble metrics
    let rps_query = "sum(rate(hubble_http_requests_total[5m]))";
    let rps_history =
        query_prometheus_range(pod_name, rps_query, duration, step, kubeconfig_path).await?;

    // Query 2xx responses per second
    let status_2xx_query = "sum(rate(hubble_http_requests_total{status=~\"2..\"}[5m]))";
    let status_2xx_history =
        query_prometheus_range(pod_name, status_2xx_query, duration, step, kubeconfig_path).await?;

    // Query 3xx responses per second
    let status_3xx_query = "sum(rate(hubble_http_requests_total{status=~\"3..\"}[5m]))";
    let status_3xx_history =
        query_prometheus_range(pod_name, status_3xx_query, duration, step, kubeconfig_path).await?;

    // Query 4xx responses per second
    let status_4xx_query = "sum(rate(hubble_http_requests_total{status=~\"4..\"}[5m]))";
    let status_4xx_history =
        query_prometheus_range(pod_name, status_4xx_query, duration, step, kubeconfig_path).await?;

    // Query 5xx responses per second
    let status_5xx_query = "sum(rate(hubble_http_requests_total{status=~\"5..\"}[5m]))";
    let status_5xx_history =
        query_prometheus_range(pod_name, status_5xx_query, duration, step, kubeconfig_path).await?;

    Ok(EnvoyMetricsHistory {
        rps_history,
        status_2xx_history,
        status_3xx_history,
        status_4xx_history,
        status_5xx_history,
    })
}
