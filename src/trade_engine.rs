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
use std::time::Instant;
use tokio::sync::mpsc;

/// TradeEngine orchestrates the entire lifecycle of a trade:
/// Buy -> Monitor -> Sell
#[derive(Clone)]
pub struct TradeEngine {
    config: Config,
    db: Arc<Database>,
    presigner: Arc<Presigner>,
    tx_manager: Arc<TransactionManager>,
    position_tracker: Arc<PositionTracker>,
}

impl TradeEngine {
    pub fn new(
        config: &Config,
        db: Arc<Database>,
        presigner: Arc<Presigner>,
        tx_manager: Arc<TransactionManager>,
        position_tracker: Arc<PositionTracker>,
    ) -> Self {
        Self {
            config: config.clone(),
            db,
            presigner,
            tx_manager,
            position_tracker,
        }
    }

    /// Execute a buy and start monitoring the position
    pub async fn execute_buy(&self, token: &EnrichedToken, amount_sol: f64, is_watchlist: bool) -> Result<()> {
        info!("🤖 TradeEngine: Initiating BUY for {} (Amount: {:.4} SOL)", token.mint, amount_sol);
        log_buy_attempt(&token.mint, amount_sol, is_watchlist);

        let start_total = Instant::now();
        let amount_lamports = (amount_sol * 1_000_000_000.0) as u64;
        let slippage_bps = 200; // Default 2%
        
        // STEP 1: Calculate Tip
        let start_step = Instant::now();
        let tip_lamports = self.tx_manager.calculate_tip(true); // High priority for buys
        log_pipeline_step(&token.mint, "Calculate Tip", start_step.elapsed().as_millis(), true);

        // STEP 2: Build Buy Transaction
        let start_step = Instant::now();
        let mut buy_tx = match self.tx_manager.build_buy_transaction(
            &token.mint,
            amount_lamports,
            slippage_bps,
            DexRouter::Jupiter, // Default to Jupiter for now
            tip_lamports,
        ).await {
            Ok(tx) => {
                log_pipeline_step(&token.mint, "Build Buy Tx", start_step.elapsed().as_millis(), true);
                tx
            },
            Err(e) => {
                log_pipeline_step(&token.mint, "Build Buy Tx", start_step.elapsed().as_millis(), false);
                log_error_detailed(&token.mint, "Build Buy Tx", &e.to_string());
                return Err(e);
            }
        };

        // STEP 3: Build Tip Transaction
        let start_step = Instant::now();
        let recent_blockhash = self.presigner.get_blockhash();
        let mut tip_tx = match self.tx_manager.build_tip_transaction(tip_lamports, recent_blockhash) {
            Ok(tx) => {
                log_pipeline_step(&token.mint, "Build Tip Tx", start_step.elapsed().as_millis(), true);
                tx
            },
            Err(e) => {
                log_pipeline_step(&token.mint, "Build Tip Tx", start_step.elapsed().as_millis(), false);
                log_error_detailed(&token.mint, "Build Tip Tx", &e.to_string());
                return Err(e);
            }
        };

        // STEP 4: Sign Transactions
        let start_step = Instant::now();
        // Update blockhash for buy tx to match tip tx (critical for bundles)
        buy_tx.message.set_recent_blockhash(recent_blockhash);
        
        if let Err(e) = self.presigner.sign_versioned_tx(&mut buy_tx) {
             log_pipeline_step(&token.mint, "Sign Buy Tx", start_step.elapsed().as_millis(), false);
             log_error_detailed(&token.mint, "Sign Buy Tx", &e.to_string());
             return Err(e);
        }
        if let Err(e) = self.presigner.sign_versioned_tx(&mut tip_tx) {
             log_pipeline_step(&token.mint, "Sign Tip Tx", start_step.elapsed().as_millis(), false);
             log_error_detailed(&token.mint, "Sign Tip Tx", &e.to_string());
             return Err(e);
        }
        log_pipeline_step(&token.mint, "Sign Txs", start_step.elapsed().as_millis(), true);

        // STEP 5: Send (Standard vs Jito)
        let mode = self.tx_manager.transaction_mode();
        let bundle_id = if mode == crate::config::TransactionMode::Standard {
            // STEP 5a: Send Standard Transaction
            let start_step = Instant::now();
            let sig = match self.presigner.send_versioned_transaction(&buy_tx) {
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
            // STEP 5b: Send Jito Bundle
            let start_step = Instant::now();
            // Build Tip Transaction (Jito Only)
            let recent_blockhash = self.presigner.get_blockhash();
            let mut tip_tx = self.tx_manager.build_tip_transaction(tip_lamports, recent_blockhash)?;
            buy_tx.message.set_recent_blockhash(recent_blockhash);
            
            self.presigner.sign_versioned_tx(&mut buy_tx)?;
            self.presigner.sign_versioned_tx(&mut tip_tx)?;

            let id = match self.presigner.send_jito_bundle(vec![buy_tx, tip_tx]).await {
                Ok(id) => {
                    log_pipeline_step(&token.mint, "Send Bundle", start_step.elapsed().as_millis(), true);
                    id
                },
                Err(e) => {
                    log_pipeline_step(&token.mint, "Send Bundle", start_step.elapsed().as_millis(), false);
                    log_error_detailed(&token.mint, "Send Bundle", &e.to_string());
                    return Err(e);
                }
            };
            
            info!("🚀 Buy Bundle Sent! ID: {}", id);
            log_bundle_sent(&id, 2);

            // Wait for confirmation (Jito Only - standard uses confirm_transaction inside send_and_confirm)
            let start_confirm = Instant::now();
            let status = self.presigner.wait_for_bundle_confirmation(&id, 30).await?;
            if !status.is_success() {
                 return Err(anyhow::anyhow!("Bundle failed: {:?}", status));
            }
            log_pipeline_step(&token.mint, "Confirm Bundle", start_confirm.elapsed().as_millis(), true);
            id
        };

        // STEP 7: Verify Position & Record to DB
        let start_step = Instant::now();
        // Fetch actual token balance - MUST succeed to confirm trade executed
        let token_amount = self.presigner.get_token_balance(&token.mint).await
            .context(format!("Failed to verify token balance after buy for {}. Transaction may have failed.", token.mint))?;
        
        if token_amount == 0 {
            log_pipeline_step(&token.mint, "Verify Balance", start_step.elapsed().as_millis(), false);
            log_error_detailed(&token.mint, "Verify Balance", "Balance is 0 after confirmed bundle");
            return Err(anyhow::anyhow!("Token balance is 0 after confirmed bundle - transaction may have failed"));
        }
        log_pipeline_step(&token.mint, "Verify Balance", start_step.elapsed().as_millis(), true);
        
        // Record Trade
        self.db.record_trade(
            &token.mint,
            "BUY",
            token_amount,
            amount_lamports,
            &bundle_id,
            Some(&bundle_id),
            Some(amount_sol / 1.0), // Placeholder price
        ).await?;

        // Log successful buy
        log_buy(&token.mint, amount_sol, &bundle_id);
        log_pipeline_step(&token.mint, "Total Buy Flow", start_total.elapsed().as_millis(), true);

        // 7. Start Monitoring (if auto-sell enabled)
        if self.config.auto_sell_enabled {
            log_position_started(
                &token.mint, 
                amount_sol / (token_amount as f64 / 1e6),  // Entry price per token
                self.config.auto_sell_profit_target_pct,
                self.config.auto_sell_stop_loss_pct
            );
            self.start_auto_sell_monitoring(token.mint.clone(), amount_sol, token_amount).await;
        }

        Ok(())
    }

    /// Execute a sell transaction
    async fn execute_sell(&self, signal: &SellSignal) -> Result<String> {
        info!("🤖 TradeEngine: Initiating SELL for {} (Trigger: {})", signal.position.mint, signal.trigger);

        let amount_token = signal.position.amount_token_raw;
        let slippage_bps = self.config.auto_sell_slippage_bps;
        let tip_lamports = self.tx_manager.calculate_tip(false); // Lower priority than buy, but still tipped

        // 1. Send (Standard vs Jito)
        let mode = self.tx_manager.transaction_mode();
        let bundle_id = if mode == crate::config::TransactionMode::Standard {
            // 1a. Build & Send Standard Transaction
            let sell_tx = self.tx_manager.build_sell_transaction(
                &signal.position.mint,
                amount_token,
                slippage_bps,
                DexRouter::Jupiter,
                true, // Close ATA
            ).await?;

            let sig = self.presigner.send_versioned_transaction(&sell_tx)?;
            info!("🚀 Standard SELL Sent! Sig: {}", sig);
            crate::trade_logger::log_priority_fees(&signal.position.mint, self.config.priority_fee_lamports);
            sig
        } else {
            // 1b. Build & Send Jito Bundle
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
            self.presigner.sign_versioned_tx(&mut sell_tx)?;
            self.presigner.sign_versioned_tx(&mut tip_tx)?;

            let id = self.presigner.send_jito_bundle(vec![sell_tx, tip_tx]).await?;
            info!("🚀 Sell Bundle Sent! ID: {}", id);
            log_bundle_sent(&id, 2);
            id
        };
        
        Ok(bundle_id)
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
    async fn start_auto_sell_monitoring(&self, mint: String, entry_sol: f64, token_amount: u64) {
        info!("📊 Starting auto-sell monitoring for {}", mint);
        
        let entry_price = if token_amount > 0 {
            (entry_sol * 1e9) / token_amount as f64
        } else {
            0.0
        };

        let position = Position {
            mint: mint.clone(),
            entry_price_sol_per_token: entry_price, 
            entry_time: Instant::now(),
            amount_token_raw: token_amount,
            amount_sol_invested: (entry_sol * 1e9) as u64,
            decimals: 6, // Default, should fetch
            highest_price_reached: 0.0,
            partial_exit_executed: false,
            remaining_amount_pct: 100.0,
            timeout_extensions: 0,
        };

        let mut rx = self.position_tracker.monitor_position(position).await;
        let trade_engine = Arc::new(self.clone()); // Self reference for the task

        // Spawn a task to handle sell signals
        tokio::spawn(async move {
            while let Some(signal) = rx.recv().await {
                info!("🚨 SELL SIGNAL for {}: {} (P/L: {:.2}%)", signal.position.mint, signal.trigger, signal.pnl_percentage);
                
                // Execute Sell
                match trade_engine.execute_sell(&signal).await {
                    Ok(bundle_id) => {
                         // Calculate P/L in SOL
                        let pnl_sol = (signal.position.amount_token_raw as f64 * signal.current_price_sol_per_token
                            - signal.position.amount_sol_invested as f64) / 1_000_000_000.0;

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
                        
                        // Log successful sell
                        log_sell(
                            &signal.position.mint, 
                            pnl_sol, 
                            signal.pnl_percentage, 
                            &signal.trigger.to_string(), 
                            &bundle_id
                        );
                    },
                    Err(e) => {
                        error!("❌ Failed to execute SELL for {}: {}", signal.position.mint, e);
                        log_sell_failed(&signal.position.mint, &signal.trigger.to_string(), &e.to_string());
                    }
                }
                
                if matches!(signal.trigger, SellTrigger::StopLoss(_) | SellTrigger::ProfitTarget(_) | SellTrigger::Timeout) {
                    break; // Stop monitoring after full exit
                }
            }
        });
    }
    
    /// Resume monitoring for positions loaded from DB
    pub async fn resume_monitoring(&self) -> Result<()> {
        let positions = self.db.get_open_positions_state().await?;
        info!("🔄 Resuming monitoring for {} active positions", positions.len());
        
        for (mint, entry_price, amount, timestamp, high, ext, sol_invested, partial_exit, remaining_pct) in positions {
            let elapsed = chrono::Utc::now().timestamp() - timestamp;
            let entry_time = Instant::now() - std::time::Duration::from_secs(elapsed as u64);
            
            let position = Position {
                mint: mint.clone(),
                entry_price_sol_per_token: entry_price,
                entry_time,
                amount_token_raw: amount,
                amount_sol_invested: sol_invested,
                decimals: 6, // TODO: Store decimals in DB
                highest_price_reached: high,
                partial_exit_executed: partial_exit,  // Restored from DB
                remaining_amount_pct: remaining_pct,   // Restored from DB
                timeout_extensions: ext,
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
                             let pnl_sol = (signal.position.amount_token_raw as f64 * signal.current_price_sol_per_token
                                - signal.position.amount_sol_invested as f64) / 1_000_000_000.0;

                             if let Err(e) = trade_engine.db.update_trade_exit(
                                &signal.position.mint,
                                signal.current_price_sol_per_token,
                                pnl_sol, 
                                &signal.trigger.to_string(),
                                &bundle_id.clone()
                            ).await {
                                error!("Failed to record trade exit: {}", e);
                            }

                            // Log successful sell (Resumed)
                            log_sell(
                                &signal.position.mint, 
                                pnl_sol, 
                                0.0, // P/L pct unknown for resumed positions without entry price tracking
                                &signal.trigger.to_string(), 
                                &bundle_id
                            );
                        },
                        Err(e) => {
                             error!("❌ Failed to execute SELL (Resumed) for {}: {}", signal.position.mint, e);
                             log_sell_failed(&signal.position.mint, &signal.trigger.to_string(), &e.to_string());
                        }
                    }

                    if matches!(signal.trigger, SellTrigger::StopLoss(_) | SellTrigger::ProfitTarget(_) | SellTrigger::Timeout) {
                        break;
                    }
                }
            });
        }
        
        Ok(())
    }
}
