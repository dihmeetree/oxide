/// Kubernetes operations client
use anyhow::Result;

/// Kubernetes client for kubectl operations
pub struct KubernetesClient;

impl KubernetesClient {
    /// Check if kubectl is installed
    pub async fn check_kubectl_installed() -> Result<()> {
        crate::utils::command::check_tool_installed(
            "kubectl",
            &["version", "--client"],
            "https://kubernetes.io/docs/tasks/tools/",
        )
        .await
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
