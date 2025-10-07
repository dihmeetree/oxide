/// Kubernetes Metrics Server management
use anyhow::Result;
use std::path::PathBuf;
use tracing::info;

use crate::utils::command::CommandBuilder;

const METRICS_SERVER_URL: &str =
    "https://github.com/kubernetes-sigs/metrics-server/releases/latest/download/components.yaml";

pub struct MetricsServer {
    kubeconfig_path: PathBuf,
}

impl MetricsServer {
    pub const fn new(kubeconfig_path: PathBuf) -> Self {
        Self { kubeconfig_path }
    }

    /// Install Kubernetes Metrics Server
    pub async fn install(&self) -> Result<()> {
        info!("Installing Kubernetes Metrics Server...");

        if !self.kubeconfig_path.exists() {
            anyhow::bail!(
                "Kubeconfig not found at {}. Please create the cluster first.",
                self.kubeconfig_path.display()
            );
        }

        // Apply the latest metrics-server manifest directly from GitHub
        info!("Applying metrics-server manifest from GitHub...");
        CommandBuilder::new("kubectl")
            .args(["apply", "-f", METRICS_SERVER_URL])
            .kubeconfig(&self.kubeconfig_path)
            .context("Failed to install metrics-server")
            .run_silent()
            .await?;

        info!("✓ Kubernetes Metrics Server installed successfully!");
        info!("");
        info!("The metrics server will start collecting metrics in a few seconds.");
        info!("You can verify it's working with:");
        info!("  kubectl top nodes");
        info!("  kubectl top pods");

        Ok(())
    }

    /// Uninstall Kubernetes Metrics Server
    pub async fn uninstall(&self) -> Result<()> {
        info!("Uninstalling Kubernetes Metrics Server...");

        if !self.kubeconfig_path.exists() {
            anyhow::bail!(
                "Kubeconfig not found at {}. Please create the cluster first.",
                self.kubeconfig_path.display()
            );
        }

        // Delete resources directly from GitHub URL
        info!("Deleting metrics-server resources...");
        CommandBuilder::new("kubectl")
            .args(["delete", "-f", METRICS_SERVER_URL])
            .kubeconfig(&self.kubeconfig_path)
            .context("Failed to delete metrics-server resources")
            .run_silent()
            .await?;

        info!("✓ Kubernetes Metrics Server uninstalled successfully!");

        Ok(())
    }
}
