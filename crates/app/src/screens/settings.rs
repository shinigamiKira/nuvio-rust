//! Settings: addons, playback, appearance, storage and about, in a two pane layout.

use eframe::egui::{self, Align2, Color32, Popup, Rect, ScrollArea, Sense, Ui, pos2, vec2};
use nuvio_core::addon::normalize_manifest_url;
use nuvio_core::paths;

use super::{Action, Cx};
use crate::icons;
use crate::state::Load;
use crate::theme::{self, PALETTES, radius, size};
use crate::ui_kit::{self, PillStyle, animate, page_padding, radius_of, switch_row, text_field, with_alpha};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Addons,
    Playback,
    Appearance,
    Storage,
    About,
}

const PAGES: [(Page, char, &str); 5] = [
    (Page::Addons, icons::EXTENSION, "Addons"),
    (Page::Playback, icons::PLAY, "Playback"),
    (Page::Appearance, icons::PALETTE, "Appearance"),
    (Page::Storage, icons::FOLDER, "Storage"),
    (Page::About, icons::INFO, "About"),
];

pub fn show(cx: &mut Cx, ui: &mut Ui) {
    let width = ui.available_width();
    let pad = page_padding(width);
    let top = 104.0;
    let nav_w = 230.0;
    let screen = ui.available_rect_before_wrap();
    let nav = Rect::from_min_size(pos2(screen.left() + pad, screen.top() + top), vec2(nav_w, screen.height() - top));
    let content = Rect::from_min_max(pos2(nav.right() + 36.0, screen.top() + top), pos2(screen.right() - pad, screen.bottom()));

    // Page list.
    let mut y = nav.top();
    ui_kit::draw_text(ui, pos2(nav.left(), y + 14.0), Align2::LEFT_CENTER, "Settings", theme::bold(size::TITLE_LG), theme::get().text);
    y += 58.0;
    for (page, icon, label) in PAGES {
        let row = Rect::from_min_size(pos2(nav.left(), y), vec2(nav_w, 46.0));
        let response = ui.interact(row, egui::Id::unique(("settings-page", label)), Sense::click());
        let hover = animate(ui, response.id.with("hover"), response.hovered(), theme::motion::FAST);
        let on = animate(ui, response.id.with("on"), cx.view.settings_page == page, theme::motion::NORMAL);
        let t = theme::get();
        ui.painter().rect_filled(row, radius_of(radius::BUTTON), with_alpha(Color32::WHITE, 0.05 * hover + 0.08 * on));
        let colour = ui_kit::lerp_color(t.text_secondary, t.accent, on);
        ui_kit::draw_icon(ui, pos2(row.left() + 26.0, row.center().y), icon, 22.0, colour);
        ui_kit::draw_text(ui, pos2(row.left() + 52.0, row.center().y), Align2::LEFT_CENTER, label, theme::semibold(size::BODY_LG), ui_kit::lerp_color(t.text_secondary, t.text, on));
        if response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if response.clicked() {
            cx.view.settings_page = page;
        }
        y += 52.0;
    }

    // Page content.
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(content).layout(egui::Layout::top_down(egui::Align::Min)));
    ScrollArea::vertical().id_salt(("settings-content", cx.view.settings_page as u8)).auto_shrink([false, false]).show(&mut child, |ui| {
        ui.set_max_width(content.width().min(780.0));
        ui.spacing_mut().item_spacing.y = 6.0;
        match cx.view.settings_page {
            Page::Addons => addons(cx, ui),
            Page::Playback => playback(cx, ui),
            Page::Appearance => appearance(cx, ui),
            Page::Storage => storage(cx, ui),
            Page::About => about(ui),
        }
        ui.add_space(60.0);
    });
}

fn heading(ui: &mut Ui, text: &str) {
    let t = theme::get();
    ui.add_space(6.0);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::hover());
    ui_kit::draw_text(ui, rect.left_center(), Align2::LEFT_CENTER, text, theme::bold(size::TITLE_MD), t.text);
}

fn note(ui: &mut Ui, text: &str) {
    let t = theme::get();
    let g = ui_kit::galley(ui, text, theme::regular(size::BODY_SM), t.text_muted, ui.available_width(), 6);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), g.size().y + 6.0), Sense::hover());
    ui.painter().galley(rect.min, g, t.text_muted);
}

fn addons(cx: &mut Cx, ui: &mut Ui) {
    let t = theme::get();
    heading(ui, "Addons");
    note(ui, "Addons provide the catalogs on Home and the streams you play. Paste an addon's install link below.");
    ui.add_space(10.0);

    ui.horizontal(|ui| {
        let field_w = ui.available_width() - 130.0;
        let response = text_field(ui, &mut cx.view.add_addon_text, "https://…/manifest.json", field_w);
        let submit = ui_kit::pill_button(ui, None, "Install", PillStyle::Primary).clicked()
            || (response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
        if submit {
            match normalize_manifest_url(&cx.view.add_addon_text) {
                Some(url) => {
                    cx.actions.push(Action::AddAddon(url));
                    cx.view.add_addon_text.clear();
                    cx.view.add_addon_error = None;
                }
                None => cx.view.add_addon_error = Some("Paste the addon's link first.".into()),
            }
        }
    });
    if let Some(error) = &cx.view.add_addon_error {
        ui.label(egui::RichText::new(error).color(t.danger).size(size::BODY_SM));
    }

    if !cx.state.kotlin_addons.is_empty() {
        let fresh = cx.state.kotlin_addons.iter().filter(|u| !cx.state.addon_list.contains(u)).count();
        if fresh > 0 {
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(format!("Found {fresh} addon{} in your Nuvio for Windows profile.", if fresh == 1 { "" } else { "s" })).color(t.text_secondary).size(size::BODY_MD));
                if ui_kit::pill_button(ui, Some(icons::DOWNLOAD), "Import", PillStyle::Soft).clicked() {
                    cx.actions.push(Action::ImportKotlinAddons);
                }
            });
        }
    }
    ui.add_space(18.0);

    let count = cx.state.addons.len();
    if count == 0 {
        note(ui, "No addons installed.");
    }
    let mut remove: Option<String> = None;
    let mut toggle: Option<(String, bool)> = None;
    let mut shift: Option<(String, isize)> = None;
    for index in 0..count {
        let addon = &cx.state.addons[index];
        let url = addon.entry.manifest_url.clone();
        let name = addon.display_name();
        let version = addon.manifest.as_ref().map(|m| format!("v{}", m.version)).unwrap_or_default();
        let description = addon.manifest.as_ref().map(|m| m.description.clone()).unwrap_or_default();
        let logo = addon.manifest.as_ref().and_then(|m| m.logo.clone()).map(|l| nuvio_core::addon::resolve_against(&url, &l));
        let catalogs = addon.manifest.as_ref().map(|m| m.catalogs.len()).unwrap_or(0);
        let has_streams = addon.manifest.as_ref().is_some_and(|m| m.has_resource("stream"));
        let status = addon.status.clone();
        let enabled = addon.entry.enabled;

        let width = ui.available_width();
        let desc = ui_kit::galley(ui, &description, theme::regular(size::BODY_SM), t.text_muted, width - 250.0, 2);
        let height = (70.0 + desc.size().y).max(92.0);
        let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
        ui.painter().rect_filled(rect, radius_of(radius::POSTER + 4.0), with_alpha(Color32::WHITE, 0.05));
        let logo_rect = Rect::from_min_size(rect.min + vec2(16.0, 16.0), vec2(52.0, 52.0));
        ui_kit::paint_cover_url(ui, &cx.svc.images, logo.as_deref(), logo_rect, 12.0);
        let text_left = logo_rect.right() + 16.0;
        ui_kit::draw_text(ui, pos2(text_left, rect.top() + 28.0), Align2::LEFT_CENTER, &name, theme::bold(size::BODY_LG), if enabled { t.text } else { t.text_muted });
        let name_w = ui.painter().layout_no_wrap(name.clone(), theme::bold(size::BODY_LG), t.text).size().x;
        ui_kit::draw_text(ui, pos2(text_left + name_w + 10.0, rect.top() + 30.0), Align2::LEFT_CENTER, &version, theme::regular(size::LABEL_SM), t.text_muted);
        let mut line = Vec::new();
        match &status {
            Load::Failed(error) => line.push(format!("Not reachable: {error}")),
            Load::Loading => line.push("Loading…".to_owned()),
            Load::Ready(()) => {
                if catalogs > 0 {
                    line.push(format!("{catalogs} catalog{}", if catalogs == 1 { "" } else { "s" }));
                }
                if has_streams {
                    line.push("streams".to_owned());
                }
            }
        }
        let status_colour = if matches!(status, Load::Failed(_)) { t.danger } else { t.text_secondary };
        ui_kit::draw_text(ui, pos2(text_left, rect.top() + 50.0), Align2::LEFT_CENTER, &line.join(" · "), theme::regular(size::LABEL_SM + 0.5), status_colour);
        if !description.is_empty() {
            ui.painter().galley(pos2(text_left, rect.top() + 62.0), desc, t.text_muted);
        }
        // Controls on the right: up, down, switch, remove.
        let mut x = rect.right() - 20.0;
        let controls_y = rect.top() + 38.0;
        let remove_rect = Rect::from_center_size(pos2(x - 16.0, controls_y), vec2(36.0, 36.0));
        if icon_hit(ui, remove_rect, icons::DELETE, "Remove this addon", ("addon-remove", index)) {
            remove = Some(url.clone());
        }
        x -= 44.0;
        let switch_rect = Rect::from_center_size(pos2(x - 24.0, controls_y), vec2(48.0, 28.0));
        let response = ui.interact(switch_rect, egui::Id::unique(("addon-switch", index)), Sense::click());
        let on = animate(ui, response.id.with("on"), enabled, theme::motion::NORMAL);
        ui.painter().rect_filled(switch_rect, radius_of(14.0), ui_kit::lerp_color(with_alpha(Color32::WHITE, 0.16), t.accent, on));
        ui.painter().circle_filled(pos2(switch_rect.left() + 14.0 + on * 20.0, switch_rect.center().y), 11.0, ui_kit::lerp_color(Color32::from_gray(210), t.on_accent, on));
        if response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if response.clicked() {
            toggle = Some((url.clone(), !enabled));
        }
        x -= 64.0;
        if index + 1 < count && icon_hit(ui, Rect::from_center_size(pos2(x - 16.0, controls_y), vec2(36.0, 36.0)), icons::EXPAND_MORE, "Move down", ("addon-down", index)) {
            shift = Some((url.clone(), 1));
        }
        x -= 38.0;
        if index > 0 && icon_hit(ui, Rect::from_center_size(pos2(x - 16.0, controls_y), vec2(36.0, 36.0)), icons::EXPAND_LESS, "Move up", ("addon-up", index)) {
            shift = Some((url.clone(), -1));
        }
        ui.add_space(8.0);
    }
    if let Some(url) = remove {
        cx.state.remove_addon(cx.svc, &url);
    }
    if let Some((url, enabled)) = toggle {
        cx.state.set_addon_enabled(cx.svc, &url, enabled);
    }
    if let Some((url, delta)) = shift {
        cx.state.move_addon(cx.svc, &url, delta);
    }
}

fn icon_hit(ui: &mut Ui, rect: Rect, icon: char, tooltip: &str, id: (&str, usize)) -> bool {
    let t = theme::get();
    let response = ui.interact(rect, egui::Id::unique(id), Sense::click());
    let hover = animate(ui, response.id.with("hover"), response.hovered(), theme::motion::FAST);
    ui.painter().circle_filled(rect.center(), 18.0, with_alpha(Color32::WHITE, 0.10 * hover));
    ui_kit::draw_icon(ui, rect.center(), icon, 22.0, ui_kit::lerp_color(t.text_secondary, t.text, hover));
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    response.on_hover_text(tooltip).clicked()
}

fn playback(cx: &mut Cx, ui: &mut Ui) {
    heading(ui, "Playback");
    let mut changed = false;
    changed |= switch_row(ui, "Hardware decoding", Some("Decode video on the graphics card. Turn off if a stream shows artefacts or stutters."), &mut cx.state.settings.hardware_decoding);
    changed |= switch_row(ui, "Play the next episode automatically", Some("When an episode ends, open the next one."), &mut cx.state.settings.auto_play_next);
    changed |= switch_row(ui, "Subtitles on by default", Some("Turn on the preferred subtitle language when a stream has it."), &mut cx.state.settings.subtitles_on);
    ui.add_space(10.0);
    changed |= language_picker(ui, "Preferred audio language", &mut cx.state.settings.preferred_audio_language, "audio-lang");
    changed |= language_picker(ui, "Preferred subtitle language", &mut cx.state.settings.preferred_subtitle_language, "sub-lang");
    if changed {
        cx.state.save_settings();
    }
    note(ui, "Hardware decoding takes effect the next time a stream is opened.");
}

const LANGUAGES: [(&str, &str); 14] = [
    ("", "Original / none"),
    ("eng", "English"),
    ("spa", "Spanish"),
    ("fre", "French"),
    ("ger", "German"),
    ("ita", "Italian"),
    ("por", "Portuguese"),
    ("rus", "Russian"),
    ("jpn", "Japanese"),
    ("kor", "Korean"),
    ("chi", "Chinese"),
    ("ara", "Arabic"),
    ("hin", "Hindi"),
    ("tur", "Turkish"),
];

fn language_picker(ui: &mut Ui, title: &str, value: &mut String, id: &str) -> bool {
    let t = theme::get();
    let mut changed = false;
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(vec2(width, 50.0), Sense::click());
    let hover = animate(ui, response.id.with("hover"), response.hovered(), theme::motion::FAST);
    ui.painter().rect_filled(rect, radius_of(radius::POSTER), with_alpha(Color32::WHITE, 0.05 + 0.04 * hover));
    ui_kit::draw_text(ui, rect.left_center() + vec2(16.0, 0.0), Align2::LEFT_CENTER, title, theme::semibold(size::BODY_LG), t.text);
    let label = LANGUAGES.iter().find(|(code, _)| code == value).map(|(_, name)| *name).unwrap_or("Custom");
    ui_kit::draw_text(ui, rect.right_center() - vec2(44.0, 0.0), Align2::RIGHT_CENTER, label, theme::semibold(size::BODY_MD), t.accent);
    ui_kit::draw_icon(ui, rect.right_center() - vec2(24.0, 0.0), icons::EXPAND_MORE, 22.0, t.text_secondary);
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    Popup::menu(&response).id(egui::Id::unique(id)).gap(6.0).show(|ui| {
        ui.set_min_width(240.0);
        for (code, name) in LANGUAGES {
            let (row, r) = ui.allocate_exact_size(vec2(ui.available_width().max(240.0), 34.0), Sense::click());
            let h = animate(ui, r.id.with("h"), r.hovered(), theme::motion::FAST);
            ui.painter().rect_filled(row, radius_of(8.0), with_alpha(Color32::WHITE, 0.08 * h));
            let selected = value == code;
            ui_kit::draw_text(ui, row.left_center() + vec2(12.0, 0.0), Align2::LEFT_CENTER, name, theme::semibold(size::BODY_SM), if selected { t.accent } else { t.text });
            if r.clicked() {
                *value = code.to_owned();
                changed = true;
            }
        }
    });
    ui.add_space(8.0);
    changed
}

fn appearance(cx: &mut Cx, ui: &mut Ui) {
    let t = theme::get();
    heading(ui, "Appearance");
    let mut changed = false;
    note(ui, "Colour theme");
    ui.add_space(4.0);
    let swatch = 64.0;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(14.0, 14.0);
        for palette in PALETTES {
            let (rect, response) = ui.allocate_exact_size(vec2(swatch + 28.0, swatch + 28.0), Sense::click());
            let selected = cx.state.settings.theme == palette.id;
            let hover = animate(ui, response.id.with("hover"), response.hovered(), theme::motion::FAST);
            let on = animate(ui, response.id.with("on"), selected, theme::motion::NORMAL);
            let circle = Rect::from_center_size(rect.center_top() + vec2(0.0, swatch / 2.0 + 4.0), vec2(swatch - 8.0 + 4.0 * hover, swatch - 8.0 + 4.0 * hover));
            ui.painter().circle_filled(circle.center(), circle.width() / 2.0, palette.secondary);
            ui.painter().circle_stroke(circle.center(), circle.width() / 2.0 + 5.0, egui::Stroke::new(2.0, with_alpha(t.text, on)));
            ui_kit::draw_text(ui, pos2(rect.center().x, rect.bottom() - 6.0), Align2::CENTER_CENTER, palette.name, theme::regular(size::LABEL_XS), if selected { t.text } else { t.text_muted });
            if response.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            if response.clicked() {
                cx.state.settings.theme = palette.id.to_owned();
                changed = true;
            }
        }
    });
    ui.add_space(14.0);
    ui.add_space(4.0);
    note(ui, "Poster shape on Home");
    ui.horizontal(|ui| {
        for (value, label) in [("auto", "Automatic"), ("portrait", "Portrait"), ("landscape", "Landscape")] {
            if ui_kit::chip(ui, label, cx.state.settings.poster_style == value).clicked() {
                cx.state.settings.poster_style = value.to_owned();
                changed = true;
            }
        }
    });
    ui.add_space(8.0);
    changed |= switch_row(ui, "Pure black background", Some("Use true black instead of the theme's dark grey."), &mut cx.state.settings.amoled);
    changed |= switch_row(ui, "Featured banner on Home", Some("Show the large rotating banner above the shelves."), &mut cx.state.settings.hero);
    let mut show_type = cx.state.settings.show_catalog_type;
    if switch_row(ui, "Show the content type in shelf titles", Some("“Popular - Movies” instead of “Popular”."), &mut show_type) {
        cx.state.settings.show_catalog_type = show_type;
        changed = true;
        cx.state.rebuild_sections(cx.svc);
    }
    if changed {
        theme::apply(ui.ctx(), &theme::Theme::new(theme::palette_by_id(&cx.state.settings.theme), cx.state.settings.amoled));
        cx.state.save_settings();
    }
}

fn storage(cx: &mut Cx, ui: &mut Ui) {
    let t = theme::get();
    heading(ui, "Storage");
    let images = nuvio_core::cache::DiskCache::new(paths::cache_dir().join("images"));
    let json = nuvio_core::cache::DiskCache::new(paths::cache_dir().join("json"));
    let mb = |bytes: u64| format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0));
    let _ = &cx;
    ui.label(egui::RichText::new(format!("Images: {}   ·   Addon responses: {}", mb(images.size_bytes()), mb(json.size_bytes()))).color(t.text_secondary).size(size::BODY_MD));
    ui.add_space(10.0);
    ui.horizontal(|ui| {
        if ui_kit::pill_button(ui, Some(icons::DELETE), "Clear caches", PillStyle::Soft).clicked() {
            images.remove_all();
            json.remove_all();
        }
        if ui_kit::pill_button(ui, Some(icons::FOLDER), "Open data folder", PillStyle::Soft).clicked() {
            let _ = std::process::Command::new("explorer").arg(paths::roaming_data_dir()).spawn();
        }
    });
    ui.add_space(14.0);
    note(ui, &format!("Settings, library and watch progress: {}", paths::roaming_data_dir().display()));
    note(ui, &format!("Caches and logs: {}", paths::local_data_dir().display()));
}

fn about(ui: &mut Ui) {
    heading(ui, "About");
    note(ui, &format!("Nuvio for Windows (Rust) {}", env!("CARGO_PKG_VERSION")));
    note(ui, "A native port of the Nuvio desktop app. Based on Nuvio by NuvioMedia, licensed under GPL-3.0.");
    note(ui, "Playback by libmpv. Interface drawn with egui. Icons: Material Icons (Apache-2.0). Font: JetBrains Sans (OFL).");
}
