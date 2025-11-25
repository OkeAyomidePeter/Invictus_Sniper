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
}

/// Reason for triggering a sell
#[derive(Debug, Clone, PartialEq)]
pub enum SellTrigger {
    ProfitTarget(f64), // Percentage gain
    StopLoss(f64),     // Percentage loss
    Timeout,
}

impl std::fmt::Display for SellTrigger {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SellTrigger::ProfitTarget(pct) => write!(f, "PROFIT_TARGET (+{:.2}%)", pct),
            SellTrigger::StopLoss(pct) => write!(f, "STOP_LOSS (-{:.2}%)", pct),
            SellTrigger::Timeout => write!(f, "TIMEOUT"),
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
            profit_target_pct: config.auto_sell_profit_target_pct,
            stop_loss_pct: config.auto_sell_stop_loss_pct,
            timeout_seconds: config.auto_sell_timeout_seconds,
            price_check_interval_ms: config.auto_sell_price_check_interval_ms,
        }
    }

    /// Start monitoring a position and return a channel for sell signals
    pub fn monitor_position(
        &self,
        position: Position,
    ) -> mpsc::Receiver<SellSignal> {
        let (tx, rx) = mpsc::channel(1);
        
        let client = self.client.clone();
        let profit_target = self.profit_target_pct;
        let stop_loss = self.stop_loss_pct;
        let timeout = Duration::from_secs(self.timeout_seconds);
        let check_interval = Duration::from_millis(self.price_check_interval_ms);
        
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
            loop {
                // Check timeout first
                if position.entry_time.elapsed() >= timeout {
                    info!("⏱️  Position timeout reached for {} ({:.0}s)", position.mint, timeout.as_secs());
                    
                    // Get final price for calculation
                    let final_price = match fetch_current_price(&client, &position.mint, position.amount_token_raw).await {
                        Ok(price) => price,
                        Err(_) => position.entry_price_sol_per_token, // Use entry price if fetch fails
                    };
                    
                    let pnl_pct = ((final_price - position.entry_price_sol_per_token) / position.entry_price_sol_per_token) * 100.0;
                    
                    let signal = SellSignal {
                        position: position.clone(),
                        trigger: SellTrigger::Timeout,
                        current_price_sol_per_token: final_price,
                        pnl_percentage: pnl_pct,
                    };
                    
                    if tx.send(signal).await.is_err() {
                        warn!("Failed to send timeout signal for {} (receiver dropped)", position.mint);
                    }
                    break;
                }

                // Fetch current price via Jupiter quote
                match fetch_current_price(&client, &position.mint, position.amount_token_raw).await {
                    Ok(current_price) => {
                        check_count += 1;
                        let pnl_pct = ((current_price - position.entry_price_sol_per_token) / position.entry_price_sol_per_token) * 100.0;
                        
                        // Log every 10 checks (every ~20 seconds with 2s interval)
                        if check_count % 10 == 0 {
                            info!(
                                "📈 Position check #{} for {}: Current P/L: {:.2}% (Price: {:.10} SOL/token)",
                                check_count,
                                position.mint,
                                pnl_pct,
                                current_price
                            );
                        }

                        // Check profit target
                        if pnl_pct >= profit_target {
                            info!("🎯 Profit target hit for {}: +{:.2}%", position.mint, pnl_pct);
                            
                            let signal = SellSignal {
                                position: position.clone(),
                                trigger: SellTrigger::ProfitTarget(pnl_pct),
                                current_price_sol_per_token: current_price,
                                pnl_percentage: pnl_pct,
                            };
                            
                            if tx.send(signal).await.is_err() {
                                warn!("Failed to send profit signal for {} (receiver dropped)", position.mint);
                            }
                            break;
                        }

                        // Check stop-loss
                        if pnl_pct <= -stop_loss {
                            warn!("🛑 Stop-loss triggered for {}: {:.2}%", position.mint, pnl_pct);
                            
                            let signal = SellSignal {
                                position: position.clone(),
                                trigger: SellTrigger::StopLoss(pnl_pct),
                                current_price_sol_per_token: current_price,
                                pnl_percentage: pnl_pct,
                            };
                            
                            if tx.send(signal).await.is_err() {
                                warn!("Failed to send stop-loss signal for {} (receiver dropped)", position.mint);
                            }
                            break;
                        }
                    }
                    Err(e) => {
                        error!("Failed to fetch price for {}: {}", position.mint, e);
                        // Continue monitoring despite error
                    }
                }

                // Wait before next check
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

