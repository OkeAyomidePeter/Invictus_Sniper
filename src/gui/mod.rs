pub mod config_tab;
pub mod logs_tab;
pub mod status_tab;
pub mod theme;

use eframe::egui;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

#[derive(Clone)]
pub struct ConfigData {
    // Core Configuration
    pub helius_api_key: String,
    pub solana_private_key: String,
    
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
}

impl Default for ConfigData {
    fn default() -> Self {
        Self {
            // Core Configuration
            helius_api_key: String::new(),
            solana_private_key: String::new(),
            
            // Telegram Bot
            telegram_bot_token: String::new(),
            telegram_chat_id: String::new(),
            
            // Database
            database_url: "sqlite://invictus.db".to_string(),
            
            // Risk Parameters
            min_liquidity_sol: "10.0".to_string(),
            min_holders: "10".to_string(),
            max_trade_size_sol: "1.0".to_string(),
            max_daily_exposure_sol: "10.0".to_string(),
            max_creator_ownership_percentage: "50.0".to_string(),
            honeypot_check_enabled: true,
            jupiter_api_timeout_ms: "5000".to_string(),
            
            // Auto-Sell Configuration
            auto_sell_enabled: true,
            auto_sell_profit_target_pct: "50.0".to_string(),
            auto_sell_stop_loss_pct: "20.0".to_string(),
            auto_sell_timeout_seconds: "120".to_string(),
            auto_sell_slippage_bps: "300".to_string(),
            auto_sell_price_check_interval_ms: "2000".to_string(),
            
            // Dynamic Jito Tips
            jito_dynamic_tips_enabled: true,
            jito_base_tip_lamports: "1000000".to_string(),
            jito_min_tip_lamports: "500000".to_string(),
            jito_max_tip_lamports: "5000000".to_string(),
            
            // Retry Logic
            tx_retry_max_attempts: "3".to_string(),
            tx_retry_initial_delay_ms: "500".to_string(),
            tx_retry_max_delay_ms: "5000".to_string(),
            tx_retry_backoff_multiplier: "2.0".to_string(),
            
            // Rate Limiting
            rate_limiting_enabled: true,
            helius_max_requests_per_second: "10.0".to_string(),
            jupiter_max_requests_per_second: "5.0".to_string(),
            
            // Wallet Monitoring
            wallet_low_balance_alert_sol: "0.5".to_string(),
            wallet_monitor_interval_secs: "60".to_string(),
            wallet_reserve_for_fees_sol: "0.1".to_string(),
            
            // Parallel Trading
            max_concurrent_trades: "5".to_string(),
            max_open_positions: "10".to_string(),
            total_exposure_limit_sol: "10.0".to_string(),
        }
    }
}

pub struct InvictusGUI {
    config: ConfigData,
    current_tab: Tab,
    logs: Arc<Mutex<Vec<LogEntry>>>,
    log_receiver: Option<mpsc::UnboundedReceiver<LogEntry>>,
    bot_status: BotStatus,
    status_message: String,
}

#[derive(Clone, Copy, PartialEq)]
enum Tab {
    Configuration,
    Logs,
    Status,
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

#[derive(Clone, Copy, PartialEq)]
enum BotStatus {
    Stopped,
    Running,
    Error,
}

impl InvictusGUI {
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        let (_log_tx, log_rx) = mpsc::unbounded_channel();
        
        Self {
            config: ConfigData::default(),
            current_tab: Tab::Configuration,
            logs: Arc::new(Mutex::new(Vec::new())),
            log_receiver: Some(log_rx),
            bot_status: BotStatus::Stopped,
            status_message: "Bot not started".to_string(),
        }
    }
    
    pub fn save_config_to_env(&self) -> std::io::Result<()> {
        use std::io::Write;
        
        let mut file = std::fs::File::create(".env")?;
        
        writeln!(file, "# ============================================")?;
        writeln!(file, "# INVICTUS SNIPER BOT - ENVIRONMENT VARIABLES")?;
        writeln!(file, "# ============================================")?;
        writeln!(file)?;
        
        writeln!(file, "# ----------------")?;
        writeln!(file, "# Core Configuration")?;
        writeln!(file, "# ----------------")?;
        writeln!(file, "HELIUS_API_KEY={}", self.config.helius_api_key)?;
        writeln!(file, "SOLANA_PRIVATE_KEY={}", self.config.solana_private_key)?;
        writeln!(file)?;
        
        writeln!(file, "# ----------------")?;
        writeln!(file, "# Telegram Bot")?;
        writeln!(file, "# ----------------")?;
        writeln!(file, "TELEGRAM_BOT_TOKEN={}", self.config.telegram_bot_token)?;
        writeln!(file, "TELEGRAM_CHAT_ID={}", self.config.telegram_chat_id)?;
        writeln!(file)?;
        
        writeln!(file, "# ----------------")?;
        writeln!(file, "# Database")?;
        writeln!(file, "# ----------------")?;
        writeln!(file, "DATABASE_URL={}", self.config.database_url)?;
        writeln!(file)?;
        
        writeln!(file, "# ----------------")?;
        writeln!(file, "# Risk Parameters")?;
        writeln!(file, "# ----------------")?;
        writeln!(file, "MIN_LIQUIDITY_SOL={}", self.config.min_liquidity_sol)?;
        writeln!(file, "MIN_HOLDERS={}", self.config.min_holders)?;
        writeln!(file, "MAX_TRADE_SIZE_SOL={}", self.config.max_trade_size_sol)?;
        writeln!(file, "MAX_DAILY_EXPOSURE_SOL={}", self.config.max_daily_exposure_sol)?;
        writeln!(file, "MAX_CREATOR_OWNERSHIP_PERCENTAGE={}", self.config.max_creator_ownership_percentage)?;
        writeln!(file, "HONEYPOT_CHECK_ENABLED={}", self.config.honeypot_check_enabled)?;
        writeln!(file, "JUPITER_API_TIMEOUT_MS={}", self.config.jupiter_api_timeout_ms)?;
        writeln!(file)?;
        
        writeln!(file, "# ----------------")?;
        writeln!(file, "# Auto-Sell Configuration")?;
        writeln!(file, "# ----------------")?;
        writeln!(file, "AUTO_SELL_ENABLED={}", self.config.auto_sell_enabled)?;
        writeln!(file, "AUTO_SELL_PROFIT_TARGET_PCT={}", self.config.auto_sell_profit_target_pct)?;
        writeln!(file, "AUTO_SELL_STOP_LOSS_PCT={}", self.config.auto_sell_stop_loss_pct)?;
        writeln!(file, "AUTO_SELL_TIMEOUT_SECONDS={}", self.config.auto_sell_timeout_seconds)?;
        writeln!(file, "AUTO_SELL_SLIPPAGE_BPS={}", self.config.auto_sell_slippage_bps)?;
        writeln!(file, "AUTO_SELL_PRICE_CHECK_INTERVAL_MS={}", self.config.auto_sell_price_check_interval_ms)?;
        writeln!(file)?;
        
        writeln!(file, "# ----------------")?;
        writeln!(file, "# Dynamic Jito Tips")?;
        writeln!(file, "# ----------------")?;
        writeln!(file, "JITO_DYNAMIC_TIPS_ENABLED={}", self.config.jito_dynamic_tips_enabled)?;
        writeln!(file, "JITO_BASE_TIP_LAMPORTS={}", self.config.jito_base_tip_lamports)?;
        writeln!(file, "JITO_MIN_TIP_LAMPORTS={}", self.config.jito_min_tip_lamports)?;
        writeln!(file, "JITO_MAX_TIP_LAMPORTS={}", self.config.jito_max_tip_lamports)?;
        writeln!(file)?;
        
        writeln!(file, "# ----------------")?;
        writeln!(file, "# Retry Logic")?;
        writeln!(file, "# ----------------")?;
        writeln!(file, "TX_RETRY_MAX_ATTEMPTS={}", self.config.tx_retry_max_attempts)?;
        writeln!(file, "TX_RETRY_INITIAL_DELAY_MS={}", self.config.tx_retry_initial_delay_ms)?;
        writeln!(file, "TX_RETRY_MAX_DELAY_MS={}", self.config.tx_retry_max_delay_ms)?;
        writeln!(file, "TX_RETRY_BACKOFF_MULTIPLIER={}", self.config.tx_retry_backoff_multiplier)?;
        writeln!(file)?;
        
        writeln!(file, "# ----------------")?;
        writeln!(file, "# Rate Limiting")?;
        writeln!(file, "# ----------------")?;
        writeln!(file, "RATE_LIMITING_ENABLED={}", self.config.rate_limiting_enabled)?;
        writeln!(file, "HELIUS_MAX_REQUESTS_PER_SECOND={}", self.config.helius_max_requests_per_second)?;
        writeln!(file, "JUPITER_MAX_REQUESTS_PER_SECOND={}", self.config.jupiter_max_requests_per_second)?;
        writeln!(file)?;
        
        writeln!(file, "# ----------------")?;
        writeln!(file, "# Wallet Monitoring")?;
        writeln!(file, "# ----------------")?;
        writeln!(file, "WALLET_LOW_BALANCE_ALERT_SOL={}", self.config.wallet_low_balance_alert_sol)?;
        writeln!(file, "WALLET_MONITOR_INTERVAL_SECS={}", self.config.wallet_monitor_interval_secs)?;
        writeln!(file, "WALLET_RESERVE_FOR_FEES_SOL={}", self.config.wallet_reserve_for_fees_sol)?;
        writeln!(file)?;
        
        writeln!(file, "# ----------------")?;
        writeln!(file, "# Parallel Trading")?;
        writeln!(file, "# ----------------")?;
        writeln!(file, "MAX_CONCURRENT_TRADES={}", self.config.max_concurrent_trades)?;
        writeln!(file, "MAX_OPEN_POSITIONS={}", self.config.max_open_positions)?;
        writeln!(file, "TOTAL_EXPOSURE_LIMIT_SOL={}", self.config.total_exposure_limit_sol)?;
        
        Ok(())
    }
    
    pub fn load_config_from_env(&mut self) {
        if let Ok(content) = std::fs::read_to_string(".env") {
            for line in content.lines() {
                if line.starts_with('#') || line.trim().is_empty() {
                    continue;
                }
                
                if let Some((key, value)) = line.split_once('=') {
                    let key = key.trim();
                    let value = value.trim();
                    
                    match key {
                        "HELIUS_API_KEY" => self.config.helius_api_key = value.to_string(),
                        "SOLANA_PRIVATE_KEY" => self.config.solana_private_key = value.to_string(),
                        "TELEGRAM_BOT_TOKEN" => self.config.telegram_bot_token = value.to_string(),
                        "TELEGRAM_CHAT_ID" => self.config.telegram_chat_id = value.to_string(),
                        "DATABASE_URL" => self.config.database_url = value.to_string(),
                        "MIN_LIQUIDITY_SOL" => self.config.min_liquidity_sol = value.to_string(),
                        "MIN_HOLDERS" => self.config.min_holders = value.to_string(),
                        "MAX_TRADE_SIZE_SOL" => self.config.max_trade_size_sol = value.to_string(),
                        "MAX_DAILY_EXPOSURE_SOL" => self.config.max_daily_exposure_sol = value.to_string(),
                        "MAX_CREATOR_OWNERSHIP_PERCENTAGE" => self.config.max_creator_ownership_percentage = value.to_string(),
                        "HONEYPOT_CHECK_ENABLED" => self.config.honeypot_check_enabled = value.parse().unwrap_or(true),
                        "JUPITER_API_TIMEOUT_MS" => self.config.jupiter_api_timeout_ms = value.to_string(),
                        "AUTO_SELL_ENABLED" => self.config.auto_sell_enabled = value.parse().unwrap_or(true),
                        "AUTO_SELL_PROFIT_TARGET_PCT" => self.config.auto_sell_profit_target_pct = value.to_string(),
                        "AUTO_SELL_STOP_LOSS_PCT" => self.config.auto_sell_stop_loss_pct = value.to_string(),
                        "AUTO_SELL_TIMEOUT_SECONDS" => self.config.auto_sell_timeout_seconds = value.to_string(),
                        "AUTO_SELL_SLIPPAGE_BPS" => self.config.auto_sell_slippage_bps = value.to_string(),
                        "AUTO_SELL_PRICE_CHECK_INTERVAL_MS" => self.config.auto_sell_price_check_interval_ms = value.to_string(),
                        "JITO_DYNAMIC_TIPS_ENABLED" => self.config.jito_dynamic_tips_enabled = value.parse().unwrap_or(true),
                        "JITO_BASE_TIP_LAMPORTS" => self.config.jito_base_tip_lamports = value.to_string(),
                        "JITO_MIN_TIP_LAMPORTS" => self.config.jito_min_tip_lamports = value.to_string(),
                        "JITO_MAX_TIP_LAMPORTS" => self.config.jito_max_tip_lamports = value.to_string(),
                        "TX_RETRY_MAX_ATTEMPTS" => self.config.tx_retry_max_attempts = value.to_string(),
                        "TX_RETRY_INITIAL_DELAY_MS" => self.config.tx_retry_initial_delay_ms = value.to_string(),
                        "TX_RETRY_MAX_DELAY_MS" => self.config.tx_retry_max_delay_ms = value.to_string(),
                        "TX_RETRY_BACKOFF_MULTIPLIER" => self.config.tx_retry_backoff_multiplier = value.to_string(),
                        "RATE_LIMITING_ENABLED" => self.config.rate_limiting_enabled = value.parse().unwrap_or(true),
                        "HELIUS_MAX_REQUESTS_PER_SECOND" => self.config.helius_max_requests_per_second = value.to_string(),
                        "JUPITER_MAX_REQUESTS_PER_SECOND" => self.config.jupiter_max_requests_per_second = value.to_string(),
                        "WALLET_LOW_BALANCE_ALERT_SOL" => self.config.wallet_low_balance_alert_sol = value.to_string(),
                        "WALLET_MONITOR_INTERVAL_SECS" => self.config.wallet_monitor_interval_secs = value.to_string(),
                        "WALLET_RESERVE_FOR_FEES_SOL" => self.config.wallet_reserve_for_fees_sol = value.to_string(),
                        "MAX_CONCURRENT_TRADES" => self.config.max_concurrent_trades = value.to_string(),
                        "MAX_OPEN_POSITIONS" => self.config.max_open_positions = value.to_string(),
                        "TOTAL_EXPOSURE_LIMIT_SOL" => self.config.total_exposure_limit_sol = value.to_string(),
                        _ => {}
                    }
                }
            }
        }
    }
}

impl eframe::App for InvictusGUI {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Poll for new log entries
        if let Some(receiver) = &mut self.log_receiver {
            while let Ok(entry) = receiver.try_recv() {
                if let Ok(mut logs) = self.logs.lock() {
                    logs.push(entry);
                    // Keep only last 1000 logs
                    // Keep only last 1000 logs
                    let len = logs.len();
                    if len > 1000 {
                        logs.drain(0..len - 1000);
                    }
                }
            }
        }
        
        theme::configure_fonts(ctx);
        
        egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("🚀 Invictus Sniper Bot - Configuration");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let status_text = match self.bot_status {
                        BotStatus::Stopped => "⭕ Stopped",
                        BotStatus::Running => "🟢 Running",
                        BotStatus::Error => "🔴 Error",
                    };
                    ui.label(status_text);
                });
            });
        });
        
        egui::TopBottomPanel::top("tab_bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.current_tab, Tab::Configuration, "⚙️ Configuration");
                ui.selectable_value(&mut self.current_tab, Tab::Logs, "📋 Logs");
                ui.selectable_value(&mut self.current_tab, Tab::Status, "📊 Status");
            });
        });
        
        egui::CentralPanel::default().show(ctx, |ui| {
            match self.current_tab {
                Tab::Configuration => config_tab::show(ui, self),
                Tab::Logs => logs_tab::show(ui, &self.logs),
                Tab::Status => status_tab::show(ui, &self.bot_status, &self.status_message),
            }
        });
    }
}
