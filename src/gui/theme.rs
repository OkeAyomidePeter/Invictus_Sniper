use eframe::egui;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Theme {
    Light,
    Dark,
}

impl Theme {
    pub fn toggle(&mut self) {
        *self = match self {
            Theme::Light => Theme::Dark,
            Theme::Dark => Theme::Light,
        };
    }
}

pub struct ThemeColors {
    pub background: egui::Color32,
    pub surface: egui::Color32,
    pub surface_hover: egui::Color32,
    pub primary: egui::Color32,
    pub primary_hover: egui::Color32,
    pub secondary: egui::Color32,
    pub text_primary: egui::Color32,
    pub text_secondary: egui::Color32,
    pub success: egui::Color32,
    pub warning: egui::Color32,
    pub error: egui::Color32,
    pub border: egui::Color32,
    pub shadow: egui::Color32,
}

impl ThemeColors {
    pub fn from_theme(theme: Theme) -> Self {
        match theme {
            Theme::Light => Self::light(),
            Theme::Dark => Self::dark(),
        }
    }

    fn light() -> Self {
        Self {
            background: egui::Color32::from_rgb(245, 247, 250),
            surface: egui::Color32::from_rgb(255, 255, 255),
            surface_hover: egui::Color32::from_rgb(248, 250, 252),
            primary: egui::Color32::from_rgb(99, 102, 241),      // Indigo
            primary_hover: egui::Color32::from_rgb(79, 70, 229),
            secondary: egui::Color32::from_rgb(139, 92, 246),    // Purple
            text_primary: egui::Color32::from_rgb(17, 24, 39),
            text_secondary: egui::Color32::from_rgb(107, 114, 128),
            success: egui::Color32::from_rgb(34, 197, 94),
            warning: egui::Color32::from_rgb(251, 146, 60),
            error: egui::Color32::from_rgb(239, 68, 68),
            border: egui::Color32::from_rgb(229, 231, 235),
            shadow: egui::Color32::from_rgba_premultiplied(0, 0, 0, 10),
        }
    }

    fn dark() -> Self {
        Self {
            background: egui::Color32::from_rgb(15, 23, 42),      // Slate 900
            surface: egui::Color32::from_rgb(30, 41, 59),         // Slate 800
            surface_hover: egui::Color32::from_rgb(51, 65, 85),   // Slate 700
            primary: egui::Color32::from_rgb(129, 140, 248),      // Indigo 400
            primary_hover: egui::Color32::from_rgb(99, 102, 241),
            secondary: egui::Color32::from_rgb(167, 139, 250),    // Purple 400
            text_primary: egui::Color32::from_rgb(248, 250, 252),
            text_secondary: egui::Color32::from_rgb(148, 163, 184),
            success: egui::Color32::from_rgb(74, 222, 128),
            warning: egui::Color32::from_rgb(251, 146, 60),
            error: egui::Color32::from_rgb(248, 113, 113),
            border: egui::Color32::from_rgb(51, 65, 85),
            shadow: egui::Color32::from_rgba_premultiplied(0, 0, 0, 30),
        }
    }
}

pub fn configure_fonts(ctx: &egui::Context) {
    use egui::FontId;

    let mut style = (*ctx.style()).clone();
    style.text_styles = [
        (egui::TextStyle::Heading, FontId::new(26.0, egui::FontFamily::Proportional)),
        (egui::TextStyle::Body, FontId::new(15.0, egui::FontFamily::Proportional)),
        (egui::TextStyle::Monospace, FontId::new(13.0, egui::FontFamily::Monospace)),
        (egui::TextStyle::Button, FontId::new(15.0, egui::FontFamily::Proportional)),
        (egui::TextStyle::Small, FontId::new(12.0, egui::FontFamily::Proportional)),
    ]
    .into();
    
    ctx.set_style(style);
}

pub fn apply_theme(ctx: &egui::Context, theme: Theme) {
    let colors = ThemeColors::from_theme(theme);
    
    let mut visuals = match theme {
        Theme::Light => egui::Visuals::light(),
        Theme::Dark => egui::Visuals::dark(),
    };
    
    // Modern rounded corners
    visuals.window_rounding = 12.0.into();
    visuals.menu_rounding = 8.0.into();
    visuals.panel_fill = colors.background;
    
    // Widget styling
    visuals.widgets.noninteractive.bg_fill = colors.surface;
    visuals.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0, colors.text_primary);
    visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, colors.border);
    visuals.widgets.noninteractive.rounding = 8.0.into();
    
    visuals.widgets.inactive.bg_fill = colors.surface;
    visuals.widgets.inactive.fg_stroke = egui::Stroke::new(1.0, colors.text_primary);
    visuals.widgets.inactive.bg_stroke = egui::Stroke::new(1.5, colors.border);
    visuals.widgets.inactive.rounding = 8.0.into();
    
    visuals.widgets.hovered.bg_fill = colors.surface_hover;
    visuals.widgets.hovered.fg_stroke = egui::Stroke::new(1.5, colors.primary);
    visuals.widgets.hovered.bg_stroke = egui::Stroke::new(2.0, colors.primary);
    visuals.widgets.hovered.rounding = 8.0.into();
    
    visuals.widgets.active.bg_fill = colors.primary;
    visuals.widgets.active.fg_stroke = egui::Stroke::new(2.0, egui::Color32::WHITE);
    visuals.widgets.active.bg_stroke = egui::Stroke::new(2.0, colors.primary_hover);
    visuals.widgets.active.rounding = 8.0.into();
    
    // Selection colors
    visuals.selection.bg_fill = colors.primary.linear_multiply(0.3);
    visuals.selection.stroke = egui::Stroke::new(1.0, colors.primary);
    
    // Window shadow
    visuals.window_shadow.extrusion = 16.0;
    visuals.window_shadow.color = colors.shadow;
    
    ctx.set_visuals(visuals);
}

// Helper function to create a card-like frame
pub fn card_frame() -> egui::Frame {
    egui::Frame::none()
        .fill(egui::Color32::TRANSPARENT)
        .inner_margin(egui::Margin::same(16.0))
        .outer_margin(egui::Margin::same(8.0))
        .rounding(12.0)
        .shadow(egui::epaint::Shadow {
            extrusion: 4.0,
            color: egui::Color32::from_rgba_premultiplied(0, 0, 0, 20),
        })
}

// Helper function to create a metric card
pub fn metric_card(
    ui: &mut egui::Ui,
    colors: &ThemeColors,
    icon: &str,
    label: &str,
    value: &str,
    color: egui::Color32,
) {
    egui::Frame::none()
        .fill(colors.surface)
        .inner_margin(egui::Margin::same(20.0))
        .rounding(12.0)
        .stroke(egui::Stroke::new(1.0, colors.border))
        .show(ui, |ui| {
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(icon).size(24.0));
                    ui.label(
                        egui::RichText::new(label)
                            .size(13.0)
                            .color(colors.text_secondary)
                    );
                });
                ui.add_space(8.0);
                ui.label(
                    egui::RichText::new(value)
                        .size(28.0)
                        .color(color)
                        .strong()
                );
            });
        });
}
