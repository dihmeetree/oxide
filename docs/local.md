# Local Clusters (Docker Provisioner)

This document covers everything specific to Oxide's **local cluster** mode,
which runs a real Talos Linux Kubernetes cluster on top of your local Docker
daemon. It's the same Talos that powers the Hetzner-Cloud flow, just packaged
into containers instead of full VMs — making it ideal for:

- fast development inner-loops (cluster up in ~2 minutes),
- CI / e2e testing,
- offline experimentation, and
- reproducing cluster-level bugs without a cloud bill.

For the cloud (Hetzner) flow, see [`hetzner.md`](hetzner.md). For the full
configuration reference covering both providers, see
[`configuration.md`](configuration.md).

## How it works

Oxide drives Talos's built-in Docker provisioner via `talosctl`:

```text
oxide create
  └── talosctl cluster --name <name> create docker \
        --workers N \
        --kubernetes-version <ver> \
        --image ghcr.io/siderolabs/talos:<ver> \
        --talosconfig-destination <output>/talosconfig \
        --config-patch '<strategic-merge YAML disabling default CNI/proxy>'
  └── talosctl kubeconfig <output>/kubeconfig \
        --talosconfig <output>/talosconfig --nodes 127.0.0.1 \
        --merge=false --force
  └── (optional) Cilium · metrics-server · kube-prometheus-stack
```

Each Talos node runs as a Docker container on a dedicated Docker bridge
network (default CIDR `10.5.0.0/24`). The control plane's API server is
exposed on the host via a forwarded port (always reachable as
`127.0.0.1:<port>` — Oxide uses this fact when exporting the kubeconfig).

The Docker provisioner currently supports **a single control plane node**.
Configurations requesting more are rejected at validation time.

## Prerequisites

| Tool       | Why                                              | Install                                                      |
| ---------- | ------------------------------------------------ | ------------------------------------------------------------ |
| Docker     | Runs the Talos containers                        | `curl -fsSL https://get.docker.com \| sh`                    |
| `talosctl` | Drives Talos cluster create/destroy and kubeconfig export | `curl -sL https://talos.dev/install \| sh`                   |
| `kubectl`  | Health checks, optional-component installs       | <https://kubernetes.io/docs/tasks/tools/>                    |
| `helm`     | Cilium / Prometheus chart installs               | `curl -sL https://raw.githubusercontent.com/helm/helm/main/scripts/get-helm-3 \| bash` |

`oxide create` runs prerequisite checks and fails fast with a friendly error
if any of these are missing or if the Docker daemon isn't running.

## Quick start

```bash
# 1. Scaffold a config tailored for local clusters
oxide init --provider docker
# → writes cluster.yaml with provider: docker, 1 CP, 1 worker, all
#   optional components enabled

# 2. (Optional) review or tweak cluster.yaml
$EDITOR cluster.yaml

# 3. Create the cluster
oxide create
# Takes ~2 minutes: image pull → containers up → etcd bootstrap →
# CNI install → metrics-server → Prometheus stack.

# 4. Use it
export KUBECONFIG=./output/kubeconfig
kubectl get nodes

# 5. Inspect status at any time
oxide status

# 6. Tear it down (also removes the output/ directory)
oxide destroy
```

## Configuration

The smallest valid local config:

```yaml
cluster_name: talos-local
provider: docker

talos:
  version: v1.13.0
  kubernetes_version: 1.35.0

cilium:
  version: 1.19.3
  enable_hubble: true
  enable_ipv6: false

control_planes:
  - name: control-plane
    count: 1
    server_type: "" # Ignored for local clusters

workers:
  - name: worker
    count: 1
    server_type: ""
```

### `docker:` block (all fields optional)

```yaml
docker:
  # Override the Talos image. Defaults to ghcr.io/siderolabs/talos:<talos.version>
  image: ghcr.io/siderolabs/talos:v1.13.0

  # Pin the host port forwarded to the Kubernetes API (container port 6443).
  # If unset, talosctl picks an ephemeral port (visible in `docker ps`).
  api_port: 6443

  # Override the Docker bridge subnet (default 10.5.0.0/24). Change only if
  # it collides with another network on your host.
  network_cidr: 10.5.0.0/24
```

### Optional components

`metrics_server`, `prometheus`, and `cilium` are configured exactly the same
way as for Hetzner — they run on top of the kubeconfig and don't care about
the underlying provider. The only caveat for local clusters is
**persistent storage**: out of the box, the Talos Docker provisioner has no
default storage class, so if you enable Prometheus you should set:

```yaml
prometheus:
  enabled: true
  enable_persistent_storage: false
```

(or install a storage class yourself before `oxide create`).

The cluster **autoscaler** is Hetzner-specific and rejected at
config-validation time when `provider: docker`.

## Commands supported on local clusters

| Command           | Behavior                                                        |
| ----------------- | --------------------------------------------------------------- |
| `oxide init --provider docker` | Writes a local-flavoured `cluster.yaml`            |
| `oxide create`    | Brings the cluster up (talosctl + optional components)          |
| `oxide status`    | Lists Talos containers (`docker ps`) and `kubectl get nodes`    |
| `oxide destroy`   | Calls `talosctl cluster destroy` and removes `output/`          |
| `oxide install-prometheus` / `install-metrics-server` / etc. | Work the same as on Hetzner — they only need the kubeconfig |
| `oxide scale`     | **Rejected** with a helpful error — destroy and re-create       |
| `oxide upgrade`   | **Rejected** with a helpful error — destroy and re-create       |

## Files written under `output/`

| File                     | Description                                              |
| ------------------------ | -------------------------------------------------------- |
| `talosconfig`            | Talos API client config (used by `talosctl`)             |
| `kubeconfig`             | Stand-alone kubeconfig (does **not** touch `~/.kube/config`) |
| `config-snapshot.yaml`   | Snapshot of the resolved cluster config (used by Prometheus install) |
| `grafana-admin-password` | Auto-generated Grafana admin password (mode 0600)        |

## Limitations

- **Single control plane only.** The Docker provisioner does not support
  multi-CP topologies. Use the Hetzner provider for HA.
- **No `oxide scale` / `oxide upgrade`.** Re-create the cluster with the
  desired counts/version.
- **No autoscaler.** Hetzner-specific.
- **`server_type` is ignored** under `control_planes` / `workers` — Docker
  containers have no instance type. Per-pool `count` is honored.
- **Storage classes are not pre-installed.** Anything requiring a PVC
  (e.g. Prometheus persistent storage) needs you to install a CSI driver
  first or to be configured with `enable_persistent_storage: false`.

## Troubleshooting

### `Cannot connect to the Docker daemon`

The Docker daemon isn't running.
On systemd hosts: `sudo systemctl start docker`. On Docker Desktop, start
the app and wait for the whale icon to settle.

### `talosctl: not found`

Install `talosctl` (`curl -sL https://talos.dev/install | sh`) or extend
your `PATH`.

### `provider = 'docker' only supports a single control plane node`

You set `control_planes[*].count` to more than 1. The Docker provisioner
hard-codes a single control plane — set the count back to `1` or switch
to `provider: hcloud` for HA.

### `JSON6902 patches are not supported for multi-document machine configuration`

You're on a Talos older than v1.13 with patches that target multiple node
classes. Oxide already uses strategic-merge patches for the local flow, so
this should not happen — please file a bug if you hit it.

### Prometheus pods stuck `Pending` (PVC pending)

The Talos Docker provisioner ships without a default StorageClass.
Either:

1. set `prometheus.enable_persistent_storage: false`, or
2. install a CSI driver (e.g.
   [`local-path-provisioner`](https://github.com/rancher/local-path-provisioner))
   before running `oxide create`.

### Port `6443` already in use

Another process — often a previous local cluster — is bound to the
default API port. Either:

- destroy the previous cluster first (`oxide destroy`), or
- set `docker.api_port` to a different port in `cluster.yaml`, or
- leave `docker.api_port` unset and let talosctl pick a free ephemeral
  port (the actual port is visible in `docker ps` and in the generated
  kubeconfig).

## See also

- [`hetzner.md`](hetzner.md) — Hetzner Cloud provider
- [`configuration.md`](configuration.md) — Full configuration reference
- [`talos.md`](talos.md) — Talos Linux integration details
- [`cilium.md`](cilium.md) — Cilium CNI configuration
