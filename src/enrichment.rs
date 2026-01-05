use crate::config::Config;
use crate::helius_listener::{ClassifiedEvent, PoolCreationEvent, TokenPlatform};
use anyhow::{Context, Result};
use log::{error, info, warn};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::time::Duration;
use tokio::sync::mpsc;
use lazy_static::lazy_static;
use parking_lot::Mutex;
use std::sync::Arc;
use std::collections::HashMap;
 use std::collections::VecDeque;
use crate::trade_logger::{TradeLogger, log_enrichment_debug};
use crate::rate_limiter::RateLimiter;
use crate::retry::{retry_with_backoff, RetryConfig, is_network_error};

// ========== NEW STRUCTS ==========

#[derive(Debug, Clone, Serialize)]
pub struct SocialLinks {
    pub twitter: Option<String>,
    pub telegram: Option<String>,
    pub website: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TokenMetadata {
    pub name: String,
    pub symbol: String,
    pub uri: String,
    pub socials: Option<SocialLinks>,
    pub price: Option<f64>,        // From DAS price_info
    pub royalty_pct: Option<f64>,  // From DAS royalty
}

#[derive(Debug, Clone, Serialize)]
pub struct HolderAnalysis {
    pub top_1_pct: f64,      // % held by top 1 holder (excluding pool)
    pub top_10_pct: f64,     // % held by top 10 holders (excluding pool)
    pub unique_holders: Option<u64>, // Total holder count (if available)
}

#[derive(Debug, Clone, Deserialize)]
struct BirdeyeTokenOverview {
    pub price: Option<f64>,
    pub liquidity: Option<f64>,
    #[serde(rename = "mc")]
    pub market_cap: Option<f64>,
    #[serde(rename = "v24hUSD")]
    pub volume_24h: Option<f64>,
    pub decimals: Option<u8>,
    // New high-value metrics
    #[serde(rename = "uniqueWallet30m")]
    pub unique_wallets_30m: Option<u64>,
    #[serde(rename = "vBuy30mUSD")]
    pub buy_volume_30m_usd: Option<f64>,
    #[serde(rename = "vSell30mUSD")]
    pub sell_volume_30m_usd: Option<f64>,
    #[serde(rename = "priceChange30mPercent")]
    pub price_change_30m_pct: Option<f64>,

    // 5m Metrics (PUMP DETECTION)
    #[serde(rename = "uniqueWallet5m")]
    pub unique_wallets_5m: Option<u64>,
    #[serde(rename = "vBuy5mUSD")]
    pub buy_volume_5m_usd: Option<f64>,
    #[serde(rename = "vSell5mUSD")]
    pub sell_volume_5m_usd: Option<f64>,
    #[serde(rename = "priceChange5mPercent")]
    pub price_change_5m_pct: Option<f64>,

    // 1m Metrics (IGNITION DETECTION)
    #[serde(rename = "uniqueWallet1m")]
    pub unique_wallets_1m: Option<u64>,
    #[serde(rename = "priceChange1mPercent")]
    pub price_change_1m_pct: Option<f64>,

    pub holder: Option<u64>, // Total holder count
    pub extensions: Option<serde_json::Value>, // For socials
}

#[derive(Debug, Clone, Deserialize)]
struct BirdeyeHolder {
    pub owner: String,
    pub ui_amount: f64,
}


/// Fully enriched token ready for scoring/trading
#[derive(Debug, Clone, Serialize)]
/// Enriched Token - GRADUATED TOKENS ONLY (Pump.fun/Bonk.fun)
/// Optimized for speed - only essential data
pub struct EnrichedToken {
    // ========== CORE IDENTIFICATION ==========
    pub mint: String,
    pub signature: String,
    pub slot: u64,
    pub timestamp: Option<i64>,
    
    // ========== TOKEN BASICS ==========
    pub decimals: u8,
    pub supply: Option<u64>,
    
    // ========== LIQUIDITY DATA ==========
    pub has_liquidity: bool,
    pub pool_address: String,
    pub pair_token: String,
    pub dex: String,
    
    // ========== PRICE DATA ==========
    pub price_sol: Option<f64>,
    pub price_usd: Option<f64>,
    pub liquidity_usd: Option<f64>, // ✅ NEW: Birdeye liquidity in USD
    pub initial_liquidity_sol: Option<f64>,
    pub fdv: Option<f64>,
    pub market_cap: Option<f64>,
    
    // ========== PLATFORM INFO ==========
    pub platform: TokenPlatform,  // PumpFun or BonkFun only
    
    // ========== RISK FLAGS ==========
    pub has_freeze_authority: bool,
    pub has_mint_authority: bool,
    
    // ========== METADATA & SOCIALS ==========
    pub metadata: Option<TokenMetadata>,
    
    // ========== HOLDER ANALYSIS ==========
    pub holders: Option<HolderAnalysis>,

    // ========== BIRDEYE METRICS (30m window) ==========
    pub unique_wallets_30m: Option<u64>,
    pub buy_volume_30m_usd: Option<f64>,
    pub sell_volume_30m_usd: Option<f64>,
    pub price_change_30m_pct: Option<f64>,

    // ========== BIRDEYE METRICS (5m window - PRIMARY) ==========
    pub unique_wallets_5m: Option<u64>,
    pub buy_volume_5m_usd: Option<f64>,
    pub sell_volume_5m_usd: Option<f64>,
    pub price_change_5m_pct: Option<f64>,

    // ========== BIRDEYE METRICS (1m window - IGNITION) ==========
    pub unique_wallets_1m: Option<u64>,
    pub price_change_1m_pct: Option<f64>,

    // ========== TIMING ==========
    pub enrichment_timestamp: i64,
    pub enrichment_duration_ms: u128,
}

// ========== MINT ACCOUNT CACHE ==========

#[derive(Debug, Clone)]
struct MintAccountCache {
    data: MintAccountData,
    timestamp: i64,
}

lazy_static! {
    static ref MINT_CACHE: Arc<Mutex<HashMap<String, MintAccountCache>>> = 
        Arc::new(Mutex::new(HashMap::new()));
}

const MINT_CACHE_TTL_SECONDS: i64 = 240; // 4 minutes

// ========== HELIUS API RESPONSES ==========

// ========== HELIUS API RESPONSES ==========

// ========== SOLANA RPC RESPONSES ==========
// Only keeping essential structures for graduated token enrichment

#[derive(Debug, Deserialize, Clone)]
struct MintAccountData {
    #[serde(rename = "mintAuthority")]
    mint_authority: Option<String>,
    #[serde(rename = "freezeAuthority")]
    freeze_authority: Option<String>,
    supply: Option<String>,
    decimals: Option<u8>,
}

#[derive(Debug, Deserialize)]
struct ParsedMintInfo {
    info: MintAccountData,
}

#[derive(Debug, Deserialize)]
struct ParsedAccountData {
    parsed: ParsedMintInfo,
}

#[derive(Debug, Deserialize)]
struct AccountInfo {
    data: ParsedAccountData,
    _owner: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RpcAccountResponse {
    value: Option<AccountInfo>,
}

/// Start the enrichment pipeline
pub async fn start(
    config: &Config,
    classified_rx: mpsc::Receiver<ClassifiedEvent>,
) -> Result<mpsc::Receiver<EnrichedToken>> {
    let (enriched_tx, enriched_rx) = mpsc::channel::<EnrichedToken>(1000);
    let api_key = config.helius_api_key.clone();
    
    info!("🔍 Starting Token Enrichment Pipeline");

    // Initialize Birdeye rate limiter (1 req/sec for free plan)
    let birdeye_limiter = Arc::new(RateLimiter::new(config.birdeye_max_requests_per_second, "Birdeye"));
    
    // Initialize Moralis rate limiter
    let moralis_limiter = Arc::new(RateLimiter::new(config.moralis_max_requests_per_second, "Moralis"));
    
    // Pending token queue for when Birdeye is rate-limited
    let pending_tokens: Arc<Mutex<VecDeque<PoolCreationEvent>>> = Arc::new(Mutex::new(VecDeque::new()));
    
    // Clone config for background tasks
    let loop_config = config.clone();
    
    // Spawn background task to process pending tokens
    let pending_clone = pending_tokens.clone();
    let birdeye_limiter_clone = birdeye_limiter.clone();
    let moralis_limiter_clone = moralis_limiter.clone();
    let enriched_tx_clone = enriched_tx.clone();
    let config_clone = loop_config.clone();
    tokio::spawn(async move {
        let client = Client::builder().timeout(Duration::from_secs(5)).build().unwrap();
        let moralis = crate::moralis_client::MoralisClient::new(
            config_clone.moralis_api_key.clone(),
            "mainnet".to_string(),
        );
        let mut interval = tokio::time::interval(Duration::from_secs(2));
        loop {
            interval.tick().await;
            
            // Try to process pending tokens
            let pool_event = {
                let mut queue = pending_clone.lock();
                queue.pop_front()
            };
            
            if let Some(event) = pool_event {
                // Try to acquire rate limit token (non-blocking)
                if birdeye_limiter_clone.try_acquire().await {
                    info!("📤 Processing pending token from queue: {}", event.token_mint);
                    
                    match enrich_token(&client, &config_clone, event.clone(), &birdeye_limiter_clone, &moralis_limiter_clone, &moralis).await {
                        Ok(enriched) => {
                            if let Err(e) = enriched_tx_clone.send(enriched).await {
                                warn!("Failed to send enriched token from queue: {}", e);
                            }
                        }
                        Err(e) => {
                            warn!("Failed to enrich pending token {}: {}", event.token_mint, e);
                        }
                    }
                } else {
                    // Put it back if we can't process yet
                    let mut queue = pending_clone.lock();
                    queue.push_front(event);
                }
            }
        }
    });

    let loop_config_final = loop_config.clone();
    tokio::spawn(async move {
        if let Err(e) = enrichment_loop(loop_config_final, classified_rx, enriched_tx, birdeye_limiter, moralis_limiter, pending_tokens).await {
            error!("Enrichment pipeline error: {}", e);
        }
    });

    Ok(enriched_rx)
}

async fn enrichment_loop(
    config: Config,
    mut classified_rx: mpsc::Receiver<ClassifiedEvent>,
    enriched_tx: mpsc::Sender<EnrichedToken>,
    birdeye_limiter: Arc<RateLimiter>,
    moralis_limiter: Arc<RateLimiter>,
    pending_tokens: Arc<Mutex<VecDeque<PoolCreationEvent>>>,
) -> Result<()> {
    let client = Client::builder()
        .timeout(Duration::from_secs(5))
        .build()?;

    let moralis = crate::moralis_client::MoralisClient::new(
        config.moralis_api_key.clone(),
        "mainnet".to_string(),
    );

    // Deduplication: Track recently processed mints to avoid duplicates
    let mut seen_mints: HashMap<String, std::time::Instant> = HashMap::new();
    let dedup_window = Duration::from_secs(60); // Skip mints seen in last 60s

    while let Some(event) = classified_rx.recv().await {
        let start_time = std::time::Instant::now();
        
        match event {
            ClassifiedEvent::PoolCreation(pool_event) => {
                // ====== DEDUPLICATION CHECK ======
                let now = std::time::Instant::now();
                
                // Clean up old entries (older than dedup_window)
                seen_mints.retain(|_, seen_at| now.duration_since(*seen_at) < dedup_window);
                
                // Check if we've seen this mint recently
                if seen_mints.contains_key(&pool_event.token_mint) {
                    info!("⏭️ Skipping duplicate mint (seen in last 60s): {}", pool_event.token_mint);
                    // TradeLogger::log(&format!(
                    //     "⏭️ DEDUP: {} skipped (seen recently)", 
                    //     &pool_event.token_mint[..12.min(pool_event.token_mint.len())]
                    // ));
                    continue;
                }
                
                // Mark as seen
                seen_mints.insert(pool_event.token_mint.clone(), now);

                info!("🔍 Enriching token: {}", pool_event.token_mint);
                
                // ====== RATE LIMIT CHECK ======
                // Try to acquire Birdeye rate limit token (non-blocking)
                if !birdeye_limiter.try_acquire().await {
                    let mut queue = pending_tokens.lock();
                    queue.push_back(pool_event.clone());
                    warn!("⚠️ Birdeye rate limited - queued token {} (queue size: {})", pool_event.token_mint, queue.len());
                    continue;
                }
                
                // Spawn the enrichment task
                let client_clone = client.clone();
                let config_clone = config.clone();
                let tx_clone = enriched_tx.clone();
                let b_limiter = birdeye_limiter.clone();
                let m_limiter = moralis_limiter.clone();
                let m_client = moralis.clone();

                tokio::spawn(async move {
                    let task_start = std::time::Instant::now();
                    info!("🔍 Enrichment task started: {}", pool_event.token_mint);
                    
                    // Wait for liquidity settle
                    tokio::time::sleep(Duration::from_secs(2)).await;

                    let max_retries: u8 = 3;
                    let mut attempt: u8 = 1;
                    let mut last_result: Result<EnrichedToken> = Err(anyhow::anyhow!("No attempts made"));

                    while attempt <= max_retries {
                        match enrich_token(&client_clone, &config_clone, pool_event.clone(), &b_limiter, &m_limiter, &m_client).await {
                            Ok(enriched) => {
                                let liq = enriched.initial_liquidity_sol.unwrap_or(0.0);
                                log_enrichment_debug(&enriched.mint, &enriched.pool_address, liq, attempt);
                                
                                if liq > 0.1 || attempt == max_retries {
                                    last_result = Ok(enriched);
                                    break;
                                } else {
                                    warn!("⚠️ Zero liquidity on attempt {}/{}, retrying...", attempt, max_retries);
                                    tokio::time::sleep(Duration::from_secs(attempt as u64 * 2)).await;
                                    attempt += 1;
                                }
                            }
                            Err(e) => {
                                warn!("⚠️ Enrichment failed on attempt {}/{}: {}", attempt, max_retries, e);
                                last_result = Err(e);
                                tokio::time::sleep(Duration::from_secs(attempt as u64 * 2)).await;
                                attempt += 1;
                            }
                        }
                    }

                    match last_result {
                        Ok(enriched) => {
                            let duration = task_start.elapsed().as_millis();
                            info!("✅ Enrichment success: {} ({}ms, {} attempts)", enriched.mint, duration, attempt);
                            if tx_clone.send(enriched).await.is_err() {
                                warn!("Failed to send enriched token (receiver dropped)");
                            }
                        }
                        Err(e) => {
                            error!("❌ Enrichment failed for {}: {}", pool_event.token_mint, e);
                        }
                    }
                });
            }
        }
    }

    Ok(())
}

/// Optimized enrichment for graduated tokens
pub async fn enrich_token(
    client: &reqwest::Client,
    config: &Config,
    pool_event: PoolCreationEvent,
    birdeye_limiter: &RateLimiter,
    moralis_limiter: &RateLimiter,
    moralis: &crate::moralis_client::MoralisClient,
) -> Result<EnrichedToken> {
    let start_time = std::time::Instant::now();
    
    info!("⚡ Enriching token: {} (Birdeye + Helius)", 
        pool_event.token_mint
    );
    
    // 1. Fetch mint data first (essential for Helius liquidity fallback if Birdeye stalls)
    let mint_data_res = fetch_mint_account_cached(client, &config.helius_api_key, &pool_event.token_mint).await;
    let mint_data = mint_data_res.ok();
    let decimals = mint_data.as_ref().and_then(|m| m.decimals).unwrap_or(9);

    // 2. Parallel fetch remaining data
    let (
        birdeye_overview,
        helius_holders_raw,
        metadata_data,
        token_price_sol
    ) = tokio::join!(
        fetch_birdeye_overview(client, &config.birdeye_api_key, &pool_event.token_mint),
        fetch_holder_analysis(client, &config.helius_api_key, &pool_event.token_mint),
        fetch_token_metadata(client, &config.helius_api_key, &pool_event.token_mint),
        fetch_native_price(client, config, &pool_event.token_mint, birdeye_limiter, moralis_limiter, moralis)
    );

    assemble_enriched_token(
        pool_event,
        mint_data,
        birdeye_overview.ok(),
        helius_holders_raw.ok(),
        metadata_data.ok(),
        token_price_sol.ok(), // This is now the token's price in SOL
        start_time,
    )
}

/// Assemble EnrichedToken from fetched data
/// Assemble EnrichedToken from multiple sources
fn assemble_enriched_token(
    pool_event: PoolCreationEvent,
    mint_data: Option<MintAccountData>,
    birdeye_ov: Option<BirdeyeTokenOverview>,
    helius_holders: Option<RawHolderData>,
    metadata: Option<TokenMetadata>,
    token_price_sol: Option<f64>, // This is now the token's price in SOL
    start_time: std::time::Instant,
) -> Result<EnrichedToken> {
    // Extract decimals (CRITICAL)
    let decimals = mint_data
        .as_ref()
        .and_then(|m| m.decimals)
        .unwrap_or(9); // Default to 9 if not found

    // Extract supply
    let supply = mint_data
        .as_ref()
        .and_then(|m| m.supply.as_ref().and_then(|s| s.parse::<u64>().ok()));

    // Risk flags (CRITICAL - always check even for graduated)
    let has_mint_authority = mint_data
        .as_ref()
        .and_then(|m| m.mint_authority.as_ref())
        .is_some();
    
    let has_freeze_authority = mint_data
        .as_ref()
        .and_then(|m| m.freeze_authority.as_ref())
        .is_some();

    // 1. LIQUIDITY PRIORITIZATION
    let liquidity_usd = birdeye_ov.as_ref().and_then(|b| b.liquidity);
    // initial_liquidity_sol is now derived from liquidity_usd and token_price_sol
    let initial_liquidity_sol = if let (Some(liq_usd), Some(price_usd), Some(price_sol)) = (liquidity_usd, birdeye_ov.as_ref().and_then(|b| b.price), token_price_sol) {
        if price_usd > 0.0 {
            Some(liq_usd / price_usd * price_sol)
        } else {
            None
        }
    } else {
        None
    };

    // 2. PRICE PRIORITIZATION
    let price_usd = birdeye_ov.as_ref()
        .and_then(|b| b.price)
        .or_else(|| metadata.as_ref().and_then(|m| m.price));
    
    let price_sol = token_price_sol; // This is the token's price in SOL

    // 3. MARKET CAP / FDV
    let market_cap = birdeye_ov.as_ref().and_then(|b| b.market_cap);
    let fdv = market_cap; // Assuming 100% circulating for graduated

    // 4. BIRDEYE METRICS (30m window)
    let unique_wallets_30m = birdeye_ov.as_ref().and_then(|b| b.unique_wallets_30m);
    let buy_volume_30m_usd = birdeye_ov.as_ref().and_then(|b| b.buy_volume_30m_usd);
    let sell_volume_30m_usd = birdeye_ov.as_ref().and_then(|b| b.sell_volume_30m_usd);
    let price_change_30m_pct = birdeye_ov.as_ref().and_then(|b| b.price_change_30m_pct);

    // 5m Metrics
    let unique_wallets_5m = birdeye_ov.as_ref().and_then(|b| b.unique_wallets_5m);
    let buy_volume_5m_usd = birdeye_ov.as_ref().and_then(|b| b.buy_volume_5m_usd);
    let sell_volume_5m_usd = birdeye_ov.as_ref().and_then(|b| b.sell_volume_5m_usd);
    let price_change_5m_pct = birdeye_ov.as_ref().and_then(|b| b.price_change_5m_pct);

    // 1m Metrics
    let unique_wallets_1m = birdeye_ov.as_ref().and_then(|b| b.unique_wallets_1m);
    let price_change_1m_pct = birdeye_ov.as_ref().and_then(|b| b.price_change_1m_pct);

    // 5. SOCIALS (Birdeye extensions)
    let birdeye_socials = birdeye_ov.as_ref().and_then(|b| {
        b.extensions.as_ref().and_then(|ext| {
            Some(SocialLinks {
                twitter: ext.get("twitter").and_then(|v| v.as_str()).map(String::from),
                telegram: ext.get("telegram").and_then(|v| v.as_str()).map(String::from),
                website: ext.get("website").and_then(|v| v.as_str()).map(String::from),
            })
        })
    });

    // Update metadata with Birdeye socials if available
    let final_metadata = metadata.map(|mut m| {
        if birdeye_socials.is_some() {
            m.socials = birdeye_socials.clone();
        }
        m
    });

    // 6. HOLDER ANALYSIS PRIORITIZATION (Helius Top 20)
    let final_holders = if let Some(raw_holders) = helius_holders {
        if let Some(s) = supply {
            let s_f64 = s as f64;
            if s_f64 > 0.0 {
                // Filter out pool address if it appears in holders
                let mut filtered_holders: Vec<f64> = raw_holders.holders.iter()
                    .filter(|h| h.address != pool_event.pool_address)
                    .map(|h| h.amount)
                    .collect();
                
                filtered_holders.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
                
                // Helius "largest accounts" returns top 20 by default
                let top_1_amount = filtered_holders.get(0).copied().unwrap_or(0.0);
                let top_10_amount: f64 = filtered_holders.iter().take(10).sum();
                let supply_decimal_adjusted = s_f64 / 10f64.powi(decimals as i32);

                Some(HolderAnalysis {
                    top_1_pct: (top_1_amount / supply_decimal_adjusted) * 100.0,
                    top_10_pct: (top_10_amount / supply_decimal_adjusted) * 100.0,
                    unique_holders: birdeye_ov.as_ref().and_then(|b| b.holder), // Total from Birdeye
                })
            } else { None }
        } else { None }
    } else {
        None
    };

    let enriched = EnrichedToken {
        // Core identification
        mint: pool_event.token_mint.clone(),
        signature: pool_event.signature.clone(),
        slot: pool_event.slot,
        timestamp: pool_event.timestamp,
        
        // Token basics
        decimals,
        supply,
        
        // Liquidity
        has_liquidity: initial_liquidity_sol.is_some(),
        pool_address: pool_event.pool_address.clone(),
        pair_token: pool_event.pair_token.clone(),
        dex: pool_event.dex.clone(),
        
        // Price & Liquidity
        price_sol,
        price_usd,
        liquidity_usd,
        initial_liquidity_sol,
        fdv,
        market_cap,
        
        // Platform (always graduated)
        platform: pool_event.token_platform.clone(),
        
        // Risk flags (authority checks only)
        has_freeze_authority,
        has_mint_authority,
        
        // New Metrics
        metadata: final_metadata,
        holders: final_holders,

        // Birdeye Metrics (30m window)
        unique_wallets_30m,
        buy_volume_30m_usd,
        sell_volume_30m_usd,
        price_change_30m_pct,

        // 5m Metrics
        unique_wallets_5m,
        buy_volume_5m_usd,
        sell_volume_5m_usd,
        price_change_5m_pct,

        // 1m Metrics
        unique_wallets_1m,
        price_change_1m_pct,

        
        // Timing
        enrichment_timestamp: chrono::Utc::now().timestamp(),
        enrichment_duration_ms: start_time.elapsed().as_millis(),
    };

    // LOGGING: Detailed enrichment data
    let top_10_pct = enriched.holders.as_ref().map(|h| h.top_10_pct).unwrap_or(0.0);
    let unique_holders = enriched.holders.as_ref().and_then(|h| h.unique_holders);
    let socials_count = enriched.metadata.as_ref().and_then(|m| m.socials.as_ref()).map(|s| {
        let mut count = 0;
        if s.twitter.is_some() { count += 1; }
        if s.telegram.is_some() { count += 1; }
        if s.website.is_some() { count += 1; }
        count
    }).unwrap_or(0);

    TradeLogger::log_enrichment_data(
        &enriched.mint,
        enriched.liquidity_usd.unwrap_or(0.0),
        top_10_pct,
        unique_holders,
        socials_count
    );
    
    // NEW: Log the full struct for deep debugging as requested by user
    // TradeLogger::log_enriched_token(&enriched);

    Ok(enriched)
}

// ========== DATA FETCHING FUNCTIONS ==========

/// Fetch mint account info with caching (4-minute TTL)
async fn fetch_mint_account_cached(
    client: &Client,
    api_key: &str,
    mint: &str,
) -> Result<MintAccountData> {
    let now = chrono::Utc::now().timestamp();
    
    // Check cache first
    {
        let cache = MINT_CACHE.lock();
        if let Some(cached) = cache.get(mint) {
            if now - cached.timestamp < MINT_CACHE_TTL_SECONDS {
                info!("✅ Cache hit for mint: {}", mint);
                return Ok(cached.data.clone());
            }
        }
    }
    
    // Cache miss or expired - fetch from RPC
    info!("🔍 Cache miss for mint: {} - fetching from RPC", mint);
    let data = fetch_mint_account_info(client, api_key, mint).await?;
    
    // Update cache
    {
        let mut cache = MINT_CACHE.lock();
        // Prevent unbounded growth
        if cache.len() > 1000 {
            cache.clear();
        }
        cache.insert(mint.to_string(), MintAccountCache {
            data: data.clone(),
            timestamp: now,
        });
    }
    
    Ok(data)
}

/// Fetch mint account info from Solana RPC
async fn fetch_mint_account_info(
    client: &Client,
    api_key: &str,
    mint: &str,
) -> Result<MintAccountData> {
    let url = format!("https://mainnet.helius-rpc.com/?api-key={}", api_key);
    
    let request_body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "getAccountInfo",
        "params": [
            mint,
            {
                "encoding": "jsonParsed"
            }
        ]
    });

    let response = client
        .post(&url)
        .json(&request_body)
        .send()
        .await
        .context("Failed to fetch mint account info")?;

    let response_json: serde_json::Value = response
        .json()
        .await
        .context("Failed to parse mint response")?;

    if let Some(result) = response_json.get("result") {
        let account_response: RpcAccountResponse = serde_json::from_value(result.clone())
            .context("Failed to deserialize account info")?;
        
        account_response
            .value
            .and_then(|v| Some(v.data.parsed.info))
            .ok_or_else(|| anyhow::anyhow!("No account data found"))
    } else {
        Err(anyhow::anyhow!("No result in mint account response"))
    }
}


/// Fetch liquidity data from pool by querying token account balances



/// Fetch token metadata using Helius DAS API (getAsset)
async fn fetch_token_metadata(
    client: &Client,
    api_key: &str,
    mint: &str,
) -> Result<TokenMetadata> {
    let url = format!("https://mainnet.helius-rpc.com/?api-key={}", api_key);
    
    let request_body = json!({
        "jsonrpc": "2.0",
        "id": "my-id",
        "method": "getAsset",
        "params": {
            "id": mint
        }
    });

    let response = client
        .post(&url)
        .json(&request_body)
        .send()
        .await
        .context("Failed to fetch asset metadata")?;

    let json: serde_json::Value = response.json().await?;
    
    let result = json.get("result").context("No result in getAsset")?;
    let content = result.get("content").context("No content in asset")?;
    let metadata = content.get("metadata").context("No metadata in content")?;
    
    let name = metadata.get("name").and_then(|s| s.as_str()).unwrap_or("Unknown").to_string();
    let symbol = metadata.get("symbol").and_then(|s| s.as_str()).unwrap_or("UNK").to_string();
    let uri = result.get("content").and_then(|c| c.get("json_uri")).and_then(|s| s.as_str()).unwrap_or("").to_string();

    // Fetch JSON URI for socials (if available)
    let mut socials = None;
    if !uri.is_empty() {
        // Fetch JSON URI for socials (if available)
        // Using a short timeout (500ms) to prevent blocking the pipeline
        if let Ok(json_meta) = client.get(&uri).timeout(Duration::from_millis(500)).send().await {
            if let Ok(meta_body) = json_meta.json::<serde_json::Value>().await {
                let twitter = meta_body.get("twitter").and_then(|s| s.as_str()).map(|s| s.to_string());
                let telegram = meta_body.get("telegram").and_then(|s| s.as_str()).map(|s| s.to_string());
                let website = meta_body.get("website").and_then(|s| s.as_str()).map(|s| s.to_string());
                
                if twitter.is_some() || telegram.is_some() || website.is_some() {
                    socials = Some(SocialLinks {
                        twitter,
                        telegram,
                        website,
                    });
                }
            }
        }
    }

    // Extract Price and Royalty from DAS
    let price = result.get("token_info")
        .and_then(|t| t.get("price_info"))
        .and_then(|p| p.get("price_per_token"))
        .and_then(|v| v.as_f64());

    let royalty_pct = result.get("royalty")
        .and_then(|r| r.get("percent"))
        .and_then(|v| v.as_f64())
        .map(|p| p * 100.0); // Convert 0.05 to 5.0%? Usually it's already percentage or basis points. 
                             // Debugger showed "percent": 0.0. Let's assume it's a float percentage (0-100) or ratio (0-1).
                             // Solscan usually shows %, e.g. 5%. If API returns 0.05 for 5%, we multiply.
                             // If it returns 5.0, we keep it. 
                             // Let's assume it's a direct percentage value based on "percent" name, but verify later.
                             // Actually, standard Metaplex is basis points. DAS might convert.
                             // Let's just store what we get for now.

    Ok(TokenMetadata {
        name,
        symbol,
        uri,
        socials,
        price,
        royalty_pct,
    })
}

// ========== HELPER FUNCTIONS ==========
// ========== HELPER FUNCTIONS ==========


/// Fetch the token's native price (SOL/token) directly
async fn fetch_native_price(
    client: &Client,
    config: &Config,
    token_mint: &str,
    birdeye_limiter: &RateLimiter,
    moralis_limiter: &RateLimiter,
    moralis: &crate::moralis_client::MoralisClient,
) -> Result<f64> {
    let api_key = config.birdeye_api_key.clone();

    match config.price_source_priority {
        crate::config::PriceSourcePriority::MoralisFirst => {
            // 1. Try Moralis (Primary)
            moralis_limiter.acquire().await;
            match moralis.get_token_price(token_mint).await {
                Ok(price) => {
                    info!("📈 Enrichment Price (Moralis): {} = {:.8} SOL", token_mint, price);
                    Ok(price)
                },
                Err(e) => {
                    warn!("⚠️ Enrichment: Moralis price fetch failed for {}: {}. Falling back to Birdeye...", token_mint, e);
                    fetch_native_price_birdeye(client, &api_key, token_mint, birdeye_limiter).await
                }
            }
        }
        crate::config::PriceSourcePriority::BirdeyeFirst => {
            // 1. Try Birdeye (Primary)
            match fetch_native_price_birdeye(client, &api_key, token_mint, birdeye_limiter).await {
                Ok(price) => {
                    info!("📈 Enrichment Price (Birdeye): {} = {:.8} SOL", token_mint, price);
                    Ok(price)
                },
                Err(e) => {
                    warn!("⚠️ Enrichment: Birdeye price fetch failed for {}: {}. Falling back to Moralis...", token_mint, e);
                    moralis_limiter.acquire().await;
                    moralis.get_token_price(token_mint).await
                }
            }
        }
    }
}

async fn fetch_native_price_birdeye(
    client: &Client,
    api_key: &str,
    mint: &str,
    limiter: &RateLimiter,
) -> Result<f64> {
    limiter.acquire().await;
    let url = format!("https://public-api.birdeye.so/defi/price?address={}", mint);
    
    let resp = client.get(&url)
        .header("X-API-KEY", api_key)
        .header("x-chain", "solana")
        .header("accept", "application/json")
        .timeout(Duration::from_secs(5))
        .send().await?;
        
    let json: serde_json::Value = resp.json().await?;
    
    if let Some(price_in_native) = json.get("data").and_then(|d| d.get("priceInNative")).and_then(|v| v.as_f64()) {
        if price_in_native > 0.0 {
            return Ok(price_in_native);
        }
    }
    
    Err(anyhow::anyhow!("Birdeye priceInNative not found for {}", mint))
}

/// Fetch detailed token overview from Birdeye (with retry)
async fn fetch_birdeye_overview(
    client: &Client,
    api_key: &str,
    mint: &str,
) -> Result<BirdeyeTokenOverview> {
    let retry_config = RetryConfig {
        max_attempts: 3,
        initial_delay_ms: 500,
        max_delay_ms: 2000,
        backoff_multiplier: 2.0,
    };
    
    let api_key = api_key.to_string();
    let mint = mint.to_string();
    let client = client.clone();
    
    retry_with_backoff(
        "Birdeye Token Overview",
        || async {
            let url = format!("https://public-api.birdeye.so/defi/token_overview?address={}", mint);
            
            let response = client
                .get(&url)
                .header("X-API-KEY", &api_key)
                .header("x-chain", "solana")
                .timeout(Duration::from_secs(5))
                .send()
                .await
                .context("Birdeye Token Overview request failed")?;

            let json: serde_json::Value = response.json().await?;
            
            if let Some(data) = json.get("data") {
                let overview: BirdeyeTokenOverview = serde_json::from_value(data.clone())
                    .context("Failed to deserialize Birdeye token overview")?;
                Ok(overview)
            } else {
                Err(anyhow::anyhow!("Birdeye overview returned no data for {}", mint))
            }
        },
        &retry_config,
        is_network_error,
    ).await
}

// ========== HELPER FUNCTIONS ==========

#[derive(Debug)]
struct Holder {
    address: String,
    amount: f64,
}

/// Raw holder data to be filtered later
struct RawHolderData {
    holders: Vec<Holder>,
}

/// Fetch holder analysis using getTokenLargestAccounts from Helius
async fn fetch_holder_analysis(
    client: &Client,
    api_key: &str,
    mint: &str,
) -> Result<RawHolderData> {
    let url = format!("https://mainnet.helius-rpc.com/?api-key={}", api_key);
    
    // Default is top 20 accounts
    let request_body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "getTokenLargestAccounts",
        "params": [
            mint
        ]
    });

    let response = client
        .post(&url)
        .json(&request_body)
        .send()
        .await
        .context("Failed to fetch largest accounts")?;

    let json: serde_json::Value = response.json().await?;
    
    let accounts = json
        .get("result")
        .and_then(|r| r.get("value"))
        .and_then(|v| v.as_array())
        .context("No accounts found")?;

    if accounts.is_empty() {
        return Err(anyhow::anyhow!("No holders found"));
    }

    // Return raw data for filtering in assembly
    let mut holders = Vec::new();
    for acc in accounts {
        let amount = acc.get("uiAmount").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let address = acc.get("address").and_then(|s| s.as_str()).unwrap_or("").to_string();
        
        holders.push(Holder {
            address,
            amount,
        });
    }

    Ok(RawHolderData {
        holders,
    })
}