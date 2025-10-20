/// Cluster insights for best practices and recommendations
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::utils::command::CommandBuilder;

/// Insight information for cluster best practices
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Insight {
    pub title: String,
    pub insight_type: String, // warning, info, optimization
    pub severity: String,     // high, medium, low
    pub description: String,
    pub recommendation: String,
    pub affected_resources: Vec<String>,
    pub category: String, // resources, security, performance, cost
}

/// Collect cluster insights for best practices
pub async fn collect_insights(kubeconfig_path: &std::path::Path) -> Result<Vec<Insight>> {
    let mut insights = Vec::new();

    // Resource Management Insights
    let pods_without_limits = check_pods_without_limits(kubeconfig_path).await?;
    if !pods_without_limits.is_empty() {
        insights.push(Insight {
            title: "Pods Without Resource Limits".to_string(),
            insight_type: "warning".to_string(),
            severity: "medium".to_string(),
            description: format!(
                "{} pod(s) are running without CPU or memory limits set. This can lead to resource contention and instability.",
                pods_without_limits.len()
            ),
            recommendation: "Set resource requests and limits for all pods to ensure predictable resource allocation and prevent resource exhaustion. Use 'kubectl set resources' or update pod manifests with appropriate limits.".to_string(),
            affected_resources: pods_without_limits,
            category: "resources".to_string(),
        });
    }

    let pods_without_requests = check_pods_without_requests(kubeconfig_path).await?;
    if !pods_without_requests.is_empty() {
        insights.push(Insight {
            title: "Pods Without Resource Requests".to_string(),
            insight_type: "warning".to_string(),
            severity: "medium".to_string(),
            description: format!(
                "{} pod(s) are running without CPU or memory requests set. This affects pod scheduling and can lead to poor resource allocation.",
                pods_without_requests.len()
            ),
            recommendation: "Set resource requests for all pods to help the Kubernetes scheduler make informed placement decisions and ensure QoS guarantees.".to_string(),
            affected_resources: pods_without_requests,
            category: "resources".to_string(),
        });
    }

    let high_restart_pods = check_high_restart_pods(kubeconfig_path).await?;
    if !high_restart_pods.is_empty() {
        insights.push(Insight {
            title: "Pods with High Restart Count".to_string(),
            insight_type: "warning".to_string(),
            severity: "high".to_string(),
            description: format!(
                "{} pod(s) have restarted more than 5 times. This indicates application instability or configuration issues.",
                high_restart_pods.len()
            ),
            recommendation: "Investigate pod logs and events to identify the root cause of restarts. Common issues include OOMKills, failed health checks, or application crashes.".to_string(),
            affected_resources: high_restart_pods,
            category: "resources".to_string(),
        });
    }

    let over_provisioned_pods = check_over_provisioned_pods(kubeconfig_path).await?;
    if !over_provisioned_pods.is_empty() {
        insights.push(Insight {
            title: "Over-Provisioned Pods".to_string(),
            insight_type: "optimization".to_string(),
            severity: "low".to_string(),
            description: format!(
                "{} pod(s) are using less than 20% of their requested resources. This leads to resource waste and inefficient cluster utilization.",
                over_provisioned_pods.len()
            ),
            recommendation: "Review and reduce resource requests to match actual usage patterns. This will improve cluster efficiency and allow better pod scheduling.".to_string(),
            affected_resources: over_provisioned_pods,
            category: "resources".to_string(),
        });
    }

    let under_provisioned_pods = check_under_provisioned_pods(kubeconfig_path).await?;
    if !under_provisioned_pods.is_empty() {
        insights.push(Insight {
            title: "Under-Provisioned Pods".to_string(),
            insight_type: "warning".to_string(),
            severity: "high".to_string(),
            description: format!(
                "{} pod(s) are using more than 90% of their resource limits. This can lead to throttling (CPU) or OOMKills (memory).",
                under_provisioned_pods.len()
            ),
            recommendation: "Increase resource limits for these pods to prevent performance degradation and crashes. Monitor actual usage and set limits with appropriate headroom.".to_string(),
            affected_resources: under_provisioned_pods,
            category: "resources".to_string(),
        });
    }

    // Security Insights
    let pods_running_as_root = check_pods_running_as_root(kubeconfig_path).await?;
    if !pods_running_as_root.is_empty() {
        insights.push(Insight {
            title: "Pods Running as Root".to_string(),
            insight_type: "warning".to_string(),
            severity: "high".to_string(),
            description: format!(
                "{} pod(s) are running containers as root user. This poses a security risk if the container is compromised.",
                pods_running_as_root.len()
            ),
            recommendation: "Configure securityContext to run containers as non-root users. Set runAsNonRoot: true and specify a runAsUser ID in your pod specifications.".to_string(),
            affected_resources: pods_running_as_root,
            category: "security".to_string(),
        });
    }

    let pods_without_security_context =
        check_pods_without_security_context(kubeconfig_path).await?;
    if !pods_without_security_context.is_empty() {
        insights.push(Insight {
            title: "Pods Without Security Context".to_string(),
            insight_type: "warning".to_string(),
            severity: "medium".to_string(),
            description: format!(
                "{} pod(s) are missing security context configuration. This leaves them vulnerable to security exploits.",
                pods_without_security_context.len()
            ),
            recommendation: "Add securityContext to all pods with settings like runAsNonRoot, allowPrivilegeEscalation: false, and readOnlyRootFilesystem where possible.".to_string(),
            affected_resources: pods_without_security_context,
            category: "security".to_string(),
        });
    }

    let privileged_containers = check_privileged_containers(kubeconfig_path).await?;
    if !privileged_containers.is_empty() {
        insights.push(Insight {
            title: "Privileged Containers Running".to_string(),
            insight_type: "warning".to_string(),
            severity: "high".to_string(),
            description: format!(
                "{} container(s) are running in privileged mode. This grants full access to the host and should be avoided unless absolutely necessary.",
                privileged_containers.len()
            ),
            recommendation: "Remove privileged: true from container security contexts unless required for system-level operations. Consider using specific capabilities instead.".to_string(),
            affected_resources: privileged_containers,
            category: "security".to_string(),
        });
    }

    let namespaces_without_network_policies =
        check_namespaces_without_network_policies(kubeconfig_path).await?;
    if !namespaces_without_network_policies.is_empty() {
        insights.push(Insight {
            title: "Namespaces Without Network Policies".to_string(),
            insight_type: "info".to_string(),
            severity: "medium".to_string(),
            description: format!(
                "{} namespace(s) don't have any network policies defined. This allows unrestricted pod-to-pod communication.",
                namespaces_without_network_policies.len()
            ),
            recommendation: "Implement network policies to control traffic between pods and enhance security through network segmentation.".to_string(),
            affected_resources: namespaces_without_network_policies,
            category: "security".to_string(),
        });
    }

    // Reliability & High Availability Insights
    let single_replica_deployments = check_single_replica_deployments(kubeconfig_path).await?;
    if !single_replica_deployments.is_empty() {
        insights.push(Insight {
            title: "Single Replica Deployments".to_string(),
            insight_type: "info".to_string(),
            severity: "medium".to_string(),
            description: format!(
                "{} deployment(s) have only 1 replica. This creates a single point of failure and provides no high availability.",
                single_replica_deployments.len()
            ),
            recommendation: "Increase replicas to at least 2-3 for production workloads to ensure high availability and rolling update capabilities.".to_string(),
            affected_resources: single_replica_deployments,
            category: "performance".to_string(),
        });
    }

    let pods_without_probes = check_pods_without_probes(kubeconfig_path).await?;
    if !pods_without_probes.is_empty() {
        insights.push(Insight {
            title: "Pods Without Health Probes".to_string(),
            insight_type: "warning".to_string(),
            severity: "high".to_string(),
            description: format!(
                "{} pod(s) are missing liveness or readiness probes. This prevents Kubernetes from detecting and recovering from failures.",
                pods_without_probes.len()
            ),
            recommendation: "Add livenessProbe and readinessProbe to all containers to enable automatic health checking and recovery.".to_string(),
            affected_resources: pods_without_probes,
            category: "performance".to_string(),
        });
    }

    let deployments_without_pdb = check_deployments_without_pdb(kubeconfig_path).await?;
    if !deployments_without_pdb.is_empty() {
        insights.push(Insight {
            title: "Deployments Without Pod Disruption Budgets".to_string(),
            insight_type: "info".to_string(),
            severity: "low".to_string(),
            description: format!(
                "{} deployment(s) don't have Pod Disruption Budgets (PDB). This can lead to unexpected downtime during node maintenance.",
                deployments_without_pdb.len()
            ),
            recommendation: "Create PodDisruptionBudget resources to ensure minimum availability during voluntary disruptions like node drains.".to_string(),
            affected_resources: deployments_without_pdb,
            category: "performance".to_string(),
        });
    }

    // Cost Optimization Insights
    let unused_pvcs = check_unused_pvcs(kubeconfig_path).await?;
    if !unused_pvcs.is_empty() {
        insights.push(Insight {
            title: "Unused Persistent Volume Claims".to_string(),
            insight_type: "optimization".to_string(),
            severity: "low".to_string(),
            description: format!(
                "{} PVC(s) are not attached to any pods. These consume storage resources and incur costs unnecessarily.",
                unused_pvcs.len()
            ),
            recommendation: "Review and delete unused PVCs to reduce storage costs. Ensure data is backed up before deletion if needed.".to_string(),
            affected_resources: unused_pvcs,
            category: "cost".to_string(),
        });
    }

    let services_without_endpoints = check_services_without_endpoints(kubeconfig_path).await?;
    if !services_without_endpoints.is_empty() {
        insights.push(Insight {
            title: "Services Without Endpoints".to_string(),
            insight_type: "optimization".to_string(),
            severity: "low".to_string(),
            description: format!(
                "{} service(s) have no endpoints/pods backing them. These may be unused or misconfigured.",
                services_without_endpoints.len()
            ),
            recommendation: "Review services without endpoints and remove unused ones or fix selector labels to match pods.".to_string(),
            affected_resources: services_without_endpoints,
            category: "cost".to_string(),
        });
    }

    // Configuration Best Practices Insights
    let pods_using_latest_tag = check_pods_using_latest_tag(kubeconfig_path).await?;
    if !pods_using_latest_tag.is_empty() {
        insights.push(Insight {
            title: "Pods Using 'latest' Image Tag".to_string(),
            insight_type: "warning".to_string(),
            severity: "medium".to_string(),
            description: format!(
                "{} pod(s) are using 'latest' or unversioned image tags. This makes deployments unpredictable and harder to rollback.",
                pods_using_latest_tag.len()
            ),
            recommendation: "Always use specific version tags for container images (e.g., 'v1.2.3') to ensure reproducible deployments and easier rollbacks.".to_string(),
            affected_resources: pods_using_latest_tag,
            category: "resources".to_string(),
        });
    }

    let namespaces_without_quotas = check_namespaces_without_quotas(kubeconfig_path).await?;
    if !namespaces_without_quotas.is_empty() {
        insights.push(Insight {
            title: "Namespaces Without Resource Quotas".to_string(),
            insight_type: "info".to_string(),
            severity: "low".to_string(),
            description: format!(
                "{} namespace(s) don't have resource quotas defined. This allows unlimited resource consumption.",
                namespaces_without_quotas.len()
            ),
            recommendation: "Create ResourceQuota objects for namespaces to limit total resource consumption and prevent resource exhaustion.".to_string(),
            affected_resources: namespaces_without_quotas,
            category: "resources".to_string(),
        });
    }

    Ok(insights)
}

/// Helper function to skip system namespaces
fn should_skip_namespace(namespace: &str) -> bool {
    matches!(
        namespace,
        "kube-system" | "monitoring" | "kube-public" | "kube-node-lease"
    )
}

/// Check for pods without resource limits
async fn check_pods_without_limits(kubeconfig_path: &std::path::Path) -> Result<Vec<String>> {
    let output = CommandBuilder::new("kubectl")
        .args(["get", "pods", "--all-namespaces", "-o", "json"])
        .kubeconfig(kubeconfig_path)
        .context("Failed to get pods")
        .output()
        .await?;

    if !output.success {
        tracing::warn!(
            "Failed to get pods for insights check: {}",
            output.stderr.trim()
        );
        return Ok(Vec::new());
    }

    #[derive(Deserialize)]
    struct PodList {
        items: Vec<Pod>,
    }

    #[derive(Deserialize)]
    struct Pod {
        metadata: PodMetadata,
        spec: PodSpec,
    }

    #[derive(Deserialize)]
    struct PodMetadata {
        name: String,
        namespace: String,
    }

    #[derive(Deserialize)]
    struct PodSpec {
        containers: Vec<Container>,
    }

    #[derive(Deserialize)]
    struct Container {
        #[allow(dead_code)]
        name: String,
        #[serde(default)]
        resources: Resources,
    }

    #[derive(Deserialize, Default)]
    struct Resources {
        #[serde(default)]
        limits: std::collections::HashMap<String, String>,
        #[serde(default)]
        #[allow(dead_code)]
        requests: std::collections::HashMap<String, String>,
    }

    let pod_list: PodList =
        serde_json::from_str(&output.stdout).context("Failed to parse pod list")?;

    let mut pods_without_limits = Vec::new();

    for pod in pod_list.items {
        // Skip system namespaces
        if should_skip_namespace(&pod.metadata.namespace) {
            continue;
        }

        let mut missing_limits = false;
        for container in &pod.spec.containers {
            // Check if CPU or memory limits are missing
            if !container.resources.limits.contains_key("cpu")
                || !container.resources.limits.contains_key("memory")
            {
                missing_limits = true;
                break;
            }
        }

        if missing_limits {
            pods_without_limits.push(format!("{}/{}", pod.metadata.namespace, pod.metadata.name));
        }
    }

    Ok(pods_without_limits)
}

/// Check for pods without resource requests
async fn check_pods_without_requests(kubeconfig_path: &std::path::Path) -> Result<Vec<String>> {
    let output = CommandBuilder::new("kubectl")
        .args(["get", "pods", "--all-namespaces", "-o", "json"])
        .kubeconfig(kubeconfig_path)
        .context("Failed to get pods")
        .output()
        .await?;

    if !output.success {
        return Ok(Vec::new());
    }

    #[derive(Deserialize)]
    struct PodList {
        items: Vec<Pod>,
    }

    #[derive(Deserialize)]
    struct Pod {
        metadata: PodMetadata,
        spec: PodSpec,
    }

    #[derive(Deserialize)]
    struct PodMetadata {
        name: String,
        namespace: String,
    }

    #[derive(Deserialize)]
    struct PodSpec {
        containers: Vec<Container>,
    }

    #[derive(Deserialize)]
    struct Container {
        #[serde(default)]
        resources: Resources,
    }

    #[derive(Deserialize, Default)]
    struct Resources {
        #[serde(default)]
        requests: std::collections::HashMap<String, String>,
    }

    let pod_list: PodList = serde_json::from_str(&output.stdout)?;
    let mut pods_without_requests = Vec::new();

    for pod in pod_list.items {
        if should_skip_namespace(&pod.metadata.namespace) {
            continue;
        }

        let mut missing_requests = false;
        for container in &pod.spec.containers {
            if !container.resources.requests.contains_key("cpu")
                || !container.resources.requests.contains_key("memory")
            {
                missing_requests = true;
                break;
            }
        }

        if missing_requests {
            pods_without_requests.push(format!("{}/{}", pod.metadata.namespace, pod.metadata.name));
        }
    }

    Ok(pods_without_requests)
}

/// Check for pods with high restart count
async fn check_high_restart_pods(kubeconfig_path: &std::path::Path) -> Result<Vec<String>> {
    let output = CommandBuilder::new("kubectl")
        .args(["get", "pods", "--all-namespaces", "-o", "json"])
        .kubeconfig(kubeconfig_path)
        .context("Failed to get pods")
        .output()
        .await?;

    if !output.success {
        return Ok(Vec::new());
    }

    #[derive(Deserialize)]
    struct PodList {
        items: Vec<Pod>,
    }

    #[derive(Deserialize)]
    struct Pod {
        metadata: PodMetadata,
        status: PodStatus,
    }

    #[derive(Deserialize)]
    struct PodMetadata {
        name: String,
        namespace: String,
    }

    #[derive(Deserialize)]
    struct PodStatus {
        #[serde(default)]
        #[serde(rename = "containerStatuses")]
        container_statuses: Vec<ContainerStatus>,
    }

    #[derive(Deserialize)]
    struct ContainerStatus {
        #[serde(rename = "restartCount")]
        restart_count: i32,
    }

    let pod_list: PodList = serde_json::from_str(&output.stdout)?;
    let mut high_restart_pods = Vec::new();

    for pod in pod_list.items {
        if should_skip_namespace(&pod.metadata.namespace) {
            continue;
        }

        for container_status in &pod.status.container_statuses {
            if container_status.restart_count > 5 {
                high_restart_pods.push(format!(
                    "{}/{} (restarts: {})",
                    pod.metadata.namespace, pod.metadata.name, container_status.restart_count
                ));
                break;
            }
        }
    }

    Ok(high_restart_pods)
}

/// Check for pods running as root
async fn check_pods_running_as_root(kubeconfig_path: &std::path::Path) -> Result<Vec<String>> {
    let output = CommandBuilder::new("kubectl")
        .args(["get", "pods", "--all-namespaces", "-o", "json"])
        .kubeconfig(kubeconfig_path)
        .context("Failed to get pods")
        .output()
        .await?;

    if !output.success {
        return Ok(Vec::new());
    }

    #[derive(Deserialize)]
    struct PodList {
        items: Vec<Pod>,
    }

    #[derive(Deserialize)]
    struct Pod {
        metadata: PodMetadata,
        spec: PodSpec,
    }

    #[derive(Deserialize)]
    struct PodMetadata {
        name: String,
        namespace: String,
    }

    #[derive(Deserialize)]
    struct PodSpec {
        containers: Vec<Container>,
        #[serde(default)]
        #[serde(rename = "securityContext")]
        security_context: Option<SecurityContext>,
    }

    #[derive(Deserialize)]
    struct Container {
        #[serde(default)]
        #[serde(rename = "securityContext")]
        security_context: Option<ContainerSecurityContext>,
    }

    #[derive(Deserialize)]
    struct SecurityContext {
        #[serde(rename = "runAsUser")]
        run_as_user: Option<i64>,
        #[serde(rename = "runAsNonRoot")]
        run_as_non_root: Option<bool>,
    }

    #[derive(Deserialize)]
    struct ContainerSecurityContext {
        #[serde(rename = "runAsUser")]
        run_as_user: Option<i64>,
        #[serde(rename = "runAsNonRoot")]
        run_as_non_root: Option<bool>,
    }

    let pod_list: PodList = serde_json::from_str(&output.stdout)?;
    let mut pods_running_as_root = Vec::new();

    for pod in pod_list.items {
        if should_skip_namespace(&pod.metadata.namespace) {
            continue;
        }

        let pod_run_as_non_root = pod
            .spec
            .security_context
            .as_ref()
            .and_then(|sc| sc.run_as_non_root);

        for container in &pod.spec.containers {
            let container_run_as_non_root = container
                .security_context
                .as_ref()
                .and_then(|sc| sc.run_as_non_root);

            // Check if explicitly running as non-root
            if container_run_as_non_root == Some(true) || pod_run_as_non_root == Some(true) {
                continue;
            }

            // Check if run_as_user is set to non-zero
            let run_as_user = container
                .security_context
                .as_ref()
                .and_then(|sc| sc.run_as_user)
                .or_else(|| {
                    pod.spec
                        .security_context
                        .as_ref()
                        .and_then(|sc| sc.run_as_user)
                });

            if run_as_user.is_none() || run_as_user == Some(0) {
                pods_running_as_root
                    .push(format!("{}/{}", pod.metadata.namespace, pod.metadata.name));
                break;
            }
        }
    }

    Ok(pods_running_as_root)
}

/// Check for pods without security context
async fn check_pods_without_security_context(
    kubeconfig_path: &std::path::Path,
) -> Result<Vec<String>> {
    let output = CommandBuilder::new("kubectl")
        .args(["get", "pods", "--all-namespaces", "-o", "json"])
        .kubeconfig(kubeconfig_path)
        .context("Failed to get pods")
        .output()
        .await?;

    if !output.success {
        return Ok(Vec::new());
    }

    #[derive(Deserialize)]
    struct PodList {
        items: Vec<Pod>,
    }

    #[derive(Deserialize)]
    struct Pod {
        metadata: PodMetadata,
        spec: PodSpec,
    }

    #[derive(Deserialize)]
    struct PodMetadata {
        name: String,
        namespace: String,
    }

    #[derive(Deserialize)]
    struct PodSpec {
        containers: Vec<Container>,
        #[serde(default)]
        #[serde(rename = "securityContext")]
        security_context: Option<serde_json::Value>,
    }

    #[derive(Deserialize)]
    struct Container {
        #[serde(default)]
        #[serde(rename = "securityContext")]
        security_context: Option<serde_json::Value>,
    }

    let pod_list: PodList = serde_json::from_str(&output.stdout)?;
    let mut pods_without_security_context = Vec::new();

    for pod in pod_list.items {
        if should_skip_namespace(&pod.metadata.namespace) {
            continue;
        }

        let has_pod_security_context = pod.spec.security_context.is_some();
        let has_container_security_context = pod
            .spec
            .containers
            .iter()
            .any(|c| c.security_context.is_some());

        if !has_pod_security_context && !has_container_security_context {
            pods_without_security_context
                .push(format!("{}/{}", pod.metadata.namespace, pod.metadata.name));
        }
    }

    Ok(pods_without_security_context)
}

/// Check for privileged containers
async fn check_privileged_containers(kubeconfig_path: &std::path::Path) -> Result<Vec<String>> {
    let output = CommandBuilder::new("kubectl")
        .args(["get", "pods", "--all-namespaces", "-o", "json"])
        .kubeconfig(kubeconfig_path)
        .context("Failed to get pods")
        .output()
        .await?;

    if !output.success {
        return Ok(Vec::new());
    }

    #[derive(Deserialize)]
    struct PodList {
        items: Vec<Pod>,
    }

    #[derive(Deserialize)]
    struct Pod {
        metadata: PodMetadata,
        spec: PodSpec,
    }

    #[derive(Deserialize)]
    struct PodMetadata {
        name: String,
        namespace: String,
    }

    #[derive(Deserialize)]
    struct PodSpec {
        containers: Vec<Container>,
    }

    #[derive(Deserialize)]
    struct Container {
        name: String,
        #[serde(default)]
        #[serde(rename = "securityContext")]
        security_context: Option<SecurityContext>,
    }

    #[derive(Deserialize)]
    struct SecurityContext {
        #[serde(default)]
        privileged: bool,
    }

    let pod_list: PodList = serde_json::from_str(&output.stdout)?;
    let mut privileged_containers = Vec::new();

    for pod in pod_list.items {
        if should_skip_namespace(&pod.metadata.namespace) {
            continue;
        }

        for container in &pod.spec.containers {
            if let Some(ref security_context) = container.security_context {
                if security_context.privileged {
                    privileged_containers.push(format!(
                        "{}/{}/{}",
                        pod.metadata.namespace, pod.metadata.name, container.name
                    ));
                }
            }
        }
    }

    Ok(privileged_containers)
}

/// Check for namespaces without network policies
async fn check_namespaces_without_network_policies(
    kubeconfig_path: &std::path::Path,
) -> Result<Vec<String>> {
    // Get all namespaces
    let ns_output = CommandBuilder::new("kubectl")
        .args(["get", "namespaces", "-o", "json"])
        .kubeconfig(kubeconfig_path)
        .context("Failed to get namespaces")
        .output()
        .await?;

    if !ns_output.success {
        return Ok(Vec::new());
    }

    // Get all network policies
    let np_output = CommandBuilder::new("kubectl")
        .args(["get", "networkpolicies", "--all-namespaces", "-o", "json"])
        .kubeconfig(kubeconfig_path)
        .context("Failed to get network policies")
        .output()
        .await?;

    #[derive(Deserialize)]
    struct NamespaceList {
        items: Vec<Namespace>,
    }

    #[derive(Deserialize)]
    struct Namespace {
        metadata: NamespaceMetadata,
    }

    #[derive(Deserialize)]
    struct NamespaceMetadata {
        name: String,
    }

    #[derive(Deserialize)]
    struct NetworkPolicyList {
        items: Vec<NetworkPolicy>,
    }

    #[derive(Deserialize)]
    struct NetworkPolicy {
        metadata: NetworkPolicyMetadata,
    }

    #[derive(Deserialize)]
    struct NetworkPolicyMetadata {
        namespace: String,
    }

    let namespace_list: NamespaceList = serde_json::from_str(&ns_output.stdout)?;
    let mut namespaces_with_policies = std::collections::HashSet::new();

    if np_output.success {
        let network_policy_list: NetworkPolicyList = serde_json::from_str(&np_output.stdout)?;
        for np in network_policy_list.items {
            namespaces_with_policies.insert(np.metadata.namespace);
        }
    }

    let mut namespaces_without_policies = Vec::new();
    for namespace in namespace_list.items {
        if should_skip_namespace(&namespace.metadata.name) {
            continue;
        }

        if !namespaces_with_policies.contains(&namespace.metadata.name) {
            namespaces_without_policies.push(namespace.metadata.name);
        }
    }

    Ok(namespaces_without_policies)
}

/// Check for single replica deployments
async fn check_single_replica_deployments(
    kubeconfig_path: &std::path::Path,
) -> Result<Vec<String>> {
    let output = CommandBuilder::new("kubectl")
        .args(["get", "deployments", "--all-namespaces", "-o", "json"])
        .kubeconfig(kubeconfig_path)
        .context("Failed to get deployments")
        .output()
        .await?;

    if !output.success {
        return Ok(Vec::new());
    }

    #[derive(Deserialize)]
    struct DeploymentList {
        items: Vec<Deployment>,
    }

    #[derive(Deserialize)]
    struct Deployment {
        metadata: DeploymentMetadata,
        spec: DeploymentSpec,
    }

    #[derive(Deserialize)]
    struct DeploymentMetadata {
        name: String,
        namespace: String,
    }

    #[derive(Deserialize)]
    struct DeploymentSpec {
        #[serde(default)]
        replicas: i32,
    }

    let deployment_list: DeploymentList = serde_json::from_str(&output.stdout)?;
    let mut single_replica_deployments = Vec::new();

    for deployment in deployment_list.items {
        if should_skip_namespace(&deployment.metadata.namespace) {
            continue;
        }

        if deployment.spec.replicas == 1 {
            single_replica_deployments.push(format!(
                "{}/{}",
                deployment.metadata.namespace, deployment.metadata.name
            ));
        }
    }

    Ok(single_replica_deployments)
}

/// Check for pods without health probes
async fn check_pods_without_probes(kubeconfig_path: &std::path::Path) -> Result<Vec<String>> {
    let output = CommandBuilder::new("kubectl")
        .args(["get", "pods", "--all-namespaces", "-o", "json"])
        .kubeconfig(kubeconfig_path)
        .context("Failed to get pods")
        .output()
        .await?;

    if !output.success {
        return Ok(Vec::new());
    }

    #[derive(Deserialize)]
    struct PodList {
        items: Vec<Pod>,
    }

    #[derive(Deserialize)]
    struct Pod {
        metadata: PodMetadata,
        spec: PodSpec,
    }

    #[derive(Deserialize)]
    struct PodMetadata {
        name: String,
        namespace: String,
    }

    #[derive(Deserialize)]
    struct PodSpec {
        containers: Vec<Container>,
    }

    #[derive(Deserialize)]
    struct Container {
        #[serde(rename = "livenessProbe")]
        liveness_probe: Option<serde_json::Value>,
        #[serde(rename = "readinessProbe")]
        readiness_probe: Option<serde_json::Value>,
    }

    let pod_list: PodList = serde_json::from_str(&output.stdout)?;
    let mut pods_without_probes = Vec::new();

    for pod in pod_list.items {
        if should_skip_namespace(&pod.metadata.namespace) {
            continue;
        }

        let mut missing_probes = false;
        for container in &pod.spec.containers {
            if container.liveness_probe.is_none() || container.readiness_probe.is_none() {
                missing_probes = true;
                break;
            }
        }

        if missing_probes {
            pods_without_probes.push(format!("{}/{}", pod.metadata.namespace, pod.metadata.name));
        }
    }

    Ok(pods_without_probes)
}

/// Check for deployments without PodDisruptionBudgets
async fn check_deployments_without_pdb(kubeconfig_path: &std::path::Path) -> Result<Vec<String>> {
    // Get all deployments
    let deploy_output = CommandBuilder::new("kubectl")
        .args(["get", "deployments", "--all-namespaces", "-o", "json"])
        .kubeconfig(kubeconfig_path)
        .context("Failed to get deployments")
        .output()
        .await?;

    if !deploy_output.success {
        return Ok(Vec::new());
    }

    // Get all PDBs
    let pdb_output = CommandBuilder::new("kubectl")
        .args([
            "get",
            "poddisruptionbudgets",
            "--all-namespaces",
            "-o",
            "json",
        ])
        .kubeconfig(kubeconfig_path)
        .context("Failed to get PDBs")
        .output()
        .await?;

    #[derive(Deserialize)]
    struct DeploymentList {
        items: Vec<Deployment>,
    }

    #[derive(Deserialize)]
    struct Deployment {
        metadata: DeploymentMetadata,
    }

    #[derive(Deserialize)]
    struct DeploymentMetadata {
        name: String,
        namespace: String,
    }

    #[derive(Deserialize)]
    struct PdbList {
        items: Vec<Pdb>,
    }

    #[derive(Deserialize)]
    struct Pdb {
        metadata: PdbMetadata,
        spec: PdbSpec,
    }

    #[derive(Deserialize)]
    struct PdbMetadata {
        namespace: String,
    }

    #[derive(Deserialize)]
    struct PdbSpec {
        selector: Option<Selector>,
    }

    #[derive(Deserialize)]
    struct Selector {
        #[serde(rename = "matchLabels")]
        #[allow(dead_code)]
        match_labels: std::collections::HashMap<String, String>,
    }

    let deployment_list: DeploymentList = serde_json::from_str(&deploy_output.stdout)?;
    let mut deployments_with_pdb = std::collections::HashSet::new();

    if pdb_output.success {
        let pdb_list: PdbList = serde_json::from_str(&pdb_output.stdout)?;
        for pdb in pdb_list.items {
            if pdb.spec.selector.is_some() {
                // Simple heuristic: assume PDB covers deployments in same namespace
                deployments_with_pdb.insert(pdb.metadata.namespace);
            }
        }
    }

    let mut deployments_without_pdb = Vec::new();
    for deployment in deployment_list.items {
        if should_skip_namespace(&deployment.metadata.namespace) {
            continue;
        }

        if !deployments_with_pdb.contains(&deployment.metadata.namespace) {
            deployments_without_pdb.push(format!(
                "{}/{}",
                deployment.metadata.namespace, deployment.metadata.name
            ));
        }
    }

    Ok(deployments_without_pdb)
}

/// Check for unused PVCs
async fn check_unused_pvcs(kubeconfig_path: &std::path::Path) -> Result<Vec<String>> {
    // Get all PVCs
    let pvc_output = CommandBuilder::new("kubectl")
        .args([
            "get",
            "persistentvolumeclaims",
            "--all-namespaces",
            "-o",
            "json",
        ])
        .kubeconfig(kubeconfig_path)
        .context("Failed to get PVCs")
        .output()
        .await?;

    if !pvc_output.success {
        return Ok(Vec::new());
    }

    // Get all pods
    let pods_output = CommandBuilder::new("kubectl")
        .args(["get", "pods", "--all-namespaces", "-o", "json"])
        .kubeconfig(kubeconfig_path)
        .context("Failed to get pods")
        .output()
        .await?;

    #[derive(Deserialize)]
    struct PvcList {
        items: Vec<Pvc>,
    }

    #[derive(Deserialize)]
    struct Pvc {
        metadata: PvcMetadata,
    }

    #[derive(Deserialize)]
    struct PvcMetadata {
        name: String,
        namespace: String,
    }

    #[derive(Deserialize)]
    struct PodList {
        items: Vec<Pod>,
    }

    #[derive(Deserialize)]
    struct Pod {
        metadata: PodMetadata,
        spec: PodSpec,
    }

    #[derive(Deserialize)]
    struct PodMetadata {
        namespace: String,
    }

    #[derive(Deserialize)]
    struct PodSpec {
        #[serde(default)]
        volumes: Vec<Volume>,
    }

    #[derive(Deserialize)]
    struct Volume {
        #[serde(rename = "persistentVolumeClaim")]
        persistent_volume_claim: Option<PvcRef>,
    }

    #[derive(Deserialize)]
    struct PvcRef {
        #[serde(rename = "claimName")]
        claim_name: String,
    }

    let pvc_list: PvcList = serde_json::from_str(&pvc_output.stdout)?;
    let mut pvcs_in_use = std::collections::HashSet::new();

    if pods_output.success {
        let pod_list: PodList = serde_json::from_str(&pods_output.stdout)?;
        for pod in pod_list.items {
            for volume in &pod.spec.volumes {
                if let Some(ref pvc_ref) = volume.persistent_volume_claim {
                    pvcs_in_use
                        .insert(format!("{}/{}", pod.metadata.namespace, pvc_ref.claim_name));
                }
            }
        }
    }

    let mut unused_pvcs = Vec::new();
    for pvc in pvc_list.items {
        let pvc_key = format!("{}/{}", pvc.metadata.namespace, pvc.metadata.name);
        if !pvcs_in_use.contains(&pvc_key) {
            unused_pvcs.push(pvc_key);
        }
    }

    Ok(unused_pvcs)
}

/// Check for services without endpoints
async fn check_services_without_endpoints(
    kubeconfig_path: &std::path::Path,
) -> Result<Vec<String>> {
    let output = CommandBuilder::new("kubectl")
        .args(["get", "services", "--all-namespaces", "-o", "json"])
        .kubeconfig(kubeconfig_path)
        .context("Failed to get services")
        .output()
        .await?;

    if !output.success {
        return Ok(Vec::new());
    }

    #[derive(Deserialize)]
    struct ServiceList {
        items: Vec<Service>,
    }

    #[derive(Deserialize)]
    struct Service {
        metadata: ServiceMetadata,
        #[allow(dead_code)]
        spec: ServiceSpec,
    }

    #[derive(Deserialize)]
    struct ServiceMetadata {
        name: String,
        namespace: String,
    }

    #[derive(Deserialize)]
    struct ServiceSpec {
        #[serde(default)]
        #[allow(dead_code)]
        selector: std::collections::HashMap<String, String>,
    }

    let service_list: ServiceList = serde_json::from_str(&output.stdout)?;
    let mut services_without_endpoints = Vec::new();

    for service in service_list.items {
        if should_skip_namespace(&service.metadata.namespace) {
            continue;
        }

        // Check if service has endpoints
        let ep_output = CommandBuilder::new("kubectl")
            .args([
                "get",
                "endpoints",
                &service.metadata.name,
                "-n",
                &service.metadata.namespace,
                "-o",
                "jsonpath={.subsets[*].addresses[*].ip}",
            ])
            .kubeconfig(kubeconfig_path)
            .output()
            .await?;

        if ep_output.success && ep_output.stdout.trim().is_empty() {
            services_without_endpoints.push(format!(
                "{}/{}",
                service.metadata.namespace, service.metadata.name
            ));
        }
    }

    Ok(services_without_endpoints)
}

/// Check for pods using 'latest' image tag
async fn check_pods_using_latest_tag(kubeconfig_path: &std::path::Path) -> Result<Vec<String>> {
    let output = CommandBuilder::new("kubectl")
        .args(["get", "pods", "--all-namespaces", "-o", "json"])
        .kubeconfig(kubeconfig_path)
        .context("Failed to get pods")
        .output()
        .await?;

    if !output.success {
        return Ok(Vec::new());
    }

    #[derive(Deserialize)]
    struct PodList {
        items: Vec<Pod>,
    }

    #[derive(Deserialize)]
    struct Pod {
        metadata: PodMetadata,
        spec: PodSpec,
    }

    #[derive(Deserialize)]
    struct PodMetadata {
        name: String,
        namespace: String,
    }

    #[derive(Deserialize)]
    struct PodSpec {
        containers: Vec<Container>,
    }

    #[derive(Deserialize)]
    struct Container {
        image: String,
    }

    let pod_list: PodList = serde_json::from_str(&output.stdout)?;
    let mut pods_using_latest = Vec::new();

    for pod in pod_list.items {
        if should_skip_namespace(&pod.metadata.namespace) {
            continue;
        }

        for container in &pod.spec.containers {
            // Check if image uses :latest or has no tag
            if container.image.ends_with(":latest") || !container.image.contains(':') {
                pods_using_latest.push(format!("{}/{}", pod.metadata.namespace, pod.metadata.name));
                break;
            }
        }
    }

    Ok(pods_using_latest)
}

/// Check for namespaces without resource quotas
async fn check_namespaces_without_quotas(kubeconfig_path: &std::path::Path) -> Result<Vec<String>> {
    // Get all namespaces
    let ns_output = CommandBuilder::new("kubectl")
        .args(["get", "namespaces", "-o", "json"])
        .kubeconfig(kubeconfig_path)
        .context("Failed to get namespaces")
        .output()
        .await?;

    if !ns_output.success {
        return Ok(Vec::new());
    }

    // Get all resource quotas
    let rq_output = CommandBuilder::new("kubectl")
        .args(["get", "resourcequotas", "--all-namespaces", "-o", "json"])
        .kubeconfig(kubeconfig_path)
        .context("Failed to get resource quotas")
        .output()
        .await?;

    #[derive(Deserialize)]
    struct NamespaceList {
        items: Vec<Namespace>,
    }

    #[derive(Deserialize)]
    struct Namespace {
        metadata: NamespaceMetadata,
    }

    #[derive(Deserialize)]
    struct NamespaceMetadata {
        name: String,
    }

    #[derive(Deserialize)]
    struct ResourceQuotaList {
        items: Vec<ResourceQuota>,
    }

    #[derive(Deserialize)]
    struct ResourceQuota {
        metadata: ResourceQuotaMetadata,
    }

    #[derive(Deserialize)]
    struct ResourceQuotaMetadata {
        namespace: String,
    }

    let namespace_list: NamespaceList = serde_json::from_str(&ns_output.stdout)?;
    let mut namespaces_with_quotas = std::collections::HashSet::new();

    if rq_output.success {
        let resource_quota_list: ResourceQuotaList = serde_json::from_str(&rq_output.stdout)?;
        for rq in resource_quota_list.items {
            namespaces_with_quotas.insert(rq.metadata.namespace);
        }
    }

    let mut namespaces_without_quotas = Vec::new();
    for namespace in namespace_list.items {
        if should_skip_namespace(&namespace.metadata.name) {
            continue;
        }

        if !namespaces_with_quotas.contains(&namespace.metadata.name) {
            namespaces_without_quotas.push(namespace.metadata.name);
        }
    }

    Ok(namespaces_without_quotas)
}

/// Check for over-provisioned pods (using <20% of requests)
async fn check_over_provisioned_pods(kubeconfig_path: &std::path::Path) -> Result<Vec<String>> {
    // Get pods with resource specs
    let pods_output = CommandBuilder::new("kubectl")
        .args(["get", "pods", "--all-namespaces", "-o", "json"])
        .kubeconfig(kubeconfig_path)
        .context("Failed to get pods")
        .output()
        .await?;

    if !pods_output.success {
        return Ok(Vec::new());
    }

    // Get pod metrics
    let metrics_output = CommandBuilder::new("kubectl")
        .args(["top", "pods", "--all-namespaces", "--no-headers"])
        .kubeconfig(kubeconfig_path)
        .context("Failed to get pod metrics")
        .output()
        .await?;

    if !metrics_output.success {
        return Ok(Vec::new());
    }

    #[derive(Deserialize)]
    struct PodList {
        items: Vec<Pod>,
    }

    #[derive(Deserialize)]
    struct Pod {
        metadata: PodMetadata,
        spec: PodSpec,
    }

    #[derive(Deserialize)]
    struct PodMetadata {
        name: String,
        namespace: String,
    }

    #[derive(Deserialize)]
    struct PodSpec {
        containers: Vec<Container>,
    }

    #[derive(Deserialize)]
    struct Container {
        #[serde(default)]
        resources: Resources,
    }

    #[derive(Deserialize, Default)]
    struct Resources {
        #[serde(default)]
        requests: std::collections::HashMap<String, String>,
    }

    let pod_list: PodList = serde_json::from_str(&pods_output.stdout)?;

    // Parse metrics into a map
    let mut pod_metrics: std::collections::HashMap<String, (f64, f64)> =
        std::collections::HashMap::new();
    for line in metrics_output.stdout.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 4 {
            let namespace = parts[0];
            let name = parts[1];
            let cpu = parts[2].trim_end_matches("m").parse::<f64>().unwrap_or(0.0);
            let memory = parts[3]
                .trim_end_matches("Mi")
                .parse::<f64>()
                .unwrap_or(0.0);
            pod_metrics.insert(format!("{}/{}", namespace, name), (cpu, memory));
        }
    }

    let mut over_provisioned_pods = Vec::new();

    for pod in pod_list.items {
        if should_skip_namespace(&pod.metadata.namespace) {
            continue;
        }

        let pod_key = format!("{}/{}", pod.metadata.namespace, pod.metadata.name);
        if let Some((actual_cpu, actual_memory)) = pod_metrics.get(&pod_key) {
            // Aggregate all container requests for this pod
            let mut total_cpu_request = 0.0;
            let mut total_memory_request = 0.0;

            for container in &pod.spec.containers {
                if let Some(cpu_request) = container.resources.requests.get("cpu") {
                    total_cpu_request += parse_cpu_to_millicores(cpu_request);
                }
                if let Some(memory_request) = container.resources.requests.get("memory") {
                    total_memory_request += parse_memory_to_mi(memory_request);
                }
            }

            // Check if pod is over-provisioned (using <20% of requests)
            let mut reasons = Vec::new();

            if total_cpu_request > 0.0 && *actual_cpu < total_cpu_request * 0.2 {
                let usage_percent = (*actual_cpu / total_cpu_request * 100.0) as u64;
                reasons.push(format!(
                    "CPU: {}m/{}m ({}%)",
                    *actual_cpu as u64, total_cpu_request as u64, usage_percent
                ));
            }

            if total_memory_request > 0.0 && *actual_memory < total_memory_request * 0.2 {
                let usage_percent = (*actual_memory / total_memory_request * 100.0) as u64;
                reasons.push(format!(
                    "Memory: {}Mi/{}Mi ({}%)",
                    *actual_memory as u64, total_memory_request as u64, usage_percent
                ));
            }

            if !reasons.is_empty() {
                over_provisioned_pods.push(format!("{} ({})", pod_key, reasons.join(", ")));
            }
        }
    }

    Ok(over_provisioned_pods)
}

/// Check for under-provisioned pods (using >90% of limits)
async fn check_under_provisioned_pods(kubeconfig_path: &std::path::Path) -> Result<Vec<String>> {
    // Get pods with resource specs
    let pods_output = CommandBuilder::new("kubectl")
        .args(["get", "pods", "--all-namespaces", "-o", "json"])
        .kubeconfig(kubeconfig_path)
        .context("Failed to get pods")
        .output()
        .await?;

    if !pods_output.success {
        return Ok(Vec::new());
    }

    // Get pod metrics
    let metrics_output = CommandBuilder::new("kubectl")
        .args(["top", "pods", "--all-namespaces", "--no-headers"])
        .kubeconfig(kubeconfig_path)
        .context("Failed to get pod metrics")
        .output()
        .await?;

    if !metrics_output.success {
        return Ok(Vec::new());
    }

    #[derive(Deserialize)]
    struct PodList {
        items: Vec<Pod>,
    }

    #[derive(Deserialize)]
    struct Pod {
        metadata: PodMetadata,
        spec: PodSpec,
    }

    #[derive(Deserialize)]
    struct PodMetadata {
        name: String,
        namespace: String,
    }

    #[derive(Deserialize)]
    struct PodSpec {
        containers: Vec<Container>,
    }

    #[derive(Deserialize)]
    struct Container {
        #[serde(default)]
        resources: Resources,
    }

    #[derive(Deserialize, Default)]
    struct Resources {
        #[serde(default)]
        limits: std::collections::HashMap<String, String>,
    }

    let pod_list: PodList = serde_json::from_str(&pods_output.stdout)?;

    // Parse metrics into a map
    let mut pod_metrics: std::collections::HashMap<String, (f64, f64)> =
        std::collections::HashMap::new();
    for line in metrics_output.stdout.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 4 {
            let namespace = parts[0];
            let name = parts[1];
            let cpu = parts[2].trim_end_matches("m").parse::<f64>().unwrap_or(0.0);
            let memory = parts[3]
                .trim_end_matches("Mi")
                .parse::<f64>()
                .unwrap_or(0.0);
            pod_metrics.insert(format!("{}/{}", namespace, name), (cpu, memory));
        }
    }

    let mut under_provisioned_pods = Vec::new();

    for pod in pod_list.items {
        if should_skip_namespace(&pod.metadata.namespace) {
            continue;
        }

        let pod_key = format!("{}/{}", pod.metadata.namespace, pod.metadata.name);
        if let Some((actual_cpu, actual_memory)) = pod_metrics.get(&pod_key) {
            // Aggregate all container limits for this pod
            let mut total_cpu_limit = 0.0;
            let mut total_memory_limit = 0.0;

            for container in &pod.spec.containers {
                if let Some(cpu_limit) = container.resources.limits.get("cpu") {
                    total_cpu_limit += parse_cpu_to_millicores(cpu_limit);
                }
                if let Some(memory_limit) = container.resources.limits.get("memory") {
                    total_memory_limit += parse_memory_to_mi(memory_limit);
                }
            }

            // Check if pod is under-provisioned (using >90% of limits)
            let mut reasons = Vec::new();

            if total_cpu_limit > 0.0 && *actual_cpu > total_cpu_limit * 0.9 {
                let usage_percent = (*actual_cpu / total_cpu_limit * 100.0) as u64;
                reasons.push(format!(
                    "CPU: {}m/{}m ({}%)",
                    *actual_cpu as u64, total_cpu_limit as u64, usage_percent
                ));
            }

            if total_memory_limit > 0.0 && *actual_memory > total_memory_limit * 0.9 {
                let usage_percent = (*actual_memory / total_memory_limit * 100.0) as u64;
                reasons.push(format!(
                    "Memory: {}Mi/{}Mi ({}%)",
                    *actual_memory as u64, total_memory_limit as u64, usage_percent
                ));
            }

            if !reasons.is_empty() {
                under_provisioned_pods.push(format!("{} ({})", pod_key, reasons.join(", ")));
            }
        }
    }

    Ok(under_provisioned_pods)
}

/// Parse CPU resource string to millicores
fn parse_cpu_to_millicores(cpu: &str) -> f64 {
    if let Some(millicores) = cpu.strip_suffix('m') {
        millicores.parse::<f64>().unwrap_or(0.0)
    } else {
        // Full cores (e.g., "1" or "0.5")
        cpu.parse::<f64>().unwrap_or(0.0) * 1000.0
    }
}

/// Parse memory resource string to Mi
fn parse_memory_to_mi(memory: &str) -> f64 {
    if let Some(mi) = memory.strip_suffix("Mi") {
        mi.parse::<f64>().unwrap_or(0.0)
    } else if let Some(gi) = memory.strip_suffix("Gi") {
        gi.parse::<f64>().unwrap_or(0.0) * 1024.0
    } else if let Some(ki) = memory.strip_suffix("Ki") {
        ki.parse::<f64>().unwrap_or(0.0) / 1024.0
    } else if let Some(m) = memory.strip_suffix('M') {
        m.parse::<f64>().unwrap_or(0.0)
    } else if let Some(g) = memory.strip_suffix('G') {
        g.parse::<f64>().unwrap_or(0.0) * 1024.0
    } else if let Some(k) = memory.strip_suffix('K') {
        k.parse::<f64>().unwrap_or(0.0) / 1024.0
    } else {
        // Bytes
        memory.parse::<f64>().unwrap_or(0.0) / (1024.0 * 1024.0)
    }
}
