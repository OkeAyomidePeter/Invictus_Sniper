use dotenv::dotenv;
use reqwest::Client;
use std::env;
use std::time::Duration;
use anyhow::{Result, Context};
use serde_json::Value;

#[tokio::main]
async fn main() -> Result<()> {
    dotenv().ok();
    let client = Client::builder()
        .timeout(Duration::from_secs(15))
        .build()?;

    let birdeye_key = env::var("BIRDEYE_API_KEY").context("BIRDEYE_API_KEY not set")?;
    let jupiter_key = env::var("JUPITER_API_KEY").context("JUPITER_API_KEY not set")?;
    let moralis_key = env::var("MORALIS_API_KEY").context("MORALIS_API_KEY not set")?;

    // Use USDC for first stability test
    let test_mint = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v"; 
    
    println!("🔍 Testing Price Endpoints for mint: {} (USDC)", test_mint);
    println!("--------------------------------------------------");

    // 1. Birdeye
    println!("🟢 Testing Birdeye...");
    match test_birdeye(&client, &birdeye_key, test_mint).await {
        Ok(price) => println!("   ✅ Birdeye: {:.12} SOL per token", price),
        Err(e) => println!("   ❌ Birdeye failed: {}", e),
    }

    // 2. Jupiter Price V1 (V1 is more stable for SOL prices)
    println!("🟢 Testing Jupiter V1...");
    match test_jupiter_v1(&client, test_mint).await {
        Ok(price) => println!("   ✅ Jupiter V1: {:.12} SOL per token", price),
        Err(e) => println!("   ❌ Jupiter V1 failed: {}", e),
    }

    // 3. DexScreener (Native Price)
    println!("🟢 Testing DexScreener (priceNative)...");
    match test_dexscreener_native(&client, test_mint).await {
        Ok(price) => println!("   ✅ DexScreener: {:.12} SOL per token", price),
        Err(e) => println!("   ❌ DexScreener failed: {}", e),
    }

    // 4. Moralis
    println!("🟢 Testing Moralis...");
    match test_moralis(&client, &moralis_key, test_mint).await {
        Ok(price) => println!("   ✅ Moralis: {:.12} SOL per token", price),
        Err(e) => println!("   ❌ Moralis failed: {}", e),
    }

    Ok(())
}

async fn test_birdeye(client: &Client, key: &str, mint: &str) -> Result<f64> {
    let url = format!("https://public-api.birdeye.so/defi/price?address={}", mint);
    let resp = client.get(&url)
        .header("X-API-KEY", key)
        .header("x-chain", "solana")
        .send().await?;
    
    let json: Value = resp.json().await?;
    println!("   DEBUG Birdeye JSON: {}", json);
    
    if let Some(price) = json["data"]["priceInNative"].as_f64() {
        return Ok(price);
    }
    
    Err(anyhow::anyhow!("Missing priceInNative in Birdeye. Response: {}", json))
}

async fn test_jupiter_v1(client: &Client, mint: &str) -> Result<f64> {
    let url = format!("https://price.jup.ag/v1/price?id={}&vsToken=SOL", mint);
    let resp = client.get(&url).send().await?;
    let json: Value = resp.json().await?;
    println!("   DEBUG Jupiter V1 JSON: {}", json);
    
    if let Some(price) = json["data"][mint]["price"].as_f64() {
        return Ok(price);
    }
    
    Err(anyhow::anyhow!("Missing price in Jupiter V1. Response: {}", json))
}

async fn test_dexscreener_native(client: &Client, mint: &str) -> Result<f64> {
    let url = format!("https://api.dexscreener.com/latest/dex/tokens/{}", mint);
    let resp = client.get(&url).send().await?;
    let json: Value = resp.json().await?;
    println!("   DEBUG DexScreener JSON: {}", json);
    
    if let Some(pairs) = json["pairs"].as_array() {
        if !pairs.is_empty() {
            if let Some(price_str) = pairs[0]["priceNative"].as_str() {
                return price_str.parse::<f64>().context("Failed to parse DexScreener native price");
            }
        }
    }
    
    Err(anyhow::anyhow!("Missing priceNative in DexScreener. Response: {}", json))
}

async fn test_moralis(client: &Client, key: &str, mint: &str) -> Result<f64> {
    let url = format!("https://solana-gateway.moralis.io/token/mainnet/{}/price", mint);
    let resp = client.get(&url)
        .header("X-API-KEY", key)
        .send().await?;
    
    let json: Value = resp.json().await?;
    println!("   DEBUG Moralis JSON: {}", json);
    
    if let Some(value_str) = json["nativePrice"]["value"].as_str() {
        let decimals = json["nativePrice"]["decimals"].as_u64().unwrap_or(9);
        let value = value_str.parse::<f64>().context("Failed to parse Moralis value")?;
        return Ok(value / 10f64.powf(decimals as f64));
    }
    
    Err(anyhow::anyhow!("Missing nativePrice.value in Moralis. Response: {}", json))
}
