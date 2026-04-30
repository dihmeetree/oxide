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

    /// Install Kubernetes Metrics Server (instance method)
    pub async fn install_metrics_server(&self) -> Result<()> {
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

        info!("[OK] Kubernetes Metrics Server installed successfully!");
        info!("The metrics server will start collecting metrics in a few seconds.");
        info!("You can verify it's working with:");
        info!("  kubectl top nodes");
        info!("  kubectl top pods");

        Ok(())
    }

    /// Uninstall Kubernetes Metrics Server (instance method)
    pub async fn uninstall_metrics_server(&self) -> Result<()> {
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

        info!("[OK] Kubernetes Metrics Server uninstalled successfully!");

        Ok(())
    }

    /// Install metrics server
    pub async fn install(output_dir: &std::path::Path) -> Result<()> {
        let kubeconfig_path = output_dir.join("kubeconfig");
        let metrics_server = Self::new(kubeconfig_path);
        metrics_server.install_metrics_server().await
    }

    /// Uninstall metrics server
    pub async fn uninstall(output_dir: &std::path::Path) -> Result<()> {
        let kubeconfig_path = output_dir.join("kubeconfig");
        let metrics_server = Self::new(kubeconfig_path);
        metrics_server.uninstall_metrics_server().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::command::test_support::MockCommandRunner;
    use crate::utils::command::with_runner;
    use std::sync::Arc;

    #[tokio::test]
    async fn test_install_metrics_server_calls_kubectl_apply() {
        let dir = tempfile::tempdir().unwrap();
        let kubeconfig = dir.path().join("kubeconfig");
        std::fs::write(&kubeconfig, "dummy").unwrap();

        let mock = Arc::new(MockCommandRunner::new());
        mock.respond("kubectl", true, "", "");

        let ms = MetricsServer::new(kubeconfig);
        let result = with_runner(mock.clone(), async { ms.install_metrics_server().await }).await;

        assert!(result.is_ok());
        let calls = mock.calls_for("kubectl");
        assert!(!calls.is_empty());
        let args: Vec<_> = calls[0].args_str();
        let args_str: Vec<&str> = args.iter().map(|s| s.as_ref()).collect();
        assert!(args_str.contains(&"apply"), "expected 'apply' arg");
        assert!(args_str.contains(&"-f"), "expected '-f' arg");
        // URL should be in arguments
        let has_url = args_str.iter().any(|a| a.contains("metrics-server"));
        assert!(has_url, "expected metrics-server URL in args");
    }

    #[tokio::test]
    async fn test_install_metrics_server_missing_kubeconfig() {
        let ms = MetricsServer::new(PathBuf::from("/nonexistent/kubeconfig"));
        let mock = Arc::new(MockCommandRunner::new());

        let result = with_runner(mock.clone(), async { ms.install_metrics_server().await }).await;

        assert!(result.is_err());
        let msg = format!("{}", result.unwrap_err());
        assert!(
            msg.contains("Kubeconfig not found") || msg.contains("kubeconfig"),
            "expected kubeconfig error, got: {}",
            msg
        );
    }

    #[tokio::test]
    async fn test_uninstall_metrics_server_calls_kubectl_delete() {
        let dir = tempfile::tempdir().unwrap();
        let kubeconfig = dir.path().join("kubeconfig");
        std::fs::write(&kubeconfig, "dummy").unwrap();

        let mock = Arc::new(MockCommandRunner::new());
        mock.respond("kubectl", true, "", "");

        let ms = MetricsServer::new(kubeconfig);
        let result = with_runner(mock.clone(), async { ms.uninstall_metrics_server().await }).await;

        assert!(result.is_ok());
        let calls = mock.calls_for("kubectl");
        assert!(!calls.is_empty());
        let args: Vec<_> = calls[0].args_str();
        let args_str: Vec<&str> = args.iter().map(|s| s.as_ref()).collect();
        assert!(args_str.contains(&"delete"), "expected 'delete' arg");
        assert!(args_str.contains(&"-f"), "expected '-f' arg");
    }

    #[tokio::test]
    async fn test_uninstall_metrics_server_missing_kubeconfig() {
        let ms = MetricsServer::new(PathBuf::from("/nonexistent/kubeconfig"));
        let mock = Arc::new(MockCommandRunner::new());

        let result = with_runner(mock.clone(), async { ms.uninstall_metrics_server().await }).await;

        assert!(result.is_err());
    }
}
