<p align="center">
  <img src="https://i.imgur.com/sUo78EC.png" alt="Oxide" width="850">
</p>

A Rust-based tool for deploying Talos Linux Kubernetes clusters with Cilium CNI. Currently supports Hetzner Cloud, with more cloud providers coming soon. Similar to [terraform-hcloud-talos](https://github.com/hcloud-talos/terraform-hcloud-talos) but built entirely in Rust without Terraform dependencies.

> [!WARNING]
> This project is under active development and is considered experimental. Features may change, and not all functionality is production-ready yet.
> If you encounter bugs or have feature requests, please open an issue on [GitHub](https://github.com/dihmeetree/oxide/issues).

## Features

- **Automated Cluster Deployment**: Create production-ready Kubernetes clusters on Hetzner Cloud
- **Talos Linux**: Immutable, minimal, and secure Kubernetes operating system
- **Cilium CNI**: High-performance networking with eBPF
- **LoadBalancer Support**: Cilium Node IPAM for LoadBalancer services using node IPs
- **Prometheus Monitoring**: Built-in support for Prometheus stack (Prometheus, Grafana, AlertManager)
- **Metrics Server**: Kubernetes resource metrics for HPA and kubectl top commands
- **Cluster Autoscaler**: Automatic worker node scaling based on pod resource demands (official Kubernetes autoscaler with Hetzner support)
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

### Pre-built Binaries

Download the latest release from the [GitHub Releases page](https://github.com/dihmeetree/oxide/releases).

#### Linux (x86_64)

```bash
curl -LO https://github.com/dihmeetree/oxide/releases/latest/download/\
oxide-linux-x86_64.tar.gz
tar xzf oxide-linux-x86_64.tar.gz
sudo mv oxide /usr/local/bin/
```

#### Linux (ARM64)

```bash
curl -LO https://github.com/dihmeetree/oxide/releases/latest/download/\
oxide-linux-aarch64.tar.gz
tar xzf oxide-linux-aarch64.tar.gz
sudo mv oxide /usr/local/bin/
```

#### macOS (Intel)

```bash
curl -LO https://github.com/dihmeetree/oxide/releases/latest/download/\
oxide-macos-x86_64.tar.gz
tar xzf oxide-macos-x86_64.tar.gz
sudo mv oxide /usr/local/bin/
```

#### macOS (Apple Silicon)

```bash
curl -LO https://github.com/dihmeetree/oxide/releases/latest/download/\
oxide-macos-aarch64.tar.gz
tar xzf oxide-macos-aarch64.tar.gz
sudo mv oxide /usr/local/bin/
```

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

> **Note**: Check the latest Talos version at https://github.com/siderolabs/talos/releases and update the version in the commands below accordingly.

```bash
# 1. Create a temporary server
hcloud server create --type cx11 --name talos-snapshot --image ubuntu-22.04 --location nbg1

# 2. Enable rescue mode and reboot
hcloud server enable-rescue talos-snapshot
hcloud server reboot talos-snapshot

# 3. Connect to rescue system and write Talos image
# SSH into the server in rescue mode
ssh root@<server-ip>
# Then run this command to write the Talos image (replace v1.11.2 with latest version)
wget -O - https://github.com/siderolabs/talos/releases/download/v1.11.2/hcloud-amd64.raw.xz | xz -d | dd of=/dev/sda && sync

# 4. Reboot the server
hcloud server reboot talos-snapshot

# 5. Wait for boot, then create snapshot
hcloud server create-image --type snapshot --description "Talos v1.11.2" talos-snapshot

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

This creates a `cluster.yaml` file with default settings that you can customize.

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
  version: v1.11.2
  kubernetes_version: 1.34.1
  hcloud_snapshot_id: "123456789" # Your snapshot ID from step 1

cilium:
  version: 1.17.8
  enable_hubble: true
  enable_ipv6: false

prometheus:
  version: 77.13.0
  enabled: true
  namespace: monitoring
  enable_grafana: true
  enable_alertmanager: true
  retention: 30d
  storage_size: 50Gi
  enable_persistent_storage: false

metrics_server:
  enabled: true

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
8. Install optional components (Metrics Server, Prometheus, Autoscaler) based on configuration

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
# Using default cluster.yaml
oxide create

# Using a custom configuration file
oxide --config my-cluster.yaml create
```

### Show Cluster Status

```bash
# Using default cluster.yaml
oxide status

# Using a custom configuration file
oxide --config my-cluster.yaml status
```

Shows information about all servers organized by node pools, including current node counts and server specifications.

### Scale Cluster Nodes

Scale the number of nodes in your cluster up or down:

```bash
# Scale workers to 5 nodes (uses first worker pool by default)
oxide scale worker --count 5

# Scale control plane nodes to 3
oxide scale control-plane --count 3

# Scale a specific node pool
oxide scale worker --count 10 --pool worker-large
```

**Scaling Behavior**:

- **Scale Up**: Creates new nodes with the same configuration as the existing pool, automatically configures them with Talos, and applies firewall rules
- **Scale Down**: Removes the newest nodes first (highest index numbers)
- **Pool-specific**: Can target specific node pools if you have multiple worker or control plane pools configured

**Example Use Cases**:

```bash
# Increase workers for higher workload
oxide scale worker --count 10

# Scale down to save costs during low-usage periods
oxide scale worker --count 2

# Add more control plane nodes for HA
oxide scale control-plane --count 3
```

**Important Notes**:

- Scaling is idempotent - if already at target count, no changes are made
- New nodes are automatically joined to the cluster
- When scaling down, ensure your workloads can handle node removals
- Control plane scaling: maintaining odd numbers (1, 3, 5) is recommended for etcd quorum

### Destroy a Cluster

```bash
# Using default cluster.yaml
oxide destroy

# Using a custom configuration file
oxide --config my-cluster.yaml destroy
```

**Warning**: This permanently deletes all servers, networks, and SSH keys.

### Install Prometheus Monitoring

Install the Prometheus monitoring stack (Prometheus, Grafana, AlertManager):

```bash
oxide install-prometheus
```

This installs the `kube-prometheus-stack` Helm chart with:

- Prometheus server with persistent storage
- Grafana dashboards (default login: admin/admin)
- AlertManager for notifications
- Service monitors for Cilium and Kubernetes components

### Show Prometheus Status

```bash
oxide prometheus-status
```

Shows the status of all Prometheus components and provides Grafana access instructions.

### Access Grafana Dashboard

To access Grafana locally, use port-forwarding:

```bash
kubectl port-forward -n monitoring svc/prometheus-grafana 3000:80 --kubeconfig=./output/kubeconfig
```

Then open http://localhost:3000 in your browser:

- Username: `admin`
- Password: `admin` (change after first login)

### Access Prometheus UI

To access Prometheus UI locally:

```bash
kubectl port-forward -n monitoring svc/prometheus-kube-prometheus-prometheus 9090:9090 --kubeconfig=./output/kubeconfig
```

Then open http://localhost:9090 in your browser.

### Access AlertManager UI

To access AlertManager UI locally:

```bash
kubectl port-forward -n monitoring svc/prometheus-kube-prometheus-alertmanager 9093:9093 --kubeconfig=./output/kubeconfig
```

Then open http://localhost:9093 in your browser.

### Uninstall Prometheus

```bash
oxide uninstall-prometheus
```

### Deploy Cluster Autoscaler

Deploy the Kubernetes Cluster Autoscaler with Hetzner support to automatically scale worker nodes based on pod resource requests:

```bash
oxide deploy-autoscaler
```

This deploys the official Kubernetes Cluster Autoscaler configured for Hetzner Cloud provider. The autoscaler will:

- Automatically add worker nodes when pods cannot be scheduled due to insufficient resources
- Remove underutilized worker nodes to save costs
- Respect min/max node limits configured per worker pool

**Configuration Example**:

```yaml
autoscaler:
  enabled: true
  worker_pools:
    - name: worker-pool
      server_type: cpx11 # Hetzner server type
      location: fsn1 # Hetzner location
      min_nodes: 1
      max_nodes: 10
```

**Monitor Autoscaler Logs**:

```bash
kubectl logs -n oxide-system -l app=cluster-autoscaler -f --kubeconfig=./output/kubeconfig
```

**Important Notes**:

- The autoscaler only scales worker nodes, not control plane nodes
- Scaling decisions are based on pod resource requests (CPU/memory), not actual usage
- Nodes are created with the same Talos configuration as your initial worker nodes
- The autoscaler respects PodDisruptionBudgets when scaling down

### Uninstall Cluster Autoscaler

```bash
oxide uninstall-autoscaler
```

### Install Metrics Server

Install the Kubernetes Metrics Server for resource metrics and HPA support:

```bash
oxide install-metrics-server
```

The Metrics Server enables:

- `kubectl top nodes` and `kubectl top pods` commands
- HorizontalPodAutoscaler (HPA) to scale pods based on CPU/memory usage
- Resource-based autoscaling decisions

**Verify Installation**:

```bash
kubectl top nodes --kubeconfig=./output/kubeconfig
```

**Note**: Metrics Server is automatically installed during cluster creation if enabled in the configuration.

### Uninstall Metrics Server

```bash
oxide uninstall-metrics-server
```

## Configuration Reference

### Cluster Configuration

| Field            | Description                    | Required |
| ---------------- | ------------------------------ | -------- |
| `cluster_name`   | Unique name for your cluster   | Yes      |
| `hcloud`         | Hetzner Cloud settings         | Yes      |
| `talos`          | Talos Linux configuration      | Yes      |
| `cilium`         | Cilium CNI settings            | Yes      |
| `prometheus`     | Prometheus monitoring settings | No       |
| `metrics_server` | Metrics Server settings        | No       |
| `autoscaler`     | Cluster autoscaler settings    | No       |
| `control_planes` | Control plane node specs       | Yes      |
| `workers`        | Worker node specs              | No       |

### Hetzner Cloud Settings

| Field                 | Description                                   | Default     |
| --------------------- | --------------------------------------------- | ----------- |
| `token`               | API token (or use `HCLOUD_TOKEN` env var)     | -           |
| `location`            | Data center location (nbg1, fsn1, hel1, etc.) | nbg1        |
| `network.cidr`        | Private network CIDR                          | 10.0.0.0/16 |
| `network.subnet_cidr` | Subnet CIDR                                   | 10.0.1.0/24 |
| `network.zone`        | Network zone                                  | eu-central  |

### Prometheus Configuration

| Field                       | Description                              | Default    |
| --------------------------- | ---------------------------------------- | ---------- |
| `version`                   | kube-prometheus-stack chart version      | 77.13.0    |
| `enabled`                   | Enable Prometheus installation           | true       |
| `namespace`                 | Kubernetes namespace for Prometheus      | monitoring |
| `enable_grafana`            | Enable Grafana dashboards                | true       |
| `enable_alertmanager`       | Enable AlertManager                      | true       |
| `retention`                 | Prometheus data retention period         | 30d        |
| `storage_size`              | Prometheus persistent storage size       | 50Gi       |
| `enable_persistent_storage` | Enable persistent storage for Prometheus | false      |

### Metrics Server Configuration

| Field     | Description           | Default |
| --------- | --------------------- | ------- |
| `enabled` | Enable metrics server | true    |

**Note**: Metrics Server is automatically installed during cluster creation when enabled.

### Cluster Autoscaler Configuration

| Field          | Description                       | Required |
| -------------- | --------------------------------- | -------- |
| `enabled`      | Enable cluster autoscaler         | Yes      |
| `worker_pools` | List of worker pools to autoscale | Yes      |

**Worker Pool Configuration**:

| Field         | Description                                                          | Required | Default |
| ------------- | -------------------------------------------------------------------- | -------- | ------- |
| `name`        | Worker pool name                                                     | Yes      | -       |
| `server_type` | Hetzner server type (cpx11, cpx21...)                                | Yes      | -       |
| `location`    | Hetzner location (fsn1, nbg1...)                                     | Yes      | -       |
| `min_nodes`   | Minimum autoscaled nodes (set to 0 to preserve initial worker nodes) | No       | 0       |
| `max_nodes`   | Maximum autoscaled nodes                                             | Yes      | -       |

> **Important**: Set `min_nodes: 0` to ensure the autoscaler only manages nodes it creates dynamically, leaving your initial worker nodes (defined in `workers.count`) untouched. This way:
>
> - Your base worker nodes always remain in the cluster
> - The autoscaler only creates/deletes additional nodes above this baseline
> - Pods will be consolidated back to original nodes when autoscaled nodes are no longer needed

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
│  - HTTPS (443): Public access                │
└──────────────────────────────────────────────┘
                    ↓
┌──────────────────────────────────────────────┐
│      Hetzner Cloud Private Network           │
│             10.0.0.0/16                      │
│         Node Subnet: 10.0.1.0/24             │
│         Pod CIDR: 10.0.16.0/20               │
│         Service CIDR: 10.0.8.0/21            │
│                                              │
│  ┌────────────┐  ┌────────────┐              │
│  │ Control    │  │ Control    │              │
│  │ Plane 1    │  │ Plane 2    │  ...         │
│  └────────────┘  └────────────┘              │
│                                              │
│  ┌────────────┐  ┌────────────┐              │
│  │ Worker 1   │  │ Worker 2   │  ...         │
│  └────────────┘  └────────────┘              │
└──────────────────────────────────────────────┘
```

### Firewall Rules

The automatically configured firewall includes:

| Port  | Protocol | Source    | Purpose        |
| ----- | -------- | --------- | -------------- |
| 6443  | TCP      | Your IP   | Kubernetes API |
| 50000 | TCP      | Your IP   | Talos API      |
| 80    | TCP      | 0.0.0.0/0 | HTTP Traffic   |
| 443   | TCP      | 0.0.0.0/0 | HTTPS Traffic  |

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
