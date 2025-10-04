/// Command execution utilities to reduce code duplication
use anyhow::{Context, Result};
use std::ffi::OsStr;
use std::path::Path;
use std::process::Stdio;
use tokio::process::Command;

/// Result from command execution with captured output
pub struct CommandOutput {
    pub stdout: String,
    pub stderr: String,
    pub success: bool,
}

impl CommandOutput {
    /// Create from tokio Command output
    fn from_output(output: std::process::Output) -> Self {
        Self {
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            success: output.status.success(),
        }
    }

    /// Return Ok if successful, otherwise error with stderr
    pub fn into_result(self) -> Result<String> {
        if self.success {
            Ok(self.stdout)
        } else {
            anyhow::bail!("{}", self.stderr)
        }
    }
}

/// Builder for executing external commands with common patterns
pub struct CommandBuilder {
    command: Command,
    context_msg: Option<String>,
}

impl CommandBuilder {
    /// Create a new command builder
    pub fn new<S: AsRef<OsStr>>(program: S) -> Self {
        let mut command = Command::new(program);
        command.stdout(Stdio::piped()).stderr(Stdio::piped());
        Self {
            command,
            context_msg: None,
        }
    }

    /// Add a single argument
    #[allow(dead_code)]
    pub fn arg<S: AsRef<OsStr>>(mut self, arg: S) -> Self {
        self.command.arg(arg);
        self
    }

    /// Add multiple arguments
    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.command.args(args);
        self
    }

    /// Set an environment variable
    pub fn env<K, V>(mut self, key: K, val: V) -> Self
    where
        K: AsRef<OsStr>,
        V: AsRef<OsStr>,
    {
        self.command.env(key, val);
        self
    }

    /// Set KUBECONFIG environment variable
    pub fn kubeconfig(self, path: &Path) -> Self {
        self.env("KUBECONFIG", path)
    }

    /// Set context message for error reporting
    pub fn context<S: Into<String>>(mut self, msg: S) -> Self {
        self.context_msg = Some(msg.into());
        self
    }

    /// Execute and return raw output
    pub async fn output(mut self) -> Result<CommandOutput> {
        let output = if let Some(ctx) = &self.context_msg {
            self.command.output().await.context(ctx.clone())?
        } else {
            self.command.output().await?
        };
        Ok(CommandOutput::from_output(output))
    }

    /// Execute and return stdout on success, error on failure
    pub async fn run(self) -> Result<String> {
        self.output().await?.into_result()
    }

    /// Execute and ignore output (just check success)
    pub async fn run_silent(self) -> Result<()> {
        self.output().await?.into_result().map(|_| ())
    }
}

/// Check if a command-line tool is installed
pub async fn check_tool_installed(
    tool_name: &str,
    version_args: &[&str],
    install_url: &str,
) -> Result<()> {
    let output = CommandBuilder::new(tool_name)
        .args(version_args)
        .output()
        .await;

    match output {
        Ok(out) if out.success => Ok(()),
        _ => anyhow::bail!(
            "{} is not installed or not in PATH. Please install from {}",
            tool_name,
            install_url
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_command_builder_basic() {
        // Test with a simple command that should exist on all systems
        let result = CommandBuilder::new("echo")
            .arg("test")
            .context("Testing echo command")
            .output()
            .await;

        assert!(result.is_ok());
        let output = result.unwrap();
        assert!(output.success);
        assert!(output.stdout.contains("test"));
    }

    #[tokio::test]
    async fn test_command_builder_env() {
        let result = CommandBuilder::new("sh")
            .arg("-c")
            .arg("echo $TEST_VAR")
            .env("TEST_VAR", "test_value")
            .output()
            .await;

        assert!(result.is_ok());
        let output = result.unwrap();
        assert!(output.success);
        assert!(output.stdout.contains("test_value"));
    }
}
