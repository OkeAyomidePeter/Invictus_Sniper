use super::theme::ThemeColors;
use eframe::egui;

pub struct Trade {
    pub id: i64,
    pub mint: String,
    pub action: String,
    pub entry_price: f64,
    pub exit_price: Option<f64>,
    pub pnl_sol: Option<f64>,
    pub timestamp: String,
    pub sell_trigger: Option<String>,
}

pub fn show(
    ui: &mut egui::Ui,
    colors: &ThemeColors,
    trades: &[Trade],
) {
    egui::ScrollArea::vertical().show(ui, |ui| {
        ui.add_space(10.0);
        
        ui.heading(egui::RichText::new("📜 Trade History").size(28.0).color(colors.text_primary));
        ui.add_space(20.0);
        
        if trades.is_empty() {
            // Empty state
            egui::Frame::none()
                .fill(colors.surface)
                .inner_margin(egui::Margin::same(40.0))
                .rounding(12.0)
                .stroke(egui::Stroke::new(1.0, colors.border))
                .show(ui, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.label(egui::RichText::new("📋").size(64.0));
                        ui.add_space(10.0);
                        ui.label(
                            egui::RichText::new("No Trades Yet")
                                .size(20.0)
                                .color(colors.text_secondary)
                        );
                        ui.add_space(5.0);
                        ui.label(
                            egui::RichText::new("Trade history will appear here after the bot executes trades")
                                .size(14.0)
                                .color(colors.text_secondary)
                        );
                    });
                });
        } else {
            // Trades table
            egui::Frame::none()
                .fill(colors.surface)
                .inner_margin(egui::Margin::same(0.0))
                .rounding(12.0)
                .stroke(egui::Stroke::new(1.0, colors.border))
                .show(ui, |ui| {
                    // Table header
                    egui::Frame::none()
                        .fill(colors.surface_hover)
                        .inner_margin(egui::Margin::symmetric(20.0, 15.0))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(
                                    egui::RichText::new("Time")
                                        .size(13.0)
                                        .color(colors.text_secondary)
                                        .strong()
                                );
                                ui.add_space(ui.available_width() * 0.15);
                                
                                ui.label(
                                    egui::RichText::new("Token")
                                        .size(13.0)
                                        .color(colors.text_secondary)
                                        .strong()
                                );
                                ui.add_space(ui.available_width() * 0.2);
                                
                                ui.label(
                                    egui::RichText::new("Action")
                                        .size(13.0)
                                        .color(colors.text_secondary)
                                        .strong()
                                );
                                ui.add_space(ui.available_width() * 0.15);
                                
                                ui.label(
                                    egui::RichText::new("Entry")
                                        .size(13.0)
                                        .color(colors.text_secondary)
                                        .strong()
                                );
                                ui.add_space(ui.available_width() * 0.15);
                                
                                ui.label(
                                    egui::RichText::new("Exit")
                                        .size(13.0)
                                        .color(colors.text_secondary)
                                        .strong()
                                );
                                ui.add_space(ui.available_width() * 0.1);
                                
                                ui.label(
                                    egui::RichText::new("P/L")
                                        .size(13.0)
                                        .color(colors.text_secondary)
                                        .strong()
                                );
                            });
                        });
                    
                    ui.separator();
                    
                    // Table rows
                    for (i, trade) in trades.iter().enumerate() {
                        if i > 0 {
                            ui.separator();
                        }
                        
                        egui::Frame::none()
                            .inner_margin(egui::Margin::symmetric(20.0, 15.0))
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    // Timestamp
                                    ui.label(
                                        egui::RichText::new(&trade.timestamp)
                                            .size(13.0)
                                            .color(colors.text_secondary)
                                    );
                                    ui.add_space(ui.available_width() * 0.1);
                                    
                                    // Token mint (truncated)
                                    let mint_short = if trade.mint.len() > 8 {
                                        format!("{}...{}", &trade.mint[..4], &trade.mint[trade.mint.len()-4..])
                                    } else {
                                        trade.mint.clone()
                                    };
                                    
                                    ui.label(
                                        egui::RichText::new(mint_short)
                                            .size(14.0)
                                            .color(colors.text_primary)
                                            .monospace()
                                    );
                                    ui.add_space(ui.available_width() * 0.15);
                                    
                                    // Action
                                    let action_color = if trade.action == "BUY" {
                                        colors.primary
                                    } else {
                                        colors.secondary
                                    };
                                    
                                    ui.label(
                                        egui::RichText::new(&trade.action)
                                            .size(14.0)
                                            .color(action_color)
                                            .strong()
                                    );
                                    ui.add_space(ui.available_width() * 0.1);
                                    
                                    // Entry price
                                    ui.label(
                                        egui::RichText::new(format!("{:.8}", trade.entry_price))
                                            .size(13.0)
                                            .color(colors.text_primary)
                                    );
                                    ui.add_space(ui.available_width() * 0.1);
                                    
                                    // Exit price
                                    let exit_text = if let Some(exit) = trade.exit_price {
                                        format!("{:.8}", exit)
                                    } else {
                                        "-".to_string()
                                    };
                                    
                                    ui.label(
                                        egui::RichText::new(exit_text)
                                            .size(13.0)
                                            .color(colors.text_primary)
                                    );
                                    ui.add_space(ui.available_width() * 0.05);
                                    
                                    // P/L
                                    if let Some(pnl) = trade.pnl_sol {
                                        let pnl_color = if pnl >= 0.0 {
                                            colors.success
                                        } else {
                                            colors.error
                                        };
                                        
                                        ui.label(
                                            egui::RichText::new(format!("{:+.4} SOL", pnl))
                                                .size(14.0)
                                                .color(pnl_color)
                                                .strong()
                                        );
                                    } else {
                                        ui.label(
                                            egui::RichText::new("Active")
                                                .size(13.0)
                                                .color(colors.warning)
                                        );
                                    }
                                });
                                
                                // Show sell trigger if available
                                if let Some(trigger) = &trade.sell_trigger {
                                    ui.add_space(5.0);
                                    ui.label(
                                        egui::RichText::new(format!("Trigger: {}", trigger))
                                            .size(12.0)
                                            .color(colors.text_secondary)
                                            .italics()
                                    );
                                }
                            });
                    }
                });
        }
        
        ui.add_space(20.0);
    });
}
