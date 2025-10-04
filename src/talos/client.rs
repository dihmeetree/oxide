/// Talos cluster operations client
use anyhow::{Context, Result};
use std::path::Path;
use std::process::Stdio;
use tokio::process::Command;
use tracing::info;

use crate::hcloud::server::ServerInfo;
use crate::utils::command::CommandBuilder;
use crate::utils::polling::PollingConfig;

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
        let api_url = format!("https://{}:6443/version", control_plane_ip);

        let config = PollingConfig::new(
            timeout_secs,
            5,
            "Waiting for Kubernetes API server to be ready",
        );

        config
            .poll_until(|| {
                let api_url = api_url.clone();
                async move {
                    // Try to reach the API server endpoint directly
                    let output = CommandBuilder::new("curl")
                        .args([
                            "-k",
                            "-s",
                            "-o",
                            "/dev/null",
                            "-w",
                            "%{http_code}",
                            &api_url,
                        ])
                        .output()
                        .await;

                    if let Ok(output) = output {
                        // 401 Unauthorized or 403 Forbidden means API server is up, just needs auth
                        let status_code = output.stdout.trim();
                        if status_code == "401" || status_code == "403" || status_code == "200" {
                            return Ok(true);
                        }
                    }
                    Ok(false)
                }
            })
            .await
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
    #[allow(dead_code)]
    pub async fn reset_node(&self, node_ip: &str, node_name: &str) -> Result<()> {
        self.reset_node_with_timeout(node_ip, node_name, 600, false, 0)
            .await
    }

    /// Reset a node with custom timeout and retry options
    ///
    /// # Arguments
    /// * `node_ip` - IP address of the node to reset
    /// * `node_name` - Name of the node for logging
    /// * `timeout_secs` - Timeout in seconds for the reset operation (default: 600)
    /// * `force` - If true, skip graceful reset and force immediate reset
    /// * `max_retries` - Number of retries if reset fails (default: 0)
    pub async fn reset_node_with_timeout(
        &self,
        node_ip: &str,
        node_name: &str,
        timeout_secs: u64,
        force: bool,
        max_retries: u32,
    ) -> Result<()> {
        info!("Resetting node {} ({})", node_name, node_ip);
        if force {
            info!("Using FORCE reset (non-graceful)");
        } else {
            info!(
                "This will cordon, drain, leave etcd (if needed), erase disks, and power down..."
            );
            info!("Timeout: {} seconds", timeout_secs);
        }

        let mut attempts = 0;
        let max_attempts = max_retries + 1;

        loop {
            attempts += 1;
            if attempts > 1 {
                info!(
                    "Retry attempt {}/{} for node {}",
                    attempts - 1,
                    max_retries,
                    node_name
                );
            }

            let mut args = vec![
                "--talosconfig".to_string(),
                self.talosconfig_path.to_str().unwrap().to_string(),
                "reset".to_string(),
                "--nodes".to_string(),
                node_ip.to_string(),
            ];

            if force {
                args.push("--graceful=false".to_string());
            }

            args.push("--wait".to_string());
            args.push("--timeout".to_string());
            args.push(format!("{}s", timeout_secs));

            let output = Command::new("talosctl")
                .args(&args)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .output()
                .await
                .context("Failed to execute talosctl reset")?;

            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);

            // Debug: always log what we got from talosctl
            if !stdout.is_empty() {
                info!("talosctl stdout for {}: {}", node_name, stdout);
            }
            if !stderr.is_empty() {
                info!("talosctl stderr for {}: {}", node_name, stderr);
            }
            info!(
                "talosctl exit code for {}: {}",
                node_name,
                output.status.code().unwrap_or(-1)
            );

            if output.status.success() {
                info!("Node {} reset successfully", node_name);
                return Ok(());
            }

            // Check if this is a retriable error (but not normal progress messages)
            let is_retriable = (stderr.contains("i/o timeout")
                || stderr.contains("connection refused")
                || stderr.contains("no route to host"))
                && !stderr.contains("events check condition met");

            if attempts < max_attempts && is_retriable {
                info!("Reset failed with retriable error, retrying in 10 seconds...");
                tokio::time::sleep(std::time::Duration::from_secs(10)).await;
                continue;
            }

            // Non-retriable error or max retries reached
            anyhow::bail!(
                "Failed to reset node {} after {} attempts: {}",
                node_name,
                attempts,
                stderr
            );
        }
    }

    /// Check if talosctl is installed
    pub async fn check_talosctl_installed() -> Result<()> {
        crate::utils::command::check_tool_installed(
            "talosctl",
            &["version", "--client"],
            "https://www.talos.dev/latest/talos-guides/install/talosctl/",
        )
        .await
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
