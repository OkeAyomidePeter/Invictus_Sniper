use super::{ConfigData, InvictusGUI};
use eframe::egui;

pub fn show(ui: &mut egui::Ui, app: &mut InvictusGUI) {
    egui::ScrollArea::vertical().show(ui, |ui| {
        ui.heading("⚙️ Bot Configuration");
        ui.separator();
        
        // Core Configuration
        ui.group(|ui| {
            ui.heading("🔑 Core Configuration");
            ui.add_space(5.0);
            
            ui.horizontal(|ui| {
                ui.label("Helius API Key:");
                ui.add(egui::TextEdit::singleline(&mut app.config.helius_api_key)
                    .password(true)
                    .hint_text("Enter your Helius API key")
                    .desired_width(300.0));
            });
            
            ui.horizontal(|ui| {
                ui.label("Solana Private Key:");
                ui.add(egui::TextEdit::singleline(&mut app.config.solana_private_key)
                    .password(true)
                    .hint_text("Base58 private key or path to keypair.json")
                    .desired_width(400.0));
            });
        });
        
        ui.add_space(10.0);
        
        // Telegram Bot
        ui.group(|ui| {
            ui.heading("📱 Telegram Bot");
            ui.add_space(5.0);
            
            ui.horizontal(|ui| {
                ui.label("Bot Token:");
                ui.add(egui::TextEdit::singleline(&mut app.config.telegram_bot_token)
                    .password(true)
                    .hint_text("Telegram bot token from @BotFather")
                    .desired_width(300.0));
            });
            
            ui.horizontal(|ui| {
                ui.label("Chat ID:");
                ui.add(egui::TextEdit::singleline(&mut app.config.telegram_chat_id)
                    .hint_text("Your Telegram chat ID")
                    .desired_width(150.0));
            });
        });
        
        ui.add_space(10.0);
        
        // Database
        ui.group(|ui| {
            ui.heading("💾 Database");
            ui.add_space(5.0);
            
            ui.horizontal(|ui| {
                ui.label("Database URL:");
                ui.add(egui::TextEdit::singleline(&mut app.config.database_url)
                    .hint_text("sqlite://invictus.db")
                    .desired_width(300.0));
            });
        });
        
        ui.add_space(10.0);
        
        // Risk Parameters
        ui.group(|ui| {
            ui.heading("⚠️ Risk Parameters");
            ui.add_space(5.0);
            
            ui.horizontal(|ui| {
                ui.label("Min Liquidity (SOL):");
                ui.add(egui::TextEdit::singleline(&mut app.config.min_liquidity_sol)
                    .desired_width(100.0));
                ui.label("Min Holders:");
                ui.add(egui::TextEdit::singleline(&mut app.config.min_holders)
                    .desired_width(100.0));
            });
            
            ui.horizontal(|ui| {
                ui.label("Max Trade Size (SOL):");
                ui.add(egui::TextEdit::singleline(&mut app.config.max_trade_size_sol)
                    .desired_width(100.0));
                ui.label("Max Daily Exposure (SOL):");
                ui.add(egui::TextEdit::singleline(&mut app.config.max_daily_exposure_sol)
                    .desired_width(100.0));
            });
            
            ui.horizontal(|ui| {
                ui.label("Max Creator Ownership (%):");
                ui.add(egui::TextEdit::singleline(&mut app.config.max_creator_ownership_percentage)
                    .desired_width(100.0));
                ui.label("Jupiter API Timeout (ms):");
                ui.add(egui::TextEdit::singleline(&mut app.config.jupiter_api_timeout_ms)
                    .desired_width(100.0));
            });
            
            ui.checkbox(&mut app.config.honeypot_check_enabled, "Enable Honeypot Check");
        });
        
        ui.add_space(10.0);
        
        // Auto-Sell Configuration
        ui.group(|ui| {
            ui.heading("🎯 Auto-Sell Configuration");
            ui.add_space(5.0);
            
            ui.checkbox(&mut app.config.auto_sell_enabled, "Enable Auto-Sell");
            
            ui.horizontal(|ui| {
                ui.label("Profit Target (%):");
                ui.add(egui::TextEdit::singleline(&mut app.config.auto_sell_profit_target_pct)
                    .desired_width(100.0));
                ui.label("Stop Loss (%):");
                ui.add(egui::TextEdit::singleline(&mut app.config.auto_sell_stop_loss_pct)
                    .desired_width(100.0));
            });
            
            ui.horizontal(|ui| {
                ui.label("Timeout (seconds):");
                ui.add(egui::TextEdit::singleline(&mut app.config.auto_sell_timeout_seconds)
                    .desired_width(100.0));
                ui.label("Slippage (BPS):");
                ui.add(egui::TextEdit::singleline(&mut app.config.auto_sell_slippage_bps)
                    .desired_width(100.0));
            });
            
            ui.horizontal(|ui| {
                ui.label("Price Check Interval (ms):");
                ui.add(egui::TextEdit::singleline(&mut app.config.auto_sell_price_check_interval_ms)
                    .desired_width(100.0));
            });
        });
        
        ui.add_space(10.0);
        
        // Dynamic Jito Tips
        ui.group(|ui| {
            ui.heading("⚡ Dynamic Jito Tips");
            ui.add_space(5.0);
            
            ui.checkbox(&mut app.config.jito_dynamic_tips_enabled, "Enable Dynamic Tips");
            
            ui.horizontal(|ui| {
                ui.label("Base Tip (lamports):");
                ui.add(egui::TextEdit::singleline(&mut app.config.jito_base_tip_lamports)
                    .desired_width(120.0));
                ui.label(format!("≈ {} SOL", 
                    app.config.jito_base_tip_lamports.parse::<f64>().unwrap_or(0.0) / 1_000_000_000.0));
            });
            
            ui.horizontal(|ui| {
                ui.label("Min Tip (lamports):");
                ui.add(egui::TextEdit::singleline(&mut app.config.jito_min_tip_lamports)
                    .desired_width(120.0));
                ui.label("Max Tip (lamports):");
                ui.add(egui::TextEdit::singleline(&mut app.config.jito_max_tip_lamports)
                    .desired_width(120.0));
            });
        });
        
        ui.add_space(10.0);
        
        // Retry Logic
        ui.group(|ui| {
            ui.heading("🔄 Retry Logic");
            ui.add_space(5.0);
            
            ui.horizontal(|ui| {
                ui.label("Max Retry Attempts:");
                ui.add(egui::TextEdit::singleline(&mut app.config.tx_retry_max_attempts)
                    .desired_width(80.0));
                ui.label("Backoff Multiplier:");
                ui.add(egui::TextEdit::singleline(&mut app.config.tx_retry_backoff_multiplier)
                    .desired_width(80.0));
            });
            
            ui.horizontal(|ui| {
                ui.label("Initial Delay (ms):");
                ui.add(egui::TextEdit::singleline(&mut app.config.tx_retry_initial_delay_ms)
                    .desired_width(100.0));
                ui.label("Max Delay (ms):");
                ui.add(egui::TextEdit::singleline(&mut app.config.tx_retry_max_delay_ms)
                    .desired_width(100.0));
            });
        });
        
        ui.add_space(10.0);
        
        // Rate Limiting
        ui.group(|ui| {
            ui.heading("🚦 Rate Limiting");
            ui.add_space(5.0);
            
            ui.checkbox(&mut app.config.rate_limiting_enabled, "Enable Rate Limiting");
            
            ui.horizontal(|ui| {
                ui.label("Helius Max RPS:");
                ui.add(egui::TextEdit::singleline(&mut app.config.helius_max_requests_per_second)
                    .desired_width(100.0));
                ui.label("Jupiter Max RPS:");
                ui.add(egui::TextEdit::singleline(&mut app.config.jupiter_max_requests_per_second)
                    .desired_width(100.0));
            });
        });
        
        ui.add_space(10.0);
        
        // Wallet Monitoring
        ui.group(|ui| {
            ui.heading("💰 Wallet Monitoring");
            ui.add_space(5.0);
            
            ui.horizontal(|ui| {
                ui.label("Low Balance Alert (SOL):");
                ui.add(egui::TextEdit::singleline(&mut app.config.wallet_low_balance_alert_sol)
                    .desired_width(100.0));
                ui.label("Reserve for Fees (SOL):");
                ui.add(egui::TextEdit::singleline(&mut app.config.wallet_reserve_for_fees_sol)
                    .desired_width(100.0));
            });
            
            ui.horizontal(|ui| {
                ui.label("Monitor Interval (seconds):");
                ui.add(egui::TextEdit::singleline(&mut app.config.wallet_monitor_interval_secs)
                    .desired_width(100.0));
            });
        });
        
        ui.add_space(10.0);
        
        // Parallel Trading
        ui.group(|ui| {
            ui.heading("🔀 Parallel Trading");
            ui.add_space(5.0);
            
            ui.horizontal(|ui| {
                ui.label("Max Concurrent Trades:");
                ui.add(egui::TextEdit::singleline(&mut app.config.max_concurrent_trades)
                    .desired_width(80.0));
                ui.label("Max Open Positions:");
                ui.add(egui::TextEdit::singleline(&mut app.config.max_open_positions)
                    .desired_width(80.0));
            });
            
            ui.horizontal(|ui| {
                ui.label("Total Exposure Limit (SOL):");
                ui.add(egui::TextEdit::singleline(&mut app.config.total_exposure_limit_sol)
                    .desired_width(100.0));
            });
        });
        
        ui.add_space(20.0);
        
        // Action Buttons
        ui.horizontal(|ui| {
            if ui.button("💾 Save Configuration").clicked() {
                match app.save_config_to_env() {
                    Ok(_) => {
                        app.status_message = "✅ Configuration saved to .env file".to_string();
                    }
                    Err(e) => {
                        app.status_message = format!("❌ Failed to save: {}", e);
                    }
                }
            }
            
            if ui.button("📂 Load Configuration").clicked() {
                app.load_config_from_env();
                app.status_message = "✅ Configuration loaded from .env file".to_string();
            }
            
            if ui.button("🔄 Reset to Defaults").clicked() {
                app.config = ConfigData::default();
                app.status_message = "✅ Configuration reset to defaults".to_string();
            }
        });
        
        ui.add_space(10.0);
        
        if !app.status_message.is_empty() {
            ui.colored_label(
                if app.status_message.starts_with("✅") {
                    egui::Color32::GREEN
                } else {
                    egui::Color32::RED
                },
                &app.status_message
            );
        }
    });
}
