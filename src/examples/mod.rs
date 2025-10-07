/// Example application deployments
use anyhow::Result;
use tracing::info;

use crate::k8s::Resources;

/// Example deployments
pub struct Examples;

impl Examples {
    /// Deploy nginx with Gateway API
    pub async fn deploy_nginx(output_dir: &std::path::Path) -> Result<()> {
        info!("Deploying nginx with Gateway API...");

        let kubeconfig_path = output_dir.join("kubeconfig");
        if !kubeconfig_path.exists() {
            anyhow::bail!(
                "Kubeconfig not found at {}. Please create the cluster first.",
                kubeconfig_path.display()
            );
        }

        // Apply nginx deployment and service
        let nginx_deployment_path = std::path::Path::new("nginx-deployment.yaml");
        if !nginx_deployment_path.exists() {
            anyhow::bail!("nginx-deployment.yaml not found in current directory");
        }
        Resources::apply_manifest(&kubeconfig_path, nginx_deployment_path).await?;

        // Apply Gateway and HTTPRoute
        let nginx_gateway_path = std::path::Path::new("nginx-gateway.yaml");
        if !nginx_gateway_path.exists() {
            anyhow::bail!("nginx-gateway.yaml not found in current directory");
        }
        Resources::apply_manifest(&kubeconfig_path, nginx_gateway_path).await?;

        info!("✓ nginx deployed successfully with Gateway API!");
        info!("To check the status:");
        info!("  kubectl get pods");
        info!("  kubectl get gateway");
        info!("  kubectl get httproute");

        Ok(())
    }
}
