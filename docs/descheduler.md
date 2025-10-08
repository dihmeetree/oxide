# Kubernetes Descheduler

This document explains how to configure and use the Kubernetes Descheduler to rebalance workloads across your cluster nodes.

## Overview

The Descheduler complements the Kubernetes Cluster Autoscaler by rebalancing existing pods across nodes. While the scheduler only runs when pods are created, the descheduler continuously evaluates running pods and evicts them when they violate policies or when node utilization becomes unbalanced.

**Key differences from Cluster Autoscaler:**

- **Cluster Autoscaler**: Adds/removes nodes based on resource demand
- **Descheduler**: Moves existing pods between nodes to optimize distribution

**Use cases:**

- **Rebalancing after scale-up**: New nodes added by autoscaler are empty while old nodes remain heavily loaded
- **Resource optimization**: Spread workloads evenly to prevent hotspots
- **Policy enforcement**: Ensure pods comply with affinity rules and topology constraints

## How It Works

```
Descheduler runs (every 1 minute via CronJob)
    ↓
Queries node resource utilization from metrics-server
    ↓
Identifies nodes below 20% CPU/memory (underutilized)
    ↓
Identifies nodes above 50% CPU/memory (overutilized)
    ↓
Evaluates pods on overutilized nodes for eviction
    ↓
Evicts pods that violate policies or can improve balance
    ↓
Kubernetes scheduler places evicted pods on underutilized nodes
    ↓
Waits until next scheduled run (1 minute)
```

**Important**: The descheduler does NOT reschedule pods itself. It only evicts them, and the Kubernetes scheduler handles placement on new nodes.

## Installation

### Prerequisites

- **metrics-server** must be installed for the descheduler to evaluate node utilization
- Cluster must have available capacity for rescheduling evicted pods

### Install via Helm

```bash
# Add the descheduler Helm repository
helm repo add descheduler https://kubernetes-sigs.github.io/descheduler
helm repo update

# Install with default configuration
helm install descheduler descheduler/descheduler \
  --namespace kube-system \
  --set kind=CronJob \
  --set schedule="*/1 * * * *" \
  --kubeconfig output/kubeconfig
```

### Install with Custom Configuration

```bash
# Install with LowNodeUtilization strategy configured
helm install descheduler descheduler/descheduler \
  --namespace kube-system \
  --set kind=CronJob \
  --set schedule="*/1 * * * *" \
  --set deschedulerPolicy.strategies.LowNodeUtilization.enabled=true \
  --set deschedulerPolicy.strategies.LowNodeUtilization.params.nodeResourceUtilizationThresholds.thresholds.cpu=20 \
  --set deschedulerPolicy.strategies.LowNodeUtilization.params.nodeResourceUtilizationThresholds.thresholds.memory=20 \
  --set deschedulerPolicy.strategies.LowNodeUtilization.params.nodeResourceUtilizationThresholds.targetThresholds.cpu=50 \
  --set deschedulerPolicy.strategies.LowNodeUtilization.params.nodeResourceUtilizationThresholds.targetThresholds.memory=50 \
  --kubeconfig output/kubeconfig
```

### Verify Installation

```bash
# Check CronJob was created
kubectl get cronjob -n kube-system descheduler --kubeconfig output/kubeconfig

# View descheduler policy
kubectl get configmap -n kube-system descheduler -o yaml --kubeconfig output/kubeconfig

# Watch for jobs being created
kubectl get jobs -n kube-system --kubeconfig output/kubeconfig

# View logs from last run
kubectl logs -n kube-system -l app.kubernetes.io/name=descheduler --tail=50 --kubeconfig output/kubeconfig
```

## Rebalancing Strategies

The descheduler supports multiple strategies for identifying which pods to evict. Strategies can be combined for comprehensive rebalancing.

### LowNodeUtilization

**Purpose**: Balance workloads between underutilized and overutilized nodes

**How it works:**

1. Calculates node utilization based on **requested resources** (not actual usage)
2. Nodes with utilization below threshold = underutilized
3. Nodes with utilization above target threshold = overutilized
4. Evicts pods from overutilized nodes to be rescheduled on underutilized nodes

**Configuration:**

```yaml
apiVersion: "descheduler/v1alpha2"
kind: "DeschedulerPolicy"
profiles:
  - name: default
    pluginConfig:
      - name: "LowNodeUtilization"
        args:
          # Nodes below these thresholds are considered underutilized
          thresholds:
            cpu: 20 # 20% CPU requested
            memory: 20 # 20% memory requested
            pods: 20 # 20% pod count

          # Nodes above these thresholds are considered overutilized
          targetThresholds:
            cpu: 50 # 50% CPU requested
            memory: 50 # 50% memory requested
            pods: 50 # 50% pod count
```

**Example scenario:**

```
Cluster state:
- Node A: 80% CPU requested (5 pods) - OVERUTILIZED
- Node B: 15% CPU requested (1 pod)  - UNDERUTILIZED
- Node C: 12% CPU requested (1 pod)  - UNDERUTILIZED

Action:
- Descheduler evicts 2-3 pods from Node A
- Scheduler places them on Nodes B and C
- Result: More balanced distribution (40%/35%/35%)
```

**Best for:**

- Clusters with autoscaling where new nodes are added but remain empty
- Preventing node hotspots
- Improving resource utilization efficiency

### RemoveDuplicates

**Purpose**: Spread replicas of the same deployment/replicaset across different nodes

**How it works:**

1. Identifies pods from the same ReplicaSet running on a single node
2. Evicts duplicate pods (keeps one per node)
3. Scheduler places evicted pods on nodes without that replica

**Example scenario:**

```
Cluster state:
- Node A: nginx-abc (replica 1), nginx-xyz (replica 2), nginx-def (replica 3)
- Node B: Empty
- Node C: Empty

Action:
- Descheduler evicts nginx-xyz and nginx-def from Node A
- Scheduler places them on Nodes B and C
- Result: nginx-abc on Node A, nginx-xyz on Node B, nginx-def on Node C
```

**Best for:**

- Improving high availability
- Preventing single points of failure
- Spreading load across nodes

### RemovePodsViolatingNodeAffinity

**Purpose**: Evict pods that don't match their node affinity requirements

**How it works:**

1. Checks each pod's `nodeAffinity` rules
2. Evicts pods on nodes that don't satisfy affinity requirements
3. Scheduler places them on compliant nodes

**Example scenario:**

```yaml
# Pod has affinity for nodes with SSD
nodeAffinity:
  requiredDuringSchedulingIgnoredDuringExecution:
    nodeSelectorTerms:
      - matchExpressions:
          - key: disktype
            operator: In
            values:
              - ssd
# But pod is running on node with disktype=hdd
# Descheduler evicts it to be rescheduled on correct node
```

**Best for:**

- Enforcing hardware requirements (GPU, SSD, etc.)
- Moving pods after node label changes
- Cleanup after node pool modifications

### RemovePodsViolatingInterPodAntiAffinity

**Purpose**: Evict pods that violate inter-pod anti-affinity rules

**How it works:**

1. Checks pods' anti-affinity rules
2. Evicts pods that shouldn't be co-located but are
3. Scheduler places them on separate nodes

**Best for:**

- Enforcing pod separation policies
- High availability configurations

### RemovePodsViolatingTopologySpreadConstraint

**Purpose**: Evict pods that violate topology spread constraints

**How it works:**

1. Evaluates `topologySpreadConstraints` defined in pod specs
2. Evicts pods that create imbalanced distribution across topology domains
3. Scheduler redistributes them for better spread

**Best for:**

- Multi-zone deployments
- Ensuring even distribution across availability zones or regions

### RemovePodsHavingTooManyRestarts

**Purpose**: Evict pods that have restarted excessively

**Configuration:**

```yaml
pluginConfig:
  - name: "RemovePodsHavingTooManyRestarts"
    args:
      podRestartThreshold: 100 # Evict if >100 restarts
      includingInitContainers: true # Count init container restarts
```

**Best for:**

- Cleaning up crashlooping pods
- Moving problematic pods to healthier nodes

### PodLifeTime

**Purpose**: Evict pods that have been running longer than a specified duration

**Configuration:**

```yaml
pluginConfig:
  - name: "PodLifeTime"
    args:
      maxPodLifeTimeSeconds: 86400 # 24 hours
```

**Best for:**

- Forcing pod recreation for updates
- Preventing stale pod states
- Regular pod rotation

## Configuration

### Default Configuration

The Helm chart installs with this default policy:

```yaml
apiVersion: "descheduler/v1alpha2"
kind: "DeschedulerPolicy"
profiles:
  - name: default
    pluginConfig:
      - name: DefaultEvictor
        args:
          evictLocalStoragePods: true # Allow evicting pods with local storage
          ignorePvcPods: true # Don't evict pods with PVCs

      - name: LowNodeUtilization
        args:
          thresholds:
            cpu: 20
            memory: 20
            pods: 20
          targetThresholds:
            cpu: 50
            memory: 50
            pods: 50

      - name: RemoveDuplicates
      - name: RemovePodsViolatingNodeAffinity
      - name: RemovePodsViolatingNodeTaints
      - name: RemovePodsViolatingInterPodAntiAffinity
      - name: RemovePodsViolatingTopologySpreadConstraint

      - name: RemovePodsHavingTooManyRestarts
        args:
          podRestartThreshold: 100
          includingInitContainers: true

    plugins:
      balance:
        enabled:
          - RemoveDuplicates
          - RemovePodsViolatingTopologySpreadConstraint
          - LowNodeUtilization

      deschedule:
        enabled:
          - RemovePodsHavingTooManyRestarts
          - RemovePodsViolatingNodeTaints
          - RemovePodsViolatingNodeAffinity
          - RemovePodsViolatingInterPodAntiAffinity
```

### Customizing the Schedule

Change how often the descheduler runs:

```bash
# Run every 1 minute
helm upgrade descheduler descheduler/descheduler \
  --namespace kube-system \
  --set schedule="*/1 * * * *" \
  --reuse-values \
  --kubeconfig output/kubeconfig

# Run every 5 minutes
helm upgrade descheduler descheduler/descheduler \
  --namespace kube-system \
  --set schedule="*/5 * * * *" \
  --reuse-values \
  --kubeconfig output/kubeconfig

# Run every 30 minutes
helm upgrade descheduler descheduler/descheduler \
  --namespace kube-system \
  --set schedule="*/30 * * * *" \
  --reuse-values \
  --kubeconfig output/kubeconfig
```

### Adjusting Utilization Thresholds

**Lower thresholds (more aggressive rebalancing):**

```bash
helm upgrade descheduler descheduler/descheduler \
  --namespace kube-system \
  --set deschedulerPolicy.strategies.LowNodeUtilization.params.nodeResourceUtilizationThresholds.thresholds.cpu=10 \
  --set deschedulerPolicy.strategies.LowNodeUtilization.params.nodeResourceUtilizationThresholds.targetThresholds.cpu=40 \
  --reuse-values \
  --kubeconfig output/kubeconfig
```

**Higher thresholds (less aggressive rebalancing):**

```bash
helm upgrade descheduler descheduler/descheduler \
  --namespace kube-system \
  --set deschedulerPolicy.strategies.LowNodeUtilization.params.nodeResourceUtilizationThresholds.thresholds.cpu=30 \
  --set deschedulerPolicy.strategies.LowNodeUtilization.params.nodeResourceUtilizationThresholds.targetThresholds.cpu=70 \
  --reuse-values \
  --kubeconfig output/kubeconfig
```

### Running as Deployment Instead of CronJob

For continuous descheduling instead of periodic:

```bash
helm upgrade descheduler descheduler/descheduler \
  --namespace kube-system \
  --set kind=Deployment \
  --set deschedulingInterval=1m \
  --kubeconfig output/kubeconfig
```

## Monitoring

### View Descheduler Activity

```bash
# Check last run time
kubectl get cronjob -n kube-system descheduler --kubeconfig output/kubeconfig

# List recent jobs
kubectl get jobs -n kube-system -l app.kubernetes.io/name=descheduler --kubeconfig output/kubeconfig

# View logs from most recent run
kubectl logs -n kube-system -l app.kubernetes.io/name=descheduler --tail=100 --kubeconfig output/kubeconfig

# Follow logs in real-time (if running as Deployment)
kubectl logs -n kube-system -l app.kubernetes.io/name=descheduler -f --kubeconfig output/kubeconfig
```

### Key Log Messages

**Pods evicted:**

```
I1008 13:20:15 pod_evictor.go:123] Evicted pod: default/nginx-6c847b5464-fqr2g
```

**Node utilization assessment:**

```
I1008 13:20:15 lownodeutilization.go:156] Node "snoculars-worker-1" is overutilized: cpu=85.5%, memory=45.2%
I1008 13:20:15 lownodeutilization.go:156] Node "snoculars-worker-2d8ed1093e51020c" is underutilized: cpu=5.2%, memory=8.1%
```

**No action needed:**

```
I1008 13:20:15 lownodeutilization.go:200] No nodes are underutilized or overutilized
```

### Check Node Balance

Before and after descheduler runs:

```bash
# View node resource allocation
kubectl top nodes --kubeconfig output/kubeconfig

# Detailed per-node breakdown
kubectl describe nodes --kubeconfig output/kubeconfig | grep -A 5 "Allocated resources"

# Count pods per node
kubectl get pods -A -o wide --kubeconfig output/kubeconfig | awk '{print $7}' | sort | uniq -c
```

## Best Practices

### 1. Start with Conservative Schedules

Begin with longer intervals and adjust based on cluster behavior:

```bash
# Start: Run every 5 minutes
schedule: "*/5 * * * *"

# After observing: Adjust to 2 minutes if needed
schedule: "*/2 * * * *"

# For active clusters: 1 minute for rapid rebalancing
schedule: "*/1 * * * *"
```

### 2. Set Appropriate Thresholds

**Production recommendations:**

- **Thresholds**: 20-30% (underutilized)
- **Target thresholds**: 50-70% (overutilized)

**Dev/test clusters:**

- **Thresholds**: 10% (more aggressive)
- **Target thresholds**: 40% (more aggressive)

### 3. Use with PodDisruptionBudgets

Prevent the descheduler from disrupting critical services:

```yaml
apiVersion: policy/v1
kind: PodDisruptionBudget
metadata:
  name: my-app-pdb
  namespace: default
spec:
  minAvailable: 2 # Always keep at least 2 pods running
  selector:
    matchLabels:
      app: my-app
```

The descheduler respects PDBs and won't evict pods that would violate them.

### 4. Combine with Cluster Autoscaler

**Recommended setup:**

1. **Cluster Autoscaler**: Handles node scaling (add/remove nodes)
2. **Descheduler**: Handles pod distribution across nodes

**Timeline example:**

```
T+0m: Load increases, pods pending
T+1m: Cluster Autoscaler adds 3 new nodes
T+2m: New nodes join, scheduler places pending pods (but only on new nodes)
T+3m: Old nodes still heavily loaded, new nodes lightly loaded
T+4m: Descheduler runs, evicts pods from old nodes
T+5m: Scheduler redistributes pods evenly across all nodes
```

### 5. Monitor for Eviction Loops

**Watch for pods being repeatedly evicted:**

```bash
kubectl get events -n default --sort-by='.lastTimestamp' --kubeconfig output/kubeconfig | grep Evicted
```

**Causes:**

- Thresholds too aggressive
- Insufficient node capacity
- Pod affinity rules conflicting with descheduler

**Solutions:**

- Increase threshold values
- Add more node capacity
- Adjust or remove conflicting policies

### 6. Exclude Specific Pods

Use annotations to prevent pods from being evicted:

```yaml
apiVersion: v1
kind: Pod
metadata:
  name: critical-app
  annotations:
    descheduler.alpha.kubernetes.io/evict: "false"
spec:
  containers:
    - name: app
      image: nginx
```

## Troubleshooting

### Descheduler Not Running

**Check CronJob status:**

```bash
kubectl get cronjob -n kube-system descheduler --kubeconfig output/kubeconfig
```

**Check for suspended CronJob:**

```bash
# Ensure SUSPEND is False
kubectl get cronjob -n kube-system descheduler -o yaml --kubeconfig output/kubeconfig | grep suspend
```

**Check for recent jobs:**

```bash
kubectl get jobs -n kube-system -l app.kubernetes.io/name=descheduler --kubeconfig output/kubeconfig
```

### No Pods Being Evicted

**Verify metrics-server is running:**

```bash
kubectl get pods -n kube-system -l k8s-app=metrics-server --kubeconfig output/kubeconfig
kubectl top nodes --kubeconfig output/kubeconfig  # Should return data
```

**Check node utilization:**

```bash
# Nodes may not be over/under threshold
kubectl top nodes --kubeconfig output/kubeconfig
```

**Check descheduler logs:**

```bash
kubectl logs -n kube-system -l app.kubernetes.io/name=descheduler --tail=100 --kubeconfig output/kubeconfig
```

### Too Many Pods Being Evicted

**Reduce aggressiveness:**

```bash
# Increase thresholds (less aggressive)
helm upgrade descheduler descheduler/descheduler \
  --namespace kube-system \
  --set deschedulerPolicy.strategies.LowNodeUtilization.params.nodeResourceUtilizationThresholds.thresholds.cpu=30 \
  --set deschedulerPolicy.strategies.LowNodeUtilization.params.nodeResourceUtilizationThresholds.targetThresholds.cpu=70 \
  --reuse-values \
  --kubeconfig output/kubeconfig

# Reduce frequency
helm upgrade descheduler descheduler/descheduler \
  --namespace kube-system \
  --set schedule="*/5 * * * *" \
  --reuse-values \
  --kubeconfig output/kubeconfig
```

### Pods Stuck in Pending After Eviction

**Check node capacity:**

```bash
kubectl describe nodes --kubeconfig output/kubeconfig | grep -A 5 "Allocated resources"
```

**Verify pods have valid node selectors:**

```bash
kubectl get pod <pod-name> -o yaml --kubeconfig output/kubeconfig | grep -A 10 nodeSelector
```

**Check for taints preventing scheduling:**

```bash
kubectl describe nodes --kubeconfig output/kubeconfig | grep Taints
```

## Uninstallation

```bash
# Remove descheduler
helm uninstall descheduler --namespace kube-system --kubeconfig output/kubeconfig

# Clean up any leftover resources
kubectl delete configmap descheduler -n kube-system --kubeconfig output/kubeconfig
```

## References

- [Kubernetes Descheduler Documentation](https://github.com/kubernetes-sigs/descheduler)
- [Descheduler Policy Reference](https://github.com/kubernetes-sigs/descheduler/blob/master/README.md#policy-and-strategies)
- [Helm Chart Documentation](https://github.com/kubernetes-sigs/descheduler/tree/master/charts/descheduler)
- [PodDisruptionBudget Best Practices](https://kubernetes.io/docs/tasks/run-application/configure-pdb/)
