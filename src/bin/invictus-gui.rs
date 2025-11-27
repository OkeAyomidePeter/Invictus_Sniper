use invictus::gui::InvictusGUI;
use eframe::egui;


fn main() -> Result<(), eframe::Error> {
    // Set up logging
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .init();

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
        Box::new(|cc| Box::new(InvictusGUI::new(cc))),
    )
}
