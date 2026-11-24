//! Search: one box, every addon that can search answers, results arrive row by row. Under the box, before anything is
//! typed, the searches made before.

use std::time::{Duration, Instant};

use eframe::egui::{self, Align2, Color32, ScrollArea, Sense, Ui, pos2, vec2};
use nuvio_core::meta::PosterShape;

use super::{Action, Cx};
use crate::icons;
use crate::theme::{self, radius, size};
use crate::ui_kit::{self, PillStyle, PosterCard, animate, page_padding, radius_of, with_alpha};

const DEBOUNCE: Duration = Duration::from_millis(380);
/// Words that stay on screen this long with results under them count as a search that was meant, and go in the history.
const REMEMBER_AFTER: Duration = Duration::from_millis(2500);

pub fn show(cx: &mut Cx, ui: &mut Ui) {
    let t = theme::get();
    let width = ui.available_width();
    let pad = page_padding(width);

    ScrollArea::vertical().id_salt("search-scroll").auto_shrink([false, false]).show(ui, |ui| {
        ui.add_space(104.0);
        let field_w = (width - pad * 2.0).min(720.0);
        ui.horizontal(|ui| {
            ui.add_space(pad);
            let response = ui_kit::text_field(ui, &mut cx.state.search.query, "Search movies, series and more", field_w);
            if !cx.view.search_focused {
                response.request_focus();
                cx.view.search_focused = true;
            }
            if response.changed() {
                cx.state.search.typed_at = Some(Instant::now());
            }
            if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                cx.state.run_search(cx.svc);
                let query = cx.state.search.submitted.clone();
                cx.state.remember_search(&query);
            }
        });
        // Search once the typing pauses.
        if let Some(at) = cx.state.search.typed_at {
            if at.elapsed() >= DEBOUNCE {
                cx.state.run_search(cx.svc);
            } else {
                ui.ctx().request_repaint_after(DEBOUNCE - at.elapsed());
            }
        }
        ui.add_space(24.0);

        let submitted = cx.state.search.submitted.clone();
        if submitted.chars().count() < 2 {
            history(cx, ui, pad, field_w);
            return;
        }
        let card_w = ((width / 8.6).clamp(150.0, 200.0) / 2.0).floor() * 2.0;
        let rows = std::mem::take(&mut cx.state.search.rows);
        for row in &rows {
            ui.horizontal(|ui| {
                ui.add_space(pad);
                ui_kit::section_header(ui, &row.title, Some(&row.subtitle));
            });
            ui.add_space(10.0);
            let scroll_id = egui::Id::unique(("search-row", &row.key));
            ui_kit::paging_row(ui, scroll_id, pad, ui_kit::card_size(PosterShape::Poster, card_w).y / 2.0, |ui| {
                for item in &row.items {
                    let response = PosterCard { images: &cx.svc.images, item, width: card_w, shape: PosterShape::Poster, show_title: true }.show(ui);
                    if response.clicked() {
                        cx.state.remember_search(&submitted);
                        cx.actions.push(Action::OpenDetails { kind: item.kind.clone(), id: item.id.clone(), preview: Some(item.clone()) });
                    }
                    super::context::title_menu(cx.state, cx.actions, &response, item);
                }
            });
            ui.add_space(26.0);
        }
        let none_found = rows.is_empty();
        cx.state.search.rows = rows;
        if cx.state.search.pending > 0 {
            let (rect, _) = ui.allocate_exact_size(vec2(width, 80.0), Sense::hover());
            ui_kit::spinner(ui, rect.center(), 14.0, t.text_muted);
        } else if none_found {
            let (rect, _) = ui.allocate_exact_size(vec2(width, 200.0), Sense::hover());
            ui_kit::draw_text(ui, rect.center(), Align2::CENTER_CENTER, &format!("Nothing found for “{submitted}”."), theme::regular(size::BODY_LG), t.text_muted);
        } else if !cx.state.search.remembered {
            // Results that stayed in front of the person for a moment: that was a search worth coming back to.
            match cx.state.search.ran_at.map(|at| at.elapsed()) {
                Some(waited) if waited >= REMEMBER_AFTER => cx.state.remember_search(&submitted),
                Some(waited) => ui.ctx().request_repaint_after(REMEMBER_AFTER - waited),
                None => {}
            }
        }
        ui.add_space(60.0);
    });
}

/// The searches made before, newest first: pick one to run it again, remove one, or clear the list.
fn history(cx: &mut Cx, ui: &mut Ui, pad: f32, width: f32) {
    let t = theme::get();
    if cx.state.history.entries.is_empty() {
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 220.0), Sense::hover());
        ui_kit::draw_text(ui, rect.center(), Align2::CENTER_CENTER, "Type at least two letters.", theme::regular(size::BODY_LG), t.text_muted);
        return;
    }
    let entries = cx.state.history.entries.clone();
    ui.horizontal(|ui| {
        ui.add_space(pad);
        ui.set_min_height(40.0);
        let (rect, _) = ui.allocate_exact_size(vec2(width - 105.0, 40.0), Sense::hover());
        ui_kit::draw_text(ui, rect.left_center(), Align2::LEFT_CENTER, "Recent searches", theme::bold(size::TITLE_SM), t.text);
        if ui_kit::pill_button(ui, None, "Clear all", PillStyle::Soft).clicked() {
            cx.state.history.clear();
            cx.state.save_history();
        }
    });
    ui.add_space(6.0);
    let mut chosen: Option<String> = None;
    let mut removed: Option<String> = None;
    for entry in &entries {
        ui.horizontal(|ui| {
            ui.add_space(pad);
            let (rect, response) = ui.allocate_exact_size(vec2(width, 48.0), Sense::click());
            let hover = animate(ui, response.id.with("hover"), response.hovered(), theme::motion::FAST);
            ui.painter().rect_filled(rect, radius_of(radius::BUTTON), with_alpha(Color32::WHITE, 0.04 + 0.06 * hover));
            ui_kit::draw_icon(ui, pos2(rect.left() + 28.0, rect.center().y), icons::HISTORY, 22.0, t.text_muted);
            let text = ui_kit::galley(ui, entry, theme::semibold(size::BODY_LG), t.text, width - 130.0, 1);
            ui.painter().galley(pos2(rect.left() + 54.0, rect.center().y - text.size().y / 2.0), text, t.text);
            // The cross sits on top of the row, so it gets the pointer first.
            let cross = egui::Rect::from_center_size(pos2(rect.right() - 28.0, rect.center().y), vec2(36.0, 36.0));
            let cross_response = ui.interact(cross, response.id.with("remove"), Sense::click());
            let cross_hover = animate(ui, cross_response.id.with("hover"), cross_response.hovered(), theme::motion::FAST);
            ui.painter().circle_filled(cross.center(), 16.0, with_alpha(Color32::WHITE, 0.10 * cross_hover));
            ui_kit::draw_icon(ui, cross.center(), icons::CLOSE, 20.0, ui_kit::lerp_color(t.text_muted, t.text, cross_hover.max(hover * 0.5)));
            if response.hovered() || cross_response.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            if cross_response.clicked() {
                removed = Some(entry.clone());
            } else if response.clicked() {
                chosen = Some(entry.clone());
            }
            cross_response.on_hover_text("Remove from the list");
        });
        ui.add_space(6.0);
    }
    if let Some(entry) = removed {
        cx.state.history.remove(&entry);
        cx.state.save_history();
    }
    if let Some(entry) = chosen {
        cx.state.search.query = entry.clone();
        cx.state.run_search(cx.svc);
        cx.state.remember_search(&entry);
        cx.view.search_focused = false;
    }
    ui.add_space(60.0);
}
