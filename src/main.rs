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
mod watchlist;
mod trade_logger;
mod trade_engine;
mod health;
mod pnl_tracker;
mod moralis_client;

use trade_logger::{
    log_startup, log_shutdown, log_discovery, log_buy, log_buy_failed,
    log_watchlist_add, log_error, log_risk_rejected, log_token_rejected,
    TradeLogger 
};
use trade_engine::TradeEngine;

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
use watchlist::{Watchlist, BuySignal};

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
    let database = std::sync::Arc::new(Database::new(&config.database_url).await?);

    // Log startup
    log_startup(
        &presigner.pubkey().to_string(), 
        config.max_trade_size_sol, 
        config.min_liquidity_usd
    );

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
    let position_tracker = PositionTracker::new(&config, database.clone());

    // Initialize Watchlist
    let (buy_tx, mut buy_rx) = tokio::sync::mpsc::channel::<BuySignal>(100);
    let watchlist = std::sync::Arc::new(Watchlist::new(&config, buy_tx));
    
    // Start Watchlist Monitoring
    if config.dip_strategy_enabled {
        watchlist.start_monitoring().await;
        info!("👀 Dip Strategy ENABLED: High score tokens will be added to watchlist");
    } else {
        info!("⚡ Dip Strategy DISABLED: Instant buying mode active");
    }

    // Initialize TradeEngine
    let trade_engine = std::sync::Arc::new(TradeEngine::new(
        &config,
        database.clone(),
        presigner.clone(),
        std::sync::Arc::new(tx_manager.clone()),
        std::sync::Arc::new(position_tracker.clone()),
    ));

    // Resume monitoring for active positions
    if let Err(e) = trade_engine.resume_monitoring().await {
        error!("❌ Failed to resume position monitoring: {}", e);
    }

    info!("🚀 Sniper Bot Initialized & Running...");

    // Main Event Loop
    loop {
        tokio::select! {
            // Handle Shutdown Signal (Ctrl+C or Telegram Kill)
            _ = signal::ctrl_c() => {
                info!("🛑 Shutdown signal received (Ctrl+C). Closing all positions...");
                let results = trade_engine.close_all_positions().await;
                let success = results.iter().filter(|(_, r)| r.is_ok()).count();
                log_shutdown(results.len(), None);
                if !results.is_empty() {
                    info!("📊 Shutdown summary: {}/{} positions closed", success, results.len());
                }
                break;
            }
            _ = shutdown_rx.recv() => {
                info!("💀 Kill signal received from Telegram. Closing all positions...");
                let results = trade_engine.close_all_positions().await;
                let success = results.iter().filter(|(_, r)| r.is_ok()).count();
                log_shutdown(results.len(), None);
                if !results.is_empty() {
                    info!("📊 Shutdown summary: {}/{} positions closed", success, results.len());
                }
                break;
            }
            Some(enriched_token) = enriched_rx.recv() => {
                // Low latency scoring
                let score = scorer.score(&enriched_token);

                info!(
                    "✨ ENRICHED: {} | Liq: ${:.0} | Score: {:.1}/150",
                    enriched_token.mint,
                    enriched_token.liquidity_usd.unwrap_or(0.0),
                    score
                );
                TradeLogger::log(&format!("✨ ENRICHED: {} | Score: {:.1}/150", 
                    &enriched_token.mint[..12.min(enriched_token.mint.len())], score));
                
                // Thresholds (adjusted for new 150-point scale)
                // Buy: ~67% of max (high-conviction only)
                // Watchlist: ~47% of max (moderate potential)
                let buy_threshold = 100.0;
                let watchlist_threshold = 70.0;

                if score >= buy_threshold {
                    // RISK CHECK: Max Open Positions
                    let active_positions = position_tracker.get_active_positions().await;
                    if active_positions.len() >= config.max_open_positions {
                        warn!("⚠️ Skipping BUY for {}: Max open positions reached ({}/{})", 
                            enriched_token.mint, active_positions.len(), config.max_open_positions);
                        log_risk_rejected(&enriched_token.mint, "Max open positions reached");
                        continue;
                    }

                    // RISK CHECK: Total Exposure
                    let current_exposure: f64 = active_positions.iter().map(|p| p.amount_sol_invested as f64 / 1_000_000_000.0).sum();
                    if current_exposure + config.max_trade_size_sol > config.total_exposure_limit_sol {
                        warn!("⚠️ Skipping BUY for {}: Total exposure limit reached ({:.2}/{:.2} SOL)", 
                            enriched_token.mint, current_exposure, config.total_exposure_limit_sol);
                        log_risk_rejected(&enriched_token.mint, "Total exposure limit reached");
                        continue;
                    }

                    // RISK CHECK: Duplicate Token Protection
                    if active_positions.iter().any(|p| p.mint == enriched_token.mint) {
                        let reason = "Already have active position";
                        warn!("⚠️ Skipping BUY for {}: {}", enriched_token.mint, reason);
                        log_risk_rejected(&enriched_token.mint, reason);
                        continue;
                    }

                    // Direct Buy
                    info!("🚀 HIGH SCORE DETECTED: {} (Score: {:.1}/120) - EXECUTING IMMEDIATE BUY", enriched_token.mint, score);
                    log_discovery(&enriched_token.mint, score, enriched_token.liquidity_usd.unwrap_or(0.0));
                    
                    // Store in DB
                    if let Err(e) = database.store_token(&enriched_token, score).await {
                        error!("Failed to store token {}: {}", enriched_token.mint, e);
                    }

                    // Execute BUY via TradeEngine
                    if let Err(e) = trade_engine.execute_buy(&enriched_token, config.max_trade_size_sol, false).await {
                        error!("❌ Failed to execute BUY for {}: {}", enriched_token.mint, e);
                        log_error("BUY_EXECUTION", &e.to_string());
                    }

                } else if score >= watchlist_threshold {
                    // Add to Watchlist
                    info!("👀 Score {:.1} - Adding to Watchlist: {}", score, enriched_token.mint);
                    log_watchlist_add(&enriched_token.mint, score, enriched_token.price_sol.unwrap_or(0.0));
                    watchlist.add_token(enriched_token).await;
                } else {
                    info!("💤 Low Score {:.1} - Ignoring: {}", score, enriched_token.mint);
                    log_token_rejected(&enriched_token.mint, score, "Score below threshold");
                }
            }
            Some(buy_signal) = buy_rx.recv() => {
                // Handle Buy Signal from Watchlist
                let token = buy_signal.token;
                let msg = format!("🚀 WATCHLIST BUY TRIGGERED: {} | Price: {:.9} | Vol: ${:.0}", 
                    token.mint, buy_signal.current_price, buy_signal.volume_5m);
                info!("{}", msg);
                TradeLogger::log(&msg);

                // Execute BUY via TradeEngine
                if let Err(e) = trade_engine.execute_buy(&token, config.max_trade_size_sol, true).await {
                    error!("❌ Failed to execute WATCHLIST BUY for {}: {}", token.mint, e);
                    log_error("WATCHLIST_BUY", &e.to_string());
                }
            }
        }
    }

    info!("👋 Shutdown complete");
    Ok(())
}