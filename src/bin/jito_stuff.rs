use anyhow::Result;
use reqwest::Client;
use serde_json::json;
use solana_sdk::pubkey::Pubkey;
use std::str::FromStr;
use tokio::time::{sleep, Duration};

#[tokio::main]
async fn main() -> Result<()> {
    println!("🔍 Testing Jito Tip Account Fetching...");
    
    let endpoints = [
        ("Frankfurt", "https://frankfurt.mainnet.block-engine.jito.wtf/api/v1/bundles"),
        ("Global", "https://mainnet.block-engine.jito.wtf/api/v1/bundles"),
        ("NY", "https://ny.mainnet.block-engine.jito.wtf/api/v1/bundles"),
    ];

    let client = Client::builder()
        .timeout(Duration::from_secs(10))
        .build()?;
    
    let request = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "getTipAccounts",
        "params": []
    });

    let max_retries = 3;

    for (name, url) in endpoints {
        println!("\n� Testing Endpoint: {} ({})", name, url);
        
        for attempt in 1..=max_retries {
            println!("   🚀 Attempt {}/{}...", attempt, max_retries);

            match client.post(url)
                .json(&request)
                .send()
                .await 
            {
                Ok(response) => {
                    println!("   ✅ Response status: {}", response.status());
                    
                    if response.status().is_success() {
                        let response_json: serde_json::Value = response.json().await?;
                        
                        if let Some(result) = response_json.get("result") {
                            if let Some(accounts) = result.as_array() {
                                let mut valid_count = 0;
                
                                println!("   📋 Accounts received:");
                                for (i, val) in accounts.iter().enumerate() {
                                    if let Some(acc_str) = val.as_str() {
                                        match Pubkey::from_str(acc_str) {
                                            Ok(_) => {
                                                println!("      {}. [VALID] {}", i+1, acc_str);
                                                valid_count += 1;
                                            },
                                            Err(_) => println!("      {}. [INVALID] {}", i+1, acc_str),
                                        }
                                    }
                                }
                                
                                println!("\n✅ SUCCESS: Fetched {} valid tip accounts from {}.", valid_count, name);
                                return Ok(()); // Exit on first success
                            }
                        }
                    } else {
                         println!("   ❌ Server returned error status: {}", response.status());
                    }
                }
                Err(e) => {
                    println!("   ❌ Request failed: {}", e);
                }
            }

            if attempt < max_retries {
                let backoff = Duration::from_secs(2u64.pow(attempt - 1));
                println!("   ⚠️ Waiting {}s before retry...", backoff.as_secs());
                sleep(backoff).await;
            }
        }
    }

    println!("\n❌ ALL ENDPOINTS FAILED after retries.");
    Ok(())
}
