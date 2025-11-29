use invictus::gui::{InvictusGUI, logger::GuiLogger};
use eframe::egui;
use tokio::sync::mpsc;

fn main() -> Result<(), eframe::Error> {
    // Create event channel
    let (event_tx, event_rx) = mpsc::unbounded_channel();

    // Set up logging with our custom GUI logger
    // We clone event_tx because the logger needs to own a sender, 
    // and we also need one for the BotRuntime (passed via InvictusGUI)
    if let Err(e) = GuiLogger::init(event_tx.clone(), log::Level::Info) {
        eprintln!("Failed to initialize logger: {}", e);
    }

    // Initialize egui-phosphor
    // We need to do this inside the app creation or just let it be if it's a font loader.
    // Actually, egui-phosphor usually provides a function to add fonts to ctx.
    // We'll do that in InvictusGUI::update or new.
    
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1400.0, 900.0])
            .with_min_inner_size([1200.0, 700.0])
            .with_title("Invictus Sniper Bot v1.0"),
        ..Default::default()
    };

    eframe::run_native(
        "Invictus Sniper Bot",
        options,
        Box::new(|cc| {
            // Configure fonts including Phosphor icons
            let mut fonts = egui::FontDefinitions::default();
            egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
            cc.egui_ctx.set_fonts(fonts);
            
            Box::new(InvictusGUI::new(cc, event_rx, event_tx))
        }),
    )
}
