use crate::config::Config;
use anyhow::{Context, Result};
use futures_util::{SinkExt, StreamExt};
use log::{error, info, warn};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tokio::time::sleep;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use std::sync::Arc;
use crate::rate_limiter::RateLimiter;
use reqwest::Client;
use crate::trade_logger::TradeLogger;

/// Raw transaction event from Helius
#[derive(Debug, Clone)]
pub struct RawTxEvent {
    pub signature: String,
    pub slot: u64,
    pub timestamp: Option<i64>,
    pub data: serde_json::Value,
}

/// Classified event types from the pipeline (POOL CREATION ONLY)
#[derive(Debug, Clone)]
pub enum ClassifiedEvent {
    PoolCreation(PoolCreationEvent),
}

impl ClassifiedEvent {
    pub fn signature(&self) -> &str {
        match self {
            ClassifiedEvent::PoolCreation(e) => &e.signature,
        }
    }
}

/// Pool creation event - covers ALL DEXes with platform detection
#[derive(Debug, Clone)]
pub struct PoolCreationEvent {
    pub pool_address: String,
    pub token_mint: String,     // The new token being listed
    pub pair_token: String,      // Usually SOL or USDC
    pub signature: String,
    pub slot: u64,
    pub timestamp: Option<i64>,
    pub dex: String,             // "Raydium AMM v4", "Raydium CPMM", "Orca Whirlpool", etc.
    pub token_platform: TokenPlatform, // NEW: Track token origin
    pub is_graduated: bool,      // NEW: True if migrated from bonding curve
}

/// Token platform detection (GRADUATED TOKENS ONLY)
#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum TokenPlatform {
    PumpFun,   // Graduated from Pump.fun bonding curve
    BonkFun,   // Graduated from Bonk.fun bonding curve
    RaydiumLaunchLab, // Graduated from Raydium LaunchLab
    Unknown,   // Not from a tracked platform
}

impl TokenPlatform {
    fn is_graduated(&self) -> bool {
        matches!(self, Self::PumpFun | Self::BonkFun | Self::RaydiumLaunchLab)
    }
}

/// Helius WebSocket message types
#[derive(Debug, Deserialize)]
#[serde(untagged)]
#[allow(dead_code)]
enum HeliusMessage {
    SubscriptionResult {
        jsonrpc: String,
        id: u64,
        result: Option<u64>,
        error: Option<serde_json::Value>,
    },
    LogsNotification {
        jsonrpc: String,
        method: String,
        params: LogsParams,
    },
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct LogsParams {
    result: LogsResult,
    subscription: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct LogsResult {
    context: serde_json::Value,
    value: LogsValue,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct LogsValue {
    signature: String,
    logs: Vec<String>,
    err: Option<serde_json::Value>,
}

// ========== PROGRAM IDs ==========

/// SPL Token program ID
const SPL_TOKEN_PROGRAM_ID: &str = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";

/// Raydium AMM v4 program ID (Most popular Raydium pools)
const RAYDIUM_AMM_V4_PROGRAM_ID: &str = "675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8";

/// Raydium CPMM program ID (Concentrated liquidity)
const RAYDIUM_CPMM_PROGRAM_ID: &str = "CPMMoo8L3F4NbTegBCKVNunggL7H1ZpdTHKxQB5qKP1C";

/// Raydium LaunchLab (Community-powered token launches)
const RAYDIUM_LAUNCHLAB_PROGRAM_ID: &str = "LanMV9sAd7wArD4vJFi2qDdfnVhFxYSUg6eADduJ3uj";

/// PumpSwap / Pump.fun AMM (Post-bonding curve trading)
const PUMP_FUN_AMM_PROGRAM_ID: &str = "pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA";

/// Pump.fun program ID (Bonding curve platform)
const PUMP_FUN_PROGRAM_ID: &str = "6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P";

/// **CRITICAL: Pump.fun Migration Account** - Monitors token graduations
const PUMPFUN_MIGRATION_ACCOUNT: &str = "39azUYFWPz3VHgKCf3VChUwbpURdCHRxjWVowf5jUJjg";

/// Bonk.fun program ID
const BONK_FUN_PROGRAM_ID: &str = "FfYek5vEz23cMkWsdJwG2oa6EphsvXSHrGpdALN4g6W1";



// ========== WELL-KNOWN TOKENS (TO EXCLUDE) ==========

/// Wrapped SOL (WSOL)
const WSOL_MINT: &str = "So11111111111111111111111111111111111111112";

/// USDC
const USDC_MINT: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";

/// USDT
const USDT_MINT: &str = "Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB";

/// Bonk
const BONK_MINT: &str = "DezXAZ8z7PnrnRJjz3wXBoRgixCa6xjnB7YaB1pPB263";

/// Jito SOL
const JITOSOL_MINT: &str = "J1toso1uCk3RLmjorhTtrVwY9HJ7X8V9yYac6Y7kGCPn";

/// mSOL (Marinade)
const MSOL_MINT: &str = "mSoLzYCxHdYgdzU16g5QSh3i5K3z3KZK7ytfqcJm7So";

/// Pyth
const PYTH_MINT: &str = "HZ1JovNiVvGrGNiiYvEozEVgZ58xaU3RKwX8eACQBCt3";

/// RAY (Raydium)
const RAY_MINT: &str = "4k3Dyjzvzp8eMZWUXbBCjEvwSkkk59S5iCNLY3QrkX6R";

/// ORCA
const ORCA_MINT: &str = "orcaEKTdK7LKz57vaAYr9QeNsVEPfiu6QeMU1kektZE";

/// Jupiter
const JUP_MINT: &str = "JUPyiwrYJFskUPiHa7hkeR8VUtAeFoSYbKedZNsDvCN";

/// WIF (dogwifhat)
const WIF_MINT: &str = "EKpQGSJtjMFqKZ9KQanSqYXRcF8fBopzLHYxdM65zcjm";

/// POPCAT
const POPCAT_MINT: &str = "7GCihgDB8fe6KNjn2MYtkzZcRjQy3t9GHdC8uHYmW2hr";

// ========== CONNECTION POOL CONFIGURATION ==========
const WS_POOL_SIZE: usize = 3; // Number of concurrent WebSocket connections
const PING_INTERVAL_SECS: u64 = 30; // Send ping every 30 seconds to keep connection alive

// ========== SNIPER BOT CONFIGURATION ==========
/// Maximum age for graduated tokens (in seconds) - only snipe fresh tokens
const MAX_TOKEN_AGE_SECONDS: i64 = 60; // 1 minute

/// Start the Helius listener with connection pool and optimized sniper architecture
pub async fn start(config: &Config) -> Result<mpsc::Receiver<ClassifiedEvent>> {
    let (raw_tx, raw_rx) = mpsc::channel::<RawTxEvent>(2000); // Increased buffer for multiple connections
    let (classified_tx, classified_rx) = mpsc::channel::<ClassifiedEvent>(2000);

    let api_key = config.helius_api_key.clone();

    info!("🚀 Starting Helius Sniper Listener (GRADUATED TOKENS ONLY)");
    info!("🎯 Strategy: Monitor Pump.fun/Bonk.fun graduated tokens EXCLUSIVELY");
    info!("⚡ Focus: Fresh graduated tokens (max age: {}s)", MAX_TOKEN_AGE_SECONDS);
    info!("🔗 Connection Pool: {} WebSocket connections", WS_POOL_SIZE);
    info!("💓 Heartbeat: Ping every {}s to prevent disconnection", PING_INTERVAL_SECS);

    // Initialize shared resources
    let client = Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap_or_else(|_| Client::new());
        
    let rate_limiter = if config.rate_limiting_enabled {
        Some(Arc::new(RateLimiter::new(
            config.helius_max_requests_per_second,
            "HeliusListener",
        )))
    } else {
        None
    };

    // Start multiple websocket listeners (connection pool)
    for connection_id in 0..WS_POOL_SIZE {
        let api_key_clone = api_key.clone();
        let raw_tx_clone = raw_tx.clone();
        let client_clone = client.clone();
        let limiter_clone = rate_limiter.clone();
        
        tokio::spawn(async move {
            listener_loop(connection_id, api_key_clone, raw_tx_clone, client_clone, limiter_clone).await;
        });
    }

    // Start the event classification pipeline
    tokio::spawn(async move {
        if let Err(e) = event_pipeline(raw_rx, classified_tx).await {
            error!("Event pipeline error: {}", e);
        }
    });

    Ok(classified_rx)
}

async fn listener_loop(
    connection_id: usize, 
    api_key: String, 
    tx: mpsc::Sender<RawTxEvent>,
    client: Client,
    rate_limiter: Option<Arc<RateLimiter>>,
) {
    let mut backoff_seconds = 1;
    let max_backoff = 60;

    info!("[Connection #{}] Listener starting (MAINNET WebSocket)", connection_id);

    loop {
        info!("[Connection #{}] Connecting to Helius WebSocket (mainnet)", connection_id);

        match connect_and_listen(connection_id, &api_key, &tx, &client, &rate_limiter).await {
            Ok(()) => {
                info!("[Connection #{}] WebSocket connection closed normally", connection_id);
                backoff_seconds = 1;
            }
            Err(e) => {
                error!(
                    "[Connection #{}] WebSocket error: {}. Reconnecting in {}s...",
                    connection_id, e, backoff_seconds
                );
                sleep(Duration::from_secs(backoff_seconds)).await;
                backoff_seconds = std::cmp::min(backoff_seconds * 2, max_backoff);
            }
        }
    }
}

async fn connect_and_listen(
    connection_id: usize,
    api_key: &str,
    tx: &mpsc::Sender<RawTxEvent>,
    client: &Client,
    rate_limiter: &Option<Arc<RateLimiter>>,
) -> Result<()> {
    let ws_url = format!("wss://mainnet.helius-rpc.com/?api-key={}", api_key);

    info!(
        "[Connection #{}] Connecting to Helius WebSocket: {}",
        connection_id,
        ws_url.replace(api_key, "***")
    );

    let (ws_stream, _) = connect_async(&ws_url)
        .await
        .context("Failed to connect to Helius WebSocket")?;

    info!("[Connection #{}] ✅ Connected to Helius WebSocket (mainnet)", connection_id);

    let (mut write, mut read) = ws_stream.split();

    // ========== CRITICAL: PUMP.FUN MIGRATION MONITORING ==========
    // This catches tokens graduating from bonding curve to Raydium
    let pumpfun_migration_sub = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "logsSubscribe",
        "params": [
            {
                "mentions": [PUMPFUN_MIGRATION_ACCOUNT]
            },
            {
                "commitment": "confirmed"
            }
        ]
    });

    write
        .send(Message::Text(pumpfun_migration_sub.to_string()))
        .await?;
    info!("[Connection #{}] 🎯 PRIORITY: Subscribed to Pump.fun Migration Account", connection_id);

    // ========== POOL CREATION MONITORING (RAYDIUM ONLY - WHERE GRADUATED TOKENS MIGRATE) ==========
    let dex_programs = vec![
        (10, RAYDIUM_AMM_V4_PROGRAM_ID, "Raydium AMM v4"),
        (11, RAYDIUM_CPMM_PROGRAM_ID, "Raydium CPMM"),
        (12, RAYDIUM_LAUNCHLAB_PROGRAM_ID, "Raydium LaunchLab"),
        // (13, PUMP_FUN_PROGRAM_ID, "Pump.fun"), // REMOVED: We don't want bonding curve events
        (14, PUMP_FUN_AMM_PROGRAM_ID, "PumpSwap"), // Pump.fun AMM
    ];

    for (id, program_id, name) in dex_programs {
        let subscribe_msg = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "logsSubscribe",
            "params": [
                {
                    "mentions": [program_id]
                },
                {
                    "commitment": "confirmed"
                }
            ]
        });

        write
            .send(Message::Text(subscribe_msg.to_string()))
            .await?;
        info!("[Connection #{}] 📡 Subscribed to graduated token pool: {}", connection_id, name);
    }

    // ========== HEARTBEAT/PING TASK ==========
    // Spawn a task to send periodic pings to keep connection alive
    let (ping_tx, mut ping_rx) = mpsc::channel::<()>(1);
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(PING_INTERVAL_SECS));
        loop {
            interval.tick().await;
            if ping_tx.send(()).await.is_err() {
                break; // Connection closed
            }
        }
    });

    // Listen for messages and handle pings
    loop {
        tokio::select! {
            // Handle incoming messages
            msg = read.next() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        if let Err(e) = handle_message(&text, api_key, tx, client, rate_limiter).await {
                            warn!("[Connection #{}] Error handling message: {}", connection_id, e);
                        }
                    }
                    Some(Ok(Message::Close(_))) => {
                        info!("[Connection #{}] WebSocket closed by server", connection_id);
                        break;
                    }
                    Some(Ok(Message::Ping(data))) => {
                        if let Err(e) = write.send(Message::Pong(data)).await {
                            error!("[Connection #{}] Failed to send pong: {}", connection_id, e);
                            break;
                        }
                    }
                    Some(Ok(Message::Pong(_))) => {
                        // Received pong response - connection is alive
                    }
                    Some(Ok(_)) => {}
                    Some(Err(e)) => {
                        return Err(anyhow::anyhow!("[Connection #{}] WebSocket error: {}", connection_id, e));
                    }
                    None => {
                        info!("[Connection #{}] WebSocket stream ended", connection_id);
                        break;
                    }
                }
            }
            // Send periodic pings
            Some(_) = ping_rx.recv() => {
                if let Err(e) = write.send(Message::Ping(vec![])).await {
                    error!("[Connection #{}] Failed to send ping: {}", connection_id, e);
                    break;
                }
                info!("[Connection #{}] 💓 Sent heartbeat ping", connection_id);
            }
        }
    }

    Ok(())
}

async fn handle_message(
    text: &str,
    api_key: &str,
    tx: &mpsc::Sender<RawTxEvent>,
    client: &Client,
    rate_limiter: &Option<Arc<RateLimiter>>,
) -> Result<()> {
    let msg: HeliusMessage =
        serde_json::from_str(text).context("Failed to parse Helius message")?;

    match msg {
        HeliusMessage::SubscriptionResult {
            id, result, error, ..
        } => {
            if let Some(err) = error {
                return Err(anyhow::anyhow!("Subscription error: {}", err));
            }
            if let Some(sub_id) = result {
                info!(
                    "✅ Subscription confirmed (id: {}, subscription: {})",
                    id, sub_id
                );
            }
        }
        HeliusMessage::LogsNotification { params, .. } => {
            let signature = params.result.value.signature;
            let logs = params.result.value.logs;
            let slot = params.result.context.get("slot").and_then(|s| s.as_u64()).unwrap_or(0);

            // CRITICAL: Quick filter for pool creation keywords
            let is_pool_creation = logs.iter().any(|log| {
                log.contains("initialize") ||
                log.contains("Initialize2") ||
                log.contains("InitializePool") ||
                log.contains("create") ||
                log.contains("CreatePool") ||
                log.contains(PUMPFUN_MIGRATION_ACCOUNT) // Graduation event!
            });

            if !is_pool_creation {
                return Ok(());
            }

            info!("⚡ Pool creation detected: {}", signature);

            // Fetch full transaction details
            if let Some(limiter) = rate_limiter {
                limiter.acquire().await;
            }
            
            let rpc_url = format!("https://mainnet.helius-rpc.com/?api-key={}", api_key);
            
            match fetch_transaction(client, &rpc_url, &signature).await {
                Ok(tx_data) => {
                    let slot = tx_data.get("slot").and_then(|s| s.as_u64()).unwrap_or(slot);
                    let block_time = tx_data.get("blockTime").and_then(|t| t.as_i64());
                    
                    let raw_event = RawTxEvent {
                        signature: signature.clone(),
                        slot,
                        timestamp: block_time,
                        data: json!({
                            "transaction": tx_data.get("transaction"),
                            "meta": tx_data.get("meta"),
                            "signature": signature,
                            "slot": slot,
                            "timestamp": block_time,
                            "logs": logs,
                        }),
                    };

                    if tx.send(raw_event).await.is_err() {
                        warn!("Failed to send event to channel");
                        return Err(anyhow::anyhow!("Channel receiver dropped"));
                    }
                    info!("✅ Forwarded transaction: {}", signature);
                }
                Err(e) => {
                    warn!("Failed to fetch transaction {}: {}", signature, e);
                }
            }
        }
    }

    Ok(())
}

/// Event classification pipeline - POOL CREATION ONLY
async fn event_pipeline(
    mut raw_rx: mpsc::Receiver<RawTxEvent>,
    classified_tx: mpsc::Sender<ClassifiedEvent>,
) -> Result<()> {
    let mut event_buffer: HashMap<String, RawTxEvent> = HashMap::new();
    let mut last_process_time = Instant::now();
    let debounce_duration = Duration::from_millis(100); // Reduced for speed

    info!("🎯 Event pipeline started (POOL CREATION ONLY - OPTIMIZED FOR SPEED)");

    loop {
        tokio::select! {
            Some(raw_event) = raw_rx.recv() => {
                event_buffer.insert(raw_event.signature.clone(), raw_event);
            }
            _ = sleep(debounce_duration) => {
                if !event_buffer.is_empty() && last_process_time.elapsed() >= debounce_duration {
                    process_event_batch(&mut event_buffer, &classified_tx).await;
                    last_process_time = Instant::now();
                }
            }
        }
    }
}

/// Process event batch
async fn process_event_batch(
    event_buffer: &mut HashMap<String, RawTxEvent>,
    classified_tx: &mpsc::Sender<ClassifiedEvent>,
) {
    let events: Vec<_> = event_buffer.drain().collect();

    info!("📦 Processing batch of {} events", events.len());

    for (_key, event) in events {
        if let Some(pool_event) = detect_pool_creation_event(&event) {
            // Log graduated tokens prominently
            if pool_event.is_graduated {
                let log_msg = format!(
                    "🎓 GRADUATED TOKEN DETECTED: {} (platform: {:?}, dex: {})",
                    pool_event.token_mint,
                    pool_event.token_platform,
                    pool_event.dex
                );
                info!("{}", log_msg);
                TradeLogger::log(&log_msg);
            }

            info!("🏊 POOL CREATION: {:#?}", pool_event);
            
            if classified_tx.send(ClassifiedEvent::PoolCreation(pool_event)).await.is_err() {
                warn!("Failed to send pool event (receiver dropped)");
                break;
            }
        }
    }
}

/// Detect pool creation events with platform detection
fn detect_pool_creation_event(event: &RawTxEvent) -> Option<PoolCreationEvent> {
    let transaction = event.data.get("transaction")?;
    let logs = event.data.get("logs")?;
    let meta = event.data.get("meta");
    let logs_array = logs.as_array()?;

    // Extract token mints first
    let (token_mint, pair_token) = extract_pool_tokens(transaction, meta)?;

    // CRITICAL: Filter out established tokens and invalid pairs
    if !is_valid_new_token_pair(&token_mint, &pair_token) {
        return None;
    }

    // Detect platform from transaction account keys (SECURE)
    let token_platform = detect_token_platform(transaction, logs_array);
    let is_graduated = token_platform.is_graduated();

    // ========== SNIPER BOT: GRADUATED TOKENS ONLY ==========
    // CRITICAL: Reject ALL non-graduated tokens
    // Only process Pump.fun and Bonk.fun graduated tokens
    if !is_graduated {
        return None; // Silent rejection for non-graduated tokens
    }

    // ========== AGE FILTER: FRESH TOKENS ONLY ==========
    // Reject tokens older than MAX_TOKEN_AGE_SECONDS
    if let Some(timestamp) = event.timestamp {
        let current_time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        
        let token_age = current_time - timestamp;
        
        if token_age > MAX_TOKEN_AGE_SECONDS {
            warn!(
                "🚫 REJECTED (AGE): Token is {} seconds old (max: {}s) - {} (platform: {:?})",
                token_age, MAX_TOKEN_AGE_SECONDS, token_mint, token_platform
            );
            return None;
        }
        
        info!(
            "✅ AGE CHECK PASSED: Token is {} seconds old (max: {}s) - {} (platform: {:?})",
            token_age, MAX_TOKEN_AGE_SECONDS, token_mint, token_platform
        );
    } else {
        warn!(
            "⚠️ No timestamp available for token {} - cannot verify age, proceeding with caution",
            token_mint
        );
    }

    // ========== ALL VALIDATIONS PASSED ==========
    info!(
        "✅ TOKEN ACCEPTED: {} paired with {} | Platform: {:?} | DEX: TBD | Age: FRESH",
        token_mint, pair_token, token_platform
    );

    // Detect DEX
    let dex = detect_dex_from_logs(logs_array)?;


    // ========== GRADUATION VERIFICATION (BONK.FUN & LAUNCHLAB) ==========
    // Ensure these tokens are actually on a valid AMM (Raydium or PumpSwap)
    if token_platform == TokenPlatform::BonkFun || token_platform == TokenPlatform::RaydiumLaunchLab {
        let is_valid_dex = dex.starts_with("Raydium") || dex == "PumpSwap";
        if !is_valid_dex {
            warn!(
                "🚫 REJECTED: Token {} (platform: {:?}) detected on invalid DEX '{}' - waiting for graduation to Raydium/PumpSwap",
                token_mint, token_platform, dex
            );
            return None;
        }
    }

    // Extract pool address
    let pool_address = extract_pool_address(transaction, meta, logs_array)?;

    Some(PoolCreationEvent {
        pool_address,
        token_mint,
        pair_token,
        signature: event.signature.clone(),
        slot: event.slot,
        timestamp: event.timestamp,
        dex,
        token_platform,
        is_graduated,
    })
}

/// Detect token platform from transaction account keys and logs
/// SECURITY: Uses cryptographic proof via account keys, NOT mint suffix
fn detect_token_platform(
    transaction: &serde_json::Value,
    logs: &[serde_json::Value]
) -> TokenPlatform {
    // LAYER 1: Check account keys (MOST SECURE - cryptographic proof)
    // This cannot be faked by scammers creating tokens ending in "pump" or "bonk"
    if let Some(message) = transaction.get("message") {
        if let Some(account_keys) = message.get("accountKeys").and_then(|k| k.as_array()) {
            for key in account_keys {
                // Extract pubkey (handles both string and object formats)
                let pubkey = if let Some(obj) = key.as_object() {
                    obj.get("pubkey").and_then(|p| p.as_str()).unwrap_or("")
                } else {
                    key.as_str().unwrap_or("")
                };
                
                // Check for Pump.fun migration account (graduated tokens)
                if pubkey == PUMPFUN_MIGRATION_ACCOUNT {
                    return TokenPlatform::PumpFun;
                }
                
                // Check for Bonk.fun program
                if pubkey == BONK_FUN_PROGRAM_ID {
                    return TokenPlatform::BonkFun;
                }

                // Check for Raydium LaunchLab program
                if pubkey == RAYDIUM_LAUNCHLAB_PROGRAM_ID {
                    return TokenPlatform::RaydiumLaunchLab;
                }
            }
        }
    }

    // LAYER 2: Check logs for program IDs (FALLBACK)
    // Less secure than account keys but still validates program interaction
    for log in logs {
        if let Some(log_str) = log.as_str() {
            // Check for Pump.fun migration account (CRITICAL for graduated tokens)
            if log_str.contains(PUMPFUN_MIGRATION_ACCOUNT) {
                return TokenPlatform::PumpFun;
            }
            
            // Check for other program IDs
            if log_str.contains(BONK_FUN_PROGRAM_ID) {
                return TokenPlatform::BonkFun;
            }
            
            // Check for Raydium LaunchLab
            if log_str.contains(RAYDIUM_LAUNCHLAB_PROGRAM_ID) {
                // Check for specific graduation instructions if possible, or assume presence means interaction
                // Launchlab graduation often involves "migrate_to_amm"
                if log_str.contains("migrate_to_amm") || log_str.contains("migrate_to_cpswap") {
                     return TokenPlatform::RaydiumLaunchLab;
                }
                // Fallback: if we see the program ID, it's likely a LaunchLab token
                return TokenPlatform::RaydiumLaunchLab;
            }
        }
    }

    // DEFAULT: Unknown (not from tracked platforms)
    TokenPlatform::Unknown
}

/// Detect DEX from logs (RAYDIUM ONLY - where graduated tokens migrate)
fn detect_dex_from_logs(logs: &[serde_json::Value]) -> Option<String> {
    let mut found_dex = None;

    for log in logs {
        if let Some(log_str) = log.as_str() {
            // Priority 1: Raydium AMM v4 (Most common for graduated tokens)
            if log_str.contains(RAYDIUM_AMM_V4_PROGRAM_ID) {
                return Some("Raydium AMM v4".to_string());
            }
            // Priority 2: Raydium CPMM
            if log_str.contains(RAYDIUM_CPMM_PROGRAM_ID) {
                return Some("Raydium CPMM".to_string());
            }
            // Priority 3: PumpSwap (Pump.fun AMM)
            if log_str.contains(PUMP_FUN_AMM_PROGRAM_ID) {
                return Some("PumpSwap".to_string());
            }
            // Priority 4: Raydium LaunchLab
            if log_str.contains(RAYDIUM_LAUNCHLAB_PROGRAM_ID) {
                found_dex = Some("Raydium LaunchLab".to_string());
            }
        }
    }
    
    found_dex.or_else(|| Some("Unknown DEX".to_string()))
}

/// Extract pool address from transaction
/// Priority: 1. Parse inner instructions for AMM program accounts
///           2. Parse logs for "pool: <address>" pattern
///           3. Fallback to finding accounts invoked by known AMM programs
fn extract_pool_address(
    transaction: &serde_json::Value,
    meta: Option<&serde_json::Value>,
    logs: &[serde_json::Value],
) -> Option<String> {
    // Known AMM program IDs
    let amm_programs = [
        PUMP_FUN_AMM_PROGRAM_ID,     // PumpSwap
        RAYDIUM_AMM_V4_PROGRAM_ID,   // Raydium v4
        RAYDIUM_CPMM_PROGRAM_ID,     // Raydium CPMM
    ];

    // STRATEGY 1: Parse innerInstructions for accounts invoked by AMM programs
    // The pool is typically the first account in an inner instruction from the AMM
    if let Some(meta) = meta {
        if let Some(inner_instructions) = meta.get("innerInstructions").and_then(|i| i.as_array()) {
            let message = transaction.get("message");
            let account_keys = message
                .and_then(|m| m.get("accountKeys"))
                .and_then(|k| k.as_array());

            if let Some(keys) = account_keys {
                for inner in inner_instructions {
                    if let Some(instructions) = inner.get("instructions").and_then(|i| i.as_array()) {
                        for ix in instructions {
                            // Get program ID for this instruction
                            let program_id_index = ix.get("programIdIndex").and_then(|i| i.as_u64());
                            
                            if let Some(idx) = program_id_index {
                                let program_id = get_pubkey_at_index(keys, idx as usize);
                                
                                // Check if this instruction is from an AMM program
                                if let Some(pid) = &program_id {
                                    if amm_programs.contains(&pid.as_str()) {
                                        // Get the accounts used by this instruction
                                        if let Some(accounts) = ix.get("accounts").and_then(|a| a.as_array()) {
                                            // The pool is typically at index 0 or 1 in AMM instructions
                                            for &account_idx in &[0usize, 1, 2] {
                                                if account_idx < accounts.len() {
                                                    if let Some(acc_idx) = accounts[account_idx].as_u64() {
                                                        if let Some(pool) = get_pubkey_at_index(keys, acc_idx as usize) {
                                                            // Validate: not a program, not token mint, not well-known
                                                            if !is_program_id(&pool) 
                                                                && pool.len() == 44 
                                                                && !is_well_known_token(&pool) 
                                                            {
                                                                info!("🎯 Pool extracted from AMM inner instruction: {}", pool);
                                                                return Some(pool);
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // STRATEGY 2: Parse logs for explicit "pool: <address>" pattern
    for log in logs {
        if let Some(log_str) = log.as_str() {
            if let Some(addr_start) = log_str.find("pool: ") {
                let addr = &log_str[addr_start + 6..]; 
                // Extract until space or end
                let end_idx = addr.find(|c: char| c.is_whitespace()).unwrap_or(addr.len());
                let pool_addr = &addr[..end_idx];
                if pool_addr.len() == 44 && !is_program_id(pool_addr) {
                    info!("🎯 Pool extracted from logs: {}", pool_addr);
                    return Some(pool_addr.to_string());
                }
            }
        }
    }

    // STRATEGY 3: Find account that is owned by AMM program in postTokenBalances
    // This is less reliable but can work for identifying pool vaults
    if let Some(meta) = meta {
        if let Some(post_balances) = meta.get("postTokenBalances").and_then(|b| b.as_array()) {
            for balance in post_balances {
                if let Some(owner) = balance.get("owner").and_then(|o| o.as_str()) {
                    // If the owner is an AMM program, this might be the pool
                    if amm_programs.contains(&owner) {
                        // Get the account index
                        if let Some(account_index) = balance.get("accountIndex").and_then(|i| i.as_u64()) {
                            let message = transaction.get("message");
                            let account_keys = message
                                .and_then(|m| m.get("accountKeys"))
                                .and_then(|k| k.as_array());
                            
                            if let Some(keys) = account_keys {
                                if let Some(pool) = get_pubkey_at_index(keys, account_index as usize) {
                                    info!("🎯 Pool extracted from token balance owner: {}", pool);
                                    return Some(pool);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    warn!("⚠️ Could not extract pool address from transaction");
    None
}

/// Helper: Get pubkey at index from account keys array
fn get_pubkey_at_index(keys: &[serde_json::Value], index: usize) -> Option<String> {
    keys.get(index).and_then(|key| {
        // Handle both object format {"pubkey": "..."} and string format
        if let Some(obj) = key.as_object() {
            obj.get("pubkey").and_then(|p| p.as_str()).map(|s| s.to_string())
        } else {
            key.as_str().map(|s| s.to_string())
        }
    })
}

/// Extract token pair from pool
/// CRITICAL: Always returns (new_token, well_known_token) to prevent well-known tokens from being enriched
fn extract_pool_tokens(
    transaction: &serde_json::Value,
    meta: Option<&serde_json::Value>,
) -> Option<(String, String)> {
    // Extract raw tokens first
    let (token_a, token_b) = if let Some(meta) = meta {
        if let Some(post_balances) = meta.get("postTokenBalances").and_then(|b| b.as_array()) {
            // CRITICAL FIX: Deduplicate tokens from postTokenBalances
            // The same token can appear multiple times (different accounts)
            let mut unique_tokens = HashSet::new();
            for balance in post_balances {
                if let Some(mint) = balance.get("mint").and_then(|m| m.as_str()) {
                    unique_tokens.insert(mint.to_string());
                }
            }
            
            let tokens: Vec<String> = unique_tokens.into_iter().collect();
            
            if tokens.len() >= 2 {
                (tokens[0].clone(), tokens[1].clone())
            } else if tokens.len() == 1 {
                (tokens[0].clone(), "So11111111111111111111111111111111111111112".to_string())
            } else {
                // Fallback to account keys parsing
                extract_from_account_keys(transaction)?
            }
        } else {
            // Fallback to account keys parsing
            extract_from_account_keys(transaction)?
        }
    } else {
        // Fallback to account keys parsing
        extract_from_account_keys(transaction)?
    };

    // CRITICAL FIX: Ensure new token is always first, well-known token second
    // This prevents SOL/USDC/USDT from being enriched as new tokens
    let (new_token, pair_token) = if is_well_known_token(&token_a) && !is_well_known_token(&token_b) {
        // token_a is well-known, token_b is new -> swap to put new token first
        (token_b, token_a)
    } else if is_well_known_token(&token_b) && !is_well_known_token(&token_a) {
        // token_b is well-known, token_a is new -> already in correct order
        (token_a, token_b)
    } else {
        // Neither or both are well-known - will be filtered by is_valid_new_token_pair
        // Keep original order
        (token_a, token_b)
    };

    // SAFETY CHECK: Ensure we never return identical tokens
    if new_token == pair_token {
        warn!(
            "⚠️ Token extraction error: Extracted identical tokens ({}). This indicates a transaction parsing issue.",
            new_token
        );
        return None;
    }

    info!(
        "🔍 Extracted token pair: new_token={}, pair_token={}",
        new_token, pair_token
    );

    Some((new_token, pair_token))
}

/// Helper function to extract tokens from account keys (fallback method)
fn extract_from_account_keys(transaction: &serde_json::Value) -> Option<(String, String)> {
    let message = transaction.get("message")?;
    let account_keys = message.get("accountKeys")?.as_array()?;

    // Use HashSet to deduplicate tokens
    let mut tokens = HashSet::new();
    for key in account_keys {
        if let Some(pubkey) = key.get("pubkey").and_then(|p| p.as_str()) {
            if !is_program_id(pubkey) && is_token_mint(pubkey) {
                tokens.insert(pubkey.to_string());
            }
        } else if let Some(pubkey_str) = key.as_str() {
            if !is_program_id(pubkey_str) && is_token_mint(pubkey_str) {
                tokens.insert(pubkey_str.to_string());
            }
        }
    }

    // Convert to Vec for indexing
    let tokens: Vec<String> = tokens.into_iter().collect();

    if tokens.len() >= 2 {
        // Ensure we have exactly 2 different tokens
        Some((tokens[0].clone(), tokens[1].clone()))
    } else if tokens.len() == 1 {
        // Single token paired with SOL
        Some((tokens[0].clone(), "So11111111111111111111111111111111111111112".to_string()))
    } else {
        warn!("⚠️ Token extraction failed: Found {} unique tokens in account keys", tokens.len());
        None
    }
}

/// Check if address is a known program ID
fn is_program_id(address: &str) -> bool {
    matches!(
        address,
        SPL_TOKEN_PROGRAM_ID
            | RAYDIUM_AMM_V4_PROGRAM_ID
            | RAYDIUM_CPMM_PROGRAM_ID
            | RAYDIUM_LAUNCHLAB_PROGRAM_ID
            | PUMP_FUN_AMM_PROGRAM_ID
            | PUMP_FUN_PROGRAM_ID
            | BONK_FUN_PROGRAM_ID
            | "11111111111111111111111111111111"
            | "ComputeBudget111111111111111111111111111111"
    )
}

/// Heuristic to check if address looks like a token mint
fn is_token_mint(address: &str) -> bool {
    address.len() == 44 && !is_program_id(address)
}

/// Check if a token is a well-known/established token that should be excluded
fn is_well_known_token(mint: &str) -> bool {
    matches!(
        mint,
        WSOL_MINT
            | USDC_MINT
            | USDT_MINT
            | BONK_MINT
            | JITOSOL_MINT
            | MSOL_MINT
            | PYTH_MINT
            | RAY_MINT
            | ORCA_MINT
            | JUP_MINT
            | WIF_MINT
            | POPCAT_MINT
    )
}

/// Validate that this is a new token pairing (not two established tokens)
fn is_valid_new_token_pair(token_a: &str, token_b: &str) -> bool {
    // Both tokens cannot be the same
    if token_a == token_b {
        warn!("⚠️ Rejected: Both tokens are identical ({})", token_a);
        return false;
    }

    // At least one token must be SOL/USDC/USDT (the pair token)
    let has_valid_pair = token_a == WSOL_MINT
        || token_b == WSOL_MINT
        || token_a == USDC_MINT
        || token_b == USDC_MINT
        || token_a == USDT_MINT
        || token_b == USDT_MINT;

    if !has_valid_pair {
        warn!(
            "⚠️ Rejected: No valid pair token (SOL/USDC/USDT) found. Tokens: {} / {}",
            token_a, token_b
        );
        return false;
    }

    // Determine which is the new token and which is the pair
    let (new_token, _pair_token) = if token_a == WSOL_MINT || token_a == USDC_MINT || token_a == USDT_MINT {
        (token_b, token_a)
    } else {
        (token_a, token_b)
    };

    // The new token cannot be a well-known token
    if is_well_known_token(new_token) {
        warn!(
            "⚠️ Rejected: Token {} is a well-known established token",
            new_token
        );
        return false;
    }

    // Valid pair structure (age and platform validation happens later)
    true
}

/// Fetch transaction details
async fn fetch_transaction(
    client: &reqwest::Client,
    rpc_url: &str,
    signature: &str,
) -> Result<serde_json::Value> {
    let request_body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "getTransaction",
        "params": [
            signature,
            {
                "encoding": "jsonParsed",
                "maxSupportedTransactionVersion": 0,
                "commitment": "confirmed"
            }
        ]
    });

    let response = client
        .post(rpc_url)
        .timeout(Duration::from_secs(10))
        .json(&request_body)
        .send()
        .await
        .context("Failed to fetch transaction")?;

    let response_json: serde_json::Value = response
        .json()
        .await
        .context("Failed to parse transaction response")?;

    if let Some(result) = response_json.get("result") {
        Ok(result.clone())
    } else if let Some(error) = response_json.get("error") {
        Err(anyhow::anyhow!("RPC error: {}", error))
    } else {
        Err(anyhow::anyhow!("Unexpected response format"))
    }
}