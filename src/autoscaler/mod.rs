/// Kubernetes Cluster Autoscaler with Hetzner support
use anyhow::Result;
use base64::{engine::general_purpose, Engine as _};
use std::path::{Path, PathBuf};
use tracing::info;

use crate::config::{AutoscalerConfig, ClusterConfig};
use crate::k8s::Resources;
use crate::utils::command::CommandBuilder;

pub struct Autoscaler {
    kubeconfig_path: PathBuf,
}

impl Autoscaler {
    pub const fn new(kubeconfig_path: PathBuf) -> Self {
        Self { kubeconfig_path }
    }

    /// Install Kubernetes Cluster Autoscaler
    pub async fn install_autoscaler(
        &self,
        config: &ClusterConfig,
        autoscaler_config: &AutoscalerConfig,
        worker_config_path: &Path,
    ) -> Result<()> {
        info!("Installing Kubernetes Cluster Autoscaler with Hetzner support...");

        if !autoscaler_config.enabled {
            anyhow::bail!("Autoscaler is disabled in configuration. Set autoscaler.enabled = true");
        }

        if !self.kubeconfig_path.exists() {
            anyhow::bail!(
                "Kubeconfig not found at {}. Please create the cluster first.",
                self.kubeconfig_path.display()
            );
        }

        // 1. Apply namespace
        info!("Creating oxide-system namespace...");
        Resources::apply_manifest(
            &self.kubeconfig_path,
            Path::new("manifests/autoscaler/01-namespace.yaml"),
        )
        .await?;

        // 2. Create Secret with Hetzner token
        info!("Creating Hetzner API token secret...");
        let hcloud_token = config.get_hcloud_token()?;
        self.create_hcloud_secret(&hcloud_token).await?;

        // 3. Create ConfigMap with Talos worker cloud-init
        info!("Creating Talos worker configuration ConfigMap...");
        self.create_talos_configmap(worker_config_path).await?;

        // 4. Apply ServiceAccount and RBAC
        info!("Creating ServiceAccount and RBAC...");
        Resources::apply_manifest(
            &self.kubeconfig_path,
            Path::new("manifests/autoscaler/02-serviceaccount.yaml"),
        )
        .await?;
        Resources::apply_manifest(
            &self.kubeconfig_path,
            Path::new("manifests/autoscaler/03-rbac.yaml"),
        )
        .await?;

        // 5. Generate and apply Deployment with dynamic configuration
        info!("Deploying autoscaler...");
        self.create_deployment(config, autoscaler_config).await?;

        info!("[OK] Kubernetes Cluster Autoscaler deployed successfully!");
        info!("Worker pools:");
        for pool in &autoscaler_config.worker_pools {
            info!(
                "  {} - min: {}, max: {}, type: {}, location: {}",
                pool.name, pool.min_nodes, pool.max_nodes, pool.server_type, pool.location
            );
        }
        info!("Monitor autoscaler logs with:");
        info!(
            "  kubectl logs -n oxide-system -l app=cluster-autoscaler -f --kubeconfig={}",
            self.kubeconfig_path.display()
        );

        Ok(())
    }

    /// Uninstall cluster autoscaler
    pub async fn uninstall_autoscaler(&self) -> Result<()> {
        info!("Uninstalling Kubernetes Cluster Autoscaler...");

        if !self.kubeconfig_path.exists() {
            anyhow::bail!(
                "Kubeconfig not found at {}. Cluster may not exist.",
                self.kubeconfig_path.display()
            );
        }

        // Delete deployment
        info!("Deleting autoscaler deployment...");
        let _ = CommandBuilder::new("kubectl")
            .args([
                "delete",
                "deployment",
                "cluster-autoscaler",
                "-n",
                "oxide-system",
                "--ignore-not-found=true",
            ])
            .kubeconfig(&self.kubeconfig_path)
            .run()
            .await;

        // Delete RBAC resources
        info!("Deleting RBAC resources...");
        let _ = CommandBuilder::new("kubectl")
            .args([
                "delete",
                "clusterrolebinding",
                "cluster-autoscaler",
                "--ignore-not-found=true",
            ])
            .kubeconfig(&self.kubeconfig_path)
            .run()
            .await;

        let _ = CommandBuilder::new("kubectl")
            .args([
                "delete",
                "clusterrole",
                "cluster-autoscaler",
                "--ignore-not-found=true",
            ])
            .kubeconfig(&self.kubeconfig_path)
            .run()
            .await;

        let _ = CommandBuilder::new("kubectl")
            .args([
                "delete",
                "rolebinding",
                "cluster-autoscaler",
                "-n",
                "kube-system",
                "--ignore-not-found=true",
            ])
            .kubeconfig(&self.kubeconfig_path)
            .run()
            .await;

        let _ = CommandBuilder::new("kubectl")
            .args([
                "delete",
                "role",
                "cluster-autoscaler",
                "-n",
                "kube-system",
                "--ignore-not-found=true",
            ])
            .kubeconfig(&self.kubeconfig_path)
            .run()
            .await;

        // Delete ServiceAccount
        info!("Deleting ServiceAccount...");
        let _ = CommandBuilder::new("kubectl")
            .args([
                "delete",
                "serviceaccount",
                "oxide-autoscaler",
                "-n",
                "oxide-system",
                "--ignore-not-found=true",
            ])
            .kubeconfig(&self.kubeconfig_path)
            .run()
            .await;

        // Delete ConfigMap and Secret
        info!("Deleting ConfigMap and Secret...");
        let _ = CommandBuilder::new("kubectl")
            .args([
                "delete",
                "configmap",
                "oxide-talos-config",
                "-n",
                "oxide-system",
                "--ignore-not-found=true",
            ])
            .kubeconfig(&self.kubeconfig_path)
            .run()
            .await;

        let _ = CommandBuilder::new("kubectl")
            .args([
                "delete",
                "secret",
                "oxide-hcloud-token",
                "-n",
                "oxide-system",
                "--ignore-not-found=true",
            ])
            .kubeconfig(&self.kubeconfig_path)
            .run()
            .await;

        info!("[OK] Cluster autoscaler uninstalled successfully!");

        Ok(())
    }

    /// Create Hetzner Cloud API token secret
    async fn create_hcloud_secret(&self, token: &str) -> Result<()> {
        let secret_yaml = CommandBuilder::new("kubectl")
            .args([
                "create",
                "secret",
                "generic",
                "oxide-hcloud-token",
                "-n",
                "oxide-system",
                &format!("--from-literal=token={token}"),
                "--dry-run=client",
                "-o",
                "yaml",
            ])
            .kubeconfig(&self.kubeconfig_path)
            .context("Failed to generate secret")
            .run()
            .await?;

        // Write to temp file and apply
        let temp_file = std::env::temp_dir().join("hcloud-secret.yaml");
        tokio::fs::write(&temp_file, secret_yaml).await?;
        Resources::apply_manifest(&self.kubeconfig_path, &temp_file).await?;
        tokio::fs::remove_file(&temp_file).await?;

        Ok(())
    }

    /// Create Talos worker config ConfigMap for autoscaler
    async fn create_talos_configmap(&self, worker_config_path: &Path) -> Result<()> {
        if !worker_config_path.exists() {
            anyhow::bail!(
                "Worker config not found at {}. Please create the cluster first.",
                worker_config_path.display()
            );
        }

        let worker_config = tokio::fs::read_to_string(worker_config_path).await?;
        let worker_config_b64 = general_purpose::STANDARD.encode(worker_config.as_bytes());

        let configmap_yaml = format!(
            r"apiVersion: v1
kind: ConfigMap
metadata:
  name: oxide-talos-config
  namespace: oxide-system
data:
  worker-config: {worker_config_b64}"
        );

        let temp_file = std::env::temp_dir().join("talos-config.yaml");
        tokio::fs::write(&temp_file, configmap_yaml).await?;
        Resources::apply_manifest(&self.kubeconfig_path, &temp_file).await?;
        tokio::fs::remove_file(&temp_file).await?;

        Ok(())
    }

    /// Create autoscaler deployment with dynamic configuration
    async fn create_deployment(
        &self,
        config: &ClusterConfig,
        autoscaler_config: &AutoscalerConfig,
    ) -> Result<()> {
        // Build node pool arguments for autoscaler
        let node_args: Vec<String> = autoscaler_config
            .worker_pools
            .iter()
            .map(|pool| {
                format!(
                    "            - --nodes={}:{}:{}:{}:{}",
                    pool.min_nodes, pool.max_nodes, pool.server_type, pool.location, pool.name
                )
            })
            .collect();

        let deployment_yaml = format!(
            r#"apiVersion: apps/v1
kind: Deployment
metadata:
  name: cluster-autoscaler
  namespace: oxide-system
  labels:
    app: cluster-autoscaler
spec:
  replicas: 1
  selector:
    matchLabels:
      app: cluster-autoscaler
  template:
    metadata:
      labels:
        app: cluster-autoscaler
    spec:
      priorityClassName: system-cluster-critical
      securityContext:
        runAsNonRoot: true
        runAsUser: 65534
        fsGroup: 65534
      serviceAccountName: oxide-autoscaler
      tolerations:
        - key: workload
          operator: Equal
          value: application
          effect: NoSchedule
      containers:
        - name: cluster-autoscaler
          image: registry.k8s.io/autoscaling/cluster-autoscaler:{}
          command:
            - ./cluster-autoscaler
            - --cloud-provider=hetzner
{}
            - --skip-nodes-with-system-pods=false
            - --skip-nodes-with-local-storage=false
            - --balance-similar-node-groups
            - --expander=least-waste
            - --scale-down-utilization-threshold=0.5
            - --scale-down-unneeded-time=10m
            - --scan-interval=5s
            - --v=4
          env:
            - name: HCLOUD_TOKEN
              valueFrom:
                secretKeyRef:
                  name: oxide-hcloud-token
                  key: token
            - name: HCLOUD_CLOUD_INIT
              valueFrom:
                configMapKeyRef:
                  name: oxide-talos-config
                  key: worker-config
            - name: HCLOUD_IMAGE
              value: "{}"
            - name: HCLOUD_NETWORK
              value: "{}-network"
            - name: HCLOUD_FIREWALL
              value: "{}-firewall"
            - name: HCLOUD_SSH_KEY
              value: "{}-oxide"
          resources:
            limits:
              cpu: 100m
              memory: 300Mi
            requests:
              cpu: 100m
              memory: 300Mi
"#,
            autoscaler_config.version,
            node_args.join("\n"),
            config
                .talos
                .hcloud_snapshot_id
                .as_deref()
                .unwrap_or("talos"),
            config.cluster_name,
            config.cluster_name,
            config.cluster_name
        );

        let temp_file = std::env::temp_dir().join("autoscaler-deployment.yaml");
        tokio::fs::write(&temp_file, deployment_yaml).await?;
        Resources::apply_manifest(&self.kubeconfig_path, &temp_file).await?;
        tokio::fs::remove_file(&temp_file).await?;

        Ok(())
    }

    /// Install autoscaler
    pub async fn install(config_path: &Path, output_dir: &Path) -> Result<()> {
        use anyhow::Context;

        let config = crate::config::ClusterConfig::from_file(config_path)
            .context("Failed to load configuration")?;

        let autoscaler_config = config
            .autoscaler
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Autoscaler not configured in cluster.yaml"))?;

        let kubeconfig_path = output_dir.join("kubeconfig");
        let worker_config_path = output_dir.join("worker.yaml");

        let autoscaler = Self::new(kubeconfig_path);
        autoscaler
            .install_autoscaler(&config, autoscaler_config, &worker_config_path)
            .await
    }

    /// Uninstall autoscaler
    pub async fn uninstall(output_dir: &Path) -> Result<()> {
        let kubeconfig_path = output_dir.join("kubeconfig");
        let autoscaler = Self::new(kubeconfig_path);
        autoscaler.uninstall_autoscaler().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::command::test_support::MockCommandRunner;
    use crate::utils::command::with_runner;
    use std::sync::Arc;

    #[tokio::test]
    async fn test_uninstall_autoscaler_calls_kubectl_delete() {
        let dir = tempfile::tempdir().unwrap();
        let kubeconfig = dir.path().join("kubeconfig");
        std::fs::write(&kubeconfig, "dummy").unwrap();

        let mock = Arc::new(MockCommandRunner::new());
        // All kubectl calls succeed (uninstall uses `let _ = ...` so errors are ignored anyway)
        mock.respond("kubectl", true, "", "");

        let autoscaler = Autoscaler::new(kubeconfig);
        let result = with_runner(mock.clone(), async {
            autoscaler.uninstall_autoscaler().await
        })
        .await;

        assert!(result.is_ok());
        let calls = mock.calls_for("kubectl");
        // Multiple kubectl delete commands should have been issued
        assert!(!calls.is_empty(), "expected kubectl to be called");
        let any_delete = calls
            .iter()
            .any(|c| c.args_str().iter().any(|a| a.as_ref() == "delete"));
        assert!(any_delete, "expected at least one 'delete' kubectl call");
    }

    #[tokio::test]
    async fn test_uninstall_autoscaler_missing_kubeconfig() {
        let mock = Arc::new(MockCommandRunner::new());
        let autoscaler = Autoscaler::new(PathBuf::from("/nonexistent/kubeconfig"));

        let result = with_runner(mock.clone(), async {
            autoscaler.uninstall_autoscaler().await
        })
        .await;

        assert!(result.is_err());
        let msg = format!("{}", result.unwrap_err());
        assert!(
            msg.contains("Kubeconfig not found") || msg.contains("kubeconfig"),
            "unexpected error: {}",
            msg
        );
    }

    #[tokio::test]
    async fn test_uninstall_autoscaler_removes_deployment() {
        let dir = tempfile::tempdir().unwrap();
        let kubeconfig = dir.path().join("kubeconfig");
        std::fs::write(&kubeconfig, "dummy").unwrap();

        let mock = Arc::new(MockCommandRunner::new());
        mock.respond("kubectl", true, "", "");

        let autoscaler = Autoscaler::new(kubeconfig);
        with_runner(mock.clone(), async {
            autoscaler.uninstall_autoscaler().await.unwrap();
        })
        .await;

        let calls = mock.calls_for("kubectl");
        // Verify the first delete removes the deployment
        let deploys_deleted = calls.iter().any(|c| {
            let args: Vec<_> = c.args_str();
            let args_str: Vec<&str> = args.iter().map(|s| s.as_ref()).collect();
            args_str.contains(&"deployment") && args_str.contains(&"cluster-autoscaler")
        });
        assert!(deploys_deleted, "expected deployment deletion call");
    }
}
