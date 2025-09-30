/// Talos configuration generation
use anyhow::{Context, Result};
use std::path::Path;
use std::process::Stdio;
use tokio::process::Command;
use tracing::info;

use crate::config::TalosConfig;

/// Talos configuration generator
pub struct TalosConfigGenerator {
    cluster_name: String,
    talos_config: TalosConfig,
}

impl TalosConfigGenerator {
    /// Create a new Talos configuration generator
    pub fn new(cluster_name: String, talos_config: TalosConfig) -> Self {
        Self {
            cluster_name,
            talos_config,
        }
    }

    /// Generate Talos configuration files using talosctl
    pub async fn generate_configs(
        &self,
        control_plane_endpoint: &str,
        output_dir: &Path,
    ) -> Result<GeneratedConfigs> {
        info!("Generating Talos configuration files...");

        // Ensure output directory exists
        tokio::fs::create_dir_all(output_dir)
            .await
            .context("Failed to create output directory")?;

        // Check if secrets file already exists
        let secrets_path = output_dir.join("secrets.yaml");
        let secrets_exists = secrets_path.exists();

        // Generate base configuration using talosctl with patches
        let mut args = vec![
            "gen",
            "config",
            &self.cluster_name,
            control_plane_endpoint,
            "--output-dir",
            output_dir.to_str().unwrap(),
            "--kubernetes-version",
            &self.talos_config.kubernetes_version,
            "--force",               // Overwrite existing config files
            "--with-docs=false",     // Exclude docs to stay under 32KB user_data limit
            "--with-examples=false", // Exclude examples to stay under 32KB user_data limit
            // Control plane patches
            "--config-patch-control-plane",
            "@patches/control-plane.yaml",
            // Worker patches
            "--config-patch-worker",
            "@patches/worker.yaml",
        ];

        // Only use existing secrets if the file exists
        if secrets_exists {
            info!("Using existing secrets file");
            args.push("--with-secrets");
            args.push(secrets_path.to_str().unwrap());
        }

        let output = Command::new("talosctl")
            .args(&args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .context("Failed to execute talosctl")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("talosctl gen config failed: {}", stderr);
        }

        info!("Talos configuration files generated successfully");

        Ok(GeneratedConfigs {
            controlplane: output_dir.join("controlplane.yaml"),
            worker: output_dir.join("worker.yaml"),
            talosconfig: output_dir.join("talosconfig"),
            secrets: output_dir.join("secrets.yaml"),
        })
    }
}

/// Generated Talos configuration files
#[derive(Debug, Clone)]
pub struct GeneratedConfigs {
    pub controlplane: std::path::PathBuf,
    pub worker: std::path::PathBuf,
    pub talosconfig: std::path::PathBuf,
    #[allow(dead_code)]
    pub secrets: std::path::PathBuf,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_generator_creation() {
        let talos_config = TalosConfig {
            version: "v1.7.0".to_string(),
            kubernetes_version: "1.30.0".to_string(),
            cluster_endpoint: None,
            hcloud_snapshot_id: None,
            config_patches: vec![],
        };

        let generator = TalosConfigGenerator::new("test-cluster".to_string(), talos_config);
        assert_eq!(generator.cluster_name, "test-cluster");
    }
}
