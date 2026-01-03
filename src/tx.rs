use crate::config::Config;
use crate::presigner::Presigner;
use crate::rate_limiter::RateLimiter;
use anyhow::{Context, Result};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use lazy_static::lazy_static;
use log::{info, warn, error};
use rand::Rng;
use reqwest::Client;
use serde_json::json;
use solana_sdk::{
    compute_budget::ComputeBudgetInstruction,
    instruction::Instruction,
    pubkey::Pubkey,
    system_instruction,
    transaction::{Transaction, VersionedTransaction},
    message::VersionedMessage,
};
use spl_associated_token_account::get_associated_token_address;
use std::str::FromStr;
use std::sync::{Arc, RwLock};
use tokio::sync::OnceCell;

// Jupiter API endpoints (authenticated with API key)
const JUPITER_QUOTE_API: &str = "https://api.jup.ag/swap/v1/quote";
const JUPITER_SWAP_API: &str = "https://api.jup.ag/swap/v1/swap";
pub const SOL_MINT: &str = "So11111111111111111111111111111111111111112";

// Global endpoint (Frankfurt was 429ing)
const JITO_BLOCK_ENGINE_URL: &str = "https://mainnet.block-engine.jito.wtf/api/v1/bundles";

// Default fallback Jito tip accounts (used if API fails)
const DEFAULT_JITO_TIP_ACCOUNTS: [&str; 8] = [
    "96gYZGLnJYVFmbjzopPSU6QiEV5fGqZNyN9nmNhvrZU5",
    "HFqU5x63VTqvQss8hp11i4wVV8bD44PuwqV8Xdn6mwX0",
    "Cw8CFyM9FkoMi7K7Crf6HNQqf4uEMzpKw6QNghXLvLkY",
    "ADaUMid9yfUytqMBgopDjb6u78m9rDbq49Jg8jJspr1",
    "DfXygSm4jCyNCybVYYK6DpnEN5qu7xiAaROf8CJxv2r",
    "ADuUkR4ykGytmZ5x5BPdGu8h4r67hQ8ea7ooM1c3kP47",
    "DttWaMuVvTiduZRnguLF7jNxTgiMBZ1hyAumKUiL2KRL",
    "3AVi9Tg9Uo68tJfuvoKvqKNWKkC5wPdSSdeBnIzKZ6jJ",
];

lazy_static! {
    /// Cached Jito tip accounts - dynamically refreshed from API
    static ref JITO_TIP_ACCOUNTS_CACHE: RwLock<Vec<String>> = RwLock::new(
        DEFAULT_JITO_TIP_ACCOUNTS.iter().map(|s| s.to_string()).collect()
    );
    
    /// Track when we last refreshed the tip accounts
    static ref LAST_TIP_REFRESH: RwLock<Option<std::time::Instant>> = RwLock::new(None);
}

/// Refresh Jito tip accounts from API (call periodically)

pub async fn refresh_jito_tip_accounts() -> Result<()> {
    let client = Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()?;
    
    // Use JSON-RPC method getTipAccounts
    let request = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "getTipAccounts",
        "params": []
    });

    // Note: We use the generic bundles endpoint for this RPC call
    match client.post(JITO_BLOCK_ENGINE_URL) // Reusing the valid Frankfurt/Global URL
        .json(&request)
        .send()
        .await 
    {
        Ok(response) => {
            let response_json: serde_json::Value = response.json().await?;
            
            if let Some(result) = response_json.get("result") {
                if let Some(accounts) = result.as_array() {
                     let valid_accounts: Vec<String> = accounts
                        .iter()
                        .filter_map(|v| v.as_str())
                        .map(|s| s.to_string())
                        .filter(|acc| {
                            match Pubkey::from_str(acc) {
                                Ok(_) => true,
                                Err(_) => {
                                    warn!("⚠️ Invalid tip account from API (not valid Base58): {}", acc);
                                    false
                                }
                            }
                        })
                        .collect();

                    if !valid_accounts.is_empty() {
                        if let Ok(mut cache) = JITO_TIP_ACCOUNTS_CACHE.write() {
                            *cache = valid_accounts.clone();
                            info!("✅ Refreshed Jito tip accounts: {} valid accounts", cache.len());
                            crate::trade_logger::log_jito_debug("REFRESH", &format!("Updated cache with {} accounts", cache.len()));
                        }
                        if let Ok(mut last_refresh) = LAST_TIP_REFRESH.write() {
                            *last_refresh = Some(std::time::Instant::now());
                        }
                    } else {
                        warn!("⚠️ No valid tip accounts from API. Using cached/default.");
                    }
                }
            } else {
                warn!("⚠️ Jito getTipAccounts response missing 'result': {:?}", response_json);
            }
        }
        Err(e) => {
            warn!("⚠️ Failed to refresh Jito tip accounts: {}. Using cached/default.", e);
        }
    }
    
    Ok(())
}


/// Get a random Jito tip account from cache with validation
fn get_random_tip_account() -> String {
    let accounts = JITO_TIP_ACCOUNTS_CACHE.read().unwrap();
    
    // Try up to 5 times to get a valid account
    for _ in 0..5 {
        let idx = rand::rng().random_range(0..accounts.len());
        let candidate = accounts[idx].trim().to_string(); // Trim whitespace
        
        // Validate before returning
        if Pubkey::from_str(&candidate).is_ok() {
            return candidate;
        } else {
             warn!("⚠️ Jito Cache contained invalid Base58: '{}'", candidate);
        }
    }
    
    // Fallback if cache is corrupted
    warn!("⚠️ Failed to find valid tip account in cache after 5 attempts. Using hardcoded fallback.");
    "96gYZGLnJYVFmbjzopPSU6QiEV5fGqZNyN9nmNhvrZU5".to_string()
}

/// DEX Router for swap instructions
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DexRouter {
    PumpSwap,  // Fastest for immediate post-graduation
    Raydium,   // Main router for buys/sells
    Jupiter,   // Multi-pool routing for later trades
}

/// Transaction Manager - Pure Builder (NO SIGNING)
#[derive(Clone)]
pub struct TransactionManager {
    client: Client,
    payer_pubkey: Pubkey,
    jupiter_limiter: Option<Arc<RateLimiter>>,
    jupiter_api_key: String,
    transaction_mode: crate::config::TransactionMode,
    priority_fee_lamports: u64,
    compute_unit_limit: u32,
    base_tip_lamports: u64,
    min_tip_lamports: u64,
    max_tip_lamports: u64,
    dynamic_tips_enabled: bool,
}

impl TransactionManager {
    pub fn new(presigner: Arc<Presigner>, config: &Config) -> Self {
        let jupiter_limiter = if config.rate_limiting_enabled {
            Some(Arc::new(RateLimiter::new(
                config.jupiter_max_requests_per_second,
                "Jupiter",
            )))
        } else {
            None
        };

        let client = Client::builder()
            .timeout(std::time::Duration::from_millis(config.jupiter_api_timeout_ms))
            .build()
            .unwrap_or_else(|_| Client::new());

        Self {
            client,
            payer_pubkey: presigner.pubkey(),
            jupiter_limiter,
            jupiter_api_key: config.jupiter_api_key.clone(),
            transaction_mode: config.transaction_mode,
            priority_fee_lamports: config.priority_fee_lamports,
            compute_unit_limit: config.compute_unit_limit,
            base_tip_lamports: config.jito_base_tip_lamports,
            min_tip_lamports: config.jito_min_tip_lamports,
            max_tip_lamports: config.jito_max_tip_lamports,
            dynamic_tips_enabled: config.jito_dynamic_tips_enabled,
        }
    }

    /// Build BUY transaction (VersionedTransaction)
    /// Returns: VersionedTransaction ready for signing
    pub async fn build_buy_transaction(
        &self,
        mint: &str,
        amount_sol_lamports: u64,
        slippage_bps: u16,
        router: DexRouter,
        _tip_lamports: u64, // Tip is now handled via Jito Bundle (separate tx)
    ) -> Result<(VersionedTransaction, u64)> {
        info!("🔨 Building BUY transaction for {} via {:?}", mint, router);

        // 1. Get swap transaction from router (Jupiter)
        // We request a VersionedTransaction directly
        let versioned_tx = self.get_swap_transaction(
            SOL_MINT,
            mint,
            amount_sol_lamports,
            slippage_bps,
            router,
        ).await?;

        info!("✅ Built BUY transaction");
        Ok(versioned_tx)
    }

    /// Build Jito Tip Transaction (Separate Transaction)
    pub fn build_tip_transaction(&self, tip_lamports: u64, _recent_blockhash: solana_sdk::hash::Hash) -> Result<VersionedTransaction> {
        crate::trade_logger::log_jito_debug("BUILD_TIP", &format!("Creating tip tx for {} lamports ({:.6} SOL)", tip_lamports, tip_lamports as f64 / 1e9));
        let tip_ix = self.create_tip_instruction(tip_lamports)?;
        let msg = solana_sdk::message::Message::new(&[tip_ix], Some(&self.payer_pubkey));
        let versioned_msg = VersionedMessage::Legacy(msg);
        Ok(VersionedTransaction {
            signatures: vec![],
            message: versioned_msg,
        })
    }

    /// Build SELL transaction (VersionedTransaction)
    /// Returns: VersionedTransaction ready for signing
    pub async fn build_sell_transaction(
        &self,
        mint: &str,
        amount_token_raw: u64,
        slippage_bps: u16,
        router: DexRouter,
        _close_ata: bool, // Jupiter usually handles this, or we need a separate cleanup tx
    ) -> Result<VersionedTransaction> {
        info!("🔨 Building SELL transaction for {} via {:?}", mint, router);

        // 1. Get swap transaction from router
        let (versioned_tx, _expected_sol) = self.get_swap_transaction(
            mint,
            SOL_MINT,
            amount_token_raw,
            slippage_bps,
            router,
        ).await?;

        info!("✅ Built SELL transaction");
        Ok(versioned_tx)
    }

    async fn get_swap_transaction(
        &self,
        input_mint: &str,
        output_mint: &str,
        amount: u64,
        slippage_bps: u16,
        router: DexRouter,
    ) -> Result<(VersionedTransaction, u64)> {
        match router {
            DexRouter::PumpSwap => {
                warn!("⚠️ PumpSwap routing not yet implemented, falling back to Jupiter");
                self.get_jupiter_swap_transaction(input_mint, output_mint, amount, slippage_bps).await
            }
            DexRouter::Raydium => {
                warn!("⚠️ Raydium routing not yet implemented, falling back to Jupiter");
                self.get_jupiter_swap_transaction(input_mint, output_mint, amount, slippage_bps).await
            }
            DexRouter::Jupiter => {
                self.get_jupiter_swap_transaction(input_mint, output_mint, amount, slippage_bps).await
            }
        }
    }

    /// Get Jupiter swap transaction (VersionedTransaction)
    async fn get_jupiter_swap_transaction(
        &self,
        input_mint: &str,
        output_mint: &str,
        amount: u64,
        slippage_bps: u16,
    ) -> Result<(VersionedTransaction, u64)> {
        // 1. Get quote
        let quote = self.get_jupiter_quote(input_mint, output_mint, amount, slippage_bps).await?;

        // 2. Get swap transaction  
        let mut request = json!({
            "quoteResponse": quote,
            "userPublicKey": self.payer_pubkey.to_string(),
            "wrapAndUnwrapSol": true,
            // "asLegacyTransaction": true // REMOVED: We want Versioned Tx
        });

        // If in Standard mode, we can ask Jupiter to bake in the priority fee
        if self.transaction_mode == crate::config::TransactionMode::Standard {
            request["prioritizationFeeLamports"] = json!(self.priority_fee_lamports);
            // Optionally set compute unit limit if needed, though Jupiter usually tunes this well
            // request["computeUnitLimit"] = json!(self.compute_unit_limit);
        }

        let response: serde_json::Value = self.client.post(JUPITER_SWAP_API)
            .header("x-api-key", &self.jupiter_api_key)
            .json(&request)
            .send().await?
            .json().await?;

        // Check if there's an error in the response
        if let Some(error) = response.get("error") {
            error!("❌ Jupiter Swap API Error: {}", error);
            return Err(anyhow::anyhow!("Jupiter API error: {}", error));
        }

        let swap_tx_base64 = response.get("swapTransaction")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                // Log the full response to debug
                error!("❌ Jupiter response missing swapTransaction. Full response: {}", 
                    serde_json::to_string_pretty(&response).unwrap_or_else(|_| "Unable to serialize".to_string()));
                anyhow::anyhow!("No swapTransaction in Jupiter response")
            })?;

        let tx_bytes = BASE64_STANDARD.decode(swap_tx_base64)?;
        
        // Deserialize as VersionedTransaction
        let versioned_tx: VersionedTransaction = bincode::deserialize(&tx_bytes)
            .context("Failed to deserialize Jupiter VersionedTransaction")?;
            
        // Extract expected output amount from quote for optimistic monitoring
        let expected_out_amount = quote.get("outAmount")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0);
            
        Ok((versioned_tx, expected_out_amount))
    }

    pub async fn get_jupiter_quote(
        &self,
        input_mint: &str,
        output_mint: &str,
        amount: u64,
        slippage_bps: u16,
    ) -> Result<serde_json::Value> {
        let url = format!(
            "{}?inputMint={}&outputMint={}&amount={}&slippageBps={}",
            JUPITER_QUOTE_API, input_mint, output_mint, amount, slippage_bps
        );

        let mut last_err = None;
        for attempt in 1..=3 {
            // Acquire rate limit token
            if let Some(limiter) = &self.jupiter_limiter {
                limiter.acquire().await;
            }

            match self.client
                .get(&url)
                .header("x-api-key", &self.jupiter_api_key)
                .send()
                .await
            {
                Ok(resp) => {
                    let json: serde_json::Value = resp.json().await?;
                    
                    // Check if Jupiter returned an error JSON instead of a quote
                    if let Some(err_msg) = json.get("error").or_else(|| json.get("message")) {
                         warn!("⚠️ Jupiter Quote API returned error: {}. Raw response: {}", err_msg, json);
                         return Err(anyhow::anyhow!("Jupiter Quote Error: {}", err_msg));
                    }
                    
                    // Validate that it looks like a real quote (has inputMint or other core fields)
                    if json.get("inputMint").is_none() {
                        error!("❌ Jupiter Quote response missing 'inputMint'. Raw: {}", json);
                        return Err(anyhow::anyhow!("Jupiter Quote response malformed: missing inputMint"));
                    }

                    return Ok(json);
                }
                Err(e) => {
                    warn!("⚠️ Jupiter Quote Attempt {} failed: {}. Retrying...", attempt, e);
                    last_err = Some(e);
                    tokio::time::sleep(std::time::Duration::from_millis(500 * attempt as u64)).await;
                }
            }
        }

        Err(anyhow::anyhow!("Jupiter Quote failed after 3 attempts: {:?}", last_err))
    }

    /// Create Jito tip instruction (single instruction, not separate tx)
    /// Uses dynamically refreshed tip accounts when available
    fn create_tip_instruction(&self, tip_lamports: u64) -> Result<Instruction> {
        // Get random tip account from dynamically refreshed cache
        let tip_account_str = get_random_tip_account();
        let tip_account = Pubkey::from_str(&tip_account_str)?;

        Ok(system_instruction::transfer(
            &self.payer_pubkey,
            &tip_account,
            tip_lamports,
        ))
    }

    /// Calculate dynamic Jito tip based on priority
    pub fn calculate_tip(&self, high_priority: bool) -> u64 {
        if !self.dynamic_tips_enabled {
            return self.base_tip_lamports;
        }

        let multiplier = if high_priority { 2.0 } else { 1.0 };
        let tip = (self.base_tip_lamports as f64 * multiplier) as u64;
        
        // Enforce hard minimum of 0.0005 SOL (500k lamports) for testing/mainnet viability
        let hard_min = 500_000;
        let effective_tip = tip.clamp(self.min_tip_lamports.max(hard_min), self.max_tip_lamports);
        
        effective_tip
    }

    pub fn transaction_mode(&self) -> crate::config::TransactionMode {
        self.transaction_mode
    }
}
