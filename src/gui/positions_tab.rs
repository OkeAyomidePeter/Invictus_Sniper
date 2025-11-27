use super::theme::ThemeColors;
use eframe::egui;
use std::sync::Arc;

pub struct Position {
    pub mint: String,
    pub entry_price: f64,
    pub current_price: f64,
    pub amount_sol: f64,
    pub pnl_pct: f64,
    pub time_elapsed: String,
}

pub fn show(
    ui: &mut egui::Ui,
    colors: &ThemeColors,
    positions: &[Position],
) {
    egui::ScrollArea::vertical().show(ui, |ui| {
        ui.add_space(10.0);
        
        ui.heading(egui::RichText::new("📊 Active Positions").size(28.0).color(colors.text_primary));
        ui.add_space(20.0);
        
        if positions.is_empty() {
            // Empty state
            egui::Frame::none()
                .fill(colors.surface)
                .inner_margin(egui::Margin::same(40.0))
                .rounding(12.0)
                .stroke(egui::Stroke::new(1.0, colors.border))
                .show(ui, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.label(egui::RichText::new("📭").size(64.0));
                        ui.add_space(10.0);
                        ui.label(
                            egui::RichText::new("No Active Positions")
                                .size(20.0)
                                .color(colors.text_secondary)
                        );
                        ui.add_space(5.0);
                        ui.label(
                            egui::RichText::new("Positions will appear here when the bot makes trades")
                                .size(14.0)
                                .color(colors.text_secondary)
                        );
                    });
                });
        } else {
            // Positions table
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
                                    egui::RichText::new("Token")
                                        .size(13.0)
                                        .color(colors.text_secondary)
                                        .strong()
                                );
                                ui.add_space(ui.available_width() * 0.25);
                                
                                ui.label(
                                    egui::RichText::new("Entry")
                                        .size(13.0)
                                        .color(colors.text_secondary)
                                        .strong()
                                );
                                ui.add_space(ui.available_width() * 0.25);
                                
                                ui.label(
                                    egui::RichText::new("Current")
                                        .size(13.0)
                                        .color(colors.text_secondary)
                                        .strong()
                                );
                                ui.add_space(ui.available_width() * 0.25);
                                
                                ui.label(
                                    egui::RichText::new("P/L")
                                        .size(13.0)
                                        .color(colors.text_secondary)
                                        .strong()
                                );
                                ui.add_space(ui.available_width() * 0.15);
                                
                                ui.label(
                                    egui::RichText::new("Time")
                                        .size(13.0)
                                        .color(colors.text_secondary)
                                        .strong()
                                );
                            });
                        });
                    
                    ui.separator();
                    
                    // Table rows
                    for (i, pos) in positions.iter().enumerate() {
                        if i > 0 {
                            ui.separator();
                        }
                        
                        egui::Frame::none()
                            .inner_margin(egui::Margin::symmetric(20.0, 15.0))
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    // Token mint (truncated)
                                    let mint_short = if pos.mint.len() > 8 {
                                        format!("{}...{}", &pos.mint[..4], &pos.mint[pos.mint.len()-4..])
                                    } else {
                                        pos.mint.clone()
                                    };
                                    
                                    ui.label(
                                        egui::RichText::new(mint_short)
                                            .size(14.0)
                                            .color(colors.text_primary)
                                            .monospace()
                                    );
                                    ui.add_space(ui.available_width() * 0.2);
                                    
                                    // Entry price
                                    ui.label(
                                        egui::RichText::new(format!("{:.8}", pos.entry_price))
                                            .size(14.0)
                                            .color(colors.text_primary)
                                    );
                                    ui.add_space(ui.available_width() * 0.2);
                                    
                                    // Current price
                                    ui.label(
                                        egui::RichText::new(format!("{:.8}", pos.current_price))
                                            .size(14.0)
                                            .color(colors.text_primary)
                                    );
                                    ui.add_space(ui.available_width() * 0.2);
                                    
                                    // P/L
                                    let pnl_color = if pos.pnl_pct >= 0.0 {
                                        colors.success
                                    } else {
                                        colors.error
                                    };
                                    
                                    ui.label(
                                        egui::RichText::new(format!("{:+.2}%", pos.pnl_pct))
                                            .size(14.0)
                                            .color(pnl_color)
                                            .strong()
                                    );
                                    ui.add_space(ui.available_width() * 0.1);
                                    
                                    // Time elapsed
                                    ui.label(
                                        egui::RichText::new(&pos.time_elapsed)
                                            .size(14.0)
                                            .color(colors.text_secondary)
                                    );
                                });
                            });
                    }
                });
        }
        
        ui.add_space(20.0);
    });
}
