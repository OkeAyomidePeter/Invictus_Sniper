use crate::monitoring::{MonitoringService, PerformanceSummary};
use crate::persistence::{Alert, DatabaseService};
use crate::pnl_portfolio::{PortfolioService, PortfolioSummary};
use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use log::{debug, error, info, warn};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use teloxide::{
    prelude::*,
    types::{
        ChatId, InlineKeyboardButton, InlineKeyboardMarkup, KeyboardButton, KeyboardMarkup,
        Message, ParseMode, ReplyMarkup,
    },
};
use tokio::sync::RwLock;

/// Telegram bot configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelegramConfig {
    pub bot_token: String,
    pub chat_id: String,
    pub enabled: bool,
    pub notification_level: NotificationLevel,
}

/// Notification level
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum NotificationLevel {
    All,       // All notifications
    Important, // Only important alerts
    Critical,  // Only critical alerts
    Disabled,  // No notifications
}

impl Default for NotificationLevel {
    fn default() -> Self {
        Self::Important
    }
}

/// Notification message
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationMessage {
    pub title: String,
    pub message: String,
    pub severity: String,
    pub timestamp: DateTime<Utc>,
    pub mint: Option<String>,
    pub signature: Option<String>,
    pub metadata: Option<HashMap<String, String>>,
}

/// Telegram notification service
#[derive(Clone)]
pub struct TelegramService {
    bot: Bot,
    config: TelegramConfig,
    db_service: DatabaseService,
    monitoring_service: Option<Arc<MonitoringService>>,
    portfolio_service: Option<Arc<PortfolioService>>,

    // Message formatting
    message_formatter: Arc<RwLock<MessageFormatter>>,
}

/// Message formatter for different types of notifications
#[derive(Debug)]
struct MessageFormatter {
    use_emoji: bool,
    use_markdown: bool,
    include_timestamps: bool,
}

impl Default for MessageFormatter {
    fn default() -> Self {
        Self {
            use_emoji: true,
            use_markdown: true,
            include_timestamps: true,
        }
    }
}

impl TelegramService {
    /// Create new Telegram service
    pub async fn new(config: TelegramConfig, db_service: DatabaseService) -> Result<Self> {
        if !config.enabled {
            info!("Telegram notifications disabled");
            return Ok(Self {
                bot: Bot::new(""),
                config,
                db_service,
                monitoring_service: None,
                portfolio_service: None,
                message_formatter: Arc::new(RwLock::new(MessageFormatter::default())),
            });
        }

        let bot = Bot::new(&config.bot_token);

        // Test bot connection with timeout
        match tokio::time::timeout(Duration::from_secs(5), bot.get_me().send()).await {
            Ok(Ok(me)) => {
                info!(
                    "Telegram bot connected: @{}",
                    me.user.username.as_ref().unwrap_or(&me.user.first_name)
                );
            }
            Ok(Err(e)) => {
                warn!("Failed to connect to Telegram bot: {}", e);
                warn!("Bot will continue without Telegram notifications");
                // Don't return error - allow bot to continue without Telegram
            }
            Err(_) => {
                warn!("Telegram bot connection timed out");
                warn!("Bot will continue without Telegram notifications");
                // Don't return error - allow bot to continue without Telegram
            }
        }

        Ok(Self {
            bot,
            config,
            db_service,
            monitoring_service: None,
            portfolio_service: None,
            message_formatter: Arc::new(RwLock::new(MessageFormatter::default())),
        })
    }

    /// Set monitoring service
    pub fn set_monitoring_service(&mut self, service: Arc<MonitoringService>) {
        self.monitoring_service = Some(service);
    }

    /// Set portfolio service
    pub fn set_portfolio_service(&mut self, service: Arc<PortfolioService>) {
        self.portfolio_service = Some(service);
    }

    /// Start the bot
    pub async fn start(&self) -> Result<()> {
        if !self.config.enabled {
            return Ok(());
        }

        info!("Starting Telegram bot service");

        let bot = self.bot.clone();
        let db_service = self.db_service.clone();
        let monitoring_service = self.monitoring_service.clone();
        let portfolio_service = self.portfolio_service.clone();

        // Create handler
        let telegram_service_clone = self.clone();
        let handler = dptree::entry().branch(Update::filter_message().endpoint(
            move |msg: Message, bot: Bot| {
                let db_service = db_service.clone();
                let monitoring_service = monitoring_service.clone();
                let portfolio_service = portfolio_service.clone();
                let telegram_service = telegram_service_clone.clone();

                async move {
                    // Create a temporary TelegramService instance for message handling
                    let temp_service = TelegramService {
                        bot: bot.clone(),
                        config: telegram_service.config.clone(),
                        db_service,
                        monitoring_service,
                        portfolio_service,
                        message_formatter: telegram_service.message_formatter.clone(),
                    };
                    temp_service.handle_message(msg).await
                }
            },
        ));

        // Run bot
        Dispatcher::builder(bot, handler).build().dispatch().await;

        Ok(())
    }

    /// Send notification
    pub async fn send_notification(&self, notification: NotificationMessage) -> Result<()> {
        if !self.config.enabled {
            return Ok(());
        }

        // Check notification level
        if !self.should_send_notification(&notification.severity) {
            debug!(
                "Skipping notification due to level filter: {}",
                notification.title
            );
            return Ok(());
        }

        let chat_id_val = self
            .config
            .chat_id
            .parse::<i64>()
            .map_err(|_| anyhow!("Invalid chat ID: {}", self.config.chat_id))?;
        let chat_id = ChatId(chat_id_val);

        let formatter = self.message_formatter.read().await;
        let message_text = self.format_notification(&notification, &formatter);

        // Send message
        self.bot
            .send_message(chat_id, message_text)
            .parse_mode(ParseMode::Html)
            .disable_web_page_preview(true)
            .send()
            .await
            .map_err(|e| anyhow!("Failed to send Telegram message: {}", e))?;

        info!("Telegram notification sent: {}", notification.title);
        Ok(())
    }

    /// Send portfolio summary
    pub async fn send_portfolio_summary(&self) -> Result<()> {
        if !self.config.enabled || self.portfolio_service.is_none() {
            return Ok(());
        }

        let portfolio_service = self.portfolio_service.as_ref().unwrap();
        let summary = portfolio_service.get_portfolio_summary().await?;

        let chat_id_val = self
            .config
            .chat_id
            .parse::<i64>()
            .map_err(|_| anyhow!("Invalid chat ID: {}", self.config.chat_id))?;
        let chat_id = ChatId(chat_id_val);

        let message = self.format_portfolio_summary(&summary).await?;

        self.bot
            .send_message(chat_id, message)
            .parse_mode(ParseMode::Html)
            .disable_web_page_preview(true)
            .send()
            .await
            .map_err(|e| anyhow!("Failed to send portfolio summary: {}", e))?;

        info!("Portfolio summary sent to Telegram");
        Ok(())
    }

    /// Send performance summary
    pub async fn send_performance_summary(&self) -> Result<()> {
        if !self.config.enabled || self.monitoring_service.is_none() {
            return Ok(());
        }

        let monitoring_service = self.monitoring_service.as_ref().unwrap();
        let summary = monitoring_service.get_performance_summary().await?;

        let chat_id_val = self
            .config
            .chat_id
            .parse::<i64>()
            .map_err(|_| anyhow!("Invalid chat ID: {}", self.config.chat_id))?;
        let chat_id = ChatId(chat_id_val);

        let message = self.format_performance_summary(&summary).await?;

        self.bot
            .send_message(chat_id, message)
            .parse_mode(ParseMode::Html)
            .disable_web_page_preview(true)
            .send()
            .await
            .map_err(|e| anyhow!("Failed to send performance summary: {}", e))?;

        info!("Performance summary sent to Telegram");
        Ok(())
    }

    /// Send alert from database
    pub async fn send_alert(&self, alert: &Alert) -> Result<()> {
        let notification = NotificationMessage {
            title: format!("🚨 {}", alert.alert_type),
            message: alert.message.clone(),
            severity: alert.severity.clone(),
            timestamp: alert.created_at,
            mint: alert.mint.clone(),
            signature: alert.signature.clone(),
            metadata: None,
        };

        self.send_notification(notification).await
    }

    /// Handle incoming message
    async fn handle_message(&self, msg: Message) -> Result<()> {
        if let Some(text) = msg.text() {
            let chat_id = msg.chat.id;

            match text {
                "/start" => {
                    let welcome_msg = r#"
🤖 <b>Sniper Bot Control Panel</b>

Welcome to the Sniper Bot Telegram interface!

Available commands:
• /status - System status
• /portfolio - Portfolio summary
• /performance - Performance metrics
• /alerts - Recent alerts
• /help - Show this help message

Use the buttons below for quick access:
                    "#;

                    self.bot
                        .send_message(chat_id, welcome_msg)
                        .parse_mode(ParseMode::Html)
                        .reply_markup(Self::create_main_keyboard())
                        .send()
                        .await?;
                }

                "/status" => {
                    let status_msg = if let Some(monitoring) = &self.monitoring_service {
                        let summary = monitoring.get_performance_summary().await?;
                        Self::format_system_status(&summary).await?
                    } else {
                        "❌ Monitoring service not available".to_string()
                    };

                    self.bot
                        .send_message(chat_id, status_msg)
                        .parse_mode(ParseMode::Html)
                        .send()
                        .await?;
                }

                "/portfolio" => {
                    let portfolio_msg = if let Some(portfolio) = &self.portfolio_service {
                        let summary = portfolio.get_portfolio_summary().await?;
                        self.format_portfolio_summary(&summary).await?
                    } else {
                        "❌ Portfolio service not available".to_string()
                    };

                    self.bot
                        .send_message(chat_id, portfolio_msg)
                        .parse_mode(ParseMode::Html)
                        .send()
                        .await?;
                }

                "/performance" => {
                    let performance_msg = if let Some(monitoring) = &self.monitoring_service {
                        let summary = monitoring.get_performance_summary().await?;
                        self.format_performance_summary(&summary).await?
                    } else {
                        "❌ Monitoring service not available".to_string()
                    };

                    self.bot
                        .send_message(chat_id, performance_msg)
                        .parse_mode(ParseMode::Html)
                        .send()
                        .await?;
                }

                "/alerts" => {
                    let alerts = self.db_service.get_unresolved_alerts().await?;
                    let alerts_msg = Self::format_alerts(&alerts).await?;

                    self.bot
                        .send_message(chat_id, alerts_msg)
                        .parse_mode(ParseMode::Html)
                        .send()
                        .await?;
                }

                "/help" | "❓ Help" => {
                    let help_msg = r#"
📖 <b>Help & Commands</b>

<b>System Commands:</b>
• /status - View system health and status
• /performance - Performance metrics and statistics

<b>Portfolio Commands:</b>
• /portfolio - Current portfolio summary
• /top - Top performing tokens
• /worst - Worst performing tokens

<b>Alerts & Monitoring:</b>
• /alerts - View recent alerts
• /clear_alerts - Mark all alerts as resolved

<b>Other:</b>
• /help - Show this help message
• /start - Return to main menu

<b>Keyboard Navigation:</b>
Use the inline buttons below for quick access to all features.
                    "#;

                    self.bot
                        .send_message(chat_id, help_msg)
                        .parse_mode(ParseMode::Html)
                        .reply_markup(Self::create_main_keyboard())
                        .send()
                        .await?;
                }

                "📊 Status" => {
                    let status_msg = if let Some(monitoring) = &self.monitoring_service {
                        let summary = monitoring.get_performance_summary().await?;
                        Self::format_system_status(&summary).await?
                    } else {
                        "❌ Monitoring service not available".to_string()
                    };

                    self.bot
                        .send_message(chat_id, status_msg)
                        .parse_mode(ParseMode::Html)
                        .send()
                        .await?;
                }

                "💰 Portfolio" => {
                    let portfolio_msg = if let Some(portfolio) = &self.portfolio_service {
                        let summary = portfolio.get_portfolio_summary().await?;
                        self.format_portfolio_summary(&summary).await?
                    } else {
                        "❌ Portfolio service not available".to_string()
                    };

                    self.bot
                        .send_message(chat_id, portfolio_msg)
                        .parse_mode(ParseMode::Html)
                        .send()
                        .await?;
                }

                "📈 Performance" => {
                    let performance_msg = if let Some(monitoring) = &self.monitoring_service {
                        let summary = monitoring.get_performance_summary().await?;
                        self.format_performance_summary(&summary).await?
                    } else {
                        "❌ Monitoring service not available".to_string()
                    };

                    self.bot
                        .send_message(chat_id, performance_msg)
                        .parse_mode(ParseMode::Html)
                        .send()
                        .await?;
                }

                "🚨 Alerts" => {
                    let alerts = self.db_service.get_unresolved_alerts().await?;
                    let alerts_msg = Self::format_alerts(&alerts).await?;

                    self.bot
                        .send_message(chat_id, alerts_msg)
                        .parse_mode(ParseMode::Html)
                        .send()
                        .await?;
                }

                _ => {
                    self.bot
                        .send_message(
                            chat_id,
                            "❓ Unknown command. Use /help for available commands.",
                        )
                        .send()
                        .await?;
                }
            }
        }

        Ok(())
    }

    /// Check if notification should be sent based on level
    fn should_send_notification(&self, severity: &str) -> bool {
        match self.config.notification_level {
            NotificationLevel::All => true,
            NotificationLevel::Important => matches!(severity, "warning" | "error" | "critical"),
            NotificationLevel::Critical => matches!(severity, "error" | "critical"),
            NotificationLevel::Disabled => false,
        }
    }

    /// Format notification message
    fn format_notification(
        &self,
        notification: &NotificationMessage,
        formatter: &MessageFormatter,
    ) -> String {
        let emoji = match notification.severity.as_str() {
            "info" => "ℹ️",
            "warning" => "⚠️",
            "error" => "❌",
            "critical" => "🚨",
            _ => "📢",
        };

        let mut message = format!("{} <b>{}</b>\n\n", emoji, notification.title);
        message.push_str(&notification.message);

        if let Some(mint) = &notification.mint {
            message.push_str(&format!("\n\n<b>Token:</b> <code>{}</code>", mint));
        }

        if let Some(signature) = &notification.signature {
            message.push_str(&format!("\n<b>Signature:</b> <code>{}</code>", signature));
        }

        if formatter.include_timestamps {
            message.push_str(&format!(
                "\n\n<b>Time:</b> {}",
                notification.timestamp.format("%Y-%m-%d %H:%M:%S UTC")
            ));
        }

        message
    }

    /// Format portfolio summary
    async fn format_portfolio_summary(&self, summary: &PortfolioSummary) -> Result<String> {
        let mut message = String::new();
        message.push_str("📊 <b>Portfolio Summary</b>\n\n");

        message.push_str(&format!(
            "💰 <b>Total Value:</b> {:.4} SOL\n",
            summary.total_value
        ));
        message.push_str(&format!(
            "💵 <b>Total Invested:</b> {:.4} SOL\n",
            summary.total_invested
        ));
        message.push_str(&format!(
            "📈 <b>Total PnL:</b> {:.4} SOL ({:.2}%)\n\n",
            summary.total_pnl, summary.total_pnl_percentage
        ));

        message.push_str(&format!("🪙 <b>Token Count:</b> {}\n", summary.token_count));

        if let Some(best) = &summary.best_performer {
            message.push_str(&format!(
                "🏆 <b>Best:</b> {} ({:.2}%)\n",
                best.symbol.as_ref().unwrap_or(&best.mint[..8].to_string()),
                best.pnl_percentage
            ));
        }

        if let Some(worst) = &summary.worst_performer {
            message.push_str(&format!(
                "📉 <b>Worst:</b> {} ({:.2}%)\n",
                worst
                    .symbol
                    .as_ref()
                    .unwrap_or(&worst.mint[..8].to_string()),
                worst.pnl_percentage
            ));
        }

        message.push_str(&format!(
            "\n🕐 <b>Updated:</b> {}",
            summary.last_updated.format("%H:%M:%S UTC")
        ));

        Ok(message)
    }

    /// Format performance summary
    async fn format_performance_summary(&self, summary: &PerformanceSummary) -> Result<String> {
        let mut message = String::new();
        message.push_str("📈 <b>Performance Summary</b>\n\n");

        if let Some(trading) = &summary.trading_metrics {
            message.push_str("💹 <b>Trading Metrics:</b>\n");
            message.push_str(&format!("• Success Rate: {:.2}%\n", trading.success_rate));
            message.push_str(&format!(
                "• Avg Execution Time: {:.1}ms\n",
                trading.average_execution_time
            ));
            message.push_str(&format!(
                "• Volume 24h: {:.4} SOL\n",
                trading.total_volume_24h
            ));
            message.push_str(&format!("• Pending: {}\n", trading.pending_transactions));
            message.push_str(&format!(
                "• Failed 24h: {}\n\n",
                trading.failed_transactions_24h
            ));
        }

        if let Some(system) = &summary.system_health {
            message.push_str("🖥️ <b>System Health:</b>\n");
            message.push_str(&format!("• CPU: {:.1}%\n", system.cpu_usage));
            message.push_str(&format!("• Memory: {:.1}%\n", system.memory_usage));
            message.push_str(&format!(
                "• Database: {}\n",
                if system.database_connected {
                    "✅ Connected"
                } else {
                    "❌ Disconnected"
                }
            ));
            message.push_str(&format!(
                "• Active Connections: {}\n\n",
                system.active_connections
            ));
        }

        message.push_str("🔄 <b>Relayer Status:</b>\n");
        for (name, health) in &summary.relayer_health {
            let status = if health.is_available { "✅" } else { "❌" };
            message.push_str(&format!(
                "• {}: {} ({:.1}% success)\n",
                name, status, health.success_rate
            ));
        }

        message.push_str(&format!(
            "\n🕐 <b>Updated:</b> {}",
            summary.last_updated.format("%H:%M:%S UTC")
        ));

        Ok(message)
    }

    /// Format system status
    async fn format_system_status(summary: &PerformanceSummary) -> Result<String> {
        let mut message = String::new();
        message.push_str("🖥️ <b>System Status</b>\n\n");

        if let Some(system) = &summary.system_health {
            message.push_str("💻 <b>Resources:</b>\n");
            message.push_str(&format!("• CPU Usage: {:.1}%\n", system.cpu_usage));
            message.push_str(&format!("• Memory Usage: {:.1}%\n", system.memory_usage));
            message.push_str(&format!("• Disk Usage: {:.1}%\n", system.disk_usage));
            message.push_str(&format!(
                "• Network Latency: {:.1}ms\n\n",
                system.network_latency
            ));

            message.push_str("🔗 <b>Connections:</b>\n");
            message.push_str(&format!(
                "• Database: {}\n",
                if system.database_connected {
                    "✅ Connected"
                } else {
                    "❌ Disconnected"
                }
            ));
            message.push_str(&format!(
                "• Active Connections: {}\n\n",
                system.active_connections
            ));
        }

        message.push_str("📊 <b>Performance:</b>\n");
        message.push_str(&format!(
            "• Success Rate (1h): {:.2}%\n",
            summary.success_rate_last_hour
        ));
        message.push_str(&format!(
            "• Avg Transaction Time: {:.1}ms\n",
            summary.average_transaction_time_ms
        ));

        message.push_str(&format!(
            "\n🕐 <b>Updated:</b> {}",
            summary.last_updated.format("%H:%M:%S UTC")
        ));

        Ok(message)
    }

    /// Format alerts
    async fn format_alerts(alerts: &[Alert]) -> Result<String> {
        if alerts.is_empty() {
            return Ok(
                "✅ <b>No unresolved alerts</b>\n\nAll systems operating normally\\!".to_string(),
            );
        }

        let mut message = format!("🚨 <b>Unresolved Alerts</b> ({})\n\n", alerts.len());

        for (i, alert) in alerts.iter().enumerate().take(10) {
            let emoji = match alert.severity.as_str() {
                "info" => "ℹ️",
                "warning" => "⚠️",
                "error" => "❌",
                "critical" => "🚨",
                _ => "📢",
            };

            message.push_str(&format!("{} <b>{}</b>\n", emoji, alert.alert_type));
            message.push_str(&format!("{}\n", alert.message));

            if let Some(mint) = &alert.mint {
                message.push_str(&format!("Token: <code>{}</code>\n", mint));
            }

            message.push_str(&format!(
                "Created: {}\n\n",
                alert.created_at.format("%Y-%m-%d %H:%M:%S UTC")
            ));
        }

        if alerts.len() > 10 {
            message.push_str(&format!("... and {} more alerts\n", alerts.len() - 10));
        }

        Ok(message)
    }

    /// Create main keyboard
    fn create_main_keyboard() -> ReplyMarkup {
        ReplyMarkup::Keyboard(
            KeyboardMarkup::new([
                vec![
                    KeyboardButton::new("📊 Status"),
                    KeyboardButton::new("💰 Portfolio"),
                ],
                vec![
                    KeyboardButton::new("📈 Performance"),
                    KeyboardButton::new("🚨 Alerts"),
                ],
                vec![KeyboardButton::new("❓ Help")],
            ])
            .resize_keyboard(true),
        )
    }
}

/// Notification scheduler for periodic updates
pub struct NotificationScheduler {
    telegram_service: Arc<TelegramService>,
    portfolio_service: Option<Arc<PortfolioService>>,
    monitoring_service: Option<Arc<MonitoringService>>,
}

impl NotificationScheduler {
    /// Create new notification scheduler
    pub fn new(
        telegram_service: Arc<TelegramService>,
        portfolio_service: Option<Arc<PortfolioService>>,
        monitoring_service: Option<Arc<MonitoringService>>,
    ) -> Self {
        Self {
            telegram_service,
            portfolio_service,
            monitoring_service,
        }
    }

    /// Start scheduled notifications
    pub async fn start_scheduled_notifications(&self) -> Result<()> {
        info!("Starting scheduled notifications");

        // Hourly portfolio summary
        let portfolio_service = self.portfolio_service.clone();
        let telegram_service_clone1 = self.telegram_service.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(3600)); // 1 hour

            loop {
                interval.tick().await;

                if let Err(e) = telegram_service_clone1.send_portfolio_summary().await {
                    error!("Failed to send scheduled portfolio summary: {}", e);
                }
            }
        });

        // Daily performance summary
        let monitoring_service = self.monitoring_service.clone();
        let telegram_service_clone2 = self.telegram_service.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(86400)); // 24 hours

            loop {
                interval.tick().await;

                if let Err(e) = telegram_service_clone2.send_performance_summary().await {
                    error!("Failed to send scheduled performance summary: {}", e);
                }
            }
        });

        info!("Scheduled notifications started");
        Ok(())
    }
}
