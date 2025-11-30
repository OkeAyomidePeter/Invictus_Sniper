mod config;
mod helius_listener;
mod enrichment;
mod scoring;

mod presigner;
mod tx;
mod db;
mod tele;
mod position_tracker;
mod rate_limiter;
mod retry;
mod wallet_monitor;

use anyhow::Result;
use log::{error, info, warn};
use tokio::signal;
use scoring::TokenScorer;

use presigner::Presigner;
use tx::TransactionManager;
use db::Database;
use tele::TelegramInterface;
use config::Config;
use position_tracker::{PositionTracker, Position};

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logger
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .format_timestamp_millis()
        .init();

    info!("🚀 Starting Invictus Sniper Bot");
    info!("================================================");

    // 1. Load Configuration
    let config = config::Config::load();
    info!("✅ Configuration loaded successfully");
    
    // Display config (masked)
    info!("{}", config.display());

    // 2. Start Helius Listener (Producer)
    // Returns a receiver for ClassifiedEvent
    info!("🎯 Starting Helius Listener...");
    let classified_rx = match helius_listener::start(&config).await {
        Ok(rx) => {
            info!("✅ Helius listener started");
            rx
        }
        Err(e) => {
            error!("❌ Failed to start Helius listener: {}", e);
            return Err(e);
        }
    };

    // 3. Start Enrichment Pipeline (Processor)
    // Takes ClassifiedEvent receiver, returns EnrichedToken receiver
    info!("🔍 Starting Enrichment Pipeline...");
    let mut enriched_rx = match enrichment::start(&config, classified_rx).await {
        Ok(rx) => {
            info!("✅ Enrichment pipeline started");
            rx
        }
        Err(e) => {
            error!("❌ Failed to start enrichment pipeline: {}", e);
            return Err(e);
        }
    };

    // 4. Initialize Scorer
    let scorer = scoring::TokenScorer::new();
    info!("⚖️  Token Scorer initialized");

    // 4. Initialize Scorer (Moved to before main loop)
    // 5. Main Event Loop (Consumer)
    // Process enriched tokens (Logging only for now)
    let shutdown_signal = signal::ctrl_c();
    tokio::pin!(shutdown_signal);

    info!("================================================");
    info!("⚡ System Operational - Waiting for opportunities");
    info!("================================================");

    // Initialize Scorer
    let scorer = TokenScorer::new();
    
    // Initialize Risk Engine


    // Initialize Presigner (Fast Tx Builder)
    let presigner = std::sync::Arc::new(Presigner::new(&config));
    
    // Initialize Transaction Manager (Jito)
    let tx_manager = TransactionManager::new(presigner.clone(), &config);

    // Initialize Database
    let database = std::sync::Arc::new(Database::new("sqlite://invictus.db").await?);

    // Initialize Wallet Monitor
    let wallet_monitor = std::sync::Arc::new(wallet_monitor::WalletMonitor::new(
        format!("https://mainnet.helius-rpc.com/?api-key={}", config.helius_api_key),
        presigner.pubkey(),
        config.wallet_low_balance_alert_sol,
        config.wallet_reserve_for_fees_sol,
        config.wallet_monitor_interval_secs,
    ));
    
    // Start wallet monitoring in background
    let _wallet_alerts_rx = wallet_monitor.clone().start_monitoring();

    // Shutdown Channel
    let (shutdown_tx, mut shutdown_rx) = tokio::sync::mpsc::channel(1);

    // Initialize Telegram Interface with wallet monitor
    let tele_interface = std::sync::Arc::new(TelegramInterface::new(
        &config, 
        database.clone(), 
        shutdown_tx,
        Some(wallet_monitor.clone())
    ));
    let tele_for_spawn = tele_interface.as_ref().clone();
    tokio::spawn(async move { tele_for_spawn.run().await });

    // Initialize Position Tracker
    let position_tracker = PositionTracker::new(&config);

    info!("🚀 Sniper Bot Initialized & Running...");

    // Main Event Loop
    loop {
        tokio::select! {
            // Handle Shutdown Signal (Ctrl+C or Telegram Kill)
            _ = signal::ctrl_c() => {
                info!("🛑 Shutdown signal received (Ctrl+C). Exiting...");
                break;
            }
            _ = shutdown_rx.recv() => {
                info!("💀 Kill signal received from Telegram. Exiting...");
                break;
            }
            Some(enriched_token) = enriched_rx.recv() => {
                // Low latency scoring
                let score = scorer.score(&enriched_token);

                info!(
                    "✨ ENRICHED: {} | Liq: ${} | Score: {:.1}/70",
                    enriched_token.mint,
                    enriched_token.initial_liquidity_sol.unwrap_or(0.0),
                    score
                );
                
                // Threshold: >50/70 (71%) for buy consideration (graduated tokens only)
                if score > 50.0 {
                    // Direct Buy (Risk Engine Removed for Speed)
                    info!("🚀 HIGH SCORE DETECTED: {} (Score: {:.1}/70) - EXECUTING IMMEDIATE BUY", enriched_token.mint, score);
                    
                    // Store in DB
                    if let Err(e) = database.store_token(&enriched_token, score).await {
                        error!("Failed to store token {}: {}", enriched_token.mint, e);
                    }

                    // Execute BUY with new transaction builder
                    let buy_amount_sol_lamports = (config.max_trade_size_sol * 1_000_000_000.0) as u64;
                    let tip_lamports = tx_manager.calculate_tip(true); // High priority
                    let slippage_bps = 300; // 3% slippage
                    
                    info!("💰 Executing BUY for {} ({} SOL)", enriched_token.mint, config.max_trade_size_sol);
                    
                    // Build buy instructions
                    let buy_result = tx_manager.build_buy_instructions(
                        &enriched_token.mint,
                        buy_amount_sol_lamports,
                        slippage_bps,
                        tx::DexRouter::Jupiter,
                        tip_lamports,
                    ).await;
                    
                    match buy_result {
                        Ok(instructions) => {
                            match presigner.build_and_sign_tx(&instructions) {
                                Ok(signed_tx) => {
                                    match presigner.send_transaction(&signed_tx) {
                                        Ok(signature) => {
                                            let bundle_id = signature;
                                            info!("🚀 BUY executed successfully! Signature: {}", bundle_id);
                                            
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
                                info!("📊 Starting auto-sell monitoring for {}", enriched_token.mint);
                                
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
                                
                                tokio::spawn(async move {
                                    if let Some(sell_signal) = sell_rx.recv().await {
                                        info!("⚡ Sell trigger: {} for {}", sell_signal.trigger, sell_signal.position.mint);
                                        
                                        // Execute SELL with new transaction builder (NO Jito tip)
                                        let sell_slippage = config_clone.auto_sell_slippage_bps;
                                        
                                        let sell_result = tx_manager_clone.build_sell_instructions(
                                            &sell_signal.position.mint,
                                            sell_signal.position.amount_token_raw,
                                            sell_slippage,
                                            tx::DexRouter::Jupiter,
                                            true, // Close ATA
                                        ).await;
                                        
                                        match sell_result {
                                            Ok(instructions) => {
                                                let presigner_sell = std::sync::Arc::new(presigner::Presigner::new(&config_clone));
                                                match presigner_sell.build_and_sign_tx(&instructions) {
                                                    Ok(signed_tx) => {
                                                        match presigner_sell.send_transaction(&signed_tx) {
                                                            Ok(signature) => {
                                                                let bundle_id = signature;
                                                                info!("🎯 Auto-sell executed: {} (Signature: {})", sell_signal.position.mint, bundle_id);
                                                                
                                                                // Calculate P/L in SOL
                                                                let pnl_sol = (sell_signal.position.amount_token_raw as f64 * sell_signal.current_price_sol_per_token
                                                                    - sell_signal.position.amount_sol_invested as f64) / 1_000_000_000.0;
                                                                
                                                                // Update database with exit info
                                                                let trigger_str = match sell_signal.trigger {
                                                                    position_tracker::SellTrigger::ProfitTarget(_) => "PROFIT_TARGET",
                                                                    position_tracker::SellTrigger::StopLoss(_) => "STOP_LOSS",
                                                                    position_tracker::SellTrigger::Timeout => "TIMEOUT",
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
                                                                
                                                                info!("💰 P/L: {:.4} SOL ({:.2}%)", pnl_sol, sell_signal.pnl_percentage);
                                                                            }
                                                                        Err(e) => {
                                                                            error!("Failed to send SELL transaction for {}: {}", sell_signal.position.mint, e);
                                                                        }
                                                                    }
                                                                }
                                                                Err(e) => {
                                                                    error!("Failed to sign SELL transaction for {}: {}", sell_signal.position.mint, e);
                                                                }
                                                            }
                                                        }
                                                        Err(e) => {
                                                            error!("Failed to build SELL instructions for {}: {}", sell_signal.position.mint, e);
                                                        }
                                        }
                                    }
                                });
                            }
                                        }
                                        Err(e) => {
                                            error!("❌ Failed to send BUY transaction: {}", e);
                                        }
                                    }
                                }
                                Err(e) => {
                                    error!("❌ Failed to sign BUY transaction: {}", e);
                                }
                            }
                        }
                        Err(e) => {
                            error!("❌ Failed to build BUY instructions: {}", e);
                        }
                    }
                }
            }
            _ = &mut shutdown_signal => {
                info!("🛑 Shutdown signal received");
                break;
            }
        }
    }

    info!("👋 Shutdown complete");
    Ok(())
}