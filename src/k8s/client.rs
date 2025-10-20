/// Kubernetes operations client
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;
use tokio::process::Command;

/// Kubernetes client for kubectl operations
pub struct KubernetesClient;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PodInfo {
    pub name: String,
    pub namespace: String,
    pub node_name: String,
    pub status: String,
    pub restarts: u32,
    pub cpu: String,
    pub memory: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceInfo {
    pub name: String,
    pub namespace: String,
    pub service_type: String,
    pub cluster_ip: String,
    pub external_ip: String,
    pub ports: String,
    pub age: String,
    pub selector: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct DeploymentInfo {
    pub name: String,
    pub namespace: String,
    pub replicas: u32,
    pub ready_replicas: u32,
    pub available_replicas: u32,
    pub age: String,
    pub images: String,
    pub selector: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct NamespaceInfo {
    pub name: String,
    pub status: String,
    pub age: String,
    pub labels: Vec<(String, String)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct EventInfo {
    pub namespace: String,
    pub name: String,
    pub event_type: String,
    pub reason: String,
    pub message: String,
    pub object_kind: String,
    pub object_name: String,
    pub object_node: Option<String>,
    pub source: String,
    pub count: u32,
    pub first_seen: String,
    pub last_seen: String,
}

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

    /// Get all pods running on a specific node with metrics
    pub async fn get_pods_on_node(kubeconfig: &Path, node_name: &str) -> Result<Vec<PodInfo>> {
        // First, get pods on the node
        let output = Command::new("kubectl")
            .arg("--kubeconfig")
            .arg(kubeconfig)
            .arg("get")
            .arg("pods")
            .arg("--all-namespaces")
            .arg("--field-selector")
            .arg(format!("spec.nodeName={}", node_name))
            .arg("-o")
            .arg("json")
            .output()
            .await
            .context("Failed to execute kubectl get pods")?;

        if !output.status.success() {
            anyhow::bail!(
                "kubectl get pods failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        let pods_json: serde_json::Value = serde_json::from_slice(&output.stdout)
            .context("Failed to parse kubectl pods output")?;

        let mut pods = Vec::new();

        if let Some(items) = pods_json["items"].as_array() {
            for pod in items {
                let name = pod["metadata"]["name"]
                    .as_str()
                    .unwrap_or("unknown")
                    .to_string();
                let namespace = pod["metadata"]["namespace"]
                    .as_str()
                    .unwrap_or("default")
                    .to_string();
                let node_name = pod["spec"]["nodeName"]
                    .as_str()
                    .unwrap_or("N/A")
                    .to_string();
                let status = pod["status"]["phase"]
                    .as_str()
                    .unwrap_or("Unknown")
                    .to_string();

                // Count restarts from all containers
                let mut restarts = 0u32;
                if let Some(container_statuses) = pod["status"]["containerStatuses"].as_array() {
                    for container in container_statuses {
                        if let Some(restart_count) = container["restartCount"].as_u64() {
                            restarts += restart_count as u32;
                        }
                    }
                }

                // Try to get metrics (may fail if metrics-server not installed)
                let (cpu, memory) = Self::get_pod_metrics(kubeconfig, &namespace, &name)
                    .await
                    .unwrap_or(("N/A".to_string(), "N/A".to_string()));

                pods.push(PodInfo {
                    name,
                    namespace,
                    node_name,
                    status,
                    restarts,
                    cpu,
                    memory,
                });
            }
        }

        Ok(pods)
    }

    /// Get pod metrics (CPU and memory)
    async fn get_pod_metrics(
        kubeconfig: &Path,
        namespace: &str,
        pod_name: &str,
    ) -> Result<(String, String)> {
        let output = Command::new("kubectl")
            .arg("--kubeconfig")
            .arg(kubeconfig)
            .arg("top")
            .arg("pod")
            .arg(pod_name)
            .arg("-n")
            .arg(namespace)
            .arg("--no-headers")
            .output()
            .await
            .context("Failed to execute kubectl top pod")?;

        if !output.status.success() {
            return Ok(("N/A".to_string(), "N/A".to_string()));
        }

        let output_str = String::from_utf8_lossy(&output.stdout);
        let parts: Vec<&str> = output_str.split_whitespace().collect();

        if parts.len() >= 3 {
            Ok((parts[1].to_string(), parts[2].to_string()))
        } else {
            Ok(("N/A".to_string(), "N/A".to_string()))
        }
    }

    /// Get all services from the cluster
    pub async fn get_services(kubeconfig: &Path) -> Result<Vec<ServiceInfo>> {
        let output = Command::new("kubectl")
            .arg("--kubeconfig")
            .arg(kubeconfig)
            .arg("get")
            .arg("services")
            .arg("--all-namespaces")
            .arg("-o")
            .arg("json")
            .output()
            .await
            .context("Failed to execute kubectl get services")?;

        if !output.status.success() {
            anyhow::bail!(
                "kubectl get services failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        let services_json: serde_json::Value = serde_json::from_slice(&output.stdout)
            .context("Failed to parse kubectl services output")?;

        let mut services = Vec::new();

        if let Some(items) = services_json["items"].as_array() {
            for svc in items {
                let name = svc["metadata"]["name"]
                    .as_str()
                    .unwrap_or("unknown")
                    .to_string();
                let namespace = svc["metadata"]["namespace"]
                    .as_str()
                    .unwrap_or("default")
                    .to_string();
                let service_type = svc["spec"]["type"]
                    .as_str()
                    .unwrap_or("ClusterIP")
                    .to_string();
                let cluster_ip = svc["spec"]["clusterIP"]
                    .as_str()
                    .unwrap_or("None")
                    .to_string();

                // Get external IPs
                let external_ip =
                    if let Some(ingress) = svc["status"]["loadBalancer"]["ingress"].as_array() {
                        if !ingress.is_empty() {
                            ingress[0]["ip"]
                                .as_str()
                                .or_else(|| ingress[0]["hostname"].as_str())
                                .unwrap_or("<pending>")
                                .to_string()
                        } else {
                            "<none>".to_string()
                        }
                    } else if let Some(external_ips) = svc["spec"]["externalIPs"].as_array() {
                        external_ips
                            .iter()
                            .filter_map(|ip| ip.as_str())
                            .collect::<Vec<_>>()
                            .join(",")
                    } else {
                        "<none>".to_string()
                    };

                // Get ports
                let ports = if let Some(port_list) = svc["spec"]["ports"].as_array() {
                    port_list
                        .iter()
                        .filter_map(|port| {
                            let port_num = port["port"].as_u64()?;
                            let protocol = port["protocol"].as_str().unwrap_or("TCP");
                            let node_port = port["nodePort"].as_u64();
                            if let Some(np) = node_port {
                                Some(format!("{}:{}/{}", port_num, np, protocol))
                            } else {
                                Some(format!("{}/{}", port_num, protocol))
                            }
                        })
                        .collect::<Vec<_>>()
                        .join(",")
                } else {
                    "N/A".to_string()
                };

                // Calculate age
                let age = if let Some(creation_timestamp) =
                    svc["metadata"]["creationTimestamp"].as_str()
                {
                    Self::calculate_age(creation_timestamp)
                } else {
                    "Unknown".to_string()
                };

                // Get selector
                let selector = if let Some(sel) = svc["spec"]["selector"].as_object() {
                    sel.iter()
                        .map(|(k, v)| format!("{}={}", k, v.as_str().unwrap_or("")))
                        .collect::<Vec<_>>()
                        .join(",")
                } else {
                    "<none>".to_string()
                };

                services.push(ServiceInfo {
                    name,
                    namespace,
                    service_type,
                    cluster_ip,
                    external_ip,
                    ports,
                    age,
                    selector,
                });
            }
        }

        Ok(services)
    }

    /// Get all deployments from the cluster
    #[allow(dead_code)]
    pub async fn get_deployments(kubeconfig: &Path) -> Result<Vec<DeploymentInfo>> {
        let output = Command::new("kubectl")
            .arg("--kubeconfig")
            .arg(kubeconfig)
            .arg("get")
            .arg("deployments")
            .arg("--all-namespaces")
            .arg("-o")
            .arg("json")
            .output()
            .await
            .context("Failed to execute kubectl get deployments")?;

        if !output.status.success() {
            anyhow::bail!(
                "kubectl get deployments failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        let deployments_json: serde_json::Value = serde_json::from_slice(&output.stdout)
            .context("Failed to parse kubectl deployments output")?;

        let mut deployments = Vec::new();

        if let Some(items) = deployments_json["items"].as_array() {
            for deploy in items {
                let name = deploy["metadata"]["name"]
                    .as_str()
                    .unwrap_or("unknown")
                    .to_string();
                let namespace = deploy["metadata"]["namespace"]
                    .as_str()
                    .unwrap_or("default")
                    .to_string();

                let replicas = deploy["spec"]["replicas"].as_u64().unwrap_or(0) as u32;
                let ready_replicas = deploy["status"]["readyReplicas"].as_u64().unwrap_or(0) as u32;
                let available_replicas =
                    deploy["status"]["availableReplicas"].as_u64().unwrap_or(0) as u32;

                // Get images from containers
                let images = if let Some(containers) =
                    deploy["spec"]["template"]["spec"]["containers"].as_array()
                {
                    containers
                        .iter()
                        .filter_map(|c| c["image"].as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                } else {
                    "N/A".to_string()
                };

                // Get selector
                let selector =
                    if let Some(sel) = deploy["spec"]["selector"]["matchLabels"].as_object() {
                        sel.iter()
                            .map(|(k, v)| format!("{}={}", k, v.as_str().unwrap_or("")))
                            .collect::<Vec<_>>()
                            .join(",")
                    } else {
                        "<none>".to_string()
                    };

                // Calculate age
                let age = if let Some(creation_timestamp) =
                    deploy["metadata"]["creationTimestamp"].as_str()
                {
                    Self::calculate_age(creation_timestamp)
                } else {
                    "Unknown".to_string()
                };

                deployments.push(DeploymentInfo {
                    name,
                    namespace,
                    replicas,
                    ready_replicas,
                    available_replicas,
                    age,
                    images,
                    selector,
                });
            }
        }

        Ok(deployments)
    }

    /// Get all namespaces from the cluster
    #[allow(dead_code)]
    pub async fn get_namespaces(kubeconfig: &Path) -> Result<Vec<NamespaceInfo>> {
        let output = Command::new("kubectl")
            .arg("--kubeconfig")
            .arg(kubeconfig)
            .arg("get")
            .arg("namespaces")
            .arg("-o")
            .arg("json")
            .output()
            .await
            .context("Failed to execute kubectl get namespaces")?;

        if !output.status.success() {
            anyhow::bail!(
                "kubectl get namespaces failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        let namespaces_json: serde_json::Value = serde_json::from_slice(&output.stdout)
            .context("Failed to parse kubectl namespaces output")?;

        let mut namespaces = Vec::new();

        if let Some(items) = namespaces_json["items"].as_array() {
            for ns in items {
                let name = ns["metadata"]["name"]
                    .as_str()
                    .unwrap_or("unknown")
                    .to_string();

                let status = ns["status"]["phase"]
                    .as_str()
                    .unwrap_or("Unknown")
                    .to_string();

                // Get labels
                let labels = if let Some(labels_obj) = ns["metadata"]["labels"].as_object() {
                    labels_obj
                        .iter()
                        .map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_string()))
                        .collect()
                } else {
                    Vec::new()
                };

                // Calculate age
                let age = if let Some(creation_timestamp) =
                    ns["metadata"]["creationTimestamp"].as_str()
                {
                    Self::calculate_age(creation_timestamp)
                } else {
                    "Unknown".to_string()
                };

                namespaces.push(NamespaceInfo {
                    name,
                    status,
                    age,
                    labels,
                });
            }
        }

        Ok(namespaces)
    }

    /// Get a single service with detailed information including endpoints
    pub async fn get_service_detail(
        kubeconfig: &Path,
        namespace: &str,
        name: &str,
    ) -> Result<Option<serde_json::Value>> {
        let output = Command::new("kubectl")
            .arg("--kubeconfig")
            .arg(kubeconfig)
            .arg("get")
            .arg("service")
            .arg(name)
            .arg("-n")
            .arg(namespace)
            .arg("-o")
            .arg("json")
            .output()
            .await
            .context("Failed to execute kubectl get service")?;

        if !output.status.success() {
            return Ok(None);
        }

        let service_json: serde_json::Value = serde_json::from_slice(&output.stdout)
            .context("Failed to parse kubectl service output")?;

        Ok(Some(service_json))
    }

    /// Get endpoints for a service
    pub async fn get_service_endpoints(
        kubeconfig: &Path,
        namespace: &str,
        name: &str,
    ) -> Result<Vec<String>> {
        let output = Command::new("kubectl")
            .arg("--kubeconfig")
            .arg(kubeconfig)
            .arg("get")
            .arg("endpoints")
            .arg(name)
            .arg("-n")
            .arg(namespace)
            .arg("-o")
            .arg("json")
            .output()
            .await
            .context("Failed to execute kubectl get endpoints")?;

        if !output.status.success() {
            return Ok(Vec::new());
        }

        let endpoints_json: serde_json::Value = serde_json::from_slice(&output.stdout)
            .context("Failed to parse kubectl endpoints output")?;

        let mut endpoints = Vec::new();

        if let Some(subsets) = endpoints_json["subsets"].as_array() {
            for subset in subsets {
                if let Some(addresses) = subset["addresses"].as_array() {
                    for addr in addresses {
                        if let Some(ip) = addr["ip"].as_str() {
                            if let Some(ports) = subset["ports"].as_array() {
                                for port in ports {
                                    if let Some(port_num) = port["port"].as_u64() {
                                        let protocol = port["protocol"].as_str().unwrap_or("TCP");
                                        endpoints.push(format!("{}:{}/{}", ip, port_num, protocol));
                                    }
                                }
                            } else {
                                endpoints.push(ip.to_string());
                            }
                        }
                    }
                }
            }
        }

        Ok(endpoints)
    }

    /// Get all events from the cluster
    #[allow(dead_code)]
    pub async fn get_events(kubeconfig: &Path) -> Result<Vec<EventInfo>> {
        let output = Command::new("kubectl")
            .arg("--kubeconfig")
            .arg(kubeconfig)
            .arg("get")
            .arg("events")
            .arg("--all-namespaces")
            .arg("--sort-by=.lastTimestamp")
            .arg("-o")
            .arg("json")
            .output()
            .await
            .context("Failed to execute kubectl get events")?;

        if !output.status.success() {
            anyhow::bail!(
                "kubectl get events failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        let events_json: serde_json::Value = serde_json::from_slice(&output.stdout)
            .context("Failed to parse kubectl events output")?;

        let mut events = Vec::new();

        if let Some(items) = events_json["items"].as_array() {
            for event in items {
                let namespace = event["metadata"]["namespace"]
                    .as_str()
                    .unwrap_or("default")
                    .to_string();
                let name = event["metadata"]["name"]
                    .as_str()
                    .unwrap_or("unknown")
                    .to_string();

                let event_type = event["type"].as_str().unwrap_or("Normal").to_string();
                let reason = event["reason"].as_str().unwrap_or("").to_string();
                let message = event["message"].as_str().unwrap_or("").to_string();

                let object_kind = event["involvedObject"]["kind"]
                    .as_str()
                    .unwrap_or("Unknown")
                    .to_string();
                let object_name = event["involvedObject"]["name"]
                    .as_str()
                    .unwrap_or("unknown")
                    .to_string();

                let source = event["source"]["component"]
                    .as_str()
                    .unwrap_or("unknown")
                    .to_string();
                let count = event["count"].as_u64().unwrap_or(1) as u32;

                let first_seen = if let Some(ts) = event["firstTimestamp"].as_str() {
                    Self::calculate_age(ts)
                } else {
                    "Unknown".to_string()
                };

                let last_seen = if let Some(ts) = event["lastTimestamp"].as_str() {
                    Self::calculate_age(ts)
                } else {
                    "Unknown".to_string()
                };

                // For Pod events, try to get the node name
                let object_node = if object_kind == "Pod" {
                    Self::get_pod_node(kubeconfig, &namespace, &object_name)
                        .await
                        .ok()
                } else {
                    None
                };

                events.push(EventInfo {
                    namespace,
                    name,
                    event_type,
                    reason,
                    message,
                    object_kind,
                    object_name,
                    object_node,
                    source,
                    count,
                    first_seen,
                    last_seen,
                });
            }
        }

        Ok(events)
    }

    /// Get the node name for a specific pod
    async fn get_pod_node(kubeconfig: &Path, namespace: &str, pod_name: &str) -> Result<String> {
        let output = Command::new("kubectl")
            .arg("--kubeconfig")
            .arg(kubeconfig)
            .arg("get")
            .arg("pod")
            .arg(pod_name)
            .arg("-n")
            .arg(namespace)
            .arg("-o")
            .arg("jsonpath={.spec.nodeName}")
            .output()
            .await
            .context("Failed to execute kubectl get pod")?;

        if !output.status.success() {
            anyhow::bail!(
                "kubectl get pod failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        let node_name = String::from_utf8(output.stdout)
            .context("Failed to parse pod node name")?
            .trim()
            .to_string();

        if node_name.is_empty() {
            anyhow::bail!("Pod has no node assigned");
        }

        Ok(node_name)
    }

    /// Calculate age from creation timestamp
    pub fn calculate_age(timestamp: &str) -> String {
        use chrono::{DateTime, Utc};

        if let Ok(created) = timestamp.parse::<DateTime<Utc>>() {
            let now = Utc::now();
            let duration = now.signed_duration_since(created);

            let days = duration.num_days();
            if days > 0 {
                return format!("{}d", days);
            }

            let hours = duration.num_hours();
            if hours > 0 {
                return format!("{}h", hours);
            }

            let minutes = duration.num_minutes();
            if minutes > 0 {
                return format!("{}m", minutes);
            }

            "< 1m".to_string()
        } else {
            "Unknown".to_string()
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
