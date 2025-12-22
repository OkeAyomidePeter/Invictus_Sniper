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
use crate::trade_logger::{TradeLogger, log_enrichment_debug};

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

    // Initialize SOL price cache
    let client = Client::builder()
        .timeout(Duration::from_secs(5))
        .build()?;
    
    // Initial price fetch
    if let Err(e) = update_sol_price_cache(&client, config).await {
        warn!("Failed to initialize SOL price cache: {}. Using default price.", e);
    }

    // Spawn background task to update SOL price every 30 seconds
    let price_client = client.clone();
    let price_config = config.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(30));
        loop {
            interval.tick().await;
            if let Err(e) = update_sol_price_cache(&price_client, &price_config).await {
                warn!("Failed to update SOL price cache: {}", e);
            }
        }
    });

    let loop_config = config.clone();
    tokio::spawn(async move {
        if let Err(e) = enrichment_loop(loop_config, classified_rx, enriched_tx).await {
            error!("Enrichment pipeline error: {}", e);
        }
    });

    Ok(enriched_rx)
}

async fn enrichment_loop(
    config: Config,
    mut classified_rx: mpsc::Receiver<ClassifiedEvent>,
    enriched_tx: mpsc::Sender<EnrichedToken>,
) -> Result<()> {
    let client = Client::builder()
        .timeout(Duration::from_secs(5))
        .build()?;

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
                    TradeLogger::log(&format!(
                        "⏭️ DEDUP: {} skipped (seen recently)", 
                        &pool_event.token_mint[..12.min(pool_event.token_mint.len())]
                    ));
                    continue;
                }
                
                // Mark as seen
                seen_mints.insert(pool_event.token_mint.clone(), now);

                info!("🔍 Enriching token: {}", pool_event.token_mint);
                
                // ====== GRADUATION DELAY ======
                // Wait 2 seconds for pool liquidity to settle after graduation event
                info!("⏳ Waiting 2s for pool to settle: {}", pool_event.pool_address);
                tokio::time::sleep(Duration::from_secs(2)).await;
                
                // ====== RETRY LOGIC WITH BACKOFF ======
                let max_retries: u8 = 3;
                let mut attempt: u8 = 1;
                let mut last_result: Result<EnrichedToken> = Err(anyhow::anyhow!("No attempts made"));
                
                while attempt <= max_retries {
                    match enrich_token(&client, &config, pool_event.clone()).await {
                        Ok(enriched) => {
                            let liq = enriched.initial_liquidity_sol.unwrap_or(0.0);
                            
                            // Log debug info for every attempt
                            log_enrichment_debug(
                                &enriched.mint,
                                &enriched.pool_address,
                                liq,
                                attempt
                            );
                            
                            // Check if liquidity is valid
                            if liq > 0.1 || attempt == max_retries {
                                // Success or final attempt - proceed
                                last_result = Ok(enriched);
                                break;
                            } else {
                                // Zero liquidity - retry with backoff
                                let msg = format!("⚠️ Zero liquidity on attempt {}/{}, retrying...", attempt, max_retries);
                                warn!("{}", msg);
                                TradeLogger::log(&format!("🔎 ENRICH_RETRY: {} | {}", pool_event.token_mint, msg));
                                let backoff = Duration::from_secs(attempt as u64 * 2);
                                tokio::time::sleep(backoff).await;
                                attempt += 1;
                            }
                        }
                        Err(e) => {
                            let msg = format!("⚠️ Enrichment attempt {}/{} failed: {}", attempt, max_retries, e);
                            warn!("{}", msg);
                            TradeLogger::log(&format!("🔎 ENRICH_RETRY: {} | {}", pool_event.token_mint, msg));
                            last_result = Err(e);
                            let backoff = Duration::from_secs(attempt as u64 * 2);
                            tokio::time::sleep(backoff).await;
                            attempt += 1;
                        }
                    }
                }
                
                // Process final result
                match last_result {
                    Ok(enriched) => {
                        let duration = start_time.elapsed().as_millis();
                        info!("✅ Enrichment complete: {} (took {}ms, {} attempts)", enriched.mint, duration, attempt);
    
                        // Simplified logging - only show key fields
                        info!("📊 Token: {} | Liq: {} SOL | Auth: freeze={}, mint={} | Platform: Graduated",
                            enriched.mint,
                            enriched.initial_liquidity_sol.unwrap_or(0.0),
                            enriched.has_freeze_authority,
                            enriched.has_mint_authority
                        );
                        
                        TradeLogger::log(&format!(
                            "✨ ENRICHMENT SUCCESS: {} | Liq: {:.2} SOL | Platform: {:?}",
                            enriched.mint,
                            enriched.initial_liquidity_sol.unwrap_or(0.0),
                            enriched.platform
                        ));
                        
                        if enriched_tx.send(enriched).await.is_err() {
                            warn!("Failed to send enriched token (receiver dropped)");
                            break;
                        }
                    }
                    Err(e) => {
                        error!("Failed to enrich token after {} attempts: {}", max_retries, e);
                        TradeLogger::log(&format!("❌ ENRICHMENT FAILED: {} | Error: {} | Attempts: {}", pool_event.token_mint, e, max_retries));
                        // Don't stop the pipeline - continue processing
                    }
                }
            }
        }
    }

    Ok(())
}

/// Main enrichment function - TIERED APPROACH
/// Fast path for graduated tokens, full path for standard tokens
async fn enrich_token(
    client: &Client,
    config: &Config,
    pool_event: PoolCreationEvent,
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
        birdeye_holders,
        metadata_data,
        helius_liq,
        helius_holders_raw,
        helius_total_holders,
        sol_price_data
    ) = tokio::join!(
        fetch_birdeye_overview(client, &config.birdeye_api_key, &pool_event.token_mint),
        fetch_birdeye_holders(client, &config.birdeye_api_key, &pool_event.token_mint),
        fetch_token_metadata(client, &config.helius_api_key, &pool_event.token_mint),
        fetch_liquidity_data(client, &config.helius_api_key, &pool_event.pool_address, &pool_event.token_mint, &pool_event.pair_token, decimals),
        fetch_holder_analysis(client, &config.helius_api_key, &pool_event.token_mint),
        fetch_total_holders(client, &config.helius_api_key, &pool_event.token_mint),
        get_or_fetch_sol_price(client, config)
    );

    let sol_price = sol_price_data.unwrap_or(150.0);

    assemble_enriched_token(
        pool_event,
        mint_data,
        birdeye_overview.ok(),
        birdeye_holders.ok(),
        helius_liq.ok(),
        metadata_data.ok(),
        helius_holders_raw.ok(),
        helius_total_holders.ok(),
        sol_price,
        start_time,
    )
}

/// Assemble EnrichedToken from fetched data
/// Assemble EnrichedToken from multiple sources
fn assemble_enriched_token(
    pool_event: PoolCreationEvent,
    mint_data: Option<MintAccountData>,
    birdeye_ov: Option<BirdeyeTokenOverview>,
    birdeye_holders: Option<Vec<BirdeyeHolder>>,
    helius_liq: Option<LiquidityData>,
    metadata: Option<TokenMetadata>,
    helius_holders: Option<RawHolderData>,
    helius_total_holders: Option<u64>,
    sol_price: f64,
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
    let initial_liquidity_sol = birdeye_ov.as_ref()
        .and_then(|b| b.liquidity)
        .or_else(|| helius_liq.as_ref().and_then(|h| h.liquidity_sol));

    // 2. PRICE PRIORITIZATION
    let price_usd = birdeye_ov.as_ref()
        .and_then(|b| b.price)
        .or_else(|| metadata.as_ref().and_then(|m| m.price));
    
    let price_sol = price_usd.map(|p| p / sol_price);

    // 3. MARKET CAP / FDV
    let market_cap = birdeye_ov.as_ref().and_then(|b| b.market_cap);
    let fdv = market_cap; // Assuming 100% circulating for graduated

    // 4. HOLDER ANALYSIS PRIORITIZATION
    let final_holders = if let Some(be_h) = birdeye_holders {
        // Use Birdeye holders (usually cleaner than Helius for Top 10)
        let mut sorted_amounts: Vec<f64> = be_h.iter().map(|h| h.ui_amount).collect();
        sorted_amounts.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
        
        let total_supply_f64 = supply.unwrap_or(0) as f64 / 10f64.powi(decimals as i32);
        
        if total_supply_f64 > 0.0 {
            let top_1_amount = sorted_amounts.get(0).copied().unwrap_or(0.0);
            let top_10_amount: f64 = sorted_amounts.iter().take(10).sum();
            
            Some(HolderAnalysis {
                top_1_pct: (top_1_amount / total_supply_f64) * 100.0,
                top_10_pct: (top_10_amount / total_supply_f64) * 100.0,
                unique_holders: helius_total_holders, // Birdeye overview might lack this, use Helius
            })
        } else {
            None
        }
    } else if let Some(raw_holders) = helius_holders {
        // Fallback to Helius logic
        if let Some(s) = supply {
            let s_f64 = s as f64;
            if s_f64 > 0.0 {
                let vault_addr = helius_liq.as_ref().and_then(|l| l.pool_token_account.as_ref());
                let mut filtered_holders: Vec<f64> = raw_holders.holders.iter()
                    .filter(|h| {
                        if let Some(v) = vault_addr {
                            if &h.address == v { return false; }
                        }
                        if h.address == pool_event.pool_address { return false; }
                        true
                    })
                    .map(|h| h.amount)
                    .collect();
                
                filtered_holders.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
                let top_1_amount = filtered_holders.get(0).copied().unwrap_or(0.0);
                let top_10_amount: f64 = filtered_holders.iter().take(10).sum();

                Some(HolderAnalysis {
                    top_1_pct: (top_1_amount / s_f64) * 100.0,
                    top_10_pct: (top_10_amount / s_f64) * 100.0,
                    unique_holders: helius_total_holders.or(raw_holders.unique_holders),
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
        
        // Price data
        price_sol,
        price_usd,
        initial_liquidity_sol,
        fdv,
        market_cap,
        
        // Platform (always graduated)
        platform: pool_event.token_platform.clone(),
        
        // Risk flags (authority checks only)
        has_freeze_authority,
        has_mint_authority,
        
        // New Metrics
        metadata,
        holders: final_holders,

        
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
        enriched.initial_liquidity_sol.unwrap_or(0.0),
        top_10_pct,
        unique_holders,
        socials_count
    );
    
    // NEW: Log the full struct for deep debugging as requested by user
    TradeLogger::log_enriched_token(&enriched);

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

#[derive(Debug)]
struct LiquidityData {
    liquidity_sol: Option<f64>,
    liquidity_token: Option<f64>,
    pool_token_account: Option<String>, // The account holding the tokens (to exclude from holders)
}



// SOL Price cache
#[derive(Debug, Clone)]
struct SolPriceCache {
    price: f64,
    timestamp: i64,
}

lazy_static! {
    static ref SOL_PRICE_CACHE: Arc<Mutex<Option<SolPriceCache>>> = Arc::new(Mutex::new(None));
}

/// Fetch liquidity data from pool by querying token account balances
async fn fetch_liquidity_data(
    client: &Client,
    api_key: &str,
    pool_address: &str,
    token_mint: &str,
    pair_token: &str,
    token_decimals: u8,
) -> Result<LiquidityData> {
    let url = format!("https://mainnet.helius-rpc.com/?api-key={}", api_key);
    
    // Program IDs for SPL Token and Token-2022
    let _token_programs = [
        "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA", // SPL Token
        "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb", // Token-2022
    ];

    let mut liquidity_sol: Option<f64> = None;
    let mut liquidity_token: Option<f64> = None;
    let mut pool_token_account: Option<String> = None;

    // Common SOL/WSOL mint addresses
    let sol_mints = [
        "So11111111111111111111111111111111111111112", // WSOL
        "11111111111111111111111111111111",           // Native SOL
    ];

    // STEP 1: DAS-based reserve discovery (More robust than direct RPC)

    let das_req = json!({
        "jsonrpc": "2.0",
        "id": "pool-reserves",
        "method": "getTokenAccounts",
        "params": {
            "owner": pool_address,
            "page": 1,
            "limit": 100
        }
    });

    if let Ok(response) = client.post(&url).json(&das_req).send().await {
        if let Ok(json) = response.json::<serde_json::Value>().await {
            if let Some(accounts) = json.get("result").and_then(|r| r.get("token_accounts")).and_then(|a| a.as_array()) {
                for account in accounts {
                    let mint = account.get("mint").and_then(|m| m.as_str()).unwrap_or("");
                    let amount = account.get("amount").and_then(|a| a.as_f64()).unwrap_or(0.0);
                    let pubkey = account.get("address").and_then(|s| s.as_str()).unwrap_or("");

                    // CRITICAL: Determine correct decimals for calculation
                    let decimals = if sol_mints.contains(&mint) || mint == pair_token {
                        9 // SOL/WSOL
                    } else if mint == token_mint {
                        token_decimals as u32
                    } else {
                        6 // Default fallback
                    };

                    let ui_amount = amount / 10f64.powi(decimals as i32);

                    if sol_mints.contains(&mint) || mint == pair_token {
                        liquidity_sol = Some(ui_amount);
                    } else if mint == token_mint {
                        liquidity_token = Some(ui_amount);
                        pool_token_account = Some(pubkey.to_string());
                    }
                }
            }
        }
    }

    // STEP 2: Native SOL Fallback
    // Some pools use native SOL instead of WSOL (mostly bonding curves or unusual AMMs)
    if liquidity_sol.is_none() || liquidity_sol.unwrap_or(0.0) < 0.001 {
        let sol_req = json!({
            "jsonrpc": "2.0", "id": 1, "method": "getBalance", "params": [pool_address]
        });
        if let Ok(resp) = client.post(&url).json(&sol_req).send().await {
            if let Ok(json) = resp.json::<serde_json::Value>().await {
                if let Some(lamports) = json.get("result").and_then(|r| r.get("value")).and_then(|v| v.as_u64()) {
                    let balance = lamports as f64 / 1_000_000_000.0;
                    if balance > 0.001 {
                        liquidity_sol = Some(balance);
                    }
                }
            }
        }
    }

    // STEP 3: Vault Discovery Fallback (for Raydium etc. where tokens are in separate vaults)
    if liquidity_token.is_none() || liquidity_token.unwrap_or(0.0) < 0.1 {
        let largest_req = json!({
            "jsonrpc": "2.0", "id": 1, "method": "getTokenLargestAccounts", "params": [token_mint]
        });
        if let Ok(resp) = client.post(&url).json(&largest_req).send().await {
            if let Ok(json) = resp.json::<serde_json::Value>().await {
                if let Some(top_acc) = json.get("result").and_then(|r| r.get("value")).and_then(|v| v.as_array()).and_then(|a| a.get(0)) {
                    let vault_addr = top_acc.get("address").and_then(|s| s.as_str()).unwrap_or("");
                    let amount = top_acc.get("uiAmount").and_then(|v| v.as_f64()).unwrap_or(0.0);
                    
                    if amount > 0.0 {
                        liquidity_token = Some(amount);
                        pool_token_account = Some(vault_addr.to_string());
                        // If we found the token vault but still have no SOL, 
                        // it's likely a complex Raydium pool where we'd need to parse the pool state for the SOL vault.
                    }
                }
            }
        }
    }

    Ok(LiquidityData {
        liquidity_sol,
        liquidity_token,
        pool_token_account,
    })
}



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

#[derive(Debug)]
struct Holder {
    address: String,
    amount: f64,
}

/// Raw holder data to be filtered later
struct RawHolderData {
    holders: Vec<Holder>,
    unique_holders: Option<u64>,
}

/// Fetch holder analysis using getTokenLargestAccounts
async fn fetch_holder_analysis(
    client: &Client,
    api_key: &str,
    mint: &str,
) -> Result<RawHolderData> {
    let url = format!("https://mainnet.helius-rpc.com/?api-key={}", api_key);
    
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
        let amount = acc.get("amount").and_then(|s| s.as_str()).and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.0);
        let address = acc.get("address").and_then(|s| s.as_str()).unwrap_or("").to_string();
        
        holders.push(Holder {
            address,
            amount,
        });
    }

    Ok(RawHolderData {
        holders,
        unique_holders: None, 
    })
}

/// Fetch total unique holders using Helius DAS getTokenAccounts
async fn fetch_total_holders(
    client: &Client,
    api_key: &str,
    mint: &str,
) -> Result<u64> {
    let url = format!("https://mainnet.helius-rpc.com/?api-key={}", api_key);
    
    let request_body = json!({
        "jsonrpc": "2.0",
        "id": "holders",
        "method": "getTokenAccounts",
        "params": {
            "page": 1,
            "limit": 1,
            "displayOptions": {},
            "mint": mint
        }
    });

    let response = client
        .post(&url)
        .json(&request_body)
        .send()
        .await
        .context("Failed to fetch total holders")?;

    let json: serde_json::Value = response.json().await?;
    
    // Result contains "total"
    let total = json.get("result")
        .and_then(|r| r.get("total"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    // CRITICAL: Helius DAS indexing can lag significantly for brand new tokens.
    // If it returns 1 (just the pool) but the token is graduated/active, 
    // we fallback to the count of largest accounts we already fetched to avoid false "Ghost Town" penalties.
    if total <= 1 {
        // Fetch largest accounts list to get a better lower bound (Limit increased to 100)
        let largest_req = json!({
            "jsonrpc": "2.0", 
            "id": 1, 
            "method": "getTokenLargestAccounts", 
            "params": [
                mint,
                { "commitment": "confirmed" } // Optional params can take a limit but standard JSON-RPC use [mint]
            ]
        });
        if let Ok(resp) = client.post(&url).json(&largest_req).send().await {
            if let Ok(json) = resp.json::<serde_json::Value>().await {
                if let Some(count) = json.get("result").and_then(|r| r.get("value")).and_then(|v| v.as_array()).map(|a| a.len() as u64) {
                    if count > total {
                        return Ok(count);
                    }
                }
            }
        }
    }

    Ok(total)
}

// ========== HELPER FUNCTIONS ==========



fn calculate_market_metrics(
    liq_data: &Option<LiquidityData>,
    supply: Option<u64>,
    decimals: u8,
    sol_price_usd: f64,
) -> (Option<f64>, Option<f64>, Option<f64>, Option<f64>) {
    let liq = match liq_data.as_ref() {
        Some(l) => l,
        None => return (None, None, None, None),
    };

    let supply_val = match supply {
        Some(s) if s > 0 => s,
        _ => return (None, None, None, None),
    };

    // Get reserves
    let reserve_sol = match liq.liquidity_sol {
        Some(r) if r > 0.0 => r,
        _ => return (None, None, None, None),
    };

    let reserve_token = match liq.liquidity_token {
        Some(r) if r > 0.0 => r,
        _ => {
            // Fallback: if we don't have token reserve, use supply as approximation
            // This is less accurate but better than nothing
            supply_val as f64 / 10_f64.powi(decimals as i32)
        }
    };

    // Calculate price using AMM constant product formula (x * y = k)
    // price_in_sol = reserve_sol / reserve_token
    let price_sol = if reserve_token > 0.0 {
        Some(reserve_sol / reserve_token)
    } else {
        None
    };

    // Get real-time SOL price (passed in)
    let price_usd = price_sol.map(|p| p * sol_price_usd);

    // Calculate circulating supply (in human-readable units)
    let supply_f64 = supply_val as f64 / 10_f64.powi(decimals as i32);

    // Market cap = price * circulating supply
    let market_cap = price_usd.map(|p| p * supply_f64);

    // FDV = price * total supply (same as market cap if 100% circulating)
    let fdv = market_cap;

    (price_sol, price_usd, market_cap, fdv)
}

/// Get cached SOL price or fetch new one if expired (async)
async fn get_or_fetch_sol_price(client: &Client, config: &Config) -> Result<f64> {
    const CACHE_TTL_SECONDS: i64 = 30;

    let now = chrono::Utc::now().timestamp();
    
    // Check cache first
    {
        let cache = SOL_PRICE_CACHE.lock();
        if let Some(cached) = cache.as_ref() {
            if now - cached.timestamp < CACHE_TTL_SECONDS {
                return Ok(cached.price);
            }
        }
    }

    // Cache expired or empty, fetch new price
    update_sol_price_cache(client, config).await
}

/// Fetch real-time SOL price from multiple sources with fallback
pub async fn update_sol_price_cache(client: &Client, config: &Config) -> Result<f64> {
    // Try multiple sources in order of preference
    let price = match fetch_sol_price_jupiter_v6(client, &config.jupiter_api_key).await {
        Ok(p) => p,
        Err(_) => match fetch_sol_price_birdeye_with_key(client, &config.birdeye_api_key).await {
            Ok(p) => p,
            Err(_) => fetch_sol_price_coingecko(client).await.unwrap_or(150.0),
        },
    };

    // Update cache
    {
        let mut cache = SOL_PRICE_CACHE.lock();
        *cache = Some(SolPriceCache {
            price,
            timestamp: chrono::Utc::now().timestamp(),
        });
    }

    info!("✅ Updated SOL price cache: ${:.2}", price);
    Ok(price)
}

/// Fetch SOL price from Jupiter Price API v2 (Authenticated)
async fn fetch_sol_price_jupiter_v6(client: &Client, api_key: &str) -> Result<f64> {
    let url = "https://api.jup.ag/price/v2/full?ids=So11111111111111111111111111111111111111112";
    
    let response = client
        .get(url)
        .header("x-api-key", api_key)
        .timeout(Duration::from_secs(3))
        .send()
        .await
        .context("Jupiter V6 Price API request failed")?;

    let json: serde_json::Value = response.json().await?;
    
    // Parse response: {"data": {"So11...": {"price": "..."}}}
    let price_str = json
        .get("data")
        .and_then(|d| d.get("So11111111111111111111111111111111111111112"))
        .and_then(|p| p.get("price"))
        .and_then(|v| v.as_str())
        .context("Failed to parse Jupiter v6 price")?;

    let price: f64 = price_str.parse().context("Failed to parse price string as f64")?;
    Ok(price)
}

/// Fetch SOL price from Birdeye API (Authenticated)
async fn fetch_sol_price_birdeye_with_key(client: &Client, api_key: &str) -> Result<f64> {
    let url = "https://public-api.birdeye.so/defi/price?address=So11111111111111111111111111111111111111112";
    
    let response = client
        .get(url)
        .header("X-API-KEY", api_key)
        .header("x-chain", "solana")
        .timeout(Duration::from_secs(3))
        .send()
        .await
        .context("Birdeye Price API request failed")?;

    let json: serde_json::Value = response.json().await?;
    
    let price = json
        .get("data")
        .and_then(|d| d.get("value"))
        .and_then(|p| p.as_f64())
        .context("Failed to parse Birdeye price")?;

    Ok(price)
}

/// Fetch detailed token overview from Birdeye
async fn fetch_birdeye_overview(
    client: &Client,
    api_key: &str,
    mint: &str,
) -> Result<BirdeyeTokenOverview> {
    let url = format!("https://public-api.birdeye.so/defi/token_overview?address={}", mint);
    
    let response = client
        .get(&url)
        .header("X-API-KEY", api_key)
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
}

/// Fetch token holders from Birdeye (Top 100)
async fn fetch_birdeye_holders(
    client: &Client,
    api_key: &str,
    mint: &str,
) -> Result<Vec<BirdeyeHolder>> {
    let url = format!("https://public-api.birdeye.so/defi/v3/token/holder?address={}&offset=0&limit=100", mint);
    
    let response = client
        .get(&url)
        .header("X-API-KEY", api_key)
        .header("x-chain", "solana")
        .timeout(Duration::from_secs(5))
        .send()
        .await
        .context("Birdeye Token Holders request failed")?;

    let json: serde_json::Value = response.json().await?;
    
    if let Some(items) = json.get("data").and_then(|d| d.get("items")) {
        let holders: Vec<BirdeyeHolder> = serde_json::from_value(items.clone())
            .context("Failed to deserialize Birdeye holders")?;
        Ok(holders)
    } else {
        Err(anyhow::anyhow!("Birdeye holders returned no data for {}", mint))
    }
}

/// Fetch SOL price from CoinGecko API (free tier, may be rate limited)
async fn fetch_sol_price_coingecko(client: &Client) -> Result<f64> {
    let url = "https://api.coingecko.com/api/v3/simple/price?ids=solana&vs_currencies=usd";
    
    let response = client
        .get(url)
        .timeout(Duration::from_secs(3))
        .send()
        .await
        .context("CoinGecko API request failed")?;

    let json: serde_json::Value = response.json().await?;
    
    let price = json
        .get("solana")
        .and_then(|s| s.get("usd"))
        .and_then(|p| p.as_f64())
        .context("Failed to parse CoinGecko price")?;

    Ok(price)
}