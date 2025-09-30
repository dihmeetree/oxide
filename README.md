# Oxide

**Talos Kubernetes with Cilium**

A Rust-based tool for deploying Talos Linux Kubernetes clusters with Cilium CNI. Currently supports Hetzner Cloud, with more cloud providers coming soon. Similar to [terraform-hcloud-talos](https://github.com/hcloud-talos/terraform-hcloud-talos) but built entirely in Rust without Terraform dependencies.

## Features

- **Automated Cluster Deployment**: Create production-ready Kubernetes clusters on Hetzner Cloud
- **Talos Linux**: Immutable, minimal, and secure Kubernetes operating system
- **Cilium CNI**: High-performance networking with eBPF
- **LoadBalancer Support**: Cilium Node IPAM for LoadBalancer services using node IPs
- **Private Networking**: Automatic setup of Hetzner Cloud private networks
- **Security First**:
  - Firewall with Talos/Kubernetes API ports pre-configured
  - IP allowlisting (restricts access to your IP only)
- **Flexible Configuration**: YAML-based cluster configuration
- **Multiple Node Types**: Support for control plane and worker nodes with different specifications
- **Health Checks**: Built-in validation and cluster readiness checks

## Prerequisites

Before using this tool, you need to install the following CLI tools:

- **talosctl** - Talos Linux CLI tool ([installation guide](https://www.talos.dev/latest/talos-guides/install/talosctl/))
- **kubectl** - Kubernetes CLI tool ([installation guide](https://kubernetes.io/docs/tasks/tools/))
- **helm** - Kubernetes package manager ([installation guide](https://helm.sh/docs/intro/install/))

## Installation

### From Source

```bash
git clone https://github.com/dihmeetree/oxide
cd oxide
cargo build --release
cargo install --path .
```

The binary will be available as `oxide`.

## Quick Start

### 1. Create Talos Snapshot

Before deploying clusters, you need to create a Hetzner Cloud snapshot containing the Talos image:

```bash
# 1. Create a temporary server
hcloud server create --type cx11 --name talos-snapshot --image ubuntu-22.04 --location nbg1

# 2. Enable rescue mode and reboot
hcloud server enable-rescue talos-snapshot
hcloud server reboot talos-snapshot

# 3. Connect to rescue system and write Talos image
# SSH into the server in rescue mode
ssh root@<server-ip>
# Then run this command to write the Talos image
wget -O - https://github.com/siderolabs/talos/releases/download/v1.7.0/hcloud-amd64.raw.xz | xz -d | dd of=/dev/sda && sync

# 4. Reboot the server
hcloud server reboot talos-snapshot

# 5. Wait for boot, then create snapshot
hcloud server create-image --type snapshot --description "Talos v1.7.0" talos-snapshot

# 6. Note the snapshot ID (you'll need this for configuration)
hcloud image list

# 7. Delete the temporary server
hcloud server delete talos-snapshot
```

### 2. Generate Configuration

Create an example configuration file:

```bash
oxide init
```

This creates a `cluster.yaml` file with default settings.

### 3. Configure Your Cluster

Edit the `cluster.yaml` file to match your requirements:

```yaml
cluster_name: my-talos-cluster

hcloud:
  # Get your token from https://console.hetzner.cloud/
  # Or set HCLOUD_TOKEN environment variable
  location: nbg1
  network:
    cidr: 10.0.0.0/16
    subnet_cidr: 10.0.1.0/24
    zone: eu-central

talos:
  version: v1.7.0
  kubernetes_version: 1.30.0
  hcloud_snapshot_id: "123456789" # Your snapshot ID from step 1

cilium:
  version: 1.15.0
  enable_hubble: true
  enable_ipv6: false

control_planes:
  - name: control-plane
    server_type: cpx21 # 3 vCPUs, 4GB RAM
    count: 3

workers:
  - name: worker
    server_type: cpx31 # 4 vCPUs, 8GB RAM
    count: 3
```

### 4. Set API Token

```bash
export HCLOUD_TOKEN=your-hetzner-cloud-api-token
```

### 5. Create Cluster

```bash
oxide create
```

This will:

1. Detect your public IP and create firewall rules
2. Create a private network
3. Provision control plane and worker servers with firewall applied
4. Generate and apply Talos configurations
5. Bootstrap the Kubernetes cluster
6. Install Cilium CNI
7. Generate kubeconfig file

**Security Notes:**

- Firewall restricts Talos and Kubernetes API access to your current IP address only
- All inter-cluster communication uses private network
- Talos provides secure API-only access (no SSH)

The process typically takes 5-10 minutes.

### 6. Access Your Cluster

```bash
export KUBECONFIG=./output/kubeconfig
kubectl get nodes
```

## Commands

### Create a Cluster

```bash
oxide create --config cluster.yaml
```

### Show Cluster Status

```bash
oxide status --config cluster.yaml
```

Shows information about all servers and Cilium pods.

### Destroy a Cluster

```bash
oxide destroy --config cluster.yaml
```

**Warning**: This permanently deletes all servers and networks.

### Generate Example Config

```bash
oxide init --config my-cluster.yaml
```

## Configuration Reference

### Cluster Configuration

| Field            | Description                  | Required |
| ---------------- | ---------------------------- | -------- |
| `cluster_name`   | Unique name for your cluster | Yes      |
| `hcloud`         | Hetzner Cloud settings       | Yes      |
| `talos`          | Talos Linux configuration    | Yes      |
| `cilium`         | Cilium CNI settings          | Yes      |
| `control_planes` | Control plane node specs     | Yes      |
| `workers`        | Worker node specs            | No       |

### Hetzner Cloud Settings

| Field                 | Description                                   | Default     |
| --------------------- | --------------------------------------------- | ----------- |
| `token`               | API token (or use `HCLOUD_TOKEN` env var)     | -           |
| `location`            | Data center location (nbg1, fsn1, hel1, etc.) | nbg1        |
| `network.cidr`        | Private network CIDR                          | 10.0.0.0/16 |
| `network.subnet_cidr` | Subnet CIDR                                   | 10.0.1.0/24 |
| `network.zone`        | Network zone                                  | eu-central  |

### Node Configuration

| Field         | Description                             | Default |
| ------------- | --------------------------------------- | ------- |
| `name`        | Node name prefix                        | -       |
| `server_type` | Hetzner server type (cx21, cpx31, etc.) | -       |
| `count`       | Number of nodes to create               | 1       |
| `labels`      | Additional Kubernetes labels            | {}      |

### Hetzner Server Types (Common Options)

| Type  | vCPUs | RAM  | Description    |
| ----- | ----- | ---- | -------------- |
| cx21  | 2     | 4GB  | Shared vCPU    |
| cpx21 | 3     | 4GB  | Dedicated vCPU |
| cpx31 | 4     | 8GB  | Dedicated vCPU |
| cpx41 | 8     | 16GB | Dedicated vCPU |
| cpx51 | 16    | 32GB | Dedicated vCPU |

See [Hetzner Cloud pricing](https://www.hetzner.com/cloud) for all available types.

## Architecture

The tool creates:

1. **Firewall**: Hetzner Cloud firewall with restricted access to Talos and Kubernetes APIs
2. **Private Network**: A Hetzner Cloud private network for inter-node communication
3. **Control Plane Nodes**: Run the Kubernetes control plane (etcd, API server, scheduler, controller manager)
4. **Worker Nodes**: Run your application workloads
5. **Cilium**: Provides networking, load balancing, and network policies

### Network Architecture

```
           Your IP (Firewall Allowed)
                    ↓
┌──────────────────────────────────────────────┐
│        Hetzner Cloud Firewall                │
│  - Talos API (50000): Your IP only           │
│  - Kubernetes API (6443): Your IP only       │
│  - HTTP (80): Public access                  │
└──────────────────────────────────────────────┘
                    ↓
┌──────────────────────────────────────────────┐
│      Hetzner Cloud Private Network           │
│             10.0.0.0/16                       │
│         Node Subnet: 10.0.1.0/24             │
│         Pod CIDR: 10.0.16.0/20               │
│         Service CIDR: 10.0.8.0/21            │
│                                               │
│  ┌────────────┐  ┌────────────┐             │
│  │ Control    │  │ Control    │             │
│  │ Plane 1    │  │ Plane 2    │  ...        │
│  └────────────┘  └────────────┘             │
│                                               │
│  ┌────────────┐  ┌────────────┐             │
│  │ Worker 1   │  │ Worker 2   │  ...        │
│  └────────────┘  └────────────┘             │
└──────────────────────────────────────────────┘
```

### Firewall Rules

The automatically configured firewall includes:

| Port  | Protocol | Source    | Purpose        |
| ----- | -------- | --------- | -------------- |
| 6443  | TCP      | Your IP   | Kubernetes API |
| 50000 | TCP      | Your IP   | Talos API      |
| 80    | TCP      | 0.0.0.0/0 | HTTP Traffic   |

**Note**: Internal cluster communication on the private network (10.0.0.0/16) is not restricted by Hetzner Cloud firewalls.

## Output Files

After cluster creation, the following files are generated in the `output/` directory:

- `controlplane.yaml` - Talos configuration for control plane nodes
- `worker.yaml` - Talos configuration for worker nodes
- `talosconfig` - Talos client configuration
- `kubeconfig` - Kubernetes client configuration
- `secrets.yaml` - Talos secrets (keep secure!)

**Important**: The secrets.yaml file contains sensitive information. Keep it secure and never commit to version control.

## Troubleshooting

### Cluster Creation Fails

1. **Check API token**: Ensure `HCLOUD_TOKEN` is set correctly
2. **Verify prerequisites**: Make sure talosctl, kubectl, and helm are installed
3. **Check logs**: Run with `--verbose` flag for detailed output
4. **Resource limits**: Verify your Hetzner account has sufficient resources

### Nodes Not Ready

```bash
# Check Talos node status
talosctl --talosconfig ./output/talosconfig --nodes <node-ip> health

# Check Kubernetes pods
kubectl get pods -A
```

### Cilium Issues

```bash
# Check Cilium status
kubectl get pods -n kube-system -l k8s-app=cilium

# View Cilium logs
kubectl logs -n kube-system -l k8s-app=cilium
```

## Cost Estimation

Example monthly costs for a 3 control plane + 3 worker cluster:

- **Control Planes** (3x cpx21): ~€12/month
- **Workers** (3x cpx31): ~€20/month
- **Network**: Free
- **Traffic**: First 20TB free per server

**Total**: ~€32/month

Costs are approximate. See [Hetzner pricing](https://www.hetzner.com/cloud) for exact rates.

## Comparison with Terraform

### Advantages of This Tool

- **Single Binary**: No Terraform or provider management
- **Type Safety**: Rust's type system catches errors at compile time
- **Performance**: Fast Rust implementation
- **Native Integration**: Direct API calls, no intermediate layers

### When to Use Terraform

- You need to manage other infrastructure beyond Hetzner
- Your team has existing Terraform expertise
- You require Terraform's extensive module ecosystem

## Development

### Building

```bash
cargo build
```

### Running Tests

```bash
cargo test --release
```

### Code Quality

```bash
cargo clippy -- -D warnings
cargo fmt
```

## Contributing

Contributions are welcome! Please ensure your code:

1. Compiles without warnings
2. Passes all tests
3. Follows Rust formatting conventions
4. Includes documentation for public APIs

## License

[Add your license here]

## Acknowledgments

- [Talos Linux](https://www.talos.dev/) - Secure Kubernetes OS
- [Cilium](https://cilium.io/) - eBPF-based networking
- [Hetzner Cloud](https://www.hetzner.com/cloud) - Affordable cloud hosting
- [terraform-hcloud-talos](https://github.com/hcloud-talos/terraform-hcloud-talos) - Inspiration for this project

## Security

- Never commit your `HCLOUD_TOKEN` or API credentials
- Store kubeconfig files securely
- Use private networks for inter-node communication
- Enable Cilium network policies for pod-to-pod security
- Regularly update Talos and Kubernetes versions

## Support

For issues and questions:

1. Check the [Troubleshooting](#troubleshooting) section
2. Review [Talos documentation](https://www.talos.dev/latest/)
3. Check [Cilium documentation](https://docs.cilium.io/)
4. Open an issue on GitHub
