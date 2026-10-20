//! The floating glass pill with the four root screens, centred at the top of the window.

use eframe::egui::{self, Align2, Area, Color32, Id, Order, Sense, vec2};

use super::Tab;
use crate::icons;
use crate::theme::{self, size};
use crate::ui_kit::{self, animate, glass_panel, radius_of, with_alpha};

const ITEMS: [(Tab, char, &str); 4] = [
    (Tab::Home, icons::HOME, "Home"),
    (Tab::Search, icons::SEARCH, "Search"),
    (Tab::Library, icons::LIBRARY, "Library"),
    (Tab::Settings, icons::SETTINGS, "Settings"),
];

const ITEM_W: f32 = 78.0;
const HEIGHT: f32 = 60.0;
const PAD: f32 = 6.0;

/// Draws the pill and returns the tab the user clicked, if any.
pub fn show(ctx: &egui::Context, current: Tab) -> Option<Tab> {
    let t = theme::get();
    let mut clicked = None;
    Area::new(Id::unique("nav-pill")).anchor(Align2::CENTER_TOP, vec2(0.0, 14.0)).order(Order::Foreground).show(ctx, |ui| {
        let width = ITEM_W * ITEMS.len() as f32 + PAD * 2.0;
        let (rect, _) = ui.allocate_exact_size(vec2(width, HEIGHT + PAD * 2.0), Sense::hover());
        glass_panel(ui, rect, (HEIGHT + PAD * 2.0) / 2.0, 1.0);

        let index = ITEMS.iter().position(|(tab, ..)| *tab == current).unwrap_or(0);
        // The highlight slides to the chosen item instead of jumping.
        let slide = ctx.animate_value_with_time(Id::unique("nav-slide"), index as f32, theme::motion::NORMAL);
        let highlight = egui::Rect::from_min_size(rect.min + vec2(PAD + slide * ITEM_W, PAD), vec2(ITEM_W, HEIGHT));
        ui.painter().rect_filled(highlight, radius_of((HEIGHT) / 2.0), with_alpha(t.accent, 0.20));
        ui.painter().rect_stroke(highlight, radius_of(HEIGHT / 2.0), egui::Stroke::new(1.0, with_alpha(t.accent, 0.35)), egui::StrokeKind::Inside);

        for (i, (tab, icon, label)) in ITEMS.iter().enumerate() {
            let item = egui::Rect::from_min_size(rect.min + vec2(PAD + i as f32 * ITEM_W, PAD), vec2(ITEM_W, HEIGHT));
            let response = ui.interact(item, Id::unique(("nav-item", i)), Sense::click());
            let hover = animate(ui, response.id.with("hover"), response.hovered(), theme::motion::FAST);
            let selected = *tab == current;
            let colour = ui_kit::lerp_color(if selected { t.accent } else { t.text_secondary }, Color32::WHITE, hover * if selected { 0.0 } else { 0.8 });
            ui_kit::draw_icon(ui, item.center_top() + vec2(0.0, 22.0), *icon, 26.0, colour);
            ui_kit::draw_text(ui, item.center_top() + vec2(0.0, 45.0), Align2::CENTER_CENTER, label, theme::semibold(size::LABEL_XS), colour);
            if response.hovered() {
                ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            if response.clicked() {
                clicked = Some(*tab);
            }
        }
    });
    clicked
}
