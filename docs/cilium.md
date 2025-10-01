# Cilium CNI Configuration

This document explains the Cilium CNI configuration used in Oxide, including Hetzner-specific networking considerations.

## Overview

Oxide uses Cilium as the Container Network Interface (CNI) provider with the following key features:

- **IPAM Mode**: Kubernetes (uses node pod CIDR allocations)
- **Routing Mode**: VXLAN tunnel mode
- **Load Balancer**: NodeIPAM with native BPF acceleration
- **Kube-proxy Replacement**: Full eBPF replacement of kube-proxy

## Routing Mode: VXLAN Tunnel vs Native

### Why VXLAN Tunnel Mode?

Oxide uses **VXLAN tunnel mode** instead of native routing due to Hetzner Cloud's private network topology.

#### Hetzner Private Network Architecture

Hetzner Cloud's private networks use **L3 routing through a gateway** rather than direct L2 connectivity:

```
Node A (10.0.1.1) ──┐
                    ├──> Gateway (10.0.0.1) ──┐
Node B (10.0.1.2) ──┘                         ├──> Private Network (10.0.0.0/16)
                                              │
Node C (10.0.1.3) ────────────────────────────┘
```

Nodes communicate through gateway `10.0.0.1`, not directly to each other.

#### Why Native Routing Fails on Hetzner

Cilium's native routing mode requires **direct L2 connectivity** between nodes. When enabled with `autoDirectNodeRoutes=true`, Cilium attempts to install direct routes like:

```
10.0.18.0/24 via 10.0.1.2  # Route to Node B's pod CIDR
```

However, since Node B (10.0.1.2) is only reachable via gateway 10.0.0.1, the kernel routing table shows:

```
10.0.1.2 via 10.0.0.1  # Must go through gateway
```

Cilium detects this and fails with:

```
route to destination 10.0.1.2 contains gateway 10.0.0.1, must be directly reachable
```

This is by design - native routing assumes same-segment L2 connectivity where nodes can ARP for each other directly.

#### VXLAN Tunnel Mode Solution

VXLAN encapsulates pod traffic in UDP packets, allowing it to traverse the gateway-routed private network:

```
Pod A (10.0.16.1) on Node A
    ↓
VXLAN encapsulation
    ↓
UDP packet from 10.0.1.1 to 10.0.1.2 (via gateway 10.0.0.1)
    ↓
VXLAN decapsulation on Node B
    ↓
Pod B (10.0.18.1) on Node B
```

### Performance Considerations

#### What Uses Native BPF (No Overhead)

- **LoadBalancer ingress traffic** - Uses `loadBalancer.acceleration=native`
- **Same-node pod access** - Direct delivery, no encapsulation
- **LoadBalancer → Pod on same node** - Pure native BPF path
- **Service load balancing** - Full eBPF kube-proxy replacement
- **NAT/Masquerading** - Uses `bpf.masquerade=true`
- **Policy enforcement** - eBPF-based network policies

#### What Uses VXLAN Encapsulation

- **Pod-to-pod traffic across different nodes** - VXLAN tunnel between nodes
- **LoadBalancer → Pod on different node** - VXLAN encapsulation for cross-node delivery

#### Traffic Path Examples

**Same-node traffic (no VXLAN):**
```
External traffic → worker-1 IP (178.156.191.97)
    ↓
BPF program on worker-1 (native, no encapsulation)
    ↓
Pod on worker-1 (10.0.18.x) - direct delivery ✅ Native BPF
```

**Cross-node traffic (uses VXLAN):**
```
External traffic → worker-1 IP (178.156.191.97)
    ↓
BPF program decides backend is on worker-2
    ↓
VXLAN encapsulation (UDP packet)
    ↓
worker-1 → worker-2 (via gateway)
    ↓
VXLAN decapsulation on worker-2
    ↓
Pod on worker-2 (10.0.19.x) ⚠️ VXLAN overhead
```

**Key Insight**: The tunnel mode setting only affects **cross-node pod traffic**. Same-node traffic always uses native BPF regardless of the routing mode configuration.

#### Performance Impact

- **VXLAN overhead**: ~50 bytes per packet (only for cross-node traffic)
- **Throughput impact**: 3-5% for large packets, negligible for most workloads
- **Latency impact**: Minimal (microseconds for encap/decap)
- **Same-node performance**: **No impact** - always native BPF
- **LoadBalancer performance**: **Minimal impact** - only cross-node backend selection uses VXLAN

For Hetzner's network topology, this is the correct trade-off. Native routing simply doesn't work without direct L2 connectivity.

## Configuration Settings

### Core Cilium Settings

```rust
// IPAM Mode
"ipam.mode=kubernetes"

// Kube-proxy replacement
"kubeProxyReplacement=true"

// Tunnel mode for pod traffic
"tunnelProtocol=vxlan"
"autoDirectNodeRoutes=false"  // Must be false with tunnel mode

// BPF optimizations
"bpf.masquerade=true"
```

### LoadBalancer Configuration

Oxide uses Cilium's **NodeIPAM** feature to automatically assign LoadBalancer IPs from worker node IPs:

```rust
"nodeIPAM.enabled=true"
"loadBalancer.acceleration=native"  // Native BPF for LoadBalancer
"defaultLBServiceIPAM=nodeipam"     // Use node IPs as LoadBalancer IPs
```

When you create a `LoadBalancer` service, Cilium automatically assigns all worker node IPs as EXTERNAL-IPs:

```yaml
apiVersion: v1
kind: Service
metadata:
  name: nginx-lb
spec:
  type: LoadBalancer
  selector:
    app: nginx
  ports:
    - port: 80
      targetPort: 80
```

Results in:

```
NAME       TYPE           EXTERNAL-IP
nginx-lb   LoadBalancer   178.156.188.143,178.156.203.237,5.161.58.170
```

All worker IPs become entry points with native BPF load balancing to backend pods across all nodes.

#### Load Distribution Strategy

**Single IP vs All IPs:**

You can send traffic to any single worker IP and Cilium will distribute it to pods across all nodes. However, **using all worker IPs is strongly recommended** for production:

**Single IP approach (works, but not optimal):**
```
All traffic → worker-1 IP (178.156.191.97)
    ↓
BPF load balances to pods on worker-1, worker-2, worker-3
    ↓
Cross-node traffic uses VXLAN (overhead)
Single NIC bandwidth limit on worker-1
```

**Limitations:**
- All ingress traffic limited to 1 node's NIC bandwidth (bottleneck)
- Single point of failure - if node goes down, service unavailable
- More VXLAN overhead due to cross-node traffic
- CPU load concentrated on one node

**All IPs approach (recommended):**
```
Traffic distributed → worker-1, worker-2, worker-3 IPs
    ↓
Each node handles ~33% of ingress traffic
    ↓
More same-node traffic (native BPF, no VXLAN)
3x aggregate NIC bandwidth capacity
```

**Benefits:**
- **3x aggregate bandwidth** - uses all node NICs in parallel
- **High availability** - survives individual node failures
- **Less VXLAN overhead** - better same-node locality (native BPF)
- **Better CPU distribution** - load spread across all nodes

**How to distribute traffic across all IPs:**

1. **DNS Round-Robin:**
   ```
   your-app.com A 178.156.188.143
   your-app.com A 178.156.191.97
   your-app.com A 178.156.203.237
   ```

2. **External Load Balancer (recommended):**
   - Use Cloudflare Load Balancing, AWS ALB, or similar
   - Configure all worker IPs as origin servers
   - Enable health checks per IP
   - Automatic failover on node failure

3. **BGP/Anycast (advanced):**
   - Advertise same IP from all nodes
   - Requires BGP support and network configuration

### Talos-Specific Settings

```rust
// KubePrism configuration for API server access
"k8sServiceHost=localhost"
"k8sServicePort=7445"

// Security context capabilities for Talos
"securityContext.capabilities.ciliumAgent={CHOWN,KILL,NET_ADMIN,NET_RAW,IPC_LOCK,SYS_ADMIN,SYS_RESOURCE,DAC_OVERRIDE,FOWNER,SETGID,SETUID}"
"securityContext.capabilities.cleanCiliumState={NET_ADMIN,SYS_ADMIN,SYS_RESOURCE}"

// cgroup v2 configuration
"cgroup.autoMount.enabled=false"
"cgroup.hostRoot=/sys/fs/cgroup"
```

### Optional Features

```rust
// Operator replicas (HA for multi-control-plane clusters)
operator_replicas = if control_plane_count > 1 { 2 } else { 1 }

// Hubble observability (optional)
"hubble.relay.enabled=true"
"hubble.ui.enabled=true"

// Gateway API support
"gatewayAPI.enabled=true"

// IPv6 (optional)
"ipv6.enabled=true"
```

## Network Architecture

### IP Address Allocation

- **Private Network CIDR**: `10.0.0.0/16` (Hetzner private network)
- **Node IPs**: `10.0.1.0/24` (allocated from private network)
- **Pod CIDR**: `10.0.16.0/20` (Kubernetes pod network)
- **Service CIDR**: `10.0.8.0/21` (Kubernetes services)

### Pod CIDR Assignment

Each node receives a `/24` subnet from the pod CIDR:

- Control Plane: `10.0.16.0/24` (254 pods)
- Worker 1: `10.0.18.0/24` (254 pods)
- Worker 2: `10.0.19.0/24` (254 pods)
- Worker 3: `10.0.17.0/24` (254 pods)

## Troubleshooting

### Check Cilium Health

```bash
kubectl exec -n kube-system <cilium-pod> -- cilium-health status
```

Expected output for healthy cluster:

```
Cluster health:   4/4 reachable
Name                                   IP              Node   Endpoints
  talos-cluster-control-plane          10.0.1.1        1/1    1/1
  talos-cluster-worker-1               10.0.1.2        1/1    1/1
  talos-cluster-worker-2               10.0.1.4        1/1    1/1
  talos-cluster-worker-3               10.0.1.3        1/1    1/1
```

### Check Cilium Status

```bash
kubectl exec -n kube-system <cilium-pod> -- cilium status --brief
```

Should return: `OK`

### Check Routing Configuration

```bash
kubectl get configmap cilium-config -n kube-system -o yaml | grep -E "routing-mode|tunnel-protocol|auto-direct"
```

Expected values:

```yaml
auto-direct-node-routes: "false"
routing-mode: tunnel
tunnel-protocol: vxlan
```

### Common Issues

#### "auto-direct-node-routes cannot be used with tunneling"

**Cause**: Conflicting configuration - `autoDirectNodeRoutes=true` with tunnel mode.

**Solution**: Set `autoDirectNodeRoutes=false` when using tunnel mode.

#### "route contains gateway, must be directly reachable"

**Cause**: Attempting to use native routing on Hetzner's gateway-routed private network.

**Solution**: Use VXLAN tunnel mode instead of native routing.

#### Pods on different nodes cannot communicate

**Cause**: Incorrect routing configuration or firewall blocking VXLAN.

**Solution**:

1. Verify tunnel mode is enabled
2. Check firewall allows UDP port 8472 (VXLAN) between nodes
3. Verify Cilium health shows all nodes reachable

## References

- [Cilium Documentation](https://docs.cilium.io/)
- [Cilium Routing Modes](https://docs.cilium.io/en/stable/network/concepts/routing/)
- [Talos + Cilium Guide](https://www.talos.dev/v1.8/kubernetes-guides/network/deploying-cilium/)
- [Hetzner Cloud Documentation](https://docs.hetzner.cloud/)
