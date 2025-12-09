//! Health monitoring module for tracking system component health
//! 
//! Tracks timestamps of last activity for each component and provides
//! health status reporting for external monitoring.

use log::{info, warn};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;

/// Component health tracker using atomic timestamps
pub struct HealthChecker {
    /// Timestamp of last WebSocket event received
    last_websocket_event: AtomicI64,
    /// Timestamp of last token enrichment completed
    last_enrichment: AtomicI64,
    /// Timestamp of last position check performed
    last_position_check: AtomicI64,
    /// Timestamp of last trade execution attempt
    last_trade_attempt: AtomicI64,
    /// System start time
    start_time: i64,
}

impl HealthChecker {
    /// Create a new health checker
    pub fn new() -> Self {
        let now = chrono::Utc::now().timestamp();
        Self {
            last_websocket_event: AtomicI64::new(now),
            last_enrichment: AtomicI64::new(now),
            last_position_check: AtomicI64::new(now),
            last_trade_attempt: AtomicI64::new(0), // No trade yet
            start_time: now,
        }
    }

    /// Record a WebSocket event was received
    pub fn record_websocket_event(&self) {
        self.last_websocket_event.store(chrono::Utc::now().timestamp(), Ordering::Relaxed);
    }

    /// Record a token enrichment was completed
    pub fn record_enrichment(&self) {
        self.last_enrichment.store(chrono::Utc::now().timestamp(), Ordering::Relaxed);
    }

    /// Record a position check was performed
    pub fn record_position_check(&self) {
        self.last_position_check.store(chrono::Utc::now().timestamp(), Ordering::Relaxed);
    }

    /// Record a trade execution was attempted
    pub fn record_trade_attempt(&self) {
        self.last_trade_attempt.store(chrono::Utc::now().timestamp(), Ordering::Relaxed);
    }

    /// Check if any components are stale (not updated within threshold)
    /// Returns a list of component names that are stale
    pub fn check_stale(&self, threshold_secs: i64) -> Vec<String> {
        let now = chrono::Utc::now().timestamp();
        let mut stale = Vec::new();

        let ws_age = now - self.last_websocket_event.load(Ordering::Relaxed);
        if ws_age > threshold_secs {
            stale.push(format!("WebSocket ({}s)", ws_age));
        }

        // Only check enrichment if we've been running long enough
        let uptime = now - self.start_time;
        if uptime > threshold_secs {
            let enrich_age = now - self.last_enrichment.load(Ordering::Relaxed);
            if enrich_age > threshold_secs * 2 {
                // More lenient for enrichment
                stale.push(format!("Enrichment ({}s)", enrich_age));
            }
        }

        stale
    }

    /// Check overall system health
    pub fn is_healthy(&self, threshold_secs: i64) -> bool {
        self.check_stale(threshold_secs).is_empty()
    }

    /// Get uptime in seconds
    pub fn uptime(&self) -> i64 {
        chrono::Utc::now().timestamp() - self.start_time
    }

    /// Get health status summary
    pub fn status_summary(&self) -> HealthStatus {
        let now = chrono::Utc::now().timestamp();
        HealthStatus {
            uptime_seconds: now - self.start_time,
            last_websocket_secs_ago: now - self.last_websocket_event.load(Ordering::Relaxed),
            last_enrichment_secs_ago: now - self.last_enrichment.load(Ordering::Relaxed),
            last_position_check_secs_ago: now - self.last_position_check.load(Ordering::Relaxed),
            last_trade_secs_ago: {
                let last = self.last_trade_attempt.load(Ordering::Relaxed);
                if last == 0 { None } else { Some(now - last) }
            },
        }
    }
}

impl Default for HealthChecker {
    fn default() -> Self {
        Self::new()
    }
}

/// Health status snapshot
#[derive(Debug, Clone)]
pub struct HealthStatus {
    pub uptime_seconds: i64,
    pub last_websocket_secs_ago: i64,
    pub last_enrichment_secs_ago: i64,
    pub last_position_check_secs_ago: i64,
    pub last_trade_secs_ago: Option<i64>,
}

impl std::fmt::Display for HealthStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Uptime: {}s | WS: {}s ago | Enrich: {}s ago | PosCheck: {}s ago | Trade: {}",
            self.uptime_seconds,
            self.last_websocket_secs_ago,
            self.last_enrichment_secs_ago,
            self.last_position_check_secs_ago,
            self.last_trade_secs_ago.map(|s| format!("{}s ago", s)).unwrap_or_else(|| "never".to_string())
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread::sleep;
    use std::time::Duration;

    #[test]
    fn test_health_checker_creation() {
        let hc = HealthChecker::new();
        assert!(hc.is_healthy(60));
        assert!(hc.uptime() >= 0);
    }

    #[test]
    fn test_record_events() {
        let hc = HealthChecker::new();
        
        hc.record_websocket_event();
        hc.record_enrichment();
        hc.record_position_check();
        hc.record_trade_attempt();
        
        let status = hc.status_summary();
        assert!(status.last_websocket_secs_ago <= 1);
        assert!(status.last_enrichment_secs_ago <= 1);
        assert!(status.last_trade_secs_ago.unwrap() <= 1);
    }

    #[test]
    fn test_stale_detection() {
        let hc = HealthChecker::new();
        
        // Should not be stale immediately
        assert!(hc.check_stale(2).is_empty());
        
        // Wait for 3 seconds
        sleep(Duration::from_secs(3));
        
        // Now should be stale with 2s threshold
        let stale = hc.check_stale(2);
        assert!(!stale.is_empty());
    }
}
