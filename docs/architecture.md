# Architecture Overview

This document provides a comprehensive overview of Oxide's architecture, including system design, component interactions, and data flow.

## System Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                     Oxide CLI                                │
│                                                               │
│  Commands: create, destroy, scale, status, init             │
└────────────┬────────────────────────────────────────────────┘
             │
             ├─────────────────┬─────────────────┬─────────────┐
             ↓                 ↓                 ↓             ↓
    ┌────────────────┐ ┌──────────────┐ ┌──────────────┐ ┌────────────┐
    │ Hetzner Cloud  │ │   talosctl   │ │   kubectl    │ │    helm    │
    │      API       │ │     CLI      │ │     CLI      │ │    CLI     │
    └────────┬───────┘ └──────┬───────┘ └──────┬───────┘ └─────┬──────┘
             │                │                │               │
             ↓                ↓                ↓               ↓
    ┌────────────────────────────────────────────────────────────────┐
    │              Hetzner Cloud Infrastructure                      │
    │                                                                 │
    │  ┌──────────────────────────────────────────────────────────┐ │
    │  │            Private Network (10.0.0.0/16)                 │ │
    │  │                                                           │ │
    │  │   ┌───────────────────┐      ┌───────────────────┐     │ │
    │  │   │  Control Plane    │      │     Workers        │     │ │
    │  │   │   - Talos OS      │      │    - Talos OS     │     │ │
    │  │   │   - etcd          │      │    - Kubelet      │     │ │
    │  │   │   - API Server    │      │    - Containerd   │     │ │
    │  │   │   - Cilium        │      │    - Cilium       │     │ │
    │  │   └───────────────────┘      └───────────────────┘     │ │
    │  │                                                           │ │
    │  └──────────────────────────────────────────────────────────┘ │
    │                                                                 │
    │  Firewall: Restricts access to Talos/K8s APIs (your IP only)  │
    └─────────────────────────────────────────────────────────────────┘
```

## Component Architecture

### Oxide CLI

The main entry point, implemented as a Rust binary with multiple modules:

```
oxide/
├── src/
│   ├── main.rs              # CLI entry point, argument parsing
│   ├── config/              # Configuration management
│   │   └── mod.rs           # YAML config parsing, validation
│   ├── hcloud/              # Hetzner Cloud API integration
│   │   ├── client.rs        # HTTP client for Hetzner API
│   │   ├── server.rs        # Server creation/deletion
│   │   ├── network.rs       # Private network management
│   │   ├── firewall.rs      # Firewall rule configuration
│   │   ├── ssh_key.rs       # SSH key management
│   │   └── models.rs        # API request/response types
│   ├── talos/               # Talos Linux operations
│   │   ├── client.rs        # Talosctl CLI wrapper
│   │   └── config.rs        # Talos config generation
│   ├── cilium/              # Cilium CNI management
│   │   └── mod.rs           # Helm-based installation
│   └── k8s/                 # Kubernetes operations
│       ├── client.rs        # kubectl prerequisite checks
│       ├── nodes.rs         # Node lifecycle management
│       └── resources.rs     # Manifest application
└── docs/                    # Documentation
```

### Module Responsibilities

#### `hcloud` Module

**Purpose:** Hetzner Cloud infrastructure provisioning

**Key Components:**

- `client.rs` - HTTP client, API token management
- `server.rs` - Create/delete servers, manage server lifecycle
- `network.rs` - Create private networks, subnets, attach servers
- `firewall.rs` - Configure firewall rules, manage IP allowlists
- `ssh_key.rs` - Generate/upload SSH keys
- `models.rs` - Type-safe API models (servers, networks, firewalls)

**External Dependencies:**

- Hetzner Cloud API (HTTPS REST API)
- Environment variable: `HCLOUD_TOKEN`

#### `talos` Module

**Purpose:** Talos Linux configuration and management

**Key Components:**

- `client.rs` - Wrapper around `talosctl` CLI
  - Bootstrap cluster
  - Reset nodes
  - Health checks
- `config.rs` - Generate Talos machine configs
  - Control plane configuration
  - Worker configuration
  - Secrets generation

**External Dependencies:**

- `talosctl` CLI (must be in PATH)
- Generated files: `talosconfig`, `secrets.yaml`

#### `cilium` Module

**Purpose:** Cilium CNI installation and configuration

**Key Components:**

- `mod.rs` - Helm-based installation
  - Gateway API CRD installation
  - Helm repository management
  - Cilium chart installation with custom values
  - Health status checking

**External Dependencies:**

- `helm` CLI (must be in PATH)
- `kubectl` CLI (for health checks)

**Configuration:**

- VXLAN tunnel mode (for Hetzner compatibility)
- NodeIPAM LoadBalancer
- KubePrism integration
- Talos-specific security contexts

#### `k8s` Module

**Purpose:** Kubernetes operations (kubectl wrappers)

**Key Components:**

- `client.rs` - kubectl prerequisite checks
- `nodes.rs` - Node lifecycle operations
  - Wait for node ready
  - Cordon/drain nodes
  - Delete nodes
  - Check node status
- `resources.rs` - Generic resource management
  - Apply manifests
  - Resource creation

**External Dependencies:**

- `kubectl` CLI (must be in PATH)
- `kubeconfig` file

#### `config` Module

**Purpose:** Configuration file management

**Key Components:**

- `mod.rs` - Parse `cluster.yaml`
  - Validate configuration
  - Provide defaults
  - Type-safe configuration structs

**File Format:** YAML

```yaml
cluster_name: my-cluster
hcloud: { ... }
talos: { ... }
cilium: { ... }
control_planes: [...]
workers: [...]
```

## Data Flow

### Cluster Creation Flow

```
User: oxide create
    ↓
main.rs
    ↓
1. Parse cluster.yaml (config module)
    ↓
2. Detect public IP (hcloud::client)
    ↓
3. Create Hetzner resources (hcloud module)
   ├─ Create private network
   ├─ Create firewall (allow user's IP)
   ├─ Generate SSH key
   └─ Create servers (control plane + workers)
    ↓
4. Generate Talos configs (talos::config)
   ├─ Generate secrets.yaml
   ├─ Generate controlplane.yaml
   ├─ Generate worker.yaml
   └─ Generate talosconfig
    ↓
5. Apply configs to nodes (user_data during creation)
    ↓
6. Bootstrap first control plane (talos::client)
   └─ talosctl bootstrap
    ↓
7. Generate kubeconfig (talos::client)
   └─ talosctl kubeconfig
    ↓
8. Install Cilium (cilium module)
   ├─ Install Gateway API CRDs (kubectl)
   ├─ Add Helm repo
   ├─ Install Cilium chart
   └─ Wait for Cilium pods ready
    ↓
9. Wait for all nodes ready (k8s::nodes)
    ↓
10. Output success
    └─ Print kubeconfig location
```

### Scaling Up Flow

```
User: oxide scale worker --count 5
    ↓
main.rs::scale_up()
    ↓
1. Read existing cluster.yaml
    ↓
2. Query current server count (hcloud::server)
    ↓
3. Calculate nodes to add (target - current)
    ↓
4. For each new node:
   ├─ Read worker.yaml (talos config)
   ├─ Create server with user_data (hcloud::server)
   ├─ Attach to private network
   └─ Apply firewall
    ↓
5. Wait for nodes to become Ready (k8s::nodes)
   └─ Poll node status every 5 seconds (300s timeout)
    ↓
6. Output success
```

### Scaling Down Flow

```
User: oxide scale worker --count 2
    ↓
main.rs::scale_down()
    ↓
1. Read existing cluster.yaml
    ↓
2. Query current servers (hcloud::server)
    ↓
3. Calculate nodes to remove (current - target)
    ↓
4. Select nodes (highest index first)
    ↓
5. For each node to remove:
   ├─ Pre-check Talos API connectivity (talos::client)
   │  └─ Fail fast if firewall blocks access
   ├─ Graceful reset (talos::client)
   │  └─ talosctl reset --graceful --wait
   ├─ Wait for cordoned status (k8s::nodes)
   │  └─ Poll for NotReady,SchedulingDisabled
   ├─ Delete from Kubernetes (k8s::nodes)
   │  └─ kubectl delete node
   └─ Delete Hetzner server (hcloud::server)
    ↓
6. Output success
```

### Destroy Flow

```
User: oxide destroy
    ↓
main.rs::destroy()
    ↓
1. Query all cluster resources (hcloud)
   ├─ List servers (by cluster name tag)
   ├─ Get firewall ID
   ├─ Get network ID
   └─ Get SSH key ID
    ↓
2. Delete servers (hcloud::server)
   └─ Delete all servers in parallel
    ↓
3. Delete network (hcloud::network)
    ↓
4. Delete firewall (hcloud::firewall)
    ↓
5. Delete SSH key (hcloud::ssh_key)
    ↓
6. Output success
   └─ Keep output/ directory (configs, secrets)
```

## Security Architecture

### Defense in Depth

```
Layer 1: Network (Hetzner Firewall)
    ├─ Port 50000 (Talos API): Your IP only
    ├─ Port 6443 (K8s API): Your IP only
    └─ Port 80 (HTTP): Public (LoadBalancer)

Layer 2: API Authentication
    ├─ Talos API: mTLS (client certificates)
    └─ Kubernetes API: Token-based auth

Layer 3: Private Network Isolation
    └─ Inter-node communication: Private network only

Layer 4: Talos Immutability
    ├─ No SSH access
    ├─ No shell access
    └─ API-only management

Layer 5: Kubernetes RBAC
    └─ Role-based access control (configured by user)

Layer 6: Cilium Network Policies
    └─ Pod-to-pod security (configured by user)
```

### Secret Management

```
output/
├── secrets.yaml       # Cluster-wide secrets (CRITICAL)
├── talosconfig        # Talos API access (CRITICAL)
├── kubeconfig         # K8s API access (CRITICAL)
├── id_ed25519         # SSH private key (SENSITIVE)
├── controlplane.yaml  # Contains config, no secrets
└── worker.yaml        # Contains config, no secrets
```

**Security Best Practices:**

- ✅ Add `output/` to `.gitignore`
- ✅ Backup secrets to secure vault (1Password, Vault, etc.)
- ✅ Rotate credentials periodically
- ❌ Never commit secrets to version control
- ❌ Never share secrets via insecure channels

### Firewall Architecture

```
Internet
    ↓
Hetzner Cloud Firewall (Stateful)
    ↓
┌─────────────────────────────────────────┐
│  Inbound Rules (Applied to all servers) │
├─────────────────────────────────────────┤
│  Port 50000 (Talos API)                 │
│    Source: <your-ip>/32                 │
│    Action: Allow                         │
├─────────────────────────────────────────┤
│  Port 6443 (Kubernetes API)             │
│    Source: <your-ip>/32                 │
│    Action: Allow                         │
├─────────────────────────────────────────┤
│  Port 80 (HTTP LoadBalancer)            │
│    Source: 0.0.0.0/0                    │
│    Action: Allow                         │
├─────────────────────────────────────────┤
│  Private Network (10.0.0.0/16)          │
│    Not restricted by Hetzner firewall   │
│    (Internal traffic flows freely)      │
└─────────────────────────────────────────┘
```

**Note:** Hetzner Cloud firewalls don't restrict traffic within the private network. All node-to-node and pod-to-pod communication on 10.0.0.0/16 is unrestricted by the firewall.

## Networking Architecture

### Network Layers

```
Layer 1: Hetzner Private Network (10.0.0.0/16)
    ├─ Subnet: 10.0.1.0/24 (Node IPs)
    ├─ Gateway: 10.0.0.1 (L3 routing)
    └─ Connectivity: All nodes can reach each other via gateway

Layer 2: Kubernetes Pod Network (10.0.16.0/20)
    ├─ Managed by: Cilium CNI
    ├─ Mode: VXLAN tunnel (encapsulation)
    ├─ Per-node CIDR: /24 (e.g., 10.0.16.0/24, 10.0.18.0/24)
    └─ Cross-node routing: VXLAN over private network

Layer 3: Kubernetes Service Network (10.0.8.0/21)
    ├─ Managed by: Cilium (kube-proxy replacement)
    ├─ ClusterIP services: Internal only
    └─ LoadBalancer services: External IPs = Worker node IPs

Layer 4: LoadBalancer (NodeIPAM)
    ├─ Service type: LoadBalancer
    ├─ External IPs: Worker public IPs
    ├─ Load balancing: Cilium BPF (native acceleration)
    └─ Backend selection: All pods across all nodes
```

### Traffic Flow Examples

#### LoadBalancer Service (External → Pod)

```
Internet Client (198.51.100.50)
    ↓ HTTP request to worker-1 IP
Worker-1 Public IP (178.156.191.97:80)
    ↓ Firewall allows (port 80 public)
Worker-1 Cilium BPF Load Balancer
    ↓ Selects backend pod
    ├─ If pod on worker-1: Direct native BPF
    └─ If pod on worker-2: VXLAN tunnel
        ↓
    VXLAN encapsulation (UDP)
        ↓ Private network
    10.0.1.1 → 10.0.1.2 (via gateway)
        ↓
    Worker-2 VXLAN decapsulation
        ↓
    Pod on Worker-2 (10.0.19.38:80)
        ↓
    HTTP response (reverse path)
        ↓
    Internet Client
```

#### ClusterIP Service (Pod → Pod)

```
Pod A on Worker-1 (10.0.18.5)
    ↓ Request to service ClusterIP (10.0.8.100)
Cilium BPF Service Load Balancer
    ↓ Resolves to backend pod IP
Pod B on Worker-2 (10.0.19.20)
    ↓ VXLAN tunnel (cross-node)
10.0.1.1 → 10.0.1.2 (via gateway)
    ↓ Decapsulation
Pod B receives request
```

#### External Egress (Pod → Internet)

```
Pod on Worker-1 (10.0.18.5)
    ↓
Cilium BPF Masquerade
    ↓ SNAT to node IP
Worker-1 Private IP (10.0.1.1)
    ↓ NAT to public IP
Worker-1 Public IP (178.156.191.97)
    ↓
Internet
```

## High Availability

### Control Plane HA

```
┌──────────────────────────────────────────┐
│         Control Plane (3 nodes)          │
├──────────────────────────────────────────┤
│  ┌────────────┐  ┌────────────┐         │
│  │ etcd-1     │  │ etcd-2     │         │
│  │ (leader)   │  │ (follower) │  ...    │
│  └─────┬──────┘  └─────┬──────┘         │
│        └────────────────┴─── Raft       │
│             Consensus                    │
│        ┌─────────────────┐              │
│        │  Quorum: 2/3    │              │
│        │  Survives 1 failure            │
│        └─────────────────┘              │
└──────────────────────────────────────────┘

Client (kubelet, kubectl, etc.)
    ↓
KubePrism (localhost:7445)
    ↓ Load balances
control-plane-1:6443, control-plane-2:6443, control-plane-3:6443
```

**Benefits:**

- etcd: Survives 1 control plane failure (2/3 quorum)
- API access: Survives any control plane failure (via KubePrism)
- No single point of failure

### Worker HA

```
┌──────────────────────────────────────────┐
│            Workers (3+ nodes)             │
├──────────────────────────────────────────┤
│  Pods distributed across nodes           │
│                                           │
│  ┌────────────┐  ┌────────────┐         │
│  │ Worker-1   │  │ Worker-2   │  ...    │
│  │  - app-1   │  │  - app-2   │         │
│  │  - app-3   │  │  - app-4   │         │
│  └────────────┘  └────────────┘         │
│                                           │
│  PodAntiAffinity: Spread across nodes    │
│  PodDisruptionBudgets: Maintain quorum   │
└──────────────────────────────────────────┘
```

**Best Practices:**

- Use `PodAntiAffinity` to spread replicas
- Set `PodDisruptionBudgets` to maintain availability
- Run ≥2 replicas per deployment

## Performance Characteristics

### Scaling Limits

**Tested Configuration:**

- Control Planes: Up to 5 nodes
- Workers: Up to 100 nodes (theoretical, not tested at scale)
- Pods per node: 110 (Kubernetes default)
- Services: Thousands (Cilium BPF scales well)

**Bottlenecks:**

- etcd: Performance degrades >5 control planes
- Network: VXLAN overhead ~3-5% (acceptable)
- Hetzner API: Rate limited (requests/second)

### Resource Requirements

**Minimum (Development):**

- Control Plane: 2 vCPU, 4GB RAM (cx21)
- Worker: 2 vCPU, 4GB RAM (cx21)
- Total: ~€15/month

**Recommended (Production):**

- Control Plane: 3 vCPU, 4GB RAM (cpx21)
- Worker: 4 vCPU, 8GB RAM (cpx31)
- Total: ~€66/month (3+3 nodes)

**Large Production:**

- Control Plane: 4 vCPU, 8GB RAM (cpx31)
- Worker: 8 vCPU, 16GB RAM (cpx41)
- Total: Scales with node count

## Monitoring and Observability

### Built-in Health Checks

**Talos:**

```bash
talosctl health
```

**Kubernetes:**

```bash
kubectl get nodes
kubectl get pods -A
```

**Cilium:**

```bash
kubectl exec -n kube-system <cilium-pod> -- cilium status
kubectl exec -n kube-system <cilium-pod> -- cilium-health status
```

### Recommended Monitoring Stack

**Metrics:**

- Prometheus (scrape metrics from nodes/pods)
- Grafana (visualize metrics)

**Logging:**

- Loki (log aggregation)
- Promtail (log shipping)

**Tracing:**

- Hubble (Cilium observability)
- Jaeger (distributed tracing)

**Alerting:**

- Alertmanager (Prometheus alerts)
- PagerDuty/Slack integration

## Future Architecture Improvements

### Planned Enhancements

1. **Multi-Cloud Support**

   - AWS (EKS-compatible)
   - GCP (GKE-compatible)
   - DigitalOcean

2. **GitOps Integration**

   - Flux CD
   - ArgoCD
   - Declarative cluster management

3. **Cluster Upgrades**

   - Rolling Talos upgrades
   - Kubernetes version upgrades
   - Zero-downtime upgrades

4. **Backup/Restore**

   - etcd backup automation
   - Persistent volume snapshots
   - Disaster recovery procedures

5. **Advanced Networking**
   - BGP/Anycast (single LoadBalancer IP)
   - Multi-region clusters
   - Service mesh integration (Istio/Linkerd)

## References

- [Oxide Source Code](https://github.com/dihmeetree/oxide)
- [Talos Architecture](https://www.talos.dev/latest/learn-more/architecture/)
- [Cilium Architecture](https://docs.cilium.io/en/stable/overview/intro/)
- [Kubernetes Architecture](https://kubernetes.io/docs/concepts/architecture/)
- [Hetzner Cloud](https://docs.hetzner.cloud/)
