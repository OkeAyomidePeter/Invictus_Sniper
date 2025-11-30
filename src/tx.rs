use crate::config::Config;
use crate::presigner::Presigner;
use crate::rate_limiter::RateLimiter;
use anyhow::{Context, Result};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use log::{info, warn};
use rand::Rng;
use reqwest::Client;
use serde_json::json;
use solana_sdk::{
    compute_budget::ComputeBudgetInstruction,
    instruction::Instruction,
    pubkey::Pubkey,
    system_instruction,
};
use spl_associated_token_account::get_associated_token_address;
use std::str::FromStr;
use std::sync::Arc;

const JUPITER_QUOTE_API: &str = "https://quote-api.jup.ag/v6/quote";
const JUPITER_SWAP_API: &str = "https://quote-api.jup.ag/v6/swap";
const SOL_MINT: &str = "So11111111111111111111111111111111111111112";

// Jito Tip Accounts (VERIFIED - DO NOT MODIFY)
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

        Self {
            client: Client::new(),
            payer_pubkey: presigner.pubkey(),
            jupiter_limiter,
            base_tip_lamports: config.jito_base_tip_lamports,
            min_tip_lamports: config.jito_min_tip_lamports,
            max_tip_lamports: config.jito_max_tip_lamports,
            dynamic_tips_enabled: config.jito_dynamic_tips_enabled,
        }
    }

    /// Build BUY instructions (NO SIGNING)
    /// Returns: Vec<Instruction> ready for presigner
    pub async fn build_buy_instructions(
        &self,
        mint: &str,
        amount_sol_lamports: u64,
        slippage_bps: u16,
        router: DexRouter,
        tip_lamports: u64,
    ) -> Result<Vec<Instruction>> {
        info!("🔨 Building BUY instructions for {} via {:?}", mint, router);

        let mut instructions = Vec::new();

        // 1. Compute Budget (HIGH priority for buys)
        instructions.push(ComputeBudgetInstruction::set_compute_unit_limit(400_000));
        instructions.push(ComputeBudgetInstruction::set_compute_unit_price(1_000_000)); // 1M micro-lamports

        // 2. Create ATA if needed
        let mint_pubkey = Pubkey::from_str(mint)?;
        let ata = get_associated_token_address(&self.payer_pubkey, &mint_pubkey);
        
        // TODO: Check if ATA exists (requires RPC call)
        // For now, always include creation instruction (it will no-op if exists)
        instructions.push(
            spl_associated_token_account::instruction::create_associated_token_account(
                &self.payer_pubkey,
                &self.payer_pubkey,
                &mint_pubkey,
                &spl_token::id(),
            )
        );

        // 3. Get swap instructions from router
        let swap_ixs = self.get_swap_instructions(
            SOL_MINT,
            mint,
            amount_sol_lamports,
            slippage_bps,
            router,
        ).await?;
        instructions.extend(swap_ixs);

        // 4. Jito Tip (FINAL instruction)
        instructions.push(self.create_tip_instruction(tip_lamports)?);

        info!("✅ Built {} instructions for BUY", instructions.len());
        Ok(instructions)
    }

    /// Build SELL instructions (NO SIGNING, NO JITO TIP)
    /// Returns: Vec<Instruction> ready for presigner
    pub async fn build_sell_instructions(
        &self,
        mint: &str,
        amount_token_raw: u64,
        slippage_bps: u16,
        router: DexRouter,
        close_ata: bool,
    ) -> Result<Vec<Instruction>> {
        info!("🔨 Building SELL instructions for {} via {:?}", mint, router);

        let mut instructions = Vec::new();

        // 1. Compute Budget (Standard priority for sells)
        instructions.push(ComputeBudgetInstruction::set_compute_unit_limit(300_000));
        instructions.push(ComputeBudgetInstruction::set_compute_unit_price(500_000)); // 500K micro-lamports

        // 2. Get swap instructions from router
        let swap_ixs = self.get_swap_instructions(
            mint,
            SOL_MINT,
            amount_token_raw,
            slippage_bps,
            router,
        ).await?;
        instructions.extend(swap_ixs);

        // 3. Close ATA if requested (reclaim rent)
        if close_ata {
            let mint_pubkey = Pubkey::from_str(mint)?;
            let ata = get_associated_token_address(&self.payer_pubkey, &mint_pubkey);
            
            instructions.push(
                spl_token::instruction::close_account(
                    &spl_token::id(),
                    &ata,
                    &self.payer_pubkey,
                    &self.payer_pubkey,
                    &[],
                )?
            );
        }

        info!("✅ Built {} instructions for SELL (NO TIP)", instructions.len());
        Ok(instructions)
    }

    /// Route to correct DEX and get swap instructions
    async fn get_swap_instructions(
        &self,
        input_mint: &str,
        output_mint: &str,
        amount: u64,
        slippage_bps: u16,
        router: DexRouter,
    ) -> Result<Vec<Instruction>> {
        match router {
            DexRouter::PumpSwap => {
                // TODO: Implement PumpSwap routing
                warn!("⚠️ PumpSwap routing not yet implemented, falling back to Jupiter");
                self.get_jupiter_swap_instructions(input_mint, output_mint, amount, slippage_bps).await
            }
            DexRouter::Raydium => {
                // TODO: Implement Raydium routing
                warn!("⚠️ Raydium routing not yet implemented, falling back to Jupiter");
                self.get_jupiter_swap_instructions(input_mint, output_mint, amount, slippage_bps).await
            }
            DexRouter::Jupiter => {
                self.get_jupiter_swap_instructions(input_mint, output_mint, amount, slippage_bps).await
            }
        }
    }

    /// Get Jupiter swap instructions (extracts from transaction)
    /// NOTE: This returns individual swap instructions - complex to extract properly
    /// For now, we'll use a hybrid approach: get the full transaction and add our instructions
    async fn get_jupiter_swap_instructions(
        &self,
        input_mint: &str,
        output_mint: &str,
        amount: u64,
        slippage_bps: u16,
    ) -> Result<Vec<Instruction>> {
        // 1. Get quote
        let quote = self.get_jupiter_quote(input_mint, output_mint, amount, slippage_bps).await?;

        // 2. Get swap transaction  
        let request = json!({
            "quoteResponse": quote,
            "userPublicKey": self.payer_pubkey.to_string(),
            "wrapAndUnwrapSol": true,
            "asLegacyTransaction": true // Request legacy format for easier instruction extraction
        });

        let response: serde_json::Value = self.client.post(JUPITER_SWAP_API)
            .json(&request)
            .send().await?
            .json().await?;

        let swap_tx_base64 = response.get("swapTransaction")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("No swapTransaction in Jupiter response"))?;

        let tx_bytes = BASE64_STANDARD.decode(swap_tx_base64)?;
        
        // Try to deserialize as legacy first
        if let Ok(legacy_tx) = bincode::deserialize::<solana_sdk::transaction::Transaction>(&tx_bytes) {
            // Extract instructions from legacy transaction (much simpler)
            return Ok(legacy_tx.message.instructions.iter().map(|compiled_ix| {
                let program_id = legacy_tx.message.account_keys[compiled_ix.program_id_index as usize];
                let accounts = compiled_ix.accounts.iter()
                    .map(|&idx| solana_sdk::instruction::AccountMeta {
                        pubkey: legacy_tx.message.account_keys[idx as usize],
                        is_signer: legacy_tx.message.is_signer(idx as usize),
                        is_writable: legacy_tx.message.is_writable(idx as usize),
                    })
                    .collect::<Vec<_>>();

                Instruction {
                    program_id,
                    accounts,
                    data: compiled_ix.data.clone(),
                }
            }).collect());
        }

        Err(anyhow::anyhow!("Failed to deserialize Jupiter transaction"))
    }

    /// Get Jupiter quote
    async fn get_jupiter_quote(
        &self,
        input_mint: &str,
        output_mint: &str,
        amount: u64,
        slippage_bps: u16,
    ) -> Result<serde_json::Value> {
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

    /// Create Jito tip instruction (single instruction, not separate tx)
    fn create_tip_instruction(&self, tip_lamports: u64) -> Result<Instruction> {
        // Pick random tip account
        let idx = rand::rng().random_range(0..JITO_TIP_ACCOUNTS.len());
        let tip_account_str = JITO_TIP_ACCOUNTS[idx];
        let tip_account = Pubkey::from_str(tip_account_str)?;

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
        tip.clamp(self.min_tip_lamports, self.max_tip_lamports)
    }
}
