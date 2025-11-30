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
}

#[derive(Debug, Clone, Serialize)]
pub struct HolderAnalysis {
    pub top_1_pct: f64,      // % held by top 1 holder (excluding pool)
    pub top_10_pct: f64,     // % held by top 10 holders (excluding pool)
    pub unique_holders: Option<u64>, // Total holder count (if available)
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

// Well-known token constants for defensive check
const WSOL_MINT: &str = "So11111111111111111111111111111111111111112";
const USDC_MINT: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
const USDT_MINT: &str = "Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB";
const BONK_MINT: &str = "DezXAZ8z7PnrnRJjz3wXBoRgixCa6xjnB7YaB1pPB263";
const JITOSOL_MINT: &str = "J1toso1uCk3RLmjorhTtrVwY9HJ7X8V9yYac6Y7kGCPn";
const MSOL_MINT: &str = "mSoLzYCxHdYgdzU16g5QSh3i5K3z3KZK7ytfqcJm7So";
const PYTH_MINT: &str = "HZ1JovNiVvGrGNiiYvEozEVgZ58xaU3RKwX8eACQBCt3";
const RAY_MINT: &str = "4k3Dyjzvzp8eMZWUXbBCjEvwSkkk59S5iCNLY3QrkX6R";
const ORCA_MINT: &str = "orcaEKTdK7LKz57vaAYr9QeNsVEPfiu6QeMU1kektZE";
const JUP_MINT: &str = "JUPyiwrYJFskUPiHa7hkeR8VUtAeFoSYbKedZNsDvCN";
const WIF_MINT: &str = "EKpQGSJtjMFqKZ9KQanSqYXRcF8fBopzLHYxdM65zcjm";
const POPCAT_MINT: &str = "7GCihgDB8fe6KNjn2MYtkzZcRjQy3t9GHdC8uHYmW2hr";

/// Check if a mint is a well-known token (defensive check)
fn is_well_known_token(mint: &str) -> bool {
    matches!(
        mint,
        WSOL_MINT | USDC_MINT | USDT_MINT | BONK_MINT
            | JITOSOL_MINT | MSOL_MINT | PYTH_MINT
            | RAY_MINT | ORCA_MINT | JUP_MINT
            | WIF_MINT | POPCAT_MINT
    )
}

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
    owner: Option<String>,
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
    if let Err(e) = update_sol_price_cache(&client).await {
        warn!("Failed to initialize SOL price cache: {}. Using default price.", e);
    }

    // Spawn background task to update SOL price every 30 seconds
    let price_client = client.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(30));
        loop {
            interval.tick().await;
            if let Err(e) = update_sol_price_cache(&price_client).await {
                warn!("Failed to update SOL price cache: {}", e);
            }
        }
    });

    tokio::spawn(async move {
        if let Err(e) = enrichment_loop(api_key, classified_rx, enriched_tx).await {
            error!("Enrichment pipeline error: {}", e);
        }
    });

    Ok(enriched_rx)
}

async fn enrichment_loop(
    api_key: String,
    mut classified_rx: mpsc::Receiver<ClassifiedEvent>,
    enriched_tx: mpsc::Sender<EnrichedToken>,
) -> Result<()> {
    let client = Client::builder()
        .timeout(Duration::from_secs(5))
        .build()?;

    while let Some(event) = classified_rx.recv().await {
        let start_time = std::time::Instant::now();
        
        match event {
            ClassifiedEvent::PoolCreation(pool_event) => {
                info!("🔍 Enriching token: {}", pool_event.token_mint);
                
                match enrich_token(&client, &api_key, pool_event).await {
                    Ok(enriched) => {
                        let duration = start_time.elapsed().as_millis();
                        info!("✅ Enrichment complete: {} (took {}ms)", enriched.mint, duration);
    
                        // Simplified logging - only show key fields
                        info!("📊 Token: {} | Liq: {} SOL | Auth: freeze={}, mint={} | Platform: Graduated",
                            enriched.mint,
                            enriched.initial_liquidity_sol.unwrap_or(0.0),
                            enriched.has_freeze_authority,
                            enriched.has_mint_authority
                        );
                        
                        if enriched_tx.send(enriched).await.is_err() {
                            warn!("Failed to send enriched token (receiver dropped)");
                            break;
                        }
                    }
                    Err(e) => {
                        error!("Failed to enrich token: {}", e);
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
    api_key: &str,
    pool_event: PoolCreationEvent,
) -> Result<EnrichedToken> {
    // DEFENSIVE CHECK: Never enrich well-known tokens
    // This should never happen if extract_pool_tokens() works correctly,
    // but this is a safety net
    if is_well_known_token(&pool_event.token_mint) {
        warn!("🚫 BLOCKED: Attempted to enrich well-known token: {}", pool_event.token_mint);
        return Err(anyhow::anyhow!("Cannot enrich well-known token: {}", pool_event.token_mint));
    }

    // SNIPER BOT: GRADUATED TOKENS ONLY (Pump.fun/Bonk.fun/LaunchLab)
    // Reject ALL non-graduated tokens - too risky for sniper bot
    if !pool_event.is_graduated {
        warn!("🚫 REJECTED: Non-graduated token - GRADUATED ONLY MODE");
        return Err(anyhow::anyhow!("Rejected non-graduated token: {}", pool_event.token_mint));
    }
    
    let start_time = std::time::Instant::now();
    // GRADUATED TOKENS ONLY - Single enrichment path
    // All tokens reaching this point are graduated (Pump.fun/Bonk.fun)
    // No need for tiered logic or tax simulation
    
    info!("⚡ Enriching graduated token: {} ({:?})", 
        pool_event.token_mint,
        pool_event.token_platform
    );
    
    // ========== PARALLEL RPC CALLS ==========
    // Fetch all required data concurrently for maximum speed
    // "Scatter-Gather" pattern: Fire all requests, wait for slowest
    let (
        mint_account_data,
        liquidity_data,
        metadata_data,
        holder_data,
        sol_price_data
    ) = tokio::join!(
        fetch_mint_account_cached(client, api_key, &pool_event.token_mint),
        fetch_liquidity_data(client, api_key, &pool_event.pool_address, &pool_event.pair_token),
        fetch_token_metadata(client, api_key, &pool_event.token_mint),
        fetch_holder_analysis(client, api_key, &pool_event.token_mint, &pool_event.pool_address),
        get_or_fetch_sol_price(client)
    );

    // Use fetched price or default if failed
    let sol_price = sol_price_data.unwrap_or(150.0);

    assemble_enriched_token(
        pool_event,
        mint_account_data.ok(),
        liquidity_data.ok(),
        metadata_data.ok(),
        holder_data.ok(),
        sol_price,
        start_time,
    )
}

/// Assemble EnrichedToken from fetched data
/// Assemble EnrichedToken from fetched data
fn assemble_enriched_token(
    pool_event: PoolCreationEvent,
    mint_data: Option<MintAccountData>,
    liq_data: Option<LiquidityData>,
    metadata: Option<TokenMetadata>,
    holders: Option<HolderAmounts>,
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

    // Calculate market metrics
    let (price_sol, price_usd, market_cap, fdv) = calculate_market_metrics(
        &liq_data,
        supply,
        decimals,
        sol_price,
    );

    // Fix Holder Analysis Percentages
    // The fetcher returns raw amounts. We calculate percentages here using supply.
    let final_holders = if let Some(h_amounts) = holders {
        if let Some(s) = supply {
            let s_f64 = s as f64;
            if s_f64 > 0.0 {
                Some(HolderAnalysis {
                    top_1_pct: (h_amounts.top_1_amount / s_f64) * 100.0,
                    top_10_pct: (h_amounts.top_10_amount / s_f64) * 100.0,
                    unique_holders: h_amounts.unique_holders,
                })
            } else {
                // Supply is 0, cannot calculate pct
                Some(HolderAnalysis {
                    top_1_pct: 0.0,
                    top_10_pct: 0.0,
                    unique_holders: h_amounts.unique_holders,
                })
            }
        } else {
            // No supply info, return 0s
             Some(HolderAnalysis {
                top_1_pct: 0.0,
                top_10_pct: 0.0,
                unique_holders: h_amounts.unique_holders,
            })
        }
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
        has_liquidity: liq_data.is_some(),
        pool_address: pool_event.pool_address.clone(),
        pair_token: pool_event.pair_token.clone(),
        dex: pool_event.dex.clone(),
        
        // Price data
        price_sol,
        price_usd,
        initial_liquidity_sol: liq_data.and_then(|l| l.liquidity_sol),
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
    pair_token: &str,
) -> Result<LiquidityData> {
    let url = format!("https://mainnet.helius-rpc.com/?api-key={}", api_key);
    
    // Fetch all token accounts owned by the pool
    let request_body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "getTokenAccountsByOwner",
        "params": [
            pool_address,
            {
                "programId": "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA"
            },
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
        .context("Failed to fetch pool token accounts")?;

    let response_json: serde_json::Value = response.json().await?;

    // Extract token accounts array
    let accounts = response_json
        .get("result")
        .and_then(|r| r.get("value"))
        .and_then(|v| v.as_array())
        .context("No token accounts found for pool")?;

    let mut liquidity_sol: Option<f64> = None;
    let mut liquidity_token: Option<f64> = None;

    // Common SOL/WSOL mint addresses
    let sol_mints = [
        "So11111111111111111111111111111111111111112", // WSOL
        "11111111111111111111111111111111",           // Native SOL
    ];

    // Parse each account to find SOL and token reserves
    for account in accounts {
        if let Some(data) = account.get("account").and_then(|a| a.get("data")) {
            let parsed = data.get("parsed").and_then(|p| p.get("info"));
            
            if let Some(info) = parsed {
                let mint = info.get("mint").and_then(|m| m.as_str()).unwrap_or("");
                let ui_amount = info
                    .get("tokenAmount")
                    .and_then(|t| t.get("uiAmount"))
                    .and_then(|u| u.as_f64());

                if let Some(amount) = ui_amount {
                    // Check if this is SOL/WSOL
                    if sol_mints.contains(&mint) {
                        liquidity_sol = Some(amount);
                    } else if mint == pair_token {
                        // This is the paired token (usually the new token)
                        liquidity_token = Some(amount);
                    }
                }
            }
        }
    }

    // Validate we found at least SOL liquidity
    if liquidity_sol.is_none() {
        warn!("No SOL liquidity found for pool {}", pool_address);
    }

    Ok(LiquidityData {
        liquidity_sol,
        liquidity_token,
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

    Ok(TokenMetadata {
        name,
        symbol,
        uri,
        socials,
    })
}

/// Temporary struct to hold raw amounts before percentage calculation
struct HolderAmounts {
    top_1_amount: f64,
    top_10_amount: f64,
    unique_holders: Option<u64>,
}

/// Fetch holder analysis using getTokenLargestAccounts
async fn fetch_holder_analysis(
    client: &Client,
    api_key: &str,
    mint: &str,
    pool_address: &str,
) -> Result<HolderAmounts> {
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

    // Calculate total supply from holders (approximation) or pass supply in?
    // We'll use the sum of top 20 as "circulating" for this check if supply isn't handy,
    // but better to calculate percentages based on the amounts returned.
    // The API returns raw amounts.
    
    let mut total_held = 0.0;
    let mut top_1_amount = 0.0;
    let mut top_10_amount = 0.0;
    let mut count = 0;

    for (_i, acc) in accounts.iter().enumerate() {
        let amount = acc.get("amount").and_then(|s| s.as_str()).and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.0);
        let address = acc.get("address").and_then(|s| s.as_str()).unwrap_or("");

        // CRITICAL: Exclude the Pool Address
        if address == pool_address {
            continue;
        }

        if count == 0 {
            top_1_amount = amount;
        }
        if count < 10 {
            top_10_amount += amount;
        }
        
        total_held += amount;
        count += 1;
    }

    // If we only found the pool, return 0s
    if total_held == 0.0 {
        return Ok(HolderAmounts {
            top_1_amount: 0.0,
            top_10_amount: 0.0,
            unique_holders: None,
        });
    }

    // We need total supply to calculate true percentages.
    // Since we don't have it easily here without passing it down, 
    // we can use the sum of top 20 + pool as a proxy for total supply, 
    // OR just return the raw amounts? 
    // Better: We fetched supply in `fetch_mint_account_cached`. 
    // But we are running in parallel! We don't have supply yet.
    // Solution: Return raw amounts or percentages of *visible* supply?
    // Actually, `getTokenLargestAccounts` returns amounts. 
    // Let's assume the supply is roughly the sum of top 20 + pool for fresh tokens.
    // Or better, let's just return the raw amounts and calculate percentages in `assemble`?
    // No, `assemble` has the supply. Let's return raw amounts here and convert to pct in `assemble`?
    // The struct expects f64 pct. 
    // Let's fetch supply inside here? No, redundant.
    // Let's change the struct to return amounts, then calculate pct in `assemble`.
    
    // WAIT: `assemble` receives `mint_data` which has supply.
    // So `fetch_holder_analysis` should return the amounts, and `assemble` calculates the %.
    // But I defined `HolderAnalysis` with `top_1_pct`.
    // Let's stick to the plan: `assemble` will do the math.
    // I will modify `HolderAnalysis` to store amounts temporarily? 
    // No, I'll just change the return type of this function to a temporary struct or tuple.
    
    Ok(HolderAmounts {
        top_1_amount, 
        top_10_amount,
        unique_holders: None,
    })
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
async fn get_or_fetch_sol_price(client: &Client) -> Result<f64> {
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
    update_sol_price_cache(client).await
}

/// Fetch real-time SOL price from multiple sources with fallback
pub async fn update_sol_price_cache(client: &Client) -> Result<f64> {
    // Try multiple sources in order of preference
    let price = match fetch_sol_price_jupiter(client).await {
        Ok(p) => p,
        Err(_) => match fetch_sol_price_birdeye(client).await {
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

/// Fetch SOL price from Jupiter Price API v3 (Lite)
async fn fetch_sol_price_jupiter(client: &Client) -> Result<f64> {
    // Using the specific Lite API endpoint for SOL
    let url = "https://lite-api.jup.ag/price/v3?ids=So11111111111111111111111111111111111111112";
    
    let response = client
        .get(url)
        .timeout(Duration::from_secs(3))
        .send()
        .await
        .context("Jupiter API request failed")?;

    let json: serde_json::Value = response.json().await?;
    
    // Parse response: {"So11...": {"usdPrice": ...}}
    let price = json
        .get("So11111111111111111111111111111111111111112")
        .and_then(|d| d.get("usdPrice"))
        .and_then(|p| p.as_f64())
        .context("Failed to parse Jupiter price")?;

    Ok(price)
}

/// Fetch SOL price from Birdeye API
async fn fetch_sol_price_birdeye(client: &Client) -> Result<f64> {
    let url = "https://public-api.birdeye.so/public/price?address=So11111111111111111111111111111111111111112";
    
    let response = client
        .get(url)
        .timeout(Duration::from_secs(3))
        .send()
        .await
        .context("Birdeye API request failed")?;

    let json: serde_json::Value = response.json().await?;
    
    let price = json
        .get("data")
        .and_then(|d| d.get("value"))
        .and_then(|p| p.as_f64())
        .context("Failed to parse Birdeye price")?;

    Ok(price)
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