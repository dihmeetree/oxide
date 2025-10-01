# Talos Linux Configuration

This document explains Talos Linux integration in Oxide, including configuration generation, security model, and operational details.

## What is Talos Linux?

[Talos Linux](https://www.talos.dev/) is a modern Linux distribution designed specifically for Kubernetes:

- **Immutable**: No SSH, no shell, no package manager - configured entirely via API
- **Minimal**: Only includes what's needed to run Kubernetes
- **Secure**: Hardened by default, minimal attack surface
- **API-driven**: All operations via gRPC API using `talosctl`

### Why Talos for Kubernetes?

**Security Benefits:**

- No SSH access = no SSH vulnerabilities
- Immutable filesystem = no runtime modifications
- API-only access = auditable, authenticated operations
- Minimal packages = reduced attack surface

**Operational Benefits:**

- Declarative configuration = infrastructure as code
- Atomic updates = reliable upgrades
- Fast boot times = quick recovery
- Predictable behavior = no configuration drift

## Talos Architecture

### Control Plane Node Components

```
┌─────────────────────────────────────────┐
│     Talos Control Plane Node            │
├─────────────────────────────────────────┤
│  Talos API (port 50000)                 │
│    ├─ apid (API server)                 │
│    ├─ machined (system manager)         │
│    └─ trustd (certificate authority)    │
├─────────────────────────────────────────┤
│  Kubernetes Control Plane               │
│    ├─ kube-apiserver                    │
│    ├─ kube-controller-manager           │
│    ├─ kube-scheduler                    │
│    └─ etcd (distributed key-value)      │
├─────────────────────────────────────────┤
│  Kubernetes Node Components             │
│    ├─ kubelet                           │
│    └─ containerd                        │
├─────────────────────────────────────────┤
│  Network (Cilium CNI)                   │
│    └─ cilium-agent                      │
└─────────────────────────────────────────┘
```

### Worker Node Components

```
┌─────────────────────────────────────────┐
│     Talos Worker Node                   │
├─────────────────────────────────────────┤
│  Talos API (port 50000)                 │
│    ├─ apid (API server)                 │
│    └─ machined (system manager)         │
├─────────────────────────────────────────┤
│  Kubernetes Node Components             │
│    ├─ kubelet                           │
│    └─ containerd                        │
├─────────────────────────────────────────┤
│  Network (Cilium CNI)                   │
│    └─ cilium-agent                      │
├─────────────────────────────────────────┤
│  Application Pods                       │
│    └─ (your workloads)                  │
└─────────────────────────────────────────┘
```

## Configuration Generation

Oxide automatically generates Talos configurations during cluster creation:

### Generated Files

After running `oxide create`, the following files are created in `output/`:

1. **`secrets.yaml`** - Cluster-wide secrets

   - Encryption keys
   - CA certificates
   - Bootstrap tokens
   - **CRITICAL**: Keep secure, never commit to version control

2. **`controlplane.yaml`** - Control plane node configuration

   - Kubernetes control plane settings
   - etcd configuration
   - Network settings for control plane

3. **`worker.yaml`** - Worker node configuration

   - Kubelet settings
   - Node-specific configuration

4. **`talosconfig`** - Talosctl client configuration
   - API endpoint
   - Client certificates
   - Used by `talosctl` CLI

### Configuration Process

```
oxide create
    ↓
1. Generate cluster secrets
    ↓
2. Create machine configs (controlplane.yaml, worker.yaml)
   - Sets Kubernetes version
   - Configures networking (pod/service CIDRs)
   - Sets up KubePrism
   - Disables default CNI (will use Cilium)
    ↓
3. Apply configs to nodes via Hetzner Cloud user_data
    ↓
4. Bootstrap first control plane node
    ↓
5. Wait for Kubernetes API to be ready
    ↓
6. Install Cilium CNI
```

## Key Configuration Settings

### Kubernetes Version

```yaml
# In cluster.yaml
talos:
  kubernetes_version: 1.30.0
```

This controls which Kubernetes version is installed on all nodes.

### Pod and Service CIDRs

```yaml
# In cluster.yaml (advanced section)
talos:
  pod_cidr: 10.0.16.0/20 # Pod IP addresses
  service_cidr: 10.0.8.0/21 # Service IP addresses
```

**Defaults:**

- Pod CIDR: `10.0.16.0/20` (4096 IP addresses)
- Service CIDR: `10.0.8.0/21` (2048 IP addresses)

**Important**: These must not overlap with your Hetzner private network CIDR.

### CNI Configuration

Oxide disables Talos's default CNI and installs Cilium instead:

```yaml
cluster:
  network:
    cni:
      name: none # Disable default CNI
```

Cilium is installed separately via Helm after the cluster is bootstrapped.

## KubePrism

KubePrism is Talos's built-in load balancer for the Kubernetes API server, running on **every node** at `localhost:7445`.

### What is KubePrism?

```
┌──────────────────────────────────────────┐
│         Any Node (Control Plane or Worker)│
│                                            │
│  Application/Kubelet                       │
│       ↓                                    │
│  localhost:7445 (KubePrism)               │
│       ↓                                    │
│  Load balances to all control planes:     │
│    - 10.0.1.1:6443 (control-plane-1)     │
│    - 10.0.1.2:6443 (control-plane-2)     │
│    - 10.0.1.3:6443 (control-plane-3)     │
└──────────────────────────────────────────┘
```

### Benefits

- **High Availability**: Pods can reach Kubernetes API even if one control plane is down
- **Load Distribution**: Spreads API requests across all control planes
- **No External Dependencies**: Built into Talos, no extra infrastructure needed
- **Automatic Updates**: KubePrism automatically tracks control plane endpoints

### Cilium Integration

Cilium is configured to use KubePrism for API server access:

```yaml
cilium:
  k8sServiceHost: localhost
  k8sServicePort: 7445
```

This ensures Cilium agents can always reach the Kubernetes API, even during control plane maintenance.

## Talos API Operations

All Talos operations are performed via the `talosctl` CLI tool, which uses the generated `talosconfig` file.

### Common Operations

#### Check Node Health

```bash
talosctl --talosconfig ./output/talosconfig \
  --nodes <node-ip> \
  health
```

#### Get Cluster Info

```bash
talosctl --talosconfig ./output/talosconfig \
  --nodes <node-ip> \
  get members
```

#### View Node Logs

```bash
talosctl --talosconfig ./output/talosconfig \
  --nodes <node-ip> \
  logs
```

#### Reboot Node

```bash
talosctl --talosconfig ./output/talosconfig \
  --nodes <node-ip> \
  reboot
```

#### Upgrade Talos Version

```bash
talosctl --talosconfig ./output/talosconfig \
  --nodes <node-ip> \
  upgrade --image ghcr.io/siderolabs/installer:v1.8.0
```

### Node Reset (Used During Scale Down)

When scaling down, Oxide performs a graceful node reset:

```bash
talosctl --talosconfig ./output/talosconfig \
  --nodes <node-ip> \
  reset --graceful --wait
```

**What happens:**

1. Node is cordoned (no new pods scheduled)
2. Pods are drained (moved to other nodes)
3. If control plane: leaves etcd cluster
4. Disks are wiped securely
5. Node powers down

See [docs/scaling.md](scaling.md) for detailed scale-down workflow.

## Security Model

### API Access Control

Talos API uses mutual TLS (mTLS) for authentication:

```
talosctl (client)
    ↓ TLS handshake
    ├─ Client certificate (from talosconfig)
    └─ Server certificate (validates node identity)
    ↓
Talos API (apid on node)
    ↓ Authorization check
    └─ Allow/Deny based on certificate
```

**Security benefits:**

- No passwords = no password attacks
- Certificate-based = cryptographically secure
- Per-client certificates = auditable access
- Short-lived sessions = limited exposure window

### Firewall Protection

Oxide automatically configures Hetzner Cloud firewall to restrict Talos API access:

```yaml
# Only your IP can access Talos API
Talos API (port 50000):
  source: <your-public-ip>/32
  protocol: TCP
```

See [docs/hetzner.md](hetzner.md) for firewall details.

### Secrets Management

**Critical Files (Keep Secure):**

- `output/secrets.yaml` - Contains all cluster secrets
- `output/talosconfig` - Grants full Talos API access
- `output/kubeconfig` - Grants full Kubernetes API access

**Best Practices:**

- ✅ Store in secure secret management system (Vault, 1Password, etc.)
- ✅ Encrypt at rest
- ✅ Add `output/` to `.gitignore`
- ❌ Never commit to version control
- ❌ Never share via insecure channels

## Machine Config Customization

For advanced use cases, you can customize Talos machine configs before cluster creation.

### Generate Custom Configs

```bash
# Generate base configs
oxide create --dry-run

# Edit the generated configs
vi output/controlplane.yaml
vi output/worker.yaml

# Apply manually (advanced)
talosctl --talosconfig ./output/talosconfig \
  --nodes <node-ip> \
  apply-config --file output/controlplane.yaml
```

### Common Customizations

**Add custom kubelet args:**

```yaml
machine:
  kubelet:
    extraArgs:
      max-pods: "250"
```

**Add system extensions:**

```yaml
machine:
  install:
    extensions:
      - image: ghcr.io/siderolabs/intel-ucode:20240531
```

**Add custom mounts:**

```yaml
machine:
  disks:
    - device: /dev/sdb
      partitions:
        - mountpoint: /var/lib/extra
```

## Bootstrap Process

The bootstrap process initializes the first control plane node and forms the Kubernetes cluster.

### Bootstrap Sequence

```
First Control Plane Node:
    ↓
1. Talos boots from snapshot image
    ↓
2. Applies machine config (from user_data)
    ↓
3. Starts Talos services (apid, machined, trustd)
    ↓
4. oxide runs: talosctl bootstrap
    ↓
5. Talos initializes etcd cluster
    ↓
6. Starts Kubernetes control plane:
   - kube-apiserver
   - kube-controller-manager
   - kube-scheduler
    ↓
7. Kubernetes API becomes available
    ↓
8. oxide installs Cilium CNI
    ↓
9. Additional nodes join cluster automatically
```

### Bootstrap Command

```bash
talosctl --talosconfig ./output/talosconfig \
  --nodes <first-control-plane-ip> \
  bootstrap
```

**Important**: Only bootstrap the first control plane node. Additional control planes join automatically.

## Troubleshooting

### Common Issues

#### Node Not Responding to Talos API

**Symptom:**

```
talosctl health
error: rpc error: code = Unavailable desc = connection error
```

**Causes & Solutions:**

1. **Firewall blocking access**

   - Check your public IP hasn't changed
   - Verify Hetzner firewall rules allow your IP

2. **Node still booting**

   - Wait 2-3 minutes for initial boot
   - Check Hetzner console for boot progress

3. **Wrong node IP**
   - Verify IP in Hetzner Cloud console
   - Check `output/` files for correct IPs

#### etcd Cluster Issues

**Symptom:**

```
kubectl get nodes
Unable to connect to the server
```

**Causes & Solutions:**

1. **Control plane nodes not healthy**

   ```bash
   talosctl --nodes <control-plane-ip> service etcd status
   ```

2. **etcd quorum lost** (majority of control planes down)
   - Restore from backup or rebuild cluster
   - Maintain odd number of control planes (1, 3, 5)

#### CNI Not Installing

**Symptom:**

```
kubectl get nodes
NAME            STATUS     ROLES           AGE
control-plane   NotReady   control-plane   5m
```

**Causes & Solutions:**

1. **Cilium installation failed**

   ```bash
   kubectl get pods -n kube-system -l k8s-app=cilium
   ```

2. **Check Cilium logs**
   ```bash
   kubectl logs -n kube-system -l k8s-app=cilium
   ```

See [docs/troubleshooting.md](troubleshooting.md) for more solutions.

## Talos Version Management

### Checking Current Version

```bash
talosctl --talosconfig ./output/talosconfig \
  --nodes <node-ip> \
  version
```

### Upgrading Talos

**Important**: Test upgrades in a non-production environment first.

```bash
# Upgrade control plane nodes one at a time
talosctl --talosconfig ./output/talosconfig \
  --nodes <control-plane-1-ip> \
  upgrade --image ghcr.io/siderolabs/installer:v1.8.0 --wait

# Wait for node to become healthy before upgrading next
talosctl --nodes <control-plane-1-ip> health

# Repeat for other control planes and workers
```

### Upgrading Kubernetes

Update the version in `cluster.yaml`:

```yaml
talos:
  kubernetes_version: 1.31.0
```

Then run upgrade:

```bash
talosctl --talosconfig ./output/talosconfig \
  --nodes <node-ip> \
  upgrade-k8s --to 1.31.0
```

**Best Practices:**

- Upgrade one minor version at a time (1.29 → 1.30 → 1.31)
- Upgrade control planes before workers
- Verify cluster health between upgrades

## References

- [Talos Linux Documentation](https://www.talos.dev/latest/)
- [Talos API Reference](https://www.talos.dev/latest/reference/api/)
- [Machine Configuration Reference](https://www.talos.dev/latest/reference/configuration/)
- [KubePrism Documentation](https://www.talos.dev/latest/kubernetes-guides/configuration/kubeprism/)
- [Talos Security](https://www.talos.dev/latest/introduction/security/)
