use crate::config::Config;
use crate::presigner::Presigner;
use crate::rate_limiter::RateLimiter;
use crate::retry::{retry_with_backoff, is_network_error, RetryConfig};
use anyhow::{Context, Result};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use log::{info, warn};
use rand::Rng;
use reqwest::Client;
use serde_json::json;
use solana_sdk::{
    pubkey::Pubkey,
    system_instruction,
    transaction::VersionedTransaction,
};
use std::collections::VecDeque;
use std::str::FromStr;
use std::sync::Arc;
use tokio::sync::Mutex;

const JUPITER_QUOTE_API: &str = "https://quote-api.jup.ag/v6/quote";
const JUPITER_SWAP_API: &str = "https://quote-api.jup.ag/v6/swap";
const JITO_BLOCK_ENGINE_URL: &str = "https://mainnet.block-engine.jito.wtf/api/v1/bundles";
const SOL_MINT: &str = "So11111111111111111111111111111111111111112";

// Jito Tip Accounts
const JITO_TIP_ACCOUNTS: [&str; 8] = [
    "96gYZGLnJYVFmbjzopPSU6QiEV5fGqZNyN9nmNhvrZU5",
    "HFqU5x63VTqvQss8hp11i4wVV8bD44PuwqV8Xdn6mwX0",
    "Cw8CFyM9FkoMi7K7Crf6HNQqf4uEMzpKw6QNghXLvLkY",
    "ADaUMid9yfUytqMBgopDjb6u78m9rDbq49Jg8jJspr1",
    "DfXygSm4jCyNCybVYYK6DpnEN5qu7xiAaROf8CJxv2r",
    "ADuUkR4ykGytmZ5x5BPdGu8h4r67hQ8ea7ooM1c3kP47",
    "DttWaMuVvTiduZRnguLF7jNxTgiMBZ1hyAumKUiL2KRL",
    "3AVi9Tg9Uo68tJfuvoKvqKNWKkC5wPdSSdeBnIzKZ6jJ",
];

/// Trade priority for dynamic tip calculation
#[derive(Debug, Clone, Copy)]
pub enum TradePriority {
    High,
    Medium,
    Low,
}

#[derive(Clone)]
pub struct TransactionManager {
    client: Client,
    presigner: Arc<Presigner>,
    jupiter_limiter: Option<Arc<RateLimiter>>,
    retry_config: RetryConfig,
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

        let retry_config = RetryConfig {
            max_attempts: config.tx_retry_max_attempts,
            initial_delay_ms: config.tx_retry_initial_delay_ms,
            max_delay_ms: config.tx_retry_max_delay_ms,
            backoff_multiplier: config.tx_retry_backoff_multiplier,
        };

        Self {
            client: Client::new(),
            presigner,
            jupiter_limiter,
            retry_config,
            base_tip_lamports: config.jito_base_tip_lamports,
            min_tip_lamports: config.jito_min_tip_lamports,
            max_tip_lamports: config.jito_max_tip_lamports,
            dynamic_tips_enabled: config.jito_dynamic_tips_enabled,
        }
    }

    /// Calculate dynamic Jito tip based on priority
    fn calculate_tip(&self, priority: TradePriority) -> u64 {
        if !self.dynamic_tips_enabled {
            return self.base_tip_lamports;
        }

        let priority_multiplier = match priority {
            TradePriority::High => 2.0,
            TradePriority::Medium => 1.5,
            TradePriority::Low => 1.0,
        };

        let tip = (self.base_tip_lamports as f64 * priority_multiplier) as u64;
        tip.clamp(self.min_tip_lamports, self.max_tip_lamports)
    }

    /// Execute a BUY order using Jito Bundle
    pub async fn buy_with_jito(
        &self,
        mint: &str,
        amount_sol_lamports: u64,
        tip_lamports: u64,
        slippage_bps: u16,
    ) -> Result<String> {
        info!("⚡ Preparing Jito BUY for {} (Amt: {} lamports, Tip: {})", mint, amount_sol_lamports, tip_lamports);

        // 1. Get Jupiter Quote (SOL -> Token)
        let quote = self.get_jupiter_quote(SOL_MINT, mint, amount_sol_lamports, slippage_bps).await?;

        // 2. Get Jupiter Swap Transaction
        let mut swap_tx = self.get_jupiter_swap_tx(quote).await?;

        // 3. Sign Swap Transaction
        self.presigner.sign_versioned_tx(&mut swap_tx)?;

        // 4. Create Tip Transaction
        let tip_tx = self.create_tip_transaction(tip_lamports)?;

        // 5. Bundle and Send
        let bundle_id = self.send_jito_bundle(vec![swap_tx, tip_tx]).await?;
        
        info!("🚀 Jito Bundle Sent! ID: {}", bundle_id);
        Ok(bundle_id)
    }

    /// Execute a SELL order using Jito Bundle
    pub async fn sell_with_jito(
        &self,
        mint: &str,
        amount_token_raw: u64,
        tip_lamports: u64,
        slippage_bps: u16,
    ) -> Result<String> {
        info!("⚡ Preparing Jito SELL for {} (Amt: {}, Tip: {})", mint, amount_token_raw, tip_lamports);

        // 1. Get Jupiter Quote (Token -> SOL)
        let quote = self.get_jupiter_quote(mint, SOL_MINT, amount_token_raw, slippage_bps).await?;

        // 2. Get Jupiter Swap Transaction
        let mut swap_tx = self.get_jupiter_swap_tx(quote).await?;

        // 3. Sign Swap Transaction
        self.presigner.sign_versioned_tx(&mut swap_tx)?;

        // 4. Create Tip Transaction
        let tip_tx = self.create_tip_transaction(tip_lamports)?;

        // 5. Bundle and Send
        let bundle_id = self.send_jito_bundle(vec![swap_tx, tip_tx]).await?;
        
        info!("🚀 Jito Bundle Sent! ID: {}", bundle_id);
        Ok(bundle_id)
    }

    // ========== HELPERS ==========

    async fn get_jupiter_quote(&self, input_mint: &str, output_mint: &str, amount: u64, slippage_bps: u16) -> Result<serde_json::Value> {
        // Acquire rate limit token if enabled
        if let Some(limiter) = &self.jupiter_limiter {
            limiter.acquire().await;
        }
        
        let url = format!(
            "{}?inputMint={}&outputMint={}&amount={}&slippageBps={}",
            JUPITER_QUOTE_API, input_mint, output_mint, amount, slippage_bps
        );
        
        let response = self.client.get(&url).send().await?.json().await?;
        Ok(response)
    }

    async fn get_jupiter_swap_tx(&self, quote: serde_json::Value) -> Result<VersionedTransaction> {
        let request = json!({
            "quoteResponse": quote,
            "userPublicKey": self.presigner.pubkey().to_string(),
            "wrapAndUnwrapSol": true
        });

        let response: serde_json::Value = self.client.post(JUPITER_SWAP_API)
            .json(&request)
            .send().await?
            .json().await?;

        let swap_tx_base64 = response.get("swapTransaction")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("No swapTransaction in Jupiter response"))?;

        let tx_bytes = BASE64_STANDARD.decode(swap_tx_base64)?;
        let tx: VersionedTransaction = bincode::deserialize(&tx_bytes)?;
        
        Ok(tx)
    }

    fn create_tip_transaction(&self, tip_lamports: u64) -> Result<VersionedTransaction> {
        // Pick random tip account
        let idx = rand::rng().random_range(0..JITO_TIP_ACCOUNTS.len());
        let tip_account_str = JITO_TIP_ACCOUNTS[idx];
        let tip_account = Pubkey::from_str(tip_account_str)?;

        // Create transfer instruction
        let ix = system_instruction::transfer(
            &self.presigner.pubkey(),
            &tip_account,
            tip_lamports,
        );

        // Build transaction using Presigner (Legacy is fine, but we need to convert to Versioned for uniformity in bundle?)
        // Jito bundles can mix Legacy and Versioned.
        // However, `send_jito_bundle` will serialize them.
        
        // Use Presigner to build signed LEGACY transaction first
        let legacy_tx = self.presigner.build_and_sign_tx(&[ix])?;
        
        // Convert to VersionedTransaction for uniform handling if needed, 
        // OR just handle serialization in send_jito_bundle.
        // `VersionedTransaction::from(legacy_tx)` exists.
        
        Ok(VersionedTransaction::from(legacy_tx))
    }

    async fn send_jito_bundle(&self, transactions: Vec<VersionedTransaction>) -> Result<String> {
        // Serialize transactions to base58
        let encoded_txs: Vec<String> = transactions.iter()
            .map(|tx| {
                let serialized = bincode::serialize(tx).unwrap(); // Should not fail
                bs58::encode(serialized).into_string()
            })
            .collect();

        let request = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "sendBundle",
            "params": [
                encoded_txs
            ]
        });

        // Jito Block Engine requires specific endpoints. 
        // Using the constant URL defined above.
        
        let response = self.client.post(JITO_BLOCK_ENGINE_URL)
            .json(&request)
            .send().await?;

        let resp_json: serde_json::Value = response.json().await?;
        
        if let Some(result) = resp_json.get("result") {
            Ok(result.to_string())
        } else {
            Err(anyhow::anyhow!("Jito Bundle Error: {:?}", resp_json))
        }
    }
}
