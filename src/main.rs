mod config;
mod helius_listener;

use anyhow::Result;
use log::{error, info, warn};
use tokio::signal;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logger
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .format_timestamp_millis()
        .init();

    info!("🚀 Starting Solana Token Launch Monitor");
    info!("================================================");

    // Load configuration
    let config = config::Config::load();
    info!("✅ Configuration loaded successfully");

    // Validate Helius API key
    if config.helius_api_key.is_empty() {
        error!("❌ Helius API key is not set in configuration");
        return Err(anyhow::anyhow!("Missing Helius API key"));
    }

    info!("🔑 Helius API key configured: {}***", &config.helius_api_key[..8]);
    info!("================================================");

    // Start Helius listener
    info!("🎯 Starting Helius WebSocket listener...");
    let mut event_receiver = match helius_listener::start(&config).await {
        Ok(rx) => {
            info!("✅ Helius listener started successfully");
            rx
        }
        Err(e) => {
            error!("❌ Failed to start Helius listener: {}", e);
            return Err(e);
        }
    };

    info!("================================================");
    info!("👂 Listening for MINT and POOL CREATION events...");
    info!("================================================");

    // Set up graceful shutdown handler
    let shutdown_signal = signal::ctrl_c();
    tokio::pin!(shutdown_signal);

    // Main event processing loop
    loop {
        tokio::select! {
            // Handle incoming classified events
            Some(classified_event) = event_receiver.recv() => {
                match classified_event {
                    helius_listener::ClassifiedEvent::Mint(mint_event) => {
                        handle_mint_event(mint_event);
                    }
                    helius_listener::ClassifiedEvent::PoolCreation(pool_event) => {
                        handle_pool_creation_event(pool_event);
                    }
                }
            }

            // Handle Ctrl+C shutdown
            _ = &mut shutdown_signal => {
                info!("");
                info!("================================================");
                info!("🛑 Shutdown signal received");
                info!("📊 Cleaning up and exiting...");
                info!("================================================");
                break;
            }
        }
    }

    info!("✅ Application terminated gracefully");
    Ok(())
}

/// Handle mint events (new token created)
fn handle_mint_event(event: helius_listener::MintEvent) {
    info!("");
    info!("╔════════════════════════════════════════════════════════════════");
    info!("║ 🪙 NEW TOKEN MINT DETECTED");
    info!("╠════════════════════════════════════════════════════════════════");
    info!("║ Mint Address:  {}", event.mint);
    info!("║ Platform:      {}", event.platform);
    info!("║ Signature:     {}", event.signature);
    info!("║ Slot:          {}", event.slot);
    
    if let Some(decimals) = event.decimals {
        info!("║ Decimals:      {}", decimals);
    }
    
    if let Some(supply) = event.supply {
        info!("║ Supply:        {}", supply);
    }
    
    if let Some(timestamp) = event.timestamp {
        info!("║ Timestamp:     {}", timestamp);
    }
    
    info!("╚════════════════════════════════════════════════════════════════");
    info!("");

    // TODO: Add your token analysis logic here
    // - Fetch token metadata
    // - Check for rug pull indicators
    // - Analyze holder distribution
    // - Make buy decision
    
    analyze_and_decide_mint(event);
}

/// Handle pool creation events (token listed on DEX)
fn handle_pool_creation_event(event: helius_listener::PoolCreationEvent) {
    info!("");
    info!("╔════════════════════════════════════════════════════════════════");
    info!("║ 🏊 NEW POOL CREATION DETECTED");
    info!("╠════════════════════════════════════════════════════════════════");
    info!("║ DEX:           {}", event.dex);
    info!("║ Pool Address:  {}", event.pool_address);
    info!("║ Token Mint:    {}", event.token_mint);
    info!("║ Pair Token:    {}", event.pair_token);
    info!("║ Signature:     {}", event.signature);
    info!("║ Slot:          {}", event.slot);
    
    if let Some(timestamp) = event.timestamp {
        info!("║ Timestamp:     {}", timestamp);
    }
    
    info!("╚════════════════════════════════════════════════════════════════");
    info!("");

    // TODO: Add your pool analysis logic here
    // - Check liquidity amount
    // - Verify lock period
    // - Analyze initial price
    // - Make buy decision
    
    analyze_and_decide_pool(event);
}

/// Analyze mint event and decide whether to buy
fn analyze_and_decide_mint(event: helius_listener::MintEvent) {
    info!("🔍 Analyzing mint event for potential trade...");
    
    // Example decision logic
    match event.platform.as_str() {
        "Pump.fun" => {
            info!("📊 Pump.fun token detected - applying Pump.fun strategy");
            // TODO: Implement Pump.fun specific strategy
            // - Check bonding curve progress
            // - Analyze social signals
            // - Fast buy if criteria met
        }
        "Raydium Launchlab" => {
            info!("📊 Raydium Launchlab token detected - applying Raydium strategy");
            // TODO: Implement Raydium Launchlab strategy
            // - Wait for pool creation
            // - Check initial liquidity
            // - Buy on pool creation
        }
        "SPL Token" | "Token 2022" => {
            info!("📊 Standard SPL token detected - waiting for pool creation");
            // TODO: Wait for corresponding pool creation event
            // - Track this mint
            // - Buy when pool is created
        }
        _ => {
            warn!("⚠️  Unknown platform: {}", event.platform);
        }
    }
    
    // Example: Check if token meets criteria
    if let Some(decimals) = event.decimals {
        if decimals != 9 && decimals != 6 {
            warn!("⚠️  Unusual decimals ({}), skipping", decimals);
            return;
        }
    }
    
    // TODO: Implement actual buy logic
    // execute_buy_order(&event.mint, buy_amount, slippage);
}

/// Analyze pool creation event and decide whether to buy
fn analyze_and_decide_pool(event: helius_listener::PoolCreationEvent) {
    info!("🔍 Analyzing pool creation event for potential trade...");
    
    // Example decision logic
    match event.dex.as_str() {
        "Raydium AMM v4" | "Raydium CPMM" => {
            info!("📊 Raydium pool detected - FAST BUY opportunity");
            // TODO: Implement immediate buy logic
            // - This is often the best entry point
            // - Execute buy within same slot if possible
            // - Use high priority fee
        }
        "Orca Whirlpool" => {
            info!("📊 Orca Whirlpool pool detected");
            // TODO: Implement Orca strategy
        }
        "Pump.fun" => {
            info!("📊 Pump.fun graduation to Raydium detected");
            // TODO: Handle Pump.fun graduation
            // - Token graduated from bonding curve
            // - Usually good signal
        }
        _ => {
            info!("📊 Pool created on {}", event.dex);
        }
    }
    
    // Check if paired with SOL (most common)
    if event.pair_token == "So11111111111111111111111111111111111111112" {
        info!("✅ SOL pair confirmed - proceeding with analysis");
        // TODO: Fetch pool liquidity
        // TODO: Check if liquidity is locked
        // TODO: Execute buy if criteria met
    } else {
        warn!("⚠️  Non-SOL pair detected: {}", event.pair_token);
    }
    
    // TODO: Implement actual buy logic
    // execute_buy_order(&event.token_mint, buy_amount, slippage);
}

// ========== UTILITY FUNCTIONS ==========

/// Execute a buy order (placeholder)
#[allow(dead_code)]
fn execute_buy_order(mint: &str, amount_sol: f64, slippage_bps: u16) {
    info!("💰 Executing buy order:");
    info!("   Token: {}", mint);
    info!("   Amount: {} SOL", amount_sol);
    info!("   Slippage: {}%", slippage_bps as f64 / 100.0);
    
    // TODO: Implement actual buy execution
    // 1. Build swap transaction (Jupiter/Raydium)
    // 2. Set high priority fee for fast execution
    // 3. Sign and send transaction
    // 4. Monitor for confirmation
    // 5. Log trade results
}

/// Fetch token metadata from chain (placeholder)
#[allow(dead_code)]
async fn fetch_token_metadata(mint: &str) -> Result<TokenMetadata> {
    // TODO: Implement metadata fetching
    // - Use Metaplex metadata account
    // - Or use Helius API for parsed metadata
    
    Ok(TokenMetadata {
        name: "Unknown".to_string(),
        symbol: "???".to_string(),
        uri: None,
    })
}

/// Check for rug pull indicators (placeholder)
#[allow(dead_code)]
fn check_rug_pull_indicators(mint: &str) -> RugPullScore {
    // TODO: Implement rug pull detection
    // - Check if mint authority is revoked
    // - Check if freeze authority is revoked
    // - Analyze top holder concentration
    // - Check for suspicious patterns
    
    RugPullScore {
        score: 0.0,
        mint_authority_revoked: false,
        freeze_authority_revoked: false,
        top_10_holder_percentage: 0.0,
        liquidity_locked: false,
    }
}

// ========== DATA STRUCTURES ==========

#[allow(dead_code)]
struct TokenMetadata {
    name: String,
    symbol: String,
    uri: Option<String>,
}

#[allow(dead_code)]
struct RugPullScore {
    score: f64,
    mint_authority_revoked: bool,
    freeze_authority_revoked: bool,
    top_10_holder_percentage: f64,
    liquidity_locked: bool,
}