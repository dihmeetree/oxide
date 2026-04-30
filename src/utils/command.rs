/// Command execution utilities to reduce code duplication
use anyhow::Result;
use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::path::Path;
use std::process::{ExitStatus, Stdio};
use std::sync::Arc;
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

/// Description of a command to be executed.
///
/// Provided to `CommandRunner` implementations so they can inspect or fake
/// the operation rather than actually spawning a subprocess.
#[derive(Debug, Clone)]
pub struct CommandSpec {
    pub program: OsString,
    pub args: Vec<OsString>,
    pub envs: HashMap<OsString, OsString>,
}

impl CommandSpec {
    /// Convenience accessor returning the program as a `&str` (lossy).
    #[allow(dead_code)]
    pub fn program_str(&self) -> std::borrow::Cow<'_, str> {
        self.program.to_string_lossy()
    }

    /// Convenience accessor returning args as `&str` slices (lossy).
    #[allow(dead_code)]
    pub fn args_str(&self) -> Vec<std::borrow::Cow<'_, str>> {
        self.args.iter().map(|a| a.to_string_lossy()).collect()
    }
}

/// Trait abstracting command execution so callers can be unit tested.
///
/// The default runner spawns a real subprocess via `tokio::process::Command`;
/// tests inject custom implementations to assert which commands ran and to
/// return canned output without touching the host system.
#[async_trait::async_trait]
pub trait CommandRunner: Send + Sync + std::fmt::Debug {
    async fn run(&self, spec: &CommandSpec) -> std::io::Result<std::process::Output>;
}

/// Default runner that delegates to `tokio::process::Command`.
#[derive(Debug, Default, Clone, Copy)]
pub struct TokioRunner;

#[async_trait::async_trait]
impl CommandRunner for TokioRunner {
    async fn run(&self, spec: &CommandSpec) -> std::io::Result<std::process::Output> {
        let mut command = Command::new(&spec.program);
        command
            .args(&spec.args)
            .envs(&spec.envs)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        command.output().await
    }
}

tokio::task_local! {
    static SCOPED_RUNNER: Arc<dyn CommandRunner>;
}

/// Run an async block with a custom `CommandRunner` installed for any
/// `CommandBuilder` that doesn't have its own runner explicitly set.
///
/// The override applies to the current task and its child tasks via the
/// `tokio::task_local!` mechanism. Used by tests to mock command execution
/// without polluting global state.
#[allow(dead_code)]
pub async fn with_runner<F, T>(runner: Arc<dyn CommandRunner>, fut: F) -> T
where
    F: std::future::Future<Output = T>,
{
    SCOPED_RUNNER.scope(runner, fut).await
}

fn current_runner() -> Arc<dyn CommandRunner> {
    SCOPED_RUNNER
        .try_with(Arc::clone)
        .unwrap_or_else(|_| Arc::new(TokioRunner))
}

/// Builder for executing external commands with common patterns
#[derive(Debug)]
pub struct CommandBuilder {
    spec: CommandSpec,
    context_msg: Option<String>,
    runner: Option<Arc<dyn CommandRunner>>,
}

impl CommandBuilder {
    /// Create a new command builder
    pub fn new<S: AsRef<OsStr>>(program: S) -> Self {
        Self {
            spec: CommandSpec {
                program: program.as_ref().to_os_string(),
                args: Vec::new(),
                envs: HashMap::new(),
            },
            context_msg: None,
            runner: None,
        }
    }

    /// Add a single argument
    #[allow(dead_code)]
    pub fn arg<S: AsRef<OsStr>>(mut self, arg: S) -> Self {
        self.spec.args.push(arg.as_ref().to_os_string());
        self
    }

    /// Add multiple arguments
    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        for arg in args {
            self.spec.args.push(arg.as_ref().to_os_string());
        }
        self
    }

    /// Set an environment variable
    pub fn env<K, V>(mut self, key: K, val: V) -> Self
    where
        K: AsRef<OsStr>,
        V: AsRef<OsStr>,
    {
        self.spec
            .envs
            .insert(key.as_ref().to_os_string(), val.as_ref().to_os_string());
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

    /// Use a specific `CommandRunner` for this invocation, overriding any
    /// scoped or default runner.
    #[allow(dead_code)]
    pub fn with_runner(mut self, runner: Arc<dyn CommandRunner>) -> Self {
        self.runner = Some(runner);
        self
    }

    /// Execute and return raw output
    pub async fn output(self) -> Result<CommandOutput> {
        let runner = self.runner.unwrap_or_else(current_runner);
        let res = runner.run(&self.spec).await;
        let output = match (res, &self.context_msg) {
            (Ok(o), _) => o,
            (Err(e), Some(ctx)) => return Err(anyhow::Error::new(e).context(ctx.clone())),
            (Err(e), None) => return Err(anyhow::Error::new(e)),
        };
        Ok(CommandOutput::from_output(output))
    }

    /// Execute and return stdout on success, error on failure
    pub async fn run(self) -> Result<String> {
        let ctx = self.context_msg.clone();
        let result = self.output().await?.into_result();
        match (result, ctx) {
            (Ok(s), _) => Ok(s),
            (Err(e), Some(c)) => Err(e.context(c)),
            (Err(e), None) => Err(e),
        }
    }

    /// Execute and ignore output (just check success)
    pub async fn run_silent(self) -> Result<()> {
        self.run().await.map(|_| ())
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

/// Construct a `std::process::Output` for use by mock runners.
///
/// Hidden behind `cfg(test)` (or `pub` for cross-module use) because the only
/// portable way to fabricate an `ExitStatus` is via platform-specific helpers.
#[cfg(test)]
pub(crate) fn make_output(success: bool, stdout: &str, stderr: &str) -> std::process::Output {
    let status: ExitStatus = make_status(success);
    std::process::Output {
        status,
        stdout: stdout.as_bytes().to_vec(),
        stderr: stderr.as_bytes().to_vec(),
    }
}

#[cfg(all(test, unix))]
fn make_status(success: bool) -> ExitStatus {
    use std::os::unix::process::ExitStatusExt;
    ExitStatus::from_raw(if success { 0 } else { 256 })
}

#[cfg(all(test, not(unix)))]
fn make_status(success: bool) -> ExitStatus {
    use std::os::windows::process::ExitStatusExt;
    ExitStatus::from_raw(if success { 0 } else { 1 })
}

// Keep the unused-import check happy on non-test builds.
#[allow(dead_code)]
fn _exit_status_marker(_: ExitStatus) {}

#[cfg(test)]
pub(crate) mod test_support {
    //! Helpers used by other modules' tests to mock command execution.

    use super::*;
    use std::sync::Mutex;

    /// Recording mock runner: stores every spec it receives and replies with
    /// canned output keyed by `program`. Falls back to a default success
    /// response if no specific output is configured.
    #[derive(Debug)]
    pub struct MockCommandRunner {
        pub responses: Mutex<HashMap<String, std::process::Output>>,
        pub default_response: Mutex<std::process::Output>,
        pub calls: Mutex<Vec<CommandSpec>>,
    }

    impl Default for MockCommandRunner {
        fn default() -> Self {
            Self::new()
        }
    }

    impl MockCommandRunner {
        pub fn new() -> Self {
            Self {
                responses: Mutex::new(HashMap::new()),
                default_response: Mutex::new(make_output(true, "", "")),
                calls: Mutex::new(Vec::new()),
            }
        }

        pub fn with_default(mut self, success: bool, stdout: &str, stderr: &str) -> Self {
            self.default_response = Mutex::new(make_output(success, stdout, stderr));
            self
        }

        pub fn respond(&self, program: &str, success: bool, stdout: &str, stderr: &str) {
            self.responses
                .lock()
                .unwrap()
                .insert(program.into(), make_output(success, stdout, stderr));
        }

        #[allow(dead_code)]
        pub fn call_count(&self) -> usize {
            self.calls.lock().unwrap().len()
        }

        pub fn calls_for(&self, program: &str) -> Vec<CommandSpec> {
            self.calls
                .lock()
                .unwrap()
                .iter()
                .filter(|s| s.program_str() == program)
                .cloned()
                .collect()
        }
    }

    fn clone_output(o: &std::process::Output) -> std::process::Output {
        std::process::Output {
            status: o.status,
            stdout: o.stdout.clone(),
            stderr: o.stderr.clone(),
        }
    }

    #[async_trait::async_trait]
    impl CommandRunner for MockCommandRunner {
        async fn run(&self, spec: &CommandSpec) -> std::io::Result<std::process::Output> {
            self.calls.lock().unwrap().push(spec.clone());
            let key = spec.program_str().into_owned();
            if let Some(o) = self.responses.lock().unwrap().get(&key) {
                return Ok(clone_output(o));
            }
            Ok(clone_output(&self.default_response.lock().unwrap()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::MockCommandRunner;
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

    #[tokio::test]
    async fn test_command_output_into_result_success() {
        let out = CommandOutput {
            stdout: "ok".into(),
            stderr: String::new(),
            success: true,
        };
        assert_eq!(out.into_result().unwrap(), "ok");
    }

    #[tokio::test]
    async fn test_command_output_into_result_failure() {
        let out = CommandOutput {
            stdout: String::new(),
            stderr: "boom".into(),
            success: false,
        };
        let err = out.into_result().unwrap_err();
        assert!(err.to_string().contains("boom"));
    }

    #[tokio::test]
    async fn test_explicit_runner_used() {
        let mock = Arc::new(MockCommandRunner::new().with_default(true, "hello\n", ""));
        let stdout = CommandBuilder::new("kubectl")
            .args(["get", "nodes"])
            .with_runner(mock.clone())
            .run()
            .await
            .unwrap();
        assert_eq!(stdout, "hello\n");
        let calls = mock.calls_for("kubectl");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].args_str(), vec!["get", "nodes"]);
    }

    #[tokio::test]
    async fn test_scoped_runner_intercepts_calls() {
        let mock = Arc::new(MockCommandRunner::new());
        mock.respond("talosctl", true, "v1.11.4", "");

        let captured = mock.clone();
        with_runner(mock.clone(), async move {
            let out = CommandBuilder::new("talosctl")
                .arg("version")
                .run()
                .await
                .unwrap();
            assert_eq!(out, "v1.11.4");
            assert_eq!(captured.calls_for("talosctl").len(), 1);
        })
        .await;
    }

    #[tokio::test]
    async fn test_run_failure_attaches_context() {
        let mock = Arc::new(MockCommandRunner::new().with_default(false, "", "kubectl exploded"));
        let err = CommandBuilder::new("kubectl")
            .arg("apply")
            .context("applying manifest")
            .with_runner(mock)
            .run()
            .await
            .unwrap_err();
        let s = format!("{err:#}");
        assert!(s.contains("applying manifest"), "{s}");
        assert!(s.contains("kubectl exploded"), "{s}");
    }

    #[tokio::test]
    async fn test_run_silent_returns_unit() {
        let mock = Arc::new(MockCommandRunner::new());
        with_runner(mock, async {
            CommandBuilder::new("kubectl")
                .arg("apply")
                .run_silent()
                .await
                .unwrap();
        })
        .await;
    }

    #[tokio::test]
    async fn test_check_tool_installed_success() {
        let mock = Arc::new(MockCommandRunner::new().with_default(true, "v1.0", ""));
        with_runner(mock, async {
            check_tool_installed("kubectl", &["version"], "https://example.com")
                .await
                .unwrap();
        })
        .await;
    }

    #[tokio::test]
    async fn test_check_tool_installed_missing() {
        let mock = Arc::new(MockCommandRunner::new().with_default(false, "", "not found"));
        with_runner(mock, async {
            let err = check_tool_installed("missing-tool", &["--version"], "https://example.com")
                .await
                .unwrap_err();
            assert!(err.to_string().contains("missing-tool"));
            assert!(err.to_string().contains("https://example.com"));
        })
        .await;
    }

    #[test]
    fn test_command_spec_helpers() {
        let cb = CommandBuilder::new("kubectl")
            .args(["get", "pods"])
            .env("FOO", "BAR");
        assert_eq!(cb.spec.program_str(), "kubectl");
        assert_eq!(cb.spec.args_str(), vec!["get", "pods"]);
        assert_eq!(
            cb.spec.envs.get(OsStr::new("FOO")).unwrap(),
            OsStr::new("BAR")
        );
    }

    #[tokio::test]
    async fn test_kubeconfig_sets_env() {
        let mock = Arc::new(MockCommandRunner::new());
        let p = Path::new("/tmp/xx-kubeconfig");
        CommandBuilder::new("kubectl")
            .kubeconfig(p)
            .with_runner(mock.clone())
            .run_silent()
            .await
            .unwrap();
        let calls = mock.calls_for("kubectl");
        assert_eq!(
            calls[0].envs.get(OsStr::new("KUBECONFIG")).unwrap(),
            OsStr::new("/tmp/xx-kubeconfig")
        );
    }
}
