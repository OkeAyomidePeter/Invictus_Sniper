use dotenv::dotenv;
use solana_sdk::signature::Keypair;
use std::env;
use std::fs::File;
use std::io::Read;

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum TransactionMode {
    Jito,
    Standard,
}

impl std::fmt::Display for TransactionMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TransactionMode::Jito => write!(f, "Jito"),
            TransactionMode::Standard => write!(f, "Standard"),
        }
    }
}

impl std::str::FromStr for TransactionMode {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "jito" => Ok(TransactionMode::Jito),
            "standard" | "helius" => Ok(TransactionMode::Standard),
            _ => Err(format!("Invalid transaction mode: {}", s)),
        }
    }
}

#[derive(Clone)]
pub struct Config {
    pub helius_api_key: String,
    pub rpc_url: String,
    pub private_key: String,
    pub telegram_token: String,
    pub telegram_chat_id: String,
    pub alternate_telegram_chat_id: Option<String>,
    pub database_url: String,
    pub min_liquidity_sol: f64,
    pub min_holders: u32,
    pub max_trade_size_sol: f64,
    pub max_daily_exposure_sol: f64,
    pub max_creator_ownership_percentage: f64,
    pub honeypot_check_enabled: bool,
    pub jupiter_api_timeout_ms: u64,
    // Transaction Mode
    pub transaction_mode: TransactionMode,
    pub priority_fee_lamports: u64,
    pub compute_unit_limit: u32,
    // Auto-sell configuration
    pub auto_sell_enabled: bool,
    pub auto_sell_profit_target_pct: f64,
    pub auto_sell_stop_loss_pct: f64,
    pub auto_sell_timeout_seconds: u64,
    pub auto_sell_slippage_bps: u16,
    pub auto_sell_price_check_interval_ms: u64,
    // Dynamic Jito tip configuration
    pub jito_base_tip_lamports: u64,
    pub jito_min_tip_lamports: u64,
    pub jito_max_tip_lamports: u64,
    pub jito_dynamic_tips_enabled: bool,
    // Retry configuration
    pub tx_retry_max_attempts: usize,
    pub tx_retry_initial_delay_ms: u64,
    pub tx_retry_max_delay_ms: u64,
    pub tx_retry_backoff_multiplier: f64,
    // Rate limiting
    pub helius_max_requests_per_second: f64,
    pub jupiter_max_requests_per_second: f64,
    pub rate_limiting_enabled: bool,
    // Wallet monitoring
    pub wallet_low_balance_alert_sol: f64,
    pub wallet_monitor_interval_secs: u64,
    pub wallet_reserve_for_fees_sol: f64,
    // Parallel trading
    pub max_concurrent_trades: usize,
    pub max_open_positions: usize,
    pub total_exposure_limit_sol: f64,
    // Dip Strategy - Basic
    pub dip_strategy_enabled: bool,
    pub dip_entry_pct: f64,        // e.g. 30.0 for -30% drop
    pub min_volume_usd_5m: f64,    // e.g. 1000.0
    pub watchlist_timeout_seconds: u64, // e.g. 300
    // Dip Strategy - Entry Validation
    pub volume_trend_enabled: bool,
    pub volume_samples_required: usize,
    pub volume_sample_interval_secs: u64,
    pub holder_stability_enabled: bool,
    pub min_holder_retention_pct: f64,
    // Trailing Stop Loss
    pub trailing_stop_enabled: bool,
    pub trailing_stop_distance_pct: f64,
    // Partial Exits
    pub partial_exit_enabled: bool,
    pub partial_exit_target_pct: f64,
    pub partial_exit_amount_pct: f64,
    // Dynamic Timeout
    pub dynamic_timeout_enabled: bool,
    pub timeout_extension_seconds: u64,
    pub max_timeout_extensions: u32,
}

impl Config {
    pub fn load() -> Self {
        dotenv().ok();

        Self {
            helius_api_key: env::var("HELIUS_API_KEY").expect("HELIUS_API_KEY must be set"),
            rpc_url: env::var("SOLANA_RPC_URL").unwrap_or("https://api.mainnet-beta.solana.com".to_string()),
            private_key: env::var("SOLANA_PRIVATE_KEY").expect("SOLANA_PRIVATE_KEY must be set"),
            telegram_token: env::var("TELEGRAM_BOT_TOKEN").expect("TELEGRAM_BOT_TOKEN must be set"),
            telegram_chat_id: env::var("TELEGRAM_CHAT_ID").expect("TELEGRAM_CHAT_ID must be set"),
            alternate_telegram_chat_id: env::var("ALTERNATE_TELEGRAM_CHAT_ID").ok().filter(|s| !s.is_empty()),
            database_url: env::var("DATABASE_URL").unwrap_or("sqlite://invictus.db".to_string()),
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
                .unwrap_or("50.0".to_string())
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
            // Transaction Mode
            transaction_mode: env::var("TRANSACTION_MODE")
                .unwrap_or("Standard".to_string())
                .parse()
                .unwrap_or(TransactionMode::Standard),
            priority_fee_lamports: env::var("PRIORITY_FEE_LAMPORTS")
                .unwrap_or("100000".to_string())
                .parse()
                .expect("PRIORITY_FEE_LAMPORTS must be a valid number"),
            compute_unit_limit: env::var("COMPUTE_UNIT_LIMIT")
                .unwrap_or("200000".to_string())
                .parse()
                .expect("COMPUTE_UNIT_LIMIT must be a valid number"),
            // Auto-sell configuration
            auto_sell_enabled: env::var("AUTO_SELL_ENABLED")
                .unwrap_or("true".to_string())
                .parse()
                .expect("AUTO_SELL_ENABLED must be true or false"),
            auto_sell_profit_target_pct: env::var("AUTO_SELL_PROFIT_TARGET_PCT")
                .unwrap_or("50.0".to_string())
                .parse()
                .expect("AUTO_SELL_PROFIT_TARGET_PCT must be a valid number"),
            auto_sell_stop_loss_pct: env::var("AUTO_SELL_STOP_LOSS_PCT")
                .unwrap_or("20.0".to_string())
                .parse()
                .expect("AUTO_SELL_STOP_LOSS_PCT must be a valid number"),
            auto_sell_timeout_seconds: env::var("AUTO_SELL_TIMEOUT_SECONDS")
                .unwrap_or("120".to_string())
                .parse()
                .expect("AUTO_SELL_TIMEOUT_SECONDS must be a valid number"),
            auto_sell_slippage_bps: env::var("AUTO_SELL_SLIPPAGE_BPS")
                .unwrap_or("300".to_string())
                .parse()
                .expect("AUTO_SELL_SLIPPAGE_BPS must be a valid number"),
            auto_sell_price_check_interval_ms: env::var("AUTO_SELL_PRICE_CHECK_INTERVAL_MS")
                .unwrap_or("2000".to_string())
                .parse()
                .expect("AUTO_SELL_PRICE_CHECK_INTERVAL_MS must be a valid number"),
            // Dynamic Jito tip configuration
            jito_base_tip_lamports: env::var("JITO_BASE_TIP_LAMPORTS")
                .unwrap_or("1000000".to_string())
                .parse()
                .expect("JITO_BASE_TIP_LAMPORTS must be a valid number"),
            jito_min_tip_lamports: env::var("JITO_MIN_TIP_LAMPORTS")
                .unwrap_or("500000".to_string())
                .parse()
                .expect("JITO_MIN_TIP_LAMPORTS must be a valid number"),
            jito_max_tip_lamports: env::var("JITO_MAX_TIP_LAMPORTS")
                .unwrap_or("5000000".to_string())
                .parse()
                .expect("JITO_MAX_TIP_LAMPORTS must be a valid number"),
            jito_dynamic_tips_enabled: env::var("JITO_DYNAMIC_TIPS_ENABLED")
                .unwrap_or("true".to_string())
                .parse()
                .expect("JITO_DYNAMIC_TIPS_ENABLED must be true or false"),
            // Retry configuration
            tx_retry_max_attempts: env::var("TX_RETRY_MAX_ATTEMPTS")
                .unwrap_or("3".to_string())
                .parse()
                .expect("TX_RETRY_MAX_ATTEMPTS must be a valid number"),
            tx_retry_initial_delay_ms: env::var("TX_RETRY_INITIAL_DELAY_MS")
                .unwrap_or("500".to_string())
                .parse()
                .expect("TX_RETRY_INITIAL_DELAY_MS must be a valid number"),
            tx_retry_max_delay_ms: env::var("TX_RETRY_MAX_DELAY_MS")
                .unwrap_or("5000".to_string())
                .parse()
                .expect("TX_RETRY_MAX_DELAY_MS must be a valid number"),
            tx_retry_backoff_multiplier: env::var("TX_RETRY_BACKOFF_MULTIPLIER")
                .unwrap_or("2.0".to_string())
                .parse()
                .expect("TX_RETRY_BACKOFF_MULTIPLIER must be a valid number"),
            // Rate limiting
            helius_max_requests_per_second: env::var("HELIUS_MAX_REQUESTS_PER_SECOND")
                .unwrap_or("10.0".to_string())
                .parse()
                .expect("HELIUS_MAX_REQUESTS_PER_SECOND must be a valid number"),
            jupiter_max_requests_per_second: env::var("JUPITER_MAX_REQUESTS_PER_SECOND")
                .unwrap_or("5.0".to_string())
                .parse()
                .expect("JUPITER_MAX_REQUESTS_PER_SECOND must be a valid number"),
            rate_limiting_enabled: env::var("RATE_LIMITING_ENABLED")
                .unwrap_or("true".to_string())
                .parse()
                .expect("RATE_LIMITING_ENABLED must be true or false"),
            // Wallet monitoring
            wallet_low_balance_alert_sol: env::var("WALLET_LOW_BALANCE_ALERT_SOL")
                .unwrap_or("0.5".to_string())
                .parse()
                .expect("WALLET_LOW_BALANCE_ALERT_SOL must be a valid number"),
            wallet_monitor_interval_secs: env::var("WALLET_MONITOR_INTERVAL_SECS")
                .unwrap_or("60".to_string())
                .parse()
                .expect("WALLET_MONITOR_INTERVAL_SECS must be a valid number"),
            wallet_reserve_for_fees_sol: env::var("WALLET_RESERVE_FOR_FEES_SOL")
                .unwrap_or("0.1".to_string())
                .parse()
                .expect("WALLET_RESERVE_FOR_FEES_SOL must be a valid number"),
            // Parallel trading
            max_concurrent_trades: env::var("MAX_CONCURRENT_TRADES")
                .unwrap_or("5".to_string())
                .parse()
                .expect("MAX_CONCURRENT_TRADES must be a valid number"),
            max_open_positions: env::var("MAX_OPEN_POSITIONS")
                .unwrap_or("10".to_string())
                .parse()
                .expect("MAX_OPEN_POSITIONS must be a valid number"),
            total_exposure_limit_sol: env::var("TOTAL_EXPOSURE_LIMIT_SOL")
                .unwrap_or("10.0".to_string())
                .parse()
                .expect("TOTAL_EXPOSURE_LIMIT_SOL must be a valid number"),
            // Dip Strategy
            dip_strategy_enabled: env::var("DIP_STRATEGY_ENABLED")
                .unwrap_or("true".to_string())
                .parse()
                .expect("DIP_STRATEGY_ENABLED must be true or false"),
            dip_entry_pct: env::var("DIP_ENTRY_PCT")
                .unwrap_or("30.0".to_string())
                .parse()
                .expect("DIP_ENTRY_PCT must be a valid number"),
            min_volume_usd_5m: env::var("MIN_VOLUME_USD_5M")
                .unwrap_or("1000.0".to_string())
                .parse()
                .expect("MIN_VOLUME_USD_5M must be a valid number"),
            watchlist_timeout_seconds: env::var("WATCHLIST_TIMEOUT_SECONDS")
                .unwrap_or("300".to_string())
                .parse()
                .expect("WATCHLIST_TIMEOUT_SECONDS must be a valid number"),
            // Dip Strategy - Entry Validation
            volume_trend_enabled: env::var("VOLUME_TREND_ENABLED")
                .unwrap_or("true".to_string())
                .parse()
                .expect("VOLUME_TREND_ENABLED must be true or false"),
            volume_samples_required: env::var("VOLUME_SAMPLES_REQUIRED")
                .unwrap_or("3".to_string())
                .parse()
                .expect("VOLUME_SAMPLES_REQUIRED must be a valid number"),
            volume_sample_interval_secs: env::var("VOLUME_SAMPLE_INTERVAL_SECS")
                .unwrap_or("5".to_string())
                .parse()
                .expect("VOLUME_SAMPLE_INTERVAL_SECS must be a valid number"),
            holder_stability_enabled: env::var("HOLDER_STABILITY_ENABLED")
                .unwrap_or("true".to_string())
                .parse()
                .expect("HOLDER_STABILITY_ENABLED must be true or false"),
            min_holder_retention_pct: env::var("MIN_HOLDER_RETENTION_PCT")
                .unwrap_or("90.0".to_string())
                .parse()
                .expect("MIN_HOLDER_RETENTION_PCT must be a valid number"),
            // Trailing Stop Loss
            trailing_stop_enabled: env::var("TRAILING_STOP_ENABLED")
                .unwrap_or("true".to_string())
                .parse()
                .expect("TRAILING_STOP_ENABLED must be true or false"),
            trailing_stop_distance_pct: env::var("TRAILING_STOP_DISTANCE_PCT")
                .unwrap_or("15.0".to_string())
                .parse()
                .expect("TRAILING_STOP_DISTANCE_PCT must be a valid number"),
            // Partial Exits
            partial_exit_enabled: env::var("PARTIAL_EXIT_ENABLED")
                .unwrap_or("true".to_string())
                .parse()
                .expect("PARTIAL_EXIT_ENABLED must be true or false"),
            partial_exit_target_pct: env::var("PARTIAL_EXIT_TARGET_PCT")
                .unwrap_or("30.0".to_string())
                .parse()
                .expect("PARTIAL_EXIT_TARGET_PCT must be a valid number"),
            partial_exit_amount_pct: env::var("PARTIAL_EXIT_AMOUNT_PCT")
                .unwrap_or("50.0".to_string())
                .parse()
                .expect("PARTIAL_EXIT_AMOUNT_PCT must be a valid number"),
            // Dynamic Timeout
            dynamic_timeout_enabled: env::var("DYNAMIC_TIMEOUT_ENABLED")
                .unwrap_or("true".to_string())
                .parse()
                .expect("DYNAMIC_TIMEOUT_ENABLED must be true or false"),
            timeout_extension_seconds: env::var("TIMEOUT_EXTENSION_SECONDS")
                .unwrap_or("60".to_string())
                .parse()
                .expect("TIMEOUT_EXTENSION_SECONDS must be a valid number"),
            max_timeout_extensions: env::var("MAX_TIMEOUT_EXTENSIONS")
                .unwrap_or("2".to_string())
                .parse()
                .expect("MAX_TIMEOUT_EXTENSIONS must be a valid number"),
        }
    }

    /// Load keypair from file or base58 string
    pub fn load_keypair(&self) -> Result<Keypair, Box<dyn std::error::Error>> {
        // Try to parse as base58 first
        if let Ok(bytes) = bs58::decode(&self.private_key).into_vec() {
            if let Ok(keypair) = Keypair::from_bytes(&bytes) {
                return Ok(keypair);
            }
        }

        // Otherwise try as file path
        let mut file = File::open(&self.private_key)?;
        let mut contents = String::new();
        file.read_to_string(&mut contents)?;

        let bytes: Vec<u8> = serde_json::from_str(&contents)?;
        let keypair = Keypair::from_bytes(&bytes)?;
        Ok(keypair)
    }

    fn mask_secret(s: &str) -> String {
        if s.len() > 8 {
            format!("{}...{}", &s[..4], &s[s.len() - 4..])
        } else {
            "***".to_string()
        }
    }

    pub fn display(&self) -> String {
        let private_key_display = if self.private_key.starts_with('/') || self.private_key.starts_with('.') {
            format!("file:{}", self.private_key)
        } else {
            Self::mask_secret(&self.private_key)
        };

        format!(
            "Config loaded: mode={}, helius_key={}, rpc_url={}, private_key={}, telegram_token={}, telegram_chat_id={}, database_url={}, min_liquidity_sol={}, min_holders={}, max_trade_size_sol={}, max_daily_exposure_sol={}, honeypot_check_enabled={}, auto_sell_enabled={}, rate_limiting_enabled={}, max_concurrent_trades={}",
            self.transaction_mode,
            Self::mask_secret(&self.helius_api_key),
            self.rpc_url,
            private_key_display,
            if self.telegram_token.is_empty() { "not_set".to_string() } else { Self::mask_secret(&self.telegram_token) },
            self.telegram_chat_id,
            self.database_url,
            self.min_liquidity_sol,
            self.min_holders,
            self.max_trade_size_sol,
            self.max_daily_exposure_sol,
            self.honeypot_check_enabled,
            self.auto_sell_enabled,
            self.rate_limiting_enabled,
            self.max_concurrent_trades
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    #[test]
    fn test_config_defaults() {
        // Set only required vars, test defaults for optionals
        env::set_var("HELIUS_API_KEY", "test_key");
        env::set_var("SOLANA_PRIVATE_KEY", "test_private_key");
        env::set_var("TELEGRAM_BOT_TOKEN", "test_token");
        env::set_var("TELEGRAM_CHAT_ID", "test_chat_id");

        // Clear optional vars to ensure defaults
        env::remove_var("MIN_LIQUIDITY_SOL");
        env::remove_var("MIN_HOLDERS");
        env::remove_var("MAX_TRADE_SIZE_SOL");
        env::remove_var("AUTO_SELL_ENABLED");
        env::remove_var("AUTO_SELL_PROFIT_TARGET_PCT");
        env::remove_var("AUTO_SELL_STOP_LOSS_PCT");
        env::remove_var("JITO_BASE_TIP_LAMPORTS");
        env::remove_var("TX_RETRY_MAX_ATTEMPTS");
        env::remove_var("RATE_LIMITING_ENABLED");
        env::remove_var("MAX_CONCURRENT_TRADES");

        let config = Config::load();

        // Test defaults
        assert_eq!(config.transaction_mode, TransactionMode::Standard);
        assert_eq!(config.min_liquidity_sol, 10.0);
        assert_eq!(config.min_holders, 10);
        assert_eq!(config.max_trade_size_sol, 1.0);
        assert_eq!(config.auto_sell_enabled, true);
        assert_eq!(config.auto_sell_profit_target_pct, 50.0);
        assert_eq!(config.auto_sell_stop_loss_pct, 20.0);
        assert_eq!(config.jito_base_tip_lamports, 1_000_000);
        assert_eq!(config.tx_retry_max_attempts, 3);
        assert_eq!(config.rate_limiting_enabled, true);
        assert_eq!(config.max_concurrent_trades, 5);
    }

    #[test]
    fn test_config_custom_values() {
        // Set custom values
        env::set_var("HELIUS_API_KEY", "test_key");
        env::set_var("SOLANA_PRIVATE_KEY", "test_private_key");
        env::set_var("TELEGRAM_BOT_TOKEN", "test_token");
        env::set_var("TELEGRAM_CHAT_ID", "test_chat_id");
        env::set_var("MIN_LIQUIDITY_SOL", "25.0");
        env::set_var("AUTO_SELL_PROFIT_TARGET_PCT", "100.0");
        env::set_var("JITO_BASE_TIP_LAMPORTS", "2000000");
        env::set_var("MAX_CONCURRENT_TRADES", "10");

        let config = Config::load();

        assert_eq!(config.min_liquidity_sol, 25.0);
        assert_eq!(config.auto_sell_profit_target_pct, 100.0);
        assert_eq!(config.jito_base_tip_lamports, 2_000_000);
        assert_eq!(config.max_concurrent_trades, 10);
    }

    #[test]
    fn test_tip_limits() {
        env::set_var("HELIUS_API_KEY", "test_key");
        env::set_var("SOLANA_PRIVATE_KEY", "test_private_key");
        env::set_var("TELEGRAM_BOT_TOKEN", "test_token");
        env::set_var("TELEGRAM_CHAT_ID", "test_chat_id");

        let config = Config::load();

        // Test tip ranges
        assert!(config.jito_min_tip_lamports < config.jito_base_tip_lamports);
        assert!(config.jito_base_tip_lamports < config.jito_max_tip_lamports);
        assert_eq!(config.jito_min_tip_lamports, 500_000);
        assert_eq!(config.jito_max_tip_lamports, 5_000_000);
    }

    #[test]
    fn test_retry_config() {
        env::set_var("HELIUS_API_KEY", "test_key");
        env::set_var("SOLANA_PRIVATE_KEY", "test_private_key");
        env::set_var("TELEGRAM_BOT_TOKEN", "test_token");
        env::set_var("TELEGRAM_CHAT_ID", "test_chat_id");

        let config = Config::load();

        assert_eq!(config.tx_retry_max_attempts, 3);
        assert_eq!(config.tx_retry_initial_delay_ms, 500);
        assert_eq!(config.tx_retry_max_delay_ms, 5000);
        assert_eq!(config.tx_retry_backoff_multiplier, 2.0);
    }

    #[test]
    fn test_rate_limiting_defaults() {
        env::set_var("HELIUS_API_KEY", "test_key");
        env::set_var("SOLANA_PRIVATE_KEY", "test_private_key");
        env::set_var("TELEGRAM_BOT_TOKEN", "test_token");
        env::set_var("TELEGRAM_CHAT_ID", "test_chat_id");

        let config = Config::load();

        assert_eq!(config.helius_max_requests_per_second, 10.0);
        assert_eq!(config.jupiter_max_requests_per_second, 5.0);
        assert!(config.rate_limiting_enabled);
    }
}
