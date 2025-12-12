use std::env;
use std::io::{self, Write};
use reqwest::Client;
use serde_json::json;
use anyhow::{Context, Result};
use dotenv::dotenv;

#[tokio::main]
async fn main() -> Result<()> {
    dotenv().ok();
    let api_key = env::var("HELIUS_API_KEY").expect("HELIUS_API_KEY must be set in .env");
    let client = Client::new();

    println!("🔧 RPC Debugger Tool 🔧");
    println!("-----------------------");

    // 1. Get Token Mint
    let mint = prompt("Enter Token Mint Address: ")?;
    if mint.trim().is_empty() {
        println!("❌ Mint address cannot be empty.");
        return Ok(());
    }

    // 2. Get Pool Address (Optional)
    let pool_address = prompt("Enter Pool Address (Optional, press Enter to skip): ")?;

    println!("\n🚀 Querying RPC for Mint: {}", mint);
    
    // =====================================================================
    // QUERY 1: Mint Account Info (getAccountInfo)
    // =====================================================================
    println!("\n[1/4] Fetching Mint Account Info...");
    let mint_url = format!("https://mainnet.helius-rpc.com/?api-key={}", api_key);
    let mint_body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "getAccountInfo",
        "params": [
            mint,
            { "encoding": "jsonParsed" }
        ]
    });
    
    match query_rpc(&client, &mint_url, &mint_body).await {
        Ok(json) => {
            println!("✅ Raw Response:\n{}", serde_json::to_string_pretty(&json)?);
            // Quick Summary
            if let Some(info) = json.get("result").and_then(|r| r.get("value")).and_then(|v| v.get("data")).and_then(|d| d.get("parsed")).and_then(|p| p.get("info")) {
                let decimals = info.get("decimals").and_then(|d| d.as_u64()).unwrap_or(0);
                let supply = info.get("supply").and_then(|s| s.as_str()).unwrap_or("?");
                let mint_auth = info.get("mintAuthority").and_then(|s| s.as_str()).unwrap_or("None");
                let freeze_auth = info.get("freezeAuthority").and_then(|s| s.as_str()).unwrap_or("None");
                println!("📋 Summary: Decimals={}, Supply={}, MintAuth={}, FreezeAuth={}", decimals, supply, mint_auth, freeze_auth);
            }
        },
        Err(e) => println!("❌ Failed: {}", e),
    }

    // =====================================================================
    // QUERY 2: Token Metadata (getAsset)
    // =====================================================================
    println!("\n[2/4] Fetching Token Metadata (DAS getAsset)...");
    let das_body = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "getAsset",
        "params": { "id": mint }
    });

    match query_rpc(&client, &mint_url, &das_body).await {
        Ok(json) => {
            println!("✅ Raw Response:\n{}", serde_json::to_string_pretty(&json)?);
        },
        Err(e) => println!("❌ Failed: {}", e),
    }

    // =====================================================================
    // QUERY 3: Holders (getTokenLargestAccounts)
    // =====================================================================
    println!("\n[3/4] Fetching Top Holders (getTokenLargestAccounts)...");
    let holders_body = json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "getTokenLargestAccounts",
        "params": [ mint ]
    });

    match query_rpc(&client, &mint_url, &holders_body).await {
        Ok(json) => {
            println!("✅ Raw Response:\n{}", serde_json::to_string_pretty(&json)?);
            if let Some(accounts) = json.get("result").and_then(|r| r.get("value")).and_then(|v| v.as_array()) {
                println!("📋 Found {} holder accounts.", accounts.len());
            }
        },
        Err(e) => println!("❌ Failed: {}", e),
    }

    // =====================================================================
    // QUERY 4: Pool Liquidity (getTokenAccountsByOwner)
    // =====================================================================
    if !pool_address.trim().is_empty() {
        println!("\n[4/4] Fetching Pool Liquidity (getTokenAccountsByOwner)...");
        println!("    Pool: {}", pool_address);
        
        let pool_body = json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "getTokenAccountsByOwner",
            "params": [
                pool_address,
                { "programId": "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA" },
                { "encoding": "jsonParsed" }
            ]
        });

        match query_rpc(&client, &mint_url, &pool_body).await {
            Ok(json) => {
                println!("✅ Raw Response:\n{}", serde_json::to_string_pretty(&json)?);
                
                // Analyze accounts
                if let Some(accounts) = json.get("result").and_then(|r| r.get("value")).and_then(|v| v.as_array()) {
                    println!("📋 Found {} token accounts owned by pool.", accounts.len());
                    for (i, acc) in accounts.iter().enumerate() {
                        if let Some(info) = acc.get("account").and_then(|a| a.get("data")).and_then(|d| d.get("parsed")).and_then(|p| p.get("info")) {
                            let mint = info.get("mint").and_then(|m| m.as_str()).unwrap_or("?");
                            let amount = info.get("tokenAmount").and_then(|t| t.get("uiAmount")).and_then(|u| u.as_f64()).unwrap_or(0.0);
                            println!("    Account #{}: Mint={}, Amount={}", i, mint, amount);
                        }
                    }
                } else {
                    println!("⚠️ No accounts found or invalid response format.");
                }
            },
            Err(e) => println!("❌ Failed: {}", e),
        }
    } else {
        println!("\n[4/4] Skipping Pool Liquidity (No pool address provided)");
    }

    println!("\n✨ Done.");
    Ok(())
}

fn prompt(msg: &str) -> Result<String> {
    print!("{}", msg);
    io::stdout().flush()?;
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    Ok(input.trim().to_string())
}

async fn query_rpc(client: &Client, url: &str, body: &serde_json::Value) -> Result<serde_json::Value> {
    let resp = client.post(url)
        .json(body)
        .send()
        .await
        .context("Request failed")?;
    
    let json: serde_json::Value = resp.json().await.context("Failed to parse JSON")?;
    Ok(json)
}
