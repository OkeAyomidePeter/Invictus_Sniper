use crate::config::Config;
use anyhow::{Context, Result};
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

const JITO_BLOCK_ENGINE_URL: &str = "https://mainnet.block-engine.jito.wtf/api/v1/bundles";

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
        
        // Start background update loop
        let blockhash_clone = blockhash.clone();
        let rpc_clone = rpc_client.clone();
        
        let update_handle = tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(2));
            loop {
                interval.tick().await;
                // We use a blocking RPC call inside spawn_blocking or just accept it here since it's a dedicated task
                // Ideally use non-blocking client, but RpcClient is blocking.
                // For this snippet, we'll wrap in spawn_blocking if strictly needed, but tokio::spawn allows blocking if it doesn't block the runtime.
                // RpcClient IS blocking. We should use RpcClient inside spawn_blocking.
                
                let rpc = rpc_clone.clone();
                let hash_lock = blockhash_clone.clone();
                
                let res = tokio::task::spawn_blocking(move || {
                    rpc.get_latest_blockhash()
                }).await;

                match res {
                    Ok(Ok(new_hash)) => {
                        if let Ok(mut w) = hash_lock.write() {
                            *w = new_hash;
                            // info!("🔄 Blockhash updated: {}", new_hash); // Verbose
                        }
                    },
                    Ok(Err(e)) => warn!("⚠️ Failed to update blockhash: {}", e),
                    Err(e) => error!("❌ Blockhash update task panicked: {}", e),
                }
            }
        });

        info!("⚡ Presigner initialized with background blockhash updates");

        Self {
            rpc_client,
            http_client: Client::new(),
            keypair,
            blockhash,
            _update_handle: update_handle,
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

    /// Send a transaction immediately
    pub fn send_transaction(&self, tx: &Transaction) -> Result<String> {
        self.rpc_client.send_and_confirm_transaction(tx)
            .context("Failed to send transaction")
            .map(|s| s.to_string())
    }

    /// Send a VersionedTransaction immediately
    pub fn send_versioned_transaction(&self, tx: &VersionedTransaction) -> Result<String> {
        match self.rpc_client.send_and_confirm_transaction(tx) {
            Ok(sig) => Ok(sig.to_string()),
            Err(e) => {
                let logs = self.extract_rpc_logs(&e);
                if !logs.is_empty() {
                    error!("❌ RPC Send Failure with Logs:");
                    for log in &logs {
                        error!("  > {}", log);
                    }
                } else {
                    error!("❌ RPC Send Failure: {:?}", e);
                }
                
                // Return a more descriptive error that includes the logs if available
                let err_msg = if !logs.is_empty() {
                    format!("Failed to send versioned transaction: {}. Logs: {:?}", e, logs)
                } else {
                    format!("Failed to send versioned transaction: {}", e)
                };
                
                Err(anyhow::anyhow!(err_msg))
            }
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

    /// Send a Jito Bundle (Swap Tx + Tip Tx)
    pub async fn send_jito_bundle(&self, transactions: Vec<VersionedTransaction>) -> Result<String> {
        if transactions.is_empty() {
            return Err(anyhow::anyhow!("Cannot send empty bundle"));
        }

        // Serialize transactions to base58
        let encoded_txs: Vec<String> = transactions.iter()
            .map(|tx| {
                let serialized = bincode::serialize(tx).unwrap();
                bs58::encode(serialized).into_string()
            })
            .collect();

        // Construct JSON-RPC request
        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "sendBundle",
            "params": [encoded_txs]
        });

        info!("🚀 Sending Jito Bundle with {} transactions...", transactions.len());

        let response = self.http_client.post(JITO_BLOCK_ENGINE_URL)
            .json(&request)
            .send()
            .await
            .context("Failed to send Jito bundle request")?;

        let response_json: serde_json::Value = response.json().await
            .context("Failed to parse Jito bundle response")?;

        if let Some(result) = response_json.get("result") {
            let bundle_id = result.as_str().unwrap_or("unknown").to_string();
            info!("✅ Jito Bundle sent! ID: {}", bundle_id);
            Ok(bundle_id)
        } else {
            let error = response_json.get("error")
                .map(|e| e.to_string())
                .unwrap_or_else(|| "Unknown error".to_string());
            Err(anyhow::anyhow!("Jito Bundle failed: {}", error))
        }
    }

    /// Wait for Jito bundle confirmation with timeout
    /// Polls getBundleStatuses endpoint every 500ms
    pub async fn wait_for_bundle_confirmation(&self, bundle_id: &str, timeout_secs: u64) -> Result<BundleStatus> {
        let start = std::time::Instant::now();
        let timeout = std::time::Duration::from_secs(timeout_secs);
        let poll_interval = std::time::Duration::from_millis(500);

        info!("⏳ Waiting for bundle confirmation: {} (timeout: {}s)", bundle_id, timeout_secs);

        while start.elapsed() < timeout {
            match self.get_bundle_status(bundle_id).await {
                Ok(status) => {
                    match status {
                        BundleStatus::Landed => {
                            info!("✅ Bundle {} confirmed on-chain!", bundle_id);
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
                            // Continue polling
                        }
                    }
                }
                Err(e) => {
                    warn!("Failed to get bundle status: {}", e);
                    // Continue polling on transient errors
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

        let response = self.http_client.post(JITO_BLOCK_ENGINE_URL)
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
    pub async fn get_token_balance(&self, mint_str: &str) -> Result<u64> {
        let mint = Pubkey::from_str(mint_str)?;
        let owner = self.keypair.pubkey();
        let ata = spl_associated_token_account::get_associated_token_address(&owner, &mint);
        
        let rpc = self.rpc_client.clone();
        
        let balance = tokio::task::spawn_blocking(move || {
            rpc.get_token_account_balance(&ata)
        }).await??;
        
        Ok(balance.amount.parse::<u64>()?)
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
