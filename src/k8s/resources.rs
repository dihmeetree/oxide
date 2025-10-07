/// Generic Kubernetes resource operations
use anyhow::Result;
use std::path::Path;
use tracing::info;

use crate::utils::command::CommandBuilder;

/// Generic Kubernetes resource management
pub struct Resources;

impl Resources {
    /// Apply a Kubernetes manifest file
    pub async fn apply_manifest(kubeconfig_path: &Path, manifest_path: &Path) -> Result<()> {
        info!("Applying Kubernetes manifest: {}", manifest_path.display());

        let manifest_path_str = manifest_path
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("Manifest path contains invalid UTF-8"))?;

        CommandBuilder::new("kubectl")
            .args(["apply", "-f", manifest_path_str])
            .kubeconfig(kubeconfig_path)
            .context("Failed to apply manifest")
            .run_silent()
            .await?;

        Ok(())
    }
}
