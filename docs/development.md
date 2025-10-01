# Development Guide

Guide for contributing to and developing Oxide.

## Table of Contents

- [Getting Started](#getting-started)
- [Project Structure](#project-structure)
- [Development Workflow](#development-workflow)
- [Testing](#testing)
- [Code Quality](#code-quality)
- [Adding Features](#adding-features)
- [Release Process](#release-process)

## Getting Started

### Prerequisites

- Rust 1.70+ ([install](https://rustup.rs/))
- Git
- CLI tools: `talosctl`, `kubectl`, `helm`
- Hetzner Cloud account (for testing)

### Clone and Build

```bash
git clone https://github.com/dihmeetree/oxide
cd oxide
cargo build
```

### Run in Development

```bash
# Run without installing
cargo run -- create --config cluster.yaml

# Enable debug logging
RUST_LOG=debug cargo run -- create
```

## Project Structure

```
oxide/
├── src/
│   ├── main.rs              # CLI entry point, command handling
│   ├── config/              # Configuration management
│   │   └── mod.rs           # Parse/validate cluster.yaml
│   ├── hcloud/              # Hetzner Cloud integration
│   │   ├── client.rs        # API client
│   │   ├── server.rs        # Server operations
│   │   ├── network.rs       # Network management
│   │   ├── firewall.rs      # Firewall rules
│   │   ├── ssh_key.rs       # SSH key management
│   │   └── models.rs        # API types
│   ├── talos/               # Talos operations
│   │   ├── client.rs        # talosctl wrapper
│   │   └── config.rs        # Config generation
│   ├── cilium/              # Cilium CNI
│   │   └── mod.rs           # Helm installation
│   └── k8s/                 # Kubernetes operations
│       ├── client.rs        # kubectl checks
│       ├── nodes.rs         # Node lifecycle
│       └── resources.rs     # Resource management
├── docs/                    # Documentation
├── Cargo.toml               # Dependencies
├── cluster.example.yaml     # Example configuration
└── README.md                # User documentation
```

### Module Responsibilities

**See [docs/architecture.md](architecture.md#module-responsibilities) for detailed module documentation.**

## Development Workflow

### 1. Create Feature Branch

```bash
git checkout -b feature/my-new-feature
```

### 2. Make Changes

Follow the [Code Quality](#code-quality) standards.

### 3. Test Changes

```bash
# Format code
cargo fmt

# Check compilation
cargo check

# Run tests
cargo test --release

# Run clippy
cargo clippy -- -D warnings
```

### 4. Commit Changes

Follow project commit guidelines (see `CLAUDE.md`):

```bash
cargo fmt  # ALWAYS format before git add
git add .
git commit -m "feat: Add new feature

Description of changes...

🤖 Generated with [Claude Code](https://claude.com/claude-code)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

### 5. Push and Create PR

```bash
git push origin feature/my-new-feature
```

Create pull request on GitHub.

## Testing

### Unit Tests

```bash
# Run all tests
cargo test

# Run specific test
cargo test test_name

# Run with output
cargo test -- --nocapture
```

### Integration Tests

**Manual testing with real cluster:**

1. Create test configuration:
   ```yaml
   # test-cluster.yaml
   cluster_name: oxide-test
   hcloud:
     location: nbg1
   # ... minimal config
   ```

2. Test create:
   ```bash
   cargo run -- create --config test-cluster.yaml
   ```

3. Test operations:
   ```bash
   cargo run -- status
   cargo run -- scale worker --count 5
   cargo run -- scale worker --count 3
   ```

4. Clean up:
   ```bash
   cargo run -- destroy --config test-cluster.yaml
   ```

### Test Coverage

Currently no automated integration tests. Manual testing required.

**TODO:** Add integration test suite.

## Code Quality

### Formatting

**Always run before committing:**

```bash
cargo fmt
```

### Linting

```bash
cargo clippy -- -D warnings
```

**Fix all warnings before committing.**

### Documentation

All public functions must have doc comments:

```rust
/// Creates a new Hetzner server
///
/// # Arguments
/// * `name` - Server name
/// * `server_type` - Hetzner server type (e.g., "cpx21")
///
/// # Returns
/// Server information including ID and IPs
///
/// # Errors
/// Returns error if API call fails or server creation times out
pub async fn create_server(name: &str, server_type: &str) -> Result<ServerInfo> {
    // ...
}
```

### Error Handling

Use `anyhow` for error context:

```rust
use anyhow::{Context, Result};

pub async fn create_network() -> Result<Network> {
    let response = api_call()
        .await
        .context("Failed to create network")?;

    Ok(response)
}
```

### Logging

Use `tracing` for structured logging:

```rust
use tracing::{info, warn, error};

info!("Creating cluster: {}", cluster_name);
warn!("Node not ready yet, retrying...");
error!("Failed to connect to API: {}", err);
```

## Adding Features

### Adding a New Command

1. **Add to CLI** (`src/main.rs`):
   ```rust
   #[derive(Subcommand)]
   enum Commands {
       Create { ... },
       MyNewCommand {
           #[arg(long)]
           my_option: String,
       },
   }
   ```

2. **Add handler**:
   ```rust
   Commands::MyNewCommand { my_option } => {
       my_new_command_handler(my_option).await?;
   }
   ```

3. **Implement logic**:
   ```rust
   async fn my_new_command_handler(option: String) -> Result<()> {
       // Implementation
       Ok(())
   }
   ```

4. **Update README.md** with new command documentation

### Adding Cloud Provider Support

To add support for AWS, GCP, etc.:

1. **Create new module**:
   ```
   src/
   └── aws/
       ├── mod.rs
       ├── client.rs
       ├── ec2.rs
       └── vpc.rs
   ```

2. **Implement provider trait** (create if doesn't exist):
   ```rust
   #[async_trait]
   pub trait CloudProvider {
       async fn create_servers(&self, count: usize) -> Result<Vec<ServerInfo>>;
       async fn delete_servers(&self, ids: Vec<String>) -> Result<()>;
       // ...
   }
   ```

3. **Update config** to support multiple providers:
   ```yaml
   provider: aws  # or hcloud, gcp, etc.
   aws:
       region: us-east-1
       # ...
   ```

4. **Add provider selection** in main.rs

### Adding Cilium Features

To add new Cilium configuration options:

1. **Update config struct** (`src/config/mod.rs`):
   ```rust
   pub struct CiliumConfig {
       pub version: String,
       pub enable_hubble: bool,
       pub my_new_option: bool,  // Add here
   }
   ```

2. **Update Helm values** (`src/cilium/mod.rs`):
   ```rust
   if self.config.my_new_option {
       args.extend_from_slice(&[
           "--set",
           "my.cilium.option=true",
       ]);
   }
   ```

3. **Update `cluster.example.yaml`**
4. **Document in `docs/cilium.md`**

## Release Process

### Version Numbering

Follow [Semantic Versioning](https://semver.org/):

- **MAJOR**: Breaking changes
- **MINOR**: New features (backwards compatible)
- **PATCH**: Bug fixes

### Creating a Release

1. **Update version** in `Cargo.toml`:
   ```toml
   [package]
   version = "0.2.0"
   ```

2. **Update CHANGELOG.md** (if exists):
   ```markdown
   ## [0.2.0] - 2025-01-15
   ### Added
   - New scaling feature
   ### Fixed
   - Firewall bug
   ```

3. **Commit version bump**:
   ```bash
   git add Cargo.toml CHANGELOG.md
   git commit -m "chore: Bump version to 0.2.0"
   ```

4. **Create tag**:
   ```bash
   git tag -a v0.2.0 -m "Release v0.2.0"
   git push origin v0.2.0
   ```

5. **Build release binary**:
   ```bash
   cargo build --release
   # Binary at: target/release/oxide
   ```

6. **Create GitHub release**:
   - Go to GitHub → Releases → Draft new release
   - Choose tag `v0.2.0`
   - Upload `target/release/oxide` binary
   - Add release notes from CHANGELOG

### CI/CD (Future)

**TODO:** Set up GitHub Actions for:
- Automated testing on PR
- Release binary builds
- Documentation deployment

## Common Development Tasks

### Adding a New Hetzner API Endpoint

1. **Add types** to `src/hcloud/models.rs`:
   ```rust
   #[derive(Serialize, Deserialize)]
   pub struct NewResource {
       pub id: u64,
       pub name: String,
   }
   ```

2. **Add client method** to `src/hcloud/client.rs`:
   ```rust
   pub async fn create_resource(&self, name: &str) -> Result<NewResource> {
       self.client
           .post("https://api.hetzner.cloud/v1/resources")
           .json(&json!({ "name": name }))
           .send()
           .await?
           .json()
           .await
           .context("Failed to create resource")
   }
   ```

3. **Use in feature code**

### Debugging API Calls

Enable request logging:

```bash
RUST_LOG=debug,reqwest=trace cargo run -- create
```

### Testing Without Creating Resources

Mock Hetzner API calls (currently not implemented):

**TODO:** Add mock testing framework.

## Troubleshooting Development Issues

### Build Fails

```bash
# Clean build artifacts
cargo clean

# Update dependencies
cargo update

# Rebuild
cargo build
```

### Tests Fail

```bash
# Run specific test with output
cargo test test_name -- --nocapture

# Run all tests with backtrace
RUST_BACKTRACE=1 cargo test
```

### Clippy Warnings

```bash
# Fix automatically (where possible)
cargo clippy --fix

# Show detailed explanations
cargo clippy -- -D warnings --verbose
```

## Contributing Guidelines

1. **Code Quality**
   - All code must compile without warnings
   - All tests must pass
   - Clippy must pass with `-D warnings`
   - Code must be formatted with `cargo fmt`

2. **Documentation**
   - Public APIs must have doc comments
   - Update relevant docs in `docs/` directory
   - Update README.md if adding user-facing features

3. **Testing**
   - Add unit tests for new functions
   - Manually test with real cluster
   - Document test procedures in PR

4. **Commits**
   - Follow project commit message format
   - Format code before `git add`
   - Keep commits focused and atomic

5. **Pull Requests**
   - PR description explains changes
   - Link related issues
   - Request review from maintainers

## Architecture Decisions

### Why Rust?

- Type safety prevents bugs
- Performance (fast CLI)
- Excellent error handling (Result types)
- Great ecosystem (tokio, serde, clap)

### Why CLI Wrappers (talosctl, kubectl)?

Instead of native Rust libraries:

**Pros:**
- Leverage existing, well-tested tools
- Faster development
- Users already have tools installed
- Easier to match official behavior

**Cons:**
- External dependencies
- Parsing CLI output (potential fragility)
- No compile-time guarantees

**Alternative:** Could use native libraries (kube-rs, etc.) in future.

### Why YAML Config?

- Familiar to Kubernetes users
- Human-readable
- Easy to template (envsubst, etc.)
- Standard for infrastructure tools

## Future Development

### Planned Features

1. **Multi-cloud support** (AWS, GCP, DigitalOcean)
2. **GitOps integration** (Flux, ArgoCD)
3. **Cluster upgrades** (rolling Talos/K8s updates)
4. **Backup/restore** (etcd, volumes)
5. **Monitoring stack** (Prometheus, Grafana)
6. **Service mesh** (Istio/Linkerd integration)

### Technical Debt

- [ ] Add integration test suite
- [ ] Mock Hetzner API for testing
- [ ] CI/CD pipeline
- [ ] Error message improvements
- [ ] Progress bars for long operations
- [ ] Parallel operations (create servers concurrently)

## Resources

- [Rust Book](https://doc.rust-lang.org/book/)
- [Tokio Tutorial](https://tokio.rs/tokio/tutorial)
- [Hetzner Cloud API](https://docs.hetzner.cloud/)
- [Talos Documentation](https://www.talos.dev/latest/)
- [Cilium Documentation](https://docs.cilium.io/)

## Getting Help

- Open an issue on GitHub
- Check existing issues for solutions
- Review documentation in `docs/` directory
- Ask in project discussions

## License

[Add license information]
