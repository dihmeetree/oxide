/// Local cluster provisioning via Talos's built-in Docker provisioner.
///
/// Hetzner Cloud is the original deployment target for `oxide`, but local
/// clusters are very useful for dev/CI iteration. Rather than re-implement
/// Talos's Docker support, we shell out to `talosctl cluster create
/// --provisioner docker`, which already knows how to:
///
///  * pull the appropriate Talos image,
///  * wire up a Docker bridge network,
///  * launch one container per control-plane / worker node,
///  * bootstrap etcd, and
///  * write a `talosconfig` for further talos-API access.
///
/// The kubeconfig is then exported from the cluster into our standard
/// `<output_dir>/kubeconfig` location so the rest of the optional-component
/// machinery (Cilium, metrics-server, Prometheus, ...) works unchanged.
use anyhow::{Context, Result};
use std::path::PathBuf;
use tokio::process::Command;
use tracing::{info, warn};

use crate::cilium::{Cilium, PUBLIC_UPSTREAM_DNS};
use crate::config::{ClusterConfig, DockerConfig};
use crate::helm::Helm;
use crate::k8s::KubernetesClient;
use crate::metrics_server::MetricsServer;
use crate::prometheus::Prometheus;
use crate::talos::TalosClient;
use crate::utils::command::CommandBuilder;

mod scaling;

/// Local cluster manager.
pub struct LocalCluster {
    config: ClusterConfig,
    output_dir: PathBuf,
}

impl LocalCluster {
    pub fn new(config: ClusterConfig, output_dir: PathBuf) -> Self {
        Self { config, output_dir }
    }

    /// Path to the talosconfig file used for the local cluster.
    fn talosconfig_path(&self) -> PathBuf {
        self.output_dir.join("talosconfig")
    }

    /// Path to the exported kubeconfig used for the local cluster.
    fn kubeconfig_path(&self) -> PathBuf {
        self.output_dir.join("kubeconfig")
    }

    /// Total number of control plane nodes configured.
    fn control_plane_count(&self) -> u32 {
        self.config.control_planes.iter().map(|p| p.count).sum()
    }

    /// Total number of worker nodes configured.
    fn worker_count(&self) -> u32 {
        self.config.workers.iter().map(|p| p.count).sum()
    }

    /// Create the local cluster.
    pub async fn create(&self) -> Result<()> {
        info!("Creating local Talos cluster: {}", self.config.cluster_name);

        // Ensure output directory exists (talosconfig + kubeconfig will live here).
        tokio::fs::create_dir_all(&self.output_dir)
            .await
            .context("Failed to create output directory")?;

        // Prerequisite checks: talosctl + kubectl + helm + docker.
        TalosClient::check_talosctl_installed()
            .await
            .context("talosctl is required for local clusters")?;
        KubernetesClient::check_kubectl_installed()
            .await
            .context("kubectl is required")?;
        Helm::check_installed().await.context("helm is required")?;
        check_docker_running().await?;

        // Create the cluster via talosctl. This is a long-running operation
        // (image pull + container start + etcd bootstrap).
        self.run_talosctl_cluster_create().await?;

        // Export a stand-alone kubeconfig pointing at the new cluster so the
        // rest of our tooling can use it without touching $HOME/.kube/config.
        self.export_kubeconfig().await?;

        info!(
            "[OK] Local cluster '{}' is up. talosconfig: {} kubeconfig: {}",
            self.config.cluster_name,
            self.talosconfig_path().display(),
            self.kubeconfig_path().display()
        );

        // Optional components. The Cilium step replaces Talos's default CNI
        // (flannel for the docker provisioner) and works the same way as on
        // Hetzner because it operates purely against the kubeconfig.
        self.install_optional_components().await?;

        info!("To access the cluster:");
        info!("  export KUBECONFIG={}", self.kubeconfig_path().display());
        info!("  kubectl get nodes");

        Ok(())
    }

    /// Build and execute `talosctl cluster create docker ...`.
    ///
    /// Note: the Talos Docker provisioner only supports a single control
    /// plane node (the `--controlplanes` flag was dropped from the
    /// `cluster create docker` subcommand in talosctl v1.13). We therefore
    /// reject configurations that ask for more than one CP rather than
    /// silently downscale.
    async fn run_talosctl_cluster_create(&self) -> Result<()> {
        let cp = self.control_plane_count();
        let workers = self.worker_count();
        if cp == 0 {
            anyhow::bail!("at least one control plane node is required");
        }
        if cp > 1 {
            anyhow::bail!(
                "talosctl's Docker provisioner only supports a single control plane \
                 (got {cp}). Set control_planes[*].count to 1 for local clusters."
            );
        }

        let kube_version = self.config.talos.kubernetes_version.clone();
        let image = self
            .config
            .docker
            .as_ref()
            .and_then(|d: &DockerConfig| d.image.clone())
            .unwrap_or_else(|| format!("ghcr.io/siderolabs/talos:{}", self.config.talos.version));

        // `--name` is a global flag on `talosctl cluster`, so it goes
        // *before* the `create docker` subcommand, not after.
        let mut args: Vec<String> = vec![
            "cluster".into(),
            "--name".into(),
            self.config.cluster_name.clone(),
            "create".into(),
            "docker".into(),
            "--workers".into(),
            workers.to_string(),
            "--kubernetes-version".into(),
            kube_version,
            "--image".into(),
            image,
            "--talosconfig-destination".into(),
            self.talosconfig_path().display().to_string(),
        ];

        if let Some(docker) = &self.config.docker {
            if let Some(port) = docker.api_port {
                args.push("--exposed-ports".into());
                args.push(format!("{port}:{port}/tcp"));
            }
            if let Some(cidr) = &docker.network_cidr {
                args.push("--subnet".into());
                args.push(cidr.clone());
            }
        }

        // Disable Talos's built-in CNI/proxy so Cilium can take over. Even
        // when Cilium is not installed downstream this still works: Talos
        // simply ships without a CNI, which is fine for users who manage
        // their own networking.
        //
        // `--config-patch` is a `stringArray` on the docker subcommand, so
        // we pass each JSON-patch operation as its own argument rather than
        // a single multi-op array.
        for patch in CILIUM_FRIENDLY_PATCHES {
            args.push("--config-patch".into());
            args.push((*patch).to_string());
        }

        info!("Running: talosctl {}", args.join(" "));

        CommandBuilder::new("talosctl")
            .args(args.iter().map(String::as_str))
            .context("talosctl cluster create failed")
            .run_silent()
            .await?;
        Ok(())
    }

    /// Export the kubeconfig for the freshly-created cluster into our
    /// standard `<output_dir>/kubeconfig` path.
    async fn export_kubeconfig(&self) -> Result<()> {
        // Remove any stale file so `talosctl kubeconfig` does not refuse to
        // overwrite it.
        let kubeconfig = self.kubeconfig_path();
        if kubeconfig.exists() {
            tokio::fs::remove_file(&kubeconfig)
                .await
                .with_context(|| format!("Failed to remove stale {}", kubeconfig.display()))?;
        }

        // The docker provisioner always exposes the Talos API on a
        // forwarded host-port (the endpoint stored in talosconfig is
        // `127.0.0.1:<port>`), so 127.0.0.1 is the correct node target for
        // any subsequent talosctl call against this cluster.
        CommandBuilder::new("talosctl")
            .args([
                "kubeconfig",
                kubeconfig.to_str().context("invalid kubeconfig path")?,
                "--talosconfig",
                self.talosconfig_path()
                    .to_str()
                    .context("invalid talosconfig path")?,
                "--nodes",
                "127.0.0.1",
                "--merge=false",
                "--force",
            ])
            .context("Failed to export kubeconfig from talosctl")
            .run_silent()
            .await?;

        Ok(())
    }

    /// Install the same optional components as the Hetzner flow: Cilium,
    /// metrics-server, Prometheus. The autoscaler is intentionally skipped
    /// because it requires the Hetzner Cloud API.
    async fn install_optional_components(&self) -> Result<()> {
        let kubeconfig = self.kubeconfig_path();
        let cp_count = self.control_plane_count();

        info!("Installing Cilium CNI...");
        let cilium = Cilium::new(
            self.config.cilium.clone(),
            kubeconfig.clone(),
            cp_count,
            PUBLIC_UPSTREAM_DNS,
        );
        cilium.install().await?;
        cilium.wait_for_ready(300).await?;

        if let Some(ms) = &self.config.metrics_server {
            if ms.enabled {
                info!("Installing metrics-server...");
                MetricsServer::install(&self.output_dir).await?;
            }
        }

        if let Some(prom) = &self.config.prometheus {
            if prom.enabled {
                info!("Installing Prometheus monitoring stack...");
                // Local clusters don't ship with a default storage class.
                // If the user (or our default config) leaves persistent
                // storage enabled, the Prometheus + Grafana + Alertmanager
                // PVCs stay Pending forever and `wait_for_ready` hangs.
                // Force-disable here and warn so the install completes.
                let mut cfg = self.config.clone();
                if let Some(p) = cfg.prometheus.as_mut() {
                    if p.enable_persistent_storage {
                        warn!(
                            "Forcing prometheus.enable_persistent_storage=false for local \
                             clusters (no default StorageClass). Set it to false in your \
                             cluster.yaml to silence this warning, or install a CSI driver \
                             before re-running."
                        );
                        p.enable_persistent_storage = false;
                    }
                }
                let cfg_path = self.output_dir.join("config-snapshot.yaml");
                tokio::fs::write(&cfg_path, serde_yaml::to_string(&cfg)?)
                    .await
                    .context("Failed to snapshot config for Prometheus install")?;
                Prometheus::install(&cfg_path, &self.output_dir).await?;
            }
        }

        if let Some(autoscaler) = &self.config.autoscaler {
            if autoscaler.enabled {
                anyhow::bail!(
                    "cluster autoscaler is not supported for local clusters; \
                     this should have been caught at config validation"
                );
            }
        }

        Ok(())
    }

    /// Tear the local cluster down. Best-effort: we still remove the
    /// generated config files even if the talosctl call fails (e.g. because
    /// the cluster was already destroyed manually).
    pub async fn destroy(&self) -> Result<()> {
        info!("Destroying local cluster: {}", self.config.cluster_name);
        TalosClient::check_talosctl_installed().await?;

        let res = CommandBuilder::new("talosctl")
            .args(["cluster", "--name", &self.config.cluster_name, "destroy"])
            .context("talosctl cluster destroy failed")
            .run_silent()
            .await;

        if let Err(e) = res {
            tracing::warn!(
                "talosctl cluster destroy reported an error (continuing with file cleanup): {e:#}"
            );
        }

        if self.output_dir.exists() {
            info!("Removing output directory: {}", self.output_dir.display());
            tokio::fs::remove_dir_all(&self.output_dir)
                .await
                .with_context(|| {
                    format!("Failed to remove output dir {}", self.output_dir.display())
                })?;
        }

        info!("[OK] Local cluster destroyed");
        Ok(())
    }

    /// Show the status of the local cluster by listing the docker
    /// containers Talos created and the nodes Kubernetes reports.
    pub async fn status(&self) -> Result<()> {
        info!("Status for local cluster '{}':", self.config.cluster_name);

        // List talosctl-managed containers. We deliberately use plain
        // `docker ps` (rather than the docker SDK) to keep the dependency
        // surface small; the same is true elsewhere in this codebase.
        let output = CommandBuilder::new("docker")
            .args([
                "ps",
                "--filter",
                "label=talos.owned=true",
                "--filter",
                &format!("name={}", self.config.cluster_name),
                "--format",
                "table {{.Names}}\t{{.Status}}\t{{.Ports}}",
            ])
            .context("docker ps failed")
            .output()
            .await?;
        if !output.stdout.trim().is_empty() {
            info!("Docker containers:\n{}", output.stdout);
        }

        let kubeconfig = self.kubeconfig_path();
        if kubeconfig.exists() {
            let nodes = CommandBuilder::new("kubectl")
                .args(["get", "nodes", "-o", "wide"])
                .kubeconfig(&kubeconfig)
                .context("kubectl get nodes failed")
                .output()
                .await?;
            info!("Kubernetes nodes:\n{}", nodes.stdout);
        } else {
            info!(
                "No kubeconfig at {} (cluster may not be running)",
                kubeconfig.display()
            );
        }

        Ok(())
    }
}

/// Verify Docker is installed and reachable. The error message is meant to
/// be actionable: missing Docker is the single most common reason local
/// cluster creation fails on a fresh machine.
async fn check_docker_running() -> Result<()> {
    let res = Command::new("docker")
        .arg("info")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await;

    match res {
        Ok(status) if status.success() => Ok(()),
        Ok(_) => anyhow::bail!(
            "`docker info` failed. Make sure the Docker daemon is running and your user has access to it."
        ),
        Err(e) => anyhow::bail!(
            "Failed to invoke `docker`: {e}. Install Docker (or Docker Desktop) to use local clusters."
        ),
    }
}

/// Talos machine-config patches applied to every node in a local cluster so
/// the cluster comes up without an opinionated CNI / kube-proxy. Cilium is
/// then installed on top, matching the Hetzner setup.
///
/// We use Talos's strategic-merge patch format (a YAML/JSON document that
/// gets merged into the generated machine config) rather than RFC6902
/// JSON-patch operations, because talosctl rejects RFC6902 patches when
/// applying a single `--config-patch` to a multi-document config (control
/// plane + worker).
const CILIUM_FRIENDLY_PATCHES: &[&str] =
    &[r#"{"cluster":{"network":{"cni":{"name":"none"}},"proxy":{"disabled":true}}}"#];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ClusterConfig;

    #[test]
    fn local_cluster_paths_use_output_dir() {
        let cfg = ClusterConfig::example_local();
        let lc = LocalCluster::new(cfg, PathBuf::from("/tmp/oxide-test"));
        assert_eq!(
            lc.kubeconfig_path(),
            PathBuf::from("/tmp/oxide-test/kubeconfig")
        );
        assert_eq!(
            lc.talosconfig_path(),
            PathBuf::from("/tmp/oxide-test/talosconfig")
        );
    }

    #[test]
    fn node_counts_sum_pools() {
        let cfg = ClusterConfig::example_local();
        let lc = LocalCluster::new(cfg, PathBuf::from("/tmp"));
        assert_eq!(lc.control_plane_count(), 1);
        assert_eq!(lc.worker_count(), 1);
    }
}
