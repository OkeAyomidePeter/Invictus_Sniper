use crate::helius_listener::{ClassifiedEvent, MintEvent, PoolCreationEvent};
use anyhow::{Context, Result};
use log::{debug, error, info, warn};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

/// Enriched token data with all metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnrichedToken {
    // Core identification
    pub mint: String,
    pub signature: String,
    pub slot: u64,
    pub timestamp: Option<i64>,
    
    // Token metadata
    pub symbol: Option<String>,
    pub name: Option<String>,
    pub decimals: Option<u8>,
    pub supply: Option<u64>,
    
    // Ownership data
    pub holders: Option<u64>,
    pub creator: Option<String>,
    pub freeze_authority: Option<String>,
    pub mint_authority: Option<String>,
    
    // Liquidity data
    pub has_liquidity: bool,
    pub pool_info: Option<PoolInfo>,
    
    // Market data
    pub fdv: Option<f64>,           // Fully Diluted Valuation
    pub market_cap: Option<f64>,
    pub price_usd: Option<f64>,
    
    // Trading costs
    pub buy_tax: Option<f64>,       // As percentage (e.g., 5.0 = 5%)
    pub sell_tax: Option<f64>,      // As percentage
    
    // Metadata
    pub metadata_uri: Option<String>,
    pub image_uri: Option<String>,
    pub social_links: SocialLinks,
    
    // Platform info
    pub platform: String,           // "Pump.fun", "Raydium", etc.
    pub dex: Option<String>,        // DEX where liquidity exists
    
    // Risk flags
    pub is_mutable: bool,
    pub has_freeze_authority: bool,
    pub has_mint_authority: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PoolInfo {
    pub pool_address: String,
    pub dex: String,
    pub base_reserve: f64,      // Token reserve
    pub quote_reserve: f64,     // SOL/USDC reserve
    pub pair_token: String,     // Usually SOL
    pub liquidity_usd: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SocialLinks {
    pub website: Option<String>,
    pub twitter: Option<String>,
    pub telegram: Option<String>,
    pub discord: Option<String>,
}

/// Cache entry with TTL
#[derive(Debug, Clone)]
struct CacheEntry<T> {
    data: T,
    cached_at: Instant,
}

impl<T> CacheEntry<T> {
    fn new(data: T) -> Self {
        Self {
            data,
            cached_at: Instant::now(),
        }
    }
    
    fn is_valid(&self, ttl: Duration) -> bool {
        self.cached_at.elapsed() < ttl
    }
}

/// Token metadata cache
#[derive(Debug, Clone)]
struct TokenMetadataCache {
    symbol: Option<String>,
    name: Option<String>,
    uri: Option<String>,
    image: Option<String>,
    creator: Option<String>,
    is_mutable: bool,
}

/// Account info cache
#[derive(Debug, Clone)]
struct AccountInfoCache {
    freeze_authority: Option<String>,
    mint_authority: Option<String>,
    decimals: Option<u8>,
}

#[derive(Clone)]
pub struct TokenEnricher {
    client: reqwest::Client,
    helius_api_key: String,
    rpc_url: String,
    
    // Caches with 5-minute TTL
    metadata_cache: Arc<RwLock<HashMap<String, CacheEntry<TokenMetadataCache>>>>,
    account_info_cache: Arc<RwLock<HashMap<String, CacheEntry<AccountInfoCache>>>>,
    social_links_cache: Arc<RwLock<HashMap<String, CacheEntry<SocialLinks>>>>,
    holder_count_cache: Arc<RwLock<HashMap<String, CacheEntry<u64>>>>,
    
    cache_ttl: Duration,
}

impl TokenEnricher {
    pub fn new(helius_api_key: String) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .expect("Failed to create HTTP client");
        
        let rpc_url = format!("https://mainnet.helius-rpc.com/?api-key={}", helius_api_key);
        
        Self {
            client,
            helius_api_key,
            rpc_url,
            metadata_cache: Arc::new(RwLock::new(HashMap::new())),
            account_info_cache: Arc::new(RwLock::new(HashMap::new())),
            social_links_cache: Arc::new(RwLock::new(HashMap::new())),
            holder_count_cache: Arc::new(RwLock::new(HashMap::new())),
            cache_ttl: Duration::from_secs(300), // 5 minutes
        }
    }
    
    /// Enrich a classified event with full token metadata
    pub async fn enrich_event(&self, event: &ClassifiedEvent) -> Result<EnrichedToken> {
        match event {
            ClassifiedEvent::Mint(mint_event) => {
                self.enrich_mint_event(mint_event, None).await
            }
            ClassifiedEvent::PoolCreation(pool_event) => {
                self.enrich_pool_event(pool_event).await
            }
        }
    }
    
    /// Enrich a mint event
    async fn enrich_mint_event(
        &self,
        mint_event: &MintEvent,
        pool_info: Option<PoolInfo>,
    ) -> Result<EnrichedToken> {
        info!("🔍 Enriching mint: {}", mint_event.mint);
        
        // Fetch all data in parallel for speed
        let (metadata, account_info, holder_count, social_links) = tokio::join!(
            self.get_or_fetch_metadata(&mint_event.mint),
            self.get_or_fetch_account_info(&mint_event.mint),
            self.get_or_fetch_holder_count(&mint_event.mint),
            self.get_or_fetch_social_links(&mint_event.mint),
        );
        
        let metadata = metadata.ok();
        let account_info = account_info.ok();
        let holder_count = holder_count.ok();
        let social_links = social_links.unwrap_or_default();
        
        // Extract authorities from account info
        let (freeze_authority, mint_authority, decimals_from_cache) = if let Some(ref info) = account_info {
            (
                info.freeze_authority.clone(),
                info.mint_authority.clone(),
                info.decimals,
            )
        } else {
            (None, None, None)
        };
        
        // Use cached decimals if available, otherwise from event
        let decimals = mint_event.decimals.or(decimals_from_cache);
        
        // Calculate market metrics if we have pool info
        let (fdv, market_cap, price_usd) = if let Some(ref pool) = pool_info {
            self.calculate_market_metrics(
                mint_event.supply,
                decimals,
                pool.base_reserve,
                pool.quote_reserve,
            )
        } else {
            (None, None, None)
        };
        
        // Simulate buy/sell tax
        let (buy_tax, sell_tax) = if let Some(ref pool) = pool_info {
            self.simulate_trade_taxes(&mint_event.mint, &pool.pool_address, &pool.pair_token).await
        } else {
            (None, None)
        };
        
        Ok(EnrichedToken {
            mint: mint_event.mint.clone(),
            signature: mint_event.signature.clone(),
            slot: mint_event.slot,
            timestamp: mint_event.timestamp,
            symbol: metadata.as_ref().and_then(|m| m.symbol.clone()),
            name: metadata.as_ref().and_then(|m| m.name.clone()),
            decimals,
            supply: mint_event.supply,
            holders: holder_count,
            creator: metadata.as_ref().and_then(|m| m.creator.clone()),
            freeze_authority: freeze_authority.clone(),
            mint_authority: mint_authority.clone(),
            has_liquidity: pool_info.is_some(),
            pool_info,
            fdv,
            market_cap,
            price_usd,
            buy_tax,
            sell_tax,
            metadata_uri: metadata.as_ref().and_then(|m| m.uri.clone()),
            image_uri: metadata.as_ref().and_then(|m| m.image.clone()),
            social_links,
            platform: mint_event.platform.clone(),
            dex: None,
            is_mutable: metadata.as_ref().map(|m| m.is_mutable).unwrap_or(false),
            has_freeze_authority: freeze_authority.is_some(),
            has_mint_authority: mint_authority.is_some(),
        })
    }
    
    /// Enrich a pool creation event
    async fn enrich_pool_event(&self, pool_event: &PoolCreationEvent) -> Result<EnrichedToken> {
        info!("🔍 Enriching pool creation for token: {}", pool_event.token_mint);
        
        // Fetch pool reserves
        let pool_info = self.fetch_pool_reserves(pool_event).await?;
        
        // Create a temporary mint event to reuse enrichment logic
        let temp_mint = MintEvent {
            mint: pool_event.token_mint.clone(),
            signature: pool_event.signature.clone(),
            slot: pool_event.slot,
            timestamp: pool_event.timestamp,
            platform: "Unknown".to_string(),
            decimals: None,
            supply: None,
        };
        
        let mut enriched = self.enrich_mint_event(&temp_mint, Some(pool_info)).await?;
        enriched.dex = Some(pool_event.dex.clone());
        
        Ok(enriched)
    }
    
    // ========== CACHED METADATA METHODS ==========
    
    /// Get metadata from cache or fetch if not cached
    async fn get_or_fetch_metadata(&self, mint: &str) -> Result<TokenMetadataCache> {
        // Check cache first
        {
            let cache = self.metadata_cache.read().await;
            if let Some(entry) = cache.get(mint) {
                if entry.is_valid(self.cache_ttl) {
                    debug!("💾 Cache hit: metadata for {}", mint);
                    return Ok(entry.data.clone());
                }
            }
        }
        
        // Cache miss or expired, fetch new data
        debug!("🌐 Cache miss: fetching metadata for {}", mint);
        let metadata = self.fetch_token_metadata(mint).await?;
        
        // Update cache
        {
            let mut cache = self.metadata_cache.write().await;
            cache.insert(mint.to_string(), CacheEntry::new(metadata.clone()));
        }
        
        Ok(metadata)
    }
    
    /// Get account info from cache or fetch if not cached
    async fn get_or_fetch_account_info(&self, mint: &str) -> Result<AccountInfoCache> {
        // Check cache first
        {
            let cache = self.account_info_cache.read().await;
            if let Some(entry) = cache.get(mint) {
                if entry.is_valid(self.cache_ttl) {
                    debug!("💾 Cache hit: account info for {}", mint);
                    return Ok(entry.data.clone());
                }
            }
        }
        
        // Cache miss or expired, fetch new data
        debug!("🌐 Cache miss: fetching account info for {}", mint);
        let info = self.fetch_account_info(mint).await?;
        
        let account_cache = AccountInfoCache {
            freeze_authority: info.get("freezeAuthority").and_then(|v| v.as_str()).map(String::from),
            mint_authority: info.get("mintAuthority").and_then(|v| v.as_str()).map(String::from),
            decimals: info.get("decimals").and_then(|v| v.as_u64()).map(|d| d as u8),
        };
        
        // Update cache
        {
            let mut cache = self.account_info_cache.write().await;
            cache.insert(mint.to_string(), CacheEntry::new(account_cache.clone()));
        }
        
        Ok(account_cache)
    }
    
    /// Get social links from cache or fetch if not cached
    async fn get_or_fetch_social_links(&self, mint: &str) -> Result<SocialLinks> {
        // Check cache first
        {
            let cache = self.social_links_cache.read().await;
            if let Some(entry) = cache.get(mint) {
                if entry.is_valid(self.cache_ttl) {
                    debug!("💾 Cache hit: social links for {}", mint);
                    return Ok(entry.data.clone());
                }
            }
        }
        
        // Cache miss or expired, fetch new data
        debug!("🌐 Cache miss: fetching social links for {}", mint);
        let links = self.fetch_social_links(mint).await?;
        
        // Update cache
        {
            let mut cache = self.social_links_cache.write().await;
            cache.insert(mint.to_string(), CacheEntry::new(links.clone()));
        }
        
        Ok(links)
    }
    
    /// Get holder count from cache or fetch if not cached
    async fn get_or_fetch_holder_count(&self, mint: &str) -> Result<u64> {
        // Check cache first
        {
            let cache = self.holder_count_cache.read().await;
            if let Some(entry) = cache.get(mint) {
                if entry.is_valid(self.cache_ttl) {
                    debug!("💾 Cache hit: holder count for {}", mint);
                    return Ok(entry.data);
                }
            }
        }
        
        // Cache miss or expired, fetch new data
        debug!("🌐 Cache miss: fetching holder count for {}", mint);
        let count = self.fetch_holder_count(mint).await?.unwrap_or(0);
        
        // Update cache
        {
            let mut cache = self.holder_count_cache.write().await;
            cache.insert(mint.to_string(), CacheEntry::new(count));
        }
        
        Ok(count)
    }
    
    // ========== FETCH METHODS ==========
    
    /// Fetch token metadata from Helius DAS API
    async fn fetch_token_metadata(&self, mint: &str) -> Result<TokenMetadataCache> {
        let request_body = json!({
            "jsonrpc": "2.0",
            "id": "metadata-fetch",
            "method": "getAsset",
            "params": {
                "id": mint,
                "displayOptions": {
                    "showCollectionMetadata": true
                }
            }
        });
        
        let response = self.client
            .post(&self.rpc_url)
            .json(&request_body)
            .send()
            .await?;
        
        let result: serde_json::Value = response.json().await?;
        
        // Parse the response
        let symbol = result.get("result")
            .and_then(|r| r.get("content"))
            .and_then(|c| c.get("metadata"))
            .and_then(|m| m.get("symbol"))
            .and_then(|s| s.as_str())
            .map(String::from);
        
        let name = result.get("result")
            .and_then(|r| r.get("content"))
            .and_then(|c| c.get("metadata"))
            .and_then(|m| m.get("name"))
            .and_then(|s| s.as_str())
            .map(String::from);
        
        let uri = result.get("result")
            .and_then(|r| r.get("content"))
            .and_then(|c| c.get("json_uri"))
            .and_then(|s| s.as_str())
            .map(String::from);
        
        let image = result.get("result")
            .and_then(|r| r.get("content"))
            .and_then(|c| c.get("files"))
            .and_then(|f| f.as_array())
            .and_then(|arr| arr.first())
            .and_then(|f| f.get("uri"))
            .and_then(|s| s.as_str())
            .map(String::from);
        
        let creator = result.get("result")
            .and_then(|r| r.get("creators"))
            .and_then(|c| c.as_array())
            .and_then(|arr| arr.first())
            .and_then(|c| c.get("address"))
            .and_then(|s| s.as_str())
            .map(String::from);
        
        let is_mutable = result.get("result")
            .and_then(|r| r.get("mutable"))
            .and_then(|m| m.as_bool())
            .unwrap_or(true);
        
        Ok(TokenMetadataCache {
            symbol,
            name,
            uri,
            image,
            creator,
            is_mutable,
        })
    }
    
    /// Fetch account info (authorities, decimals, etc.)
    async fn fetch_account_info(&self, mint: &str) -> Result<serde_json::Value> {
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
        
        let response = self.client
            .post(&self.rpc_url)
            .json(&request_body)
            .send()
            .await?;
        
        let result: serde_json::Value = response.json().await?;
        
        result.get("result")
            .and_then(|r| r.get("value"))
            .and_then(|v| v.get("data"))
            .and_then(|d| d.get("parsed"))
            .and_then(|p| p.get("info"))
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Failed to parse account info"))
    }
    
    /// Fetch holder count using token accounts
    async fn fetch_holder_count(&self, mint: &str) -> Result<Option<u64>> {
        let request_body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "getTokenLargestAccounts",
            "params": [mint]
        });
        
        let response = self.client
            .post(&self.rpc_url)
            .json(&request_body)
            .send()
            .await?;
        
        let result: serde_json::Value = response.json().await?;
        
        // This gives us largest accounts, not total count
        // For exact count, we'd need to use a specialized indexer
        let accounts = result.get("result")
            .and_then(|r| r.get("value"))
            .and_then(|v| v.as_array())
            .map(|arr| arr.len() as u64);
        
        Ok(accounts)
    }
    
    /// Fetch social links from metadata URI
    async fn fetch_social_links(&self, mint: &str) -> Result<SocialLinks> {
        // First get the metadata URI
        let metadata = self.fetch_token_metadata(mint).await?;
        
        if let Some(uri) = metadata.uri {
            // Fetch the actual JSON metadata
            match self.client.get(&uri).send().await {
                Ok(response) => {
                    if let Ok(json) = response.json::<serde_json::Value>().await {
                        let website = json.get("external_url")
                            .and_then(|s| s.as_str())
                            .map(String::from);
                        
                        let twitter = json.get("twitter")
                            .or_else(|| json.get("properties").and_then(|p| p.get("twitter")))
                            .and_then(|s| s.as_str())
                            .map(String::from);
                        
                        let telegram = json.get("telegram")
                            .or_else(|| json.get("properties").and_then(|p| p.get("telegram")))
                            .and_then(|s| s.as_str())
                            .map(String::from);
                        
                        let discord = json.get("discord")
                            .or_else(|| json.get("properties").and_then(|p| p.get("discord")))
                            .and_then(|s| s.as_str())
                            .map(String::from);
                        
                        return Ok(SocialLinks {
                            website,
                            twitter,
                            telegram,
                            discord,
                        });
                    }
                }
                Err(e) => {
                    debug!("Failed to fetch metadata JSON: {}", e);
                }
            }
        }
        
        Ok(SocialLinks::default())
    }
    
    /// Fetch pool reserves and liquidity data
    async fn fetch_pool_reserves(&self, pool_event: &PoolCreationEvent) -> Result<PoolInfo> {
        // Get token balances for the pool
        let token_accounts = self.fetch_pool_token_accounts(&pool_event.pool_address).await?;
        
        let (base_reserve, quote_reserve) = self.extract_reserves(&token_accounts, pool_event)?;
        
        // Calculate liquidity in USD (assuming SOL price, or fetch from oracle)
        let sol_price = self.fetch_sol_price().await.unwrap_or(0.0);
        let liquidity_usd = if pool_event.pair_token.contains("So11111111111111111111111111111111111111112") {
            Some(quote_reserve * sol_price)
        } else {
            Some(quote_reserve) // Assume USDC
        };
        
        Ok(PoolInfo {
            pool_address: pool_event.pool_address.clone(),
            dex: pool_event.dex.clone(),
            base_reserve,
            quote_reserve,
            pair_token: pool_event.pair_token.clone(),
            liquidity_usd,
        })
    }
    
    /// Fetch token accounts for a pool
    async fn fetch_pool_token_accounts(&self, pool_address: &str) -> Result<Vec<serde_json::Value>> {
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
        
        let response = self.client
            .post(&self.rpc_url)
            .json(&request_body)
            .send()
            .await?;
        
        let result: serde_json::Value = response.json().await?;
        
        let accounts = result.get("result")
            .and_then(|r| r.get("value"))
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        
        Ok(accounts)
    }
    
    /// Extract reserves from token accounts
    fn extract_reserves(
        &self,
        token_accounts: &[serde_json::Value],
        pool_event: &PoolCreationEvent,
    ) -> Result<(f64, f64)> {
        let mut base_reserve = 0.0;
        let mut quote_reserve = 0.0;
        
        for account in token_accounts {
            if let Some(mint) = account.get("account")
                .and_then(|a| a.get("data"))
                .and_then(|d| d.get("parsed"))
                .and_then(|p| p.get("info"))
                .and_then(|i| i.get("mint"))
                .and_then(|m| m.as_str()) {
                
                let amount = account.get("account")
                    .and_then(|a| a.get("data"))
                    .and_then(|d| d.get("parsed"))
                    .and_then(|p| p.get("info"))
                    .and_then(|i| i.get("tokenAmount"))
                    .and_then(|t| t.get("uiAmount"))
                    .and_then(|a| a.as_f64())
                    .unwrap_or(0.0);
                
                if mint == pool_event.token_mint {
                    base_reserve = amount;
                } else {
                    quote_reserve = amount;
                }
            }
        }
        
        Ok((base_reserve, quote_reserve))
    }
    
    /// Calculate market metrics (FDV, Market Cap, Price)
    fn calculate_market_metrics(
        &self,
        supply: Option<u64>,
        decimals: Option<u8>,
        base_reserve: f64,
        quote_reserve: f64,
    ) -> (Option<f64>, Option<f64>, Option<f64>) {
        if base_reserve == 0.0 {
            return (None, None, None);
        }
        
        // Price = quote_reserve / base_reserve
        let price_usd = quote_reserve / base_reserve;
        
        if let (Some(supply), Some(decimals)) = (supply, decimals) {
            let total_supply = supply as f64 / 10_f64.powi(decimals as i32);
            let fdv = total_supply * price_usd;
            let market_cap = Some(fdv); // Simplified: assume all tokens in circulation
            
            (Some(fdv), market_cap, Some(price_usd))
        } else {
            (None, None, Some(price_usd))
        }
    }
    
    // ========== TAX SIMULATION WITH JUPITER ==========
    
    /// Simulate buy and sell to detect taxes/fees
    async fn simulate_trade_taxes(
        &self, 
        mint: &str, 
        pool_address: &str,
        pair_token: &str,
    ) -> (Option<f64>, Option<f64>) {
        info!("🧪 Simulating trade taxes for {}", mint);
        
        // Simulate buy (SOL -> Token)
        let buy_tax = self.simulate_jupiter_swap(
            pair_token,  // Input: SOL or USDC
            mint,        // Output: Token
            1_000_000,   // 0.001 SOL in lamports
        ).await.ok();
        
        // Simulate sell (Token -> SOL)
        let sell_tax = self.simulate_jupiter_swap(
            mint,        // Input: Token
            pair_token,  // Output: SOL or USDC
            1_000_000,   // 1 million base units of token
        ).await.ok();
        
        (buy_tax, sell_tax)
    }
    
    /// Simulate a Jupiter swap and calculate effective tax
    async fn simulate_jupiter_swap(
        &self,
        input_mint: &str,
        output_mint: &str,
        amount: u64,
    ) -> Result<f64> {
        // Step 1: Get Jupiter quote
        let quote = self.get_jupiter_quote(input_mint, output_mint, amount).await?;
        
        // Step 2: Get swap transaction
        let swap_tx = self.get_jupiter_swap_transaction(&quote).await?;
        
        // Step 3: Simulate the transaction
        let simulation_result = self.simulate_transaction(&swap_tx).await?;
        
        // Step 4: Calculate tax from simulation
        let tax = self.calculate_tax_from_simulation(&simulation_result, &quote)?;
        
        Ok(tax)
    }
    
    /// Get quote from Jupiter API
    async fn get_jupiter_quote(
        &self,
        input_mint: &str,
        output_mint: &str,
        amount: u64,
    ) -> Result<serde_json::Value> {
        let url = format!(
            "https://quote-api.jup.ag/v6/quote?inputMint={}&outputMint={}&amount={}&slippageBps=50",
            input_mint, output_mint, amount
        );
        
        let response = self.client
            .get(&url)
            .send()
            .await
            .context("Failed to get Jupiter quote")?;
        
        let quote: serde_json::Value = response
            .json()
            .await
            .context("Failed to parse Jupiter quote")?;
        
        if quote.get("error").is_some() {
            return Err(anyhow::anyhow!("Jupiter quote error: {:?}", quote.get("error")));
        }
        
        Ok(quote)
    }
    
    /// Get swap transaction from Jupiter API
    async fn get_jupiter_swap_transaction(&self, quote: &serde_json::Value) -> Result<String> {
        // Use a dummy wallet for simulation
        let dummy_wallet = "11111111111111111111111111111111";
        
        let request_body = json!({
            "quoteResponse": quote,
            "userPublicKey": dummy_wallet,
            "wrapAndUnwrapSol": true,
            "dynamicComputeUnitLimit": true,
            "prioritizationFeeLamports": "auto"
        });
        
        let response = self.client
            .post("https://quote-api.jup.ag/v6/swap")
            .json(&request_body)
            .send()
            .await
            .context("Failed to get Jupiter swap transaction")?;
        
        let result: serde_json::Value = response
            .json()
            .await
            .context("Failed to parse Jupiter swap response")?;
        
        let swap_transaction = result
            .get("swapTransaction")
            .and_then(|s| s.as_str())
            .ok_or_else(|| anyhow::anyhow!("No swap transaction in response"))?;
        
        Ok(swap_transaction.to_string())
    }
    
    /// Simulate transaction using Solana RPC
    async fn simulate_transaction(&self, transaction_base64: &str) -> Result<serde_json::Value> {
        let request_body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "simulateTransaction",
            "params": [
                transaction_base64,
                {
                    "encoding": "base64",
                    "commitment": "confirmed",
                    "replaceRecentBlockhash": true,
                    "sigVerify": false
                }
            ]
        });
        
        let response = self.client
            .post(&self.rpc_url)
            .json(&request_body)
            .send()
            .await
            .context("Failed to simulate transaction")?;
        
        let result: serde_json::Value = response
            .json()
            .await
            .context("Failed to parse simulation response")?;
        
        // Check for simulation errors
        if let Some(error) = result.get("error") {
            return Err(anyhow::anyhow!("Simulation error: {:?}", error));
        }
        
        result
            .get("result")
            .and_then(|r| r.get("value"))
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("No simulation result"))
    }
    
    /// Calculate tax percentage from simulation results
    fn calculate_tax_from_simulation(
        &self,
        simulation: &serde_json::Value,
        quote: &serde_json::Value,
    ) -> Result<f64> {
        // Check if simulation succeeded
        if let Some(err) = simulation.get("err") {
            if !err.is_null() {
                warn!("Simulation failed: {:?}", err);
                // If simulation fails, it might indicate 100% tax or blocked transfer
                return Ok(100.0);
            }
        }
        
        // Get expected output amount from quote
        let expected_out = quote
            .get("outAmount")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<u64>().ok())
            .ok_or_else(|| anyhow::anyhow!("No outAmount in quote"))?;
        
        // Get input amount from quote
        let input_amount = quote
            .get("inAmount")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<u64>().ok())
            .ok_or_else(|| anyhow::anyhow!("No inAmount in quote"))?;
        
        // Parse token balances from simulation to get actual output
        let post_balances = simulation
            .get("accounts")
            .and_then(|a| a.as_array());
        
        // If we have detailed balance info, use it
        // Otherwise fall back to comparing expected vs price impact
        if let Some(price_impact) = quote.get("priceImpactPct").and_then(|p| p.as_f64()) {
            // Calculate effective slippage/tax
            // Tax = (Expected - Actual) / Expected * 100
            // If price impact is already high, the token likely has fees
            let effective_tax = price_impact.abs();
            
            // Jupiter already accounts for normal slippage, so anything above 1% is suspicious
            if effective_tax > 1.0 {
                return Ok(effective_tax);
            }
        }
        
        // Check for transfer hooks or other restrictions in logs
        if let Some(logs) = simulation.get("logs").and_then(|l| l.as_array()) {
            for log in logs {
                if let Some(log_str) = log.as_str() {
                    // Check for common tax/fee indicators
                    if log_str.contains("Transfer fee") 
                        || log_str.contains("tax") 
                        || log_str.contains("fee") 
                        || log_str.contains("TransferHook") {
                        // Estimate tax from logs if possible
                        // This is heuristic-based
                        return Ok(5.0); // Conservative estimate
                    }
                }
            }
        }
        
        // If simulation passed with no obvious fees, return low tax
        Ok(0.0)
    }
    
    /// Fetch current SOL price (for liquidity calculations)
    async fn fetch_sol_price(&self) -> Result<f64> {
        // You can use Jupiter Price API or other oracle
        let response = self.client
            .get("https://price.jup.ag/v6/price?ids=SOL")
            .send()
            .await?;
        
        let result: serde_json::Value = response.json().await?;
        
        let price = result.get("data")
            .and_then(|d| d.get("SOL"))
            .and_then(|s| s.get("price"))
            .and_then(|p| p.as_f64())
            .unwrap_or(0.0);
        
        Ok(price)
    }
    
    // ========== CACHE MANAGEMENT ==========
    
    /// Clear expired entries from all caches
    pub async fn clean_expired_cache(&self) {
        let now = Instant::now();
        
        // Clean metadata cache
        {
            let mut cache = self.metadata_cache.write().await;
            cache.retain(|_, entry| entry.is_valid(self.cache_ttl));
        }
        
        // Clean account info cache
        {
            let mut cache = self.account_info_cache.write().await;
            cache.retain(|_, entry| entry.is_valid(self.cache_ttl));
        }
        
        // Clean social links cache
        {
            let mut cache = self.social_links_cache.write().await;
            cache.retain(|_, entry| entry.is_valid(self.cache_ttl));
        }
        
        // Clean holder count cache
        {
            let mut cache = self.holder_count_cache.write().await;
            cache.retain(|_, entry| entry.is_valid(self.cache_ttl));
        }
        
        debug!("🧹 Cache cleanup completed");
    }
    
    /// Get cache statistics
    pub async fn get_cache_stats(&self) -> CacheStats {
        let metadata_size = self.metadata_cache.read().await.len();
        let account_info_size = self.account_info_cache.read().await.len();
        let social_links_size = self.social_links_cache.read().await.len();
        let holder_count_size = self.holder_count_cache.read().await.len();
        
        CacheStats {
            metadata_entries: metadata_size,
            account_info_entries: account_info_size,
            social_links_entries: social_links_size,
            holder_count_entries: holder_count_size,
            total_entries: metadata_size + account_info_size + social_links_size + holder_count_size,
        }
    }
    
    /// Clear all caches (useful for testing or manual refresh)
    pub async fn clear_all_caches(&self) {
        self.metadata_cache.write().await.clear();
        self.account_info_cache.write().await.clear();
        self.social_links_cache.write().await.clear();
        self.holder_count_cache.write().await.clear();
        info!("🗑️  All caches cleared");
    }
}

#[derive(Debug, Clone)]
pub struct CacheStats {
    pub metadata_entries: usize,
    pub account_info_entries: usize,
    pub social_links_entries: usize,
    pub holder_count_entries: usize,
    pub total_entries: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[tokio::test]
    async fn test_cache_expiry() {
        let enricher = TokenEnricher::new("test_key".to_string());
        
        // Insert a test entry
        {
            let mut cache = enricher.metadata_cache.write().await;
            let test_data = TokenMetadataCache {
                symbol: Some("TEST".to_string()),
                name: Some("Test Token".to_string()),
                uri: None,
                image: None,
                creator: None,
                is_mutable: false,
            };
            cache.insert("test_mint".to_string(), CacheEntry::new(test_data));
        }
        
        // Check cache size
        let stats = enricher.get_cache_stats().await;
        assert_eq!(stats.metadata_entries, 1);
        
        // Clear cache
        enricher.clear_all_caches().await;
        let stats = enricher.get_cache_stats().await;
        assert_eq!(stats.total_entries, 0);
    }
    
    #[tokio::test]
    async fn test_cache_ttl() {
        let mut enricher = TokenEnricher::new("test_key".to_string());
        enricher.cache_ttl = Duration::from_millis(100); // Very short TTL for testing
        
        {
            let mut cache = enricher.metadata_cache.write().await;
            let test_data = TokenMetadataCache {
                symbol: Some("TEST".to_string()),
                name: Some("Test Token".to_string()),
                uri: None,
                image: None,
                creator: None,
                is_mutable: false,
            };
            cache.insert("test_mint".to_string(), CacheEntry::new(test_data));
        }
        
        // Wait for expiry
        tokio::time::sleep(Duration::from_millis(150)).await;
        
        // Clean expired
        enricher.clean_expired_cache().await;
        
        // Should be empty now
        let stats = enricher.get_cache_stats().await;
        assert_eq!(stats.metadata_entries, 0);
    }
}