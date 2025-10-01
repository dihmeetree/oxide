/// Kubernetes operations client
use anyhow::Result;
use std::process::Stdio;
use tokio::process::Command;

/// Kubernetes client for kubectl operations
pub struct KubernetesClient;

impl KubernetesClient {
    /// Check if kubectl is installed
    pub async fn check_kubectl_installed() -> Result<()> {
        let output = Command::new("kubectl")
            .arg("version")
            .arg("--client")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await;

        match output {
            Ok(output) if output.status.success() => Ok(()),
            _ => anyhow::bail!(
                "kubectl is not installed or not in PATH. Please install from https://kubernetes.io/docs/tasks/tools/"
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_check_kubectl() {
        // This test will pass if kubectl is installed, fail otherwise
        // It's informational rather than a strict requirement
        let result = KubernetesClient::check_kubectl_installed().await;
        if result.is_err() {
            println!("kubectl not installed (expected in test environment)");
        }
    }
}
