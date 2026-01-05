use crate::config::Config;
use anyhow::{Context, Result};
use log::{error, info, warn};
use reqwest::Client;
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::Mutex;
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tokio::time::sleep;
use crate::db::Database;
use solana_sdk::program_pack::Pack;
use spl_token::state::Account as TokenAccount;
use crate::rate_limiter::RateLimiter;
use crate::moralis_client::MoralisClient;
const SOL_MINT: &str = "So11111111111111111111111111111111111111112";

/// Represents an active trading position
#[derive(Debug, Clone)]
pub struct Position {
    pub mint: String,
    pub entry_price_sol_per_token: f64,
    pub entry_time: Instant,
    pub amount_token_raw: u64,
    pub amount_sol_invested: u64,
    pub decimals: u8,
    pub highest_price_reached: f64,     // For trailing stop
    pub partial_exit_executed: bool,    // Track if partial exit done
    pub remaining_amount_pct: f64,      // Track remaining position size
    pub timeout_extensions: u32,        // Count timeout extensions
    pub entry_1m_move: f64,             // Momentum at entry for adaptive exits
    
    // Analytics Snapshots
    pub liquidity_usd: f64,
    pub top_10_pct: f64,
    pub holder_count: u64,
    pub socials: u32,
}

/// Reason for triggering a sell
#[derive(Debug, Clone, PartialEq)]
pub enum SellTrigger {
    ProfitTarget(f64),       // Full profit target
    PartialProfit(f64),      // Partial exit (sell portion)
    TrailingStopLoss(f64),   // Trailing stop triggered
    StopLoss(f64),           // Fixed stop loss
    Timeout,
    ExtendedTimeout(u32),    // Extended timeout
}

impl std::fmt::Display for SellTrigger {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SellTrigger::ProfitTarget(pct) => write!(f, "PROFIT_TARGET (+{:.2}%)", pct),
            SellTrigger::PartialProfit(pct) => write!(f, "PARTIAL_PROFIT (+{:.2}%)", pct),
            SellTrigger::TrailingStopLoss(pct) => write!(f, "TRAILING_STOP ({:.2}%)", pct),
            SellTrigger::StopLoss(pct) => write!(f, "STOP_LOSS ({:.2}%)", pct),
            SellTrigger::Timeout => write!(f, "TIMEOUT"),
            SellTrigger::ExtendedTimeout(count) => write!(f, "TIMEOUT (Extended x{})", count),
        }
    }
}

/// Signal to execute a sell
#[derive(Debug, Clone)]
pub struct SellSignal {
    pub position: Position,
    pub trigger: SellTrigger,
    pub current_price_sol_per_token: f64,
    pub pnl_percentage: f64,
}



/// Position tracker manages active positions and monitors for sell conditions
#[derive(Clone)]
pub struct PositionTracker {
    client: Client,
    config: Config,
    db: Arc<Database>,
    active_positions: Arc<Mutex<HashMap<String, Position>>>,
    profit_target_pct: f64,
    stop_loss_pct: f64,
    timeout_seconds: u64,
    price_check_interval_ms: u64,
    rpc_url: String,
    birdeye_limiter: Arc<RateLimiter>,
    moralis_limiter: Arc<RateLimiter>,
    moralis_client: Arc<MoralisClient>,
}

impl PositionTracker {
    pub fn new(config: &Config, db: Arc<Database>) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_millis(config.jupiter_api_timeout_ms))
            .build()
            .unwrap_or_else(|_| Client::new());

        let birdeye_limiter = Arc::new(RateLimiter::new(
            config.birdeye_max_requests_per_second,
            "BirdeyePositionTracker",
        ));

        let moralis_limiter = Arc::new(RateLimiter::new(
            config.moralis_max_requests_per_second,
            "MoralisPositionTracker",
        ));

        let moralis_client = Arc::new(MoralisClient::new(
            config.moralis_api_key.clone(),
            "mainnet".to_string(),
        ));

        Self {
            client,
            config: config.clone(),
            db,
            active_positions: Arc::new(Mutex::new(HashMap::new())),
            profit_target_pct: config.auto_sell_profit_target_pct,
            stop_loss_pct: config.auto_sell_stop_loss_pct,
            timeout_seconds: config.auto_sell_timeout_seconds,
            price_check_interval_ms: config.auto_sell_price_check_interval_ms,
            rpc_url: config.rpc_url.clone(),
            birdeye_limiter,
            moralis_limiter,
            moralis_client,
        }
    }

    /// Get all active positions
    pub async fn get_active_positions(&self) -> Vec<Position> {
        let guard = self.active_positions.lock().await;
        guard.values().cloned().collect()
    }

    /// Start monitoring a position and return a channel for sell signals
    pub async fn monitor_position(
        &self,
        mut position: Position,
    ) -> mpsc::Receiver<SellSignal> {
        let (tx, rx) = mpsc::channel(1);
        
        let client = self.client.clone();
        let config = self.config.clone();
        let db = self.db.clone();
        let profit_target = self.profit_target_pct;
        let stop_loss = self.stop_loss_pct;
        let mut timeout = Duration::from_secs(self.timeout_seconds);
        let check_interval = Duration::from_millis(self.price_check_interval_ms);
        let active_positions = self.active_positions.clone();
        let mint = position.mint.clone();
        let rpc_url = self.rpc_url.clone();
        
        // Initialize position tracking (if not restoring)
        if position.highest_price_reached == 0.0 {
            position.highest_price_reached = position.entry_price_sol_per_token;
        }
        // position.partial_exit_executed and timeout_extensions are preserved from DB restoration
        
        // Add to active positions SYNCHRONOUSLY to prevent race conditions
        // This ensures the position is visible immediately before monitoring starts
        {
            let mut guard = active_positions.lock().await;
            guard.insert(position.mint.clone(), position.clone());
        }

        // Clone necessary Arcs for the spawned task
        let active_positions_clone = self.active_positions.clone();
        let db = self.db.clone();
        let config = self.config.clone();
        let client = self.client.clone();
        let birdeye_limiter = self.birdeye_limiter.clone();
        let moralis_limiter = self.moralis_limiter.clone();
        let moralis_client = self.moralis_client.clone();
        let mint_clone = position.mint.clone();

        tokio::spawn(async move {
            info!(
                "📊 Position monitoring started for {} (Entry: {:.10} SOL/token, Target: +{}%, Stop: -{}%, Timeout: {}s)",
                position.mint,
                position.entry_price_sol_per_token,
                profit_target,
                stop_loss,
                timeout.as_secs()
            );

            let mut check_count = 0;
            
            loop {
                // Check timeout
                if position.entry_time.elapsed() >= timeout {
                    // Fetch price to make intelligent decision
                    let current_price = match fetch_current_price(&client, &config, &position.mint, &birdeye_limiter, &moralis_limiter, &moralis_client).await {
                        Ok(p) => p,
                        Err(e) => {
                            warn!("Failed to fetch price at timeout check for {}: {}. Assuming entry price.", position.mint, e);
                            position.entry_price_sol_per_token
                        }
                    };
                    
                    let pnl_pct = ((current_price - position.entry_price_sol_per_token) / position.entry_price_sol_per_token) * 100.0;
                    
                    // Check if we should extend timeout based on PROFITABILITY
                    if config.dynamic_timeout_enabled 
                        && position.timeout_extensions < config.max_timeout_extensions {
                        
                        // Rule 1: MOONING (>50% profit) -> Big Extension
                        if pnl_pct >= 50.0 {
                            position.timeout_extensions += 1;
                            let ext_duration = Duration::from_secs(config.timeout_extension_seconds * 3); // 3x extension
                            timeout += ext_duration;
                            info!("🚀 MOONING (+{:.1}%) - Extending timeout for {} by {}s (Big Extension #{})", 
                                pnl_pct, position.mint, ext_duration.as_secs(), position.timeout_extensions);
                            continue;
                        }
                        // Rule 2: PROFITABLE (>5% profit) -> Normal Extension
                        else if pnl_pct >= 5.0 {
                             position.timeout_extensions += 1;
                             let ext_duration = Duration::from_secs(config.timeout_extension_seconds);
                             timeout += ext_duration;
                             info!("✅ PROFITABLE (+{:.1}%) - Extending timeout for {} by {}s (Extension #{})", 
                                 pnl_pct, position.mint, ext_duration.as_secs(), position.timeout_extensions);
                             continue;
                        }
                        // Rule 3: LOSING/STAGNANT -> Do not extend, let it sell
                        else {
                            info!("⏱️  Timeout reached for {}. PnL: {:.2}%. Not extending (below profit threshold).", position.mint, pnl_pct);
                        }
                    } else {
                         info!("⏱️  Timeout reached for {} ({:.0}s). Max extensions used or dynamic disabled.", position.mint, timeout.as_secs());
                    }
                    
                    let trigger = if position.timeout_extensions > 0 {
                        SellTrigger::ExtendedTimeout(position.timeout_extensions)
                    } else {
                        SellTrigger::Timeout
                    };
                    
                    let signal = SellSignal {
                        position: position.clone(),
                        trigger,
                        current_price_sol_per_token: current_price,
                        pnl_percentage: pnl_pct,
                    };
                    
                    if tx.send(signal).await.is_err() {
                        warn!("Failed to send timeout signal for {} (receiver dropped)", position.mint);
                        break; // Channel closed, give up
                    }

                    // We don't break here! We wait for the trade engine to succeed and remove us from the map.
                    // This allows retries if the sell fails.
                    sleep(Duration::from_secs(10)).await; // Slow down checking while waiting for exit
                    continue;
                }

                // Fetch current price
                match fetch_current_price(&client, &config, &position.mint, &birdeye_limiter, &moralis_limiter, &moralis_client).await {
                    Ok(current_price) => {
                        check_count += 1;
                        let pnl_pct = ((current_price - position.entry_price_sol_per_token) / position.entry_price_sol_per_token) * 100.0;
                        
                        // Update highest price for trailing stop
                        if current_price > position.highest_price_reached {
                            position.highest_price_reached = current_price;
                            
                            // Persist state to DB
                            if let Err(e) = db.update_position_snapshot(
                                &position.mint, 
                                position.highest_price_reached, 
                                position.timeout_extensions
                            ).await {
                                warn!("Failed to update position snapshot for {}: {}", position.mint, e);
                            }
                        }
                        
                        // Log every 10 checks
                        if check_count % 10 == 0 {
                            info!(
                                "📈 Position check #{} for {}: P/L: {:.2}% | Price: {:.10} | Peak: {:.10}",
                                check_count, position.mint, pnl_pct, current_price, position.highest_price_reached
                            );
                        }

                        // CHECK 1: Partial Exit (Adaptive or Fixed)
                        let partial_exit_threshold = if position.entry_1m_move > 0.0 {
                            (1.5 * position.entry_1m_move).max(config.partial_exit_target_pct)
                        } else {
                            config.partial_exit_target_pct
                        };

                        if config.partial_exit_enabled 
                            && !position.partial_exit_executed 
                            && pnl_pct >= partial_exit_threshold {
                            
                            info!("💰 Adaptive partial profit target hit for {}: +{:.2}% (Threshold: {:.1}%) - Selling {}%", 
                                position.mint, pnl_pct, partial_exit_threshold, config.partial_exit_amount_pct);
                            
                            position.partial_exit_executed = true;
                            position.remaining_amount_pct = 100.0 - config.partial_exit_amount_pct;
                            
                            // Persist partial exit state to database
                            if let Err(e) = db.update_position_partial_exit(
                                &position.mint,
                                true,
                                position.remaining_amount_pct,
                            ).await {
                                warn!("⚠️ Failed to persist partial exit state for {}: {}", position.mint, e);
                            }
                            
                            let signal = SellSignal {
                                position: position.clone(),
                                trigger: SellTrigger::PartialProfit(pnl_pct),
                                current_price_sol_per_token: current_price,
                                pnl_percentage: pnl_pct,
                            };
                            
                            if tx.send(signal).await.is_err() {
                                warn!("Failed to send partial profit signal for {}", position.mint);
                                break;
                            }
                            
                            // Continue monitoring remaining position
                            info!("📊 Continuing to monitor {:.0}% of position for {}", 
                                position.remaining_amount_pct, position.mint);
                            continue;
                        }

                        // CHECK 2: Full Profit Target
                        if pnl_pct >= profit_target {
                            info!("🎯 Full profit target hit for {}: +{:.2}%", position.mint, pnl_pct);
                            
                            let signal = SellSignal {
                                position: position.clone(),
                                trigger: SellTrigger::ProfitTarget(pnl_pct),
                                current_price_sol_per_token: current_price,
                                pnl_percentage: pnl_pct,
                            };
                            
                            if tx.send(signal).await.is_err() {
                                warn!("Failed to send profit signal for {}", position.mint);
                                break;
                            }
                            
                            // Don't break loop, let TradeEngine confirmation remove us from map
                            sleep(Duration::from_secs(5)).await; 
                            continue;
                        }

                        // CHECK 3: Trailing Stop Loss (if enabled)
                        // GRACE PERIOD: Disable trailing stop for first 45s to survive initial volatility/latency
                        let grace_period = Duration::from_secs(45);
                        let in_grace_period = position.entry_time.elapsed() < grace_period;

                        if config.trailing_stop_enabled && !in_grace_period {
                            // Rule: Only activate trailing stop after both profit threshold AND minimum absolute price move are reached
                            let absolute_move = (current_price - position.entry_price_sol_per_token).abs();
                            if pnl_pct < config.trailing_stop_activation_pct || absolute_move < config.trailing_stop_min_price_move_sol {
                                // Trailing stop not yet active
                            } else {
                                // Rule: After 90s without new high, tighten trail stop distance
                                let mut trail_distance_pct = config.trailing_stop_distance_pct;
                                if position.entry_time.elapsed() > Duration::from_secs(90) {
                                    // Dynamic peak check: if highest_price_reached hasn't moved in a while?
                                    // Simplified: if we've been in trade > 90s, tighten from 25% to 12.5% (example)
                                    if trail_distance_pct > 15.0 {
                                        trail_distance_pct = 12.5;
                                    }
                                }
                                
                                let trail_distance = trail_distance_pct / 100.0;
                                let trailing_stop_price = position.highest_price_reached * (1.0 - trail_distance);
                                
                                if current_price <= trailing_stop_price {
                                    let pnl_from_entry = ((current_price - position.entry_price_sol_per_token) / position.entry_price_sol_per_token) * 100.0;
                                    let drop_from_peak = ((position.highest_price_reached - current_price) / position.highest_price_reached) * 100.0;
                                    
                                    warn!("🛑 Trailing stop triggered for {}: Peak {:.10} → {:.10} (-{:.1}% from peak, {:.2}% from entry, Trail: {:.1}%)", 
                                        position.mint, position.highest_price_reached, current_price, drop_from_peak, pnl_from_entry, trail_distance_pct);
                                    
                                    let signal = SellSignal {
                                        position: position.clone(),
                                        trigger: SellTrigger::TrailingStopLoss(pnl_from_entry),
                                        current_price_sol_per_token: current_price,
                                        pnl_percentage: pnl_from_entry,
                                    };
                                    
                                    if tx.send(signal).await.is_err() {
                                        warn!("Failed to send trailing stop signal for {}", position.mint);
                                        break;
                                    }
                                    
                                    // Don't break loop, let TradeEngine confirmation remove us from map
                                    sleep(Duration::from_secs(5)).await; 
                                    continue;
                                }
                            }
                        } else if in_grace_period && config.trailing_stop_enabled {
                             // Optional: Log once that we are in grace period? (Maybe too spammy)
                             // info!("🛡️ Grace Period active for {}: Skipping trailing stop", position.mint);
                        }

                        // CHECK 4: Fixed Stop Loss (Safety Floor)
                        if pnl_pct <= -stop_loss {
                            warn!("🛑 Stop-loss triggered for {}: {:.2}%", position.mint, pnl_pct);
                            
                            let signal = SellSignal {
                                position: position.clone(),
                                trigger: SellTrigger::StopLoss(pnl_pct),
                                current_price_sol_per_token: current_price,
                                pnl_percentage: pnl_pct,
                            };
                            
                            if tx.send(signal).await.is_err() {
                                warn!("Failed to send stop-loss signal for {}", position.mint);
                                break;
                            }
                            
                            // Don't break loop, let TradeEngine confirmation remove us from map
                            sleep(Duration::from_secs(5)).await; 
                            continue;
                        }
                    }
                    Err(e) => {
                        error!("Failed to fetch price for {}: {}", position.mint, e);
                    }
                }

                // EXIT CHECK: Only stop monitoring if the position is no longer in the active map.
                // This happens when TradeEngine successfully calls update_trade_exit and cleans up.
                {
                    let guard = active_positions_clone.lock().await;
                    if !guard.contains_key(&mint_clone) {
                        info!("🏁 Position {} removed from active map. Exiting monitoring loop.", mint_clone);
                        break;
                    }
                }

                sleep(check_interval).await;
            }
            
            info!("📊 Position monitoring ended for {}", position.mint);
            
            // Remove from active positions
            let mut guard = active_positions_clone.lock().await;
            guard.remove(&mint_clone);
        });

        rx
    }

    /// Remove a position from tracking (safely stops monitoring loop)
    pub async fn remove_position(&self, mint: &str) {
        let mut guard = self.active_positions.lock().await;
        if guard.remove(mint).is_some() {
            info!("🏁 Position tracker: Removed {} from active map.", mint);
        }
    }

    /// Update the token amount for a position (used after background verification)
    pub async fn update_position_amount(&self, mint: &str, new_amount: u64) {
        let mut guard = self.active_positions.lock().await;
        if let Some(pos) = guard.get_mut(mint) {
            info!("⚖️  Position tracker: Updating amount for {} from {} to {}", mint, pos.amount_token_raw, new_amount);
            pos.amount_token_raw = new_amount;
        }
    }
}

async fn fetch_current_price(
    client: &Client,
    config: &Config,
    token_mint: &str,
    birdeye_limiter: &RateLimiter,
    moralis_limiter: &RateLimiter,
    moralis: &MoralisClient,
) -> Result<f64> {
    let mint = token_mint.to_string();
    let api_key = config.birdeye_api_key.clone();

    match config.price_source_priority {
        crate::config::PriceSourcePriority::MoralisFirst => {
            // 1. Try Moralis (Primary)
            moralis_limiter.acquire().await;
            match moralis.get_token_price(&mint).await {
                Ok(price) => {
                    info!("📈 Price update (Moralis): {} = {:.8} SOL", token_mint, price);
                    Ok(price)
                },
                Err(e) => {
                    warn!("⚠️ Moralis price fetch failed for {}: {}. Falling back to Birdeye...", token_mint, e);
                    // 2. Try Birdeye (Fallback)
                    fetch_price_from_birdeye(client, &api_key, &mint, birdeye_limiter).await
                }
            }
        }
        crate::config::PriceSourcePriority::BirdeyeFirst => {
            // 1. Try Birdeye (Primary)
            match fetch_price_from_birdeye(client, &api_key, &mint, birdeye_limiter).await {
                Ok(price) => {
                    info!("📈 Price update (Birdeye): {} = {:.8} SOL", token_mint, price);
                    Ok(price)
                },
                Err(e) => {
                    warn!("⚠️ Birdeye price fetch failed for {}: {}. Falling back to Moralis...", token_mint, e);
                    // 2. Try Moralis (Fallback)
                    moralis_limiter.acquire().await;
                    moralis.get_token_price(&mint).await
                }
            }
        }
    }
}



/// Fetch price from Birdeye API (Returns price in SOL directly via priceInNative)
async fn fetch_price_from_birdeye(
    client: &Client, 
    api_key: &str, 
    mint: &str,
    limiter: &RateLimiter,
) -> Result<f64> {
    // Apply rate limiting
    limiter.acquire().await;

    let url = format!("https://public-api.birdeye.so/defi/price?address={}", mint);
    
    let resp = client.get(&url)
        .header("X-API-KEY", api_key)
        .header("x-chain", "solana")
        .header("accept", "application/json")
        .timeout(Duration::from_secs(5))
        .send().await?;
        
    let json: serde_json::Value = resp.json().await?;
    
    if let Some(price_in_native) = json.get("data").and_then(|d| d.get("priceInNative")).and_then(|v| v.as_f64()) {
        if price_in_native > 0.0 {
            return Ok(price_in_native);
        }
    }
    
    Err(anyhow::anyhow!("Birdeye priceInNative not found for {}", mint))
}



#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_profit_target_trigger() {
        let entry_price = 0.00001f64;
        let current_price = 0.000015f64; // +50%
        let pnl_pct = ((current_price - entry_price) / entry_price) * 100.0;
        
        assert!((pnl_pct - 50.0).abs() < 0.01); // Use epsilon comparison for floats
    }

    #[test]
    fn test_stop_loss_trigger() {
        let entry_price = 0.00001f64;
        let current_price = 0.000008f64; // -20%
        let pnl_pct = ((current_price - entry_price) / entry_price) * 100.0;
        
        assert!((pnl_pct + 20.0).abs() < 0.01); // Use epsilon comparison
    }

    #[test]
    fn test_no_trigger_within_bounds() {
        let entry_price = 0.00001;
        let current_price = 0.000012; // +20% (within bounds)
        let pnl_pct = ((current_price - entry_price) / entry_price) * 100.0;
        
        let profit_target = 50.0;
        let stop_loss = 20.0;
        
        assert!(pnl_pct < profit_target);
        assert!(pnl_pct > -stop_loss);
    }

    #[test]
    fn test_sell_trigger_display() {
        let profit = SellTrigger::ProfitTarget(52.5);
        assert_eq!(profit.to_string(), "PROFIT_TARGET (+52.50%)");
        
        let stop_loss = SellTrigger::StopLoss(-18.3);
        assert_eq!(stop_loss.to_string(), "STOP_LOSS (-18.30%)"); // Negative is already in value
        
        let timeout = SellTrigger::Timeout;
        assert_eq!(timeout.to_string(), "TIMEOUT");
    }
}

