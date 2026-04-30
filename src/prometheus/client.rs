//! Persistent Prometheus HTTP client backed by a long-lived `kubectl port-forward`.
//!
//! Replaces the old per-call `kubectl exec wget` pattern, which was the
//! dominant cost on the dashboard's first cache refresh: each Prometheus
//! query spawned a fresh `kubectl` process, performed a TLS handshake to the
//! API server, started an exec session inside the Prometheus pod, ran wget,
//! and tore everything down. With many series × per-pod fan-out this added up
//! to tens of seconds per refresh.
//!
//! This module:
//! 1. Spawns a single `kubectl port-forward svc/prometheus-kube-prometheus-prometheus :9090`
//!    against an OS-assigned local port and parses the chosen port from stdout.
//! 2. Uses `reqwest` to talk to `http://127.0.0.1:<port>/api/v1/...` directly.
//! 3. Caches one client per `kubeconfig_path` in a global registry so all
//!    callers share the same forwarder and HTTP connection pool.
//!
//! Each `PromClient` owns the child process and kills it on `Drop` so we don't
//! leak port-forwards across cache rebuilds. If a forwarder dies (e.g. the
//! Prometheus pod restarts), the next request transparently re-establishes it.

use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{Mutex, OnceCell, RwLock};
use tokio::time::timeout;
use tracing::{debug, warn};

use super::{PrometheusRangeResponse, PrometheusResponse};

/// Default service to forward; aligns with kube-prometheus-stack defaults.
const PROMETHEUS_SERVICE: &str = "svc/prometheus-kube-prometheus-prometheus";
const PROMETHEUS_NAMESPACE: &str = "monitoring";
const PROMETHEUS_PORT: u16 = 9090;

/// HTTP read/connect timeout for individual queries.
const QUERY_TIMEOUT: Duration = Duration::from_secs(20);
/// How long to wait for `kubectl port-forward` to print its bound port.
const PORT_FORWARD_READY_TIMEOUT: Duration = Duration::from_secs(15);

/// Persistent Prometheus client over a long-lived port-forward.
pub struct PromClient {
    kubeconfig: PathBuf,
    inner: RwLock<Option<ActiveForward>>,
    /// Serializes concurrent forwarder rebuild attempts.
    rebuild_lock: Mutex<()>,
    http: reqwest::Client,
}

struct ActiveForward {
    /// Local port `kubectl port-forward` is listening on.
    port: u16,
    /// Child handle - kept alive for the duration of the forwarder; killed on drop.
    _child: Child,
}

impl PromClient {
    /// Build a new client; does not spawn a port-forward yet (lazy on first request).
    fn new(kubeconfig: PathBuf) -> Self {
        let http = reqwest::Client::builder()
            .timeout(QUERY_TIMEOUT)
            .pool_max_idle_per_host(8)
            // Localhost only - HTTP, no TLS overhead.
            .no_proxy()
            .build()
            .expect("reqwest client");
        Self {
            kubeconfig,
            inner: RwLock::new(None),
            rebuild_lock: Mutex::new(()),
            http,
        }
    }

    /// Resolve the local port, spawning the port-forward if needed.
    async fn ensure_port(&self) -> Result<u16> {
        if let Some(active) = self.inner.read().await.as_ref() {
            return Ok(active.port);
        }
        // Slow path: serialize rebuild so we only spawn one child even if
        // multiple callers race here on first request.
        let _guard = self.rebuild_lock.lock().await;
        if let Some(active) = self.inner.read().await.as_ref() {
            return Ok(active.port);
        }
        let active = spawn_port_forward(&self.kubeconfig).await?;
        let port = active.port;
        *self.inner.write().await = Some(active);
        Ok(port)
    }

    /// Drop the current forwarder so the next call rebuilds. Used when an
    /// HTTP request fails (Prometheus pod restarted, etc.).
    async fn invalidate(&self) {
        let _guard = self.rebuild_lock.lock().await;
        *self.inner.write().await = None;
    }

    /// Build a `Url` for a given path against the active port-forward.
    async fn url(&self, path: &str, query: &[(&str, &str)]) -> Result<String> {
        let port = self.ensure_port().await?;
        let mut url = format!("http://127.0.0.1:{}{}", port, path);
        if !query.is_empty() {
            url.push('?');
            for (i, (k, v)) in query.iter().enumerate() {
                if i > 0 {
                    url.push('&');
                }
                url.push_str(k);
                url.push('=');
                url.push_str(&urlencoding::encode(v));
            }
        }
        Ok(url)
    }

    /// Issue an HTTP GET; on connection failure, drop and rebuild the
    /// forwarder once before bubbling up the error.
    async fn get_text(&self, path: &str, query: &[(&str, &str)]) -> Result<String> {
        for attempt in 0..2 {
            let url = self.url(path, query).await?;
            match self.http.get(&url).send().await {
                Ok(resp) => {
                    let status = resp.status();
                    if status.is_success() {
                        return resp.text().await.context("read prometheus response body");
                    }
                    let body = resp.text().await.unwrap_or_default();
                    anyhow::bail!("prometheus returned status {}: {}", status, body);
                }
                Err(e) if attempt == 0 => {
                    debug!("PromClient request failed ({}); rebuilding forwarder", e);
                    self.invalidate().await;
                    continue;
                }
                Err(e) => return Err(e).context("prometheus HTTP request"),
            }
        }
        unreachable!()
    }

    /// Run an instant query and return the first scalar (if any).
    pub async fn instant_scalar(&self, query: &str) -> Result<Option<f64>> {
        let body = self.get_text("/api/v1/query", &[("query", query)]).await?;
        let resp: PrometheusResponse =
            serde_json::from_str(&body).context("parse instant query response")?;
        if resp.status != "success" || resp.data.result.is_empty() {
            return Ok(None);
        }
        Ok(resp.data.result[0].value.1.parse::<f64>().ok())
    }

    /// Run a range query and return the first series' (timestamp, value) pairs.
    pub async fn range_single(
        &self,
        query: &str,
        duration_secs: u64,
        step: &str,
    ) -> Result<Vec<(i64, f64)>> {
        let series = self.range_multi(query, duration_secs, step).await?;
        Ok(series
            .into_iter()
            .next()
            .map(|(_, v)| v)
            .unwrap_or_default())
    }

    /// Run a range query and return every series with its label set.
    pub async fn range_multi(
        &self,
        query: &str,
        duration_secs: u64,
        step: &str,
    ) -> Result<Vec<(HashMap<String, String>, Vec<(i64, f64)>)>> {
        let end = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let start = end.saturating_sub(duration_secs);
        let start_s = start.to_string();
        let end_s = end.to_string();
        let body = self
            .get_text(
                "/api/v1/query_range",
                &[
                    ("query", query),
                    ("start", &start_s),
                    ("end", &end_s),
                    ("step", step),
                ],
            )
            .await?;
        let resp: PrometheusRangeResponse =
            serde_json::from_str(&body).context("parse range query response")?;
        if resp.status != "success" {
            return Ok(Vec::new());
        }
        Ok(resp
            .data
            .result
            .into_iter()
            .map(|r| {
                let values = r
                    .values
                    .into_iter()
                    .map(|(t, v)| (t as i64, v.parse::<f64>().unwrap_or(0.0)))
                    .collect();
                (r.metric, values)
            })
            .collect())
    }

    /// Fetch raw JSON text from a Prometheus API endpoint (e.g. `/api/v1/alerts`).
    pub async fn get_json(&self, path: &str) -> Result<String> {
        self.get_text(path, &[]).await
    }
}

/// Spawn `kubectl port-forward` and parse the bound local port from stdout.
async fn spawn_port_forward(kubeconfig: &Path) -> Result<ActiveForward> {
    let mut child = Command::new("kubectl")
        .arg("--kubeconfig")
        .arg(kubeconfig)
        .arg("port-forward")
        .arg("-n")
        .arg(PROMETHEUS_NAMESPACE)
        .arg(PROMETHEUS_SERVICE)
        // Asking for ":9090" lets the kernel pick a free local port and
        // kubectl prints "Forwarding from 127.0.0.1:NNNN -> 9090".
        .arg(format!(":{}", PROMETHEUS_PORT))
        .arg("--address")
        .arg("127.0.0.1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .context("spawn kubectl port-forward")?;

    let stdout = child.stdout.take().context("kubectl port-forward stdout")?;
    let mut reader = BufReader::new(stdout).lines();

    let port = timeout(PORT_FORWARD_READY_TIMEOUT, async {
        while let Some(line) = reader.next_line().await? {
            if let Some(port) = parse_port_from_kubectl_line(&line) {
                return Ok::<u16, anyhow::Error>(port);
            }
        }
        anyhow::bail!("kubectl port-forward exited before printing a port")
    })
    .await
    .context("timed out waiting for kubectl port-forward")?
    .context("kubectl port-forward failed to bind")?;

    // Drain remaining stdout so kubectl's pipe never blocks.
    tokio::spawn(async move {
        while let Ok(Some(line)) = reader.next_line().await {
            debug!("kubectl port-forward: {}", line);
        }
    });
    if let Some(stderr) = child.stderr.take() {
        let mut stderr_reader = BufReader::new(stderr).lines();
        tokio::spawn(async move {
            while let Ok(Some(line)) = stderr_reader.next_line().await {
                if !line.is_empty() {
                    warn!("kubectl port-forward stderr: {}", line);
                }
            }
        });
    }

    Ok(ActiveForward {
        port,
        _child: child,
    })
}

fn parse_port_from_kubectl_line(line: &str) -> Option<u16> {
    // Format: "Forwarding from 127.0.0.1:54321 -> 9090"
    let prefix = "Forwarding from 127.0.0.1:";
    let rest = line.strip_prefix(prefix)?;
    let port_str = rest.split_whitespace().next()?;
    port_str.parse::<u16>().ok()
}

/// Global registry of clients keyed by kubeconfig path. Allows every
/// caller (cache.rs fetchers, route handlers, etc.) to share the same
/// port-forward + reqwest connection pool without wiring a parameter through
/// every signature.
static REGISTRY: OnceCell<RwLock<HashMap<PathBuf, Arc<PromClient>>>> = OnceCell::const_new();

async fn registry() -> &'static RwLock<HashMap<PathBuf, Arc<PromClient>>> {
    REGISTRY
        .get_or_init(|| async { RwLock::new(HashMap::new()) })
        .await
}

/// Fetch (or create) the shared client for a given kubeconfig path.
pub async fn shared_client(kubeconfig: &Path) -> Arc<PromClient> {
    let key = kubeconfig.to_path_buf();
    {
        let map = registry().await.read().await;
        if let Some(c) = map.get(&key) {
            return Arc::clone(c);
        }
    }
    let mut map = registry().await.write().await;
    if let Some(c) = map.get(&key) {
        return Arc::clone(c);
    }
    let client = Arc::new(PromClient::new(key.clone()));
    map.insert(key, Arc::clone(&client));
    client
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_kubectl_port_line() {
        assert_eq!(
            parse_port_from_kubectl_line("Forwarding from 127.0.0.1:54321 -> 9090"),
            Some(54321)
        );
        assert_eq!(
            parse_port_from_kubectl_line("Forwarding from [::1]:54321 -> 9090"),
            None
        );
        assert_eq!(parse_port_from_kubectl_line("garbage"), None);
    }
}
