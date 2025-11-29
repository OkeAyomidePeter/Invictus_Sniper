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

    let icon_play = "▶"; 
    let icon_stop = "⏹";
    let icon_chart = "📈";
    let icon_wallet = "💰";
    
    // Use a container that fills the space but allows scrolling if needed
    egui::ScrollArea::vertical().show(ui, |ui| {
        ui.add_space(10.0);
        
        // Header with Start/Stop Button
        ui.horizontal(|ui| {
            ui.heading(egui::RichText::new("Dashboard").size(28.0).color(colors.text_primary));
            
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let (button_text, button_color, enabled) = match bot_state {
                    BotState::Stopped => (format!("{} Start Bot", icon_play), colors.success, true),
                    BotState::Starting => ("Starting...".to_string(), colors.warning, false),
                    BotState::Running => (format!("{} Stop Bot", icon_stop), colors.error, true),
                    BotState::Stopping => ("Stopping...".to_string(), colors.warning, false),
                    BotState::Error => ("Error - Restart".to_string(), colors.error, true),
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
                        BotState::Stopped => (egui_phosphor::regular::PROHIBIT, "STOPPED", colors.text_secondary),
                        BotState::Starting => (egui_phosphor::regular::HOURGLASS, "STARTING", colors.warning),
                        BotState::Running => (egui_phosphor::regular::CHECK_CIRCLE, "RUNNING", colors.success),
                        BotState::Stopping => (egui_phosphor::regular::HOURGLASS, "STOPPING", colors.warning),
                        BotState::Error => (egui_phosphor::regular::WARNING, "ERROR", colors.error),
                    };
                    
                    ui.label(egui::RichText::new(status_icon).size(32.0).color(status_color));
                    ui.add_space(10.0);
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
                            ui.add_space(10.0);
                            ui.label(egui::RichText::new(egui_phosphor::regular::TIMER).size(24.0).color(colors.text_secondary));
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
        
        egui::Grid::new("metrics_grid")
            .num_columns(3)
            .spacing([15.0, 15.0])
            .max_col_width(300.0) 
            .show(ui, |ui| {
                // Row 1
                ui.allocate_ui(egui::vec2(ui.available_width(), 100.0), |ui| {
                    metric_card(
                        ui,
                        colors,
                        egui_phosphor::regular::WALLET,
                        "Wallet Balance",
                        &format!("{:.4} SOL", metrics.wallet_balance_sol),
                        colors.primary,
                    );
                });
                
                ui.allocate_ui(egui::vec2(ui.available_width(), 100.0), |ui| {
                    metric_card(
                        ui,
                        colors,
                        egui_phosphor::regular::TREND_UP,
                        "Active Positions",
                        &format!("{}", metrics.active_positions),
                        colors.secondary,
                    );
                });
                
                ui.allocate_ui(egui::vec2(ui.available_width(), 100.0), |ui| {
                    metric_card(
                        ui,
                        colors,
                        egui_phosphor::regular::TARGET,
                        "Total Trades",
                        &format!("{}", metrics.total_trades),
                        colors.text_primary,
                    );
                });
                
                ui.end_row();
                
                // Row 2
                ui.allocate_ui(egui::vec2(ui.available_width(), 100.0), |ui| {
                    let pnl_color = if metrics.total_pnl_sol >= 0.0 {
                        colors.success
                    } else {
                        colors.error
                    };
                    
                    metric_card(
                        ui,
                        colors,
                        if metrics.total_pnl_sol >= 0.0 { egui_phosphor::regular::CHART_LINE_UP } else { egui_phosphor::regular::CHART_LINE_DOWN },
                        "Total P/L",
                        &format!("{:+.4} SOL", metrics.total_pnl_sol),
                        pnl_color,
                    );
                });
                
                ui.allocate_ui(egui::vec2(ui.available_width(), 100.0), |ui| {
                    metric_card(
                        ui,
                        colors,
                        egui_phosphor::regular::TROPHY,
                        "Win Rate",
                        &format!("{:.1}%", metrics.win_rate),
                        if metrics.win_rate >= 50.0 { colors.success } else { colors.warning },
                    );
                });
                
                // Empty cell for now or another metric
                ui.allocate_ui(egui::vec2(ui.available_width(), 100.0), |ui| {
                     // Placeholder for future metric
                });
                
                ui.end_row();
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
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(egui_phosphor::regular::INFO).size(20.0).color(colors.primary));
                    ui.label(
                        egui::RichText::new("About Invictus Sniper Bot")
                            .size(16.0)
                            .color(colors.text_primary)
                            .strong()
                    );
                });
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
