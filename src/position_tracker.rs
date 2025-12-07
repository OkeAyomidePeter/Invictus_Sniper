use crate::config::Config;
use anyhow::{Context, Result};
use log::{error, info, warn};
use reqwest::Client;
use serde_json::Value;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tokio::time::sleep;

const JUPITER_QUOTE_API: &str = "https://quote-api.jup.ag/v6/quote";
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
            SellTrigger::StopLoss(pct) => write!(f, "STOP_LOSS (-{:.2}%)", pct),
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
pub struct PositionTracker {
    client: Client,
    helius_api_key: String,
    config: Config,
    profit_target_pct: f64,
    stop_loss_pct: f64,
    timeout_seconds: u64,
    price_check_interval_ms: u64,
}

impl PositionTracker {
    pub fn new(config: &Config) -> Self {
        Self {
            client: Client::new(),
            helius_api_key: config.helius_api_key.clone(),
            config: config.clone(),
            profit_target_pct: config.auto_sell_profit_target_pct,
            stop_loss_pct: config.auto_sell_stop_loss_pct,
            timeout_seconds: config.auto_sell_timeout_seconds,
            price_check_interval_ms: config.auto_sell_price_check_interval_ms,
        }
    }

    /// Start monitoring a position and return a channel for sell signals
    pub fn monitor_position(
        &self,
        mut position: Position,
    ) -> mpsc::Receiver<SellSignal> {
        let (tx, rx) = mpsc::channel(1);
        
        let client = self.client.clone();
        let config = self.config.clone();
        let profit_target = self.profit_target_pct;
        let stop_loss = self.stop_loss_pct;
        let mut timeout = Duration::from_secs(self.timeout_seconds);
        let check_interval = Duration::from_millis(self.price_check_interval_ms);
        
        // Initialize position tracking
        position.highest_price_reached = position.entry_price_sol_per_token;
        position.partial_exit_executed = false;
        position.remaining_amount_pct = 100.0;
        position.timeout_extensions = 0;
        
        // Spawn monitoring task
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
            let mut last_volume_5m = 0.0;
            
            loop {
                // Check timeout
                if position.entry_time.elapsed() >= timeout {
                    // Check if we should extend timeout
                    if config.dynamic_timeout_enabled 
                        && position.timeout_extensions < config.max_timeout_extensions
                        && last_volume_5m > 0.0 {
                        
                        // Extend if volume is strong
                        position.timeout_extensions += 1;
                        timeout += Duration::from_secs(config.timeout_extension_seconds);
                        
                        info!("⏱️  Timeout extended for {} (extension #{}, now: {}s) - Volume: ${:.0}", 
                            position.mint, position.timeout_extensions, timeout.as_secs(), last_volume_5m);
                        continue;
                    }
                    
                    info!("⏱️  Position timeout reached for {} ({:.0}s)", position.mint, timeout.as_secs());
                    
                    let final_price = match fetch_current_price(&client, &position.mint, position.amount_token_raw).await {
                        Ok(price) => price,
                        Err(_) => position.entry_price_sol_per_token,
                    };
                    
                    let pnl_pct = ((final_price - position.entry_price_sol_per_token) / position.entry_price_sol_per_token) * 100.0;
                    
                    let trigger = if position.timeout_extensions > 0 {
                        SellTrigger::ExtendedTimeout(position.timeout_extensions)
                    } else {
                        SellTrigger::Timeout
                    };
                    
                    let signal = SellSignal {
                        position: position.clone(),
                        trigger,
                        current_price_sol_per_token: final_price,
                        pnl_percentage: pnl_pct,
                    };
                    
                    if tx.send(signal).await.is_err() {
                        warn!("Failed to send timeout signal for {} (receiver dropped)", position.mint);
                    }
                    break;
                }

                // Fetch current price
                match fetch_current_price(&client, &position.mint, position.amount_token_raw).await {
                    Ok(current_price) => {
                        check_count += 1;
                        let pnl_pct = ((current_price - position.entry_price_sol_per_token) / position.entry_price_sol_per_token) * 100.0;
                        
                        // Update highest price for trailing stop
                        if current_price > position.highest_price_reached {
                            position.highest_price_reached = current_price;
                        }
                        
                        // Log every 10 checks
                        if check_count % 10 == 0 {
                            info!(
                                "📈 Position check #{} for {}: P/L: {:.2}% | Price: {:.10} | Peak: {:.10}",
                                check_count, position.mint, pnl_pct, current_price, position.highest_price_reached
                            );
                        }

                        // CHECK 1: Partial Exit (if enabled and not yet executed)
                        if config.partial_exit_enabled 
                            && !position.partial_exit_executed 
                            && pnl_pct >= config.partial_exit_target_pct {
                            
                            info!("💰 Partial profit target hit for {}: +{:.2}% - Selling {}%", 
                                position.mint, pnl_pct, config.partial_exit_amount_pct);
                            
                            position.partial_exit_executed = true;
                            position.remaining_amount_pct = 100.0 - config.partial_exit_amount_pct;
                            
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
                            }
                            break;
                        }

                        // CHECK 3: Trailing Stop Loss (if enabled)
                        if config.trailing_stop_enabled {
                            let trail_distance = config.trailing_stop_distance_pct / 100.0;
                            let trailing_stop_price = position.highest_price_reached * (1.0 - trail_distance);
                            
                            if current_price <= trailing_stop_price {
                                let pnl_from_entry = ((current_price - position.entry_price_sol_per_token) / position.entry_price_sol_per_token) * 100.0;
                                let drop_from_peak = ((position.highest_price_reached - current_price) / position.highest_price_reached) * 100.0;
                                
                                warn!("🛑 Trailing stop triggered for {}: Peak {:.10} → {:.10} (-{:.1}% from peak, {:.2}% from entry)", 
                                    position.mint, position.highest_price_reached, current_price, drop_from_peak, pnl_from_entry);
                                
                                let signal = SellSignal {
                                    position: position.clone(),
                                    trigger: SellTrigger::TrailingStopLoss(pnl_from_entry),
                                    current_price_sol_per_token: current_price,
                                    pnl_percentage: pnl_from_entry,
                                };
                                
                                if tx.send(signal).await.is_err() {
                                    warn!("Failed to send trailing stop signal for {}", position.mint);
                                }
                                break;
                            }
                        }

                        // CHECK 4: Fixed Stop Loss (if trailing stop not enabled or hasn't risen yet)
                        if !config.trailing_stop_enabled && pnl_pct <= -stop_loss {
                            warn!("🛑 Stop-loss triggered for {}: {:.2}%", position.mint, pnl_pct);
                            
                            let signal = SellSignal {
                                position: position.clone(),
                                trigger: SellTrigger::StopLoss(pnl_pct),
                                current_price_sol_per_token: current_price,
                                pnl_percentage: pnl_pct,
                            };
                            
                            if tx.send(signal).await.is_err() {
                                warn!("Failed to send stop-loss signal for {}", position.mint);
                            }
                            break;
                        }
                    }
                    Err(e) => {
                        error!("Failed to fetch price for {}: {}", position.mint, e);
                    }
                }

                sleep(check_interval).await;
            }
            
            info!("📊 Position monitoring ended for {}", position.mint);
        });

        rx
    }
}

/// Fetch current price for a token via Jupiter quote
async fn fetch_current_price(
    client: &Client,
    token_mint: &str,
    amount_token_raw: u64,
) -> Result<f64> {
    // Get quote for token -> SOL
    let url = format!(
        "{}?inputMint={}&outputMint={}&amount={}&slippageBps=50",
        JUPITER_QUOTE_API,
        token_mint,
        SOL_MINT,
        amount_token_raw
    );

    let response: Value = client
        .get(&url)
        .send()
        .await
        .context("Failed to send Jupiter quote request")?
        .json()
        .await
        .context("Failed to parse Jupiter quote response")?;

    // Extract outAmount (SOL lamports we'd get)
    let out_amount_lamports = response
        .get("outAmount")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse::<u64>().ok())
        .context("Failed to extract outAmount from Jupiter quote")?;

    // Calculate price per token
    // price_per_token = total_sol_received / token_amount
    let price_per_token = (out_amount_lamports as f64) / (amount_token_raw as f64);
    
    Ok(price_per_token)
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
        assert_eq!(stop_loss.to_string(), "STOP_LOSS (--18.30%)"); // Negative is already in value
        
        let timeout = SellTrigger::Timeout;
        assert_eq!(timeout.to_string(), "TIMEOUT");
    }
}

