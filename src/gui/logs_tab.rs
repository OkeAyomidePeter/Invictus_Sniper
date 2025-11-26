use super::{LogEntry, LogLevel};
use eframe::egui;
use std::sync::{Arc, Mutex};

pub fn show(ui: &mut egui::Ui, logs: &Arc<Mutex<Vec<LogEntry>>>) {
    ui.heading("📋 Bot Logs");
    ui.separator();
    
    ui.horizontal(|ui| {
        ui.label("Log output from the bot will appear here in real-time");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("🗑️ Clear Logs").clicked() {
                if let Ok(mut logs) = logs.lock() {
                    logs.clear();
                }
            }
        });
    });
    
    ui.separator();
    
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .stick_to_bottom(true)
        .show(ui, |ui| {
            if let Ok(logs) = logs.lock() {
                if logs.is_empty() {
                    ui.colored_label(egui::Color32::GRAY, "No logs yet. Start the bot to see logs.");
                } else {
                    for log in logs.iter() {
                        let color = match log.level {
                            LogLevel::Info => egui::Color32::from_rgb(100, 200, 100),
                            LogLevel::Warn => egui::Color32::from_rgb(255, 200, 0),
                            LogLevel::Error => egui::Color32::from_rgb(255, 100, 100),
                            LogLevel::Debug => egui::Color32::GRAY,
                        };
                        
                        let icon = match log.level {
                            LogLevel::Info => "ℹ️",
                            LogLevel::Warn => "⚠️",
                            LogLevel::Error => "❌",
                            LogLevel::Debug => "🔍",
                        };
                        
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new(&log.timestamp).monospace().color(egui::Color32::GRAY));
                            ui.label(icon);
                            ui.colored_label(color, &log.message);
                        });
                    }
                }
            }
        });
}
