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
