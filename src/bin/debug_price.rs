use anyhow::{Result, Context};
use reqwest::Client;
use std::env;
use std::sync::Arc;
use tokio::sync::RwLock;
use std::time::{Instant, Duration};
use dotenv::dotenv;

// Replicate structs from position_tracker
#[derive(Debug, Clone)]
pub struct SolCache {
    pub price: f64,
    pub timestamp: Instant,
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenv().ok();
    
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        println!("Usage: cargo run --bin debug_price <TOKEN_MINT>");
        return Ok(());
    }
    
    let mint = &args[1];
    let birdeye_key = env::var("BIRDEYE_API_KEY").context("BIRDEYE_API_KEY not set")?;
    
    let client = Client::builder()
        .timeout(Duration::from_secs(10))
        .build()?;
        
    let sol_cache = Arc::new(RwLock::new(None));
    
    println!("🔍 Debugging Price for: {}", mint);
    println!("--------------------------------------------------");

    // Replicate production logic exactly
    let start = Instant::now();
    
    let birdeye_task = fetch_price_from_birdeye(&client, &birdeye_key, mint, sol_cache.clone());
    let jupiter_task = fetch_price_from_jupiter(&client, mint);

    println!("📡 Fetching from Jupiter and Birdeye concurrently...");
    let (birdeye_res, jupiter_res) = tokio::join!(birdeye_task, jupiter_task);

    println!("\n📊 RESULTS (in SOL per token):");
    
    match jupiter_res {
        Ok(p) => println!("✅ Jupiter: {:.10} SOL", p),
        Err(e) => println!("❌ Jupiter Failed: {}", e),
    }

    match birdeye_res {
        Ok(p) => println!("✅ Birdeye: {:.10} SOL", p),
        Err(e) => println!("❌ Birdeye Failed: {}", e),
    }

    let duration = start.elapsed();
    println!("\n⏱️ Total time: {:?}", duration);
    
    Ok(())
}

async fn fetch_price_from_jupiter(client: &Client, mint: &str) -> Result<f64> {
    let sol_mint = "So11111111111111111111111111111111111111112";
    let url = format!("https://api.jup.ag/price/v2/full?ids={}&vsToken={}", mint, sol_mint);
    
    let resp = client.get(&url)
        .timeout(Duration::from_secs(5))
        .send().await?;
        
    let json: serde_json::Value = resp.json().await?;
    
    if let Some(data) = json.get("data").and_then(|d| d.get(mint)) {
        if let Some(price_str) = data.get("price").and_then(|p| p.as_str()) {
            let price = price_str.parse::<f64>()?;
            if price > 0.0 {
                return Ok(price);
            }
        }
    }
    
    Err(anyhow::anyhow!("Jupiter price not found"))
}

async fn fetch_price_from_birdeye(
    client: &reqwest::Client, 
    api_key: &str, 
    mint: &str,
    _sol_cache: Arc<RwLock<Option<SolCache>>>,
) -> Result<f64> {
    let url = format!("https://public-api.birdeye.so/defi/price?address={}", mint);
    
    let resp = client.get(&url)
        .header("X-API-KEY", api_key)
        .header("x-chain", "solana")
        .header("accept", "application/json") // User's requested header
        .send().await?;
        
    let json: serde_json::Value = resp.json().await?;
    
    if let Some(price_in_native) = json.get("data").and_then(|d| d.get("priceInNative")).and_then(|v| v.as_f64()) {
        if price_in_native > 0.0 {
            return Ok(price_in_native);
        }
    }
    
    // Fallback if priceInNative is missing (shouldn't happen with the correct endpoint)
    Err(anyhow::anyhow!("Birdeye priceInNative not found for {}", mint))
}

async fn fetch_sol_price(
    client: &reqwest::Client, 
    api_key: &str,
    sol_cache: Arc<RwLock<Option<SolCache>>>,
) -> Option<f64> {
    let sol_mint = "So11111111111111111111111111111111111111112";
    let url = format!("https://public-api.birdeye.so/defi/price?address={}", sol_mint);
    
    let fetch_result = async {
        let resp = client.get(&url)
            .header("X-API-KEY", api_key)
            .header("x-chain", "solana")
            .timeout(Duration::from_secs(3)) 
            .send().await?;
            
        let json = resp.json::<serde_json::Value>().await?;
        let price = json.get("data").and_then(|d| d.get("value")).and_then(|v| v.as_f64())
            .ok_or_else(|| anyhow::anyhow!("No price in JSON"))?;
            
        Ok::<f64, anyhow::Error>(price)
    }.await;

    match fetch_result {
        Ok(price) => {
            let mut cache = sol_cache.write().await;
            *cache = Some(SolCache {
                price,
                timestamp: Instant::now(),
            });
            Some(price)
        }
        Err(_) => {
            let cache = sol_cache.read().await;
            if let Some(cached) = &*cache {
                if cached.timestamp.elapsed().as_secs() < 60 {
                    return Some(cached.price);
                }
            }
            None
        }
    }
}
