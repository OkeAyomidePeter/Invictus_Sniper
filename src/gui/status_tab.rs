use super::BotStatus;
use eframe::egui;

pub fn show(ui: &mut egui::Ui, bot_status: &BotStatus, status_message: &str) {
    ui.heading("📊 Bot Status");
    ui.separator();
    
    ui.add_space(20.0);
    
    // Status Display
    ui.group(|ui| {
        ui.set_min_width(ui.available_width());
        ui.vertical_centered(|ui| {
            let (status_text, status_color, status_icon) = match bot_status {
                BotStatus::Stopped => ("STOPPED", egui::Color32::GRAY, "⭕"),
                BotStatus::Running => ("RUNNING", egui::Color32::GREEN, "🟢"),
                BotStatus::Error => ("ERROR", egui::Color32::RED, "🔴"),
            };
            
            ui.label(egui::RichText::new(status_icon).size(48.0));
            ui.label(egui::RichText::new(status_text)
                .size(32.0)
                .color(status_color)
                .strong());
        });
    });
    
    ui.add_space(20.0);
    
    if !status_message.is_empty() {
        ui.group(|ui| {
            ui.set_min_width(ui.available_width());
            ui.label(egui::RichText::new("Status Message:").strong());
            ui.label(status_message);
        });
    }
    
    ui.add_space(20.0);
    
    // Information Box
    ui.group(|ui| {
        ui.set_min_width(ui.available_width());
        ui.heading("ℹ️ How to Use");
        ui.add_space(5.0);
        
        ui.label("1. Configure your bot settings in the Configuration tab");
        ui.label("2. Click 'Save Configuration' to write settings to .env file");
        ui.label("3. Run the bot using: cargo run --release");
        ui.label("4. Monitor logs in the Logs tab");
        ui.label("5. Control the bot via Telegram or by stopping the process");
        
        ui.add_space(10.0);
        
        ui.label(egui::RichText::new("Note:").strong());
        ui.label("The GUI is currently for configuration only.");
        ui.label("Bot start/stop from GUI will be added in a future update.");
    });
    
    ui.add_space(20.0);
    
    // Quick Stats (placeholder for future enhancement)
    ui.group(|ui| {
        ui.set_min_width(ui.available_width());
        ui.heading("📈 Quick Stats");
        ui.add_space(5.0);
        
        ui.horizontal(|ui| {
            ui.label("Total Trades:");
            ui.label(egui::RichText::new("N/A").color(egui::Color32::GRAY));
        });
        
        ui.horizontal(|ui| {
            ui.label("Active Positions:");
            ui.label(egui::RichText::new("N/A").color(egui::Color32::GRAY));
        });
        
        ui.horizontal(|ui| {
            ui.label("Wallet Balance:");
            ui.label(egui::RichText::new("N/A").color(egui::Color32::GRAY));
        });
        
        ui.add_space(5.0);
        ui.label(egui::RichText::new("(Stats will be available when bot is running)")
            .small()
            .color(egui::Color32::GRAY));
    });
}
