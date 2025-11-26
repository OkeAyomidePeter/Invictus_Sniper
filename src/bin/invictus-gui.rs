use invictus::gui::InvictusGUI;
use eframe::egui;


fn main() -> Result<(), eframe::Error> {
    // Set up logging
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .init();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1024.0, 768.0])
            .with_min_inner_size([800.0, 600.0]),
        ..Default::default()
    };

    eframe::run_native(
        "Invictus Sniper Bot",
        options,
        Box::new(|cc| Box::new(InvictusGUI::new(cc))),
    )
}
