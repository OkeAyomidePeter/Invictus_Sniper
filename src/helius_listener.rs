use crate::config::Config;
use anyhow::{Context, Result};
use futures_util::{SinkExt, StreamExt};
use log::{debug, error, info, warn};
use serde::Deserialize;
use serde_json::json;
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tokio::time::sleep;
use tokio_tungstenite::{connect_async, tungstenite::Message};

/// Raw transaction event from Helius
#[derive(Debug, Clone)]
pub struct RawTxEvent {
    pub signature: String,
    pub slot: u64,
    pub timestamp: Option<i64>,
    pub data: serde_json::Value,
}

/// Classified event types from the pipeline (SIMPLIFIED - Only Mint and Pool Creation)
#[derive(Debug, Clone)]
pub enum ClassifiedEvent {
    Mint(MintEvent),
    PoolCreation(PoolCreationEvent),
}

impl ClassifiedEvent {
    pub fn signature(&self) -> &str {
        match self {
            ClassifiedEvent::Mint(e) => &e.signature,
            ClassifiedEvent::PoolCreation(e) => &e.signature,
        }
    }
}

/// Mint event - covers ALL platforms (Raydium Launchlab, Pump.fun, SPL Token, etc.)
#[derive(Debug, Clone)]
pub struct MintEvent {
    pub mint: String,
    pub signature: String,
    pub slot: u64,
    pub timestamp: Option<i64>,
    pub platform: String, // "SPL Token", "Raydium Launchlab", "Pump.fun", etc.
    pub decimals: Option<u8>,
    pub supply: Option<u64>,
}

/// Pool creation event - covers ALL DEXes (Jupiter, Orca, Raydium AMM, CPMM, etc.)
#[derive(Debug, Clone)]
pub struct PoolCreationEvent {
    pub pool_address: String,
    pub token_mint: String, // The new token being listed
    pub pair_token: String, // Usually SOL or USDC
    pub signature: String,
    pub slot: u64,
    pub timestamp: Option<i64>,
    pub dex: String, // "Raydium AMM v4", "Raydium CPMM", "Orca Whirlpool", "Pump.fun", etc.
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

// ========== REAL PROGRAM IDs (Verified on Solana Mainnet) ==========

/// SPL Token program ID
const SPL_TOKEN_PROGRAM_ID: &str = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";

/// Token-2022 program ID
const TOKEN_2022_PROGRAM_ID: &str = "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb";

/// Metaplex Token Metadata program ID
const METAPLEX_TOKEN_METADATA_PROGRAM_ID: &str = "metaqbxxUerdq28cj1RbAWkYQm3ybzjb6a8bt518x1s";

/// Raydium AMM v4 program ID (Most popular Raydium pools)
const RAYDIUM_AMM_V4_PROGRAM_ID: &str = "675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8";

/// Raydium CPMM program ID (Concentrated liquidity)
const RAYDIUM_CPMM_PROGRAM_ID: &str = "CPMMoo8L3F4NbTegBCKVNunggL7H1ZpdTHKxQB5qKP1C";

/// Raydium Liquidity Pool V4 (Another variant)
const RAYDIUM_LIQUIDITY_POOL_V4: &str = "RVKd61ztZW9GUwhRbbLoYVRE5Xf1B2tVscKqwZqXgEr";

/// Pump.fun program ID (Bonding curve token launch platform)
const PUMP_FUN_PROGRAM_ID: &str = "6EF8rrecthR5Dkzon8Nwi3bTW1w4Q5PgdHzCfyqXYUVh";

/// Orca Whirlpool program ID (Concentrated liquidity AMM)
const ORCA_WHIRLPOOL_PROGRAM_ID: &str = "whirLbMiicVdio4qvUfM5KAg6Ct8VwpYzGff3uctyCc";

/// Jupiter Aggregator v6 program ID
const JUPITER_V6_PROGRAM_ID: &str = "JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4";

/// Meteora DLMM (Dynamic Liquidity Market Maker)
const METEORA_DLMM_PROGRAM_ID: &str = "LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo";

/// Phoenix DEX program ID
const PHOENIX_PROGRAM_ID: &str = "PhoeNiXZ8ByJGLkxNfZRnkUfjvmuYqLR89jjFHGqdXY";

/// Start the Helius listener with event pipeline (MAINNET ONLY, WebSocket ONLY)
pub async fn start(config: &Config) -> Result<mpsc::Receiver<ClassifiedEvent>> {
    let (raw_tx, raw_rx) = mpsc::channel::<RawTxEvent>(1000);
    let (classified_tx, classified_rx) = mpsc::channel::<ClassifiedEvent>(1000);

    let api_key = config.helius_api_key.clone();

    info!("🚀 Starting Helius listener (MAINNET ONLY, WebSocket ONLY)");

    // Start the websocket listener
    tokio::spawn(async move {
        listener_loop(api_key, raw_tx).await;
    });

    // Start the event classification pipeline
    tokio::spawn(async move {
        if let Err(e) = event_pipeline(raw_rx, classified_tx).await {
            error!("Event pipeline error: {}", e);
        }
    });

    Ok(classified_rx)
}

async fn listener_loop(api_key: String, tx: mpsc::Sender<RawTxEvent>) {
    let mut backoff_seconds = 1;
    let max_backoff = 60;

    info!("Listener starting (MAINNET WebSocket only)");

    loop {
        info!("Connecting to Helius WebSocket (mainnet)");

        match connect_and_listen(&api_key, &tx).await {
            Ok(()) => {
                info!("WebSocket connection closed normally");
                backoff_seconds = 1;
            }
            Err(e) => {
                error!(
                    "WebSocket error: {}. Reconnecting in {}s...",
                    e, backoff_seconds
                );
                sleep(Duration::from_secs(backoff_seconds)).await;
                backoff_seconds = std::cmp::min(backoff_seconds * 2, max_backoff);
            }
        }
    }
}

async fn connect_and_listen(
    api_key: &str,
    tx: &mpsc::Sender<RawTxEvent>,
) -> Result<()> {
    // MAINNET ONLY - Helius WebSocket endpoint
    let ws_url = format!("wss://mainnet.helius-rpc.com/?api-key={}", api_key);

    info!(
        "Connecting to Helius WebSocket: {}",
        ws_url.replace(api_key, "***")
    );

    let (ws_stream, _) = connect_async(&ws_url)
        .await
        .context("Failed to connect to Helius WebSocket")?;

    info!("✅ Connected to Helius WebSocket (mainnet)");

    let (mut write, mut read) = ws_stream.split();

    // ========== SUBSCRIBE TO MINT EVENTS (All Platforms) ==========
    // We use logsSubscribe because it catches mint events from ALL platforms
    // including Pump.fun bonding curves, Raydium Launchlab, and standard SPL mints

    let mint_programs = vec![
        (1, SPL_TOKEN_PROGRAM_ID, "SPL Token"),
        (2, TOKEN_2022_PROGRAM_ID, "Token 2022"),
        (3, METAPLEX_TOKEN_METADATA_PROGRAM_ID, "Metaplex Metadata"),
        (4, PUMP_FUN_PROGRAM_ID, "Pump.fun"),
    ];

    for (id, program_id, name) in mint_programs {
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
        info!("📡 Subscribed to mint events for {}", name);
    }

    // ========== SUBSCRIBE TO POOL CREATION EVENTS (All DEXes) ==========

    let dex_programs = vec![
        (10, RAYDIUM_AMM_V4_PROGRAM_ID, "Raydium AMM v4"),
        (11, RAYDIUM_CPMM_PROGRAM_ID, "Raydium CPMM"),
        (12, RAYDIUM_LIQUIDITY_POOL_V4, "Raydium Liquidity Pool v4"),
        (13, ORCA_WHIRLPOOL_PROGRAM_ID, "Orca Whirlpool"),
        (14, JUPITER_V6_PROGRAM_ID, "Jupiter v6"),
        (15, METEORA_DLMM_PROGRAM_ID, "Meteora DLMM"),
        (16, PHOENIX_PROGRAM_ID, "Phoenix DEX"),
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
        info!("📡 Subscribed to pool creation events for {}", name);
    }

    // Listen for messages
    while let Some(msg) = read.next().await {
        match msg {
            Ok(Message::Text(text)) => {
                if let Err(e) = handle_message(&text, api_key, tx).await {
                    warn!("Error handling message: {}", e);
                }
            }
            Ok(Message::Close(_)) => {
                info!("WebSocket closed by server");
                break;
            }
            Ok(Message::Ping(data)) => {
                // Respond to ping to keep connection alive
                let _ = write.send(Message::Pong(data)).await;
            }
            Ok(_) => {
                // Ignore other message types
            }
            Err(e) => {
                return Err(anyhow::anyhow!("WebSocket error: {}", e));
            }
        }
    }

    Ok(())
}

async fn handle_message(
    text: &str,
    api_key: &str,
    tx: &mpsc::Sender<RawTxEvent>,
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

            // Quick filter: Check if logs contain keywords for mint or pool creation
            let is_relevant = logs.iter().any(|log| {
                log.contains("InitializeMint") ||
                log.contains("initialize") ||
                log.contains("create") ||
                log.contains("CreatePool") ||
                log.contains("Initialize2") ||
                log.contains("InitializePool")
            });

            if !is_relevant {
                // Skip transactions that don't contain relevant keywords
                return Ok(());
            }

            info!("🔍 Relevant transaction detected: {}", signature);

            // Fetch the full transaction
            let client = reqwest::Client::new();
            let rpc_url = format!("https://mainnet.helius-rpc.com/?api-key={}", api_key);
            
            match fetch_transaction(&client, &rpc_url, &signature).await {
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
                        warn!("Failed to send event to channel (receiver dropped)");
                        return Err(anyhow::anyhow!("Channel receiver dropped"));
                    }
                    info!("✅ Processed transaction: {}", signature);
                }
                Err(e) => {
                    warn!("Failed to fetch transaction {}: {}", signature, e);
                }
            }
        }
    }

    Ok(())
}

/// Event classification pipeline with debouncing
async fn event_pipeline(
    mut raw_rx: mpsc::Receiver<RawTxEvent>,
    classified_tx: mpsc::Sender<ClassifiedEvent>,
) -> Result<()> {
    let mut event_buffer: HashMap<String, RawTxEvent> = HashMap::new();
    let mut last_process_time = Instant::now();
    let debounce_duration = Duration::from_millis(300);

    info!("🎯 Event classification pipeline started (MINT + POOL CREATION ONLY)");

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

/// Process a batch of buffered events and classify them
async fn process_event_batch(
    event_buffer: &mut HashMap<String, RawTxEvent>,
    classified_tx: &mpsc::Sender<ClassifiedEvent>,
) {
    let events: Vec<_> = event_buffer.drain().collect();

    info!("📦 Processing batch of {} events", events.len());

    for (_key, event) in events {
        if let Some(classified_event) = classify_event(&event) {
            let event_type = match &classified_event {
                ClassifiedEvent::Mint(_) => "🪙 MINT",
                ClassifiedEvent::PoolCreation(_) => "🏊 POOL CREATION",
            };
            info!("{} event detected: {}", event_type, event.signature);
            
            // ========== LOG FULL EVENT DATA BEFORE SENDING ==========
            info!("📤 SENDING CLASSIFIED EVENT: {:#?}", classified_event);
            // ========================================================
            
            if classified_tx.send(classified_event).await.is_err() {
                warn!("Failed to send classified event (receiver dropped)");
                break;
            }
        }
    }
}

/// Classify a single event (MINT or POOL CREATION only)
fn classify_event(event: &RawTxEvent) -> Option<ClassifiedEvent> {
    let transaction = event.data.get("transaction")?;
    let logs = event.data.get("logs")?;
    let meta = event.data.get("meta");

    // Check logs for mint events
    if let Some(mint_event) = detect_mint_event(event, transaction, logs, meta) {
        return Some(ClassifiedEvent::Mint(mint_event));
    }

    // Check logs for pool creation events
    if let Some(pool_event) = detect_pool_creation_event(event, transaction, logs, meta) {
        return Some(ClassifiedEvent::PoolCreation(pool_event));
    }

    None
}

/// Detect mint events from transaction logs (ALL PLATFORMS)
fn detect_mint_event(
    event: &RawTxEvent,
    transaction: &serde_json::Value,
    logs: &serde_json::Value,
    meta: Option<&serde_json::Value>,
) -> Option<MintEvent> {
    let logs_array = logs.as_array()?;
    
    // First, check if this involves Pump.fun by looking at mint address suffix
    let mint_address = extract_mint_address(transaction, meta)?;
    
    // Pump.fun tokens end with "pump" - this is the most reliable detection
    let platform = if mint_address.ends_with("pump") {
        "Pump.fun".to_string()
    } else {
        // Check logs for platform-specific program IDs
        determine_mint_platform(logs_array).unwrap_or("SPL Token".to_string())
    };
    
    // Check for SPL Token InitializeMint
    for log in logs_array {
        let log_str = log.as_str()?;
        
        if log_str.contains("InitializeMint") || log_str.contains("Initialize2") {
            let decimals = extract_decimals(transaction, meta);
            
            return Some(MintEvent {
                mint: mint_address,
                signature: event.signature.clone(),
                slot: event.slot,
                timestamp: event.timestamp,
                platform,
                decimals,
                supply: extract_supply(meta),
            });
        }
    }
    
    None
}

/// Detect pool creation events from transaction logs (ALL DEXes)
fn detect_pool_creation_event(
    event: &RawTxEvent,
    transaction: &serde_json::Value,
    logs: &serde_json::Value,
    meta: Option<&serde_json::Value>,
) -> Option<PoolCreationEvent> {
    let logs_array = logs.as_array()?;
    
    // Pool creation keywords by DEX
    let pool_keywords = [
        ("initialize", "Raydium"),
        ("InitializePool", "Raydium"),
        ("create", "Orca"),
        ("CreatePool", "Orca"),
        ("initialize_pool", "Meteora"),
        ("create_pool", "Phoenix"),
    ];
    
    for log in logs_array {
        let log_str = log.as_str()?;
        
        for (keyword, dex) in &pool_keywords {
            if log_str.contains(keyword) {
                // Extract pool details
                let pool_address = extract_pool_address(transaction, meta, log_str)?;
                let (token_mint, pair_token) = extract_pool_tokens(transaction, meta)?;
                
                return Some(PoolCreationEvent {
                    pool_address,
                    token_mint,
                    pair_token,
                    signature: event.signature.clone(),
                    slot: event.slot,
                    timestamp: event.timestamp,
                    dex: dex.to_string(),
                });
            }
        }
    }
    
    None
}

/// Extract mint address from transaction
fn extract_mint_address(
    transaction: &serde_json::Value,
    meta: Option<&serde_json::Value>,
) -> Option<String> {
    // Try to get from postTokenBalances (most reliable)
    if let Some(meta) = meta {
        if let Some(post_balances) = meta.get("postTokenBalances").and_then(|b| b.as_array()) {
            if let Some(first) = post_balances.first() {
                if let Some(mint) = first.get("mint").and_then(|m| m.as_str()) {
                    return Some(mint.to_string());
                }
            }
        }
    }
    
    // Fallback: parse from account keys
    let message = transaction.get("message")?;
    let account_keys = message.get("accountKeys")?.as_array()?;
    
    // Usually mint is the first account after program IDs
    for key in account_keys {
        if let Some(pubkey) = key.get("pubkey").and_then(|p| p.as_str()) {
            // Skip known program IDs
            if !is_program_id(pubkey) {
                return Some(pubkey.to_string());
            }
        } else if let Some(pubkey_str) = key.as_str() {
            if !is_program_id(pubkey_str) {
                return Some(pubkey_str.to_string());
            }
        }
    }
    
    None
}

/// Extract decimals from transaction
fn extract_decimals(
    transaction: &serde_json::Value,
    meta: Option<&serde_json::Value>,
) -> Option<u8> {
    if let Some(meta) = meta {
        if let Some(post_balances) = meta.get("postTokenBalances").and_then(|b| b.as_array()) {
            if let Some(first) = post_balances.first() {
                if let Some(decimals) = first.get("uiTokenAmount")
                    .and_then(|u| u.get("decimals"))
                    .and_then(|d| d.as_u64()) {
                    return Some(decimals as u8);
                }
            }
        }
    }
    None
}

/// Extract supply from metadata
fn extract_supply(meta: Option<&serde_json::Value>) -> Option<u64> {
    if let Some(meta) = meta {
        if let Some(post_balances) = meta.get("postTokenBalances").and_then(|b| b.as_array()) {
            if let Some(first) = post_balances.first() {
                if let Some(amount) = first.get("uiTokenAmount")
                    .and_then(|u| u.get("amount"))
                    .and_then(|a| a.as_str())
                    .and_then(|s| s.parse::<u64>().ok()) {
                    return Some(amount);
                }
            }
        }
    }
    None
}

/// Determine mint platform from logs and mint address
fn determine_mint_platform(logs: &[serde_json::Value]) -> Option<String> {
    for log in logs {
        if let Some(log_str) = log.as_str() {
            // Check for Pump.fun program ID in logs
            if log_str.contains("6EF8rrecthR5Dkzon8Nwi3bTW1w4Q5PgdHzCfyqXYUVh") 
                || log_str.contains(PUMP_FUN_PROGRAM_ID) {
                return Some("Pump.fun".to_string());
            }
            // Check for Raydium program IDs
            if log_str.contains("675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8") 
                || log_str.contains(RAYDIUM_AMM_V4_PROGRAM_ID)
                || log_str.contains(RAYDIUM_CPMM_PROGRAM_ID) {
                return Some("Raydium Launchlab".to_string());
            }
        }
    }
    Some("SPL Token".to_string())
}

/// Extract pool address from transaction
fn extract_pool_address(
    transaction: &serde_json::Value,
    meta: Option<&serde_json::Value>,
    log: &str,
) -> Option<String> {
    // Try to parse from logs first
    if let Some(addr_start) = log.find("pool: ") {
        let addr = &log[addr_start + 6..];
        if let Some(space_idx) = addr.find(' ') {
            return Some(addr[..space_idx].to_string());
        }
    }
    
    // Fallback: get from account keys
    let message = transaction.get("message")?;
    let account_keys = message.get("accountKeys")?.as_array()?;
    
    // Pool address is usually one of the writable accounts
    if let Some(meta) = meta {
        // Check for newly created accounts (pool accounts)
        // This is a heuristic - adjust based on actual transaction structure
    }
    
    // Return first non-program account as pool address
    for key in account_keys {
        if let Some(pubkey) = key.get("pubkey").and_then(|p| p.as_str()) {
            if !is_program_id(pubkey) {
                return Some(pubkey.to_string());
            }
        }
    }
    
    None
}

/// Extract token pair from pool creation
fn extract_pool_tokens(
    transaction: &serde_json::Value,
    meta: Option<&serde_json::Value>,
) -> Option<(String, String)> {
    if let Some(meta) = meta {
        if let Some(post_balances) = meta.get("postTokenBalances").and_then(|b| b.as_array()) {
            if post_balances.len() >= 2 {
                let token_a = post_balances[0].get("mint")?.as_str()?;
                let token_b = post_balances[1].get("mint")?.as_str()?;
                return Some((token_a.to_string(), token_b.to_string()));
            }
        }
    }
    
    // Fallback: assume SOL pair
    let message = transaction.get("message")?;
    let account_keys = message.get("accountKeys")?.as_array()?;
    
    let mut tokens = Vec::new();
    for key in account_keys {
        if let Some(pubkey) = key.get("pubkey").and_then(|p| p.as_str()) {
            if !is_program_id(pubkey) {
                tokens.push(pubkey.to_string());
            }
        }
    }
    
    if tokens.len() >= 2 {
        Some((tokens[0].clone(), tokens[1].clone()))
    } else if tokens.len() == 1 {
        Some((tokens[0].clone(), "So11111111111111111111111111111111111111112".to_string()))
    } else {
        None
    }
}

/// Check if address is a known program ID
fn is_program_id(address: &str) -> bool {
    matches!(
        address,
        SPL_TOKEN_PROGRAM_ID
            | TOKEN_2022_PROGRAM_ID
            | METAPLEX_TOKEN_METADATA_PROGRAM_ID
            | RAYDIUM_AMM_V4_PROGRAM_ID
            | RAYDIUM_CPMM_PROGRAM_ID
            | RAYDIUM_LIQUIDITY_POOL_V4
            | PUMP_FUN_PROGRAM_ID
            | ORCA_WHIRLPOOL_PROGRAM_ID
            | JUPITER_V6_PROGRAM_ID
            | METEORA_DLMM_PROGRAM_ID
            | PHOENIX_PROGRAM_ID
            | "11111111111111111111111111111111" // System program
            | "ComputeBudget111111111111111111111111111111" // Compute budget
    )
}

/// Fetch full transaction details for a signature
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