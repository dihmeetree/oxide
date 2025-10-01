/// Kubernetes node operations
use anyhow::{Context, Result};
use std::path::Path;
use std::process::Stdio;
use tokio::process::Command;
use tracing::info;

/// Kubernetes node management operations
pub struct NodeManager;

impl NodeManager {
    /// Delete a Kubernetes node
    pub async fn delete_node(kubeconfig_path: &Path, node_name: &str) -> Result<()> {
        info!("Deleting Kubernetes node: {}", node_name);

        let output = Command::new("kubectl")
            .args(["delete", "node", node_name])
            .env("KUBECONFIG", kubeconfig_path)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .context("Failed to delete Kubernetes node")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            // Don't fail if node doesn't exist
            if stderr.contains("NotFound") || stderr.contains("not found") {
                info!(
                    "Node {} not found in Kubernetes (already removed)",
                    node_name
                );
                return Ok(());
            }
            anyhow::bail!("Failed to delete node {}: {}", node_name, stderr);
        }

        info!("Kubernetes node {} deleted successfully", node_name);

        Ok(())
    }

    /// Wait for a Kubernetes node to become Ready
    pub async fn wait_for_node_ready(
        kubeconfig_path: &Path,
        node_name: &str,
        timeout_secs: u64,
    ) -> Result<()> {
        info!("Waiting for node {} to become Ready...", node_name);

        let start = std::time::Instant::now();
        let timeout = std::time::Duration::from_secs(timeout_secs);

        loop {
            let output = Command::new("kubectl")
                .args([
                    "get",
                    "node",
                    node_name,
                    "-o",
                    "jsonpath={.status.conditions[?(@.type=='Ready')].status}",
                ])
                .env("KUBECONFIG", kubeconfig_path)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .output()
                .await;

            if let Ok(output) = output {
                if output.status.success() {
                    let status = String::from_utf8_lossy(&output.stdout);
                    if status.trim().eq_ignore_ascii_case("true") {
                        info!("✓ Node {} is Ready", node_name);
                        return Ok(());
                    }
                }
            }

            if start.elapsed() > timeout {
                anyhow::bail!(
                    "Timeout waiting for node {} to become Ready after {} seconds",
                    node_name,
                    timeout_secs
                );
            }

            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        }
    }

    /// Wait for all Kubernetes nodes to be Ready
    pub async fn wait_for_all_nodes_ready(kubeconfig_path: &Path, timeout_secs: u64) -> Result<()> {
        info!("Waiting for all nodes to be Ready...");

        // Get list of all node names
        let output = Command::new("kubectl")
            .args(["get", "nodes", "-o", "jsonpath={.items[*].metadata.name}"])
            .env("KUBECONFIG", kubeconfig_path)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .context("Failed to get node names")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("Failed to get node names: {}", stderr);
        }

        let node_names = String::from_utf8_lossy(&output.stdout);
        let nodes: Vec<&str> = node_names.split_whitespace().collect();

        if nodes.is_empty() {
            anyhow::bail!("No nodes found in cluster");
        }

        // Wait for each node to be Ready
        for node_name in nodes {
            Self::wait_for_node_ready(kubeconfig_path, node_name, timeout_secs).await?;
        }

        info!("All nodes are Ready");
        Ok(())
    }

    /// Wait for a node to be cordoned (SchedulingDisabled) and NotReady
    /// This is used during graceful node removal to ensure the node has been properly cordoned and is shutting down
    pub async fn wait_for_node_cordoned(
        kubeconfig_path: &Path,
        node_name: &str,
        timeout_secs: u64,
    ) -> Result<()> {
        info!(
            "Waiting for node {} to be cordoned and NotReady...",
            node_name
        );

        let start = std::time::Instant::now();
        let timeout = std::time::Duration::from_secs(timeout_secs);

        loop {
            // Check both spec.unschedulable and Ready condition status
            let output = Command::new("kubectl")
                .args([
                    "get",
                    "node",
                    node_name,
                    "-o",
                    "jsonpath={.spec.unschedulable},{.status.conditions[?(@.type=='Ready')].status}",
                ])
                .env("KUBECONFIG", kubeconfig_path)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .output()
                .await;

            if let Ok(output) = output {
                if output.status.success() {
                    let result = String::from_utf8_lossy(&output.stdout);
                    let parts: Vec<&str> = result.trim().split(',').collect();

                    if parts.len() == 2 {
                        let unschedulable = parts[0];
                        let ready_status = parts[1];

                        // Node should be unschedulable=true (SchedulingDisabled) AND Ready=False (NotReady)
                        if unschedulable == "true" && ready_status.eq_ignore_ascii_case("false") {
                            info!(
                                "✓ Node {} is cordoned and NotReady (NotReady,SchedulingDisabled)",
                                node_name
                            );
                            return Ok(());
                        }
                    }
                } else {
                    // Node might have been deleted already
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    if stderr.contains("NotFound") || stderr.contains("not found") {
                        info!("Node {} not found (may have been removed)", node_name);
                        return Ok(());
                    }
                }
            }

            if start.elapsed() > timeout {
                anyhow::bail!(
                    "Timeout waiting for node {} to be cordoned and NotReady after {} seconds",
                    node_name,
                    timeout_secs
                );
            }

            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }
    }
}
