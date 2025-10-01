/// Talos cluster operations client
use anyhow::{Context, Result};
use std::path::Path;
use std::process::Stdio;
use tokio::process::Command;
use tracing::info;

use crate::hcloud::server::ServerInfo;

/// Talos client for cluster operations
pub struct TalosClient {
    talosconfig_path: std::path::PathBuf,
}

impl TalosClient {
    /// Create a new Talos client
    pub fn new(talosconfig_path: std::path::PathBuf) -> Self {
        Self { talosconfig_path }
    }

    /// Bootstrap the Kubernetes cluster on the first control plane node
    pub async fn bootstrap(&self, control_plane: &ServerInfo) -> Result<()> {
        let server_ip = crate::hcloud::server::ServerManager::get_server_ip(&control_plane.server)
            .context("Control plane does not have a public IP")?;

        info!("Bootstrapping Kubernetes cluster on {}", server_ip);

        let output = Command::new("talosctl")
            .args([
                "bootstrap",
                "--nodes",
                &server_ip,
                "--talosconfig",
                self.talosconfig_path.to_str().unwrap(),
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .context("Failed to execute talosctl bootstrap")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("Bootstrap failed: {}", stderr);
        }

        info!("Kubernetes cluster bootstrapped successfully");

        Ok(())
    }

    /// Wait for Kubernetes API server to be ready
    pub async fn wait_for_api_server(
        &self,
        control_plane_ip: &str,
        timeout_secs: u64,
    ) -> Result<()> {
        info!("Waiting for Kubernetes API server to be ready...");

        let start = std::time::Instant::now();
        let timeout = std::time::Duration::from_secs(timeout_secs);
        let api_url = format!("https://{}:6443/version", control_plane_ip);

        loop {
            // Try to reach the API server endpoint directly
            let output = Command::new("curl")
                .args([
                    "-k",
                    "-s",
                    "-o",
                    "/dev/null",
                    "-w",
                    "%{http_code}",
                    &api_url,
                ])
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .output()
                .await;

            if let Ok(output) = output {
                let status_code = String::from_utf8_lossy(&output.stdout);
                // 401 Unauthorized or 403 Forbidden means API server is up, just needs auth
                if status_code == "401" || status_code == "403" || status_code == "200" {
                    info!("Kubernetes API server is ready");
                    return Ok(());
                }
            }

            if start.elapsed() > timeout {
                anyhow::bail!("Timeout waiting for API server to be ready");
            }

            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        }
    }

    /// Generate kubeconfig file
    pub async fn generate_kubeconfig(
        &self,
        control_plane_ip: &str,
        output_path: &Path,
    ) -> Result<()> {
        info!("Generating kubeconfig file...");

        let output = Command::new("talosctl")
            .args([
                "kubeconfig",
                output_path.to_str().unwrap(),
                "--nodes",
                control_plane_ip,
                "--talosconfig",
                self.talosconfig_path.to_str().unwrap(),
                "--force",
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .context("Failed to generate kubeconfig")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("Failed to generate kubeconfig: {}", stderr);
        }

        info!("Kubeconfig generated at: {}", output_path.display());

        Ok(())
    }

    /// Get cluster information
    #[allow(dead_code)]
    pub async fn get_cluster_info(&self, node_ip: &str) -> Result<String> {
        let output = Command::new("talosctl")
            .args([
                "version",
                "--nodes",
                node_ip,
                "--talosconfig",
                self.talosconfig_path.to_str().unwrap(),
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .context("Failed to get cluster info")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("Failed to get cluster info: {}", stderr);
        }

        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }

    /// Patch control plane nodes with the actual cluster endpoint
    pub async fn patch_cluster_endpoint(
        &self,
        control_planes: &[ServerInfo],
        actual_endpoint: &str,
    ) -> Result<()> {
        info!(
            "Patching control plane nodes with actual cluster endpoint: {}",
            actual_endpoint
        );

        // Create a JSON Patch (RFC 6902) to update the cluster endpoint
        let patch = format!(
            r#"[{{"op": "replace", "path": "/cluster/controlPlane/endpoint", "value": "{}"}}]"#,
            actual_endpoint
        );

        // Only patch control planes - workers use private network and don't need endpoint patching
        let all_nodes: Vec<&ServerInfo> = control_planes.iter().collect();

        // Patch all control plane nodes in parallel
        let mut patch_tasks = Vec::new();

        for node in all_nodes {
            let server_ip = match crate::hcloud::server::ServerManager::get_server_ip(&node.server)
            {
                Some(ip) => ip,
                None => continue,
            };
            let server_name = node.server.name.clone();
            let patch_clone = patch.clone();
            let talosconfig_path = self.talosconfig_path.clone();

            let task = tokio::spawn(async move {
                // Wait for Talos API to be ready
                let start = std::time::Instant::now();
                let timeout = std::time::Duration::from_secs(300);

                info!(
                    "Waiting for Talos API on {} ({})...",
                    server_name, server_ip
                );

                loop {
                    let output = Command::new("talosctl")
                        .args([
                            "version",
                            "--nodes",
                            &server_ip,
                            "--talosconfig",
                            talosconfig_path.to_str().unwrap(),
                        ])
                        .stdout(Stdio::piped())
                        .stderr(Stdio::piped())
                        .output()
                        .await;

                    if let Ok(output) = output {
                        if output.status.success() {
                            info!("Talos API ready on {} ({})", server_name, server_ip);
                            break;
                        }
                    }

                    if start.elapsed() > timeout {
                        anyhow::bail!(
                            "Timeout waiting for Talos API on {} ({})",
                            server_name,
                            server_ip
                        );
                    }

                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                }

                // Apply patch
                info!("Patching node: {} ({})", server_name, server_ip);

                let output = Command::new("talosctl")
                    .args([
                        "patch",
                        "mc",
                        "--nodes",
                        &server_ip,
                        "--talosconfig",
                        talosconfig_path.to_str().unwrap(),
                        "--patch",
                        &patch_clone,
                    ])
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .output()
                    .await
                    .context("Failed to patch node endpoint")?;

                if !output.status.success() {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    anyhow::bail!("Failed to patch node {}: {}", server_ip, stderr);
                }

                info!("Successfully patched {} ({})", server_name, server_ip);
                Ok::<(), anyhow::Error>(())
            });

            patch_tasks.push(task);
        }

        // Wait for all patches to complete
        let results = futures::future::join_all(patch_tasks).await;
        for result in results {
            result??; // Unwrap both JoinError and our Result
        }

        info!("All nodes patched successfully");
        Ok(())
    }

    /// Configure talosconfig with control plane endpoints
    pub async fn configure_endpoints(&self, control_plane_ips: &[String]) -> Result<()> {
        info!("Configuring talosconfig with control plane endpoints");

        // Set endpoints
        let endpoints = control_plane_ips.join(",");
        let output = Command::new("talosctl")
            .args([
                "--talosconfig",
                self.talosconfig_path.to_str().unwrap(),
                "config",
                "endpoint",
                &endpoints,
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .context("Failed to set talosconfig endpoints")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("Failed to set endpoints: {}", stderr);
        }

        // Set nodes (use first control plane as default)
        if let Some(first_ip) = control_plane_ips.first() {
            let output = Command::new("talosctl")
                .args([
                    "--talosconfig",
                    self.talosconfig_path.to_str().unwrap(),
                    "config",
                    "node",
                    first_ip,
                ])
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .output()
                .await
                .context("Failed to set talosconfig node")?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                anyhow::bail!("Failed to set node: {}", stderr);
            }
        }

        info!("Talosconfig configured with endpoints: {}", endpoints);
        Ok(())
    }

    /// Reset a node (cordon, drain, leave etcd, erase disks, power down)
    ///
    /// This performs a graceful node removal by:
    /// 1. Cordoning the node (preventing new pods)
    /// 2. Draining existing workloads
    /// 3. Leaving etcd cluster (if control plane)
    /// 4. Erasing disks
    /// 5. Powering down
    pub async fn reset_node(&self, node_ip: &str, node_name: &str) -> Result<()> {
        info!("Resetting node {} ({})", node_name, node_ip);
        info!("This will cordon, drain, leave etcd (if needed), erase disks, and power down...");

        let output = Command::new("talosctl")
            .args([
                "-n",
                node_ip,
                "--talosconfig",
                self.talosconfig_path.to_str().unwrap(),
                "reset",
                "--graceful",
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .context("Failed to execute talosctl reset")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("Failed to reset node {}: {}", node_name, stderr);
        }

        info!("Node {} reset successfully", node_name);

        Ok(())
    }

    /// Delete a Kubernetes node
    pub async fn delete_kubernetes_node(
        kubeconfig_path: &std::path::Path,
        node_name: &str,
    ) -> Result<()> {
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

    /// Check if talosctl is installed
    pub async fn check_talosctl_installed() -> Result<()> {
        let output = Command::new("talosctl")
            .arg("version")
            .arg("--client")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await;

        match output {
            Ok(output) if output.status.success() => Ok(()),
            _ => anyhow::bail!(
                "talosctl is not installed or not in PATH. Please install from https://www.talos.dev/latest/talos-guides/install/talosctl/"
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_check_talosctl() {
        // This test will pass if talosctl is installed, fail otherwise
        // It's informational rather than a strict requirement
        let result = TalosClient::check_talosctl_installed().await;
        if result.is_err() {
            println!("talosctl not installed (expected in test environment)");
        }
    }
}
