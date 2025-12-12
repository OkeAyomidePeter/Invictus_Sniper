use crate::config::Config;
use crate::enrichment::EnrichedToken;
use anyhow::{Context, Result};
use log::{error, info, warn};
use reqwest::Client;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tokio::sync::Mutex;

/// Token waiting in the watchlist
#[derive(Debug, Clone)]
pub struct WatchlistToken {
    pub mint: String,
    pub graduation_price: f64,
    pub target_price: f64,
    pub added_at: Instant,
    pub initial_data: EnrichedToken,
    pub initial_volume_5m: f64,
    pub initial_holders: usize,
    pub volume_samples: Vec<(Instant, f64)>,  // Track volume over time
    pub holder_samples: Vec<(Instant, usize)>, // Track holder changes
}

/// Signal to execute a buy from the watchlist
#[derive(Debug, Clone)]
pub struct BuySignal {
    pub token: EnrichedToken,
    pub current_price: f64,
    pub volume_5m: f64,
}

pub struct Watchlist {
    tokens: Arc<Mutex<HashMap<String, WatchlistToken>>>,
    client: Client,
    config: Config,
    buy_tx: mpsc::Sender<BuySignal>,
}

impl Watchlist {
    pub fn new(config: &Config, buy_tx: mpsc::Sender<BuySignal>) -> Self {
        Self {
            tokens: Arc::new(Mutex::new(HashMap::new())),
            client: Client::new(),
            config: config.clone(),
            buy_tx,
        }
    }

    /// Add a token to the watchlist
    pub async fn add_token(&self, token: EnrichedToken) {
        let mut tokens = self.tokens.lock().await;
        
        // Calculate target price
        // If we don't have a price yet (likely), we might need to fetch it or wait for the first check.
        // For now, let's assume we can get a rough price from liquidity or set a flag to fetch initial price.
        // Actually, `EnrichedToken` has `price_sol`.
        
        let initial_price = token.price_sol.unwrap_or(0.0);
        
        if initial_price == 0.0 {
            warn!("⚠️ Cannot add {} to watchlist: No initial price", token.mint);
            return;
        }

        let target_price = initial_price * (1.0 - (self.config.dip_entry_pct / 100.0));
        
        // Get initial stats for validation
        // Note: volume_5m_usd and holder_count fields don't exist yet in EnrichedToken
        // Using fallbacks until enrichment is updated
        let initial_volume = 0.0; // Volume not available in EnrichedToken yet
        let initial_holders = token.holders.as_ref()
            .and_then(|h| h.unique_holders)
            .unwrap_or(0) as usize;
        
        info!("👀 Added to Watchlist: {} | Entry: {:.9} SOL | Target: {:.9} SOL (-{}%) | Vol: ${:.0} | Holders: {}", 
            token.mint, initial_price, target_price, self.config.dip_entry_pct, initial_volume, initial_holders);

        tokens.insert(token.mint.clone(), WatchlistToken {
            mint: token.mint.clone(),
            graduation_price: initial_price,
            target_price,
            added_at: Instant::now(),
            initial_data: token,
            initial_volume_5m: initial_volume,
            initial_holders,
            volume_samples: Vec::new(),
            holder_samples: Vec::new(),
        });
    }

    /// Start the monitoring loop
    pub async fn start_monitoring(&self) {
        let tokens = self.tokens.clone();
        let client = self.client.clone();
        let config = self.config.clone();
        let buy_tx = self.buy_tx.clone();

        tokio::spawn(async move {
            info!("🕵️ Watchlist monitoring started");
            
            loop {
                // 1. Get snapshot of tokens to check
                let tokens_to_check: Vec<WatchlistToken> = {
                    let guard = tokens.lock().await;
                    guard.values().cloned().collect()
                };

                if tokens_to_check.is_empty() {
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    continue;
                }

                // 2. Check each token
                for token in tokens_to_check {
                    // Check timeout
                    if token.added_at.elapsed().as_secs() > config.watchlist_timeout_seconds {
                        info!("🗑️ Removing {} from watchlist: Timeout ({}s)", token.mint, config.watchlist_timeout_seconds);
                        let mut guard = tokens.lock().await;
                        guard.remove(&token.mint);
                        continue;
                    }

                    // Check DexScreener
                    match fetch_dexscreener_data(&client, &token.mint).await {
                        Ok((price_usd, price_sol, volume_5m)) => {
                            // Update volume samples (for trend analysis)
                            if config.volume_trend_enabled {
                                let mut guard = tokens.lock().await;
                                if let Some(tracked_token) = guard.get_mut(&token.mint) {
                                    tracked_token.volume_samples.push((Instant::now(), volume_5m));
                                    
                                    // Keep only recent samples (last N samples)
                                    let max_samples = config.volume_samples_required + 2;
                                    if tracked_token.volume_samples.len() > max_samples {
                                        tracked_token.volume_samples.drain(0..1);
                                    }
                                }
                                drop(guard);
                            }

                            // CHECK 1: Price Dip
                            if price_sol <= token.target_price {
                                // CHECK 2: Enhanced Volume Validation
                                let volume_valid = if config.volume_trend_enabled {
                                    validate_volume_with_trend(&token, volume_5m, &config)
                                } else {
                                    // Simple check (backward compatible)
                                    volume_5m >= config.min_volume_usd_5m
                                };

                                if volume_valid {
                                    info!("🚀 DIP BUY TRIGGERED: {} | Price: {:.9} | Vol: ${:.0}", 
                                        token.mint, price_sol, volume_5m);
                                    
                                    // LOGGING: Dip Trigger
                                    let dip_pct = (token.graduation_price - price_sol) / token.graduation_price * 100.0;
                                    crate::trade_logger::log_dip_trigger(&token.mint, dip_pct, volume_5m);

                                    // Send Buy Signal
                                    let signal = BuySignal {
                                        token: token.initial_data.clone(),
                                        current_price: price_sol,
                                        volume_5m,
                                    };
                                    
                                    if let Err(e) = buy_tx.send(signal).await {
                                        error!("Failed to send buy signal: {}", e);
                                    }

                                    // Remove from watchlist
                                    let mut guard = tokens.lock().await;
                                    guard.remove(&token.mint);
                                } else {
                                    // Price is good, but volume validation failed
                                    if config.volume_trend_enabled && token.volume_samples.len() < config.volume_samples_required {
                                        // Still collecting samples, wait
                                    } else {
                                        warn!("📉 Dip hit for {} but volume validation failed (Vol: ${:.0})", 
                                            token.mint, volume_5m);
                                        crate::trade_logger::log_token_rejected(&token.mint, 0.0, "Dip hit but volume too low");
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            warn!("Failed to fetch DexScreener for {}: {}", token.mint, e);
                        }
                    }
                    
                    // Rate limit protection (DexScreener allows ~300 req/min, so be gentle)
                    tokio::time::sleep(Duration::from_millis(500)).await; 
                }

                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        });
    }
}

/// Fetch Price and Volume from DexScreener
/// Returns (Price USD, Price SOL, Volume 5m USD)
async fn fetch_dexscreener_data(client: &Client, mint: &str) -> Result<(f64, f64, f64)> {
    let url = format!("https://api.dexscreener.com/latest/dex/tokens/{}", mint);
    
    let response = client.get(&url)
        .timeout(Duration::from_secs(5))
        .send()
        .await?
        .json::<serde_json::Value>()
        .await?;

    let pairs = response.get("pairs").and_then(|v| v.as_array()).context("No pairs found")?;
    
    // Find the best pair (usually the first one, or filter for Raydium)
    let pair = pairs.first().context("No pairs in list")?;
    
    let price_usd = pair.get("priceUsd")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(0.0);
        
    let price_sol = pair.get("priceNative")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(0.0);

    let volume_5m = pair.get("volume")
        .and_then(|v| v.get("m5"))
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);

    Ok((price_usd, price_sol, volume_5m))
}

/// Validate volume with trend analysis
/// Returns true if volume is acceptable (above threshold AND increasing/stable)
fn validate_volume_with_trend(token: &WatchlistToken, current_volume: f64, config: &Config) -> bool {
    // CHECK 1: Must meet minimum volume threshold
    if current_volume < config.min_volume_usd_5m {
        return false;
    }
    
    // CHECK 2: Need enough samples for trend analysis
    if token.volume_samples.len() < config.volume_samples_required {
        // Not enough samples yet, wait
        return false;
    }
    
    // CHECK 3: Calculate volume trend
    let samples = &token.volume_samples;
    let recent_samples: Vec<f64> = samples.iter()
        .rev()
        .take(config.volume_samples_required)
        .map(|(_, vol)| *vol)
        .collect();
    
    if recent_samples.len() < 2 {
        return false;
    }
    
    // Calculate average of first half vs second half
    let mid = recent_samples.len() / 2;
    let first_half_avg: f64 = recent_samples[..mid].iter().sum::<f64>() / mid as f64;
    let second_half_avg: f64 = recent_samples[mid..].iter().sum::<f64>() / (recent_samples.len() - mid) as f64;
    
    // Volume is INCREASING if second half > first half (or within 10% - stable)
    let volume_trend_ok = second_half_avg >= first_half_avg * 0.9;
    
    if !volume_trend_ok {
        info!("📉 Volume declining for {}: ${:.0} → ${:.0}", 
            token.mint, first_half_avg, second_half_avg);
        return false;
    }
    
    // All checks passed - volume is good and trending up (or stable)
    let trend_pct = if first_half_avg > 0.0 {
        ((second_half_avg - first_half_avg) / first_half_avg * 100.0)
    } else {
        0.0
    };

    info!("✅ Volume validation passed for {}: ${:.0} (trend: {:.1}%)", 
        token.mint, current_volume, trend_pct);
    
    true
}
