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

/// Manages blockhash updates and rapid transaction signing
pub struct Presigner {
    rpc_client: Arc<RpcClient>,
    keypair: Arc<Keypair>,
    blockhash: Arc<RwLock<Hash>>,
    _update_handle: JoinHandle<()>,
}

impl Presigner {
    pub fn new(config: &Config) -> Self {
        let rpc_url = "https://api.mainnet-beta.solana.com".to_string(); // Should be from config
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
        let payer_pubkey = self.keypair.pubkey();
        let recent_blockhash = self.get_blockhash();

        let message = Message::new(instructions, Some(&payer_pubkey));
        let mut tx = Transaction::new_unsigned(message);
        
        // Sign immediately
        tx.try_sign(&[self.keypair.as_ref()], recent_blockhash)?;
        
        Ok(tx)
    }

    /// Send a transaction immediately
    pub fn send_transaction(&self, tx: &Transaction) -> Result<String> {
        self.rpc_client.send_and_confirm_transaction(tx)
            .context("Failed to send transaction")
            .map(|s| s.to_string())
    }

    /// Sign an existing VersionedTransaction with the loaded keypair
    pub fn sign_versioned_tx(&self, tx: &mut VersionedTransaction) -> Result<()> {
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
}
