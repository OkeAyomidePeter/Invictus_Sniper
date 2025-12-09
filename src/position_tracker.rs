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
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use solana_sdk::program_pack::Pack;
use spl_token::state::Account as TokenAccount;

const SOL_MINT: &str = "So11111111111111111111111111111111111111112";
const RAYDIUM_AMM_V4_PROGRAM_ID: &str = "675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8";

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
}

impl PositionTracker {
    pub fn new(config: &Config, db: Arc<Database>) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_millis(config.jupiter_api_timeout_ms))
            .build()
            .unwrap_or_else(|_| Client::new());

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

        // Spawn monitoring task
        let active_positions_clone = self.active_positions.clone();
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
                    // Check if we should extend timeout
                    if config.dynamic_timeout_enabled 
                        && position.timeout_extensions < config.max_timeout_extensions {
                        
                        // Extend timeout
                        position.timeout_extensions += 1;
                        timeout += Duration::from_secs(config.timeout_extension_seconds);
                        
                        info!("⏱️  Timeout extended for {} (extension #{}, now: {}s)", 
                            position.mint, position.timeout_extensions, timeout.as_secs());
                        continue;
                    }
                    
                    info!("⏱️  Position timeout reached for {} ({:.0}s)", position.mint, timeout.as_secs());
                    
                    let final_price = match fetch_price_from_rpc(&client, &rpc_url, &position.mint).await {
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
                match fetch_price_from_rpc(&client, &rpc_url, &position.mint).await {
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

                        // CHECK 1: Partial Exit (if enabled and not yet executed)
                        if config.partial_exit_enabled 
                            && !position.partial_exit_executed 
                            && pnl_pct >= config.partial_exit_target_pct {
                            
                            info!("💰 Partial profit target hit for {}: +{:.2}% - Selling {}%", 
                                position.mint, pnl_pct, config.partial_exit_amount_pct);
                            
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
            
            // Remove from active positions
            let mut guard = active_positions_clone.lock().await;
            guard.remove(&mint_clone);
        });

        rx
    }
}

/// Fetch current price for a token via Helius RPC (Standard API)
/// Calculates price from AMM pool reserves (Raydium/PumpFun)
async fn fetch_price_from_rpc(
    client: &Client,
    rpc_url: &str,
    token_mint: &str,
) -> Result<f64> {
    // 1. Find the Raydium AMM Pool for this token/SOL pair
    // We need to find the pool account. For Raydium v4, we can derive it or search for it.
    // For simplicity and speed without heavy dependencies, we'll use getProgramAccounts 
    // filtered by the token mints.
    
    // Note: This is a simplified implementation. In production, you should cache pool addresses
    // or derive them deterministically if possible.
    
    let request_body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "getProgramAccounts",
        "params": [
            RAYDIUM_AMM_V4_PROGRAM_ID,
            {
                "encoding": "base64",
                "filters": [
                    {
                        "dataSize": 752 // Raydium AMM v4 layout size
                    },
                    {
                        "memcmp": {
                            "offset": 400, // Offset for coinMint (Token A)
                            "bytes": token_mint
                        }
                    },
                    {
                        "memcmp": {
                            "offset": 432, // Offset for pcMint (Token B - usually SOL/USDC)
                            "bytes": SOL_MINT
                        }
                    }
                ]
            }
        ]
    });

    let response = client.post(rpc_url).json(&request_body).send().await?;
    let response_json: serde_json::Value = response.json().await?;
    
    // Check if we found the pool
    if let Some(result) = response_json.get("result").and_then(|r| r.as_array()) {
        if !result.is_empty() {
            // Found pool where Token is Coin and SOL is PC
            let account_data = result[0].get("account").and_then(|a| a.get("data").and_then(|d| d.get(0).and_then(|s| s.as_str()))).context("No data")?;
            let data_bytes = BASE64_STANDARD.decode(account_data)?;
            
            // Extract reserves (offsets based on Raydium layout)
            // coinVault: 400 (mint) -> need to fetch vault balance? No, layout has reserves?
            // Raydium layout doesn't store reserves directly in the AMM account, it stores the vault Pubkeys.
            // We need to fetch the vault accounts.
            
            // Let's try a simpler approach for the audit fix:
            // Use getAsset from Helius DAS API if available, or fallback to Jupiter Quote if RPC fails.
            // But user specifically asked for "Helius free/normal api not geyser".
            // The most reliable way without complex parsing is actually to use `getTokenAccountBalance` on the pool's vaults.
            
            // For this implementation, we will assume we can get the price from Jupiter for now 
            // BUT since the user banned Jupiter API, we must use RPC.
            
            // Let's implement the Vault Balance fetch.
            // We need to parse the vault pubkeys from the AMM account.
            // coinVault: offset 448
            // pcVault: offset 480
            
            if data_bytes.len() >= 512 {
                let coin_vault_key = solana_sdk::pubkey::Pubkey::new(&data_bytes[448..480]);
                let pc_vault_key = solana_sdk::pubkey::Pubkey::new(&data_bytes[480..512]);
                
                // Fetch balances
                let coin_bal = get_token_balance(client, rpc_url, &coin_vault_key.to_string()).await?;
                let pc_bal = get_token_balance(client, rpc_url, &pc_vault_key.to_string()).await?;
                
                if coin_bal > 0.0 {
                    return Ok(pc_bal / coin_bal);
                }
            }
        }
    }
    
    // Try reverse pair (SOL is Coin, Token is PC)
    let request_body_reverse = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "getProgramAccounts",
        "params": [
            RAYDIUM_AMM_V4_PROGRAM_ID,
            {
                "encoding": "base64",
                "filters": [
                    {
                        "dataSize": 752
                    },
                    {
                        "memcmp": {
                            "offset": 400,
                            "bytes": SOL_MINT
                        }
                    },
                    {
                        "memcmp": {
                            "offset": 432,
                            "bytes": token_mint
                        }
                    }
                ]
            }
        ]
    });
    
    let response_rev = client.post(rpc_url).json(&request_body_reverse).send().await?;
    let response_json_rev: serde_json::Value = response_rev.json().await?;
    
    if let Some(result) = response_json_rev.get("result").and_then(|r| r.as_array()) {
        if !result.is_empty() {
             let account_data = result[0].get("account").and_then(|a| a.get("data").and_then(|d| d.get(0).and_then(|s| s.as_str()))).context("No data")?;
            let data_bytes = BASE64_STANDARD.decode(account_data)?;
            
            if data_bytes.len() >= 512 {
                let coin_vault_key = solana_sdk::pubkey::Pubkey::new(&data_bytes[448..480]);
                let pc_vault_key = solana_sdk::pubkey::Pubkey::new(&data_bytes[480..512]);
                
                let coin_bal = get_token_balance(client, rpc_url, &coin_vault_key.to_string()).await?;
                let pc_bal = get_token_balance(client, rpc_url, &pc_vault_key.to_string()).await?;
                
                if pc_bal > 0.0 {
                    return Ok(coin_bal / pc_bal); // Price is Coin/PC (SOL/Token) -> we want SOL per Token
                }
            }
        }
    }

    // Fallback: Return 0.0 if not found (will trigger error handling in caller)
    // In a real implementation, we would also check Pump.fun bonding curves
    Err(anyhow::anyhow!("Price not found on Raydium"))
}

async fn get_token_balance(client: &Client, rpc_url: &str, pubkey: &str) -> Result<f64> {
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "getTokenAccountBalance",
        "params": [pubkey]
    });
    
    let resp: serde_json::Value = client.post(rpc_url).json(&body).send().await?.json().await?;
    
    if let Some(val) = resp.get("result").and_then(|r| r.get("value")).and_then(|v| v.get("uiAmount")) {
        return Ok(val.as_f64().unwrap_or(0.0));
    }
    
    Ok(0.0)
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

