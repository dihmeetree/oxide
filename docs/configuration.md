# Configuration Reference

Complete reference for `cluster.yaml` configuration file.

## File Structure

```yaml
cluster_name: string          # Required: Unique cluster identifier
hcloud: { ... }               # Required: Hetzner Cloud settings
talos: { ... }                # Required: Talos Linux configuration
cilium: { ... }               # Required: Cilium CNI settings
control_planes: [...]         # Required: Control plane node pools
workers: [...]                # Optional: Worker node pools
```

## Top-Level Fields

### `cluster_name`

**Type:** `string`
**Required:** Yes
**Description:** Unique name for your cluster. Used as prefix for all resources.

**Example:**
```yaml
cluster_name: my-production-cluster
```

**Validation:**
- Must be alphanumeric + hyphens
- No spaces or special characters
- Used in resource names: `{cluster_name}-worker-1`

## Hetzner Cloud Configuration

### `hcloud`

```yaml
hcloud:
  token: string                     # Optional: API token (use env var instead)
  location: string                  # Required: Data center location
  network:
    cidr: string                    # Required: Private network CIDR
    subnet_cidr: string             # Required: Node subnet CIDR
    zone: string                    # Required: Network zone
```

#### `hcloud.token`

**Type:** `string`
**Required:** No (use `HCLOUD_TOKEN` env var instead)
**Description:** Hetzner Cloud API token

**Best Practice:**
```bash
export HCLOUD_TOKEN=your-token-here
# Don't put token in cluster.yaml
```

#### `hcloud.location`

**Type:** `string`
**Required:** Yes
**Default:** `nbg1`
**Description:** Hetzner data center location

**Valid Values:**
- `nbg1` - Nuremberg, Germany
- `fsn1` - Falkenstein, Germany
- `hel1` - Helsinki, Finland
- `ash` - Ashburn, USA
- `hil` - Hillsboro, USA

#### `hcloud.network.cidr`

**Type:** `string` (CIDR notation)
**Required:** Yes
**Default:** `10.0.0.0/16`
**Description:** Private network CIDR range

**Constraints:**
- Must not overlap with pod_cidr or service_cidr
- Recommended: /16 network (65,536 IPs)

#### `hcloud.network.subnet_cidr`

**Type:** `string` (CIDR notation)
**Required:** Yes
**Default:** `10.0.1.0/24`
**Description:** Subnet for node IPs within private network

**Constraints:**
- Must be within network.cidr range
- /24 allows ~250 nodes

#### `hcloud.network.zone`

**Type:** `string`
**Required:** Yes
**Default:** `eu-central`
**Description:** Hetzner network zone

**Valid Values:**
- `eu-central` (for nbg1, fsn1, hel1)
- `us-east` (for ash)
- `us-west` (for hil)

**Must match location region!**

## Talos Configuration

### `talos`

```yaml
talos:
  version: string                   # Required: Talos version
  kubernetes_version: string        # Required: Kubernetes version
  hcloud_snapshot_id: string        # Required: Talos snapshot ID
  pod_cidr: string                  # Optional: Pod network CIDR
  service_cidr: string              # Optional: Service network CIDR
```

#### `talos.version`

**Type:** `string`
**Required:** Yes
**Description:** Talos Linux version

**Example:** `v1.8.0`

**Must match snapshot version!**

#### `talos.kubernetes_version`

**Type:** `string`
**Required:** Yes
**Description:** Kubernetes version to install

**Example:** `1.30.0`

**Supported Versions:** Check [Talos compatibility matrix](https://www.talos.dev/latest/introduction/support-matrix/)

#### `talos.hcloud_snapshot_id`

**Type:** `string`
**Required:** Yes
**Description:** Hetzner snapshot ID containing Talos image

**Example:** `"123456789"`

**How to get:** See [README.md - Create Talos Snapshot](../README.md#1-create-talos-snapshot)

#### `talos.pod_cidr`

**Type:** `string` (CIDR notation)
**Required:** No
**Default:** `10.0.16.0/20`
**Description:** CIDR range for pod IPs

**Constraints:**
- Must not overlap with hcloud.network.cidr
- /20 provides 4,096 IPs
- Each node gets /24 subnet (254 pods/node)

#### `talos.service_cidr`

**Type:** `string` (CIDR notation)
**Required:** No
**Default:** `10.0.8.0/21`
**Description:** CIDR range for Kubernetes service IPs

**Constraints:**
- Must not overlap with pod_cidr or network.cidr
- /21 provides 2,048 IPs

## Cilium Configuration

### `cilium`

```yaml
cilium:
  version: string                   # Required: Cilium version
  enable_hubble: boolean            # Optional: Enable Hubble observability
  enable_ipv6: boolean              # Optional: Enable IPv6 support
```

#### `cilium.version`

**Type:** `string`
**Required:** Yes
**Description:** Cilium Helm chart version

**Example:** `1.17.8`

**Compatible Versions:** 1.15.0+

#### `cilium.enable_hubble`

**Type:** `boolean`
**Required:** No
**Default:** `true`
**Description:** Enable Hubble observability UI

**Note:** Adds resource overhead (extra pods)

#### `cilium.enable_ipv6`

**Type:** `boolean`
**Required:** No
**Default:** `false`
**Description:** Enable IPv6 support

**Note:** Experimental, not tested with Oxide

## Node Pool Configuration

### Control Plane Pools

```yaml
control_planes:
  - name: string                    # Required: Pool name
    server_type: string             # Required: Hetzner server type
    count: integer                  # Required: Number of nodes
    labels: map[string]string       # Optional: Kubernetes labels
```

**Example:**
```yaml
control_planes:
  - name: control-plane
    server_type: cpx21              # 3 vCPU, 4GB RAM
    count: 3                        # HA configuration
```

### Worker Pools

```yaml
workers:
  - name: string                    # Required: Pool name
    server_type: string             # Required: Hetzner server type
    count: integer                  # Required: Number of nodes
    labels: map[string]string       # Optional: Kubernetes labels
```

**Example:**
```yaml
workers:
  - name: worker-small
    server_type: cpx21
    count: 3

  - name: worker-large
    server_type: cpx41              # 8 vCPU, 16GB RAM
    count: 2
    labels:
      workload-type: memory-intensive
```

#### `name`

**Type:** `string`
**Required:** Yes
**Description:** Pool name, used in node naming

**Example:** `worker` → creates `cluster-worker-1`, `cluster-worker-2`, etc.

#### `server_type`

**Type:** `string`
**Required:** Yes
**Description:** Hetzner server type

**Common Types:**
- `cx21` - 2 vCPU, 4GB RAM (shared)
- `cpx21` - 3 vCPU, 4GB RAM (dedicated)
- `cpx31` - 4 vCPU, 8GB RAM (dedicated)
- `cpx41` - 8 vCPU, 16GB RAM (dedicated)

**Full list:** https://www.hetzner.com/cloud

#### `count`

**Type:** `integer`
**Required:** Yes
**Minimum:** 1
**Description:** Number of nodes in pool

**Recommendations:**
- Control plane: 1 (dev), 3 (production), 5 (large)
- Workers: 2+ (for redundancy)

#### `labels`

**Type:** `map[string]string`
**Required:** No
**Description:** Kubernetes labels to apply to nodes

**Example:**
```yaml
labels:
  workload-type: compute-intensive
  environment: production
```

## Complete Example

```yaml
cluster_name: production-cluster

hcloud:
  location: nbg1
  network:
    cidr: 10.0.0.0/16
    subnet_cidr: 10.0.1.0/24
    zone: eu-central

talos:
  version: v1.8.0
  kubernetes_version: 1.30.0
  hcloud_snapshot_id: "123456789"
  pod_cidr: 10.0.16.0/20
  service_cidr: 10.0.8.0/21

cilium:
  version: 1.17.8
  enable_hubble: true
  enable_ipv6: false

control_planes:
  - name: control-plane
    server_type: cpx21
    count: 3

workers:
  - name: worker-general
    server_type: cpx31
    count: 3
    labels:
      workload-type: general

  - name: worker-memory
    server_type: cpx41
    count: 2
    labels:
      workload-type: memory-intensive
```

## Validation

Oxide validates configuration on load:

✅ **Checked:**
- Required fields present
- Valid CIDR notation
- No CIDR overlaps
- Valid Hetzner location
- Server types exist

❌ **Not Checked:**
- Snapshot exists
- API token valid
- Account has sufficient quota

**Errors reported immediately on `oxide create`**

## Environment Variables

| Variable | Description | Required |
|----------|-------------|----------|
| `HCLOUD_TOKEN` | Hetzner Cloud API token | Yes |
| `KUBECONFIG` | Path to kubeconfig file | No (for kubectl commands) |

## References

- [Hetzner Cloud API](https://docs.hetzner.cloud/)
- [Talos Configuration Reference](https://www.talos.dev/latest/reference/configuration/)
- [Cilium Helm Values](https://docs.cilium.io/en/stable/installation/k8s-install-helm/)
