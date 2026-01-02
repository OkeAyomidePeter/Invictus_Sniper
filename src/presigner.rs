use crate::config::Config;
use anyhow::{Context, Result};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use log::{error, info, warn};
use solana_client::rpc_client::RpcClient;
use solana_sdk::{
    commitment_config::CommitmentConfig,
    hash::Hash,
    instruction::Instruction,
    message::Message,
    pubkey::Pubkey,
    signature::{Keypair, Signer},
    transaction::{Transaction, VersionedTransaction},
};
use std::sync::{Arc, RwLock};
use std::time::Duration;
use tokio::task::JoinHandle;
use std::str::FromStr;
use reqwest::Client; // For Jito Bundle API
use crate::moralis_client::MoralisClient;
use solana_client::rpc_response::RpcSimulateTransactionResult;

// (Removed redundant constant, using JITO_ENDPOINTS[0] instead)

/// Bundle confirmation status from Jito
#[derive(Debug, Clone, PartialEq)]
pub enum BundleStatus {
    /// Bundle is still being processed
    Pending,
    /// Bundle landed on-chain successfully
    Landed,
    /// Bundle failed to land
    Failed(String),
    /// Bundle was invalid (signature issues, etc.)
    Invalid(String),
    /// Bundle not found (may not have been received yet)
    NotFound,
}

impl BundleStatus {
    pub fn is_success(&self) -> bool {
        matches!(self, BundleStatus::Landed)
    }

    pub fn is_terminal(&self) -> bool {
        matches!(self, BundleStatus::Landed | BundleStatus::Failed(_) | BundleStatus::Invalid(_))
    }
}


/// Manages blockhash updates and rapid transaction signing
pub struct Presigner {
    rpc_client: Arc<RpcClient>,
    http_client: Client, // For Jito
    keypair: Arc<Keypair>,
    blockhash: Arc<RwLock<Hash>>,
    _update_handle: JoinHandle<()>,
    moralis_client: Option<MoralisClient>, // Optional Moralis for balance checks
    wallet_address: String, // For Moralis queries
}

impl Presigner {
    pub fn new(config: &Config) -> Self {
        let rpc_url = config.rpc_url.clone();
        let rpc_client = Arc::new(RpcClient::new_with_commitment(
            rpc_url.clone(),
            CommitmentConfig::confirmed(),
        ));

        // Load keypair
        let keypair = if std::path::Path::new(&config.private_key).exists() {
             solana_sdk::signature::read_keypair_file(&config.private_key)
                .expect("Failed to read keypair file")
        } else {
            Keypair::from_base58_string(&config.private_key)
        };
        let keypair = Arc::new(keypair);

        // Initial blockhash fetch
        let initial_hash = match rpc_client.get_latest_blockhash() {
            Ok(h) => h,
            Err(e) => {
                error!("Failed to fetch initial blockhash: {}", e);
                Hash::default()
            }
        };
        
        let blockhash = Arc::new(RwLock::new(initial_hash));
        
        // Start background update loop with supervisor (restarts on panic or accidental exit)
        let blockhash_clone = blockhash.clone();
        let rpc_clone = rpc_client.clone();
        
        let update_handle = tokio::spawn(async move {
            loop {
                let rpc = rpc_clone.clone();
                let hash_lock = blockhash_clone.clone();
                
                let task = tokio::spawn(async move {
                    let mut interval = tokio::time::interval(Duration::from_secs(2));
                    loop {
                        interval.tick().await;
                        
                        let rpc_inner = rpc.clone();
                        let res = tokio::task::spawn_blocking(move || {
                            rpc_inner.get_latest_blockhash()
                        }).await;

                        match res {
                            Ok(Ok(new_hash)) => {
                                if let Ok(mut w) = hash_lock.write() {
                                    *w = new_hash;
                                }
                            },
                            Ok(Err(e)) => warn!("⚠️ Failed to update blockhash: {}", e),
                            Err(e) => {
                                error!("❌ Blockhash update task crashed: {}", e);
                                break; // Exit inner loop to trigger restart
                            }
                        }
                    }
                });

                if let Err(e) = task.await {
                    error!("❌ Blockhash supervisor detected panic: {}. Restarting in 5s...", e);
                    tokio::time::sleep(Duration::from_secs(5)).await;
                } else {
                    warn!("⚠️ Blockhash loop exited unexpectedly. Restarting in 1s...");
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            }
        });

        info!("⚡ Presigner initialized with background blockhash updates");
        
        let wallet_address = keypair.pubkey().to_string();
        
        // Initialize Moralis client if API key is available
        let moralis_client = if !config.moralis_api_key.is_empty() {
            Some(MoralisClient::new(
                config.moralis_api_key.clone(),
                "mainnet".to_string()
            ))
        } else {
            None
        };
        
        Self {
            rpc_client,
            http_client: Client::new(),
            keypair,
            blockhash,
            _update_handle: update_handle,
            moralis_client,
            wallet_address,
        }
    }

    /// Get the currently cached fresh blockhash
    pub fn get_blockhash(&self) -> Hash {
        *self.blockhash.read().unwrap()
    }

    /// Get the signer's public key
    pub fn pubkey(&self) -> Pubkey {
        self.keypair.pubkey()
    }

    /// Build and sign a transaction immediately using cached blockhash
    /// "Injects" dynamic instructions into a transaction template
    pub fn build_and_sign_tx(
        &self,
        instructions: &[Instruction],
    ) -> Result<Transaction> {
        // 1. Validate instructions
        self.validate_instructions(instructions)?;

        let payer_pubkey = self.keypair.pubkey();
        let recent_blockhash = self.get_blockhash();

        let message = Message::new(instructions, Some(&payer_pubkey));
        
        // 2. Verify Payer Match (Critical)
        if message.account_keys.get(0) != Some(&payer_pubkey) {
            return Err(anyhow::anyhow!("Transaction payer mismatch! Expected {}", payer_pubkey));
        }

        let mut tx = Transaction::new_unsigned(message);
        
        // Sign immediately
        tx.try_sign(&[self.keypair.as_ref()], recent_blockhash)?;
        
        Ok(tx)
    }

    /// Validate instructions for safety and best practices
    fn validate_instructions(&self, instructions: &[Instruction]) -> Result<()> {
        if instructions.is_empty() {
            return Err(anyhow::anyhow!("Transaction has no instructions"));
        }

        // Check for Compute Budget (Critical for landing)
        let compute_budget_program = Pubkey::from_str("ComputeBudget111111111111111111111111111111").unwrap();
        let has_compute_budget = instructions.iter().any(|ix| ix.program_id == compute_budget_program);

        if !has_compute_budget {
            warn!("⚠️ Transaction missing Compute Budget instruction - likely to fail!");
            // We could return Err here, but for now just warn
        }

        Ok(())
    }

    /// Simulates a transaction
    pub async fn simulate_transaction(&self, tx: &VersionedTransaction) -> Result<RpcSimulateTransactionResult> {
        let rpc = self.rpc_client.clone();
        let tx_clone = tx.clone();
        
        let res = tokio::task::spawn_blocking(move || {
            rpc.simulate_transaction(&tx_clone)
        }).await??;
        
        Ok(res.value)
    }

    /// Send a transaction immediately
    pub async fn send_transaction(&self, tx: &Transaction) -> Result<String> {
        let rpc = self.rpc_client.clone();
        let tx_clone = tx.clone();
        
        let sig = tokio::task::spawn_blocking(move || {
            rpc.send_and_confirm_transaction(&tx_clone)
        }).await??;
        
        Ok(sig.to_string())
    }

    /// Send a VersionedTransaction immediately
    pub async fn send_versioned_transaction(&self, tx: &VersionedTransaction) -> Result<String> {
        let rpc = self.rpc_client.clone();
        let tx_clone = tx.clone();
        
        let res = tokio::task::spawn_blocking(move || {
            rpc.send_and_confirm_transaction(&tx_clone)
        }).await;

        match res {
            Ok(Ok(sig)) => Ok(sig.to_string()),
            Ok(Err(e)) => {
                let logs = self.extract_rpc_logs(&e);
                if !logs.is_empty() {
                    error!("❌ RPC Send Failure with Logs:");
                    for log in &logs {
                        error!("  > {}", log);
                    }
                } else {
                    error!("❌ RPC Send Failure: {:?}", e);
                }
                
                let err_msg = if !logs.is_empty() {
                    format!("Failed to send versioned transaction: {}. Logs: {:?}", e, logs)
                } else {
                    format!("Failed to send versioned transaction: {}", e)
                };
                
                Err(anyhow::anyhow!(err_msg))
            }
            Err(e) => Err(anyhow::anyhow!("RPC task panicked: {}", e)),
        }
    }

    /// Extract program logs from a Solana RPC client error
    fn extract_rpc_logs(&self, err: &solana_client::client_error::ClientError) -> Vec<String> {
        use solana_client::client_error::ClientErrorKind;
        use solana_client::rpc_request::RpcError;
        use solana_client::rpc_request::RpcResponseErrorData;

        match err.kind() {
            ClientErrorKind::RpcError(RpcError::RpcResponseError { 
                data: RpcResponseErrorData::SendTransactionPreflightFailure(result),
                ..
            }) => {
                result.logs.clone().unwrap_or_default()
            }
            _ => Vec::new()
        }
    }

    /// Sign an existing VersionedTransaction with the loaded keypair
    pub fn sign_versioned_tx(&self, tx: &mut VersionedTransaction) -> Result<()> {
        // Verify we are the payer (first account in static keys)
        let payer = tx.message.static_account_keys().get(0)
            .ok_or_else(|| anyhow::anyhow!("Transaction has no account keys"))?;
            
        if payer != &self.keypair.pubkey() {
             return Err(anyhow::anyhow!("Transaction payer mismatch! Expected {}, got {}", self.keypair.pubkey(), payer));
        }

        let message_data = tx.message.serialize();
        let signature = self.keypair.sign_message(&message_data);
        
        // Assuming we are the primary/only signer or the first one.
        // Jupiter transactions usually expect the user to be the first signer.
        if tx.signatures.is_empty() {
            tx.signatures = vec![signature];
        } else {
            // If there are placeholders, we might need to place it correctly.
            // For simple swaps, we are usually index 0.
            tx.signatures[0] = signature;
        }
        Ok(())
    }
}

// Jito Block Engine Endpoints
const JITO_ENDPOINTS: &[&str] = &[
    "https://mainnet.block-engine.jito.wtf/api/v1/bundles",
    "https://ny.mainnet.block-engine.jito.wtf/api/v1/bundles",
    "https://amsterdam.mainnet.block-engine.jito.wtf/api/v1/bundles",
    "https://frankfurt.mainnet.block-engine.jito.wtf/api/v1/bundles",
    "https://tokyo.mainnet.block-engine.jito.wtf/api/v1/bundles",
];

impl Presigner {
    /// Send a Jito Bundle (Swap Tx + Tip Tx)
    /// Returns (Bundle ID, Transaction Signatures)
    pub async fn send_jito_bundle(&self, mut transactions: Vec<VersionedTransaction>) -> Result<(String, Vec<solana_sdk::signature::Signature>)> {
        if transactions.is_empty() {
            return Err(anyhow::anyhow!("Cannot send empty bundle"));
        }

        // Capture signatures before encoding
        let signatures: Vec<solana_sdk::signature::Signature> = transactions.iter()
            .map(|tx| tx.signatures.get(0).copied().unwrap_or_default())
            .collect();

        // Retry logic with endpoint cycling and exponential backoff
        let max_attempts = 10; // More attempts since we cycle endpoints
        let mut last_error = String::new();
        let mut endpoint_index = 0;
        
        for attempt in 1..=max_attempts {
            let current_endpoint = JITO_ENDPOINTS[endpoint_index % JITO_ENDPOINTS.len()];

            // 💡 Loophole Fix: Refresh blockhash and re-sign on every attempt (especially retries)
            let current_hash = self.get_blockhash();
            for tx in transactions.iter_mut() {
                match &mut tx.message {
                    solana_sdk::message::VersionedMessage::Legacy(m) => m.recent_blockhash = current_hash,
                    solana_sdk::message::VersionedMessage::V0(m) => m.recent_blockhash = current_hash,
                }
                self.sign_versioned_tx(tx)?;
            }

            // Serialize transactions to base64
            let encoded_txs: Vec<String> = transactions.iter()
                .map(|tx| {
                    let serialized = bincode::serialize(tx).unwrap();
                    BASE64_STANDARD.encode(serialized)
                })
                .collect();

            let request = serde_json::json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "sendBundle",
                "params": [encoded_txs, {"encoding": "base64"}]
            });

            if attempt == 1 {
                info!("🚀 Sending Jito Bundle (hash: {}) to {}...", current_hash, current_endpoint);
            }

            let response = match self.http_client.post(current_endpoint)
                .json(&request)
                .send()
                .await {
                    Ok(resp) => resp,
                    Err(e) => {
                        warn!("⚠️ Jito request failed for {}: {}. Trying next endpoint...", current_endpoint, e);
                        endpoint_index += 1;
                        continue;
                    }
                };

            let response_json: serde_json::Value = response.json().await
                .context("Failed to parse Jito bundle response")?;

            if let Some(result) = response_json.get("result") {
                let bundle_id = result.as_str().unwrap_or("unknown").to_string();
                info!("✅ Jito Bundle sent! ID: {} (via {})", bundle_id, current_endpoint);
                return Ok((bundle_id, signatures));
            } else if let Some(error) = response_json.get("error") {
                let error_str = error.to_string();
                last_error = error_str.clone();
                
                let is_rate_limited = error.get("code")
                    .and_then(|c| c.as_i64())
                    .map(|c| c == -32097)
                    .unwrap_or(false)
                    || error_str.to_lowercase().contains("rate limit")
                    || error_str.to_lowercase().contains("congested");
                
                if is_rate_limited && attempt < max_attempts {
                    // 💡 Multi-Endpoint logic: Cycle endpoint immediately
                    endpoint_index += 1;
                    
                    // Only sleep if we have exhausted all endpoints in this attempt cycle
                    if endpoint_index % JITO_ENDPOINTS.len() == 0 {
                        let backoff_ms = 1000 * (1 << (attempt / JITO_ENDPOINTS.len()));
                        warn!("⚠️ All Jito endpoints rate limited. Backoff {}ms...", backoff_ms);
                        tokio::time::sleep(Duration::from_millis(backoff_ms)).await;
                    } else {
                        crate::trade_logger::log_jito_debug("RATE_LIMIT", &format!("Cycling to {} (Attempt {})", JITO_ENDPOINTS[endpoint_index % JITO_ENDPOINTS.len()], attempt));
                    }
                    continue;
                }
                
                return Err(anyhow::anyhow!("Jito Bundle failed: {}", error_str));
            } else {
                return Err(anyhow::anyhow!("Jito Bundle failed: Unknown error"));
            }
        }
        
        Err(anyhow::anyhow!("Jito Bundle failed after {} attempts: {}", max_attempts, last_error))
    }

    /// Wait for Jito bundle confirmation with timeout
    /// Polls getBundleStatuses endpoint AND standard RPC for signature confirmation
    pub async fn wait_for_bundle_confirmation(
        &self, 
        bundle_id: &str, 
        timeout_secs: u64,
        expected_signatures: &[solana_sdk::signature::Signature]
    ) -> Result<BundleStatus> {
        let start = std::time::Instant::now();
        let timeout = std::time::Duration::from_secs(timeout_secs);
        let poll_interval = std::time::Duration::from_millis(500);

        info!("⏳ Waiting for bundle confirmation: {} (timeout: {}s)", bundle_id, timeout_secs);

        while start.elapsed() < timeout {
            // 1. Check Jito Bundle Status
            match self.get_bundle_status(bundle_id).await {
                Ok(status) => {
                    match status {
                        BundleStatus::Landed => {
                            info!("✅ Bundle {} confirmed on-chain via Jito!", bundle_id);
                            return Ok(status);
                        }
                        BundleStatus::Failed(ref reason) => {
                            warn!("❌ Bundle {} failed: {}", bundle_id, reason);
                            return Ok(status);
                        }
                        BundleStatus::Invalid(ref reason) => {
                            warn!("❌ Bundle {} invalid: {}", bundle_id, reason);
                            return Ok(status);
                        }
                        BundleStatus::Pending | BundleStatus::NotFound => {
                            // detailed polling logs (every 4th poll = every 2s)
                            if (start.elapsed().as_millis() / 500) % 4 == 0 {
                                crate::trade_logger::log_jito_debug("POLL", &format!("Bundle {} status: {:?}", bundle_id, status));
                            }
                        }
                    }
                }
                Err(e) => {
                    warn!("Failed to get bundle status from Jito: {}", e);
                }
            }

            // 2. 💡 Fallback Check: Check RPC for actual signature confirmation
            // This catches cases where Jito API is lagging but the TX actually landed.
            if !expected_signatures.is_empty() {
                for sig in expected_signatures {
                    match self.check_signature_success(&sig.to_string()).await {
                        Ok(true) => {
                            info!("🎯 Transaction confirmed on-chain via RPC! (Sig: {})", sig);
                            crate::trade_logger::log_jito_debug("FALLBACK", &format!("Landed via RPC fallback: {}", sig));
                            return Ok(BundleStatus::Landed);
                        },
                        _ => {} // Continue polling
                    }
                }
            }

            tokio::time::sleep(poll_interval).await;
        }

        warn!("⏰ Bundle confirmation timeout after {}s: {}", timeout_secs, bundle_id);
        Err(anyhow::anyhow!("Bundle confirmation timeout after {}s", timeout_secs))
    }

    /// Get current bundle status from Jito
    async fn get_bundle_status(&self, bundle_id: &str) -> Result<BundleStatus> {
        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "getBundleStatuses",
            "params": [[bundle_id]]
        });

        let response = self.http_client.post(JITO_ENDPOINTS[0])
            .json(&request)
            .send()
            .await
            .context("Failed to get bundle status")?;

        let response_json: serde_json::Value = response.json().await
            .context("Failed to parse bundle status response")?;

        // Parse response: { "result": { "value": [{ "bundle_id": "...", "status": "Landed" | "Pending" | ... }] } }
        if let Some(result) = response_json.get("result") {
            if let Some(value) = result.get("value").and_then(|v| v.as_array()) {
                if let Some(bundle_status) = value.first() {
                    let status_str = bundle_status.get("confirmation_status")
                        .or_else(|| bundle_status.get("status"))
                        .and_then(|s| s.as_str())
                        .unwrap_or("unknown");

                    return Ok(match status_str.to_lowercase().as_str() {
                        "landed" | "confirmed" | "finalized" => BundleStatus::Landed,
                        "pending" | "processed" => BundleStatus::Pending,
                        "failed" => {
                            let reason = bundle_status.get("err")
                                .map(|e| e.to_string())
                                .unwrap_or_else(|| "Unknown failure".to_string());
                            BundleStatus::Failed(reason)
                        }
                        "invalid" => {
                            let reason = bundle_status.get("err")
                                .map(|e| e.to_string())
                                .unwrap_or_else(|| "Invalid bundle".to_string());
                            BundleStatus::Invalid(reason)
                        }
                        _ => BundleStatus::Pending,
                    });
                }
            }
        }

        Ok(BundleStatus::NotFound)
    }

    /// Get token balance for a specific mint
    /// Get token balance (Moralis with RPC fallback)
    pub async fn get_token_balance(&self, mint_str: &str) -> Result<u64> {
        // Try Moralis first if available
        if let Some(moralis) = &self.moralis_client {
            for attempt in 1..=3 {
                match moralis.get_spl_token_balance(&self.wallet_address, mint_str).await {
                    Ok(balance) => {
                        if balance > 0 {
                            info!("✅ Moralis: Token balance for {} = {}", &mint_str[..8.min(mint_str.len())], balance);
                            return Ok(balance);
                        } else {
                            if attempt < 3 {
                                warn!("⚠️ Moralis returned 0 for {}. Retrying (attempt {}/3)...", &mint_str[..8.min(mint_str.len())], attempt);
                                tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
                            } else {
                                warn!("⚠️ Moralis returned 0 after 3 attempts for {}. Falling back to RPC.", &mint_str[..8.min(mint_str.len())]);
                            }
                        }
                    }
                    Err(e) => {
                        if attempt < 3 {
                            warn!("⚠️ Moralis check failed: {}. Retrying (attempt {}/3)...", e, attempt);
                            tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
                        } else {
                            warn!("⚠️ Moralis failed after 3 attempts: {}. Falling back to RPC.", e);
                        }
                    }
                }
            }
        }
        
        // Fallback to RPC
        let mint = Pubkey::from_str(mint_str)?;
        let owner = self.keypair.pubkey();
        let ata = spl_associated_token_account::get_associated_token_address(&owner, &mint);
        
        let rpc = self.rpc_client.clone();
        
        let res = tokio::task::spawn_blocking(move || {
            rpc.get_token_account_balance(&ata)
        }).await?;

        match res {
            Ok(balance) => Ok(balance.amount.parse::<u64>()?),
            Err(e) => Err(anyhow::anyhow!("Failed to fetch token balance from RPC: {}", e)),
        }
    }

    /// Check if a transaction signature resulted in a successful execution
    pub async fn check_signature_success(&self, signature: &str) -> Result<bool> {
        let sig = solana_sdk::signature::Signature::from_str(signature)?;
        let rpc = self.rpc_client.clone();
        
        let res = tokio::task::spawn_blocking(move || {
            rpc.get_signature_status(&sig)
        }).await?;
        
        match res {
            Ok(Some(status)) => Ok(status.is_ok()),
            Ok(None) => Ok(false), // Not found or dropped
            Err(e) => Err(anyhow::anyhow!("Failed to verify transaction status: {}", e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use solana_sdk::system_instruction;

    // Helper to create a dummy config
    fn create_test_config() -> Config {
        Config {
            helius_api_key: "test".to_string(),
            rpc_url: "https://api.mainnet-beta.solana.com".to_string(),
            private_key: Keypair::new().to_base58_string(), // Random key
            telegram_token: "".to_string(),
            telegram_chat_id: "".to_string(),
            database_url: "".to_string(),
            min_liquidity_usd: 0.0,
            min_holders: 0,
            max_trade_size_sol: 0.0,
            max_daily_exposure_sol: 0.0,
            max_creator_ownership_percentage: 0.0,
            honeypot_check_enabled: false,
            jupiter_api_timeout_ms: 0,
            auto_sell_enabled: false,
            auto_sell_profit_target_pct: 0.0,
            auto_sell_stop_loss_pct: 0.0,
            auto_sell_timeout_seconds: 0,
            auto_sell_slippage_bps: 0,
            auto_sell_price_check_interval_ms: 0,
            jito_base_tip_lamports: 0,
            jito_min_tip_lamports: 0,
            jito_max_tip_lamports: 0,
            jito_dynamic_tips_enabled: false,
            tx_retry_max_attempts: 0,
            tx_retry_initial_delay_ms: 0,
            tx_retry_max_delay_ms: 0,
            tx_retry_backoff_multiplier: 0.0,
            helius_max_requests_per_second: 0.0,
            jupiter_max_requests_per_second: 0.0,
            rate_limiting_enabled: false,
            wallet_low_balance_alert_sol: 0.0,
            wallet_monitor_interval_secs: 0,
            wallet_reserve_for_fees_sol: 0.0,
            max_concurrent_trades: 0,
            max_open_positions: 0,
            total_exposure_limit_sol: 0.0,
            dip_strategy_enabled: false,
            dip_entry_pct: 0.0,
            min_volume_usd_5m: 0.0,
            watchlist_timeout_seconds: 0,
            volume_trend_enabled: false,
            volume_samples_required: 0,
            volume_sample_interval_secs: 0,
            holder_stability_enabled: false,
            min_holder_retention_pct: 0.0,
            trailing_stop_enabled: false,
            trailing_stop_distance_pct: 0.0,
            trailing_stop_activation_pct: 0.0,
            partial_exit_enabled: false,
            partial_exit_target_pct: 0.0,
            partial_exit_amount_pct: 0.0,
            dynamic_timeout_enabled: false,
            timeout_extension_seconds: 0,
            max_timeout_extensions: 0,
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_validate_empty_instructions() {
        let config = create_test_config();
        let presigner = Presigner::new(&config);
        
        let result = presigner.validate_instructions(&[]);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().to_string(), "Transaction has no instructions");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_validate_missing_compute_budget() {
        let config = create_test_config();
        let presigner = Presigner::new(&config);
        
        // Create a simple transfer instruction (no compute budget)
        let ix = system_instruction::transfer(&Pubkey::new_unique(), &Pubkey::new_unique(), 1000);
        
        // It should warn but NOT error (based on current implementation)
        let result = presigner.validate_instructions(&[ix]);
        assert!(result.is_ok());
    }
}
