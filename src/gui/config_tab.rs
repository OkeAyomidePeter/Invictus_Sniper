use super::{ConfigData, InvictusGUI};
use super::theme::ThemeColors;
use eframe::egui;

pub fn show(ui: &mut egui::Ui, app: &mut InvictusGUI, colors: &ThemeColors) {
    egui::ScrollArea::vertical().show(ui, |ui| {
        ui.add_space(10.0);
        
        ui.heading(egui::RichText::new("⚙️ Configuration").size(28.0).color(colors.text_primary));
        ui.add_space(20.0);
        
        // Core Configuration
        config_section(ui, colors, "🔑 Core Configuration", |ui| {
            config_field(ui, colors, "Helius API Key:", &mut app.config.helius_api_key, true, 350.0);
            config_field(ui, colors, "Solana Private Key:", &mut app.config.solana_private_key, true, 450.0);
        });
        
        ui.add_space(15.0);
        
        // Telegram Bot
        config_section(ui, colors, "📱 Telegram Bot", |ui| {
            config_field(ui, colors, "Bot Token:", &mut app.config.telegram_bot_token, true, 350.0);
            config_field(ui, colors, "Chat ID:", &mut app.config.telegram_chat_id, false, 200.0);
        });
        
        ui.add_space(15.0);
        
        // Risk Parameters
        config_section(ui, colors, "⚠️ Risk Parameters", |ui| {
            ui.horizontal(|ui| {
                config_field_inline(ui, colors, "Min Liquidity (SOL):", &mut app.config.min_liquidity_sol, 120.0);
                ui.add_space(20.0);
                config_field_inline(ui, colors, "Min Holders:", &mut app.config.min_holders, 120.0);
            });
            
            ui.add_space(10.0);
            
            ui.horizontal(|ui| {
                config_field_inline(ui, colors, "Max Trade Size (SOL):", &mut app.config.max_trade_size_sol, 120.0);
                ui.add_space(20.0);
                config_field_inline(ui, colors, "Max Daily Exposure (SOL):", &mut app.config.max_daily_exposure_sol, 120.0);
            });
            
            ui.add_space(10.0);
            
            ui.horizontal(|ui| {
                config_field_inline(ui, colors, "Max Creator Ownership (%):", &mut app.config.max_creator_ownership_percentage, 120.0);
                ui.add_space(20.0);
                config_field_inline(ui, colors, "Jupiter Timeout (ms):", &mut app.config.jupiter_api_timeout_ms, 120.0);
            });
            
            ui.add_space(10.0);
            ui.checkbox(&mut app.config.honeypot_check_enabled, 
                egui::RichText::new("Enable Honeypot Check").color(colors.text_primary));
        });
        
        ui.add_space(15.0);
        
        // Auto-Sell Configuration
        config_section(ui, colors, "🎯 Auto-Sell Configuration", |ui| {
            ui.checkbox(&mut app.config.auto_sell_enabled, 
                egui::RichText::new("Enable Auto-Sell").color(colors.text_primary));
            
            ui.add_space(10.0);
            
            ui.horizontal(|ui| {
                config_field_inline(ui, colors, "Profit Target (%):", &mut app.config.auto_sell_profit_target_pct, 120.0);
                ui.add_space(20.0);
                config_field_inline(ui, colors, "Stop Loss (%):", &mut app.config.auto_sell_stop_loss_pct, 120.0);
            });
            
            ui.add_space(10.0);
            
            ui.horizontal(|ui| {
                config_field_inline(ui, colors, "Timeout (seconds):", &mut app.config.auto_sell_timeout_seconds, 120.0);
                ui.add_space(20.0);
                config_field_inline(ui, colors, "Slippage (BPS):", &mut app.config.auto_sell_slippage_bps, 120.0);
            });
        });
        
        ui.add_space(15.0);
        
        // Jito Tips
        config_section(ui, colors, "⚡ Dynamic Jito Tips", |ui| {
            ui.checkbox(&mut app.config.jito_dynamic_tips_enabled, 
                egui::RichText::new("Enable Dynamic Tips").color(colors.text_primary));
            
            ui.add_space(10.0);
            
            ui.horizontal(|ui| {
                config_field_inline(ui, colors, "Base Tip (lamports):", &mut app.config.jito_base_tip_lamports, 150.0);
                
                // Show SOL conversion
                if let Ok(lamports) = app.config.jito_base_tip_lamports.parse::<f64>() {
                    ui.label(
                        egui::RichText::new(format!("≈ {:.4} SOL", lamports / 1_000_000_000.0))
                            .size(13.0)
                            .color(colors.text_secondary)
                    );
                }
            });
            
            ui.add_space(10.0);
            
            ui.horizontal(|ui| {
                config_field_inline(ui, colors, "Min Tip:", &mut app.config.jito_min_tip_lamports, 150.0);
                ui.add_space(20.0);
                config_field_inline(ui, colors, "Max Tip:", &mut app.config.jito_max_tip_lamports, 150.0);
            });
        });
        
        ui.add_space(15.0);
        
        // Wallet Monitoring
        config_section(ui, colors, "💰 Wallet Monitoring", |ui| {
            ui.horizontal(|ui| {
                config_field_inline(ui, colors, "Low Balance Alert (SOL):", &mut app.config.wallet_low_balance_alert_sol, 120.0);
                ui.add_space(20.0);
                config_field_inline(ui, colors, "Reserve for Fees (SOL):", &mut app.config.wallet_reserve_for_fees_sol, 120.0);
            });
        });
        
        ui.add_space(25.0);
        
        // Action Buttons
        ui.horizontal(|ui| {
            let save_button = egui::Button::new(
                egui::RichText::new("💾 Save Configuration")
                    .size(15.0)
                    .color(egui::Color32::WHITE)
            )
            .fill(colors.success)
            .rounding(8.0)
            .min_size(egui::vec2(180.0, 40.0));
            
            if ui.add(save_button).clicked() {
                match app.save_config_to_env() {
                    Ok(_) => {
                        app.status_message = "✅ Configuration saved to .env file".to_string();
                        app.add_log(super::LogLevel::Info, "Configuration saved".to_string());
                    }
                    Err(e) => {
                        app.status_message = format!("❌ Failed to save: {}", e);
                        app.add_log(super::LogLevel::Error, format!("Failed to save config: {}", e));
                    }
                }
            }
            
            ui.add_space(10.0);
            
            let load_button = egui::Button::new(
                egui::RichText::new("📂 Load Configuration")
                    .size(15.0)
                    .color(egui::Color32::WHITE)
            )
            .fill(colors.primary)
            .rounding(8.0)
            .min_size(egui::vec2(180.0, 40.0));
            
            if ui.add(load_button).clicked() {
                app.load_config_from_env();
                app.status_message = "✅ Configuration loaded from .env file".to_string();
                app.add_log(super::LogLevel::Info, "Configuration loaded".to_string());
            }
            
            ui.add_space(10.0);
            
            let reset_button = egui::Button::new(
                egui::RichText::new("🔄 Reset to Defaults")
                    .size(15.0)
            )
            .fill(colors.surface_hover)
            .rounding(8.0)
            .min_size(egui::vec2(180.0, 40.0));
            
            if ui.add(reset_button).clicked() {
                app.config = ConfigData::default();
                app.status_message = "✅ Configuration reset to defaults".to_string();
                app.add_log(super::LogLevel::Info, "Configuration reset".to_string());
            }
        });
        
        ui.add_space(15.0);
        
        // Status message
        if !app.status_message.is_empty() {
            let msg_color = if app.status_message.starts_with("✅") {
                colors.success
            } else {
                colors.error
            };
            
            ui.label(
                egui::RichText::new(&app.status_message)
                    .size(14.0)
                    .color(msg_color)
            );
        }
        
        ui.add_space(20.0);
    });
}

fn config_section(
    ui: &mut egui::Ui,
    colors: &ThemeColors,
    title: &str,
    content: impl FnOnce(&mut egui::Ui),
) {
    egui::Frame::none()
        .fill(colors.surface)
        .inner_margin(egui::Margin::same(20.0))
        .rounding(12.0)
        .stroke(egui::Stroke::new(1.0, colors.border))
        .show(ui, |ui| {
            ui.label(
                egui::RichText::new(title)
                    .size(18.0)
                    .color(colors.text_primary)
                    .strong()
            );
            ui.add_space(15.0);
            content(ui);
        });
}

fn config_field(
    ui: &mut egui::Ui,
    colors: &ThemeColors,
    label: &str,
    value: &mut String,
    password: bool,
    width: f32,
) {
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(label)
                .size(14.0)
                .color(colors.text_primary)
        );
        
        let text_edit = egui::TextEdit::singleline(value)
            .password(password)
            .desired_width(width);
        
        ui.add(text_edit);
    });
    ui.add_space(8.0);
}

fn config_field_inline(
    ui: &mut egui::Ui,
    colors: &ThemeColors,
    label: &str,
    value: &mut String,
    width: f32,
) {
    ui.label(
        egui::RichText::new(label)
            .size(14.0)
            .color(colors.text_primary)
    );
    
    let text_edit = egui::TextEdit::singleline(value)
        .desired_width(width);
    
    ui.add(text_edit);
}
