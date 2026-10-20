//! The stream list of a title or episode: every addon's answer in one list, narrowed by resolution, addon, service,
//! HDR and language.

use eframe::egui::{self, Align2, Color32, CornerRadius, Popup, Rect, ScrollArea, Sense, Ui, pos2, vec2};
use nuvio_core::stream::{DOLBY_VISION, Facet, Stream, facet_options, format_size};

use super::{Action, Cx, PlayRequest};
use crate::icons;
use crate::state::{Load, StreamsState};
use crate::theme::{self, radius, size};
use crate::ui_kit::{self, animate, galley, page_padding, radius_of, with_alpha};

pub fn show(cx: &mut Cx, ui: &mut Ui) {
    let t = theme::get();
    let screen = ui.available_rect_before_wrap();
    let Some(state) = cx.state.streams.as_ref() else { return };
    let width = screen.width();
    let pad = page_padding(width);

    // The backdrop, dimmed, behind everything.
    ui_kit::paint_cover_url_with(ui, &cx.svc.images, state.background.as_deref().or(state.poster.as_deref()), screen, CornerRadius::ZERO);
    ui.painter().rect_filled(screen, 0.0, with_alpha(t.background, 0.78));
    ui_kit::vertical_gradient(ui, Rect::from_min_max(pos2(screen.left(), screen.bottom() - 260.0), screen.max), Color32::from_black_alpha(0), with_alpha(t.background, 0.9));

    let wide = width >= 1080.0;
    let panel_w = if wide { (width * 0.56).min(820.0) } else { (width - pad * 2.0).min(820.0) };
    let panel = Rect::from_min_max(pos2(screen.right() - pad.min(40.0) - panel_w, screen.top() + 24.0), pos2(screen.right() - pad.min(40.0), screen.bottom() - 24.0));

    if wide {
        summary(cx, ui, screen, pad, panel.left() - 40.0);
    }
    super::details::back_button(cx, ui, screen.left_top() + vec2(20.0, 20.0));

    ui_kit::glass_panel(ui, panel, radius::CARD, 1.0);
    let inner = panel.shrink2(vec2(22.0, 18.0));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(inner).layout(egui::Layout::top_down(egui::Align::Min)));
    panel_contents(cx, &mut child);
}

/// Poster, title and episode on the left of the panel.
fn summary(cx: &Cx, ui: &mut Ui, screen: Rect, pad: f32, right_edge: f32) {
    let t = theme::get();
    let Some(state) = cx.state.streams.as_ref() else { return };
    let col_w = (right_edge - pad - 20.0).clamp(240.0, 420.0);
    let poster_w = (col_w * 0.62).min(260.0);
    let top = screen.top() + 110.0;
    let poster = Rect::from_min_size(pos2(screen.left() + pad, top), vec2(poster_w, poster_w * 1.5));
    ui.painter().add(egui::Shadow { offset: [0, 14], blur: 36, spread: 0, color: Color32::from_black_alpha(150) }.as_shape(poster, radius_of(radius::POSTER + 4.0)));
    ui_kit::paint_cover_url(ui, &cx.svc.images, state.poster.as_deref().or(state.background.as_deref()), poster, radius::POSTER + 4.0);
    let mut y = poster.bottom() + 22.0;
    let name = galley(ui, &state.name, theme::bold(size::TITLE_LG), t.text, col_w, 3);
    ui.painter().galley(pos2(poster.left(), y), name.clone(), t.text);
    y += name.size().y + 8.0;
    if let Some(label) = &state.episode_label {
        let line = match &state.episode_title {
            Some(title) => format!("{label} · {title}"),
            None => label.clone(),
        };
        let g = galley(ui, &line, theme::semibold(size::BODY_LG), t.text_secondary, col_w, 2);
        ui.painter().galley(pos2(poster.left(), y), g, t.text_secondary);
    }
}

fn panel_contents(cx: &mut Cx, ui: &mut Ui) {
    let t = theme::get();
    let (pending, total, any_failed) = {
        let state = cx.state.streams.as_ref().expect("streams state");
        (state.pending(), state.all_streams().len(), state.groups.iter().any(|g| matches!(g.state, Load::Failed(_))))
    };

    // Header.
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(ui.available_width() - 48.0, 36.0), Sense::hover());
        ui_kit::draw_text(ui, r.left_center(), Align2::LEFT_CENTER, "Streams", theme::bold(size::TITLE_MD), t.text);
        let count = if total > 0 { format!("{total}") } else { String::new() };
        let title_w = ui.painter().layout_no_wrap("Streams".into(), theme::bold(size::TITLE_MD), t.text).size().x;
        ui_kit::draw_text(ui, r.left_center() + vec2(title_w + 12.0, 2.0), Align2::LEFT_CENTER, &count, theme::semibold(size::BODY_MD), t.text_muted);
        if ui_kit::icon_button(ui, icons::REFRESH, 20.0, "Ask the addons again").clicked() {
            let state = cx.state.streams.take();
            if let Some(state) = state {
                let fresh = StreamsState { groups: Vec::new(), ..state };
                cx.state.open_streams(cx.svc, fresh);
            }
        }
    });
    ui.add_space(8.0);

    filters(cx, ui);
    ui.add_space(10.0);

    // Status line while addons are still answering.
    if pending > 0 {
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::hover());
            ui_kit::spinner(ui, r.center(), 8.0, t.text_muted);
            ui.label(egui::RichText::new(format!("Waiting for {pending} addon{}…", if pending == 1 { "" } else { "s" })).color(t.text_muted).size(size::BODY_SM));
        });
        ui.add_space(6.0);
    } else if any_failed && total == 0 {
        ui.label(egui::RichText::new("The addons did not return any streams.").color(t.text_muted).size(size::BODY_SM));
    }
    if cx.state.streams.as_ref().is_some_and(|s| s.groups.is_empty()) {
        ui.add_space(12.0);
        ui.label(egui::RichText::new("None of your addons provides streams for this title. Add one in Settings → Addons.").color(t.text_muted).size(size::BODY_MD));
        return;
    }

    let streams: Vec<Stream> = {
        let state = cx.state.streams.as_ref().expect("streams state");
        state.all_streams().into_iter().filter(|s| s.matches(&state.selection, None)).cloned().collect()
    };
    if streams.is_empty() && pending == 0 && total > 0 {
        ui.add_space(16.0);
        ui.label(egui::RichText::new("No stream matches these filters.").color(t.text_muted).size(size::BODY_MD));
        return;
    }

    ScrollArea::vertical().id_salt("stream-list").auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 10.0;
        for stream in &streams {
            if stream_card(ui, stream).clicked() {
                if let Some(request) = play_request(cx, stream) {
                    cx.actions.push(Action::Play(Box::new(request)));
                }
            }
        }
        ui.add_space(8.0);
    });
}

fn play_request(cx: &Cx, stream: &Stream) -> Option<PlayRequest> {
    let state = cx.state.streams.as_ref()?;
    let video = state.meta.as_ref().and_then(|m| m.video(&state.video_id)).cloned();
    Some(PlayRequest {
        kind: state.kind.clone(),
        title_id: state.title_id.clone(),
        video_id: state.video_id.clone(),
        name: state.name.clone(),
        poster: state.poster.clone(),
        background: state.background.clone(),
        video,
        stream: stream.clone(),
    })
}

fn filters(cx: &mut Cx, ui: &mut Ui) {
    let t = theme::get();
    let (all, selection): (Vec<Stream>, _) = {
        let state = cx.state.streams.as_ref().expect("streams state");
        (state.all_streams().into_iter().cloned().collect(), state.selection.clone())
    };
    if all.is_empty() {
        return;
    }
    let mut change: Option<(Facet, Option<String>)> = None;
    let mut clear = false;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
        for facet in Facet::ALL {
            let options = facet_options(&all, facet, &selection);
            let selected = selection.get(&facet).cloned();
            // A facet with one value (or none) cannot narrow anything, unless it is already narrowed.
            if options.len() < 2 && selected.is_none() {
                continue;
            }
            let label = selected.clone().unwrap_or_else(|| facet.label().to_owned());
            let text = galley(ui, &label, theme::semibold(size::BODY_SM), t.text, 260.0, 1);
            let (rect, response) = ui.allocate_exact_size(vec2(text.size().x + 50.0, 34.0), Sense::click());
            let hover = animate(ui, response.id.with("hover"), response.hovered(), theme::motion::FAST);
            let active = selected.is_some();
            let fill = if active { with_alpha(t.accent, 0.9) } else { with_alpha(Color32::WHITE, 0.08 + 0.06 * hover) };
            let fg = if active { t.on_accent } else { t.text };
            ui.painter().rect_filled(rect, radius_of(radius::CHIP), fill);
            ui.painter().galley_with_override_text_color(pos2(rect.left() + 14.0, rect.center().y - text.size().y / 2.0), text, fg);
            ui_kit::draw_icon(ui, pos2(rect.right() - 18.0, rect.center().y), icons::EXPAND_MORE, 20.0, fg);
            if response.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            Popup::menu(&response).gap(6.0).show(|ui| {
                ui.set_min_width(200.0);
                if option_row(ui, "All", None, selected.is_none()).clicked() {
                    change = Some((facet, None));
                }
                for option in &options {
                    let label = if facet == Facet::Hdr && option.value == DOLBY_VISION { "Dolby Vision".to_owned() } else { option.value.clone() };
                    if option_row(ui, &label, Some(option.count), selected.as_deref() == Some(option.value.as_str())).clicked() {
                        change = Some((facet, Some(option.value.clone())));
                    }
                }
            });
        }
        if !selection.is_empty() && ui_kit::chip(ui, "Clear filters", false).clicked() {
            clear = true;
        }
    });
    if clear {
        if let Some(state) = cx.state.streams.as_mut() {
            state.selection.clear();
        }
    }
    if let Some((facet, value)) = change {
        if let Some(state) = cx.state.streams.as_mut() {
            match value {
                Some(value) => {
                    state.selection.insert(facet, value);
                }
                None => {
                    state.selection.remove(&facet);
                }
            }
        }
    }
}

fn option_row(ui: &mut Ui, label: &str, count: Option<usize>, selected: bool) -> egui::Response {
    let t = theme::get();
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width().max(200.0), 34.0), Sense::click());
    let hover = animate(ui, response.id.with("hover"), response.hovered(), theme::motion::FAST);
    if hover > 0.0 {
        ui.painter().rect_filled(rect, radius_of(8.0), with_alpha(Color32::WHITE, 0.08 * hover));
    }
    let colour = if selected { t.accent } else { t.text };
    ui_kit::draw_text(ui, rect.left_center() + vec2(12.0, 0.0), Align2::LEFT_CENTER, label, theme::semibold(size::BODY_SM), colour);
    if let Some(count) = count {
        ui_kit::draw_text(ui, rect.right_center() - vec2(12.0, 0.0), Align2::RIGHT_CENTER, &count.to_string(), theme::regular(size::LABEL_SM), t.text_muted);
    }
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    response
}

fn stream_card(ui: &mut Ui, stream: &Stream) -> egui::Response {
    let t = theme::get();
    let width = ui.available_width();
    let inner_w = width - 32.0;
    let playable = stream.is_playable();
    let colour = if playable { t.text } else { t.text_muted };

    let title = galley(ui, stream.label(), theme::bold(size::BODY_LG), colour, inner_w, 2);
    let body = stream
        .description
        .as_deref()
        .map(|d| ui_kit::galley_ex(ui, d, theme::regular(size::BODY_SM), t.text_muted, inner_w, 7, true));
    let mut chips: Vec<String> = Vec::new();
    if let Some(size) = stream.video_size {
        chips.push(format!("SIZE {}", format_size(size).to_uppercase()));
    }
    if stream.info_hash.is_some() || stream.torrent_hash().is_some() {
        chips.push("P2P".to_owned());
    }
    if !stream.addon_name.is_empty() {
        chips.push(stream.addon_name.clone());
    }
    let height = 14.0 + title.size().y + 6.0 + body.as_ref().map(|g| g.size().y + 8.0).unwrap_or(0.0) + 28.0 + 12.0;
    let (rect, response) = ui.allocate_exact_size(vec2(width, height), Sense::click());
    let hover = animate(ui, response.id.with("hover"), response.hovered() && playable, theme::motion::FAST);
    let press = animate(ui, response.id.with("press"), response.is_pointer_button_down_on() && playable, 0.08);
    let rect = rect.shrink(press * 1.2);
    ui.painter().rect_filled(rect, radius_of(radius::POSTER + 2.0), with_alpha(Color32::WHITE, 0.07 + 0.06 * hover));
    if hover > 0.0 {
        ui.painter().rect_stroke(rect, radius_of(radius::POSTER + 2.0), egui::Stroke::new(1.5, with_alpha(t.accent, 0.9 * hover)), egui::StrokeKind::Inside);
    }
    let mut y = rect.top() + 14.0;
    let title_h = title.size().y;
    ui.painter().galley(pos2(rect.left() + 16.0, y), title, colour);
    y += title_h + 6.0;
    if let Some(body) = body {
        let h = body.size().y;
        ui.painter().galley(pos2(rect.left() + 16.0, y), body, t.text_muted);
        y += h + 8.0;
    }
    let mut x = rect.left() + 16.0;
    for chip in chips {
        let g = galley(ui, &chip, theme::bold(size::LABEL_XS), Color32::WHITE, 260.0, 1);
        let chip_rect = Rect::from_min_size(pos2(x, y), vec2(g.size().x + 20.0, 24.0));
        ui.painter().rect_filled(chip_rect, radius_of(8.0), with_alpha(Color32::BLACK, 0.45));
        ui.painter().galley(pos2(chip_rect.left() + 10.0, chip_rect.center().y - g.size().y / 2.0), g, Color32::WHITE);
        x = chip_rect.right() + 8.0;
    }
    if response.hovered() && playable {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    response
}
