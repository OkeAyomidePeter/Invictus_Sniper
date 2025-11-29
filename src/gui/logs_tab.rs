use super::{LogEntry, LogLevel};
use super::theme::ThemeColors;
use eframe::egui;
use std::sync::{Arc, Mutex};

pub fn show(
    ui: &mut egui::Ui,
    colors: &ThemeColors,
    logs: &Arc<Mutex<Vec<LogEntry>>>,
    filter_level: &mut Option<LogLevel>,
    auto_scroll: &mut bool,
) {
    ui.add_space(10.0);
    
    // Header with controls
    ui.horizontal(|ui| {
        ui.heading(egui::RichText::new("Logs").size(28.0).color(colors.text_primary));
        
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // Auto-scroll toggle
            ui.checkbox(auto_scroll, "Auto-scroll");
            
            ui.add_space(10.0);
            
            // Filter dropdown
            ui.label(egui::RichText::new("Filter:").color(colors.text_secondary));
            egui::ComboBox::from_id_source("log_filter")
                .selected_text(match filter_level {
                    None => "All",
                    Some(LogLevel::Info) => "Info",
                    Some(LogLevel::Warn) => "Warn",
                    Some(LogLevel::Error) => "Error",
                    Some(LogLevel::Debug) => "Debug",
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(filter_level, None, "All");
                    ui.selectable_value(filter_level, Some(LogLevel::Info), "Info");
                    ui.selectable_value(filter_level, Some(LogLevel::Warn), "Warn");
                    ui.selectable_value(filter_level, Some(LogLevel::Error), "Error");
                    ui.selectable_value(filter_level, Some(LogLevel::Debug), "Debug");
                });
            
            ui.add_space(10.0);
            
            // Clear button
            if ui.button("🗑 Clear").clicked() {
                if let Ok(mut logs) = logs.lock() {
                    logs.clear();
                }
            }
        });
    });
    
    ui.add_space(15.0);
    
    // Logs display
    egui::Frame::none()
        .fill(colors.surface)
        .inner_margin(egui::Margin::same(0.0))
        .rounding(12.0)
        .stroke(egui::Stroke::new(1.0, colors.border))
        .show(ui, |ui| {
            let scroll_area = egui::ScrollArea::vertical()
                .max_height(ui.available_height() - 20.0)
                .stick_to_bottom(*auto_scroll);
            
            scroll_area.show(ui, |ui| {
                ui.add_space(10.0);
                
                if let Ok(logs_vec) = logs.lock() {
                    if logs_vec.is_empty() {
                        ui.vertical_centered(|ui| {
                            ui.add_space(40.0);
                            ui.label(egui::RichText::new(egui_phosphor::regular::TRAY).size(48.0).color(colors.text_secondary));
                            ui.add_space(10.0);
                            ui.label(
                                egui::RichText::new("No logs yet")
                                    .size(16.0)
                                    .color(colors.text_secondary)
                            );
                            ui.add_space(40.0);
                        });
                    } else {
                        for entry in logs_vec.iter() {
                            // Apply filter
                            if let Some(filter) = filter_level {
                                if entry.level != *filter {
                                    continue;
                                }
                            }
                            
                            let (level_icon, _level_color) = match entry.level {
                                LogLevel::Info => (egui_phosphor::regular::INFO, colors.primary),
                                LogLevel::Warn => (egui_phosphor::regular::WARNING, colors.warning),
                                LogLevel::Error => (egui_phosphor::regular::X_CIRCLE, colors.error),
                                LogLevel::Debug => (egui_phosphor::regular::WRENCH, colors.text_secondary),
                            };
                            
                            egui::Frame::none()
                                .inner_margin(egui::Margin::symmetric(15.0, 8.0))
                                .show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                        ui.label(egui::RichText::new(level_icon).size(16.0).color(_level_color));
                                        
                                        ui.label(
                                            egui::RichText::new(&entry.timestamp)
                                                .size(12.0)
                                                .color(colors.text_secondary)
                                                .monospace()
                                        );
                                        
                                        ui.label(
                                            egui::RichText::new(&entry.message)
                                                .size(13.0)
                                                .color(colors.text_primary)
                                        );
                                    });
                                });
                            
                            ui.separator();
                        }
                    }
                }
                
                ui.add_space(10.0);
            });
        });
    
    ui.add_space(10.0);
}
