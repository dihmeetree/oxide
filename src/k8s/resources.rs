/// Generic Kubernetes resource operations
use anyhow::{Context, Result};
use std::path::Path;
use std::process::Stdio;
use tokio::process::Command;
use tracing::info;

/// Generic Kubernetes resource management
pub struct ResourceManager;

impl ResourceManager {
    /// Apply a Kubernetes manifest file
    pub async fn apply_manifest(kubeconfig_path: &Path, manifest_path: &Path) -> Result<()> {
        info!("Applying Kubernetes manifest: {}", manifest_path.display());

        let output = Command::new("kubectl")
            .args(["apply", "-f", manifest_path.to_str().unwrap()])
            .env("KUBECONFIG", kubeconfig_path)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .context("Failed to apply manifest")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("Failed to apply manifest: {}", stderr);
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        info!("{}", stdout.trim());

        Ok(())
    }
}
