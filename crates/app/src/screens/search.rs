//! Search: one box, every addon that can search answers, results arrive row by row.

use std::time::{Duration, Instant};

use eframe::egui::{self, Align2, ScrollArea, Sense, Ui, vec2};
use nuvio_core::meta::PosterShape;

use super::{Action, Cx};
use crate::theme::{self, size};
use crate::ui_kit::{self, PosterCard, page_padding};

const DEBOUNCE: Duration = Duration::from_millis(380);

pub fn show(cx: &mut Cx, ui: &mut Ui) {
    let t = theme::get();
    let width = ui.available_width();
    let pad = page_padding(width);

    ScrollArea::vertical().id_salt("search-scroll").auto_shrink([false, false]).show(ui, |ui| {
        ui.add_space(104.0);
        ui.horizontal(|ui| {
            ui.add_space(pad);
            let field_w = (width - pad * 2.0).min(720.0);
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
        if submitted.is_empty() {
            let (rect, _) = ui.allocate_exact_size(vec2(width, 220.0), Sense::hover());
            ui_kit::draw_text(ui, rect.center(), Align2::CENTER_CENTER, "Type at least two letters.", theme::regular(size::BODY_LG), t.text_muted);
            return;
        }
        let card_w = (width / 8.6).clamp(150.0, 200.0);
        let rows = std::mem::take(&mut cx.state.search.rows);
        for row in &rows {
            ui.horizontal(|ui| {
                ui.add_space(pad);
                ui_kit::section_header(ui, &row.title, Some(&row.subtitle));
            });
            ui.add_space(10.0);
            ScrollArea::horizontal().id_salt(("search-row", &row.key)).auto_shrink([false, true]).scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 14.0;
                    ui.add_space(pad);
                    for item in &row.items {
                        let response = PosterCard { images: &cx.svc.images, item, width: card_w, shape: PosterShape::Poster, show_title: true }.show(ui);
                        if response.clicked() {
                            cx.actions.push(Action::OpenDetails { kind: item.kind.clone(), id: item.id.clone(), preview: Some(item.clone()) });
                        }
                    }
                    ui.add_space(pad);
                });
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
        }
        ui.add_space(60.0);
    });
}
