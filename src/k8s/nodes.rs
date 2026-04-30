/// Kubernetes node operations
use anyhow::Result;
use std::path::Path;
use tracing::info;

use crate::utils::command::CommandBuilder;
use crate::utils::polling::PollingConfig;

/// Kubernetes node management operations
pub struct NodeManager;

impl NodeManager {
    /// Delete a Kubernetes node
    pub async fn delete_node(kubeconfig_path: &Path, node_name: &str) -> Result<()> {
        info!("Deleting Kubernetes node: {}", node_name);

        let output = CommandBuilder::new("kubectl")
            .args(["delete", "node", node_name])
            .kubeconfig(kubeconfig_path)
            .context("Failed to delete Kubernetes node")
            .output()
            .await?;

        if !output.success {
            // Don't fail if node doesn't exist
            if output.stderr.contains("NotFound") || output.stderr.contains("not found") {
                info!(
                    "Node {} not found in Kubernetes (already removed)",
                    node_name
                );
                return Ok(());
            }
            anyhow::bail!("Failed to delete node {}: {}", node_name, output.stderr);
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
        let kubeconfig_path = kubeconfig_path.to_path_buf();
        let node_name = node_name.to_string();

        let config = PollingConfig::new(
            timeout_secs,
            5,
            format!("Waiting for node {} to become Ready", node_name),
        );

        config
            .poll_until(|| {
                let kubeconfig_path = kubeconfig_path.clone();
                let node_name = node_name.clone();
                async move {
                    let output = CommandBuilder::new("kubectl")
                        .args([
                            "get",
                            "node",
                            &node_name,
                            "-o",
                            "jsonpath={.status.conditions[?(@.type=='Ready')].status}",
                        ])
                        .kubeconfig(&kubeconfig_path)
                        .output()
                        .await;

                    if let Ok(output) = output {
                        if output.success && output.stdout.trim().eq_ignore_ascii_case("true") {
                            return Ok(true);
                        }
                    }
                    Ok(false)
                }
            })
            .await
    }

    /// Wait for all Kubernetes nodes to be Ready
    pub async fn wait_for_all_nodes_ready(kubeconfig_path: &Path, timeout_secs: u64) -> Result<()> {
        info!("Waiting for all nodes to be Ready...");

        // Get list of all node names
        let node_names = CommandBuilder::new("kubectl")
            .args(["get", "nodes", "-o", "jsonpath={.items[*].metadata.name}"])
            .kubeconfig(kubeconfig_path)
            .context("Failed to get node names")
            .run()
            .await?;

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
        let kubeconfig_path = kubeconfig_path.to_path_buf();
        let node_name = node_name.to_string();

        let config = PollingConfig::new(
            timeout_secs,
            2,
            format!("Waiting for node {} to be cordoned and NotReady", node_name),
        );

        config
            .poll_until(|| {
                let kubeconfig_path = kubeconfig_path.clone();
                let node_name = node_name.clone();
                async move {
                    // Check both spec.unschedulable and Ready condition status
                    let output = CommandBuilder::new("kubectl")
                        .args([
                            "get",
                            "node",
                            &node_name,
                            "-o",
                            "jsonpath={.spec.unschedulable},{.status.conditions[?(@.type=='Ready')].status}",
                        ])
                        .kubeconfig(&kubeconfig_path)
                        .output()
                        .await;

                    if let Ok(output) = output {
                        if output.success {
                            let parts: Vec<&str> = output.stdout.trim().split(',').collect();

                            if parts.len() == 2 {
                                let unschedulable = parts[0];
                                let ready_status = parts[1];

                                // Node should be unschedulable=true (SchedulingDisabled) AND Ready=False (NotReady)
                                if unschedulable == "true"
                                    && ready_status.eq_ignore_ascii_case("false")
                                {
                                    info!(
                                        "[OK] Node {} is cordoned and NotReady (NotReady,SchedulingDisabled)",
                                        node_name
                                    );
                                    return Ok(true);
                                }
                            }
                        } else {
                            // Node might have been deleted already
                            if output.stderr.contains("NotFound")
                                || output.stderr.contains("not found")
                            {
                                info!("Node {} not found (may have been removed)", node_name);
                                return Ok(true);
                            }
                        }
                    }
                    Ok(false)
                }
            })
            .await
    }

    /// Get pods running on a specific node
    pub async fn get_pods_on_node(kubeconfig_path: &Path, node_name: &str) -> Result<Vec<String>> {
        let output = CommandBuilder::new("kubectl")
            .args([
                "get",
                "pods",
                "--all-namespaces",
                "--field-selector",
                &format!("spec.nodeName={}", node_name),
                "-o",
                "jsonpath={.items[*].metadata.name}",
            ])
            .kubeconfig(kubeconfig_path)
            .context("Failed to get pods on node")
            .output()
            .await?;

        if !output.success {
            // If node doesn't exist, return empty list
            if output.stderr.contains("NotFound") || output.stderr.contains("not found") {
                return Ok(Vec::new());
            }
            anyhow::bail!(
                "Failed to get pods on node {}: {}",
                node_name,
                output.stderr
            );
        }

        let pods: Vec<String> = output
            .stdout
            .split_whitespace()
            .map(|s| s.to_string())
            .collect();

        Ok(pods)
    }

    /// Monitor pod draining progress on a node.
    ///
    /// Returns `Ok(())` once every pod has been evicted from `node_name`. If the
    /// timeout elapses while pods remain, an error is returned so callers can
    /// react explicitly (e.g. log a warning, fall back to forced reset, or abort
    /// destructive follow-up work). Previously the timeout path returned
    /// `Ok(())`, which silently masked incomplete drains.
    pub async fn monitor_drain_progress(
        kubeconfig_path: &Path,
        node_name: &str,
        timeout_secs: u64,
    ) -> Result<()> {
        info!("Monitoring pod drain progress on node {}...", node_name);

        let start = std::time::Instant::now();
        let timeout = std::time::Duration::from_secs(timeout_secs);

        let mut last_pod_count = usize::MAX;

        loop {
            let pods = Self::get_pods_on_node(kubeconfig_path, node_name).await?;
            let pod_count = pods.len();

            // Show progress if pod count changed
            if pod_count != last_pod_count {
                if pod_count == 0 {
                    info!("[OK] All pods drained from node {}", node_name);
                    return Ok(());
                }
                info!("  {} pods remaining on node {}", pod_count, node_name);
                last_pod_count = pod_count;
            }

            if start.elapsed() > timeout {
                anyhow::bail!(
                    "Drain timeout: {} pods still running on node {} after {}s",
                    pod_count,
                    node_name,
                    timeout_secs
                );
            }

            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        }
    }

    /// Validate that removing nodes won't break etcd quorum
    /// Requires maintaining odd number of control planes (1, 3, 5)
    pub async fn validate_etcd_quorum(
        kubeconfig_path: &Path,
        nodes_to_remove: &[String],
    ) -> Result<()> {
        // Get all control plane nodes
        let output = CommandBuilder::new("kubectl")
            .args([
                "get",
                "nodes",
                "-l",
                "node-role.kubernetes.io/control-plane",
                "-o",
                "jsonpath={.items[*].metadata.name}",
            ])
            .kubeconfig(kubeconfig_path)
            .context("Failed to get control plane nodes")
            .output()
            .await?;

        if !output.success {
            // If we can't get nodes, skip validation (cluster might not be accessible)
            return Ok(());
        }

        let control_planes: Vec<&str> = output.stdout.split_whitespace().collect();
        let current_count = control_planes.len();

        // Deduplicate caller-provided node names so a duplicated entry can't be
        // counted twice when computing the remaining quorum.
        let mut seen = std::collections::HashSet::new();
        let control_planes_to_remove: Vec<&String> = nodes_to_remove
            .iter()
            .filter(|node| control_planes.contains(&node.as_str()))
            .filter(|node| seen.insert(node.as_str()))
            .collect();

        if control_planes_to_remove.is_empty() {
            // Only removing workers, no etcd quorum impact
            return Ok(());
        }

        // Use saturating subtraction to defend against bookkeeping bugs that could
        // otherwise underflow (panicking in debug, wrapping in release).
        let remaining_count = current_count.saturating_sub(control_planes_to_remove.len());

        info!(
            "Control plane nodes: {} current, {} to remove, {} remaining",
            current_count,
            control_planes_to_remove.len(),
            remaining_count
        );

        // Validate quorum requirements
        if remaining_count == 0 {
            anyhow::bail!(
                "Cannot remove all control plane nodes. At least 1 control plane must remain."
            );
        }

        // Warn if remaining count is even (not recommended for etcd)
        if remaining_count.is_multiple_of(2) {
            info!(
                "WARNING:  Warning: Remaining control plane count ({}) is even. Etcd recommends odd numbers (1, 3, 5).",
                remaining_count
            );
            info!("   This will reduce fault tolerance.");
        }

        // Check minimum quorum: need majority to maintain quorum
        let quorum_size = (current_count / 2) + 1;
        if remaining_count < quorum_size {
            anyhow::bail!(
                "Cannot remove {} control plane nodes. Would break etcd quorum.\n\
                Current: {} nodes, Quorum requires: {} nodes, Remaining: {} nodes.\n\
                You can remove at most {} control plane nodes at a time.",
                control_planes_to_remove.len(),
                current_count,
                quorum_size,
                remaining_count,
                current_count - quorum_size
            );
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::command::test_support::MockCommandRunner;
    use crate::utils::command::with_runner;
    use std::sync::Arc;

    #[tokio::test]
    async fn test_delete_node_success() {
        let mock = Arc::new(MockCommandRunner::new());
        mock.respond("kubectl", true, "node \"worker-1\" deleted", "");

        let result = with_runner(mock.clone(), async {
            NodeManager::delete_node(Path::new("/fake/kubeconfig"), "worker-1").await
        })
        .await;

        assert!(result.is_ok());
        let calls = mock.calls_for("kubectl");
        assert!(!calls.is_empty());
        let args: Vec<_> = calls[0].args_str();
        let args_str: Vec<&str> = args.iter().map(|s| s.as_ref()).collect();
        assert!(args_str.contains(&"delete"));
        assert!(args_str.contains(&"node"));
        assert!(args_str.contains(&"worker-1"));
    }

    #[tokio::test]
    async fn test_delete_node_not_found_ok() {
        let mock = Arc::new(MockCommandRunner::new());
        mock.respond(
            "kubectl",
            false,
            "",
            "Error from server (NotFound): nodes \"worker-1\" not found",
        );

        let result = with_runner(mock.clone(), async {
            NodeManager::delete_node(Path::new("/fake/kubeconfig"), "worker-1").await
        })
        .await;

        // NotFound is silently accepted
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_delete_node_real_failure() {
        let mock = Arc::new(MockCommandRunner::new());
        mock.respond("kubectl", false, "", "connection refused");

        let result = with_runner(mock.clone(), async {
            NodeManager::delete_node(Path::new("/fake/kubeconfig"), "worker-1").await
        })
        .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_get_pods_on_node_success() {
        let mock = Arc::new(MockCommandRunner::new());
        mock.respond("kubectl", true, "coredns-abc nginx-xyz", "");

        let result = with_runner(mock.clone(), async {
            NodeManager::get_pods_on_node(Path::new("/fake/kubeconfig"), "worker-1").await
        })
        .await;

        assert!(result.is_ok());
        let pods = result.unwrap();
        assert_eq!(pods.len(), 2);
        assert!(pods.contains(&"coredns-abc".to_string()));
        assert!(pods.contains(&"nginx-xyz".to_string()));
    }

    #[tokio::test]
    async fn test_get_pods_on_node_empty() {
        let mock = Arc::new(MockCommandRunner::new());
        mock.respond("kubectl", true, "", "");

        let result = with_runner(mock.clone(), async {
            NodeManager::get_pods_on_node(Path::new("/fake/kubeconfig"), "worker-1").await
        })
        .await;

        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_get_pods_on_node_not_found() {
        let mock = Arc::new(MockCommandRunner::new());
        mock.respond("kubectl", false, "", "not found");

        let result = with_runner(mock.clone(), async {
            NodeManager::get_pods_on_node(Path::new("/fake/kubeconfig"), "worker-1").await
        })
        .await;

        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_get_pods_on_node_correct_args() {
        let mock = Arc::new(MockCommandRunner::new());
        mock.respond("kubectl", true, "", "");

        with_runner(mock.clone(), async {
            NodeManager::get_pods_on_node(Path::new("/fake/kube"), "my-node")
                .await
                .unwrap();
        })
        .await;

        let calls = mock.calls_for("kubectl");
        let args: Vec<_> = calls[0].args_str();
        let args_str: Vec<&str> = args.iter().map(|s| s.as_ref()).collect();
        assert!(args_str.contains(&"get"));
        assert!(args_str.contains(&"pods"));
        assert!(args_str.contains(&"--all-namespaces"));
        // field selector includes the node name
        let has_selector = args_str.iter().any(|a| a.contains("my-node"));
        assert!(has_selector, "expected node name in field selector");
    }
}
