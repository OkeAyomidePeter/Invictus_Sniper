use crate::config::Config;
use crate::db::Database;
use crate::wallet_monitor::WalletMonitor;
use log::{info, warn};
use std::sync::Arc;
use teloxide::{
    dispatching::UpdateHandler,
    prelude::*,
    types::{KeyboardButton, KeyboardMarkup, ParseMode, ReplyMarkup},
    utils::command::BotCommands,
    RequestError,
};
use tokio::sync::mpsc::Sender;

type ResponseResult<T> = Result<T, RequestError>;

#[derive(Clone)]
struct AllowedChatId(i64);

#[derive(BotCommands, Clone)]
#[command(rename = "lowercase")]
enum Command {
    Start,
    Stats,
    Kill,
    Help,
}

#[derive(Clone)]
pub struct TelegramInterface {
    bot: Arc<Bot>,
    db: Arc<Database>,
    shutdown_tx: Sender<()>,
    allowed_chat_id: i64,
    wallet_monitor: Option<Arc<WalletMonitor>>,
}

impl TelegramInterface {
    pub fn new(config: &Config, db: Arc<Database>, shutdown_tx: Sender<()>, wallet_monitor: Option<Arc<WalletMonitor>>) -> Self {
        let bot = Arc::new(Bot::new(&config.telegram_token));
        let allowed_chat_id = config.telegram_chat_id.parse::<i64>().unwrap_or(0);
        
        Self {
            bot,
            db,
            shutdown_tx,
            allowed_chat_id,
            wallet_monitor,
        }
    }

    /// Send auto-sell notification
    pub async fn notify_auto_sell(
        &self,
        mint: &str,
        trigger: &str,
        pnl_percentage: f64,
        entry_price: f64,
        exit_price: f64,
        pnl_sol: f64,
        bundle_id: &str,
    ) {
        if self.allowed_chat_id == 0 {
            return; // Telegram not configured
        }

        let emoji = if pnl_sol > 0.0 { "✅" } else { "🛑" };
        let pnl_sign = if pnl_sol > 0.0 { "+" } else { "" };

        let message = format!(
            "🎯 <b>AUTO-SELL EXECUTED</b>\n\n\
            Mint: <code>{}</code>\n\
            Trigger: {} {}\n\
            Entry: {:.10} SOL/token\n\
            Exit: {:.10} SOL/token\n\
            P/L: {}{:.4} SOL ({}{:.2}%)\n\
            Bundle: <code>{}</code>",
            &mint[..16.min(mint.len())],
            emoji,
            trigger,
            entry_price,
            exit_price,
            pnl_sign,
            pnl_sol,
            pnl_sign,
            pnl_percentage,
            &bundle_id[..20.min(bundle_id.len())]
        );

        if let Err(e) = self
            .bot
            .send_message(ChatId(self.allowed_chat_id), message)
            .parse_mode(ParseMode::Html)
            .send()
            .await
        {
            warn!("Failed to send auto-sell notification: {}", e);
        }
    }

    pub async fn run(self) {
        info!("🤖 Starting Telegram Bot...");

        let command_handler = Update::filter_message()
            .filter_command::<Command>()
            .endpoint(command_handler);

        let message_handler = Update::filter_message().endpoint(message_handler);

        Dispatcher::builder(
            self.bot.as_ref().clone(),
            teloxide::dptree::entry()
                .branch(command_handler)
                .branch(message_handler),
        )
        .dependencies(teloxide::dptree::deps![
            self.db,
            self.shutdown_tx,
            AllowedChatId(self.allowed_chat_id),
            self.wallet_monitor
        ])
        .build()
        .dispatch()
        .await;
    }
}

async fn command_handler(
    bot: Bot,
    msg: Message,
    cmd: Command,
    db: Arc<Database>,
    shutdown_tx: Sender<()>,
    allowed_chat_id: AllowedChatId,
) -> ResponseResult<()> {
    if msg.chat.id.0 != allowed_chat_id.0 {
        warn!("⚠️ Unauthorized access from chat ID: {}", msg.chat.id);
        return Ok(());
    }

    match cmd {
        Command::Start => {
            let welcome_msg = r#"🤖 <b>Invictus Bot Control Panel</b>

Welcome to the Invictus Bot Telegram interface!

Available commands:
• 📊 Stats - View bot statistics
• 📜 Recent Trades - See recent trades
• 🏆 Top Tokens - View top tokens
• 💀 Kill Bot - Shutdown the bot
• ❓ Help - Show this help message

Use the buttons below for quick access:
                    "#;
            
            bot.send_message(msg.chat.id, welcome_msg)
                .parse_mode(ParseMode::Html)
                .reply_markup(create_main_keyboard())
                .send()
                .await?;
        }
        Command::Stats => {
            send_stats(&bot, msg.chat.id, &db).await?;
        }
        Command::Kill => {
            let keyboard = ReplyMarkup::Keyboard(
                KeyboardMarkup::new([
                    vec![
                        KeyboardButton::new("✅ Confirm Kill"),
                        KeyboardButton::new("❌ Cancel"),
                    ],
                ])
                .resize_keyboard(true),
            );
            
            bot.send_message(msg.chat.id, "⚠️ <b>Are you sure you want to shutdown the bot?</b>")
                .parse_mode(ParseMode::Html)
                .reply_markup(keyboard)
                .send()
                .await?;
        }
        Command::Help => {
            let help_msg = r#"📖 <b>Help & Commands</b>

<b>Bot Commands:</b>
• 📊 Stats - View token and trade statistics
• 📜 Recent Trades - See the last 5 trades
• 🏆 Top Tokens - View top 5 tokens by score
• 💀 Kill Bot - Safely shutdown the bot
• ❓ Help - Show this help message

<b>Keyboard Navigation:</b>
Use the persistent keyboard buttons below for quick access to all features.
                    "#;
            
            bot.send_message(msg.chat.id, help_msg)
                .parse_mode(ParseMode::Html)
                .reply_markup(create_main_keyboard())
                .send()
                .await?;
        }
    }
    Ok(())
}

// The original callback_handler is removed as its functionality is now integrated into message_handler
// async fn callback_handler(...) { ... }

async fn message_handler(
    bot: Bot,
    msg: Message,
    db: Arc<Database>,
    shutdown_tx: Sender<()>,
    allowed_chat_id: AllowedChatId,
    wallet_monitor: Option<Arc<WalletMonitor>>,
) -> ResponseResult<()> {
    if msg.chat.id.0 != allowed_chat_id.0 {
        warn!("⚠️ Unauthorized access from chat ID: {}", msg.chat.id);
        return Ok(());
    }

    if let Some(text) = msg.text() {
        match text {
            "📊 Stats" => {
                send_stats(&bot, msg.chat.id, &db).await?;
            }
            "📜 Recent Trades" => {
                let trades = db.get_recent_trades(5).await.unwrap_or_default();
                let mut text = "<b>📜 Recent Trades:</b>\n\n".to_string();
                if trades.is_empty() {
                    text.push_str("No trades yet.");
                } else {
                    for (action, mint, price) in trades {
                        text.push_str(&format!("• {} {}... @ {:.4} SOL\n", action, &mint[..8.min(mint.len())], price));
                    }
                }
                bot.send_message(msg.chat.id, text).parse_mode(ParseMode::Html).send().await?;
            }
            "🏆 Top Tokens" => {
                let tokens = db.get_top_tokens(5).await.unwrap_or_default();
                let mut text = "<b>🏆 Top Tokens:</b>\n\n".to_string();
                if tokens.is_empty() {
                    text.push_str("No tokens.");
                } else {
                    for (mint, score, liq) in tokens {
                        text.push_str(&format!("• {}... [{:.1}] {:.1} SOL\n", &mint[..8.min(mint.len())], score, liq));
                    }
                }
                bot.send_message(msg.chat.id, text).parse_mode(ParseMode::Html).send().await?;
            }
            "💰 Wallet Balance" => {
                if let Some(monitor) = &wallet_monitor {
                    match monitor.get_balance().await {
                        Ok(balance_lamports) => {
                            let balance_sol = balance_lamports as f64 / 1_000_000_000.0;
                            let available = monitor.get_available_balance().await.unwrap_or(0) as f64 / 1_000_000_000.0;
                            let text = format!(
                                "<b>💰 Wallet Balance</b>\n\n\
                                • <b>Total:</b> {:.4} SOL\n\
                                • <b>Available:</b> {:.4} SOL\n\
                                • <b>Reserved (fees):</b> {:.4} SOL",
                                balance_sol,
                                available,
                                balance_sol - available
                            );
                            bot.send_message(msg.chat.id, text).parse_mode(ParseMode::Html).send().await?;
                        }
                        Err(e) => {
                            bot.send_message(msg.chat.id, format!("❌ Failed to get balance: {}", e)).send().await?;
                        }
                    }
                } else {
                    bot.send_message(msg.chat.id, "❌ Wallet monitoring not enabled").send().await?;
                }
            }
            "📈 Active Positions" => {
                let positions = db.get_active_positions().await.unwrap_or_default();
                let mut text = "<b>📈 Active Positions:</b>\n\n".to_string();
                if positions.is_empty() {
                    text.push_str("No active positions.");
                } else {
                    for (mint, entry_price, amount, timestamp) in positions {
                        let elapsed = chrono::Utc::now().timestamp() - timestamp;
                        let minutes = elapsed / 60;
                        text.push_str(&format!(
                            "• {}...\n  Entry: {:.10} SOL\n  Amount: {} tokens\n  Time: {}m ago\n\n",
                            &mint[..8.min(mint.len())],
                            entry_price,
                            amount,
                            minutes
                        ));
                    }
                }
                bot.send_message(msg.chat.id, text).parse_mode(ParseMode::Html).send().await?;
            }
            "🔍 System Status" => {
                // Simple health check
                let token_count = db.get_token_count().await.unwrap_or(0);
                let trade_count = db.get_trade_count().await.unwrap_or(0);
                let wallet_status = if wallet_monitor.is_some() { "✅ Online" } else { "⚠️ Disabled" };
                
                let text = format!(
                    "<b>🔍 System Status</b>\n\n\
                    • <b>Database:</b> ✅ Connected ({} tokens, {} trades)\n\
                    • <b>Wallet Monitor:</b> {}\n\
                    • <b>Telegram:</b> ✅ Online\n\
                    • <b>Bot Core:</b> ✅ Running",
                    token_count,
                    trade_count,
                    wallet_status
                );
                bot.send_message(msg.chat.id, text).parse_mode(ParseMode::Html).send().await?;
            }
            "📊 Detailed Stats" => {
                send_detailed_stats(&bot, msg.chat.id, &db).await?;
            }
            "💀 Kill Bot" => {
                let keyboard = ReplyMarkup::Keyboard(
                    KeyboardMarkup::new([
                        vec![
                            KeyboardButton::new("✅ Confirm Kill"),
                            KeyboardButton::new("❌ Cancel"),
                        ],
                    ])
                    .resize_keyboard(true),
                );
                
                bot.send_message(msg.chat.id, "⚠️ <b>Are you sure you want to shutdown the bot?</b>")
                    .parse_mode(ParseMode::Html)
                    .reply_markup(keyboard)
                    .send()
                    .await?;
            }
            "✅ Confirm Kill" => {
                bot.send_message(msg.chat.id, "💀 <b>SHUTTING DOWN...</b>")
                    .parse_mode(ParseMode::Html)
                    .send()
                    .await?;
                info!("💀 Kill signal from Telegram!");
                let _ = shutdown_tx.send(()).await;
            }
            "❌ Cancel" => {
                bot.send_message(msg.chat.id, "✅ <b>Operation cancelled.</b>")
                    .parse_mode(ParseMode::Html)
                    .reply_markup(create_main_keyboard())
                    .send()
                    .await?;
            }
            "❓ Help" => {
                let help_msg = r#"📖 <b>Help & Commands</b>

<b>Bot Commands:</b>
• 📊 Stats - View token and trade statistics
• 📜 Recent Trades - See the last 5 trades
• 🏆 Top Tokens - View top 5 tokens by score
• 💀 Kill Bot - Safely shutdown the bot
• ❓ Help - Show this help message

<b>Keyboard Navigation:</b>
Use the persistent keyboard buttons below for quick access to all features.
                    "#;
                
                bot.send_message(msg.chat.id, help_msg)
                    .parse_mode(ParseMode::Html)
                    .reply_markup(create_main_keyboard())
                    .send()
                    .await?;
            }
            _ => {
                // Unknown text message, ignore
            }
        }
    }
    Ok(())
}

async fn send_stats(bot: &Bot, chat_id: ChatId, db: &Database) -> ResponseResult<()> {
    let token_count = db.get_token_count().await.unwrap_or(0);
    let trade_count = db.get_trade_count().await.unwrap_or(0);
    
    let text = format!(
        "<b>📊 Bot Statistics</b>\n\n\
        • <b>Tokens:</b> {}\n\
        • <b>Trades:</b> {}",
        token_count, trade_count
    );
    
    bot.send_message(chat_id, text).parse_mode(ParseMode::Html).send().await?;
    Ok(())
}

/// Create main menu keyboard
fn create_main_keyboard() -> ReplyMarkup {
    ReplyMarkup::Keyboard(
        KeyboardMarkup::new([
            vec![
                KeyboardButton::new("📊 Stats"),
                KeyboardButton::new("📊 Detailed Stats"),
            ],
            vec![
                KeyboardButton::new("📜 Recent Trades"),
                KeyboardButton::new("🏆 Top Tokens"),
            ],
            vec![
                KeyboardButton::new("💰 Wallet Balance"),
                KeyboardButton::new("📈 Active Positions"),
            ],
            vec![
                KeyboardButton::new("🔍 System Status"),
                KeyboardButton::new("💀 Kill Bot"),
            ],
            vec![KeyboardButton::new("❓ Help")],
        ])
        .resize_keyboard(true),
    )
}

/// Send detailed trade statistics
async fn send_detailed_stats(bot: &Bot, chat_id: ChatId, db: &Database) -> ResponseResult<()> {
    let (total_trades, closed_trades, total_pnl, win_rate, active_positions) = 
        db.get_trade_statistics().await.unwrap_or((0, 0, 0.0, 0.0, 0));
    
    let pnl_emoji = if total_pnl > 0.0 { "✅" } else if total_pnl < 0.0 { "🛑" } else { "➖" };
    let pnl_sign = if total_pnl > 0.0 { "+" } else { "" };
    
    let text = format!(
        "<b>📊 Detailed Statistics</b>\n\n\
        • <b>Total Trades:</b> {}\n\
        • <b>Closed Positions:</b> {}\n\
        • <b>Active Positions:</b> {}\n\
        • <b>Win Rate:</b> {:.1}%\n\
        • <b>Total P/L:</b> {} {}{:.4} SOL",
        total_trades,
        closed_trades,
        active_positions,
        win_rate,
        pnl_emoji,
        pnl_sign,
        total_pnl
    );
    
    bot.send_message(chat_id, text).parse_mode(ParseMode::Html).send().await?;
    Ok(())
}
