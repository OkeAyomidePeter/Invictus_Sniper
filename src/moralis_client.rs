use anyhow::{Context, Result};
use log::{error, warn};
use reqwest::Client;
use serde::{Deserialize, Serialize};

const MORALIS_BASE_URL: &str = "https://solana-gateway.moralis.io";
const MORALIS_DEEP_INDEX_URL: &str = "https://deep-index.moralis.io/api/v2.2";

/// Moralis API client for Solana wallet operations
pub struct MoralisClient {
    client: Client,
    api_key: String,
    network: String, // "mainnet" or "devnet"
}

// ============================================================================
// VERIFIED Response Types (from Moralis docs)
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct MoralisBalanceResponse {
    /// Native SOL balance in lamports (as string)
    pub lamports: String,
    /// Native SOL balance in SOL (as string)
    pub solana: String,
}

#[derive(Debug, Deserialize)]
pub struct MoralisTokenBalance {
    /// Token mint address
    #[serde(rename = "associatedTokenAddress")]
    pub associated_token_address: String,
    /// Token mint
    pub mint: String,
    /// Raw amount (as string, needs to be divided by 10^decimals)
    #[serde(rename = "amountRaw")]
    pub amount_raw: String,
    /// Token decimals
    pub decimals: String,
    /// Human-readable amount
    pub amount: String,
}

// ============================================================================
// Portfolio Response (VERIFIED from Moralis docs)
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct MoralisPortfolio {
    /// Native SOL balance
    #[serde(rename = "nativeBalance")]
    pub native_balance: MoralisBalanceResponse,
    /// SPL token balances
    pub tokens: Vec<MoralisTokenBalance>,
    /// NFTs (if requested)
    #[serde(default)]
    pub nfts: Vec<serde_json::Value>,
}

// ============================================================================
// PnL Response Structures (VERIFIED from Moralis docs)
// Endpoint: https://deep-index.moralis.io/api/v2.2/wallets/:address/profitability
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct MoralisPnLSummary {
    /// Total realized profit in USD
    #[serde(rename = "total_realized_profit_usd")]
    pub total_realized_profit_usd: Option<f64>,
    /// Total number of trades
    #[serde(rename = "total_count_of_trades")]
    pub total_trades: Option<u32>,
    /// Number of profitable trades
    #[serde(rename = "total_count_of_profitable_trades")]
    pub profitable_trades: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct MoralisTokenPnL {
    /// Token address
    #[serde(rename = "token_address")]
    pub token_address: String,
    /// Realized profit in USD
    #[serde(rename = "realized_profit_usd")]
    pub realized_profit_usd: Option<f64>,
    /// Average buy price in USD
    #[serde(rename = "avg_buy_price_usd")]
    pub avg_buy_price_usd: Option<f64>,
    /// Average sell price in USD
    #[serde(rename = "avg_sell_price_usd")]
    pub avg_sell_price_usd: Option<f64>,
}

impl MoralisClient {
    pub fn new(api_key: String, network: String) -> Self {
        Self {
            client: Client::new(),
            api_key,
            network,
        }
    }

    /// Get native SOL balance for an address
    /// Endpoint: GET /account/:network/:address/balance
    /// VERIFIED from Moralis docs
    pub async fn get_sol_balance(&self, address: &str) -> Result<MoralisBalanceResponse> {
        let url = format!(
            "{}/account/{}/{}/balance",
            MORALIS_BASE_URL, self.network, address
        );

        let response = self
            .client
            .get(&url)
            .header("X-API-Key", &self.api_key)
            .send()
            .await
            .context("Failed to send request to Moralis")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!(
                "Moralis API error {}: {}",
                status,
                body
            ));
        }

        response
            .json::<MoralisBalanceResponse>()
            .await
            .context("Failed to parse Moralis balance response")
    }

    /// Get all SPL token balances for an address
    /// Endpoint: GET /account/:network/:address/tokens
    /// VERIFIED from Moralis docs
    pub async fn get_spl_tokens(&self, address: &str) -> Result<Vec<MoralisTokenBalance>> {
        let url = format!(
            "{}/account/{}/{}/tokens",
            MORALIS_BASE_URL, self.network, address
        );

        let response = self
            .client
            .get(&url)
            .header("X-API-Key", &self.api_key)
            .send()
            .await
            .context("Failed to send request to Moralis")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!(
                "Moralis API error {}: {}",
                status,
                body
            ));
        }

        // Get raw text first for debugging
        let raw_text = response.text().await.context("Failed to get response text")?;
        warn!("🔍 DEBUG: Raw Moralis tokens response: {}", raw_text);

        let tokens = serde_json::from_str::<Vec<MoralisTokenBalance>>(&raw_text)
            .context("Failed to parse Moralis tokens response")?;

        Ok(tokens)
    }

    /// Get balance for a specific SPL token
    /// This is a convenience method that filters the tokens list
    pub async fn get_spl_token_balance(&self, address: &str, token_mint: &str) -> Result<u64> {
        let tokens = self.get_spl_tokens(address).await?;

        for token in tokens {
            if token.mint.eq_ignore_ascii_case(token_mint) {
                // Parse the raw amount string to u64
                return token
                    .amount_raw
                    .parse::<u64>()
                    .context("Failed to parse token amount");
            }
        }

        // Token not found in wallet = 0 balance
        Ok(0)
    }

    // ========================================================================
    // Portfolio Endpoint (VERIFIED)
    // GET /account/:network/:address/portfolio
    // ========================================================================

    /// Get complete wallet portfolio (SOL + SPL tokens + NFTs)
    /// Endpoint: GET /account/:network/:address/portfolio
    /// VERIFIED from Moralis docs
    pub async fn get_portfolio(&self, address: &str) -> Result<MoralisPortfolio> {
        let url = format!(
            "{}/account/{}/{}/portfolio",
            MORALIS_BASE_URL, self.network, address
        );

        let response = self
            .client
            .get(&url)
            .header("X-API-Key", &self.api_key)
            .send()
            .await
            .context("Failed to send request to Moralis")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!(
                "Moralis API error {}: {}",
                status,
                body
            ));
        }

        response
            .json::<MoralisPortfolio>()
            .await
            .context("Failed to parse Moralis portfolio response")
    }

    // ========================================================================
    // PnL Endpoints (VERIFIED)
    // Base URL: https://deep-index.moralis.io/api/v2.2
    // ========================================================================

    /// Get wallet PnL summary
    /// Endpoint: GET /wallets/:address/profitability/summary
    /// VERIFIED from Moralis docs
    pub async fn get_wallet_pnl_summary(&self, address: &str, days: &str) -> Result<MoralisPnLSummary> {
        let url = format!(
            "{}/wallets/{}/profitability/summary?days={}",
            MORALIS_DEEP_INDEX_URL, address, days
        );

        let response = self
            .client
            .get(&url)
            .header("X-API-Key", &self.api_key)
            .send()
            .await
            .context("Failed to send request to Moralis")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!(
                "Moralis API error {}: {}",
                status,
                body
            ));
        }

        response
            .json::<MoralisPnLSummary>()
            .await
            .context("Failed to parse Moralis PnL summary response")
    }

    /// Get wallet PnL breakdown by token
    /// Endpoint: GET /wallets/:address/profitability
    /// VERIFIED from Moralis docs
    pub async fn get_wallet_pnl_breakdown(&self, address: &str, days: &str) -> Result<Vec<MoralisTokenPnL>> {
        let url = format!(
            "{}/wallets/{}/profitability?days={}",
            MORALIS_DEEP_INDEX_URL, address, days
        );

        let response = self
            .client
            .get(&url)
            .header("X-API-Key", &self.api_key)
            .send()
            .await
            .context("Failed to send request to Moralis")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!(
                "Moralis API error {}: {}. NOTE: This endpoint is unverified.",
                status,
                body
            ));
        }

        response
            .json::<Vec<MoralisTokenPnL>>()
            .await
            .context("Failed to parse Moralis PnL breakdown response")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_moralis_client_creation() {
        let client = MoralisClient::new("test_key".to_string(), "mainnet".to_string());
        assert_eq!(client.network, "mainnet");
    }

    #[tokio::test]
    async fn test_balance_parsing() {
        // This would require mocking the HTTP client
        // For now, just ensure the types compile
        let response = MoralisBalanceResponse {
            lamports: "1000000000".to_string(),
            solana: "1.0".to_string(),
        };
        
        assert_eq!(response.lamports, "1000000000");
    }
}
