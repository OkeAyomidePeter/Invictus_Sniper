use anyhow::{Context, Result};
use log::{info, warn};
use std::future::Future;
use std::pin::Pin;
use tokio::time::{sleep, Duration};

/// Configuration for retry logic
#[derive(Clone, Debug)]
pub struct RetryConfig {
    pub max_attempts: usize,
    pub initial_delay_ms: u64,
    pub max_delay_ms: u64,
    pub backoff_multiplier: f64,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            initial_delay_ms: 500,
            max_delay_ms: 5000,
            backoff_multiplier: 2.0,
        }
    }
}

/// Retry an async operation with exponential backoff
/// 
/// # Arguments
/// * `operation_name` - Name for logging
/// * `operation` - Async function to retry
/// * `config` - Retry configuration
/// * `is_retryable` - Function to determine if error is retryable
pub async fn retry_with_backoff<F, Fut, T, E>(
    operation_name: &str,
    mut operation: F,
    config: &RetryConfig,
    is_retryable: fn(&E) -> bool,
) -> Result<T, E>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, E>>,
{
    let mut delay = config.initial_delay_ms;
    let mut last_error = None;
    
    for attempt in 1..=config.max_attempts {
        match operation().await {
            Ok(result) => {
                if attempt > 1 {
                    info!("{}: succeeded on attempt {}/{}", operation_name, attempt, config.max_attempts);
                }
                return Ok(result);
            }
            Err(e) => {
                let retryable = is_retryable(&e);
                
                if retryable && attempt < config.max_attempts {
                    warn!(
                        "{}: attempt {}/{} failed (retryable), retrying in {}ms",
                        operation_name, attempt, config.max_attempts, delay
                    );
                    
                    sleep(Duration::from_millis(delay)).await;
                    
                    // Exponential backoff with cap
                    delay = ((delay as f64 * config.backoff_multiplier) as u64).min(config.max_delay_ms);
                    last_error = Some(e);
                } else {
                    if retryable {
                        warn!(
                            "{}: attempt {}/{} failed, max retries reached",
                            operation_name, attempt, config.max_attempts
                        );
                    } else {
                        warn!(
                            "{}: attempt {}/{} failed (non-retryable), giving up",
                            operation_name, attempt, config.max_attempts
                        );
                    }
                    return Err(e);
                }
            }
        }
    }
    
    // This should never be reached, but compiler needs it
    Err(last_error.unwrap())
}

/// Determine if an anyhow error is network-related and retryable
pub fn is_network_error(error: &anyhow::Error) -> bool {
    let error_msg = error.to_string().to_lowercase();
    
    // Common network error patterns
    error_msg.contains("connection") ||
    error_msg.contains("timeout") ||
    error_msg.contains("timed out") ||
    error_msg.contains("network") ||
    error_msg.contains("dns") ||
    error_msg.contains("refused") ||
    error_msg.contains("reset") ||
    error_msg.contains("broken pipe") ||
    error_msg.contains("temporary failure")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test]
    async fn test_retry_succeeds_on_first_attempt() {
        let config = RetryConfig::default();
        let attempt_count = Arc::new(AtomicUsize::new(0));
        let count_clone = attempt_count.clone();
        
        let result = retry_with_backoff(
            "test_operation",
            || async {
                count_clone.fetch_add(1, Ordering::SeqCst);
                Ok::<_, anyhow::Error>(42)
            },
            &config,
            is_network_error,
        ).await;
        
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 42);
        assert_eq!(attempt_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_retry_succeeds_after_failures() {
        let config = RetryConfig {
            max_attempts: 3,
            initial_delay_ms: 10,
            max_delay_ms: 100,
            backoff_multiplier: 2.0,
        };
        
        let attempt_count = Arc::new(AtomicUsize::new(0));
        let count_clone = attempt_count.clone();
        
        let result = retry_with_backoff(
            "test_operation",
            || async {
                let count = count_clone.fetch_add(1, Ordering::SeqCst) + 1;
                if count < 3 {
                    Err(anyhow::anyhow!("network timeout"))
                } else {
                    Ok(42)
                }
            },
            &config,
            is_network_error,
        ).await;
        
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 42);
        assert_eq!(attempt_count.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn test_retry_fails_after_max_attempts() {
        let config = RetryConfig {
            max_attempts: 3,
            initial_delay_ms: 10,
            max_delay_ms: 100,
            backoff_multiplier: 2.0,
        };
        
        let attempt_count = Arc::new(AtomicUsize::new(0));
        let count_clone = attempt_count.clone();
        
        let result = retry_with_backoff(
            "test_operation",
            || async {
                count_clone.fetch_add(1, Ordering::SeqCst);
                Err::<i32, _>(anyhow::anyhow!("connection refused"))
            },
            &config,
            is_network_error,
        ).await;
        
        assert!(result.is_err());
        assert_eq!(attempt_count.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn test_non_retryable_error_fails_immediately() {
        let config = RetryConfig::default();
        let attempt_count = Arc::new(AtomicUsize::new(0));
        let count_clone = attempt_count.clone();
        
        let result = retry_with_backoff(
            "test_operation",
            || async {
                count_clone.fetch_add(1, Ordering::SeqCst);
                Err::<i32, _>(anyhow::anyhow!("invalid parameter"))
            },
            &config,
            is_network_error,
        ).await;
        
        assert!(result.is_err());
        assert_eq!(attempt_count.load(Ordering::SeqCst), 1); // No retries for non-network error
    }

    #[test]
    fn test_is_network_error() {
        assert!(is_network_error(&anyhow::anyhow!("connection timeout")));
        assert!(is_network_error(&anyhow::anyhow!("Network error occurred")));
        assert!(is_network_error(&anyhow::anyhow!("DNS lookup failed")));
        assert!(is_network_error(&anyhow::anyhow!("Connection refused")));
        
        assert!(!is_network_error(&anyhow::anyhow!("invalid parameter")));
        assert!(!is_network_error(&anyhow::anyhow!("Not found")));
    }
}
