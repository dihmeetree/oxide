# Troubleshooting Guide

Common issues and solutions when using Oxide.

## Table of Contents

- [Cluster Creation Issues](#cluster-creation-issues)
- [Node Issues](#node-issues)
- [Networking Issues](#networking-issues)
- [Scaling Issues](#scaling-issues)
- [API Access Issues](#api-access-issues)
- [Performance Issues](#performance-issues)

## Cluster Creation Issues

### "Insufficient Resources" Error

**Symptom:**

```
Error: Server creation failed: insufficient resources
```

**Solutions:**

1. Try different server type
2. Try different Hetzner location
3. Contact Hetzner support to check capacity
4. Check account limits in Hetzner Console

### "Snapshot Not Found" Error

**Symptom:**

```
Error: Snapshot ID 123456789 not found
```

**Solutions:**

1. Verify snapshot ID in Hetzner Console
2. Ensure snapshot exists in same project
3. Create Talos snapshot (see [README.md](../README.md#1-create-talos-snapshot))

### Bootstrap Timeout

**Symptom:**

```
Timeout waiting for Kubernetes API to become available
```

**Solutions:**

1. Check control plane node is running (Hetzner Console)
2. Verify firewall allows your IP:
   ```bash
   curl https://api.ipify.org  # Check your current IP
   ```
3. Check Talos logs:
   ```bash
   talosctl --talosconfig ./output/talosconfig \
     --nodes <control-plane-ip> logs
   ```
4. Increase timeout and retry

### Cilium Installation Fails

**Symptom:**

```
Error: Cilium installation failed
```

**Solutions:**

1. Check helm is installed: `helm version`
2. Check kubectl is working: `kubectl get nodes`
3. Check Cilium pods:
   ```bash
   kubectl get pods -n kube-system -l k8s-app=cilium
   kubectl logs -n kube-system -l k8s-app=cilium
   ```
4. Verify network connectivity between nodes

## Node Issues

### Node Stuck in "NotReady"

**Symptom:**

```bash
kubectl get nodes
NAME       STATUS     ROLES    AGE
worker-1   NotReady   <none>   5m
```

**Solutions:**

1. Check node conditions:
   ```bash
   kubectl describe node worker-1
   ```
2. Check Cilium pod on node:
   ```bash
   kubectl get pods -n kube-system -o wide | grep worker-1
   ```
3. Check Talos status:
   ```bash
   talosctl --talosconfig ./output/talosconfig \
     --nodes <node-ip> health
   ```

### Node Not Joining Cluster

**Symptom:**

- Server created but doesn't appear in `kubectl get nodes`

**Solutions:**

1. Wait 2-3 minutes (initial join takes time)
2. Check Talos logs:
   ```bash
   talosctl --nodes <node-ip> logs
   ```
3. Verify network connectivity:
   ```bash
   ping <node-private-ip>
   ```
4. Check private network attachment (Hetzner Console)

### etcd Issues

**Symptom:**

```
kubectl get nodes
Unable to connect to the server
```

**Solutions:**

1. Check etcd status:
   ```bash
   talosctl --nodes <control-plane-ip> service etcd status
   ```
2. Check etcd members:
   ```bash
   talosctl --nodes <control-plane-ip> etcd members
   ```
3. If quorum lost, may need to rebuild cluster
4. Always maintain odd number of control planes (1, 3, 5)

## Networking Issues

### Pods Can't Reach Each Other Across Nodes

**Symptom:**

- Pods on same node can communicate
- Pods on different nodes cannot communicate

**Solutions:**

1. Check Cilium health:

   ```bash
   kubectl exec -n kube-system <cilium-pod> -- cilium-health status
   ```

   Expected: "X/X reachable"

2. Verify VXLAN tunnel mode:

   ```bash
   kubectl get configmap cilium-config -n kube-system -o yaml | grep routing-mode
   ```

   Should show: `routing-mode: tunnel`

3. Check Cilium logs for routing errors:

   ```bash
   kubectl logs -n kube-system -l k8s-app=cilium | grep -i error
   ```

4. Verify private network connectivity:
   ```bash
   # From any node
   ping <other-node-private-ip>
   ```

### LoadBalancer Not Working

**Symptom:**

- LoadBalancer service created but not accessible

**Solutions:**

1. Check service has EXTERNAL-IPs:

   ```bash
   kubectl get svc
   ```

   Should show worker IPs as EXTERNAL-IP

2. Test from each worker IP:

   ```bash
   curl http://<worker-ip>/
   ```

3. Check firewall allows port 80 (Hetzner Console)

4. Check backend pods are running:

   ```bash
   kubectl get pods -l app=<your-app>
   ```

5. Check Cilium LoadBalancer status:
   ```bash
   kubectl exec -n kube-system <cilium-pod> -- cilium service list
   ```

### DNS Resolution Fails

**Symptom:**

```bash
kubectl run -it --rm debug --image=busybox --restart=Never -- nslookup kubernetes
;; connection timed out; no servers could be reached
```

**Solutions:**

1. Check CoreDNS pods:

   ```bash
   kubectl get pods -n kube-system -l k8s-app=kube-dns
   ```

2. Check CoreDNS logs:

   ```bash
   kubectl logs -n kube-system -l k8s-app=kube-dns
   ```

3. Verify service exists:
   ```bash
   kubectl get svc -n kube-system kube-dns
   ```

## Scaling Issues

### Scale Up: Nodes Not Becoming Ready

**Symptom:**

- New nodes created but stuck in NotReady

**Solutions:**
See [Node Stuck in "NotReady"](#node-stuck-in-notready) above

### Scale Down: "Cannot connect to Talos API"

**Symptom:**

```
Error: Cannot connect to Talos API on worker-3 (10.0.1.6)
```

**Solutions:**

1. Check your current IP:

   ```bash
   curl https://api.ipify.org
   ```

2. Update firewall in Hetzner Console if IP changed

3. Check node is still running (Hetzner Console)

4. Verify network connectivity:
   ```bash
   ping <node-ip>
   ```

### Scale Down: Reset Timeout

**Symptom:**

```
Timeout waiting for node to be cordoned and NotReady
```

**Solutions:**

1. Check PodDisruptionBudgets:

   ```bash
   kubectl get pdb -A
   ```

   May be preventing pod eviction

2. Manually drain node:

   ```bash
   kubectl drain <node-name> --ignore-daemonsets --delete-emptydir-data --force
   ```

3. Force delete if necessary (last resort):
   ```bash
   kubectl delete node <node-name> --force --grace-period=0
   ```

## API Access Issues

### "Connection Refused" to Kubernetes API

**Symptom:**

```bash
kubectl get nodes
Unable to connect to the server: dial tcp <ip>:6443: connect: connection refused
```

**Solutions:**

1. Check your IP hasn't changed:

   ```bash
   curl https://api.ipify.org
   ```

2. Update firewall rules (Hetzner Console)

3. Verify control plane is running (Hetzner Console)

4. Check kubeconfig points to correct IP:
   ```bash
   kubectl config view
   ```

### "Connection Refused" to Talos API

**Symptom:**

```bash
talosctl health
error: rpc error: code = Unavailable desc = connection error
```

**Solutions:**

1. Same as Kubernetes API issues above (firewall/IP)

2. Verify port 50000 is allowed in firewall

3. Check node is booted and running Talos

### Certificate Errors

**Symptom:**

```
x509: certificate is valid for <other-ips>, not <your-ip>
```

**Solutions:**

1. Regenerate kubeconfig:

   ```bash
   talosctl --talosconfig ./output/talosconfig kubeconfig ./output/
   ```

2. Update KUBECONFIG environment variable:
   ```bash
   export KUBECONFIG=./output/kubeconfig
   ```

## Performance Issues

### High CPU Usage on Nodes

**Solutions:**

1. Check pod resource usage:

   ```bash
   kubectl top pods -A
   ```

2. Check node resource usage:

   ```bash
   kubectl top nodes
   ```

3. Scale up worker nodes if needed:

   ```bash
   oxide scale worker --count <higher-number>
   ```

4. Add resource limits to pods:
   ```yaml
   resources:
     limits:
       cpu: "1"
       memory: "512Mi"
     requests:
       cpu: "100m"
       memory: "128Mi"
   ```

### High Memory Usage

**Solutions:**

1. Identify memory-hungry pods:

   ```bash
   kubectl top pods -A --sort-by=memory
   ```

2. Check for memory leaks in applications

3. Add memory limits to prevent OOM:

   ```yaml
   resources:
     limits:
       memory: "1Gi"
   ```

4. Scale to larger server types:
   ```yaml
   # In cluster.yaml
   workers:
     - server_type: cpx41 # 8 vCPU, 16GB RAM
   ```

### Slow Network Performance

**Solutions:**

1. Check Cilium health:

   ```bash
   kubectl exec -n kube-system <cilium-pod> -- cilium status
   ```

2. Verify VXLAN is working (expected small overhead)

3. Check network bandwidth (Hetzner limits)

4. Consider CDN for static content (Cloudflare)

5. Use nodeAffinity to keep traffic local when possible

## Debugging Commands

### Useful kubectl Commands

```bash
# Get all resources
kubectl get all -A

# Describe resource with events
kubectl describe <resource-type> <name>

# Get logs
kubectl logs <pod-name> -n <namespace>
kubectl logs -f <pod-name>  # Follow logs
kubectl logs <pod-name> --previous  # Previous container logs

# Execute commands in pod
kubectl exec -it <pod-name> -- /bin/sh

# Port forward
kubectl port-forward <pod-name> 8080:80

# Get events
kubectl get events --sort-by='.lastTimestamp'
kubectl get events -A  # All namespaces

# Resource usage
kubectl top nodes
kubectl top pods -A
```

### Useful talosctl Commands

```bash
# Health check
talosctl --talosconfig ./output/talosconfig \
  --nodes <node-ip> health

# Get logs
talosctl --nodes <node-ip> logs
talosctl --nodes <node-ip> logs --follow
talosctl --nodes <node-ip> logs -k  # Kernel logs

# Service status
talosctl --nodes <node-ip> services
talosctl --nodes <node-ip> service kubelet status

# etcd operations
talosctl --nodes <control-plane-ip> etcd members
talosctl --nodes <control-plane-ip> etcd status

# System information
talosctl --nodes <node-ip> version
talosctl --nodes <node-ip> get members
```

### Useful Cilium Commands

```bash
# Check Cilium status
kubectl exec -n kube-system <cilium-pod> -- cilium status

# Check network health
kubectl exec -n kube-system <cilium-pod> -- cilium-health status

# List endpoints
kubectl exec -n kube-system <cilium-pod> -- cilium endpoint list

# List services
kubectl exec -n kube-system <cilium-pod> -- cilium service list

# Check BPF maps
kubectl exec -n kube-system <cilium-pod> -- cilium bpf lb list
```

## Getting Help

If you're still stuck:

1. Check [Oxide Issues](https://github.com/dihmeetree/oxide/issues)
2. Review [Talos Documentation](https://www.talos.dev/latest/)
3. Check [Cilium Documentation](https://docs.cilium.io/)
4. Check [Hetzner Status Page](https://status.hetzner.com/)
5. Open a new issue with:
   - Oxide version
   - Cluster configuration
   - Error messages
   - Relevant logs

## References

- [Talos Troubleshooting](https://www.talos.dev/latest/learn-more/troubleshooting/)
- [Cilium Troubleshooting](https://docs.cilium.io/en/stable/operations/troubleshooting/)
- [Kubernetes Troubleshooting](https://kubernetes.io/docs/tasks/debug/)
- [Hetzner Cloud Status](https://status.hetzner.com/)
