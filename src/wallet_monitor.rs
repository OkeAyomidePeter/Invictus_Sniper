use anyhow::{Context, Result};
use log::{error, info, warn};
use solana_client::rpc_client::RpcClient;
use solana_sdk::pubkey::Pubkey;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::time::{sleep, Duration};

const LAMPORTS_PER_SOL: u64 = 1_000_000_000;

/// Alert types for wallet monitoring
#[derive(Debug, Clone)]
pub enum WalletAlert {
    LowBalance { current_sol: f64, threshold_sol: f64 },
    InsufficientForTrade { required_sol: f64, available_sol: f64 },
}

/// Monitors wallet SOL balance and sends alerts
pub struct WalletMonitor {
    rpc_url: String,
    pubkey: Pubkey,
    low_balance_threshold_lamports: u64,
    reserve_for_fees_lamports: u64,
    check_interval_secs: u64,
}

impl WalletMonitor {
    pub fn new(
        rpc_url: String,
        pubkey: Pubkey,
        low_balance_threshold_sol: f64,
        reserve_for_fees_sol: f64,
        check_interval_secs: u64,
    ) -> Self {
        Self {
            rpc_url,
            pubkey,
            low_balance_threshold_lamports: (low_balance_threshold_sol * LAMPORTS_PER_SOL as f64) as u64,
            reserve_for_fees_lamports: (reserve_for_fees_sol * LAMPORTS_PER_SOL as f64) as u64,
            check_interval_secs,
        }
    }

    /// Start monitoring wallet balance in background task
    /// Returns channel for receiving alerts
    pub fn start_monitoring(self: Arc<Self>) -> mpsc::Receiver<WalletAlert> {
        let (tx, rx) = mpsc::channel(10);
        
        tokio::spawn(async move {
            info!("💰 Wallet monitoring started for {}", self.pubkey);
            let mut last_alert_sent = false;
            
            loop {
                match self.get_balance().await {
                    Ok(balance_lamports) => {
                        let balance_sol = balance_lamports as f64 / LAMPORTS_PER_SOL as f64;
                        
                        if balance_lamports < self.low_balance_threshold_lamports {
                            if !last_alert_sent {
                                warn!(
                                    "⚠️  Low wallet balance: {:.4} SOL (threshold: {:.4} SOL)",
                                    balance_sol,
                                    self.low_balance_threshold_lamports as f64 / LAMPORTS_PER_SOL as f64
                                );
                                
                                let alert = WalletAlert::LowBalance {
                                    current_sol: balance_sol,
                                    threshold_sol: self.low_balance_threshold_lamports as f64 / LAMPORTS_PER_SOL as f64,
                                };
                                
                                if tx.send(alert).await.is_err() {
                                    error!("Failed to send low balance alert (receiver dropped)");
                                    break;
                                }
                                
                                last_alert_sent = true;
                            }
                        } else {
                            last_alert_sent = false;
                        }
                    }
                    Err(e) => {
                        error!("Failed to query wallet balance: {}", e);
                    }
                }
                
                sleep(Duration::from_secs(self.check_interval_secs)).await;
            }
            
            info!("💰 Wallet monitoring stopped");
        });
        
        rx
    }

    /// Get current SOL balance
    pub async fn get_balance(&self) -> Result<u64> {
        let client = RpcClient::new(self.rpc_url.clone());
        let pubkey = self.pubkey;
        let balance = tokio::task::spawn_blocking(move || {
            client.get_balance(&pubkey)
        })
        .await?
        .context("Failed to get wallet balance")?;
        
        Ok(balance)
    }

    /// Check if wallet has sufficient balance for a trade
    /// Accounts for reserve needed for fees
    pub async fn has_sufficient_balance(&self, required_lamports: u64) -> Result<bool> {
        let balance = self.get_balance().await?;
        let total_needed = required_lamports + self.reserve_for_fees_lamports;
        Ok(balance >= total_needed)
    }

    /// Get available balance (total - reserve for fees)
    pub async fn get_available_balance(&self) -> Result<u64> {
        let balance = self.get_balance().await?;
        Ok(balance.saturating_sub(self.reserve_for_fees_lamports))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use solana_sdk::signature::{Keypair, Signer};

    #[test]
    fn test_wallet_alert_creation() {
        let alert = WalletAlert::LowBalance {
            current_sol: 0.3,
            threshold_sol: 0.5,
        };
        
        match alert {
            WalletAlert::LowBalance { current_sol, threshold_sol } => {
                assert_eq!(current_sol, 0.3);
                assert_eq!(threshold_sol, 0.5);
            }
            _ => panic!("Wrong alert type"),
        }
    }

    #[test]
    fn test_wallet_monitor_creation() {
        let keypair = Keypair::new();
        let monitor = WalletMonitor::new(
            "https://api.mainnet-beta.solana.com".to_string(),
            keypair.pubkey(),
            0.5,
            0.1,
            60,
        );
        
        assert_eq!(monitor.check_interval_secs, 60);
        assert_eq!(monitor.low_balance_threshold_lamports, 500_000_000); // 0.5 SOL
        assert_eq!(monitor.reserve_for_fees_lamports, 100_000_000); // 0.1 SOL
    }
}
