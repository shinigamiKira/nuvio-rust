//! The menu a right click opens on a title, wherever one is shown as a card: a poster on a shelf, in the search results or the
//! library, or a card of Continue Watching.

use eframe::egui::{Popup, Response, Sense, Ui, vec2};
use nuvio_core::meta::MetaPreview;
use nuvio_core::store::ContinueItem;

use super::Action;
use crate::icons;
use crate::state::State;
use crate::theme::{self, size};
use crate::ui_kit;

const WIDTH: f32 = 280.0;

/// Opens on a right click on `response`. `item` is the title as the screen knows it.
pub fn title_menu(state: &State, actions: &mut Vec<Action>, response: &Response, item: &MetaPreview) {
    ui_kit::glass_popup(Popup::context_menu(response), |ui| {
        ui.set_min_width(WIDTH);
        body(state, actions, ui, item);
    });
}

/// The title a Continue Watching card stands for.
pub fn preview_of(entry: &ContinueItem) -> MetaPreview {
    let progress = &entry.progress;
    MetaPreview {
        id: progress.id.clone(),
        kind: progress.kind.clone(),
        name: progress.name.clone(),
        poster: progress.poster.clone(),
        background: progress.background.clone(),
        ..MetaPreview::default()
    }
}

fn body(state: &State, actions: &mut Vec<Action>, ui: &mut Ui, item: &MetaPreview) {
    let t = theme::get();
    let heading = ui_kit::galley(ui, &item.name, theme::bold(size::BODY_SM), t.text_muted, WIDTH - 28.0, 1);
    let (row, _) = ui.allocate_exact_size(vec2(ui.available_width().max(WIDTH), heading.size().y + 12.0), Sense::hover());
    ui.painter().galley(row.min + vec2(14.0, 4.0), heading, t.text_muted);

    let resume = state.progress.continue_watching().into_iter().find(|c| c.progress.kind == item.kind && c.progress.id == item.id);
    let is_series = item.kind != "movie";
    if let Some(entry) = &resume {
        let label = match (entry.season(), entry.episode(), entry.up_next.is_some()) {
            (Some(s), Some(e), true) => format!("Play the next episode · S{s} E{e}"),
            (Some(s), Some(e), false) => format!("Resume watching · S{s} E{e}"),
            (_, _, true) => "Play the next episode".to_owned(),
            _ => format!("Resume watching · {} min left", entry.progress.remaining_minutes()),
        };
        if ui_kit::menu_item(ui, Some(icons::PLAY), &label, false).clicked() {
            actions.push(Action::Resume(Box::new(entry.clone())));
        }
    } else if !is_series && ui_kit::menu_item(ui, Some(icons::PLAY), "Play", false).clicked() {
        actions.push(Action::OpenDetails { kind: item.kind.clone(), id: item.id.clone(), preview: Some(item.clone()) });
        actions.push(Action::OpenStreams { video: None });
    }

    if ui_kit::menu_item(ui, Some(if is_series { icons::TV } else { icons::INFO }), if is_series { "Go to show" } else { "Go to movie" }, false).clicked() {
        actions.push(Action::OpenDetails { kind: item.kind.clone(), id: item.id.clone(), preview: Some(item.clone()) });
    }
    if let Some(season) = resume.as_ref().and_then(ContinueItem::season).filter(|_| is_series) {
        if ui_kit::menu_item(ui, Some(icons::SEASONS), &format!("Go to season {season}"), false).clicked() {
            actions.push(Action::OpenSeason { kind: item.kind.clone(), id: item.id.clone(), preview: Some(item.clone()), season });
        }
    }

    if state.library.contains(&item.kind, &item.id) {
        if ui_kit::menu_item(ui, Some(icons::BOOKMARK_REMOVE), "Remove from library", false).clicked() {
            actions.push(Action::ToggleLibrary(item.clone()));
        }
    } else if ui_kit::menu_item(ui, Some(icons::BOOKMARK_ADD), "Add to library", false).clicked() {
        actions.push(Action::ToggleLibrary(item.clone()));
    }

    if resume.is_some() && ui_kit::menu_item(ui, Some(icons::REMOVE_CIRCLE), "Remove from Continue Watching", false).clicked() {
        actions.push(Action::RemoveFromContinue { kind: item.kind.clone(), id: item.id.clone() });
    }
}
