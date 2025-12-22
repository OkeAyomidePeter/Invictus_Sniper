pub mod config_tab;
pub mod logs_tab;
pub mod status_tab;
pub mod theme;
pub mod bot_runtime;
pub mod dashboard_tab;
pub mod positions_tab;
pub mod trades_tab;
pub mod logger;


use eframe::egui;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;
use chrono::Local;

use theme::{Theme, ThemeColors};
use bot_runtime::{BotRuntime, BotState, BotMetrics, BotEvent};
use crate::config::Config;

#[derive(Clone)]
pub struct ConfigData {
    // Core Configuration
    pub helius_api_key: String,
    pub rpc_url: String,
    pub solana_private_key: String,
    pub birdeye_api_key: String,
    pub jupiter_api_key: String,
    
    // Telegram Bot
    pub telegram_bot_token: String,
    pub telegram_chat_id: String,
    
    // Database
    pub database_url: String,
    
    // Risk Parameters
    pub min_liquidity_sol: String,
    pub min_holders: String,
    pub max_trade_size_sol: String,
    pub max_daily_exposure_sol: String,
    pub max_creator_ownership_percentage: String,
    pub honeypot_check_enabled: bool,
    pub jupiter_api_timeout_ms: String,
    
    // Auto-Sell Configuration
    pub auto_sell_enabled: bool,
    pub auto_sell_profit_target_pct: String,
    pub auto_sell_stop_loss_pct: String,
    pub auto_sell_timeout_seconds: String,
    pub auto_sell_slippage_bps: String,
    pub auto_sell_price_check_interval_ms: String,
    
    // Dynamic Jito Tips
    pub jito_dynamic_tips_enabled: bool,
    pub jito_base_tip_lamports: String,
    pub jito_min_tip_lamports: String,
    pub jito_max_tip_lamports: String,
    
    // Retry Logic
    pub tx_retry_max_attempts: String,
    pub tx_retry_initial_delay_ms: String,
    pub tx_retry_max_delay_ms: String,
    pub tx_retry_backoff_multiplier: String,
    
    // Rate Limiting
    pub rate_limiting_enabled: bool,
    pub helius_max_requests_per_second: String,
    pub jupiter_max_requests_per_second: String,
    
    // Wallet Monitoring
    pub wallet_low_balance_alert_sol: String,
    pub wallet_monitor_interval_secs: String,
    pub wallet_reserve_for_fees_sol: String,
    
    // Parallel Trading
    pub max_concurrent_trades: String,
    pub max_open_positions: String,
    pub total_exposure_limit_sol: String,
    
    // Dip Strategy
    pub dip_strategy_enabled: bool,
    pub dip_entry_pct: String,
    pub min_volume_usd_5m: String,
    pub watchlist_timeout_seconds: String,
    
    // Enhanced Entry Validation
    pub volume_trend_enabled: bool,
    pub volume_samples_required: String,
    pub volume_sample_interval_secs: String,
    pub holder_stability_enabled: bool,
    pub min_holder_retention_pct: String,
    
    // Trailing Stop Loss
    pub trailing_stop_enabled: bool,
    pub trailing_stop_distance_pct: String,
    
    // Partial Exits
    pub partial_exit_enabled: bool,
    pub partial_exit_target_pct: String,
    pub partial_exit_amount_pct: String,
    
    // Dynamic Timeout
    pub dynamic_timeout_enabled: bool,
    pub timeout_extension_seconds: String,
    pub max_timeout_extensions: String,
}

impl Default for ConfigData {
    fn default() -> Self {
        Self {
            helius_api_key: String::new(),
            rpc_url: "https://api.mainnet-beta.solana.com".to_string(),
            solana_private_key: String::new(),
            birdeye_api_key: String::new(),
            jupiter_api_key: String::new(),
            telegram_bot_token: String::new(),
            telegram_chat_id: String::new(),
            database_url: "sqlite://invictus.db".to_string(),
            min_liquidity_sol: "10.0".to_string(),
            min_holders: "10".to_string(),
            max_trade_size_sol: "1.0".to_string(),
            max_daily_exposure_sol: "10.0".to_string(),
            max_creator_ownership_percentage: "50.0".to_string(),
            honeypot_check_enabled: true,
            jupiter_api_timeout_ms: "5000".to_string(),
            auto_sell_enabled: true,
            auto_sell_profit_target_pct: "50.0".to_string(),
            auto_sell_stop_loss_pct: "20.0".to_string(),
            auto_sell_timeout_seconds: "120".to_string(),
            auto_sell_slippage_bps: "300".to_string(),
            auto_sell_price_check_interval_ms: "2000".to_string(),
            jito_dynamic_tips_enabled: true,
            jito_base_tip_lamports: "1000000".to_string(),
            jito_min_tip_lamports: "500000".to_string(),
            jito_max_tip_lamports: "5000000".to_string(),
            tx_retry_max_attempts: "3".to_string(),
            tx_retry_initial_delay_ms: "500".to_string(),
            tx_retry_max_delay_ms: "5000".to_string(),
            tx_retry_backoff_multiplier: "2.0".to_string(),
            rate_limiting_enabled: true,
            helius_max_requests_per_second: "10.0".to_string(),
            jupiter_max_requests_per_second: "5.0".to_string(),
            wallet_low_balance_alert_sol: "0.5".to_string(),
            wallet_monitor_interval_secs: "60".to_string(),
            wallet_reserve_for_fees_sol: "0.1".to_string(),
            max_concurrent_trades: "5".to_string(),
            max_open_positions: "10".to_string(),
            total_exposure_limit_sol: "10.0".to_string(),
            dip_strategy_enabled: true,
            dip_entry_pct: "30.0".to_string(),
            min_volume_usd_5m: "1000.0".to_string(),
            watchlist_timeout_seconds: "300".to_string(),
            // Enhanced Entry Validation
            volume_trend_enabled: true,
            volume_samples_required: "3".to_string(),
            volume_sample_interval_secs: "5".to_string(),
            holder_stability_enabled: true,
            min_holder_retention_pct: "90.0".to_string(),
            // Trailing Stop Loss
            trailing_stop_enabled: true,
            trailing_stop_distance_pct: "15.0".to_string(),
            // Partial Exits
            partial_exit_enabled: true,
            partial_exit_target_pct: "30.0".to_string(),
            partial_exit_amount_pct: "50.0".to_string(),
            // Dynamic Timeout
            dynamic_timeout_enabled: true,
            timeout_extension_seconds: "60".to_string(),
            max_timeout_extensions: "2".to_string(),
        }
    }
}

impl ConfigData {
    fn to_config(&self) -> Result<Config, String> {
        Ok(Config {
            helius_api_key: self.helius_api_key.clone(),
            rpc_url: self.rpc_url.clone(),
            private_key: self.solana_private_key.clone(),
            telegram_token: self.telegram_bot_token.clone(),
            telegram_chat_id: self.telegram_chat_id.clone(),
            birdeye_api_key: self.birdeye_api_key.clone(),
            jupiter_api_key: self.jupiter_api_key.clone(),
            alternate_telegram_chat_id: None, // Loaded from env
            database_url: self.database_url.clone(),
            min_liquidity_sol: self.min_liquidity_sol.parse().map_err(|_| "Invalid min_liquidity_sol")?,
            min_holders: self.min_holders.parse().map_err(|_| "Invalid min_holders")?,
            max_trade_size_sol: self.max_trade_size_sol.parse().map_err(|_| "Invalid max_trade_size_sol")?,
            max_daily_exposure_sol: self.max_daily_exposure_sol.parse().map_err(|_| "Invalid max_daily_exposure_sol")?,
            max_creator_ownership_percentage: self.max_creator_ownership_percentage.parse().map_err(|_| "Invalid max_creator_ownership_percentage")?,
            honeypot_check_enabled: self.honeypot_check_enabled,
            jupiter_api_timeout_ms: self.jupiter_api_timeout_ms.parse().map_err(|_| "Invalid jupiter_api_timeout_ms")?,
            auto_sell_enabled: self.auto_sell_enabled,
            auto_sell_profit_target_pct: self.auto_sell_profit_target_pct.parse().map_err(|_| "Invalid auto_sell_profit_target_pct")?,
            auto_sell_stop_loss_pct: self.auto_sell_stop_loss_pct.parse().map_err(|_| "Invalid auto_sell_stop_loss_pct")?,
            auto_sell_timeout_seconds: self.auto_sell_timeout_seconds.parse().map_err(|_| "Invalid auto_sell_timeout_seconds")?,
            auto_sell_slippage_bps: self.auto_sell_slippage_bps.parse().map_err(|_| "Invalid auto_sell_slippage_bps")?,
            auto_sell_price_check_interval_ms: self.auto_sell_price_check_interval_ms.parse().map_err(|_| "Invalid auto_sell_price_check_interval_ms")?,
            jito_base_tip_lamports: self.jito_base_tip_lamports.parse().map_err(|_| "Invalid jito_base_tip_lamports")?,
            jito_min_tip_lamports: self.jito_min_tip_lamports.parse().map_err(|_| "Invalid jito_min_tip_lamports")?,
            jito_max_tip_lamports: self.jito_max_tip_lamports.parse().map_err(|_| "Invalid jito_max_tip_lamports")?,
            jito_dynamic_tips_enabled: self.jito_dynamic_tips_enabled,
            tx_retry_max_attempts: self.tx_retry_max_attempts.parse().map_err(|_| "Invalid tx_retry_max_attempts")?,
            tx_retry_initial_delay_ms: self.tx_retry_initial_delay_ms.parse().map_err(|_| "Invalid tx_retry_initial_delay_ms")?,
            tx_retry_max_delay_ms: self.tx_retry_max_delay_ms.parse().map_err(|_| "Invalid tx_retry_max_delay_ms")?,
            tx_retry_backoff_multiplier: self.tx_retry_backoff_multiplier.parse().map_err(|_| "Invalid tx_retry_backoff_multiplier")?,
            helius_max_requests_per_second: self.helius_max_requests_per_second.parse().map_err(|_| "Invalid helius_max_requests_per_second")?,
            jupiter_max_requests_per_second: self.jupiter_max_requests_per_second.parse().map_err(|_| "Invalid jupiter_max_requests_per_second")?,
            rate_limiting_enabled: self.rate_limiting_enabled,
            wallet_low_balance_alert_sol: self.wallet_low_balance_alert_sol.parse().map_err(|_| "Invalid wallet_low_balance_alert_sol")?,
            wallet_monitor_interval_secs: self.wallet_monitor_interval_secs.parse().map_err(|_| "Invalid wallet_monitor_interval_secs")?,
            wallet_reserve_for_fees_sol: self.wallet_reserve_for_fees_sol.parse().map_err(|_| "Invalid wallet_reserve_for_fees_sol")?,
            max_concurrent_trades: self.max_concurrent_trades.parse().map_err(|_| "Invalid max_concurrent_trades")?,
            max_open_positions: self.max_open_positions.parse().map_err(|_| "Invalid max_open_positions")?,
            total_exposure_limit_sol: self.total_exposure_limit_sol.parse().map_err(|_| "Invalid total_exposure_limit_sol")?,
            dip_strategy_enabled: self.dip_strategy_enabled,
            dip_entry_pct: self.dip_entry_pct.parse().map_err(|_| "Invalid dip_entry_pct")?,
            min_volume_usd_5m: self.min_volume_usd_5m.parse().map_err(|_| "Invalid min_volume_usd_5m")?,
            watchlist_timeout_seconds: self.watchlist_timeout_seconds.parse().map_err(|_| "Invalid watchlist_timeout_seconds")?,
            // Enhanced Entry Validation
            volume_trend_enabled: self.volume_trend_enabled,
            volume_samples_required: self.volume_samples_required.parse().map_err(|_| "Invalid volume_samples_required")?,
            volume_sample_interval_secs: self.volume_sample_interval_secs.parse().map_err(|_| "Invalid volume_sample_interval_secs")?,
            holder_stability_enabled: self.holder_stability_enabled,
            min_holder_retention_pct: self.min_holder_retention_pct.parse().map_err(|_| "Invalid min_holder_retention_pct")?,
            // Trailing Stop Loss
            trailing_stop_enabled: self.trailing_stop_enabled,
            trailing_stop_distance_pct: self.trailing_stop_distance_pct.parse().map_err(|_| "Invalid trailing_stop_distance_pct")?,
            // Partial Exits
            partial_exit_enabled: self.partial_exit_enabled,
            partial_exit_target_pct: self.partial_exit_target_pct.parse().map_err(|_| "Invalid partial_exit_target_pct")?,
            partial_exit_amount_pct: self.partial_exit_amount_pct.parse().map_err(|_| "Invalid partial_exit_amount_pct")?,
            // Dynamic Timeout
            dynamic_timeout_enabled: self.dynamic_timeout_enabled,
            timeout_extension_seconds: self.timeout_extension_seconds.parse().map_err(|_| "Invalid timeout_extension_seconds")?,
            max_timeout_extensions: self.max_timeout_extensions.parse().map_err(|_| "Invalid max_timeout_extensions")?,
        })
    }
}

pub struct InvictusGUI {
    config: ConfigData,
    current_tab: Tab,
    theme: Theme,
    logs: Arc<Mutex<Vec<LogEntry>>>,
    event_receiver: Option<mpsc::UnboundedReceiver<BotEvent>>,
    bot_runtime: Option<BotRuntime>,
    bot_state: BotState,
    bot_metrics: BotMetrics,
    status_message: String,
    log_filter: Option<LogLevel>,
    auto_scroll: bool,
    positions: Vec<positions_tab::Position>,
    trades: Vec<trades_tab::Trade>,
}

#[derive(Clone, Copy, PartialEq)]
enum Tab {
    Dashboard,
    Configuration,
    Positions,
    Trades,
    Logs,
}

#[derive(Clone)]
pub struct LogEntry {
    pub timestamp: String,
    pub level: LogLevel,
    pub message: String,
}

#[derive(Clone, Copy, PartialEq)]
pub enum LogLevel {
    Info,
    Warn,
    Error,
    Debug,
}

impl InvictusGUI {
    pub fn new(_cc: &eframe::CreationContext<'_>, event_rx: mpsc::UnboundedReceiver<BotEvent>, event_tx: mpsc::UnboundedSender<BotEvent>) -> Self {
        let mut app = Self {
            config: ConfigData::default(),
            current_tab: Tab::Dashboard,
            theme: Theme::Dark,
            logs: Arc::new(Mutex::new(Vec::new())),
            event_receiver: Some(event_rx),
            bot_runtime: Some(BotRuntime::new(event_tx)),
            bot_state: BotState::Stopped,
            bot_metrics: BotMetrics::default(),
            status_message: String::new(),
            log_filter: None,
            auto_scroll: true,
            positions: Vec::new(),
            trades: Vec::new(),
        };
        
        // Try to load existing config
        app.load_config_from_env();
        
        app
    }
    
    fn add_log(&mut self, level: LogLevel, message: String) {
        let timestamp = Local::now().format("%H:%M:%S").to_string();
        let entry = LogEntry {
            timestamp,
            level,
            message,
        };
        
        if let Ok(mut logs) = self.logs.lock() {
            logs.push(entry);
            if logs.len() > 1000 {
                logs.remove(0);
            }
        }
    }
    
    pub fn save_config_to_env(&self) -> std::io::Result<()> {
        use std::io::Write;
        
        let mut file = std::fs::File::create(".env")?;
        
        writeln!(file, "# ============================================")?;
        writeln!(file, "# INVICTUS SNIPER BOT - ENVIRONMENT VARIABLES")?;
        writeln!(file, "# ============================================")?;
        writeln!(file)?;
        
        writeln!(file, "# Core Configuration")?;
        writeln!(file, "HELIUS_API_KEY={}", self.config.helius_api_key)?;
        writeln!(file, "BIRDEYE_API_KEY={}", self.config.birdeye_api_key)?;
        writeln!(file, "JUPITER_API_KEY={}", self.config.jupiter_api_key)?;
        writeln!(file, "SOLANA_RPC_URL={}", self.config.rpc_url)?;
        writeln!(file, "SOLANA_PRIVATE_KEY={}", self.config.solana_private_key)?;
        writeln!(file)?;
        
        writeln!(file, "# Telegram Bot")?;
        writeln!(file, "TELEGRAM_BOT_TOKEN={}", self.config.telegram_bot_token)?;
        writeln!(file, "TELEGRAM_CHAT_ID={}", self.config.telegram_chat_id)?;
        writeln!(file)?;
        
        writeln!(file, "# Database")?;
        writeln!(file, "DATABASE_URL={}", self.config.database_url)?;
        writeln!(file)?;
        
        writeln!(file, "# Risk Parameters")?;
        writeln!(file, "MIN_LIQUIDITY_SOL={}", self.config.min_liquidity_sol)?;
        writeln!(file, "MIN_HOLDERS={}", self.config.min_holders)?;
        writeln!(file, "MAX_TRADE_SIZE_SOL={}", self.config.max_trade_size_sol)?;
        writeln!(file, "MAX_DAILY_EXPOSURE_SOL={}", self.config.max_daily_exposure_sol)?;
        writeln!(file, "MAX_CREATOR_OWNERSHIP_PERCENTAGE={}", self.config.max_creator_ownership_percentage)?;
        writeln!(file, "HONEYPOT_CHECK_ENABLED={}", self.config.honeypot_check_enabled)?;
        writeln!(file, "JUPITER_API_TIMEOUT_MS={}", self.config.jupiter_api_timeout_ms)?;
        writeln!(file)?;
        
        writeln!(file, "# Auto-Sell Configuration")?;
        writeln!(file, "AUTO_SELL_ENABLED={}", self.config.auto_sell_enabled)?;
        writeln!(file, "AUTO_SELL_PROFIT_TARGET_PCT={}", self.config.auto_sell_profit_target_pct)?;
        writeln!(file, "AUTO_SELL_STOP_LOSS_PCT={}", self.config.auto_sell_stop_loss_pct)?;
        writeln!(file, "AUTO_SELL_TIMEOUT_SECONDS={}", self.config.auto_sell_timeout_seconds)?;
        writeln!(file, "AUTO_SELL_SLIPPAGE_BPS={}", self.config.auto_sell_slippage_bps)?;
        writeln!(file, "AUTO_SELL_PRICE_CHECK_INTERVAL_MS={}", self.config.auto_sell_price_check_interval_ms)?;
        writeln!(file)?;
        
        writeln!(file, "# Dynamic Jito Tips")?;
        writeln!(file, "JITO_DYNAMIC_TIPS_ENABLED={}", self.config.jito_dynamic_tips_enabled)?;
        writeln!(file, "JITO_BASE_TIP_LAMPORTS={}", self.config.jito_base_tip_lamports)?;
        writeln!(file, "JITO_MIN_TIP_LAMPORTS={}", self.config.jito_min_tip_lamports)?;
        writeln!(file, "JITO_MAX_TIP_LAMPORTS={}", self.config.jito_max_tip_lamports)?;
        writeln!(file)?;
        
        writeln!(file, "# Retry Logic")?;
        writeln!(file, "TX_RETRY_MAX_ATTEMPTS={}", self.config.tx_retry_max_attempts)?;
        writeln!(file, "TX_RETRY_INITIAL_DELAY_MS={}", self.config.tx_retry_initial_delay_ms)?;
        writeln!(file, "TX_RETRY_MAX_DELAY_MS={}", self.config.tx_retry_max_delay_ms)?;
        writeln!(file, "TX_RETRY_BACKOFF_MULTIPLIER={}", self.config.tx_retry_backoff_multiplier)?;
        writeln!(file)?;
        
        writeln!(file, "# Rate Limiting")?;
        writeln!(file, "RATE_LIMITING_ENABLED={}", self.config.rate_limiting_enabled)?;
        writeln!(file, "HELIUS_MAX_REQUESTS_PER_SECOND={}", self.config.helius_max_requests_per_second)?;
        writeln!(file, "JUPITER_MAX_REQUESTS_PER_SECOND={}", self.config.jupiter_max_requests_per_second)?;
        writeln!(file)?;
        
        writeln!(file, "# Wallet Monitoring")?;
        writeln!(file, "WALLET_LOW_BALANCE_ALERT_SOL={}", self.config.wallet_low_balance_alert_sol)?;
        writeln!(file, "WALLET_MONITOR_INTERVAL_SECS={}", self.config.wallet_monitor_interval_secs)?;
        writeln!(file, "WALLET_RESERVE_FOR_FEES_SOL={}", self.config.wallet_reserve_for_fees_sol)?;
        writeln!(file)?;
        
        writeln!(file, "# Parallel Trading")?;
        writeln!(file, "MAX_CONCURRENT_TRADES={}", self.config.max_concurrent_trades)?;
        writeln!(file, "MAX_OPEN_POSITIONS={}", self.config.max_open_positions)?;
        writeln!(file, "TOTAL_EXPOSURE_LIMIT_SOL={}", self.config.total_exposure_limit_sol)?;
        writeln!(file)?;
        
        writeln!(file, "# Dip Strategy")?;
        writeln!(file, "DIP_STRATEGY_ENABLED={}", self.config.dip_strategy_enabled)?;
        writeln!(file, "DIP_ENTRY_PCT={}", self.config.dip_entry_pct)?;
        writeln!(file, "MIN_VOLUME_USD_5M={}", self.config.min_volume_usd_5m)?;
        writeln!(file, "WATCHLIST_TIMEOUT_SECONDS={}", self.config.watchlist_timeout_seconds)?;
        writeln!(file)?;
        
        writeln!(file, "# Enhanced Entry Validation")?;
        writeln!(file, "VOLUME_TREND_ENABLED={}", self.config.volume_trend_enabled)?;
        writeln!(file, "VOLUME_SAMPLES_REQUIRED={}", self.config.volume_samples_required)?;
        writeln!(file, "VOLUME_SAMPLE_INTERVAL_SECS={}", self.config.volume_sample_interval_secs)?;
        writeln!(file, "HOLDER_STABILITY_ENABLED={}", self.config.holder_stability_enabled)?;
        writeln!(file, "MIN_HOLDER_RETENTION_PCT={}", self.config.min_holder_retention_pct)?;
        writeln!(file)?;
        
        writeln!(file, "# Trailing Stop Loss")?;
        writeln!(file, "TRAILING_STOP_ENABLED={}", self.config.trailing_stop_enabled)?;
        writeln!(file, "TRAILING_STOP_DISTANCE_PCT={}", self.config.trailing_stop_distance_pct)?;
        writeln!(file)?;
        
        writeln!(file, "# Partial Exits")?;
        writeln!(file, "PARTIAL_EXIT_ENABLED={}", self.config.partial_exit_enabled)?;
        writeln!(file, "PARTIAL_EXIT_TARGET_PCT={}", self.config.partial_exit_target_pct)?;
        writeln!(file, "PARTIAL_EXIT_AMOUNT_PCT={}", self.config.partial_exit_amount_pct)?;
        writeln!(file)?;
        
        writeln!(file, "# Dynamic Timeout")?;
        writeln!(file, "DYNAMIC_TIMEOUT_ENABLED={}", self.config.dynamic_timeout_enabled)?;
        writeln!(file, "TIMEOUT_EXTENSION_SECONDS={}", self.config.timeout_extension_seconds)?;
        writeln!(file, "MAX_TIMEOUT_EXTENSIONS={}", self.config.max_timeout_extensions)?;
        
        Ok(())
    }
    
    pub fn load_config_from_env(&mut self) {
        // Load .env file
        dotenv::dotenv().ok();
        
        // Helper to load env var into string field
        let load_str = |field: &mut String, key: &str| {
            if let Ok(val) = std::env::var(key) {
                *field = val;
            }
        };
        
        // Helper to load env var into bool field
        let load_bool = |field: &mut bool, key: &str| {
            if let Ok(val) = std::env::var(key) {
                if let Ok(parsed) = val.parse() {
                    *field = parsed;
                }
            }
        };

        load_str(&mut self.config.helius_api_key, "HELIUS_API_KEY");
        load_str(&mut self.config.birdeye_api_key, "BIRDEYE_API_KEY");
        load_str(&mut self.config.jupiter_api_key, "JUPITER_API_KEY");
        load_str(&mut self.config.rpc_url, "SOLANA_RPC_URL");
        load_str(&mut self.config.solana_private_key, "SOLANA_PRIVATE_KEY");
        load_str(&mut self.config.telegram_bot_token, "TELEGRAM_BOT_TOKEN");
        load_str(&mut self.config.telegram_chat_id, "TELEGRAM_CHAT_ID");
        load_str(&mut self.config.database_url, "DATABASE_URL");
        
        load_str(&mut self.config.min_liquidity_sol, "MIN_LIQUIDITY_SOL");
        load_str(&mut self.config.min_holders, "MIN_HOLDERS");
        load_str(&mut self.config.max_trade_size_sol, "MAX_TRADE_SIZE_SOL");
        load_str(&mut self.config.max_daily_exposure_sol, "MAX_DAILY_EXPOSURE_SOL");
        load_str(&mut self.config.max_creator_ownership_percentage, "MAX_CREATOR_OWNERSHIP_PERCENTAGE");
        
        load_bool(&mut self.config.honeypot_check_enabled, "HONEYPOT_CHECK_ENABLED");
        load_str(&mut self.config.jupiter_api_timeout_ms, "JUPITER_API_TIMEOUT_MS");
        
        load_bool(&mut self.config.auto_sell_enabled, "AUTO_SELL_ENABLED");
        load_str(&mut self.config.auto_sell_profit_target_pct, "AUTO_SELL_PROFIT_TARGET_PCT");
        load_str(&mut self.config.auto_sell_stop_loss_pct, "AUTO_SELL_STOP_LOSS_PCT");
        load_str(&mut self.config.auto_sell_timeout_seconds, "AUTO_SELL_TIMEOUT_SECONDS");
        load_str(&mut self.config.auto_sell_slippage_bps, "AUTO_SELL_SLIPPAGE_BPS");
        load_str(&mut self.config.auto_sell_price_check_interval_ms, "AUTO_SELL_PRICE_CHECK_INTERVAL_MS");
        
        load_bool(&mut self.config.jito_dynamic_tips_enabled, "JITO_DYNAMIC_TIPS_ENABLED");
        load_str(&mut self.config.jito_base_tip_lamports, "JITO_BASE_TIP_LAMPORTS");
        load_str(&mut self.config.jito_min_tip_lamports, "JITO_MIN_TIP_LAMPORTS");
        load_str(&mut self.config.jito_max_tip_lamports, "JITO_MAX_TIP_LAMPORTS");
        
        load_str(&mut self.config.tx_retry_max_attempts, "TX_RETRY_MAX_ATTEMPTS");
        load_str(&mut self.config.tx_retry_initial_delay_ms, "TX_RETRY_INITIAL_DELAY_MS");
        load_str(&mut self.config.tx_retry_max_delay_ms, "TX_RETRY_MAX_DELAY_MS");
        load_str(&mut self.config.tx_retry_backoff_multiplier, "TX_RETRY_BACKOFF_MULTIPLIER");
        
        load_bool(&mut self.config.rate_limiting_enabled, "RATE_LIMITING_ENABLED");
        load_str(&mut self.config.helius_max_requests_per_second, "HELIUS_MAX_REQUESTS_PER_SECOND");
        load_str(&mut self.config.jupiter_max_requests_per_second, "JUPITER_MAX_REQUESTS_PER_SECOND");
        
        load_str(&mut self.config.wallet_low_balance_alert_sol, "WALLET_LOW_BALANCE_ALERT_SOL");
        load_str(&mut self.config.wallet_monitor_interval_secs, "WALLET_MONITOR_INTERVAL_SECS");
        load_str(&mut self.config.wallet_reserve_for_fees_sol, "WALLET_RESERVE_FOR_FEES_SOL");
        
        load_str(&mut self.config.max_concurrent_trades, "MAX_CONCURRENT_TRADES");
        load_str(&mut self.config.max_open_positions, "MAX_OPEN_POSITIONS");
        load_str(&mut self.config.total_exposure_limit_sol, "TOTAL_EXPOSURE_LIMIT_SOL");
        
        load_bool(&mut self.config.dip_strategy_enabled, "DIP_STRATEGY_ENABLED");
        load_str(&mut self.config.dip_entry_pct, "DIP_ENTRY_PCT");
        load_str(&mut self.config.min_volume_usd_5m, "MIN_VOLUME_USD_5M");
        load_str(&mut self.config.watchlist_timeout_seconds, "WATCHLIST_TIMEOUT_SECONDS");
        
        load_bool(&mut self.config.volume_trend_enabled, "VOLUME_TREND_ENABLED");
        load_str(&mut self.config.volume_samples_required, "VOLUME_SAMPLES_REQUIRED");
        load_str(&mut self.config.volume_sample_interval_secs, "VOLUME_SAMPLE_INTERVAL_SECS");
        load_bool(&mut self.config.holder_stability_enabled, "HOLDER_STABILITY_ENABLED");
        load_str(&mut self.config.min_holder_retention_pct, "MIN_HOLDER_RETENTION_PCT");
        
        load_bool(&mut self.config.trailing_stop_enabled, "TRAILING_STOP_ENABLED");
        load_str(&mut self.config.trailing_stop_distance_pct, "TRAILING_STOP_DISTANCE_PCT");
        
        load_bool(&mut self.config.partial_exit_enabled, "PARTIAL_EXIT_ENABLED");
        load_str(&mut self.config.partial_exit_target_pct, "PARTIAL_EXIT_TARGET_PCT");
        load_str(&mut self.config.partial_exit_amount_pct, "PARTIAL_EXIT_AMOUNT_PCT");
        
        load_bool(&mut self.config.dynamic_timeout_enabled, "DYNAMIC_TIMEOUT_ENABLED");
        load_str(&mut self.config.timeout_extension_seconds, "TIMEOUT_EXTENSION_SECONDS");
        load_str(&mut self.config.max_timeout_extensions, "MAX_TIMEOUT_EXTENSIONS");
    }
}

impl eframe::App for InvictusGUI {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Process bot events - collect them first to avoid borrow checker issues
        let mut events = Vec::new();
        if let Some(receiver) = &mut self.event_receiver {
            while let Ok(event) = receiver.try_recv() {
                events.push(event);
            }
        }
        
        // Now process the collected events
        for event in events {
            match event {
                BotEvent::StateChanged(state) => {
                    self.bot_state = state;
                }
                BotEvent::MetricsUpdated(metrics) => {
                    self.bot_metrics = metrics;
                }
                BotEvent::LogMessage { level, message } => {
                    let log_level = match level.as_str() {
                        "WARN" => LogLevel::Warn,
                        "ERROR" => LogLevel::Error,
                        "DEBUG" => LogLevel::Debug,
                        _ => LogLevel::Info,
                    };
                    self.add_log(log_level, message);
                }
                BotEvent::Error(err) => {
                    self.add_log(LogLevel::Error, err);
                }
            }
        }
        
        // Update metrics if bot is running
        if self.bot_state == BotState::Running {
            if let Some(runtime) = &self.bot_runtime {
                self.bot_metrics = runtime.get_metrics();
            }
        }
        
        // Apply theme
        theme::configure_fonts(ctx);
        theme::apply_theme(ctx, self.theme);
        let colors = ThemeColors::from_theme(self.theme);
        
        // Top panel with title and theme toggle
        egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
            egui::Frame::none()
                .fill(colors.surface)
                .inner_margin(egui::Margin::symmetric(20.0, 15.0))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new("🚀 Invictus Sniper Bot")
                                .size(24.0)
                                .color(colors.primary)
                                .strong()
                        );
                        
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            // Theme toggle
                            let theme_icon = match self.theme {
                                Theme::Light => egui_phosphor::regular::MOON,
                                Theme::Dark => egui_phosphor::regular::SUN,
                            };
                            
                            if ui.button(egui::RichText::new(theme_icon).size(18.0)).clicked() {
                                self.theme.toggle();
                            }
                            
                            ui.add_space(20.0);
                            
                            // Status indicator
                            let (status_icon, status_color) = match self.bot_state {
                                BotState::Stopped => ("⭕", colors.text_secondary),
                                BotState::Starting => ("⏳", colors.warning),
                                BotState::Running => ("🟢", colors.success),
                                BotState::Stopping => ("⏳", colors.warning),
                                BotState::Error => ("🔴", colors.error),
                            };
                            
                            ui.label(egui::RichText::new(status_icon).size(16.0));
                            ui.label(
                                egui::RichText::new(format!("{:?}", self.bot_state))
                                    .size(14.0)
                                    .color(status_color)
                            );
                        });
                    });
                });
        });
        
        // Side panel for navigation
        egui::SidePanel::left("nav_panel")
            .resizable(false)
            .exact_width(200.0)
            .show(ctx, |ui| {
                egui::Frame::none()
                    .fill(colors.surface)
                    .inner_margin(egui::Margin::symmetric(10.0, 20.0))
                    .show(ui, |ui| {
                        ui.add_space(10.0);
                        
                        let tab_button = |ui: &mut egui::Ui, icon: &str, text: &str, tab: Tab, current: Tab| {
                            let is_selected = tab == current;
                            let bg_color = if is_selected { colors.primary } else { colors.surface };
                            let text_color = if is_selected { egui::Color32::WHITE } else { colors.text_primary };
                            
                            let button = egui::Button::new(
                                egui::RichText::new(format!("{} {}", icon, text))
                                    .size(15.0)
                                    .color(text_color)
                            )
                            .fill(bg_color)
                            .rounding(8.0)
                            .min_size(egui::vec2(180.0, 40.0));
                            
                            ui.add(button).clicked()
                        };
                        
                        if tab_button(ui, egui_phosphor::regular::CHART_BAR, "Dashboard", Tab::Dashboard, self.current_tab) {
                            self.current_tab = Tab::Dashboard;
                        }
                        ui.add_space(5.0);
                        
                        if tab_button(ui, egui_phosphor::regular::GEAR, "Configuration", Tab::Configuration, self.current_tab) {
                            self.current_tab = Tab::Configuration;
                        }
                        ui.add_space(5.0);
                        
                        if tab_button(ui, egui_phosphor::regular::TREND_UP, "Positions", Tab::Positions, self.current_tab) {
                            self.current_tab = Tab::Positions;
                        }
                        ui.add_space(5.0);
                        
                        if tab_button(ui, egui_phosphor::regular::LIST_DASHES, "Trades", Tab::Trades, self.current_tab) {
                            self.current_tab = Tab::Trades;
                        }
                        ui.add_space(5.0);
                        
                        if tab_button(ui, egui_phosphor::regular::TERMINAL_WINDOW, "Logs", Tab::Logs, self.current_tab) {
                            self.current_tab = Tab::Logs;
                        }
                    });
            });
        
        // Central panel for content
        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(colors.background).inner_margin(egui::Margin::same(20.0)))
            .show(ctx, |ui| {
                let mut start_bot = false;
                let mut stop_bot = false;
                
                match self.current_tab {
                    Tab::Dashboard => {
                        dashboard_tab::show(
                            ui,
                            &colors,
                            self.bot_state,
                            &self.bot_metrics,
                            &mut start_bot,
                            &mut stop_bot,
                        );
                    }
                    Tab::Configuration => {
                        config_tab::show(ui, self, &colors);
                    }
                    Tab::Positions => {
                        positions_tab::show(ui, &colors, &self.positions);
                    }
                    Tab::Trades => {
                        trades_tab::show(ui, &colors, &self.trades);
                    }
                    Tab::Logs => {
                        logs_tab::show(
                            ui,
                            &colors,
                            &self.logs,
                            &mut self.log_filter,
                            &mut self.auto_scroll,
                        );
                    }
                }
                
                // Handle bot start/stop
                if start_bot {
                    match self.config.to_config() {
                        Ok(config) => {
                            if let Some(runtime) = &mut self.bot_runtime {
                                match runtime.start(config) {
                                    Ok(_) => {
                                        self.add_log(LogLevel::Info, "Bot started successfully".to_string());
                                        self.status_message = "✅ Bot started".to_string();
                                    }
                                    Err(e) => {
                                        self.add_log(LogLevel::Error, format!("Failed to start bot: {}", e));
                                        self.status_message = format!("❌ Failed to start: {}", e);
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            self.add_log(LogLevel::Error, format!("Invalid configuration: {}", e));
                            self.status_message = format!("❌ Invalid config: {}", e);
                        }
                    }
                }
                
                if stop_bot {
                    if let Some(runtime) = &mut self.bot_runtime {
                        match runtime.stop() {
                            Ok(_) => {
                                self.add_log(LogLevel::Info, "Bot stopped successfully".to_string());
                                self.status_message = "✅ Bot stopped".to_string();
                            }
                            Err(e) => {
                                self.add_log(LogLevel::Error, format!("Failed to stop bot: {}", e));
                                self.status_message = format!("❌ Failed to stop: {}", e);
                            }
                        }
                    }
                }
            });
        
        // Request repaint for smooth animations
        ctx.request_repaint();
    }
}
