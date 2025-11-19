mod config;
mod helius_listener;
mod enrichment;

use anyhow::Result;
use enrichment::{EnrichedToken, TokenEnricher};
use helius_listener::ClassifiedEvent;
use log::{error, info, warn};
use tokio::signal;
use tokio::sync::mpsc;
use tokio::time::{interval, Duration};

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logger
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .format_timestamp_millis()
        .init();

    info!("🚀 Starting Solana Token Launch Monitor with Enrichment");
    info!("================================================");

    // Load configuration
    let config = config::Config::load();
    info!("✅ Configuration loaded successfully");

    // Validate configuration
    validate_config(&config)?;

    info!("🔑 Helius API key configured: {}***", &config.helius_api_key[..8]);
    info!("🔑 Private key configured: {}***", &config.private_key[..8]); 
    info!("🔑 Telegram Bot Token configured: {}***", &config.telegram_token[..8]);
    info!("🔑 Telegram Chat Id configured: {}***", &config.telegram_chat_id[..5]);
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

    // Create token enricher with caching
    info!("🔍 Initializing token enricher with 5-minute cache...");
    let enricher = TokenEnricher::new(config.helius_api_key.clone());
    info!("✅ Token enricher initialized");

    // Create channel for enriched tokens
    let (enriched_tx, mut enriched_rx) = mpsc::channel::<EnrichedToken>(100);

    // Spawn enrichment worker
    let enricher_clone = enricher.clone();
    tokio::spawn(async move {
        info!("🔄 Enrichment worker started");
        while let Some(event) = event_receiver.recv().await {
            let event_sig = event.signature().to_string();
            info!("📥 Received event: {}", event_sig);
            
            match enricher_clone.enrich_event(&event).await {
                Ok(enriched) => {
                    info!("✅ Successfully enriched: {} ({})", 
                        enriched.mint, 
                        enriched.symbol.as_deref().unwrap_or("NO_SYMBOL")
                    );
                    
                    if let Err(e) = enriched_tx.send(enriched).await {
                        error!("❌ Failed to send enriched token: {}", e);
                        break;
                    }
                }
                Err(e) => {
                    error!("❌ Failed to enrich event {}: {}", event_sig, e);
                }
            }
        }
        warn!("⚠️ Enrichment worker stopped");
    });

    // Spawn cache cleanup task (every 5 minutes)
    let enricher_clone = enricher.clone();
    tokio::spawn(async move {
        let mut cleanup_interval = interval(Duration::from_secs(300));
        loop {
            cleanup_interval.tick().await;
            enricher_clone.clean_expired_cache().await;
            
            let stats = enricher_clone.get_cache_stats().await;
            info!("🧹 Cache cleanup: {} total entries cached", stats.total_entries);
        }
    });

    // Spawn cache stats reporter (every 30 seconds)
    let enricher_clone = enricher.clone();
    tokio::spawn(async move {
        let mut stats_interval = interval(Duration::from_secs(30));
        loop {
            stats_interval.tick().await;
            let stats = enricher_clone.get_cache_stats().await;
            if stats.total_entries > 0 {
                info!("📊 Cache Stats: Metadata={}, Accounts={}, Social={}, Holders={}, Total={}", 
                    stats.metadata_entries,
                    stats.account_info_entries,
                    stats.social_links_entries,
                    stats.holder_count_entries,
                    stats.total_entries
                );
            }
        }
    });

    info!("================================================");
    info!("👂 Listening for MINT and POOL CREATION events...");
    info!("🔍 All events will be enriched with full metadata");
    info!("================================================");

    // Set up graceful shutdown handler
    let shutdown_signal = signal::ctrl_c();
    tokio::pin!(shutdown_signal);

    // Main event processing loop
    loop {
        tokio::select! {
            // Handle enriched tokens
            Some(enriched_token) = enriched_rx.recv() => {
                handle_enriched_token(enriched_token).await;
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

/// Validate configuration
fn validate_config(config: &config::Config) -> Result<()> {
    if config.helius_api_key.is_empty() {
        error!("❌ Helius API key is not set in configuration");
        return Err(anyhow::anyhow!("Missing Helius API key"));
    }
    if config.private_key.is_empty() {
        error!("❌ No private key is setup");
        return Err(anyhow::anyhow!("Missing private key"));
    }
    if config.telegram_token.is_empty() {
        error!("❌ No telegram bot token is setup");
        return Err(anyhow::anyhow!("Missing telegram bot token"));
    }
    if config.telegram_chat_id.is_empty() {
        error!("❌ No telegram_chat_id is setup");
        return Err(anyhow::anyhow!("Missing telegram_chat_id"));
    }
    Ok(())
}

/// Handle enriched token - main decision logic
async fn handle_enriched_token(token: EnrichedToken) {
    info!("");
    info!("╔════════════════════════════════════════════════════════════════");
    info!("║ 🎯 ENRICHED TOKEN ANALYSIS");
    info!("╠════════════════════════════════════════════════════════════════");
    info!("║ Mint:          {}", token.mint);
    info!("║ Symbol:        {}", token.symbol.as_deref().unwrap_or("N/A"));
    info!("║ Name:          {}", token.name.as_deref().unwrap_or("N/A"));
    info!("║ Platform:      {}", token.platform);
    info!("║ Signature:     {}", token.signature);
    info!("╠════════════════════════════════════════════════════════════════");
    
    // Token details
    if let Some(decimals) = token.decimals {
        info!("║ Decimals:      {}", decimals);
    }
    if let Some(supply) = token.supply {
        info!("║ Supply:        {}", format_number(supply as f64));
    }
    if let Some(holders) = token.holders {
        info!("║ Holders:       {}", holders);
    }
    
    info!("╠════════════════════════════════════════════════════════════════");
    
    // Authority info (CRITICAL for safety)
    info!("║ 🔐 AUTHORITY CHECK:");
    info!("║   Freeze Auth: {}", if token.has_freeze_authority { "❌ YES (RISKY)" } else { "✅ NONE" });
    info!("║   Mint Auth:   {}", if token.has_mint_authority { "❌ YES (RISKY)" } else { "✅ NONE" });
    info!("║   Mutable:     {}", if token.is_mutable { "⚠️ YES" } else { "✅ NO" });
    
    info!("╠════════════════════════════════════════════════════════════════");
    
    // Liquidity info
    if token.has_liquidity {
        info!("║ 💧 LIQUIDITY:");
        if let Some(pool) = &token.pool_info {
            info!("║   DEX:          {}", pool.dex);
            info!("║   Pool:         {}", pool.pool_address);
            info!("║   Base Reserve: {}", format_number(pool.base_reserve));
            info!("║   Quote Reserve: {}", format_number(pool.quote_reserve));
            if let Some(liq_usd) = pool.liquidity_usd {
                info!("║   Liquidity:    ${}", format_number(liq_usd));
            }
        }
    } else {
        info!("║ 💧 Liquidity:   ❌ NO POOL YET");
    }
    
    info!("╠════════════════════════════════════════════════════════════════");
    
    // Market data
    if let Some(price) = token.price_usd {
        info!("║ 💰 MARKET DATA:");
        info!("║   Price:       ${:.10}", price);
        if let Some(fdv) = token.fdv {
            info!("║   FDV:         ${}", format_number(fdv));
        }
        if let Some(mc) = token.market_cap {
            info!("║   Market Cap:  ${}", format_number(mc));
        }
    }
    
    info!("╠════════════════════════════════════════════════════════════════");
    
    // Tax info (CRITICAL for trading)
    info!("║ 📊 TAX ANALYSIS:");
    if let Some(buy_tax) = token.buy_tax {
        let buy_status = if buy_tax > 10.0 { "❌ HIGH" } else if buy_tax > 5.0 { "⚠️ MEDIUM" } else { "✅ LOW" };
        info!("║   Buy Tax:     {:.2}% {}", buy_tax, buy_status);
    } else {
        info!("║   Buy Tax:     ⏳ Calculating...");
    }
    
    if let Some(sell_tax) = token.sell_tax {
        let sell_status = if sell_tax > 10.0 { "❌ HIGH" } else if sell_tax > 5.0 { "⚠️ MEDIUM" } else { "✅ LOW" };
        info!("║   Sell Tax:    {:.2}% {}", sell_tax, sell_status);
    } else {
        info!("║   Sell Tax:    ⏳ Calculating...");
    }
    
    info!("╠════════════════════════════════════════════════════════════════");
    
    // Social links
    if token.social_links.website.is_some() 
        || token.social_links.twitter.is_some() 
        || token.social_links.telegram.is_some() {
        info!("║ 🌐 SOCIAL LINKS:");
        if let Some(website) = &token.social_links.website {
            info!("║   Website:     {}", website);
        }
        if let Some(twitter) = &token.social_links.twitter {
            info!("║   Twitter:     {}", twitter);
        }
        if let Some(telegram) = &token.social_links.telegram {
            info!("║   Telegram:    {}", telegram);
        }
    }
    
    info!("╚════════════════════════════════════════════════════════════════");
    
    
    info!("");
}

fn format_number(num: f64) -> String {
    if num >= 1_000_000_000.0 {
        format!("{:.2}B", num / 1_000_000_000.0)
    } else if num >= 1_000_000.0 {
        format!("{:.2}M", num / 1_000_000.0)
    } else if num >= 1_000.0 {
        format!("{:.2}K", num / 1_000.0)
    } else {
        format!("{:.2}", num)
    }
}