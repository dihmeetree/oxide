# Hetzner Cloud Integration

This document explains how Oxide integrates with Hetzner Cloud, including infrastructure setup, networking, security, and cost optimization.

## Overview

Oxide uses the [Hetzner Cloud API](https://docs.hetzner.cloud/) to provision and manage infrastructure for Talos Kubernetes clusters.

### What Gets Created

When you run `oxide create`, the following Hetzner Cloud resources are created:

1. **Private Network** - Isolated network for inter-node communication
2. **Firewall** - Security rules restricting access to your IP
3. **SSH Key** - ED25519 key pair for server management (if needed)
4. **Servers** - Control plane and worker nodes
5. **Snapshots** - Used as base images for Talos Linux

## Authentication

### API Token

Oxide requires a Hetzner Cloud API token with **Read & Write** permissions.

**Get your token:**
1. Log in to [Hetzner Cloud Console](https://console.hetzner.cloud/)
2. Select your project
3. Go to Security → API Tokens
4. Click "Generate API Token"
5. Select **Read & Write** permissions
6. Copy the token (shown only once!)

**Provide token to Oxide:**

Option 1: Environment variable (recommended)
```bash
export HCLOUD_TOKEN=your-token-here
oxide create
```

Option 2: In cluster.yaml
```yaml
hcloud:
  token: your-token-here  # Not recommended for version control
```

**Security Note**: Never commit API tokens to version control. Use environment variables or secret management systems.

## Private Network

### Network Architecture

Oxide creates a Hetzner Cloud private network for secure inter-node communication:

```
Public Internet
    ↓ (Firewall-protected)
┌────────────────────────────────────────┐
│  Hetzner Cloud Private Network         │
│  CIDR: 10.0.0.0/16 (default)           │
│                                         │
│  ┌──────────────────────────────────┐ │
│  │  Subnet: 10.0.1.0/24 (nodes)     │ │
│  │                                   │ │
│  │  Control Planes:                 │ │
│  │  - 10.0.1.1 (control-plane-1)   │ │
│  │  - 10.0.1.2 (control-plane-2)   │ │
│  │  - 10.0.1.3 (control-plane-3)   │ │
│  │                                   │ │
│  │  Workers:                        │ │
│  │  - 10.0.1.4 (worker-1)          │ │
│  │  - 10.0.1.5 (worker-2)          │ │
│  │  - 10.0.1.6 (worker-3)          │ │
│  └──────────────────────────────────┘ │
│                                         │
│  Gateway: 10.0.0.1                     │
└────────────────────────────────────────┘

Note: Nodes route to each other via gateway (L3 routing)
This is why Cilium uses VXLAN tunnel mode
```

### Network Configuration

```yaml
# In cluster.yaml
hcloud:
  network:
    cidr: 10.0.0.0/16          # Private network range
    subnet_cidr: 10.0.1.0/24   # Subnet for nodes
    zone: eu-central           # Network zone
```

**Default Values:**
- Network CIDR: `10.0.0.0/16` (65,536 IPs)
- Subnet CIDR: `10.0.1.0/24` (254 usable IPs for nodes)
- Zone: `eu-central`

### IP Address Allocation

**Node IPs** (10.0.1.0/24):
- Control plane nodes start at `10.0.1.1`
- Worker nodes continue sequentially
- Maximum ~250 nodes per subnet

**Pod IPs** (10.0.16.0/20):
- Managed by Kubernetes/Cilium
- 4,096 IPs available
- Each node gets a /24 subnet (254 pods per node)

**Service IPs** (10.0.8.0/21):
- Kubernetes ClusterIP services
- 2,048 IPs available

### Network Routing

Hetzner private networks use **L3 routing through a gateway (10.0.0.1)**:

```
Node A (10.0.1.1) wants to reach Node B (10.0.1.2):
    ↓
Packet sent to gateway (10.0.0.1)
    ↓
Gateway routes to Node B
    ↓
Node B receives packet
```

**Implications:**
- Nodes are not directly reachable at Layer 2
- Cilium must use VXLAN tunnel mode (not native routing)
- See [docs/cilium.md](cilium.md) for details

### Network Costs

- **Private networks**: Free
- **Internal traffic**: Free (between nodes on same private network)
- **Public traffic**: First 20TB free per server, then €1/TB

## Firewall

### Automatic Firewall Configuration

Oxide automatically detects your public IP and creates restrictive firewall rules.

**Default Rules:**

| Port  | Protocol | Source         | Destination | Purpose              |
|-------|----------|----------------|-------------|----------------------|
| 50000 | TCP      | Your IP/32     | Any         | Talos API            |
| 6443  | TCP      | Your IP/32     | Any         | Kubernetes API       |
| 80    | TCP      | 0.0.0.0/0      | Any         | HTTP (LoadBalancer)  |
| *     | *        | 10.0.0.0/16    | 10.0.0.0/16 | Internal (private)   |

**Security Model:**

✅ **Protected:**
- Talos API (50000) - Only your IP
- Kubernetes API (6443) - Only your IP

✅ **Public:**
- HTTP port 80 - Open to internet (for LoadBalancer services)

✅ **Internal (not restricted by Hetzner firewall):**
- All traffic within private network (10.0.0.0/16)
- Node-to-node communication
- Pod-to-pod traffic

### IP Detection

Oxide automatically detects your public IP using:

```rust
// Queries https://api.ipify.org to get your current public IP
let my_ip = detect_public_ip().await?;
```

**If your IP changes:**

Your IP might change if you:
- Switch networks (home → office → coffee shop)
- Have a dynamic IP from ISP
- Use VPN that changes IPs

**Solutions:**

1. **Update firewall manually** (Hetzner Console):
   - Go to Firewalls
   - Edit rules for ports 50000 and 6443
   - Add your new IP

2. **Use IP range** (if known):
   ```yaml
   # Future feature: custom firewall rules
   hcloud:
     firewall:
       allowed_ips:
         - 203.0.113.0/24  # Your office IP range
   ```

3. **Use bastion/VPN** (enterprise):
   - Route all access through fixed-IP bastion host
   - Or use company VPN with static IP

### Firewall Costs

- **Firewalls**: Free
- **Rules**: Unlimited, free

## SSH Key Management

### Automatic SSH Key Generation

Oxide automatically generates an ED25519 SSH key pair for your cluster:

```bash
oxide create
# Creates: output/id_ed25519 (private key)
# Uploads public key to Hetzner Cloud as "cluster-name-ssh-key"
```

**Key Details:**
- **Algorithm**: ED25519 (modern, secure, fast)
- **Location**: `./output/id_ed25519`
- **Permissions**: 0600 (owner read/write only)
- **Usage**: Attached to all servers (for emergency access if needed)

**Important**: With Talos, you don't need SSH! The key is mainly for:
- Emergency recovery scenarios
- Debugging during development
- Compatibility with Hetzner Cloud requirements

### SSH Key Lifecycle

**On `oxide create`:**
1. Check if `output/id_ed25519` exists
2. If not, generate new ED25519 key pair
3. Upload public key to Hetzner Cloud
4. Attach to all created servers

**On `oxide destroy`:**
1. Delete all servers
2. Delete SSH key from Hetzner Cloud
3. Keep local private key (in case you need to recreate cluster)

### Manual SSH Key Management

If you want to use your own SSH key:

```bash
# Generate your own key
ssh-keygen -t ed25519 -f ./output/id_ed25519 -N ""

# Oxide will detect and use existing key
oxide create
```

**Note**: Even with SSH keys, Talos nodes don't run SSH daemon. Keys are for potential future use or emergency scenarios.

## Server Types

### Choosing Server Types

Hetzner Cloud offers various server types with different CPU/RAM configurations.

**Server Type Naming:**
- **cx** = Shared vCPU (cheaper, good for dev/test)
- **cpx** = Dedicated vCPU (better performance, production)
- **ccx** = Dedicated CPU (highest performance, compute-intensive)

### Recommended Configurations

#### Development/Testing

```yaml
control_planes:
  - name: control-plane
    server_type: cx21      # 2 vCPU, 4GB RAM
    count: 1

workers:
  - name: worker
    server_type: cx21      # 2 vCPU, 4GB RAM
    count: 2
```

**Cost**: ~€15/month
**Use case**: Learning, testing, small projects

#### Production (Small)

```yaml
control_planes:
  - name: control-plane
    server_type: cpx21     # 3 vCPU, 4GB RAM (dedicated)
    count: 3

workers:
  - name: worker
    server_type: cpx31     # 4 vCPU, 8GB RAM
    count: 3
```

**Cost**: ~€32/month
**Use case**: Small production apps, startups

#### Production (Medium)

```yaml
control_planes:
  - name: control-plane
    server_type: cpx31     # 4 vCPU, 8GB RAM
    count: 3

workers:
  - name: worker
    server_type: cpx41     # 8 vCPU, 16GB RAM
    count: 5
```

**Cost**: ~€100/month
**Use case**: Medium-traffic applications

### Server Type Reference

| Type   | vCPUs | RAM   | Storage | Price/Month | CPU Type  |
|--------|-------|-------|---------|-------------|-----------|
| cx21   | 2     | 4GB   | 40GB    | €4.90       | Shared    |
| cx31   | 2     | 8GB   | 80GB    | €8.90       | Shared    |
| cx41   | 4     | 16GB  | 160GB   | €16.90      | Shared    |
| cpx21  | 3     | 4GB   | 80GB    | €7.90       | Dedicated |
| cpx31  | 4     | 8GB   | 160GB   | €13.90      | Dedicated |
| cpx41  | 8     | 16GB  | 240GB   | €26.90      | Dedicated |
| cpx51  | 16    | 32GB  | 360GB   | €51.90      | Dedicated |
| ccx13  | 2     | 8GB   | 80GB    | €28.00      | Dedicated |
| ccx23  | 4     | 16GB  | 160GB   | €54.00      | Dedicated |
| ccx33  | 8     | 32GB  | 240GB   | €104.00     | Dedicated |

**Full list**: https://www.hetzner.com/cloud

### Sizing Guidelines

**Control Plane Nodes:**
- Minimum: 2 vCPU, 4GB RAM (cx21/cpx21)
- Recommended: 3+ vCPU, 4GB+ RAM (cpx21 or higher)
- For large clusters (>50 nodes): cpx31 or higher

**Worker Nodes:**
- Depends on your workload
- Start with cpx31 (4 vCPU, 8GB RAM)
- Scale up based on resource usage
- Use node pools for different workload types

**Control Plane Count:**
- Development: 1 node (not HA)
- Production: 3 nodes (HA, survives 1 failure)
- Large production: 5 nodes (survives 2 failures)

**Worker Count:**
- Minimum: 2 (for redundancy)
- Scale based on workload demand
- Consider resource requests/limits of pods

## Locations and Regions

### Available Locations

Hetzner Cloud has data centers in:

| Location | Region           | Code  |
|----------|------------------|-------|
| Nuremberg| Germany          | nbg1  |
| Falkenstein| Germany        | fsn1  |
| Helsinki | Finland          | hel1  |
| Ashburn  | USA (Virginia)   | ash   |
| Hillsboro| USA (Oregon)     | hil   |

### Choosing a Location

```yaml
# In cluster.yaml
hcloud:
  location: nbg1  # Nuremberg, Germany
```

**Considerations:**

1. **Latency** - Choose closest to your users
   - EU users → nbg1, fsn1, or hel1
   - US East users → ash
   - US West users → hil

2. **Data Residency** - Legal/compliance requirements
   - EU data laws → EU locations (nbg1, fsn1, hel1)
   - US data laws → US locations (ash, hil)

3. **Availability** - Some server types may be unavailable in some locations
   - Check Hetzner Cloud console for current availability

4. **Network Zone** - Must match location
   - EU locations → `zone: eu-central`
   - US locations → `zone: us-east` or `us-west`

**Network Zone Configuration:**

```yaml
hcloud:
  location: nbg1
  network:
    zone: eu-central  # Must match location
```

## Snapshots (Talos Images)

### What Are Snapshots?

Hetzner Cloud doesn't have official Talos Linux images. You need to create a **snapshot** containing the Talos image.

A snapshot is a point-in-time copy of a server's disk that can be used to create new servers.

### Creating a Talos Snapshot

**One-time setup per Hetzner project:**

```bash
# 1. Create temporary server
hcloud server create \
  --type cx11 \
  --name talos-snapshot \
  --image ubuntu-22.04 \
  --location nbg1

# 2. Enable rescue mode and reboot
hcloud server enable-rescue talos-snapshot
hcloud server reboot talos-snapshot

# 3. SSH into rescue system
ssh root@<server-ip>

# 4. Download and write Talos image
wget -O - https://github.com/siderolabs/talos/releases/download/v1.8.0/hcloud-amd64.raw.xz \
  | xz -d | dd of=/dev/sda && sync

# 5. Exit SSH and reboot server
exit
hcloud server reboot talos-snapshot

# 6. Wait ~1 minute for Talos to boot, then create snapshot
hcloud server create-image \
  --type snapshot \
  --description "Talos v1.8.0" \
  talos-snapshot

# 7. Get snapshot ID
hcloud image list | grep Talos

# 8. Delete temporary server
hcloud server delete talos-snapshot
```

### Using Snapshot in Oxide

Add the snapshot ID to your `cluster.yaml`:

```yaml
talos:
  version: v1.8.0
  hcloud_snapshot_id: "123456789"  # Your snapshot ID
```

**Important**: Snapshot version must match `talos.version`.

### Updating Talos Version

When upgrading Talos:

1. Create new snapshot with new Talos version
2. Update `cluster.yaml` with new version and snapshot ID
3. For existing clusters: Use `talosctl upgrade` (see [docs/talos.md](talos.md))
4. For new clusters: Use new snapshot during creation

### Snapshot Costs

- **Snapshots**: €0.0119/GB/month (~€0.50/month for ~40GB Talos image)
- **One snapshot per Talos version** is sufficient for all your clusters

## Cost Optimization

### Monthly Cost Breakdown

**Example 3-node HA cluster:**

```yaml
# Configuration
control_planes: 3x cpx21 (€7.90 each)
workers: 3x cpx31 (€13.90 each)

# Costs
Control planes: 3 × €7.90  = €23.70
Workers:        3 × €13.90 = €41.70
Network:                   = Free
Firewall:                  = Free
Snapshot:                  = €0.50
Traffic:                   = Free (up to 20TB/server)
─────────────────────────────────────
Total:                     = €65.90/month
```

### Cost Saving Strategies

#### 1. Right-Size Servers

**Monitor resource usage:**
```bash
kubectl top nodes
kubectl top pods -A
```

**If nodes are underutilized:**
- Scale down to smaller server types
- Reduce worker count
- Use shared vCPU (cx) for non-production

#### 2. Development vs Production Clusters

**Development cluster** (€15/month):
```yaml
control_planes:
  - server_type: cx21
    count: 1         # Not HA, acceptable for dev

workers:
  - server_type: cx21
    count: 2
```

**Production cluster** (€66/month):
```yaml
control_planes:
  - server_type: cpx21
    count: 3         # HA

workers:
  - server_type: cpx31
    count: 3
```

#### 3. Spot Pricing (Not Available)

Note: Hetzner Cloud doesn't offer spot instances. Prices are consistent.

#### 4. Scale Down During Off-Hours

For development environments:

```bash
# Scale down at night
oxide scale worker --count 1

# Scale up during work hours
oxide scale worker --count 3
```

#### 5. Use Shared Resources

- Single cluster for multiple projects (use namespaces)
- Resource quotas per namespace
- LimitRanges to prevent resource waste

### Traffic Costs

**Included Traffic:**
- 20TB/month per server (free)
- Internal traffic (private network): Free
- Incoming traffic: Free

**Additional Traffic:**
- €1/TB after included quota
- Applied per server

**For typical web apps:**
- 20TB/month is very generous
- 1 million page views ≈ 100GB traffic
- Most small-medium apps stay within free tier

## Resource Limits

### Hetzner Cloud Limits (Per Project)

**Default Limits:**
- Servers: 25
- Floating IPs: 5
- Volumes: 100
- Networks: 100
- Load Balancers: 100

**Request Limit Increase:**
- Contact Hetzner support
- Usually approved within 24 hours
- Free, no additional cost

### Recommended Limits

**For production:**
- Request 50-100 servers if planning large-scale clusters
- Plan ahead - increases can take up to 24 hours

## Troubleshooting

### Common Issues

#### "Insufficient Resources" Error

**Symptom:**
```
Error: Server creation failed: insufficient resources
```

**Causes:**
1. Server type unavailable in location
2. Location at capacity
3. Account limits reached

**Solutions:**
1. Try different server type
2. Try different location
3. Contact Hetzner support to increase limits

#### Firewall Blocking Access

**Symptom:**
```
talosctl health
error: connection timeout
```

**Causes:**
- Your IP changed
- Firewall rules incorrect

**Solutions:**
1. Check your current IP: `curl https://api.ipify.org`
2. Update firewall in Hetzner Console
3. Or recreate with `oxide destroy` and `oxide create`

#### Network Connectivity Issues

**Symptom:**
- Pods can't reach each other across nodes
- LoadBalancer not working

**Solutions:**
1. Verify private network exists and is attached to all servers
2. Check Cilium health: `kubectl exec -n kube-system <cilium-pod> -- cilium-health status`
3. See [docs/cilium.md](cilium.md) and [docs/troubleshooting.md](troubleshooting.md)

## References

- [Hetzner Cloud Documentation](https://docs.hetzner.cloud/)
- [Hetzner Cloud API](https://docs.hetzner.cloud/api)
- [Hetzner Cloud Console](https://console.hetzner.cloud/)
- [Hetzner Cloud Pricing](https://www.hetzner.com/cloud)
- [Hetzner Status Page](https://status.hetzner.com/)
