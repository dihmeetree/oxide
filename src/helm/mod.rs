/// Helm package manager operations
use anyhow::Result;

/// Helm package manager
pub struct Helm;

impl Helm {
    /// Check if helm is installed
    pub async fn check_installed() -> Result<()> {
        crate::utils::command::check_tool_installed(
            "helm",
            &["version"],
            "https://helm.sh/docs/intro/install/",
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use crate::utils::command::test_support::MockCommandRunner;
    use crate::utils::command::with_runner;
    use std::sync::Arc;

    #[tokio::test]
    async fn test_check_installed_success() {
        let mock = Arc::new(MockCommandRunner::new());
        mock.respond("helm", true, "version.BuildInfo{Version:\"v3.14.0\"}", "");

        let result =
            with_runner(mock.clone(), async { super::Helm::check_installed().await }).await;

        assert!(result.is_ok());
        let calls = mock.calls_for("helm");
        assert!(!calls.is_empty());
        let args: Vec<_> = calls[0].args_str();
        let args_str: Vec<&str> = args.iter().map(|s| s.as_ref()).collect();
        assert!(args_str.contains(&"version"));
    }

    #[tokio::test]
    async fn test_check_installed_missing() {
        let mock = Arc::new(MockCommandRunner::new());
        mock.respond("helm", false, "", "helm: command not found");

        let result =
            with_runner(mock.clone(), async { super::Helm::check_installed().await }).await;

        assert!(result.is_err());
        let msg = format!("{}", result.unwrap_err());
        assert!(
            msg.contains("helm") || msg.contains("not found") || msg.contains("install"),
            "expected useful error message, got: {}",
            msg
        );
    }
}
