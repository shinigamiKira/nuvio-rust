//! Home: the hero, Continue Watching and one shelf per catalog.

use std::time::Duration;

use eframe::egui::{self, Align2, Color32, CornerRadius, Rect, ScrollArea, Sense, Ui, pos2, vec2};
use nuvio_core::meta::{MetaPreview, PosterShape};
use nuvio_core::store::ContinueItem;

use super::{Action, Cx};
use crate::images::Image;
use crate::state::Load;
use crate::theme::{self, radius, size};
use crate::ui_kit::{self, PillStyle, PosterCard, animate, lerp_color, page_padding, radius_of, with_alpha};

const HERO_INTERVAL: Duration = Duration::from_secs(8);
const HERO_FADE: f32 = 0.7;
const MAX_HERO_ITEMS: usize = 6;

pub fn show(cx: &mut Cx, ui: &mut Ui) {
    let width = ui.available_width();
    let pad = page_padding(width);
    ScrollArea::vertical().id_salt("home-scroll").auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
        let hero_items = hero_items(cx);
        let has_hero = cx.state.settings.hero && !hero_items.is_empty();
        if has_hero {
            hero(cx, ui, &hero_items);
            ui.add_space(26.0);
        } else {
            ui.add_space(96.0);
        }

        import_offer(cx, ui, pad);
        let resume: Vec<ContinueItem> = cx.state.progress.continue_watching().into_iter().take(12).collect();
        if !resume.is_empty() {
            continue_watching(cx, ui, &resume, pad);
            ui.add_space(28.0);
        }

        let any_addon = cx.state.addons.iter().any(|a| a.entry.enabled);
        if cx.state.sections.is_empty() {
            empty_state(cx, ui, any_addon);
        }
        for index in 0..cx.state.sections.len() {
            shelf(cx, ui, index, pad);
            ui.add_space(28.0);
        }
        ui.add_space(60.0);
    });
}

/// The titles the hero rotates through: the first entries with a backdrop, one per title, across the shelves.
/// First run next to an installed Nuvio for Windows (Kotlin): offer to bring its addons over, once.
fn import_offer(cx: &mut Cx, ui: &mut Ui, pad: f32) {
    let fresh = cx.state.kotlin_addons.iter().filter(|u| !cx.state.addon_list.contains(u)).count();
    if fresh == 0 || cx.state.settings.import_prompt_dismissed {
        return;
    }
    let t = theme::get();
    let width = ui.available_width() - pad * 2.0;
    ui.horizontal(|ui| {
        ui.add_space(pad);
        let (rect, _) = ui.allocate_exact_size(vec2(width, 68.0), Sense::hover());
        ui_kit::glass_panel(ui, rect, 22.0, 1.0);
        let text = format!("Nuvio for Windows is installed with {fresh} addon{}. Bring {} over?", if fresh == 1 { "" } else { "s" }, if fresh == 1 { "it" } else { "them" });
        ui_kit::draw_text(ui, rect.left_center() + vec2(24.0, 0.0), Align2::LEFT_CENTER, &text, theme::semibold(size::BODY_MD), t.text);
        let mut buttons = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_size(rect.right_top() + vec2(-290.0, 12.0), vec2(270.0, 44.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
        if ui_kit::pill_button(&mut buttons, Some(crate::icons::DOWNLOAD), "Import", PillStyle::Primary).clicked() {
            cx.actions.push(Action::ImportKotlinAddons);
            cx.state.settings.import_prompt_dismissed = true;
            cx.state.save_settings();
        }
        buttons.add_space(8.0);
        if ui_kit::pill_button(&mut buttons, None, "Not now", PillStyle::Soft).clicked() {
            cx.state.settings.import_prompt_dismissed = true;
            cx.state.save_settings();
        }
    });
    ui.add_space(22.0);
}

fn hero_items(cx: &Cx) -> Vec<MetaPreview> {
    let mut seen = std::collections::HashSet::new();
    let mut picked = Vec::new();
    'outer: for section in &cx.state.sections {
        for item in section.items.iter().take(4) {
            if item.background.is_some() && seen.insert(item.key()) {
                picked.push(item.clone());
                if picked.len() >= MAX_HERO_ITEMS {
                    break 'outer;
                }
            }
        }
    }
    picked
}

fn hero(cx: &mut Cx, ui: &mut Ui, items: &[MetaPreview]) {
    let t = theme::get();
    let width = ui.available_width();
    // The banner is the picture, nearly as tall as the window: the backdrops are 16:9, and a short strip of one cuts the
    // people out of it. The first shelf's title peeks out under it.
    let height = (ui.ctx().content_rect().height() * 0.86).min(width * 9.0 / 16.0).max(380.0);
    let (rect, response) = ui.allocate_exact_size(vec2(width, height), Sense::click());
    let view = &mut *cx.view;
    if view.hero_index >= items.len() {
        view.hero_index = 0;
        view.hero_prev = 0;
    }
    // The banner stands still while the pointer is on it and while the window is in the background.
    let attended = ui.ctx().input(|i| i.focused);
    if view.hero_since.elapsed() > HERO_INTERVAL && !response.hovered() && attended {
        view.hero_prev = view.hero_index;
        view.hero_index = (view.hero_index + 1) % items.len();
        view.hero_since = std::time::Instant::now();
    }
    // Wake up when the next slide is due, not before: an idle home screen should cost nothing.
    // A slide that is due but held back (pointer on it, window in the background) needs no timer: the input that ends the
    // hold draws a frame.
    let remaining = HERO_INTERVAL.saturating_sub(view.hero_since.elapsed());
    if !remaining.is_zero() {
        ui.ctx().request_repaint_after(remaining + Duration::from_millis(30));
    }
    let item = &items[view.hero_index];
    // The next backdrop and wordmark are fetched ahead, so the crossfade never starts from a blank placeholder.
    if items.len() > 1 {
        let next = &items[(view.hero_index + 1) % items.len()];
        let ppp = ui.ctx().pixels_per_point();
        if let Some(url) = next.background.as_deref() {
            cx.svc.images.prefetch(url, rect.width() * ppp);
        }
        if let Some(url) = next.logo.as_deref() {
            cx.svc.images.prefetch(url, 560.0 * ppp);
        }
    }

    let bottom = CornerRadius { nw: 0, ne: 0, sw: 28, se: 28 };
    // For a moment after a switch the old backdrop stays and the new one fades in over it. Backdrops are shown whole: one that
    // is not as wide as the banner sits against its right edge with the rest filled in, nothing is cropped away.
    let fade = (view.hero_since.elapsed().as_secs_f32() / HERO_FADE).min(1.0);
    let crossfading = fade < 1.0 && view.hero_prev != view.hero_index;
    if crossfading {
        if let Some(url) = items[view.hero_prev].background.as_deref() {
            ui_kit::paint_backdrop_whole(ui, &cx.svc.images, url, rect, bottom, 1.0);
        }
        crate::services::animate_on(ui.ctx());
    }
    match item.background.as_deref() {
        Some(url) => ui_kit::paint_backdrop_whole(ui, &cx.svc.images, url, rect, bottom, if crossfading { fade } else { 1.0 }),
        None => {
            ui.painter().rect_filled(rect, bottom, t.card);
        }
    }

    // Legibility: a dark wash from the left, and the page colour rising from the bottom.
    ui_kit::horizontal_gradient(ui, Rect::from_min_max(rect.min, pos2(rect.left() + rect.width() * 0.62, rect.bottom())), Color32::from_black_alpha(190), Color32::from_black_alpha(0));
    ui_kit::vertical_gradient(ui, Rect::from_min_max(pos2(rect.left(), rect.bottom() - 260.0), rect.max), Color32::from_black_alpha(0), with_alpha(t.background, 1.0));

    let pad = page_padding(width);
    let text_w = (width * 0.5).min(620.0);
    let mut y = rect.bottom() - 52.0;

    // Page dots.
    if items.len() > 1 {
        let mut x = rect.left() + pad;
        for i in 0..items.len() {
            let on = i == view.hero_index;
            let w = animate(ui, egui::Id::unique(("hero-dot", i)), on, theme::motion::NORMAL);
            let dot = Rect::from_min_size(pos2(x, y), vec2(8.0 + 24.0 * w, 8.0));
            ui.painter().rect_filled(dot, radius_of(4.0), lerp_color(with_alpha(Color32::WHITE, 0.35), t.accent, w));
            let dot_response = ui.interact(dot.expand(6.0), egui::Id::unique(("hero-dot-hit", i)), Sense::click());
            if dot_response.clicked() {
                view.hero_prev = view.hero_index;
                view.hero_index = i;
                view.hero_since = std::time::Instant::now();
            }
            x += dot.width() + 8.0;
        }
    }
    y -= 20.0;

    // Buttons.
    let buttons_top = y - 42.0;
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_size(pos2(rect.left() + pad, buttons_top), vec2(text_w, 42.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
    let play = ui_kit::pill_button(&mut child, Some(crate::icons::PLAY), "Play", PillStyle::Primary);
    child.add_space(10.0);
    let info = ui_kit::pill_button(&mut child, Some(crate::icons::INFO), "More info", PillStyle::Soft);
    let open = |cx: &mut Cx| cx.actions.push(Action::OpenDetails { kind: item.kind.clone(), id: item.id.clone(), preview: Some(item.clone()) });
    if play.clicked() {
        if item.kind == "movie" {
            cx.actions.push(Action::OpenDetails { kind: item.kind.clone(), id: item.id.clone(), preview: Some(item.clone()) });
            cx.actions.push(Action::OpenStreams { video: None });
        } else {
            open(cx);
        }
    } else if info.clicked() || (response.clicked() && !play.hovered()) {
        open(cx);
    }
    y = buttons_top - 16.0;

    // Description.
    if let Some(description) = item.description.as_deref() {
        let g = ui_kit::galley(ui, description, theme::regular(size::BODY_APP), with_alpha(t.text, 0.86), text_w, 3);
        y -= g.size().y;
        ui.painter().galley(pos2(rect.left() + pad, y), g, t.text);
        y -= 14.0;
    }

    // Facts line: rating, year, genres.
    let mut facts: Vec<String> = Vec::new();
    if let Some(rating) = item.imdb_rating.as_deref() {
        facts.push(format!("{rating} IMDb"));
    }
    if let Some(year) = item.release_info.as_deref() {
        facts.push(year.to_owned());
    }
    facts.extend(item.genres.iter().take(3).cloned());
    if !facts.is_empty() {
        y -= 20.0;
        ui_kit::draw_text(ui, pos2(rect.left() + pad, y + 10.0), Align2::LEFT_CENTER, &facts.join("   ·   "), theme::semibold(size::BODY_SM), t.text_secondary);
        y -= 14.0;
    }

    // Title: the wordmark when the addon has one, text otherwise.
    let logo = item.logo.as_deref().map(|url| cx.svc.images.get(url, 560.0 * ui.ctx().pixels_per_point()));
    match logo {
        Some(Image::Ready { texture, age }) => {
            let tex = texture.size_vec2();
            let scale = (text_w / tex.x).min(110.0 / tex.y);
            let size = tex * scale;
            let alpha = (age.as_secs_f32() / 0.3).min(1.0);
            let logo_rect = Rect::from_min_size(pos2(rect.left() + pad, y - size.y), size);
            egui::Image::new((texture.id(), size)).tint(Color32::from_white_alpha((alpha * 255.0) as u8)).paint_at(ui, logo_rect);
        }
        // The wordmark is on its way: showing the name until it arrives would swap text for picture in front of the viewer.
        Some(Image::Loading) => {}
        _ => {
            let g = ui_kit::galley(ui, &item.name, theme::bold(size::PAGE_DISPLAY + 6.0), Color32::WHITE, text_w, 2);
            y -= g.size().y;
            ui.painter().galley(pos2(rect.left() + pad, y), g, Color32::WHITE);
        }
    }
    if response.hovered() && !play.hovered() && !info.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
}

fn continue_watching(cx: &mut Cx, ui: &mut Ui, entries: &[ContinueItem], pad: f32) {
    let t = theme::get();
    ui.horizontal(|ui| {
        ui.add_space(pad);
        ui_kit::section_header(ui, "Continue Watching", None);
    });
    ui.add_space(10.0);
    let card_w = 300.0;
    let card_h = card_w / 1.77;
    ui_kit::paging_row(ui, egui::Id::unique("cw-scroll"), pad, card_h / 2.0, |ui| {
        for entry in entries {
            let progress = &entry.progress;
            let (rect, response) = ui.allocate_exact_size(vec2(card_w, card_h + 54.0), Sense::click());
            let hover = animate(ui, response.id.with("hover"), response.hovered(), theme::motion::NORMAL);
            let art = Rect::from_min_size(rect.min, vec2(card_w, card_h));
            let grown = art.expand2(art.size() * 0.02 * hover);
            ui_kit::paint_cover_url_sized(ui, &cx.svc.images, progress.background.as_deref().or(progress.poster.as_deref()), grown, card_w, radius_of(radius::POSTER));
            ui_kit::vertical_gradient(ui, Rect::from_min_max(pos2(grown.left(), grown.bottom() - 70.0), grown.max), Color32::from_black_alpha(0), Color32::from_black_alpha(190));
            if hover > 0.0 {
                ui.painter().rect_stroke(grown, radius_of(radius::POSTER), egui::Stroke::new(2.0, with_alpha(t.accent, hover)), egui::StrokeKind::Inside);
            }
            // Progress bar along the bottom edge; an episode that is still to start has none.
            if entry.up_next.is_none() {
                let bar = Rect::from_min_size(pos2(grown.left() + 10.0, grown.bottom() - 12.0), vec2(grown.width() - 20.0, 4.0));
                ui.painter().rect_filled(bar, radius_of(2.0), with_alpha(Color32::WHITE, 0.25));
                ui.painter().rect_filled(Rect::from_min_size(bar.min, vec2(bar.width() * progress.fraction() as f32, 4.0)), radius_of(2.0), t.accent);
            }
            // "21m left" badge, or "Up next" when the episode before it was watched to the end.
            let badge = if entry.up_next.is_some() { "Up next".to_owned() } else { format!("{}m left", progress.remaining_minutes()) };
            let g = ui_kit::galley(ui, &badge, theme::semibold(size::LABEL_XS), Color32::WHITE, 120.0, 1);
            let badge_rect = Rect::from_min_size(pos2(grown.right() - g.size().x - 22.0, grown.top() + 8.0), g.size() + vec2(14.0, 8.0));
            ui.painter().rect_filled(badge_rect, radius_of(8.0), if entry.up_next.is_some() { t.accent } else { Color32::from_black_alpha(170) });
            ui.painter().galley(badge_rect.min + vec2(7.0, 4.0), g, if entry.up_next.is_some() { t.on_accent } else { Color32::WHITE });

            let label = match (entry.season(), entry.episode()) {
                (Some(s), Some(e)) => format!("S{s} E{e}"),
                _ => "Movie".to_owned(),
            };
            ui_kit::draw_text(ui, pos2(rect.left() + 12.0, art.bottom() - 30.0), Align2::LEFT_CENTER, &label, theme::semibold(size::LABEL_SM), Color32::WHITE);
            let title = ui_kit::galley(ui, &progress.name, theme::semibold(size::BODY_MD), t.text, card_w, 1);
            ui.painter().galley(pos2(rect.left(), art.bottom() + 10.0), title, t.text);
            if let Some(episode) = entry.episode_title() {
                let sub = ui_kit::galley(ui, episode, theme::regular(size::LABEL_SM), t.text_muted, card_w, 1);
                ui.painter().galley(pos2(rect.left(), art.bottom() + 30.0), sub, t.text_muted);
            }
            if response.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            if response.clicked() {
                cx.actions.push(Action::Resume(Box::new(entry.clone())));
            }
            super::context::title_menu(cx.state, cx.actions, &response, &super::context::preview_of(entry));
        }
    });
}

fn shelf(cx: &mut Cx, ui: &mut Ui, index: usize, pad: f32) {
    let t = theme::get();
    let width = ui.available_width();
    let (title, subtitle, key, state, count, has_more, loading_more) = {
        let s = &cx.state.sections[index];
        (s.title.clone(), s.subtitle.clone(), s.key.clone(), s.state.clone(), s.items.len(), s.has_more, s.loading_more)
    };
    // Portrait posters get wider with the window, within a range that keeps the title readable. Whole points, in steps of
    // two: the pictures are prepared at the exact width they are drawn at, and resizing the window should not mean
    // preparing every one again for each pixel.
    let card_w = ((width / 8.4).clamp(150.0, 210.0) / 2.0).floor() * 2.0;
    let own_shape = if cx.state.sections[index].items.first().is_some_and(|i| i.shape == PosterShape::Landscape) { PosterShape::Landscape } else { PosterShape::Poster };
    let shape = match cx.state.settings.poster_style.as_str() {
        "portrait" => PosterShape::Poster,
        "landscape" => PosterShape::Landscape,
        _ => own_shape,
    };
    let card_w = if shape == PosterShape::Landscape { (card_w * 1.45).floor() } else { card_w };
    let row_h = ui_kit::card_size(shape, card_w).y + 44.0;

    ui.horizontal(|ui| {
        ui.add_space(pad);
        ui.allocate_ui(vec2(width - pad * 2.0, 34.0), |ui| {
            ui_kit::section_header(ui, &title, Some(&subtitle));
        });
    });
    ui.add_space(10.0);

    if count == 0 {
        match state {
            Load::Failed(error) => {
                ui.horizontal(|ui| {
                    ui.add_space(pad);
                    ui.label(egui::RichText::new(format!("Could not load this catalog: {error}")).color(t.text_muted).size(size::BODY_SM));
                });
            }
            Load::Loading => skeleton_row(ui, pad, card_w, row_h, shape),
            Load::Ready(()) => {}
        }
        return;
    }

    let scroll_id = egui::Id::unique(("shelf", &key));
    let view = ui_kit::paging_row(ui, scroll_id, pad, ui_kit::card_size(shape, card_w).y / 2.0, |ui| {
        for item in cx.state.sections[index].items.iter() {
            let response = PosterCard { images: &cx.svc.images, item, width: card_w, shape, show_title: true }.show(ui);
            if response.clicked() {
                cx.actions.push(Action::OpenDetails { kind: item.kind.clone(), id: item.id.clone(), preview: Some(item.clone()) });
            }
            super::context::title_menu(cx.state, cx.actions, &response, item);
        }
        if loading_more {
            ui.add_space(20.0);
            let (r, _) = ui.allocate_exact_size(vec2(40.0, row_h), Sense::hover());
            ui_kit::spinner(ui, r.center_top() + vec2(0.0, card_w * 0.7), 14.0, t.text_muted);
        }
    });
    // Ask for the next page when the end is near.
    if has_more && view.offset + view.view_w > view.content_w - card_w * 3.0 {
        cx.state.load_more(cx.svc, &key);
    }
}

fn skeleton_row(ui: &mut Ui, pad: f32, card_w: f32, row_h: f32, shape: PosterShape) {
    ui.horizontal(|ui| {
        ui.add_space(pad);
        ui.spacing_mut().item_spacing.x = 14.0;
        for _ in 0..8 {
            let (rect, _) = ui.allocate_exact_size(vec2(card_w, row_h), Sense::hover());
            if !ui.is_rect_visible(rect) {
                continue;
            }
            let art = Rect::from_min_size(rect.min, ui_kit::card_size(shape, card_w));
            ui_kit::paint_cover(ui, art, &Image::Loading, radius::POSTER);
            ui.painter().rect_filled(Rect::from_min_size(pos2(rect.left(), art.bottom() + 12.0), vec2(card_w * 0.7, 12.0)), radius_of(6.0), with_alpha(Color32::WHITE, 0.06));
        }
    });
}

fn empty_state(cx: &mut Cx, ui: &mut Ui, any_addon: bool) {
    let t = theme::get();
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(width, 320.0), Sense::hover());
    let loading = cx.state.addons.iter().any(|a| matches!(a.status, Load::Loading));
    if loading {
        ui_kit::spinner(ui, rect.center() - vec2(0.0, 30.0), 18.0, t.accent);
        ui_kit::draw_text(ui, rect.center() + vec2(0.0, 24.0), Align2::CENTER_CENTER, "Loading your addons…", theme::semibold(size::BODY_LG), t.text_secondary);
        return;
    }
    let message = if any_addon { "Your addons have no catalogs to show here." } else { "Add an addon to see what to watch." };
    ui_kit::draw_text(ui, rect.center() - vec2(0.0, 30.0), Align2::CENTER_CENTER, message, theme::semibold(size::TITLE_SM), t.text);
    ui_kit::draw_text(ui, rect.center(), Align2::CENTER_CENTER, "Addons provide the catalogs and the streams. Settings → Addons has the list.", theme::regular(size::BODY_SM), t.text_muted);
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_center_size(rect.center() + vec2(0.0, 56.0), vec2(420.0, 44.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
    child.add_space(70.0);
    if ui_kit::pill_button(&mut child, Some(crate::icons::SETTINGS), "Open addon settings", PillStyle::Primary).clicked() {
        cx.actions.push(Action::Tab(super::Tab::Settings));
    }
}
