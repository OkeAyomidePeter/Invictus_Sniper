use dotenv::dotenv;
use solana_sdk::signature::Keypair;
use std::env;
use std::fs::File;
use std::io::Read;

pub struct Config {
    pub helius_api_key: String,
    pub private_key: String,
    pub telegram_token: String,
    // pub trading_mode: String,
    pub telegram_chat_id: String,
    pub database_url: String,
    pub min_liquidity_sol: f64,
    pub min_holders: u32,
    pub max_trade_size_sol: f64,
    pub max_daily_exposure_sol: f64,
    pub max_creator_ownership_percentage: f64,
    pub honeypot_check_enabled: bool,
    pub jupiter_api_timeout_ms: u64,
}

impl Config {
    pub fn load() -> Self {
        dotenv().ok();
        Self {
            helius_api_key: env::var("HELIUS_API_KEY").expect("HELIUS_API_KEY required"),
            private_key: env::var("SOLANA_PRIVATE_KEY")
                .or_else(|_| env::var("SOLANA_PRIVATE_KEY_PATH"))
                .expect("SOLANA_PRIVATE_KEY or SOLANA_PRIVATE_KEY_PATH required"),
            telegram_token: env::var("TELEGRAM_BOT_TOKEN").unwrap_or_default(),
            // trading_mode: env::var("TRADING_MODE").unwrap_or("devnet".to_string()),
            telegram_chat_id: env::var("TELEGRAM_CHAT_ID").unwrap_or("".to_string()),
            database_url: env::var("DATABASE_URL").unwrap_or("sqlite:./sniper_bot.db".to_string()),
            // Risk engine configuration from environment
            min_liquidity_sol: env::var("MIN_LIQUIDITY_SOL")
                .unwrap_or("10.0".to_string())
                .parse()
                .expect("MIN_LIQUIDITY_SOL must be a valid number"),
            min_holders: env::var("MIN_HOLDERS")
                .unwrap_or("10".to_string())
                .parse()
                .expect("MIN_HOLDERS must be a valid number"),
            max_trade_size_sol: env::var("MAX_TRADE_SIZE_SOL")
                .unwrap_or("1.0".to_string())
                .parse()
                .expect("MAX_TRADE_SIZE_SOL must be a valid number"),
            max_daily_exposure_sol: env::var("MAX_DAILY_EXPOSURE_SOL")
                .unwrap_or("10.0".to_string())
                .parse()
                .expect("MAX_DAILY_EXPOSURE_SOL must be a valid number"),
            max_creator_ownership_percentage: env::var("MAX_CREATOR_OWNERSHIP_PERCENTAGE")
                .unwrap_or("20.0".to_string())
                .parse()
                .expect("MAX_CREATOR_OWNERSHIP_PERCENTAGE must be a valid number"),
            honeypot_check_enabled: env::var("HONEYPOT_CHECK_ENABLED")
                .unwrap_or("true".to_string())
                .parse()
                .expect("HONEYPOT_CHECK_ENABLED must be true or false"),
            jupiter_api_timeout_ms: env::var("JUPITER_API_TIMEOUT_MS")
                .unwrap_or("5000".to_string())
                .parse()
                .expect("JUPITER_API_TIMEOUT_MS must be a valid number"),
        }
    }

    /// Load keypair from file path or raw private key string
    pub fn load_keypair(&self) -> Result<Keypair, Box<dyn std::error::Error>> {
        let contents = if self.private_key.starts_with('/') || self.private_key.contains('.') {
            // Treat as file path
            let mut file = File::open(&self.private_key)?;
            let mut contents = String::new();
            file.read_to_string(&mut contents)?;
            contents
        } else {
            // Treat as raw private key string
            self.private_key.clone()
        };

        let contents = contents.trim();

        // Try to parse as base58 first
        if let Ok(keypair_bytes) = bs58::decode(contents.trim()).into_vec() {
            if keypair_bytes.len() == 64 {
                return Ok(Keypair::from_bytes(&keypair_bytes)?);
            }
        }

        // Try to parse as JSON array
        if let Ok(keypair_array) = serde_json::from_str::<Vec<u8>>(&contents) {
            if keypair_array.len() == 64 {
                return Ok(Keypair::from_bytes(&keypair_array)?);
            }
        }

        // Try to parse as JSON array format
        if contents.starts_with('[') {
            let keypair_data: serde_json::Value = serde_json::from_str(contents)?;
            if let Some(array) = keypair_data.as_array() {
                let keypair_bytes: Vec<u8> = array
                    .iter()
                    .take(64)
                    .filter_map(|v| v.as_u64().map(|n| n as u8))
                    .collect();

                if keypair_bytes.len() == 64 {
                    return Ok(Keypair::from_bytes(&keypair_bytes)?);
                }
            }
        }

        Err("Invalid keypair format".into())
    }

    /// Mask secrets for logging
    pub fn mask_secret(secret: &str) -> String {
        if secret.is_empty() {
            return "".to_string();
        }
        if secret.len() <= 8 {
            return "***".to_string();
        }
        format!("{}...{}", &secret[..4], &secret[secret.len() - 4..])
    }

    /// Display config with masked secrets
    pub fn display(&self) -> String {
        let private_key_display =
            if self.private_key.starts_with('/') || self.private_key.contains('.') {
                self.private_key.clone()
            } else {
                Self::mask_secret(&self.private_key)
            };

        format!(
            "Config loaded:  helius_key={}, private_key={}, telegram_token={}, telegram_chat_id={}, database_url={}, min_liquidity_sol={}, min_holders={}, max_trade_size_sol={}, max_daily_exposure_sol={}, honeypot_check_enabled={}",
            // self.trading_mode,
            Self::mask_secret(&self.helius_api_key),
            private_key_display,
            if self.telegram_token.is_empty() { "not_set".to_string() } else { Self::mask_secret(&self.telegram_token) },
            self.telegram_chat_id,
            self.database_url,
            self.min_liquidity_sol,
            self.min_holders,
            self.max_trade_size_sol,
            self.max_daily_exposure_sol,
            self.honeypot_check_enabled
        )
    }
}
