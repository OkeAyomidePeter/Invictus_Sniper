use crate::config::Config;
use crate::enrichment::EnrichedToken;
use anyhow::{Context, Result};
use log::{error, info, warn};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use solana_sdk::{
    commitment_config::CommitmentConfig,
    pubkey::Pubkey,
    signature::{Keypair, Signer},
    transaction::VersionedTransaction,
};
use std::str::FromStr;
use std::sync::Arc;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};

const JUPITER_QUOTE_API: &str = "https://quote-api.jup.ag/v6/quote";
const JUPITER_SWAP_API: &str = "https://quote-api.jup.ag/v6/swap";
const SOL_MINT: &str = "So11111111111111111111111111111111111111112";
const TEST_BUY_AMOUNT_LAMPORTS: u64 = 100_000; // 0.0001 SOL (Very tiny)

#[derive(Debug, Clone, Serialize)]
pub struct RiskVerifiedToken {
    pub token: EnrichedToken,
    pub real_buy_tax: Option<f64>,
    pub real_sell_tax: Option<f64>,
    pub is_honeypot: bool,
    pub verification_log: Vec<String>,
}

pub struct RiskEngine {
    client: Client,
    keypair: Arc<Keypair>,
    rpc_client: Arc<solana_client::rpc_client::RpcClient>,
}

impl RiskEngine {
    pub fn new(config: &Config) -> Self {
        // Load keypair from config (assuming config has it or path)
        // For now, we'll try to load from the private key string in config
        let keypair = if std::path::Path::new(&config.private_key).exists() {
             solana_sdk::signature::read_keypair_file(&config.private_key)
                .expect("Failed to read keypair file")
        } else {
            Keypair::from_base58_string(&config.private_key)
        };

        let rpc_client = solana_client::rpc_client::RpcClient::new_with_commitment(
            "https://api.mainnet-beta.solana.com".to_string(), // Should come from config ideally
            CommitmentConfig::confirmed(),
        );

        Self {
            client: Client::new(),
            keypair: Arc::new(keypair),
            rpc_client: Arc::new(rpc_client),
        }
    }

    pub async fn verify_token(&self, token: EnrichedToken) -> Result<RiskVerifiedToken> {
        let mut logs = Vec::new();
        logs.push(format!("Starting risk verification for {}", token.mint));

        // 1. Perform Tiny Buy
        logs.push("Attempting tiny buy...".to_string());
        let buy_result = self.execute_swap(
            SOL_MINT, 
            &token.mint, 
            TEST_BUY_AMOUNT_LAMPORTS
        ).await;

        let token_balance = match buy_result {
            Ok((sig, _)) => {
                logs.push(format!("Buy successful: {}", sig));
                // Fetch new balance
                self.get_token_balance(&token.mint).await?
            },
            Err(e) => {
                logs.push(format!("Buy failed: {}", e));
                return Ok(RiskVerifiedToken {
                    token,
                    real_buy_tax: None,
                    real_sell_tax: None,
                    is_honeypot: true, // If we can't buy, it's effectively a honeypot or broken
                    verification_log: logs,
                });
            }
        };

        if token_balance == 0 {
             logs.push("Buy successful but balance is 0 (100% tax?)".to_string());
             return Ok(RiskVerifiedToken {
                token,
                real_buy_tax: Some(100.0),
                real_sell_tax: None,
                is_honeypot: true,
                verification_log: logs,
            });
        }

        // 2. Perform Sell (All tokens)
        logs.push(format!("Attempting sell of {} tokens...", token_balance));
        // We need to know decimals to convert raw amount for Jupiter if needed, 
        // but Jupiter usually takes raw amount (lamports/atomic units)
        
        let sell_result = self.execute_swap(
            &token.mint, 
            SOL_MINT, 
            token_balance
        ).await;

        let is_honeypot = match sell_result {
            Ok((sig, _)) => {
                logs.push(format!("Sell successful: {}", sig));
                false
            },
            Err(e) => {
                logs.push(format!("Sell failed: {}", e));
                true
            }
        };

        // TODO: Calculate actual taxes by comparing expected vs actual balance changes
        // For now, we just check if it's possible to buy and sell.
        
        Ok(RiskVerifiedToken {
            token,
            real_buy_tax: None, // Implement calculation later
            real_sell_tax: None, // Implement calculation later
            is_honeypot,
            verification_log: logs,
        })
    }

    async fn execute_swap(&self, input_mint: &str, output_mint: &str, amount: u64) -> Result<(String, u64)> {
        // 1. Get Quote
        let quote_url = format!(
            "{}?inputMint={}&outputMint={}&amount={}&slippageBps=50", // 0.5% slippage
            JUPITER_QUOTE_API, input_mint, output_mint, amount
        );
        
        let quote_res: serde_json::Value = self.client.get(&quote_url)
            .send().await?
            .json().await?;
            
        if let Some(err) = quote_res.get("error") {
            return Err(anyhow::anyhow!("Quote error: {}", err));
        }

        // 2. Get Swap Transaction
        let swap_req = serde_json::json!({
            "quoteResponse": quote_res,
            "userPublicKey": self.keypair.pubkey().to_string(),
            "wrapAndUnwrapSol": true
        });

        let swap_res: serde_json::Value = self.client.post(JUPITER_SWAP_API)
            .json(&swap_req)
            .send().await?
            .json().await?;

        let swap_tx_base64 = swap_res.get("swapTransaction")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("No swap transaction in response"))?;

        // 3. Deserialize and Sign
        let tx_bytes = BASE64_STANDARD.decode(swap_tx_base64)?;
        let mut transaction: VersionedTransaction = bincode::deserialize(&tx_bytes)?;
        
        // Sign
        let message = transaction.message.clone(); // VersionedMessage
        // We need to sign with our keypair. 
        // VersionedTransaction signing is a bit specific.
        let signature = self.keypair.sign_message(&message.serialize());
        transaction.signatures = vec![signature];

        // 4. Send and Confirm
        let signature = self.rpc_client.send_and_confirm_transaction(&transaction)?;
        
        // Return signature and output amount (from quote for now, ideally from simulation/balance diff)
        let out_amount = quote_res.get("outAmount")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0);

        Ok((signature.to_string(), out_amount))
    }

    async fn get_token_balance(&self, mint: &str) -> Result<u64> {
        let mint_pubkey = Pubkey::from_str(mint)?;
        let user_pubkey = self.keypair.pubkey();
        
        // Find ATA
        let ata = spl_associated_token_account::get_associated_token_address(
            &user_pubkey,
            &mint_pubkey,
        );

        match self.rpc_client.get_token_account_balance(&ata) {
            Ok(balance) => Ok(balance.amount.parse()?),
            Err(_) => Ok(0), // Account might not exist yet
        }
    }
}
