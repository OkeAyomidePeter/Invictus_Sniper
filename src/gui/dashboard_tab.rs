use super::theme::{ThemeColors, metric_card};
use super::bot_runtime::{BotMetrics, BotState};
use eframe::egui;

pub fn show(
    ui: &mut egui::Ui,
    colors: &ThemeColors,
    bot_state: BotState,
    metrics: &BotMetrics,
    on_start: &mut bool,
    on_stop: &mut bool,
) {
    egui::ScrollArea::vertical().show(ui, |ui| {
        ui.add_space(10.0);
        
        // Header with Start/Stop Button
        ui.horizontal(|ui| {
            ui.heading(egui::RichText::new("📊 Dashboard").size(28.0).color(colors.text_primary));
            
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let (button_text, button_color, enabled) = match bot_state {
                    BotState::Stopped => ("▶ Start Bot", colors.success, true),
                    BotState::Starting => ("⏳ Starting...", colors.warning, false),
                    BotState::Running => ("⏸ Stop Bot", colors.error, true),
                    BotState::Stopping => ("⏳ Stopping...", colors.warning, false),
                    BotState::Error => ("⚠ Error - Restart", colors.error, true),
                };
                
                ui.add_enabled_ui(enabled, |ui| {
                    let button = egui::Button::new(
                        egui::RichText::new(button_text)
                            .size(16.0)
                            .color(egui::Color32::WHITE)
                    )
                    .fill(button_color)
                    .rounding(8.0)
                    .min_size(egui::vec2(140.0, 40.0));
                    
                    if ui.add(button).clicked() {
                        match bot_state {
                            BotState::Stopped | BotState::Error => *on_start = true,
                            BotState::Running => *on_stop = true,
                            _ => {}
                        }
                    }
                });
            });
        });
        
        ui.add_space(20.0);
        
        // Status Card
        egui::Frame::none()
            .fill(colors.surface)
            .inner_margin(egui::Margin::same(20.0))
            .rounding(12.0)
            .stroke(egui::Stroke::new(1.0, colors.border))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let (status_icon, status_text, status_color) = match bot_state {
                        BotState::Stopped => ("⭕", "STOPPED", colors.text_secondary),
                        BotState::Starting => ("⏳", "STARTING", colors.warning),
                        BotState::Running => ("🟢", "RUNNING", colors.success),
                        BotState::Stopping => ("⏳", "STOPPING", colors.warning),
                        BotState::Error => ("🔴", "ERROR", colors.error),
                    };
                    
                    ui.label(egui::RichText::new(status_icon).size(32.0));
                    ui.vertical(|ui| {
                        ui.label(
                            egui::RichText::new("Bot Status")
                                .size(13.0)
                                .color(colors.text_secondary)
                        );
                        ui.label(
                            egui::RichText::new(status_text)
                                .size(24.0)
                                .color(status_color)
                                .strong()
                        );
                    });
                    
                    if bot_state == BotState::Running {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let uptime_mins = metrics.uptime_seconds / 60;
                            let uptime_hours = uptime_mins / 60;
                            let uptime_text = if uptime_hours > 0 {
                                format!("{}h {}m", uptime_hours, uptime_mins % 60)
                            } else {
                                format!("{}m", uptime_mins)
                            };
                            
                            ui.vertical(|ui| {
                                ui.label(
                                    egui::RichText::new("Uptime")
                                        .size(13.0)
                                        .color(colors.text_secondary)
                                );
                                ui.label(
                                    egui::RichText::new(uptime_text)
                                        .size(20.0)
                                        .color(colors.text_primary)
                                );
                            });
                        });
                    }
                });
            });
        
        ui.add_space(20.0);
        
        // Metrics Grid
        ui.label(
            egui::RichText::new("Key Metrics")
                .size(18.0)
                .color(colors.text_primary)
                .strong()
        );
        ui.add_space(10.0);
        
        // First row of metrics
        ui.horizontal(|ui| {
            ui.allocate_ui(egui::vec2(ui.available_width() / 3.0 - 10.0, 100.0), |ui| {
                metric_card(
                    ui,
                    colors,
                    "💰",
                    "Wallet Balance",
                    &format!("{:.4} SOL", metrics.wallet_balance_sol),
                    colors.primary,
                );
            });
            
            ui.add_space(10.0);
            
            ui.allocate_ui(egui::vec2(ui.available_width() / 2.0 - 5.0, 100.0), |ui| {
                metric_card(
                    ui,
                    colors,
                    "📈",
                    "Active Positions",
                    &format!("{}", metrics.active_positions),
                    colors.secondary,
                );
            });
            
            ui.add_space(10.0);
            
            ui.allocate_ui(egui::vec2(ui.available_width(), 100.0), |ui| {
                metric_card(
                    ui,
                    colors,
                    "🎯",
                    "Total Trades",
                    &format!("{}", metrics.total_trades),
                    colors.text_primary,
                );
            });
        });
        
        ui.add_space(10.0);
        
        // Second row of metrics
        ui.horizontal(|ui| {
            ui.allocate_ui(egui::vec2(ui.available_width() / 2.0 - 5.0, 100.0), |ui| {
                let pnl_color = if metrics.total_pnl_sol >= 0.0 {
                    colors.success
                } else {
                    colors.error
                };
                
                metric_card(
                    ui,
                    colors,
                    if metrics.total_pnl_sol >= 0.0 { "📊" } else { "📉" },
                    "Total P/L",
                    &format!("{:+.4} SOL", metrics.total_pnl_sol),
                    pnl_color,
                );
            });
            
            ui.add_space(10.0);
            
            ui.allocate_ui(egui::vec2(ui.available_width(), 100.0), |ui| {
                metric_card(
                    ui,
                    colors,
                    "🎲",
                    "Win Rate",
                    &format!("{:.1}%", metrics.win_rate),
                    if metrics.win_rate >= 50.0 { colors.success } else { colors.warning },
                );
            });
        });
        
        ui.add_space(30.0);
        
        // Quick Info Section
        ui.label(
            egui::RichText::new("Quick Info")
                .size(18.0)
                .color(colors.text_primary)
                .strong()
        );
        ui.add_space(10.0);
        
        egui::Frame::none()
            .fill(colors.surface)
            .inner_margin(egui::Margin::same(20.0))
            .rounding(12.0)
            .stroke(egui::Stroke::new(1.0, colors.border))
            .show(ui, |ui| {
                ui.label(
                    egui::RichText::new("ℹ️ About Invictus Sniper Bot")
                        .size(16.0)
                        .color(colors.text_primary)
                        .strong()
                );
                ui.add_space(10.0);
                
                ui.label(
                    egui::RichText::new(
                        "Invictus is an automated Solana token sniper bot that:\n\n\
                        • Monitors new token launches via Helius\n\
                        • Analyzes and scores tokens based on liquidity, holders, and risk\n\
                        • Executes fast trades using Jito bundles\n\
                        • Manages positions with auto-sell (profit targets, stop-loss, timeout)\n\
                        • Tracks wallet balance and sends alerts\n\
                        • Provides Telegram notifications for all activities"
                    )
                    .size(14.0)
                    .color(colors.text_secondary)
                );
            });
        
        ui.add_space(20.0);
    });
}
