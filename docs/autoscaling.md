# Kubernetes Cluster Autoscaler

This document explains how to configure and use the Kubernetes Cluster Autoscaler with Hetzner Cloud integration for automatic worker node scaling.

## Overview

The Cluster Autoscaler automatically adjusts the number of worker nodes in your cluster based on pod resource demands:

- **Scale Up**: Adds nodes when pods cannot be scheduled due to insufficient resources
- **Scale Down**: Removes underutilized nodes to save costs

The autoscaler uses the official Kubernetes Cluster Autoscaler with native Hetzner Cloud provider support.

## How It Works

```
Pods pending due to insufficient resources
    ↓
Cluster Autoscaler detects unschedulable pods
    ↓
Calculates required nodes based on resource requests
    ↓
Calls Hetzner API to create new servers
    ↓
Configures servers with Talos using cloud-init
    ↓
Nodes automatically join cluster
    ↓
Pods get scheduled on new nodes
    ↓
(After 10 minutes of low utilization)
    ↓
Autoscaler removes underutilized nodes
    ↓
Deletes servers via Hetzner API
```

## Configuration

### Enable in Cluster Configuration

Add the autoscaler configuration to your `cluster.yaml`:

```yaml
autoscaler:
  # Enable cluster autoscaler
  enabled: true

  # Cluster autoscaler version
  # See: https://github.com/kubernetes/autoscaler/releases
  version: v1.35.0

  # Worker pools to autoscale
  worker_pools:
    - name: worker-pool
      server_type: cpx11 # Hetzner server type
      location: fsn1 # Hetzner location
      min_nodes: 1 # Minimum nodes (won't scale below)
      max_nodes: 10 # Maximum nodes (won't scale above)
```

### Configuration Options

| Field          | Description                        | Required | Default |
| -------------- | ---------------------------------- | -------- | ------- |
| `enabled`      | Enable cluster autoscaler          | Yes      | false   |
| `version`      | Autoscaler version (e.g., v1.35.0) | No       | v1.35.0 |
| `worker_pools` | List of worker pools to autoscale  | Yes      | []      |

### Worker Pool Configuration

| Field         | Description                               | Required |
| ------------- | ----------------------------------------- | -------- |
| `name`        | Worker pool name (must match node prefix) | Yes      |
| `server_type` | Hetzner server type (cpx11, cpx21, etc.)  | Yes      |
| `location`    | Hetzner location (fsn1, nbg1, ash, etc.)  | Yes      |
| `min_nodes`   | Minimum number of nodes                   | Yes      |
| `max_nodes`   | Maximum number of nodes                   | Yes      |

### Multiple Worker Pools

You can configure multiple worker pools with different specifications:

```yaml
autoscaler:
  enabled: true
  worker_pools:
    # Small general-purpose pool
    - name: worker-small
      server_type: cpx11
      location: fsn1
      min_nodes: 2
      max_nodes: 10

    # Large CPU-intensive pool
    - name: worker-large
      server_type: cpx41
      location: fsn1
      min_nodes: 0
      max_nodes: 5
```

## Installation

### Automatic Installation (During Cluster Creation)

The autoscaler is automatically deployed when you create a cluster with `autoscaler.enabled: true`:

```bash
oxide create
```

The cluster creation process will:

1. Create the cluster infrastructure
2. Install Cilium CNI
3. Deploy the Cluster Autoscaler (if enabled)
4. Create required RBAC permissions
5. Store Hetzner API token and Talos config

### Manual Installation (Existing Cluster)

Deploy autoscaler to an existing cluster:

```bash
oxide deploy-autoscaler
```

This will:

- Create `oxide-system` namespace
- Deploy autoscaler deployment
- Create ServiceAccount and RBAC permissions
- Store Hetzner Cloud API token as secret
- Store Talos worker config as ConfigMap

### Verify Installation

```bash
# Check autoscaler pod
kubectl get pods -n oxide-system -l app=cluster-autoscaler

# View autoscaler logs
kubectl logs -n oxide-system -l app=cluster-autoscaler -f
```

## Scaling Behavior

### Scale Up

**Trigger**: Pods in Pending state due to insufficient resources

**Process**:

1. Autoscaler detects unschedulable pods every 10 seconds
2. Calculates minimum nodes needed based on pod resource requests
3. Selects appropriate node group based on server type
4. Creates new Hetzner servers via API
5. Configures servers with Talos using cloud-init
6. Waits for nodes to join cluster (typically 1-2 minutes)
7. Kubernetes scheduler places pending pods on new nodes

**Scale-up characteristics**:

- **Scan interval**: 10 seconds (checks for pending pods)
- **Max nodes per cycle**: Unlimited (scales to handle all pending pods)
- **Node creation time**: ~2 minutes (Hetzner server boot + Talos init)

**Example scale-up scenario**:

```bash
# Deploy app requiring 59 pods × 100m CPU each = 5900m total
kubectl scale deployment nginx --replicas=59

# Current capacity: 3 workers × 1950m CPU = 5850m
# Required: 5900m
# Result: Autoscaler creates 1 additional worker
```

### Scale Down

**Trigger**: Node utilization below threshold for 10 minutes

**Process**:

1. Autoscaler identifies underutilized nodes every 10 seconds
2. Simulates pod rescheduling to verify safety
3. Checks PodDisruptionBudgets won't be violated
4. Waits for 10-minute stabilization window
5. Drains pods from node gracefully
6. Deletes node from Kubernetes
7. Deletes Hetzner server via API

**Scale-down characteristics**:

- **Utilization threshold**: CPU/memory requests < 50% of allocatable
- **Unneeded duration**: 10 minutes (configurable)
- **Max nodes removed per cycle**: 1-2 nodes
- **Respect**: PodDisruptionBudgets, system pods, local storage

**Nodes are NOT removed if**:

- Node utilization is above threshold (>50% CPU or memory requested)
- Less than 10 minutes have passed since becoming underutilized
- PodDisruptionBudget would be violated
- Pods with local storage present (unless configured otherwise)
- System pods without tolerations are present

**Example scale-down scenario**:

```bash
# Scale down deployment
kubectl scale deployment nginx --replicas=3

# 4 workers now exist but only 3 needed
# Autoscaler marks 4th worker as "unneeded"
# After 10 minutes: autoscaler removes the unneeded worker
```

### Important Timing Defaults

| Event                  | Default Interval | Description                                   |
| ---------------------- | ---------------- | --------------------------------------------- |
| Pod check              | 10 seconds       | How often to scan for pending pods            |
| Node utilization check | 10 seconds       | How often to check for underutilized nodes    |
| Unneeded duration      | 10 minutes       | How long a node must be underutilized         |
| Scale-up cooldown      | 0 seconds        | Wait time after scale-up before next scale-up |
| Scale-down cooldown    | 10 minutes       | Wait time after scale-down before next        |

## Resource Request Requirements

**Critical**: The autoscaler makes decisions based on **pod resource requests**, not actual usage.

### Pods MUST Have Resource Requests

```yaml
# Good - Autoscaler can calculate capacity
apiVersion: v1
kind: Pod
metadata:
  name: my-app
spec:
  containers:
    - name: app
      image: nginx
      resources:
        requests:
          cpu: 100m
          memory: 128Mi
        limits:
          cpu: 200m
          memory: 256Mi
```

```yaml
# Bad - Autoscaler ignores pods without requests
apiVersion: v1
kind: Pod
metadata:
  name: broken-app
spec:
  containers:
    - name: app
      image: nginx
      # No resources specified!
```

**Without resource requests:**

- Pods won't trigger scale-up even if unschedulable
- Nodes appear underutilized even if heavily loaded
- Autoscaler cannot make informed decisions

## Monitoring Autoscaler

### View Autoscaler Logs

```bash
# Follow autoscaler logs in real-time
kubectl logs -n oxide-system -l app=cluster-autoscaler -f

# View last 100 lines
kubectl logs -n oxide-system -l app=cluster-autoscaler --tail=100
```

### Key Log Messages

**Scale-up triggered:**

```
I1006 07:38:09 scale_up.go:476] Scale-up: group snoculars-worker total size increase: 1
```

**Node marked as unneeded:**

```
I1006 07:39:36 nodes.go:153] snoculars-worker-652a25ad0aafca7b is unneeded since 2025-10-06 07:39:36
```

**Starting scale-down:**

```
I1006 07:49:36 static_autoscaler.go:634] Starting scale down
```

**No unschedulable pods:**

```
I1006 07:48:59 static_autoscaler.go:525] No unschedulable pods
```

### Check Cluster Status

```bash
# View all nodes
kubectl get nodes

# Check node resource allocation
kubectl describe nodes | grep -A 5 "Allocated resources"

# View pending pods (triggers scale-up)
kubectl get pods -A --field-selector=status.phase=Pending

# Check autoscaler events
kubectl get events -n oxide-system --sort-by='.lastTimestamp'
```

### Monitor Resource Usage

```bash
# Requires metrics-server
kubectl top nodes
kubectl top pods -A

# View pod resource requests per node
kubectl describe node <node-name> | grep -A 10 "Allocated resources"
```

## Testing Autoscaling

### Test Scale-Up

Create a deployment that requires more resources than available:

```bash
# Deploy nginx with specific resource requests
cat <<EOF | kubectl apply -f -
apiVersion: apps/v1
kind: Deployment
metadata:
  name: nginx-scale-test
  namespace: default
spec:
  replicas: 60  # This will exceed initial cluster capacity
  selector:
    matchLabels:
      app: nginx-scale-test
  template:
    metadata:
      labels:
        app: nginx-scale-test
    spec:
      containers:
        - name: nginx
          image: nginx
          resources:
            requests:
              cpu: 100m
              memory: 128Mi
            limits:
              cpu: 100m
              memory: 128Mi
EOF
```

**Watch the autoscaler**:

```bash
# Terminal 1: Watch nodes
watch kubectl get nodes

# Terminal 2: Watch autoscaler logs
kubectl logs -n oxide-system -l app=cluster-autoscaler -f

# Terminal 3: Watch pod status
watch "kubectl get pods -n default | grep nginx-scale-test"
```

**Expected behavior**:

1. Some pods remain in Pending state
2. Autoscaler detects unschedulable pods (~10 seconds)
3. New worker nodes are created (~2 minutes)
4. Nodes join cluster and become Ready
5. Pending pods get scheduled

### Test Scale-Down

Reduce the deployment size to trigger scale-down:

```bash
# Scale down to minimal replicas
kubectl scale deployment nginx-scale-test --replicas=3

# Watch autoscaler logs
kubectl logs -n oxide-system -l app=cluster-autoscaler -f
```

**Expected behavior**:

1. Autoscaler marks nodes as "unneeded" immediately
2. Waits 10 minutes for stabilization
3. After 10 minutes: drains and removes node
4. Deletes Hetzner server

**Check unneeded node duration**:

```bash
# Look for log entries like:
# "snoculars-worker-xyz is unneeded since ... duration 9m32s"
kubectl logs -n oxide-system -l app=cluster-autoscaler | grep "unneeded since"
```

## Best Practices

### 1. Always Set Resource Requests

```yaml
# Every pod should have resource requests
resources:
  requests:
    cpu: 100m # Minimum CPU required
    memory: 128Mi # Minimum memory required
  limits:
    cpu: 200m # Maximum CPU allowed
    memory: 256Mi # Maximum memory allowed
```

### 2. Use PodDisruptionBudgets

Prevent autoscaler from disrupting critical services:

```yaml
apiVersion: policy/v1
kind: PodDisruptionBudget
metadata:
  name: my-app-pdb
spec:
  minAvailable: 2 # Keep at least 2 pods running
  selector:
    matchLabels:
      app: my-app
```

### 3. Configure Appropriate Min/Max Nodes

```yaml
worker_pools:
  - name: worker-pool
    min_nodes: 2 # Handle baseline load + some buffer
    max_nodes: 10 # Prevent runaway costs
```

**Recommendations**:

- **min_nodes**: Set to handle baseline load + 1 for buffer
- **max_nodes**: Set based on budget constraints
- Start conservative, adjust based on observed patterns

### 4. Use HorizontalPodAutoscaler (HPA) Together

Combine Cluster Autoscaler with HPA for complete autoscaling:

```yaml
# HPA scales pods based on CPU/memory usage
apiVersion: autoscaling/v2
kind: HorizontalPodAutoscaler
metadata:
  name: my-app-hpa
spec:
  scaleTargetRef:
    apiVersion: apps/v1
    kind: Deployment
    name: my-app
  minReplicas: 3
  maxReplicas: 100
  metrics:
    - type: Resource
      resource:
        name: cpu
        target:
          type: Utilization
          averageUtilization: 60
```

**How they work together**:

1. HPA scales pods up based on actual CPU/memory usage
2. If pods can't be scheduled, Cluster Autoscaler adds nodes
3. When load decreases, HPA scales pods down
4. Cluster Autoscaler removes underutilized nodes

### 5. Monitor Costs

```bash
# Check current node count
kubectl get nodes -l node-role.kubernetes.io/worker

# Calculate estimated costs
# Example: 5 cpx11 workers × €4.90/month = €24.50/month
```

### 6. Set Up Alerts

Monitor for:

- Nodes stuck in NotReady state
- Persistent pending pods
- Frequent scale-up/down cycles (thrashing)
- Autoscaler errors in logs

## Autoscaler Configuration Flags

The autoscaler is deployed with these flags:

```yaml
args:
  - --cloud-provider=hetzner
  - --nodes=3:10:cpx11:fsn1:worker-pool # min:max:type:location:name
  - --skip-nodes-with-system-pods=false
  - --skip-nodes-with-local-storage=false
  - --balance-similar-node-groups
  - --expander=least-waste
  - --v=4 # Verbosity level (1-10)
```

### Flag Explanations

| Flag                              | Description                                            |
| --------------------------------- | ------------------------------------------------------ |
| `--cloud-provider=hetzner`        | Use Hetzner Cloud API                                  |
| `--nodes=min:max:type:loc:name`   | Define autoscaling group                               |
| `--skip-nodes-with-system-pods`   | Don't remove nodes with system pods (false = remove)   |
| `--skip-nodes-with-local-storage` | Don't remove nodes with local storage (false = remove) |
| `--balance-similar-node-groups`   | Distribute load across similar node groups             |
| `--expander=least-waste`          | Choose node group that wastes least resources          |
| `--v=4`                           | Log verbosity (4 = informational)                      |

## Troubleshooting

### Pods Stay Pending Despite Autoscaler

**Check**: Do pods have resource requests?

```bash
kubectl get pod <pod-name> -o yaml | grep -A 10 "resources:"
```

**Solution**: Add resource requests to pod spec.

**Check**: Are you at max_nodes limit?

```bash
# Count current nodes
kubectl get nodes -l node-role.kubernetes.io/worker --no-headers | wc -l

# Compare to max_nodes in cluster.yaml
```

**Solution**: Increase max_nodes or scale down other workloads.

### Nodes Not Scaling Down

**Check**: How long has node been underutilized?

```bash
kubectl logs -n oxide-system -l app=cluster-autoscaler | grep "unneeded since"
```

**Solution**: Wait 10 minutes from when node becomes unneeded.

**Check**: Are there pods preventing scale-down?

```bash
# Check for pods with local storage
kubectl get pods -o wide | grep <node-name>

# Check pod specs for local volumes
kubectl get pod <pod-name> -o yaml | grep -A 5 "volumes:"
```

**Solution**: Use PersistentVolumes instead of local storage.

**Check**: Is PodDisruptionBudget blocking?

```bash
kubectl get pdb -A
```

**Solution**: Adjust PDB minAvailable/maxUnavailable settings.

### Autoscaler Not Creating Nodes

**Check**: Autoscaler logs for errors

```bash
kubectl logs -n oxide-system -l app=cluster-autoscaler --tail=100
```

**Common errors**:

1. **"Failed to create node group"** - Check Hetzner API token

   ```bash
   kubectl get secret oxide-hcloud-token -n oxide-system -o yaml
   ```

2. **"Snapshot not found"** - Verify Talos snapshot ID exists

   ```bash
   hcloud image list | grep snapshot
   ```

3. **"Server creation failed"** - Check Hetzner quota limits
   - Log in to Hetzner Console → Check resource limits

### Nodes Created But Don't Join Cluster

**Check**: Talos config validity

```bash
kubectl get configmap oxide-talos-config -n oxide-system -o yaml
```

**Check**: Node can reach cluster endpoint

```bash
# SSH into node (if rescue mode available) and check connectivity
ping <cluster-endpoint-ip>
```

**Check**: Firewall rules

```bash
hcloud firewall describe <firewall-name>
```

**Solution**: Ensure Talos API (50000) and Kubernetes API (6443) are accessible.

### Frequent Scale Up/Down Cycles (Thrashing)

**Symptom**: Nodes constantly being added and removed

**Causes**:

- Resource requests too close to node capacity
- HPA settings too aggressive
- Workload with high variance

**Solutions**:

1. Increase buffer between min_nodes and actual usage

   ```yaml
   min_nodes: 3 # Instead of 1
   ```

2. Adjust HPA settings for slower scaling

   ```yaml
   behavior:
     scaleDown:
       stabilizationWindowSeconds: 300 # Wait 5 minutes
   ```

3. Use multiple worker pools for different workload types

## Uninstallation

```bash
# Remove autoscaler deployment
oxide uninstall-autoscaler

# Manually clean up if needed
kubectl delete namespace oxide-system
```

**Note**: This does NOT remove nodes created by autoscaler. Remove them manually:

```bash
# Scale down to desired count
oxide scale worker --count 3

# Or delete specific autoscaler-created nodes
kubectl delete node <node-name>
hcloud server delete <server-name>
```

## Advanced Configuration

### Custom Expander Strategy

The expander determines which node group to use when multiple groups can satisfy the scale-up:

**Available expanders**:

- `least-waste` (default) - Choose group that minimizes wasted resources
- `most-pods` - Choose group that fits most pending pods
- `priority` - Use predefined priority order
- `random` - Random selection

**Change expander** (requires editing deployment):

```bash
kubectl edit deployment cluster-autoscaler -n oxide-system

# Change --expander=least-waste to desired strategy
```

### Adjust Scale-Down Timing

To change how long nodes must be underutilized before removal:

```bash
kubectl edit deployment cluster-autoscaler -n oxide-system

# Add flag:
# - --scale-down-unneeded-time=5m  # Default: 10m
```

### Increase Logging Verbosity

```bash
kubectl edit deployment cluster-autoscaler -n oxide-system

# Change --v=4 to --v=6 for more detailed logs
# Levels: 1 (errors) to 10 (everything)
```

## References

- [Kubernetes Cluster Autoscaler Documentation](https://github.com/kubernetes/autoscaler/tree/master/cluster-autoscaler)
- [Cluster Autoscaler FAQ](https://github.com/kubernetes/autoscaler/blob/master/cluster-autoscaler/FAQ.md)
- [Hetzner Cloud Provider](https://github.com/kubernetes/autoscaler/tree/master/cluster-autoscaler/cloudprovider/hetzner)
- [Resource Requests and Limits](https://kubernetes.io/docs/concepts/configuration/manage-resources-containers/)
- [PodDisruptionBudgets](https://kubernetes.io/docs/tasks/run-application/configure-pdb/)
- [HorizontalPodAutoscaler](https://kubernetes.io/docs/tasks/run-application/horizontal-pod-autoscale/)
