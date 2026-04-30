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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::command::test_support::MockCommandRunner;
    use crate::utils::command::with_runner;
    use std::sync::Arc;

    #[tokio::test]
    async fn test_apply_manifest_calls_kubectl() {
        let mock = Arc::new(MockCommandRunner::new());
        mock.respond("kubectl", true, "", "");

        let kubeconfig = Path::new("/fake/kubeconfig");
        let manifest = Path::new("/fake/manifest.yaml");

        with_runner(mock.clone(), async {
            Resources::apply_manifest(kubeconfig, manifest)
                .await
                .unwrap();
        })
        .await;

        let calls = mock.calls_for("kubectl");
        assert!(!calls.is_empty(), "expected kubectl to be called");
        let args: Vec<_> = calls[0].args_str();
        let args_str: Vec<&str> = args.iter().map(|s| s.as_ref()).collect();
        assert!(args_str.contains(&"apply"), "expected 'apply' in args");
        assert!(args_str.contains(&"-f"), "expected '-f' in args");
        assert!(
            args_str.contains(&"/fake/manifest.yaml"),
            "expected manifest path"
        );
    }

    #[tokio::test]
    async fn test_apply_manifest_failure_propagates() {
        let mock = Arc::new(MockCommandRunner::new());
        mock.respond("kubectl", false, "", "failed to connect");

        let kubeconfig = Path::new("/fake/kubeconfig");
        let manifest = Path::new("/fake/manifest.yaml");

        let result = with_runner(mock.clone(), async {
            Resources::apply_manifest(kubeconfig, manifest).await
        })
        .await;

        assert!(result.is_err(), "expected error when kubectl fails");
    }
}
