/// CLI command definitions and handling
use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "oxide")]
#[command(about = "Deploy Talos Linux clusters on Hetzner Cloud", long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,

    /// Configuration file path
    #[arg(short, long, default_value = "cluster.yaml")]
    pub config: PathBuf,

    /// Output directory for generated files
    #[arg(short, long, default_value = "./output")]
    pub output: PathBuf,

    /// Enable verbose logging
    #[arg(short, long)]
    pub verbose: bool,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Create a new Talos cluster
    Create,

    /// Destroy an existing cluster
    Destroy,

    /// Show cluster status
    Status,

    /// Generate example configuration file
    Init,

    /// Scale cluster nodes
    Scale {
        /// Node type to scale
        #[arg(value_enum)]
        node_type: NodeType,

        /// Target number of nodes
        #[arg(short, long)]
        count: u32,

        /// Node pool name (optional, uses first pool if not specified)
        #[arg(short, long)]
        pool: Option<String>,

        /// Force non-graceful scale down (skip drain, immediate removal)
        #[arg(long)]
        force: bool,

        /// Timeout in seconds for graceful node reset (default: 600)
        #[arg(long, default_value = "600")]
        timeout: u64,
    },

    /// Upgrade cluster
    Upgrade {
        /// Talos version (e.g., v1.11.3)
        #[arg(long)]
        version: String,

        /// Preserve node data during upgrade
        #[arg(long, default_value = "true")]
        preserve: bool,

        /// Upgrade control plane nodes
        #[arg(long)]
        control_plane: bool,

        /// Upgrade worker nodes
        #[arg(long)]
        workers: bool,

        /// Wait and observe the upgrade process for each node
        #[arg(long)]
        wait: bool,

        /// Stage the upgrade (useful if upgrade fails due to open files)
        #[arg(long)]
        stage: bool,
    },

    /// Deploy nginx with Gateway API
    DeployNginx,

    /// Install Prometheus monitoring stack
    InstallPrometheus,

    /// Show Prometheus status
    PrometheusStatus,

    /// Uninstall Prometheus monitoring stack
    UninstallPrometheus,

    /// Install cluster autoscaler
    InstallAutoscaler,

    /// Uninstall cluster autoscaler
    UninstallAutoscaler,

    /// Install Kubernetes Metrics Server
    InstallMetricsServer,

    /// Uninstall Kubernetes Metrics Server
    UninstallMetricsServer,

    /// Start web dashboard for cluster management
    Dashboard {
        /// Port to listen on
        #[arg(short, long, default_value = "3000")]
        port: u16,
    },
}

#[derive(Debug, Clone, clap::ValueEnum)]
pub enum NodeType {
    ControlPlane,
    Worker,
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn test_cli_verify() {
        // Verifies that CLI is correctly configured
        Cli::command().debug_assert();
    }

    #[test]
    fn test_cli_defaults() {
        let args = vec!["oxide", "create"];
        let cli = Cli::try_parse_from(args).unwrap();

        assert_eq!(cli.config, PathBuf::from("cluster.yaml"));
        assert_eq!(cli.output, PathBuf::from("./output"));
        assert!(!cli.verbose);
        assert!(matches!(cli.command, Commands::Create));
    }

    #[test]
    fn test_cli_with_custom_paths() {
        let args = vec![
            "oxide",
            "-c",
            "/path/to/config.yaml",
            "-o",
            "/tmp/output",
            "create",
        ];
        let cli = Cli::try_parse_from(args).unwrap();

        assert_eq!(cli.config, PathBuf::from("/path/to/config.yaml"));
        assert_eq!(cli.output, PathBuf::from("/tmp/output"));
    }

    #[test]
    fn test_cli_verbose_flag() {
        let args = vec!["oxide", "--verbose", "status"];
        let cli = Cli::try_parse_from(args).unwrap();

        assert!(cli.verbose);
        assert!(matches!(cli.command, Commands::Status));
    }

    #[test]
    fn test_scale_command_parsing() {
        let args = vec![
            "oxide", "scale", "worker", "--count", "5", "--pool", "my-pool",
        ];
        let cli = Cli::try_parse_from(args).unwrap();

        match cli.command {
            Commands::Scale {
                node_type,
                count,
                pool,
                force,
                timeout,
            } => {
                assert!(matches!(node_type, NodeType::Worker));
                assert_eq!(count, 5);
                assert_eq!(pool, Some("my-pool".to_string()));
                assert!(!force);
                assert_eq!(timeout, 600); // default
            }
            _ => panic!("Expected Scale command"),
        }
    }

    #[test]
    fn test_scale_command_with_force() {
        let args = vec![
            "oxide",
            "scale",
            "control-plane",
            "--count",
            "3",
            "--force",
            "--timeout",
            "300",
        ];
        let cli = Cli::try_parse_from(args).unwrap();

        match cli.command {
            Commands::Scale {
                node_type,
                count,
                force,
                timeout,
                ..
            } => {
                assert!(matches!(node_type, NodeType::ControlPlane));
                assert_eq!(count, 3);
                assert!(force);
                assert_eq!(timeout, 300);
            }
            _ => panic!("Expected Scale command"),
        }
    }

    #[test]
    fn test_upgrade_command_parsing() {
        let args = vec![
            "oxide",
            "upgrade",
            "--version",
            "v1.8.0",
            "--control-plane",
            "--workers",
            "--wait",
        ];
        let cli = Cli::try_parse_from(args).unwrap();

        match cli.command {
            Commands::Upgrade {
                version,
                preserve,
                control_plane,
                workers,
                wait,
                stage,
            } => {
                assert_eq!(version, "v1.8.0");
                assert!(preserve); // default true
                assert!(control_plane);
                assert!(workers);
                assert!(wait);
                assert!(!stage);
            }
            _ => panic!("Expected Upgrade command"),
        }
    }

    #[test]
    fn test_upgrade_command_with_stage() {
        let args = vec![
            "oxide",
            "upgrade",
            "--version",
            "v1.9.0",
            "--control-plane",
            "--stage",
        ];
        let cli = Cli::try_parse_from(args).unwrap();

        match cli.command {
            Commands::Upgrade {
                version,
                preserve,
                stage,
                ..
            } => {
                assert_eq!(version, "v1.9.0");
                assert!(preserve); // default is true, no way to set false with current CLI definition
                assert!(stage);
            }
            _ => panic!("Expected Upgrade command"),
        }
    }

    #[test]
    fn test_all_simple_commands() {
        let simple_commands = vec![
            ("create", Commands::Create),
            ("destroy", Commands::Destroy),
            ("status", Commands::Status),
            ("init", Commands::Init),
            ("deploy-nginx", Commands::DeployNginx),
            ("install-prometheus", Commands::InstallPrometheus),
            ("prometheus-status", Commands::PrometheusStatus),
            ("uninstall-prometheus", Commands::UninstallPrometheus),
            ("install-autoscaler", Commands::InstallAutoscaler),
            ("uninstall-autoscaler", Commands::UninstallAutoscaler),
            ("install-metrics-server", Commands::InstallMetricsServer),
            ("uninstall-metrics-server", Commands::UninstallMetricsServer),
        ];

        for (cmd_str, expected_variant) in simple_commands {
            let args = vec!["oxide", cmd_str];
            let cli = Cli::try_parse_from(args).unwrap();

            // Use discriminant to compare enum variants
            assert_eq!(
                std::mem::discriminant(&cli.command),
                std::mem::discriminant(&expected_variant),
                "Failed for command: {}",
                cmd_str
            );
        }
    }

    #[test]
    fn test_node_type_parsing_in_cli() {
        // Test that NodeType can be parsed through CLI
        let args = vec!["oxide", "scale", "control-plane", "--count", "3"];
        let cli = Cli::try_parse_from(args).unwrap();

        if let Commands::Scale { node_type, .. } = cli.command {
            assert!(matches!(node_type, NodeType::ControlPlane));
        } else {
            panic!("Expected Scale command");
        }

        let args = vec!["oxide", "scale", "worker", "--count", "5"];
        let cli = Cli::try_parse_from(args).unwrap();

        if let Commands::Scale { node_type, .. } = cli.command {
            assert!(matches!(node_type, NodeType::Worker));
        } else {
            panic!("Expected Scale command");
        }
    }

    #[test]
    fn test_scale_missing_required_args() {
        let args = vec!["oxide", "scale", "worker"];
        let result = Cli::try_parse_from(args);
        assert!(result.is_err()); // Missing --count
    }

    #[test]
    fn test_upgrade_missing_required_args() {
        let args = vec!["oxide", "upgrade"];
        let result = Cli::try_parse_from(args);
        assert!(result.is_err()); // Missing --version
    }

    #[test]
    fn test_long_and_short_flags() {
        let args_long = vec![
            "oxide",
            "--config",
            "test.yaml",
            "--output",
            "/tmp",
            "status",
        ];
        let args_short = vec!["oxide", "-c", "test.yaml", "-o", "/tmp", "status"];

        let cli_long = Cli::try_parse_from(args_long).unwrap();
        let cli_short = Cli::try_parse_from(args_short).unwrap();

        assert_eq!(cli_long.config, cli_short.config);
        assert_eq!(cli_long.output, cli_short.output);
    }
}
