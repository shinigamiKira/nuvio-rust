//! Library: the titles the user saved, as a wrapping grid.

use eframe::egui::{self, Align2, ScrollArea, Ui, vec2};
use nuvio_core::meta::PosterShape;

use super::{Action, Cx};
use crate::theme::{self, size};
use crate::ui_kit::{self, PosterCard, page_padding};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Filter {
    All,
    Kind(String),
}

pub fn show(cx: &mut Cx, ui: &mut Ui) {
    let t = theme::get();
    let width = ui.available_width();
    let pad = page_padding(width);

    let mut kinds: Vec<String> = cx.state.library.items.iter().map(|i| i.item.kind.clone()).collect();
    kinds.sort();
    kinds.dedup();

    ScrollArea::vertical().id_salt("library-scroll").auto_shrink([false, false]).show(ui, |ui| {
        ui.add_space(104.0);
        ui.horizontal(|ui| {
            ui.add_space(pad);
            ui_kit::section_header(ui, "Library", None);
        });
        ui.add_space(8.0);
        if cx.state.library.items.is_empty() {
            let (rect, _) = ui.allocate_exact_size(vec2(width, 260.0), egui::Sense::hover());
            ui_kit::draw_text(ui, rect.center() - vec2(0.0, 14.0), Align2::CENTER_CENTER, "Nothing saved yet.", theme::semibold(size::TITLE_SM), t.text);
            ui_kit::draw_text(ui, rect.center() + vec2(0.0, 16.0), Align2::CENTER_CENTER, "Use “Add to library” on a title's page to keep it here.", theme::regular(size::BODY_SM), t.text_muted);
            return;
        }
        if kinds.len() > 1 {
            ui.horizontal(|ui| {
                ui.add_space(pad);
                ui.spacing_mut().item_spacing.x = 8.0;
                if ui_kit::chip(ui, "All", cx.view.library_filter == Filter::All).clicked() {
                    cx.view.library_filter = Filter::All;
                }
                for kind in &kinds {
                    let selected = cx.view.library_filter == Filter::Kind(kind.clone());
                    if ui_kit::chip(ui, &crate::state::type_label(kind), selected).clicked() {
                        cx.view.library_filter = Filter::Kind(kind.clone());
                    }
                }
            });
            ui.add_space(16.0);
        }

        let card_w = 176.0;
        let gap = 18.0;
        let columns = (((width - pad * 2.0 + gap) / (card_w + gap)).floor() as usize).max(1);
        let items: Vec<_> = cx
            .state
            .library
            .items
            .iter()
            .filter(|entry| match &cx.view.library_filter {
                Filter::All => true,
                Filter::Kind(kind) => &entry.item.kind == kind,
            })
            .map(|entry| entry.item.clone())
            .collect();
        for row in items.chunks(columns) {
            ui.horizontal(|ui| {
                ui.add_space(pad);
                ui.spacing_mut().item_spacing.x = gap;
                for item in row {
                    let response = PosterCard { images: &cx.svc.images, item, width: card_w, shape: PosterShape::Poster, show_title: true }.show(ui);
                    if response.clicked() {
                        cx.actions.push(Action::OpenDetails { kind: item.kind.clone(), id: item.id.clone(), preview: Some(item.clone()) });
                    }
                }
            });
            ui.add_space(10.0);
        }
        ui.add_space(60.0);
    });
}
