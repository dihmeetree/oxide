/// Generic Kubernetes resource operations
use anyhow::Result;
use std::path::Path;
use tracing::info;

use crate::utils::command::CommandBuilder;

/// Generic Kubernetes resource management
pub struct ResourceManager;

impl ResourceManager {
    /// Apply a Kubernetes manifest file
    pub async fn apply_manifest(kubeconfig_path: &Path, manifest_path: &Path) -> Result<()> {
        info!("Applying Kubernetes manifest: {}", manifest_path.display());

        let stdout = CommandBuilder::new("kubectl")
            .args(["apply", "-f", manifest_path.to_str().unwrap()])
            .kubeconfig(kubeconfig_path)
            .context("Failed to apply manifest")
            .run()
            .await?;

        info!("{}", stdout.trim());

        Ok(())
    }
}
