use crate::config::Config;
use crate::db::Database;
use crate::enrichment::EnrichedToken;
use crate::position_tracker::{Position, PositionTracker, SellSignal, SellTrigger};
use crate::presigner::{BundleStatus, Presigner};
use crate::tx::{DexRouter, TransactionManager};
use crate::trade_logger::{
    log_buy_attempt, log_buy, log_buy_failed, log_sell, log_sell_failed,
    log_bundle_sent, log_bundle_confirmed, log_position_started,
    log_pipeline_step, log_error_detailed
};
use anyhow::{Context, Result};
use log::{error, info, warn};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Instant;
use tokio::sync::mpsc;
use crate::tele::TelegramInterface;

/// TradeEngine orchestrates the entire lifecycle of a trade:
/// Buy -> Monitor -> Sell
#[derive(Clone)]
pub struct TradeEngine {
    config: Config,
    db: Arc<Database>,
    presigner: Arc<Presigner>,
    tx_manager: Arc<TransactionManager>,
    position_tracker: Arc<PositionTracker>,
    tele: Option<Arc<TelegramInterface>>,
    consecutive_losses: Arc<AtomicU32>,
}

impl TradeEngine {
    pub fn new(
        config: &Config,
        db: Arc<Database>,
        presigner: Arc<Presigner>,
        tx_manager: Arc<TransactionManager>,
        position_tracker: Arc<PositionTracker>,
        tele: Option<Arc<TelegramInterface>>,
    ) -> Self {
        Self {
            config: config.clone(),
            db,
            presigner,
            tx_manager,
            position_tracker,
            tele,
            consecutive_losses: Arc::new(AtomicU32::new(0)),
        }
    }

    /// Execute a buy and start monitoring the position
    pub async fn execute_buy(&self, token: &EnrichedToken, amount_sol: f64, is_watchlist: bool) -> Result<()> {
        info!("🤖 TradeEngine: Initiating BUY for {} (Amount: {:.4} SOL)", token.mint, amount_sol);
        log_buy_attempt(&token.mint, amount_sol, is_watchlist);

        let start_total = Instant::now();
        let amount_lamports = (amount_sol * 1_000_000_000.0) as u64;
        let slippage_bps = 200; // Default 2%
        
        // --- PRICE STABILITY GUARD ---
        if !self.ensure_price_stability(&token.mint, amount_lamports).await? {
            warn!("🥀 BUY ABORTED: {} failed price stability check", token.mint);
            return Ok(());
        }
        
        // STEP 2: Build Buy Transaction
        let start_step = Instant::now();
        let (mut buy_tx, expected_out_amount) = match self.tx_manager.build_buy_transaction(
            &token.mint,
            amount_lamports,
            slippage_bps,
            DexRouter::Jupiter, // Default to Jupiter for now
            0, // Tip lamports not needed here anymore
        ).await {
            Ok(res) => {
                log_pipeline_step(&token.mint, "Build Buy Tx", start_step.elapsed().as_millis(), true);
                res
            },
            Err(e) => {
                log_pipeline_step(&token.mint, "Build Buy Tx", start_step.elapsed().as_millis(), false);
                log_error_detailed(&token.mint, "Build Buy Tx", &e.to_string());
                return Err(e);
            }
        };

        // STEP 3: Sign & Send (Standard vs Jito)
        let mode = self.tx_manager.transaction_mode();
        let identifier = if mode == crate::config::TransactionMode::Standard {
            // STEP 3a: Sign and Send Standard Transaction
            let start_step = Instant::now();
            
            // REMOVED Simulation Check for latency
            
            // Sign the transaction first
            self.presigner.sign_versioned_tx(&mut buy_tx)?;
            
            let sig = match self.presigner.send_versioned_transaction(&buy_tx).await {
                Ok(s) => {
                    log_pipeline_step(&token.mint, "Send Standard Tx", start_step.elapsed().as_millis(), true);
                    crate::trade_logger::log_priority_fees(&token.mint, self.config.priority_fee_lamports);
                    s
                },
                Err(e) => {
                    log_pipeline_step(&token.mint, "Send Standard Tx", start_step.elapsed().as_millis(), false);
                    log_error_detailed(&token.mint, "Send Standard Tx", &e.to_string());
                    return Err(e);
                }
            };
            info!("🚀 Standard BUY Sent! Sig: {}", sig);
            sig
        } else {
            // STEP 3b: Send Jito Bundle
            let tip_lamports = self.tx_manager.calculate_tip(true);
            let recent_blockhash = self.presigner.get_blockhash();
            
            // Build Tip Transaction (Jito Only)
            let mut tip_tx = self.tx_manager.build_tip_transaction(tip_lamports, recent_blockhash)?;
            buy_tx.message.set_recent_blockhash(recent_blockhash);
            
            // REMOVED Simulation Check for latency

            self.presigner.sign_versioned_tx(&mut buy_tx)?;
            self.presigner.sign_versioned_tx(&mut tip_tx)?;

            let start_step = Instant::now();
            let (bundle_id, bundle_sigs) = match self.presigner.send_jito_bundle(vec![buy_tx, tip_tx]).await {
                Ok(res) => {
                    log_pipeline_step(&token.mint, "Send Bundle", start_step.elapsed().as_millis(), true);
                    res
                },
                Err(e) => {
                    log_pipeline_step(&token.mint, "Send Bundle", start_step.elapsed().as_millis(), false);
                    log_error_detailed(&token.mint, "Send Bundle", &e.to_string());
                    return Err(e);
                }
            };
            
            info!("🚀 Buy Bundle Sent! Jito IDs: {} | Main Tx: {}", bundle_id, bundle_sigs.get(0).map(|s| s.to_string()).unwrap_or_default());
            log_bundle_sent(&bundle_id, 2);

            // Wait for confirmation (Jito Only)
            let start_confirm = Instant::now();
            let status = self.presigner.wait_for_bundle_confirmation(&bundle_id, 30, &bundle_sigs).await?;
            if !status.is_success() {
                 return Err(anyhow::anyhow!("Bundle failed to confirm: {:?}", status));
            }
            log_pipeline_step(&token.mint, "Confirm Bundle", start_confirm.elapsed().as_millis(), true);
            
            // Use the actual swap transaction signature as the identifier (NOT the bundle_id)
            bundle_sigs.get(0).map(|s| s.to_string()).unwrap_or(bundle_id)
        };

        // STEP 7: Start Optimistic Monitoring (ASAP!)
        if self.config.auto_sell_enabled {
            crate::trade_logger::log_optimistic_start(&token.mint, expected_out_amount);
            
            let top_10_pct = token.holders.as_ref().map(|h| h.top_10_pct).unwrap_or(0.0);
            let holder_count = token.holders.as_ref().and_then(|h| h.unique_holders).unwrap_or(0);
            let socials_count = token.metadata.as_ref().and_then(|m| m.socials.as_ref()).map(|s| {
                let mut count = 0;
                if s.twitter.is_some() { count += 1; }
                if s.telegram.is_some() { count += 1; }
                if s.website.is_some() { count += 1; }
                count
            }).unwrap_or(0);

            self.start_auto_sell_monitoring(
                token.mint.clone(), 
                amount_sol, 
                expected_out_amount, 
                token.decimals,
                token.price_change_1m_pct.unwrap_or(0.0),
                token.liquidity_usd.unwrap_or(0.0),
                top_10_pct,
                holder_count,
                socials_count
            ).await;
        }

        // STEP 8: Background Verification & DB Recording
        let engine_clone = self.clone();
        let token_clone = token.clone();
        let id_clone = identifier.clone();
        
        tokio::spawn(async move {
            let start_bg = Instant::now();
            let mut final_token_amount = expected_out_amount;
            let mut verified = false;

            // Faster Polling: 500ms instead of 2.0s
            for attempt in 1..=10 {
                match engine_clone.presigner.get_token_balance(&token_clone.mint).await {
                    Ok(amount) if amount > 0 => {
                        final_token_amount = amount;
                        verified = true;
                        info!("✅ BG-VERIFY: Balance confirmed for {} after {} attempts: {} tokens", token_clone.mint, attempt, final_token_amount);
                        
                        // Update the optimistic monitoring with the real amount
                        engine_clone.position_tracker.update_position_amount(&token_clone.mint, final_token_amount).await;
                        break;
                    }
                    _ => {
                        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                    }
                }
            }

            if verified {
                log_pipeline_step(&token_clone.mint, "Verify Balance (BG)", start_bg.elapsed().as_millis(), true);
            } else {
                warn!("⚠️  BG-VERIFY: Failed to verify exact balance for {} within 5s. Sticking with optimistic amount.", token_clone.mint);
            }

            // Record Trade in DB
            if let Err(e) = engine_clone.db.record_trade(
                &token_clone.mint,
                "BUY",
                final_token_amount,
                amount_lamports,
                &id_clone,
                Some(&id_clone),
                Some(amount_sol / (final_token_amount as f64 / 10f64.powf(token_clone.decimals as f64))),
                token_clone.decimals,
                token_clone.price_change_1m_pct.unwrap_or(0.0),
            ).await {
                error!("❌ BG-DB: Failed to record trade for {}: {}", token_clone.mint, e);
            }

            // Telegram Notification
            if let Some(tele) = &engine_clone.tele {
                let amt_token_whole = final_token_amount as f64 / 10f64.powf(token_clone.decimals as f64);
                let entry = if final_token_amount > 0 { amount_sol / amt_token_whole } else { 0.0 };
                tele.notify_buy(&token_clone.mint, amount_sol, amt_token_whole, entry, &id_clone).await;
            }
        });

        Ok(())
    }

    /// Pre-buy price stability check to avoid buying the top of a candle or a dumping token
    async fn ensure_price_stability(&self, mint: &str, in_amount_lamports: u64) -> Result<bool> {
        info!("⚖️  Checking price stability for {} ({}s window)...", &mint[..12], self.config.price_stability_window_secs);
        
        // Sample 1: Initial quote
        let quote1 = match self.tx_manager.get_jupiter_quote(crate::tx::SOL_MINT, mint, in_amount_lamports, 200).await {
            Ok(q) => q,
            Err(e) => {
                warn!("⚠️ Stability check Sample 1 failed: {}. Proceeding anyway.", e);
                return Ok(true);
            }
        };
        
        let out_amount1: u64 = quote1["outAmount"].as_str()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
            
        if out_amount1 == 0 { return Ok(true); }

        // Wait for stability window
        tokio::time::sleep(std::time::Duration::from_secs(self.config.price_stability_window_secs)).await;

        // Sample 2: Final quote
        let quote2 = match self.tx_manager.get_jupiter_quote(crate::tx::SOL_MINT, mint, in_amount_lamports, 200).await {
            Ok(q) => q,
            Err(e) => {
                warn!("⚠️ Stability check Sample 2 failed: {}. Proceeding anyway.", e);
                return Ok(true);
            }
        };

        let out_amount2: u64 = quote2["outAmount"].as_str()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);

        if out_amount2 == 0 { return Ok(true); }

        // Compare out amounts (more tokens for same SOL = lower price)
        // price1 = SOL / out1
        // price2 = SOL / out2
        // drop = (price1 - price2) / price1 = (1/out1 - 1/out2) / (1/out1) = 1 - out1/out2
        // If out2 < out1, price2 > price1 (Pumping) -> 1 - out1/out2 is negative
        // If out2 > out1, price2 < price1 (Dumping) -> 1 - out1/out2 is positive
        
        let drop_pct = (1.0 - (out_amount1 as f64 / out_amount2 as f64)) * 100.0;
        
        if drop_pct > self.config.price_stability_max_drop_pct {
            warn!("🥀 STABILITY FAILED: {} dropped {:.2}% in {}s (Max: {:.1}%)", 
                &mint[..12], drop_pct, self.config.price_stability_window_secs, self.config.price_stability_max_drop_pct);
            crate::trade_logger::log_price_stability_failed(mint, drop_pct);
            return Ok(false);
        }

        info!("✅ Price stable: {} delta: {:.2}%", &mint[..12], drop_pct);
        Ok(true)
    }

    /// Execute a sell transaction
    async fn execute_sell(&self, signal: &SellSignal) -> Result<String> {
        info!("🤖 TradeEngine: Initiating SELL for {} (Trigger: {})", signal.position.mint, signal.trigger);

        let amount_token = signal.position.amount_token_raw;
        let slippage_bps = self.config.auto_sell_slippage_bps;

        // 1. Send (Standard vs Jito)
        let mode = self.tx_manager.transaction_mode();
        let identifier = if mode == crate::config::TransactionMode::Standard {
            // 1a. Build, Sign & Send Standard Transaction
            let mut sell_tx = self.tx_manager.build_sell_transaction(
                &signal.position.mint,
                amount_token,
                slippage_bps,
                DexRouter::Jupiter,
                true, // Close ATA
            ).await?;

            // REMOVED Simulation Check for latency
            
            // Sign the transaction first
            self.presigner.sign_versioned_tx(&mut sell_tx)?;

            let sig = self.presigner.send_versioned_transaction(&sell_tx).await?;
            info!("🚀 Standard SELL Sent! Sig: {}", sig);
            crate::trade_logger::log_priority_fees(&signal.position.mint, self.config.priority_fee_lamports);
            sig
        } else {
            // 1b. Jito Bundle Mode
            let tip_lamports = self.tx_manager.calculate_tip(false);
            
            let mut sell_tx = self.tx_manager.build_sell_transaction(
                &signal.position.mint,
                amount_token,
                slippage_bps,
                DexRouter::Jupiter,
                true, // Close ATA
            ).await?;

            let recent_blockhash = self.presigner.get_blockhash();
            let mut tip_tx = self.tx_manager.build_tip_transaction(tip_lamports, recent_blockhash)?;

            sell_tx.message.set_recent_blockhash(recent_blockhash);

            // REMOVED Simulation Check for latency

            self.presigner.sign_versioned_tx(&mut sell_tx)?;
            self.presigner.sign_versioned_tx(&mut tip_tx)?;

            let (id, sigs) = self.presigner.send_jito_bundle(vec![sell_tx, tip_tx]).await?;
            info!("🚀 Sell Bundle Sent! Jito IDs: {} | Main Tx: {}", id, sigs.get(0).map(|s| s.to_string()).unwrap_or_default());
            log_bundle_sent(&id, 2);
            
            // Use the actual swap transaction signature as the identifier
            sigs.get(0).map(|s| s.to_string()).unwrap_or(id)
        };
        
        Ok(identifier)
    }

    /// Close all open positions (graceful shutdown)
    /// Returns a list of (mint, Result<bundle_id>) for each attempted sell
    pub async fn close_all_positions(&self) -> Vec<(String, Result<String>)> {
        let positions = self.position_tracker.get_active_positions().await;
        
        if positions.is_empty() {
            info!("🛑 No active positions to close during shutdown");
            return Vec::new();
        }

        info!("🛑 EMERGENCY SHUTDOWN: Closing {} active positions...", positions.len());
        
        let mut results = Vec::new();
        
        for position in positions {
            info!("🛑 Closing position: {} ({} tokens)", position.mint, position.amount_token_raw);
            
            // Create a synthetic sell signal for emergency exit
            let signal = SellSignal {
                position: position.clone(),
                trigger: SellTrigger::Timeout, // Using Timeout as the emergency trigger
                current_price_sol_per_token: position.entry_price_sol_per_token, // Use entry as fallback
                pnl_percentage: 0.0, // Unknown without current price
            };
            
            match self.execute_sell(&signal).await {
                Ok(bundle_id) => {
                    info!("✅ Position closed for {}: Bundle {}", position.mint, bundle_id);
                    
                    // Record the exit in DB
                    if let Err(e) = self.db.update_trade_exit(
                        &position.mint,
                        position.entry_price_sol_per_token, // Best effort price
                        0.0, // P/L unknown
                        "GRACEFUL_SHUTDOWN",
                        &bundle_id,
                    ).await {
                        warn!("Failed to record emergency exit for {}: {}", position.mint, e);
                    }
                    
                    results.push((position.mint.clone(), Ok(bundle_id)));
                }
                Err(e) => {
                    error!("❌ Failed to close position for {}: {}", position.mint, e);
                    results.push((position.mint.clone(), Err(e)));
                }
            }
            
            // Small delay between sells to avoid overwhelming the network
            tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
        }

        let success_count = results.iter().filter(|(_, r)| r.is_ok()).count();
        info!("🛑 Graceful shutdown complete: {}/{} positions closed successfully", 
            success_count, results.len());
        
        results
    }

    /// Start monitoring a position for auto-sell
    async fn start_auto_sell_monitoring(
        &self, 
        mint: String, 
        entry_sol: f64, 
        token_amount: u64, 
        decimals: u8, 
        entry_1m_move: f64,
        liquidity_usd: f64,
        top_10_pct: f64,
        holder_count: u64,
        socials: u32
    ) {
        // Standardize Price to SOL per 1.0 Token (Fixes Unit Mismatch)
        let entry_price = if token_amount > 0 {
            let token_amount_whole = token_amount as f64 / 10f64.powf(decimals as f64);
            entry_sol / token_amount_whole
        } else {
            0.0
        };

        crate::trade_logger::log_position_started(&mint, entry_price, self.config.auto_sell_profit_target_pct, self.config.auto_sell_stop_loss_pct);

        let position = Position {
            mint: mint.clone(),
            entry_price_sol_per_token: entry_price, 
            entry_time: Instant::now(),
            amount_token_raw: token_amount,
            amount_sol_invested: (entry_sol * 1e9) as u64,
            decimals,
            highest_price_reached: entry_price,
            partial_exit_executed: false,
            remaining_amount_pct: 100.0,
            timeout_extensions: 0,
            entry_1m_move,
            liquidity_usd,
            top_10_pct,
            holder_count,
            socials,
        };

        let mut rx = self.position_tracker.monitor_position(position).await;
        let trade_engine = Arc::new(self.clone()); // Self reference for the task

        // Spawn a task to handle sell signals
        tokio::spawn(async move {
            while let Some(signal) = rx.recv().await {
                info!("🚨 SELL SIGNAL for {}: {} (P/L: {:.2}%)", signal.position.mint, signal.trigger, signal.pnl_percentage);
                
                let mut sell_success = false;
                
                // Execute Sell
                match trade_engine.execute_sell(&signal).await {
                    Ok(bundle_id) => {
                         sell_success = true;
                         // Calculate P/L in SOL (Cleaned units)
                        let token_amount_whole = signal.position.amount_token_raw as f64 / 10f64.powf(signal.position.decimals as f64);
                        let pnl_sol = token_amount_whole * signal.current_price_sol_per_token
                            - (signal.position.amount_sol_invested as f64 / 1_000_000_000.0);

                        // Record exit in DB
                        if let Err(e) = trade_engine.db.update_trade_exit(
                            &signal.position.mint,
                            signal.current_price_sol_per_token,
                            pnl_sol,
                            &signal.trigger.to_string(),
                            &bundle_id.clone()
                        ).await {
                            error!("Failed to record trade exit: {}", e);
                        }

                        // NEW: Record Post-Trade Analytics for AI training
                        let analytics = crate::db::TradeAnalytics {
                            mint: signal.position.mint.clone(),
                            entry_price: signal.position.entry_price_sol_per_token,
                            exit_price: signal.current_price_sol_per_token,
                            pnl_sol,
                            pnl_pct: signal.pnl_percentage,
                            liquidity_usd: signal.position.liquidity_usd,
                            top_10_pct: signal.position.top_10_pct,
                            holder_count: signal.position.holder_count,
                            socials: signal.position.socials,
                        };
                        
                        let db_for_analytics = trade_engine.db.clone();
                        tokio::spawn(async move {
                            if let Err(e) = db_for_analytics.record_trade_analytics(&analytics).await {
                                error!("Failed to record trade analytics for {}: {}", analytics.mint, e);
                            }
                        });
                        
                        // Update Circuit Breaker logic
                        if signal.pnl_percentage < 0.0 {
                            let val = trade_engine.consecutive_losses.fetch_add(1, Ordering::SeqCst) + 1;
                            warn!("📉 TRADING-WIDE: Consecutive loss count: {}", val);
                        } else if signal.pnl_percentage > 5.0 { // Significant win
                            trade_engine.consecutive_losses.store(0, Ordering::SeqCst);
                            info!("✅ TRADING-WIDE: Consecutive losses reset to 0");
                        }

                        // Log successful sell
                        log_sell(
                            &signal.position.mint, 
                            pnl_sol, 
                            signal.pnl_percentage, 
                            &signal.trigger.to_string(), 
                            &bundle_id
                        );

                        // Send Telegram Notification
                        if let Some(tele) = &trade_engine.tele {
                            let tele = tele.clone();
                            let mint = signal.position.mint.clone();
                            let trigger = signal.trigger.to_string();
                            let pnl_pct = signal.pnl_percentage;
                            let entry = signal.position.entry_price_sol_per_token;
                            let exit = signal.current_price_sol_per_token;
                            let pnl = pnl_sol;
                            let bid = bundle_id.clone();
                            
                            tokio::spawn(async move {
                                tele.notify_auto_sell(
                                    &mint,
                                    &trigger,
                                    pnl_pct,
                                    entry,
                                    exit,
                                    pnl,
                                    &bid
                                ).await;
                            });
                        }
                    },
                    Err(e) => {
                        error!("❌ Failed to execute SELL for {}: {}", signal.position.mint, e);
                        log_sell_failed(&signal.position.mint, &signal.trigger.to_string(), &e.to_string());
                    }
                }
                
                
                // Only stop monitoring if the sell was successfully sent
                if sell_success && matches!(signal.trigger, SellTrigger::StopLoss(_) | SellTrigger::ProfitTarget(_) | SellTrigger::Timeout | SellTrigger::ExtendedTimeout(_)) {
                    info!("✅ Trade lifecycle complete for {}. Stopping monitoring.", signal.position.mint);
                    trade_engine.position_tracker.remove_position(&signal.position.mint).await;
                    break; // Stop monitoring after full exit
                } else if !sell_success {
                    warn!("🔄 Sell failed for {}. Continuing to monitor/retry...", signal.position.mint);
                }
            }
        });
    }
    
    /// Resume monitoring for positions loaded from DB
    pub async fn resume_monitoring(&self) -> Result<()> {
        let positions = self.db.get_open_positions_state().await?;
        info!("🔄 Resuming monitoring for {} active positions", positions.len());
        
        for (mint, entry_price, amount, timestamp, high, ext, sol_invested, partial_exit, remaining_pct, decimals, entry_1m_move) in positions {
            let elapsed = chrono::Utc::now().timestamp() - timestamp;
            let entry_time = Instant::now() - std::time::Duration::from_secs(elapsed as u64);
            
            let mut final_amount = amount;
            
            // Auto-heal: If DB says 0 amount (due to previous bug), check actual wallet balance
            if final_amount == 0 {
                warn!("⚠️ Resuming position for {} has 0 tokens. Attempting to fetch actual balance...", mint);
                match self.presigner.get_token_balance(&mint).await {
                    Ok(bal) => {
                        if bal > 0 {
                           info!("✅ Recovered zombie position: {} has {} tokens. Updating state.", mint, bal);
                           final_amount = bal;
                           // We use this fresh balance for the Position struct so selling works
                        } else {
                           warn!("❌ Still 0 balance for {}. Skipping monitoring to avoid 'No Route' errors.", mint);
                           continue; // Skip monitoring this dead/failed position
                        }
                    },
                    Err(e) => {
                       warn!("❌ Failed to fetch balance for {}: {}. Skipping.", mint, e);
                       continue;
                    }
                }
            }

            let position = Position {
                mint: mint.clone(),
                entry_price_sol_per_token: entry_price,
                entry_time,
                amount_token_raw: final_amount,
                amount_sol_invested: sol_invested,
                decimals,
                highest_price_reached: high,
                partial_exit_executed: partial_exit,  // Restored from DB
                remaining_amount_pct: remaining_pct,   // Restored from DB
                timeout_extensions: ext,
                entry_1m_move,
                liquidity_usd: 0.0,
                top_10_pct: 0.0,
                holder_count: 0,
                socials: 0,
            };
            
            info!("🔄 Restoring position: {} (partial_exit: {}, remaining: {:.0}%)", 
                mint, partial_exit, remaining_pct);
            
            let mut rx = self.position_tracker.monitor_position(position).await;
            
            let trade_engine = Arc::new(self.clone()); // Self reference

            // Spawn listener (simplified version of above)
            tokio::spawn(async move {
                while let Some(signal) = rx.recv().await {
                    info!("🚨 SELL SIGNAL (Resumed) for {}: {}", signal.position.mint, signal.trigger);
                    
                    match trade_engine.execute_sell(&signal).await {
                        Ok(bundle_id) => {
                             // Calculate P/L in SOL (Cleaned units)
                             let token_amount_whole = signal.position.amount_token_raw as f64 / 10f64.powf(signal.position.decimals as f64);
                             let pnl_sol = token_amount_whole * signal.current_price_sol_per_token
                                - (signal.position.amount_sol_invested as f64 / 1_000_000_000.0);

                             if let Err(e) = trade_engine.db.update_trade_exit(
                                &signal.position.mint,
                                signal.current_price_sol_per_token,
                                pnl_sol, 
                                &signal.trigger.to_string(),
                                &bundle_id.clone()
                            ).await {
                                error!("Failed to record trade exit: {}", e);
                            }

                            // Record Post-Trade Analytics (Partial data for resumed)
                            let analytics = crate::db::TradeAnalytics {
                                mint: signal.position.mint.clone(),
                                entry_price: signal.position.entry_price_sol_per_token,
                                exit_price: signal.current_price_sol_per_token,
                                pnl_sol,
                                pnl_pct: signal.pnl_percentage,
                                liquidity_usd: signal.position.liquidity_usd,
                                top_10_pct: signal.position.top_10_pct,
                                holder_count: signal.position.holder_count,
                                socials: signal.position.socials,
                            };
                            
                            let db_for_analytics = trade_engine.db.clone();
                            tokio::spawn(async move {
                                let _ = db_for_analytics.record_trade_analytics(&analytics).await;
                            });

                            // Update Circuit Breaker logic
                            if signal.pnl_percentage < 0.0 {
                                let val = trade_engine.consecutive_losses.fetch_add(1, Ordering::SeqCst) + 1;
                                warn!("📉 TRADING-WIDE (Resumed): Consecutive loss count: {}", val);
                            } else if signal.pnl_percentage > 5.0 { // Significant win
                                trade_engine.consecutive_losses.store(0, Ordering::SeqCst);
                                info!("✅ TRADING-WIDE (Resumed): Consecutive losses reset to 0");
                            }

                            // Log successful sell (Resumed)
                            log_sell(
                                &signal.position.mint, 
                                pnl_sol, 
                                0.0, // P/L pct unknown for resumed positions without entry price tracking
                                &signal.trigger.to_string(), 
                                &bundle_id
                            );

                            // Send Telegram Notification (Resumed)
                            if let Some(tele) = &trade_engine.tele {
                                let tele = tele.clone();
                                let mint = signal.position.mint.clone();
                                let trigger = signal.trigger.to_string();
                                let pnl_pct = 0.0; // PnL pct unknown for resumed
                                let entry = signal.position.entry_price_sol_per_token;
                                let exit = signal.current_price_sol_per_token;
                                let pnl = pnl_sol;
                                let bid = bundle_id.clone();
                                
                                tokio::spawn(async move {
                                    tele.notify_auto_sell(
                                        &mint,
                                        &trigger,
                                        pnl_pct,
                                        entry,
                                        exit,
                                        pnl,
                                        &bid
                                    ).await;
                                });
                            }
                            
                             // 💡 Zombie Fix: Remove position after successful full exit (Resumed trades)
                             if matches!(signal.trigger, SellTrigger::StopLoss(_) | SellTrigger::ProfitTarget(_) | SellTrigger::Timeout | SellTrigger::ExtendedTimeout(_)) {
                                 trade_engine.position_tracker.remove_position(&signal.position.mint).await;
                                 break;
                             }
                        },
                        Err(e) => {
                             error!("❌ Failed to execute SELL (Resumed) for {}: {}", signal.position.mint, e);
                             log_sell_failed(&signal.position.mint, &signal.trigger.to_string(), &e.to_string());
                        }
                    }
                }
            });
        }
        
        Ok(())
    }
    pub fn get_consecutive_losses(&self) -> u32 {
        self.consecutive_losses.load(Ordering::SeqCst)
    }
}
