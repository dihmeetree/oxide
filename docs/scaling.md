# Cluster Scaling Operations

This document explains how to scale your Kubernetes cluster up and down, including detailed workflows, best practices, and troubleshooting.

## Overview

Oxide supports dynamic scaling of both worker and control plane nodes:

```bash
# Scale workers to 5 nodes
oxide scale worker --count 5

# Scale control plane to 3 nodes (HA)
oxide scale control-plane --count 3

# Scale specific node pool
oxide scale worker --count 10 --pool worker-large
```

## Scaling Up (Adding Nodes)

### Scale Up Workflow

```
oxide scale worker --count 5
    ↓
1. Read existing cluster configuration
    ↓
2. Calculate nodes to add (target - current)
    ↓
3. For each new node:
   a. Read Talos config from output/ directory
   b. Create Hetzner server with user_data (Talos config)
   c. Attach to private network
   d. Apply firewall rules
    ↓
4. Wait for servers to boot (~1-2 minutes)
    ↓
5. Nodes automatically join cluster (via Talos config)
    ↓
6. Wait for each new node to become Ready in Kubernetes
    ↓
7. Done! Nodes are ready for workloads
```

### Scale Up Example

**Current state:**
```bash
kubectl get nodes
NAME                READY   STATUS   ROLES    AGE
control-plane-1     Ready   control-plane    5d
worker-1            Ready   <none>           5d
worker-2            Ready   <none>           5d
```

**Scale up:**
```bash
oxide scale worker --count 5
```

**Process output:**
```
Scaling up: Current count = 2, Target count = 5
Creating 3 new worker nodes...

Creating worker-3...
✓ Server created: worker-3 (10.0.1.6)

Creating worker-4...
✓ Server created: worker-4 (10.0.1.7)

Creating worker-5...
✓ Server created: worker-5 (10.0.1.8)

Waiting for new nodes to become Ready...
✓ worker-3 is Ready
✓ worker-4 is Ready
✓ worker-5 is Ready

Scale up completed! 3 nodes added.
```

**Result:**
```bash
kubectl get nodes
NAME                READY   STATUS   ROLES    AGE
control-plane-1     Ready   control-plane    5d
worker-1            Ready   <none>           5d
worker-2            Ready   <none>           5d
worker-3            Ready   <none>           30s
worker-4            Ready   <none>           30s
worker-5            Ready   <none>           30s
```

### Scale Up Configuration

**Where configs come from:**

```
output/
  ├── controlplane.yaml  # Used for new control plane nodes
  └── worker.yaml        # Used for new worker nodes
```

**Important:** Scaling requires existing configuration files. If they don't exist, the operation fails:

```
Error: Configuration file not found: output/worker.yaml
The cluster must already exist to perform scaling operations.
```

### Node Naming

New nodes follow the naming convention:

```
{cluster_name}-{pool_name}-{index}

Examples:
  my-cluster-worker-1
  my-cluster-worker-2
  my-cluster-control-plane-1
```

Indices start at 1 and increment sequentially.

### Automatic Configuration

New nodes automatically:
- ✅ Join the Kubernetes cluster (via Talos config)
- ✅ Get assigned to private network
- ✅ Receive firewall protection
- ✅ Get pod CIDR allocated (by Kubernetes)
- ✅ Start Cilium agent
- ✅ Become "Ready" for pod scheduling

No manual intervention required!

## Scaling Down (Removing Nodes)

### Scale Down Workflow

```
oxide scale worker --count 2
    ↓
1. Calculate nodes to remove (current - target)
    ↓
2. Select nodes to remove (highest index first)
    ↓
3. For each node to remove:

   Step A: Pre-check Talos API connectivity
   ├─ Try to connect to node's Talos API
   ├─ If fails: Error with firewall troubleshooting tips
   └─ If succeeds: Continue
    ↓
   Step B: Graceful reset via Talos
   ├─ talosctl reset --graceful --wait
   ├─ Talos cordons node (SchedulingDisabled)
   ├─ Talos drains pods to other nodes
   ├─ If control plane: Leaves etcd cluster
   ├─ Securely wipes disks
   └─ Powers down node
    ↓
   Step C: Wait for cordoned status
   ├─ Poll node status
   └─ Wait for NotReady,SchedulingDisabled
    ↓
   Step D: Delete from Kubernetes
   ├─ kubectl delete node <node-name>
   └─ Remove from cluster
    ↓
   Step E: Delete Hetzner server
   └─ Permanently delete infrastructure
    ↓
4. Done! Nodes removed gracefully
```

### Scale Down Example

**Current state:**
```bash
kubectl get nodes
NAME                READY   STATUS   ROLES    AGE
control-plane-1     Ready   control-plane    5d
worker-1            Ready   <none>           5d
worker-2            Ready   <none>           5d
worker-3            Ready   <none>           2h
worker-4            Ready   <none>           2h
worker-5            Ready   <none>           2h
```

**Scale down:**
```bash
oxide scale worker --count 2
```

**Process output:**
```
Scaling down: Current count = 5, Target count = 2
Removing 3 worker nodes (newest first)...

Removing worker-5 (10.0.1.8)...
✓ Pre-check: Talos API accessible
✓ Graceful reset initiated
✓ Node cordoned and drained
✓ Deleted from Kubernetes
✓ Server deleted from Hetzner

Removing worker-4 (10.0.1.7)...
✓ Pre-check: Talos API accessible
✓ Graceful reset initiated
✓ Node cordoned and drained
✓ Deleted from Kubernetes
✓ Server deleted from Hetzner

Removing worker-3 (10.0.1.6)...
✓ Pre-check: Talos API accessible
✓ Graceful reset initiated
✓ Node cordoned and drained
✓ Deleted from Kubernetes
✓ Server deleted from Hetzner

Scale down completed! 3 nodes removed.
```

**Result:**
```bash
kubectl get nodes
NAME                READY   STATUS   ROLES    AGE
control-plane-1     Ready   control-plane    5d
worker-1            Ready   <none>           5d
worker-2            Ready   <none>           5d
```

### Node Selection Order

**Nodes are removed in reverse order (highest index first):**

```
Current: worker-1, worker-2, worker-3, worker-4, worker-5
Target: 2 nodes

Removed: worker-5, worker-4, worker-3
Kept:    worker-1, worker-2
```

**Rationale:**
- Removes newest nodes first
- Preserves original/stable nodes
- Predictable behavior

### Graceful Reset Details

The `talosctl reset --graceful --wait` command:

**What `--graceful` does:**
1. **Cordons node** - Prevents new pods from scheduling
2. **Drains pods** - Evicts all pods gracefully
   - Respects PodDisruptionBudgets
   - Waits for graceful shutdown (terminationGracePeriodSeconds)
3. **Leaves etcd** (control plane only) - Safely removes from etcd cluster
4. **Wipes disks** - Securely erases all data
5. **Powers down** - Shuts down the node

**What `--wait` does:**
- Blocks until reset completes
- Returns when node powers down
- May timeout if node takes too long (expected behavior)

**Expected behaviors:**
- ✅ Connection timeout when node powers down (normal)
- ✅ Node becomes NotReady,SchedulingDisabled (expected state)
- ❌ Connection error before reset starts (firewall issue)

### Status Checking

During scale down, Oxide checks for:

**NotReady,SchedulingDisabled status:**
```bash
kubectl get nodes
NAME       STATUS                        ROLES    AGE
worker-3   NotReady,SchedulingDisabled   <none>   5d
```

**Why this status matters:**
- `SchedulingDisabled` = Node is cordoned (step 1 of graceful reset)
- `NotReady` = Node is shutting down or powered off (step 4-5)

Oxide waits for both conditions before deleting the node from Kubernetes.

## Node Pools

### Using Multiple Pools

You can configure multiple node pools with different specifications:

```yaml
# In cluster.yaml
workers:
  - name: worker-small
    server_type: cpx21
    count: 3

  - name: worker-large
    server_type: cpx41
    count: 2
```

### Scaling Specific Pools

```bash
# Scale the "worker-small" pool
oxide scale worker --count 5 --pool worker-small

# Scale the "worker-large" pool
oxide scale worker --count 4 --pool worker-large
```

**Default behavior:** If `--pool` is not specified, the first worker pool is scaled.

### Pool-Based Workload Distribution

Use Kubernetes labels and nodeSelectors to target specific pools:

```yaml
# Pods will only run on worker-large nodes
apiVersion: v1
kind: Pod
metadata:
  name: high-memory-app
spec:
  nodeSelector:
    node.kubernetes.io/instance-type: cpx41
  containers:
    - name: app
      image: my-app
```

## Control Plane Scaling

### HA Recommendations

**Control plane count guidelines:**

| Count | Availability | etcd Quorum | Use Case |
|-------|--------------|-------------|----------|
| 1     | Single point of failure | 1/1 | Development only |
| 3     | Survives 1 failure | 2/3 | Production (recommended) |
| 5     | Survives 2 failures | 3/5 | Large production |
| 7     | Survives 3 failures | 4/7 | Critical infrastructure |

**Why odd numbers?**

etcd requires a majority (quorum) for consensus:

```
3 nodes: Quorum = 2, survives 1 failure
4 nodes: Quorum = 3, survives 1 failure  ← Same as 3! Waste of resources
5 nodes: Quorum = 3, survives 2 failures
```

**Always use odd numbers for control planes.**

### Scaling Control Planes

**Scale up to 3 (HA):**
```bash
oxide scale control-plane --count 3
```

**Process:**
1. Creates 2 new control plane nodes
2. Nodes automatically join etcd cluster
3. KubePrism updates to include new endpoints
4. Cluster is now highly available

**Scale down to 1 (dev mode):**
```bash
oxide scale control-plane --count 1
```

**WARNING:** Scaling control plane down:
- Removes nodes from etcd cluster
- Can cause cluster unavailability if quorum is lost
- Only do this in development environments

### etcd Health Monitoring

**Check etcd cluster status:**
```bash
talosctl --talosconfig ./output/talosconfig \
  --nodes <control-plane-ip> \
  service etcd status
```

**Check etcd members:**
```bash
talosctl --talosconfig ./output/talosconfig \
  --nodes <control-plane-ip> \
  etcd members
```

## Best Practices

### Scale Up Best Practices

1. **Gradual scaling** - Add nodes in small batches
   ```bash
   # Good: Scale from 3 to 5
   oxide scale worker --count 5

   # Avoid: Scale from 3 to 50 at once
   # Better: 3 → 10 → 20 → 50
   ```

2. **Wait for readiness** - Let new nodes stabilize before adding more
   ```bash
   oxide scale worker --count 5
   # Wait a few minutes, verify workloads are distributed
   oxide scale worker --count 10
   ```

3. **Monitor resource usage** - Scale based on actual demand
   ```bash
   kubectl top nodes
   kubectl top pods -A
   ```

4. **Consider pod distribution** - Ensure pods spread across nodes
   ```yaml
   # Use pod anti-affinity for redundancy
   affinity:
     podAntiAffinity:
       preferredDuringSchedulingIgnoredDuringExecution:
         - weight: 100
           podAffinityTerm:
             labelSelector:
               matchLabels:
                 app: my-app
             topologyKey: kubernetes.io/hostname
   ```

### Scale Down Best Practices

1. **Check workload capacity** - Ensure remaining nodes can handle load
   ```bash
   # Check resource requests before scaling down
   kubectl describe nodes | grep -A 5 "Allocated resources"
   ```

2. **Verify PodDisruptionBudgets** - Don't break availability requirements
   ```bash
   kubectl get pdb -A
   ```

3. **Drain manually first** (optional, for safety)
   ```bash
   kubectl drain worker-5 --ignore-daemonsets --delete-emptydir-data
   # Verify pods moved successfully
   oxide scale worker --count 4
   ```

4. **Scale down slowly** - Remove 1-2 nodes at a time
   ```bash
   # Good: 10 → 8 → 6 → 5
   # Avoid: 10 → 2 (sudden removal)
   ```

5. **Avoid control plane disruption** - Maintain quorum
   ```bash
   # Bad: Scale from 3 to 1 (loses quorum temporarily)
   # Better: Keep 3 or scale to 5
   ```

### Cost Optimization

**Auto-scaling pattern for dev environments:**

```bash
# Scale down during off-hours
crontab:
0 18 * * 1-5  oxide scale worker --count 1  # 6 PM weekdays
0 8  * * 1-5  oxide scale worker --count 3  # 8 AM weekdays
```

**Production:**
- Keep minimum viable capacity (don't over-optimize)
- Use Horizontal Pod Autoscaler (HPA) for application scaling
- Scale nodes when HPA can't schedule pods (pending pods)

## Troubleshooting

### Scale Up Issues

#### New Nodes Not Joining Cluster

**Symptom:**
```bash
kubectl get nodes
# New nodes don't appear after 5+ minutes
```

**Causes & Solutions:**

1. **Configuration file missing**
   ```
   Error: output/worker.yaml not found
   ```
   - Solution: Cluster must exist first, can't scale non-existent cluster

2. **Network issues**
   - Check Hetzner private network is attached
   - Verify nodes have private IPs assigned

3. **Cilium not ready**
   ```bash
   kubectl get pods -n kube-system -l k8s-app=cilium
   ```
   - Wait for Cilium to be healthy on existing nodes first

#### New Nodes Stuck in "NotReady"

**Symptom:**
```bash
kubectl get nodes
NAME       STATUS     ROLES    AGE
worker-3   NotReady   <none>   5m
```

**Causes & Solutions:**

1. **CNI not ready**
   ```bash
   kubectl get pods -n kube-system -l k8s-app=cilium -o wide
   # Check if Cilium pod is running on the new node
   ```

2. **Check node logs**
   ```bash
   kubectl describe node worker-3
   # Look at "Conditions" and "Events"
   ```

3. **Check Talos logs**
   ```bash
   talosctl --talosconfig ./output/talosconfig \
     --nodes <new-node-ip> \
     logs
   ```

### Scale Down Issues

#### "Cannot connect to Talos API" Error

**Symptom:**
```
Error: Cannot connect to Talos API on worker-3 (10.0.1.6)
Check firewall rules: Ensure port 50000 is accessible from your IP
```

**Causes & Solutions:**

1. **Your IP changed**
   - Check current IP: `curl https://api.ipify.org`
   - Update firewall in Hetzner Console

2. **Node already powered off**
   - Check Hetzner Console
   - If deleted externally, update cluster.yaml manually

3. **Network connectivity issue**
   - Verify node is reachable: `ping <node-ip>`
   - Check Hetzner status page

#### Node Reset Timeout

**Symptom:**
```
Timeout waiting for node worker-3 to be cordoned and NotReady
```

**Causes & Solutions:**

1. **Pods not draining** (PodDisruptionBudget too restrictive)
   ```bash
   kubectl get pods -o wide | grep worker-3
   # Check which pods are still running

   kubectl get pdb -A
   # Check PodDisruptionBudgets
   ```

2. **Node frozen/unresponsive**
   - Hard reboot via Hetzner Console
   - Or forcefully delete:
     ```bash
     kubectl delete node worker-3 --force --grace-period=0
     hcloud server delete worker-3
     ```

#### etcd Quorum Lost During Control Plane Scale Down

**Symptom:**
```
kubectl get nodes
Unable to connect to the server: dial tcp: connection refused
```

**Cause:** Scaled down control plane too aggressively, lost etcd quorum.

**Recovery:**

If you still have access to Talos API:
```bash
# Check remaining etcd members
talosctl --nodes <any-control-plane-ip> etcd members

# If quorum lost, rebuild cluster or restore from backup
```

**Prevention:** Always maintain odd number of control planes (1, 3, 5).

## Monitoring Scaling Operations

### Watch Node Status

```bash
# Watch nodes in real-time
watch kubectl get nodes

# Watch with more details
watch "kubectl get nodes -o wide"
```

### Check Scaling Progress

```bash
# View node events
kubectl get events --sort-by='.lastTimestamp' | grep Node

# Check pod distribution
kubectl get pods -A -o wide | grep worker-
```

### Verify Resource Distribution

```bash
# Check resource allocation per node
kubectl describe nodes | grep -A 5 "Allocated resources"

# View pod count per node
kubectl get pods -A -o wide | awk '{print $7}' | sort | uniq -c
```

## Advanced Scaling Scenarios

### Blue-Green Node Updates

```bash
# Add new nodes with updated configuration
oxide scale worker --count 6  # Original 3 + new 3

# Cordon old nodes
kubectl cordon worker-1 worker-2 worker-3

# Drain old nodes
kubectl drain worker-1 --ignore-daemonsets --delete-emptydir-data

# Remove old nodes
oxide scale worker --count 3 --pool new-workers
```

### Spot/Preemptible Node Simulation

While Hetzner doesn't have spot instances, you can simulate similar behavior:

```yaml
# Create two pools
workers:
  - name: worker-stable
    server_type: cpx31
    count: 2           # Always-on baseline

  - name: worker-burst
    server_type: cpx21
    count: 0           # Scale up during high load
```

```bash
# High load: scale up burst pool
oxide scale worker --count 5 --pool worker-burst

# Low load: scale down burst pool
oxide scale worker --count 0 --pool worker-burst
```

## References

- [Talos Node Reset Documentation](https://www.talos.dev/latest/reference/cli/#talosctl-reset)
- [Kubernetes Node Management](https://kubernetes.io/docs/concepts/architecture/nodes/)
- [etcd Administration](https://etcd.io/docs/latest/op-guide/runtime-configuration/)
- [PodDisruptionBudgets](https://kubernetes.io/docs/tasks/run-application/configure-pdb/)
