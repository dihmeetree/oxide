//! Local cluster scaling.
//!
//! `talosctl cluster create docker` is a one-shot bootstrap; it doesn't
//! expose an "add/remove worker" subcommand. To support `oxide scale` for
//! local clusters we manage worker containers ourselves.
//!
//! The strategy:
//!   * **Scale up:** `docker inspect` an existing worker to learn the exact
//!     image, network, env (incl. the base64-encoded `USERDATA` machine
//!     config), security options, mounts, and sysctls Talos uses. Then
//!     `docker run` additional containers with the same spec but with a
//!     fresh name/hostname and fresh anonymous volumes.
//!   * **Scale down:** drain the highest-numbered workers via `kubectl
//!     drain`, remove their docker containers (`docker rm -f`), and delete
//!     the corresponding Kubernetes node objects.
//!
//! Control-plane scaling is intentionally rejected: the Talos Docker
//! provisioner only supports a single CP (the `--controlplanes` flag was
//! dropped from `cluster create docker` in talosctl v1.13).
//!
//! All worker containers in a Talos Docker cluster share the same machine
//! config (the worker `USERDATA` is identical across workers), which makes
//! cloning the spec safe — only the hostname and Kubernetes node name need
//! to differ between instances.

use anyhow::{Context, Result};
use serde_json::Value;
use std::path::Path;
use std::time::Duration;
use tokio::time::sleep;
use tracing::{info, warn};

use crate::hcloud::server::NodeRole;
use crate::utils::command::CommandBuilder;

use super::LocalCluster;

/// Mount specification cloned from an existing worker container. We only
/// reproduce the *targets*; the *sources* must be fresh per-container so
/// each worker gets its own state directory.
#[derive(Debug, Clone)]
struct WorkerMount {
    /// "tmpfs" or "volume". Bind mounts are not used by the Talos docker
    /// provisioner so we don't model them here.
    kind: String,
    /// In-container target path (e.g. `/var`).
    target: String,
}

/// Snapshot of a worker container's runtime spec, sufficient to spawn new
/// workers with `docker run`. Captured from `docker inspect` of an existing
/// worker.
#[derive(Debug, Clone)]
pub(crate) struct WorkerSpec {
    image: String,
    network: String,
    env: Vec<String>,
    privileged: bool,
    readonly_rootfs: bool,
    security_opts: Vec<String>,
    cap_add: Vec<String>,
    mounts: Vec<WorkerMount>,
    sysctls: Vec<(String, String)>,
    labels: Vec<(String, String)>,
}

impl WorkerSpec {
    /// Parse a `docker inspect <container>` JSON document (the array form
    /// emitted by docker; we expect exactly one element).
    fn from_inspect_json(raw: &str) -> Result<Self> {
        let arr: Value =
            serde_json::from_str(raw).context("failed to parse docker inspect output as JSON")?;
        let entry = arr
            .as_array()
            .and_then(|a| a.first())
            .context("docker inspect returned no container data")?;

        let cfg = &entry["Config"];
        let host = &entry["HostConfig"];
        let image = cfg["Image"]
            .as_str()
            .context("missing Config.Image in docker inspect")?
            .to_string();

        let env = cfg["Env"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        // The original NetworkMode is "bridge" but the cluster-specific
        // network shows up in NetworkSettings.Networks. Pick the first
        // non-default network we find (Talos creates exactly one named
        // network per cluster).
        let network = entry["NetworkSettings"]["Networks"]
            .as_object()
            .and_then(|o| o.keys().next().cloned())
            .context("could not determine cluster network from docker inspect")?;

        let privileged = host["Privileged"].as_bool().unwrap_or(false);
        let readonly_rootfs = host["ReadonlyRootfs"].as_bool().unwrap_or(false);

        let security_opts = host["SecurityOpt"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        let cap_add = host["CapAdd"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        // Prefer HostConfig.Mounts (modern form); fall back to top-level
        // Mounts for compatibility with older Docker engines.
        let mounts: Vec<WorkerMount> =
            if let Some(arr) = host["Mounts"].as_array().filter(|a| !a.is_empty()) {
                arr.iter()
                    .filter_map(|m| {
                        let kind = m["Type"].as_str()?.to_string();
                        let target = m["Target"].as_str()?.to_string();
                        Some(WorkerMount { kind, target })
                    })
                    .collect()
            } else {
                entry["Mounts"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(|m| {
                                let kind = m["Type"].as_str()?.to_string();
                                let target = m["Destination"].as_str()?.to_string();
                                Some(WorkerMount { kind, target })
                            })
                            .collect()
                    })
                    .unwrap_or_default()
            };

        let sysctls = host["Sysctls"]
            .as_object()
            .map(|o| {
                o.iter()
                    .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string())))
                    .collect()
            })
            .unwrap_or_default();

        // Preserve labels so `docker ps --filter label=talos.owned=true`
        // (used by `oxide status`) continues to show new workers.
        let labels = cfg["Labels"]
            .as_object()
            .map(|o| {
                o.iter()
                    .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string())))
                    .collect()
            })
            .unwrap_or_default();

        Ok(Self {
            image,
            network,
            env,
            privileged,
            readonly_rootfs,
            security_opts,
            cap_add,
            mounts,
            sysctls,
            labels,
        })
    }

    /// Build the argument list for `docker run -d` to spawn a new worker
    /// container with the captured spec but a fresh name/hostname and
    /// fresh anonymous volumes.
    fn docker_run_args(&self, name: &str) -> Vec<String> {
        let mut args: Vec<String> = vec![
            "run".into(),
            "-d".into(),
            "--name".into(),
            name.into(),
            "--hostname".into(),
            name.into(),
            "--network".into(),
            self.network.clone(),
            "--restart".into(),
            "always".into(),
        ];

        if self.privileged {
            args.push("--privileged".into());
        }
        if self.readonly_rootfs {
            args.push("--read-only".into());
        }
        for opt in &self.security_opts {
            args.push("--security-opt".into());
            args.push(opt.clone());
        }
        for cap in &self.cap_add {
            args.push("--cap-add".into());
            args.push(cap.clone());
        }
        for m in &self.mounts {
            match m.kind.as_str() {
                "tmpfs" => {
                    args.push("--tmpfs".into());
                    args.push(m.target.clone());
                }
                "volume" => {
                    // No source -> docker creates a fresh anonymous
                    // volume for this container, which is what we want
                    // (each worker has its own /var, /etc/cni, etc.).
                    args.push("--mount".into());
                    args.push(format!("type=volume,target={}", m.target));
                }
                other => {
                    warn!(
                        "Skipping unrecognized mount type '{other}' for target {}; \
                         the new worker may misbehave if this mount is required",
                        m.target
                    );
                }
            }
        }
        for (k, v) in &self.sysctls {
            args.push("--sysctl".into());
            args.push(format!("{k}={v}"));
        }
        for (k, v) in &self.labels {
            args.push("--label".into());
            args.push(format!("{k}={v}"));
        }
        for env in &self.env {
            args.push("--env".into());
            args.push(env.clone());
        }
        args.push(self.image.clone());
        args
    }
}

impl LocalCluster {
    /// Public scale entry point dispatched from `Cluster::scale`.
    pub async fn scale(
        &self,
        role: NodeRole,
        target: u32,
        force: bool,
        timeout: u64,
    ) -> Result<()> {
        if matches!(role, NodeRole::ControlPlane) {
            anyhow::bail!(
                "scaling control planes is not supported for local clusters; \
                 the Talos Docker provisioner only supports a single control plane"
            );
        }

        let workers = self.list_worker_containers().await?;
        let current = workers.len() as u32;
        info!(
            "Local cluster '{}': currently {current} worker(s), target {target}",
            self.config.cluster_name
        );
        if target == current {
            info!("Already at the requested worker count; nothing to do");
            return Ok(());
        }

        if target > current {
            self.scale_up_workers(&workers, target - current, timeout)
                .await
        } else {
            self.scale_down_workers(&workers, current - target, force, timeout)
                .await
        }
    }

    /// List all worker containers belonging to this cluster, sorted by
    /// their numeric suffix so scale-down predictably removes the
    /// highest-numbered workers first.
    async fn list_worker_containers(&self) -> Result<Vec<String>> {
        let out = CommandBuilder::new("docker")
            .args([
                "ps",
                "-a",
                "--filter",
                &format!("label=talos.cluster.name={}", self.config.cluster_name),
                "--filter",
                "label=talos.type=worker",
                "--format",
                "{{.Names}}",
            ])
            .context("docker ps failed while listing worker containers")
            .output()
            .await?;
        let mut names: Vec<String> = out
            .stdout
            .lines()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect();
        names.sort_by_key(|n| worker_index(n).unwrap_or(u32::MAX));
        Ok(names)
    }

    /// Inspect an existing worker and use it as the template for new
    /// containers.
    async fn worker_template_spec(&self, name: &str) -> Result<WorkerSpec> {
        let raw = CommandBuilder::new("docker")
            .args(["inspect", name])
            .context("docker inspect failed")
            .run()
            .await?;
        WorkerSpec::from_inspect_json(&raw)
            .with_context(|| format!("failed to parse docker inspect output for {name}"))
    }

    async fn scale_up_workers(&self, existing: &[String], delta: u32, timeout: u64) -> Result<()> {
        let template_name = existing.first().context(
            "cannot scale up: no existing worker container to clone; \
             create the cluster first or scale by re-running `oxide create`",
        )?;
        let spec = self.worker_template_spec(template_name).await?;

        let start_idx = existing
            .iter()
            .filter_map(|n| worker_index(n))
            .max()
            .unwrap_or(0)
            + 1;

        let mut spawned: Vec<String> = Vec::with_capacity(delta as usize);
        for offset in 0..delta {
            let name = format!("{}-worker-{}", self.config.cluster_name, start_idx + offset);
            info!("Spawning worker container {name}");
            CommandBuilder::new("docker")
                .args(spec.docker_run_args(&name).iter().map(String::as_str))
                .context("docker run failed for new worker")
                .run_silent()
                .await?;
            spawned.push(name);
        }

        let kubeconfig = self.kubeconfig_path();
        if !kubeconfig.exists() {
            warn!(
                "kubeconfig not found at {}; skipping node-ready wait",
                kubeconfig.display()
            );
            return Ok(());
        }

        for name in &spawned {
            wait_for_node_ready(&kubeconfig, name, timeout).await?;
        }
        info!("[OK] Scaled local cluster up by {delta} worker(s)");
        Ok(())
    }

    async fn scale_down_workers(
        &self,
        existing: &[String],
        delta: u32,
        force: bool,
        timeout: u64,
    ) -> Result<()> {
        if delta as usize > existing.len() {
            anyhow::bail!(
                "cannot remove {delta} workers: only {} present",
                existing.len()
            );
        }
        let kubeconfig = self.kubeconfig_path();
        let victims: Vec<String> = existing
            .iter()
            .rev()
            .take(delta as usize)
            .cloned()
            .collect();
        for name in &victims {
            self.remove_worker(name, &kubeconfig, force, timeout)
                .await?;
        }
        info!("[OK] Scaled local cluster down by {delta} worker(s)");
        Ok(())
    }

    async fn remove_worker(
        &self,
        name: &str,
        kubeconfig: &Path,
        force: bool,
        timeout: u64,
    ) -> Result<()> {
        if kubeconfig.exists() {
            // Best-effort drain. Failures are warnings (and become hard
            // errors only when --force is *not* set) because we still
            // want to be able to remove orphaned containers whose
            // matching node object is already gone.
            info!("Cordoning {name}");
            let cordon = CommandBuilder::new("kubectl")
                .args(["cordon", name])
                .kubeconfig(kubeconfig)
                .run_silent()
                .await;
            if let Err(e) = cordon {
                if force {
                    warn!("kubectl cordon {name} failed (continuing due to --force): {e:#}");
                } else {
                    return Err(e).with_context(|| {
                        format!("failed to cordon {name}; rerun with --force to bypass")
                    });
                }
            }

            info!("Draining {name} (timeout {timeout}s)");
            let drain = CommandBuilder::new("kubectl")
                .args([
                    "drain",
                    name,
                    "--ignore-daemonsets",
                    "--delete-emptydir-data",
                    "--force",
                    &format!("--timeout={timeout}s"),
                ])
                .kubeconfig(kubeconfig)
                .run_silent()
                .await;
            if let Err(e) = drain {
                if force {
                    warn!("kubectl drain {name} failed (continuing due to --force): {e:#}");
                } else {
                    return Err(e).with_context(|| {
                        format!("failed to drain {name}; rerun with --force to bypass")
                    });
                }
            }
        }

        info!("Removing docker container {name}");
        CommandBuilder::new("docker")
            .args(["rm", "-f", "-v", name])
            .context("docker rm -f failed")
            .run_silent()
            .await?;

        if kubeconfig.exists() {
            // Delete the corresponding node object so `kubectl get nodes`
            // stays clean. Best-effort: a NotFound is fine.
            let del = CommandBuilder::new("kubectl")
                .args(["delete", "node", name, "--ignore-not-found"])
                .kubeconfig(kubeconfig)
                .run_silent()
                .await;
            if let Err(e) = del {
                warn!("kubectl delete node {name} failed: {e:#}");
            }
        }
        Ok(())
    }
}

/// Extract the trailing numeric index from a container name like
/// `mycluster-worker-3` -> 3.
fn worker_index(name: &str) -> Option<u32> {
    name.rsplit('-').next().and_then(|s| s.parse::<u32>().ok())
}

/// Poll `kubectl get node` until the named node reports `Ready=True` or
/// the timeout (in seconds) elapses. We poll instead of `kubectl wait`
/// because the node object itself appears asynchronously after the
/// container starts.
async fn wait_for_node_ready(kubeconfig: &Path, name: &str, timeout_secs: u64) -> Result<()> {
    let deadline = std::time::Instant::now() + Duration::from_secs(timeout_secs);
    info!("Waiting for node {name} to become Ready (timeout {timeout_secs}s)");
    loop {
        let out = CommandBuilder::new("kubectl")
            .args([
                "get",
                "node",
                name,
                "-o",
                "jsonpath={range .status.conditions[?(@.type==\"Ready\")]}{.status}{end}",
            ])
            .kubeconfig(kubeconfig)
            .output()
            .await?;
        if out.success && out.stdout.trim() == "True" {
            info!("Node {name} is Ready");
            return Ok(());
        }
        if std::time::Instant::now() >= deadline {
            anyhow::bail!(
                "timed out after {timeout_secs}s waiting for node {name} to become Ready"
            );
        }
        sleep(Duration::from_secs(5)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_INSPECT: &str = r#"[{
        "Config": {
            "Image": "ghcr.io/siderolabs/talos:v1.13.0",
            "Env": ["PLATFORM=container", "USERDATA=abc"],
            "Labels": {"talos.cluster.name": "test", "talos.type": "worker", "talos.owned": "true"}
        },
        "HostConfig": {
            "Privileged": true,
            "ReadonlyRootfs": true,
            "SecurityOpt": ["seccomp:unconfined", "label=disable"],
            "CapAdd": null,
            "Mounts": [
                {"Type": "tmpfs", "Target": "/run"},
                {"Type": "volume", "Target": "/var"}
            ],
            "Sysctls": {"net.ipv6.conf.all.disable_ipv6": "0"}
        },
        "NetworkSettings": {"Networks": {"test": {}}}
    }]"#;

    #[test]
    fn parses_worker_inspect() {
        let s = WorkerSpec::from_inspect_json(SAMPLE_INSPECT).unwrap();
        assert_eq!(s.image, "ghcr.io/siderolabs/talos:v1.13.0");
        assert_eq!(s.network, "test");
        assert_eq!(s.env.len(), 2);
        assert!(s.privileged);
        assert!(s.readonly_rootfs);
        assert_eq!(s.security_opts, vec!["seccomp:unconfined", "label=disable"]);
        assert_eq!(s.mounts.len(), 2);
        assert_eq!(s.sysctls.len(), 1);
        assert_eq!(s.labels.len(), 3);
    }

    #[test]
    fn docker_run_args_include_required_flags() {
        let s = WorkerSpec::from_inspect_json(SAMPLE_INSPECT).unwrap();
        let args = s.docker_run_args("test-worker-2");
        let joined = args.join(" ");
        assert!(joined.starts_with("run -d --name test-worker-2 --hostname test-worker-2"));
        assert!(joined.contains("--network test"));
        assert!(joined.contains("--privileged"));
        assert!(joined.contains("--read-only"));
        assert!(joined.contains("--tmpfs /run"));
        assert!(joined.contains("type=volume,target=/var"));
        assert!(joined.contains("--sysctl net.ipv6.conf.all.disable_ipv6=0"));
        assert!(joined.contains("--label talos.owned=true"));
        assert!(joined.contains("--env PLATFORM=container"));
        assert!(joined.contains("--env USERDATA=abc"));
        assert!(joined.ends_with("ghcr.io/siderolabs/talos:v1.13.0"));
    }

    #[test]
    fn worker_index_parses_trailing_number() {
        assert_eq!(worker_index("foo-worker-1"), Some(1));
        assert_eq!(worker_index("foo-worker-42"), Some(42));
        assert_eq!(worker_index("foo-controlplane-1"), Some(1));
        assert_eq!(worker_index("no-suffix"), None);
    }
}
