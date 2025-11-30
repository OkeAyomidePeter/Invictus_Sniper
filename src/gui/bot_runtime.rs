use anyhow::Result;
use log::{error, info};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;
use tokio::runtime::Runtime;

use crate::config::Config;
use crate::helius_listener;
use crate::enrichment;
use crate::scoring::TokenScorer;
// use crate::risk_engine::RiskEngine;
use crate::presigner::Presigner;
use crate::tx::TransactionManager;
use crate::db::Database;
use crate::tele::TelegramInterface;
use crate::position_tracker::{PositionTracker, Position, SellTrigger};
use crate::wallet_monitor::WalletMonitor;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum BotState {
    Stopped,
    Starting,
    Running,
    Stopping,
    Error,
}

#[derive(Clone)]
pub struct BotMetrics {
    pub wallet_balance_sol: f64,
    pub active_positions: usize,
    pub total_trades: i64,
    pub total_pnl_sol: f64,
    pub win_rate: f64,
    pub uptime_seconds: u64,
}

impl Default for BotMetrics {
    fn default() -> Self {
        Self {
            wallet_balance_sol: 0.0,
            active_positions: 0,
            total_trades: 0,
            total_pnl_sol: 0.0,
            win_rate: 0.0,
            uptime_seconds: 0,
        }
    }
}

pub enum BotEvent {
    StateChanged(BotState),
    MetricsUpdated(BotMetrics),
    LogMessage { level: String, message: String },
    Error(String),
}

pub struct BotRuntime {
    state: Arc<Mutex<BotState>>,
    runtime: Option<Runtime>,
    shutdown_tx: Option<mpsc::Sender<()>>,
    event_tx: mpsc::UnboundedSender<BotEvent>,
    metrics: Arc<Mutex<BotMetrics>>,
    start_time: Arc<Mutex<Option<std::time::Instant>>>,
}

impl BotRuntime {
    pub fn new(event_tx: mpsc::UnboundedSender<BotEvent>) -> Self {
        Self {
            state: Arc::new(Mutex::new(BotState::Stopped)),
            runtime: None,
            shutdown_tx: None,
            event_tx,
            metrics: Arc::new(Mutex::new(BotMetrics::default())),
            start_time: Arc::new(Mutex::new(None)),
        }
    }

    pub fn get_state(&self) -> BotState {
        *self.state.lock().unwrap()
    }

    pub fn get_metrics(&self) -> BotMetrics {
        let mut metrics = self.metrics.lock().unwrap().clone();
        
        // Update uptime
        if let Some(start) = *self.start_time.lock().unwrap() {
            metrics.uptime_seconds = start.elapsed().as_secs();
        }
        
        metrics
    }

    fn set_state(&mut self, new_state: BotState) {
        *self.state.lock().unwrap() = new_state;
        let _ = self.event_tx.send(BotEvent::StateChanged(new_state));
    }

    fn send_log(&self, level: &str, message: String) {
        let _ = self.event_tx.send(BotEvent::LogMessage {
            level: level.to_string(),
            message,
        });
    }

    pub fn start(&mut self, config: Config) -> Result<()> {
        if self.get_state() != BotState::Stopped {
            return Err(anyhow::anyhow!("Bot is already running or starting"));
        }

        self.set_state(BotState::Starting);
        self.send_log("INFO", "🚀 Starting Invictus Sniper Bot...".to_string());

        // Create new runtime
        let runtime = Runtime::new()?;
        let (shutdown_tx, mut shutdown_rx) = mpsc::channel::<()>(1);

        let state = self.state.clone();
        let event_tx = self.event_tx.clone();
        let metrics = self.metrics.clone();
        let start_time = self.start_time.clone();

        // Spawn bot logic in the runtime
        runtime.spawn(async move {
            if let Err(e) = run_bot_logic(
                config,
                &mut shutdown_rx,
                state.clone(),
                event_tx.clone(),
                metrics.clone(),
            ).await {
                error!("Bot error: {}", e);
                *state.lock().unwrap() = BotState::Error;
                let _ = event_tx.send(BotEvent::Error(format!("Bot error: {}", e)));
            }
        });

        self.runtime = Some(runtime);
        self.shutdown_tx = Some(shutdown_tx);
        *self.start_time.lock().unwrap() = Some(std::time::Instant::now());
        
        self.set_state(BotState::Running);
        self.send_log("INFO", "✅ Bot is now running".to_string());

        Ok(())
    }

    pub fn stop(&mut self) -> Result<()> {
        if self.get_state() != BotState::Running {
            return Err(anyhow::anyhow!("Bot is not running"));
        }

        self.set_state(BotState::Stopping);
        self.send_log("INFO", "🛑 Stopping bot...".to_string());

        // Send shutdown signal
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.blocking_send(());
        }

        // Drop the runtime to force shutdown
        self.runtime = None;
        *self.start_time.lock().unwrap() = None;

        self.set_state(BotState::Stopped);
        self.send_log("INFO", "✅ Bot stopped successfully".to_string());

        Ok(())
    }
}

async fn run_bot_logic(
    config: Config,
    shutdown_rx: &mut mpsc::Receiver<()>,
    state: Arc<Mutex<BotState>>,
    event_tx: mpsc::UnboundedSender<BotEvent>,
    metrics: Arc<Mutex<BotMetrics>>,
) -> Result<()> {
    // Start Helius Listener
    let classified_rx = helius_listener::start(&config).await?;
    let _ = event_tx.send(BotEvent::LogMessage {
        level: "INFO".to_string(),
        message: "✅ Helius listener started".to_string(),
    });

    // Start Enrichment Pipeline
    let mut enriched_rx = enrichment::start(&config, classified_rx).await?;
    let _ = event_tx.send(BotEvent::LogMessage {
        level: "INFO".to_string(),
        message: "✅ Enrichment pipeline started".to_string(),
    });

    // Initialize components
    let scorer = TokenScorer::new();
    // let risk_engine = RiskEngine::new(&config);
    let presigner = Arc::new(Presigner::new(&config));
    let tx_manager = TransactionManager::new(presigner.clone(), &config);
    let database = Arc::new(Database::new("sqlite://invictus.db").await?);
    
    // Initialize Wallet Monitor
    let wallet_monitor = Arc::new(WalletMonitor::new(
        format!("https://mainnet.helius-rpc.com/?api-key={}", config.helius_api_key),
        presigner.pubkey(),
        config.wallet_low_balance_alert_sol,
        config.wallet_reserve_for_fees_sol,
        config.wallet_monitor_interval_secs,
    ));
    
    let _wallet_alerts_rx = wallet_monitor.clone().start_monitoring();

    // Shutdown Channel for Telegram
    let (tele_shutdown_tx, mut tele_shutdown_rx) = mpsc::channel(1);

    // Initialize Telegram Interface
    let tele_interface = Arc::new(TelegramInterface::new(
        &config,
        database.clone(),
        tele_shutdown_tx,
        Some(wallet_monitor.clone()),
    ));
    let tele_for_spawn = tele_interface.as_ref().clone();
    tokio::spawn(async move { tele_for_spawn.run().await });

    // Initialize Position Tracker
    let position_tracker = PositionTracker::new(&config);

    let _ = event_tx.send(BotEvent::LogMessage {
        level: "INFO".to_string(),
        message: "⚡ System Operational - Waiting for opportunities".to_string(),
    });

    // Spawn metrics updater
    let metrics_clone = metrics.clone();
    let db_clone = database.clone();
    let wallet_clone = wallet_monitor.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
            
            if let Ok(balance) = wallet_clone.get_balance().await {
                let balance_sol = balance as f64 / 1_000_000_000.0;
                
                if let Ok((total_trades, _closed, total_pnl, win_rate, active_pos)) = 
                    db_clone.get_trade_statistics().await {
                    
                    let mut m = metrics_clone.lock().unwrap();
                    m.wallet_balance_sol = balance_sol;
                    m.active_positions = active_pos as usize;
                    m.total_trades = total_trades;
                    m.total_pnl_sol = total_pnl;
                    m.win_rate = win_rate;
                }
            }
        }
    });

    // Main event loop
    loop {
        tokio::select! {
            _ = shutdown_rx.recv() => {
                info!("🛑 Shutdown signal received");
                break;
            }
            _ = tele_shutdown_rx.recv() => {
                info!("💀 Kill signal from Telegram");
                break;
            }
            Some(enriched_token) = enriched_rx.recv() => {
                let score = scorer.score(&enriched_token);

                let log_msg = format!(
                    "✨ ENRICHED: {} | Liq: ${:.2} | Score: {:.1}/70",
                    enriched_token.mint,
                    enriched_token.initial_liquidity_sol.unwrap_or(0.0),
                    score
                );
                let _ = event_tx.send(BotEvent::LogMessage {
                    level: "INFO".to_string(),
                    message: log_msg,
                });

                if score > 50.0 {
                    let _ = event_tx.send(BotEvent::LogMessage {
                        level: "INFO".to_string(),
                        message: format!("🚀 HIGH SCORE: {} ({:.1}/70)", enriched_token.mint, score),
                    });

                    // Store in DB
                    if let Err(e) = database.store_token(&enriched_token, score).await {
                        error!("Failed to store token: {}", e);
                    }

                    // Risk verification and trading logic (same as main.rs)
                    // Direct Buy (Risk Engine Removed)
                    let _ = event_tx.send(BotEvent::LogMessage {
                        level: "INFO".to_string(),
                        message: format!("🚀 HIGH SCORE: {} ({:.1}/70) - EXECUTING IMMEDIATE BUY", enriched_token.mint, score),
                    });

                    // Execute BUY with new transaction builder
                    let buy_amount_sol_lamports = (config.max_trade_size_sol * 1_000_000_000.0) as u64;
                    let tip_lamports = tx_manager.calculate_tip(true); // High priority for buys
                    let slippage_bps = 300; // 3% slippage
                    
                    let _ = event_tx.send(BotEvent::LogMessage {
                        level: "INFO".to_string(),
                        message: format!("💰 Executing BUY for {} ({} SOL)", enriched_token.mint, config.max_trade_size_sol),
                    });
                    
                    // Build buy instructions
                    let buy_result = tx_manager.build_buy_instructions(
                        &enriched_token.mint,
                        buy_amount_sol_lamports,
                        slippage_bps,
                        crate::tx::DexRouter::Jupiter, // Use Jupiter for now
                        tip_lamports,
                    ).await;
                    
                    match buy_result {
                        Ok(instructions) => {
                            // Sign with presigner
                            match presigner.build_and_sign_tx(&instructions) {
                                Ok(signed_tx) => {
                                    // Send transaction (using existing RPC or Jito bundle logic)
                                    // For now, use presigner's send method
                                    match presigner.send_transaction(&signed_tx) {
                                        Ok(signature) => {
                                            let bundle_id = signature; // Signature as bundle ID
                                            let _ = event_tx.send(BotEvent::LogMessage {
                                                level: "INFO".to_string(),
                                                message: format!("🚀 BUY executed successfully! Signature: {}", bundle_id),
                                            });
                                            
                                            // Calculate entry price (estimated based on liquidity)
                                            let entry_price_estimate = if let Some(liq) = enriched_token.initial_liquidity_sol {
                                                liq / (enriched_token.supply.unwrap_or(1_000_000_000) as f64)
                                            } else {
                                                0.0
                                            };
                                            
                                            // Record trade in database
                                            if let Err(e) = database.record_trade(
                                                &enriched_token.mint,
                                                "BUY",
                                                0, // Token amount unknown until we query balance
                                                buy_amount_sol_lamports,
                                                &bundle_id,
                                                Some(&bundle_id),
                                                Some(entry_price_estimate),
                                            ).await {
                                                error!("Failed to record BUY trade: {}", e);
                                            }
                            
                            // Start auto-sell monitoring if enabled
                            if config.auto_sell_enabled {
                                let _ = event_tx.send(BotEvent::LogMessage {
                                    level: "INFO".to_string(),
                                    message: format!("📊 Starting auto-sell monitoring for {}", enriched_token.mint),
                                });
                                
                                // Sleep briefly to allow transaction to settle
                                tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;
                                
                                // Query token balance to get exact amount
                                // For now, estimate based on buy amount
                                let estimated_token_amount = (buy_amount_sol_lamports as f64 / entry_price_estimate) as u64;
                                
                                let position = Position {
                                    mint: enriched_token.mint.clone(),
                                    entry_price_sol_per_token: entry_price_estimate,
                                    entry_time: std::time::Instant::now(),
                                    amount_token_raw: estimated_token_amount,
                                    amount_sol_invested: buy_amount_sol_lamports,
                                    decimals: enriched_token.decimals,
                                };
                                
                                let mut sell_rx = position_tracker.monitor_position(position);
                                
                                // Spawn task to handle sell signal
                                let tx_manager_clone = tx_manager.clone();
                                let db_clone = database.clone();
                                let tele_clone = tele_interface.clone();
                                let config_clone = config.clone();
                                let event_tx_clone = event_tx.clone(); // Clone for the task
                                
                                tokio::spawn(async move {
                                    if let Some(sell_signal) = sell_rx.recv().await {
                                        let _ = event_tx_clone.send(BotEvent::LogMessage {
                                            level: "INFO".to_string(),
                                            message: format!("⚡ Sell trigger: {} for {}", sell_signal.trigger, sell_signal.position.mint),
                                        });
                                        
                                        // Execute SELL with new transaction builder (NO Jito tip)
                                        let sell_slippage = config_clone.auto_sell_slippage_bps;
                                        
                                        // Build sell instructions
                                        let sell_result = tx_manager_clone.build_sell_instructions(
                                            &sell_signal.position.mint,
                                            sell_signal.position.amount_token_raw,
                                            sell_slippage,
                                            crate::tx::DexRouter::Jupiter, // Use Jupiter for now
                                            true, // Close ATA to reclaim rent
                                        ).await;
                                        
                                        match sell_result {
                                            Ok(instructions) => {
                                                // Get presigner from outer scope (need to pass it)
                                                // For now, create a new one (not ideal but works)
                                                let presigner_sell = Arc::new(crate::presigner::Presigner::new(&config_clone));
                                                
                                                match presigner_sell.build_and_sign_tx(&instructions) {
                                                    Ok(signed_tx) => {
                                                        match presigner_sell.send_transaction(&signed_tx) {
                                                            Ok(signature) => {
                                                                let bundle_id = signature; // Use signature as bundle ID
                                                                let _ = event_tx_clone.send(BotEvent::LogMessage {
                                                                    level: "INFO".to_string(),
                                                                    message: format!("🎯 Auto-sell executed: {} (Signature: {})", sell_signal.position.mint, bundle_id),
                                                                });
                                                                
                                                                // Calculate P/L in SOL
                                                                let pnl_sol = (sell_signal.position.amount_token_raw as f64 * sell_signal.current_price_sol_per_token
                                                                    - sell_signal.position.amount_sol_invested as f64) / 1_000_000_000.0;
                                                                
                                                                // Update database with exit info
                                                                let trigger_str = match sell_signal.trigger {
                                                                    SellTrigger::ProfitTarget(_) => "PROFIT_TARGET",
                                                                    SellTrigger::StopLoss(_) => "STOP_LOSS",
                                                                    SellTrigger::Timeout => "TIMEOUT",
                                                                };
                                                                
                                                                if let Err(e) = db_clone.update_trade_exit(
                                                                    &sell_signal.position.mint,
                                                                    sell_signal.current_price_sol_per_token,
                                                                    pnl_sol,
                                                                    trigger_str,
                                                                ).await {
                                                                    error!("Failed to update trade exit: {}", e);
                                                                }
                                                                
                                                                // Send Telegram notification
                                                                tele_clone.notify_auto_sell(
                                                                    &sell_signal.position.mint,
                                                                    &sell_signal.trigger.to_string(),
                                                                    sell_signal.pnl_percentage,
                                                                    sell_signal.position.entry_price_sol_per_token,
                                                                    sell_signal.current_price_sol_per_token,
                                                                    pnl_sol,
                                                                    &bundle_id,
                                                                ).await;
                                                                
                                                                let _ = event_tx_clone.send(BotEvent::LogMessage {
                                                                    level: "INFO".to_string(),
                                                                    message: format!("💰 P/L: {:.4} SOL ({:.2}%)", pnl_sol, sell_signal.pnl_percentage),
                                                                });
                                                                            }
                                                                        Err(e) => {
                                                                            let _ = event_tx_clone.send(BotEvent::LogMessage {
                                                                                level: "ERROR".to_string(),
                                                                                message: format!("Failed to send SELL transaction for {}: {}", sell_signal.position.mint, e),
                                                                            });
                                                                        }
                                                                    }
                                                                }
                                                                Err(e) => {
                                                                    let _ = event_tx_clone.send(BotEvent::LogMessage {
                                                                        level: "ERROR".to_string(),
                                                                        message: format!("Failed to sign SELL transaction for {}: {}", sell_signal.position.mint, e),
                                                                    });
                                                                }
                                                            }
                                                        }
                                                        Err(e) => {
                                                            let _ = event_tx_clone.send(BotEvent::LogMessage {
                                                                level: "ERROR".to_string(),
                                                                message: format!("Failed to build SELL instructions for {}: {}", sell_signal.position.mint, e),
                                                            });
                                                        }
                                        }
                                    }
                                });
                            }
                                        }
                                        Err(e) => {
                                            let _ = event_tx.send(BotEvent::LogMessage {
                                                level: "ERROR".to_string(),
                                                message: format!("❌ Failed to send BUY transaction: {}", e),
                                            });
                                        }
                                    }
                                }
                                Err(e) => {
                                    let _ = event_tx.send(BotEvent::LogMessage {
                                        level: "ERROR".to_string(),
                                        message: format!("❌ Failed to sign BUY transaction: {}", e),
                                    });
                                }
                            }
                        }
                        Err(e) => {
                            let _ = event_tx.send(BotEvent::LogMessage {
                                level: "ERROR".to_string(),
                                message: format!("❌ Failed to build BUY instructions: {}", e),
                            });
                        }
                    }
                }
            }
        }
    }

    Ok(())
}
