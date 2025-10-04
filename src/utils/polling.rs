/// Polling utilities for waiting on conditions with timeout
use anyhow::Result;
use std::future::Future;
use std::time::{Duration, Instant};
use tracing::info;

/// Configuration for polling operations
pub struct PollingConfig {
    pub timeout: Duration,
    pub interval: Duration,
    pub description: String,
}

impl PollingConfig {
    /// Create a new polling configuration
    pub fn new(timeout_secs: u64, interval_secs: u64, description: impl Into<String>) -> Self {
        Self {
            timeout: Duration::from_secs(timeout_secs),
            interval: Duration::from_secs(interval_secs),
            description: description.into(),
        }
    }

    /// Poll until condition is met or timeout
    ///
    /// The condition function should return:
    /// - Ok(Some(T)) when condition is met (returns T)
    /// - Ok(None) when condition is not yet met (continues polling)
    /// - Err(e) when an error occurs (stops polling and returns error)
    pub async fn poll<F, Fut, T>(&self, condition: F) -> Result<T>
    where
        F: Fn() -> Fut,
        Fut: Future<Output = Result<Option<T>>>,
    {
        info!("{}...", self.description);

        let start = Instant::now();

        loop {
            // Check condition
            match condition().await {
                Ok(Some(value)) => {
                    info!("✓ {}", self.description);
                    return Ok(value);
                }
                Ok(None) => {
                    // Continue polling
                }
                Err(e) => {
                    return Err(e);
                }
            }

            // Check timeout
            if start.elapsed() > self.timeout {
                anyhow::bail!(
                    "Timeout after {} seconds: {}",
                    self.timeout.as_secs(),
                    self.description
                );
            }

            // Wait before next attempt
            tokio::time::sleep(self.interval).await;
        }
    }

    /// Poll until condition returns Ok(true) or timeout
    ///
    /// Simplified version for boolean conditions
    pub async fn poll_until<F, Fut>(&self, condition: F) -> Result<()>
    where
        F: Fn() -> Fut,
        Fut: Future<Output = Result<bool>>,
    {
        self.poll(|| async {
            match condition().await {
                Ok(true) => Ok(Some(())),
                Ok(false) => Ok(None),
                Err(e) => Err(e),
            }
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    #[tokio::test]
    async fn test_polling_success() {
        let counter = Arc::new(AtomicU32::new(0));
        let counter_clone = counter.clone();

        let config = PollingConfig::new(10, 1, "test polling");

        let result = config
            .poll(|| {
                let c = counter_clone.clone();
                async move {
                    let val = c.fetch_add(1, Ordering::SeqCst);
                    if val >= 2 {
                        Ok(Some(val))
                    } else {
                        Ok(None)
                    }
                }
            })
            .await;

        assert!(result.is_ok());
        assert!(counter.load(Ordering::SeqCst) >= 3);
    }

    #[tokio::test]
    async fn test_polling_timeout() {
        let config = PollingConfig::new(2, 1, "test timeout");

        let result = config
            .poll(|| async { Ok::<Option<()>, anyhow::Error>(None) })
            .await;

        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("Timeout"));
    }

    #[tokio::test]
    async fn test_poll_until_success() {
        let counter = Arc::new(AtomicU32::new(0));
        let counter_clone = counter.clone();

        let config = PollingConfig::new(10, 1, "test poll_until");

        let result = config
            .poll_until(|| {
                let c = counter_clone.clone();
                async move {
                    let val = c.fetch_add(1, Ordering::SeqCst);
                    Ok(val >= 2)
                }
            })
            .await;

        assert!(result.is_ok());
    }
}
