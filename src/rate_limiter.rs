use log::debug;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::time::{sleep, Duration, Instant};

/// Token bucket rate limiter for API calls
/// Uses the token bucket algorithm to enforce rate limits
#[derive(Clone)]
pub struct RateLimiter {
    tokens: Arc<Mutex<f64>>,
    max_tokens: f64,
    refill_rate: f64, // tokens per second
    last_refill: Arc<Mutex<Instant>>,
    name: String,
}

impl RateLimiter {
    /// Create a new rate limiter
    /// 
    /// # Arguments
    /// * `max_requests_per_second` - Maximum requests allowed per second
    /// * `name` - Name for logging purposes (e.g., "Helius", "Jupiter")
    pub fn new(max_requests_per_second: f64, name: impl Into<String>) -> Self {
        Self {
            tokens: Arc::new(Mutex::new(max_requests_per_second)),
            max_tokens: max_requests_per_second,
            refill_rate: max_requests_per_second,
            last_refill: Arc::new(Mutex::new(Instant::now())),
            name: name.into(),
        }
    }

    /// Acquire a token, blocking until one is available
    /// This should be called before making an API request
    pub async fn acquire(&self) {
        loop {
            self.refill_tokens().await;
            
            let mut tokens = self.tokens.lock().await;
            if *tokens >= 1.0 {
                *tokens -= 1.0;
                debug!("{} rate limiter: token acquired ({:.2} remaining)", self.name, *tokens);
                break;
            }
            
            // No tokens available, wait for refill
            drop(tokens);
            sleep(Duration::from_millis(10)).await;
        }
    }

    /// Try to acquire a token without blocking
    /// Returns true if token was acquired, false otherwise
    pub async fn try_acquire(&self) -> bool {
        self.refill_tokens().await;
        
        let mut tokens = self.tokens.lock().await;
        if *tokens >= 1.0 {
            *tokens -= 1.0;
            debug!("{} rate limiter: token acquired ({:.2} remaining)", self.name, *tokens);
            true
        } else {
            false
        }
    }

    /// Refill tokens based on time elapsed since last refill
    async fn refill_tokens(&self) {
        let now = Instant::now();
        let mut last_refill = self.last_refill.lock().await;
        let elapsed = now.duration_since(*last_refill).as_secs_f64();
        
        if elapsed > 0.0 {
            let mut tokens = self.tokens.lock().await;
            let new_tokens = *tokens + elapsed * self.refill_rate;
            *tokens = new_tokens.min(self.max_tokens);
            *last_refill = now;
            
            debug!("{} rate limiter: refilled to {:.2} tokens", self.name, *tokens);
        }
    }

    /// Get current number of available tokens (for monitoring)
    pub async fn available_tokens(&self) -> f64 {
        self.refill_tokens().await;
        let tokens = self.tokens.lock().await;
        *tokens
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_rate_limiter_allows_within_limit() {
        let limiter = RateLimiter::new(10.0, "test");
        
        let start = Instant::now();
        for _ in 0..10 {
            limiter.acquire().await;
        }
        let elapsed = start.elapsed();
        
        // Should complete quickly (within 200ms buffer)
        assert!(elapsed < Duration::from_millis(200));
    }

    #[tokio::test]
    async fn test_rate_limiter_blocks_above_limit() {
        let limiter = RateLimiter::new(5.0, "test");
        
        let start = Instant::now();
        for _ in 0..10 {
            limiter.acquire().await;
        }
        let elapsed = start.elapsed();
        
        // Should take approximately 2 seconds (10 requests / 5 RPS = 2s)
        // Allow 1.5s minimum to account for timing variations
        assert!(elapsed >= Duration::from_millis(1500));
        assert!(elapsed < Duration::from_secs(3));
    }

    #[tokio::test]
    async fn test_try_acquire() {
        let limiter = RateLimiter::new(2.0, "test");
        
        // First two should succeed
        assert!(limiter.try_acquire().await);
        assert!(limiter.try_acquire().await);
        
        // Third should fail (no tokens)
        assert!(!limiter.try_acquire().await);
        
        // Wait for refill
        sleep(Duration::from_millis(600)).await;
        
        // Should succeed after refill
        assert!(limiter.try_acquire().await);
    }

    #[tokio::test]
    async fn test_available_tokens() {
        let limiter = RateLimiter::new(10.0, "test");
        
        // Should start with max tokens
        let tokens = limiter.available_tokens().await;
        assert!((tokens - 10.0).abs() < 0.1);
        
        // Acquire some tokens
        limiter.acquire().await;
        limiter.acquire().await;
        
        let tokens = limiter.available_tokens().await;
        assert!((tokens - 8.0).abs() < 0.1);
    }
}
