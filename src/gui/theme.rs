use eframe::egui;

pub fn configure_fonts(ctx: &egui::Context) {
    use egui::FontId;

    let mut style = (*ctx.style()).clone();
    style.text_styles = [
        (egui::TextStyle::Heading, FontId::new(24.0, egui::FontFamily::Proportional)),
        (egui::TextStyle::Body, FontId::new(14.0, egui::FontFamily::Proportional)),
        (egui::TextStyle::Monospace, FontId::new(12.0, egui::FontFamily::Monospace)),
        (egui::TextStyle::Button, FontId::new(14.0, egui::FontFamily::Proportional)),
        (egui::TextStyle::Small, FontId::new(10.0, egui::FontFamily::Proportional)),
    ]
    .into();
    
    ctx.set_style(style);
}

pub fn configure_visuals(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();
    visuals.window_rounding = 8.0.into();
    visuals.menu_rounding = 6.0.into();
    ctx.set_visuals(visuals);
}
