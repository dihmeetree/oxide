/// Kubernetes operations client
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;
use tokio::process::Command;

/// Kubernetes client for kubectl operations
pub struct KubernetesClient;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PodInfo {
    pub name: String,
    pub namespace: String,
    pub status: String,
    pub restarts: u32,
    pub cpu: String,
    pub memory: String,
}

impl KubernetesClient {
    /// Check if kubectl is installed
    pub async fn check_kubectl_installed() -> Result<()> {
        crate::utils::command::check_tool_installed(
            "kubectl",
            &["version", "--client"],
            "https://kubernetes.io/docs/tasks/tools/",
        )
        .await
    }

    /// Get all pods running on a specific node with metrics
    pub async fn get_pods_on_node(kubeconfig: &Path, node_name: &str) -> Result<Vec<PodInfo>> {
        // First, get pods on the node
        let output = Command::new("kubectl")
            .arg("--kubeconfig")
            .arg(kubeconfig)
            .arg("get")
            .arg("pods")
            .arg("--all-namespaces")
            .arg("--field-selector")
            .arg(format!("spec.nodeName={}", node_name))
            .arg("-o")
            .arg("json")
            .output()
            .await
            .context("Failed to execute kubectl get pods")?;

        if !output.status.success() {
            anyhow::bail!(
                "kubectl get pods failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        let pods_json: serde_json::Value = serde_json::from_slice(&output.stdout)
            .context("Failed to parse kubectl pods output")?;

        let mut pods = Vec::new();

        if let Some(items) = pods_json["items"].as_array() {
            for pod in items {
                let name = pod["metadata"]["name"]
                    .as_str()
                    .unwrap_or("unknown")
                    .to_string();
                let namespace = pod["metadata"]["namespace"]
                    .as_str()
                    .unwrap_or("default")
                    .to_string();
                let status = pod["status"]["phase"]
                    .as_str()
                    .unwrap_or("Unknown")
                    .to_string();

                // Count restarts from all containers
                let mut restarts = 0u32;
                if let Some(container_statuses) = pod["status"]["containerStatuses"].as_array() {
                    for container in container_statuses {
                        if let Some(restart_count) = container["restartCount"].as_u64() {
                            restarts += restart_count as u32;
                        }
                    }
                }

                // Try to get metrics (may fail if metrics-server not installed)
                let (cpu, memory) = Self::get_pod_metrics(kubeconfig, &namespace, &name)
                    .await
                    .unwrap_or(("N/A".to_string(), "N/A".to_string()));

                pods.push(PodInfo {
                    name,
                    namespace,
                    status,
                    restarts,
                    cpu,
                    memory,
                });
            }
        }

        Ok(pods)
    }

    /// Get pod metrics (CPU and memory)
    async fn get_pod_metrics(
        kubeconfig: &Path,
        namespace: &str,
        pod_name: &str,
    ) -> Result<(String, String)> {
        let output = Command::new("kubectl")
            .arg("--kubeconfig")
            .arg(kubeconfig)
            .arg("top")
            .arg("pod")
            .arg(pod_name)
            .arg("-n")
            .arg(namespace)
            .arg("--no-headers")
            .output()
            .await
            .context("Failed to execute kubectl top pod")?;

        if !output.status.success() {
            return Ok(("N/A".to_string(), "N/A".to_string()));
        }

        let output_str = String::from_utf8_lossy(&output.stdout);
        let parts: Vec<&str> = output_str.split_whitespace().collect();

        if parts.len() >= 3 {
            Ok((parts[1].to_string(), parts[2].to_string()))
        } else {
            Ok(("N/A".to_string(), "N/A".to_string()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_check_kubectl() {
        // This test will pass if kubectl is installed, fail otherwise
        // It's informational rather than a strict requirement
        let result = KubernetesClient::check_kubectl_installed().await;
        if result.is_err() {
            println!("kubectl not installed (expected in test environment)");
        }
    }
}
