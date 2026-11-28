//! A title's page: backdrop, facts, the play button, episodes of a series, cast and credits.

use eframe::egui::{self, Align2, Color32, CornerRadius, Rect, ScrollArea, Sense, Ui, pos2, vec2};
use nuvio_core::meta::{MetaDetails, Video};
use nuvio_core::store::{NextEpisode, Progress, unix_seconds};

use super::{Action, Cx};
use crate::icons;
use crate::images::Image;
use crate::state::Load;
use crate::theme::{self, radius, size};
use crate::ui_kit::{self, PillStyle, animate, page_padding, radius_of, with_alpha};

pub fn show(cx: &mut Cx, ui: &mut Ui) {
    let width = ui.available_width();
    let pad = page_padding(width);
    let Some(details) = cx.state.details.as_ref() else { return };
    let id = details.id.clone();
    let preview = details.preview.clone();
    let season = details.season;
    let meta_state = details.meta.clone();

    match meta_state {
        Load::Loading => {
            let rect = ui.available_rect_before_wrap();
            if let Some(preview) = &preview {
                backdrop(cx, ui, preview.background.as_deref().or(preview.poster.as_deref()), rect.width(), 420.0);
            }
            let t = theme::get();
            ui_kit::spinner(ui, rect.center(), 18.0, t.accent);
            back_button(cx, ui, rect.left_top() + vec2(20.0, 20.0));
        }
        Load::Failed(error) => {
            let t = theme::get();
            let rect = ui.available_rect_before_wrap();
            ui_kit::draw_text(ui, rect.center() - vec2(0.0, 10.0), Align2::CENTER_CENTER, "This title could not be loaded.", theme::semibold(size::TITLE_SM), t.text);
            ui_kit::draw_text(ui, rect.center() + vec2(0.0, 18.0), Align2::CENTER_CENTER, &error, theme::regular(size::BODY_SM), t.text_muted);
            back_button(cx, ui, rect.left_top() + vec2(20.0, 20.0));
        }
        Load::Ready(meta) => {
            ScrollArea::vertical().id_salt(("details-scroll", &id)).auto_shrink([false, false]).show(ui, |ui| {
                ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                header(cx, ui, &meta, pad);
                ui.add_space(28.0);
                if meta.is_series() && !meta.videos.is_empty() {
                    episodes(cx, ui, &meta, season, pad);
                    ui.add_space(30.0);
                }
                if !meta.cast.is_empty() {
                    cast(cx, ui, &meta, pad);
                    ui.add_space(30.0);
                }
                credits(ui, &meta, pad);
                ui.add_space(80.0);
            });
        }
    }
}

fn backdrop(cx: &Cx, ui: &mut Ui, url: Option<&str>, width: f32, height: f32) -> Rect {
    let t = theme::get();
    let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    ui_kit::paint_cover_url_with(ui, &cx.svc.images, url, rect, CornerRadius::ZERO);
    ui_kit::horizontal_gradient(ui, Rect::from_min_max(rect.min, pos2(rect.left() + rect.width() * 0.6, rect.bottom())), Color32::from_black_alpha(200), Color32::from_black_alpha(0));
    ui_kit::vertical_gradient(ui, Rect::from_min_max(pos2(rect.left(), rect.bottom() - height * 0.5), rect.max), Color32::from_black_alpha(0), t.background);
    rect
}

fn header(cx: &mut Cx, ui: &mut Ui, meta: &MetaDetails, pad: f32) {
    let t = theme::get();
    let width = ui.available_width();
    let height = (width * 0.46).clamp(440.0, 620.0);
    let rect = backdrop(cx, ui, meta.background.as_deref().or(meta.poster.as_deref()), width, height);
    let text_w = (width * 0.5).min(660.0);
    let left = rect.left() + pad;
    let mut y = rect.bottom() - 36.0;

    // Buttons, from the bottom up.
    let next = play_target(cx, meta);
    let in_library = cx.state.library.contains(&meta.kind, &meta.id);
    let buttons = Rect::from_min_size(pos2(left, y - 44.0), vec2(text_w + 200.0, 44.0));
    let mut row = ui.new_child(egui::UiBuilder::new().max_rect(buttons).layout(egui::Layout::left_to_right(egui::Align::Center)));
    row.spacing_mut().item_spacing.x = 10.0;
    if ui_kit::pill_button(&mut row, Some(icons::PLAY), &next.label, PillStyle::Primary).clicked() {
        cx.actions.push(Action::OpenStreams { video: next.video.clone() });
    }
    let library_label = if in_library { "In library" } else { "Add to library" };
    let library_icon = if in_library { icons::BOOKMARK } else { icons::BOOKMARK_BORDER };
    if ui_kit::pill_button(&mut row, Some(library_icon), library_label, PillStyle::Soft).clicked() {
        cx.actions.push(Action::ToggleLibrary(meta.as_preview()));
    }
    if let Some(trailer) = meta.trailers.first() {
        if ui_kit::pill_button(&mut row, Some(icons::OPEN_IN_NEW), "Trailer", PillStyle::Soft).clicked() {
            cx.actions.push(Action::OpenExternal(format!("https://www.youtube.com/watch?v={}", trailer.key)));
        }
    }
    y -= 44.0 + 18.0;

    if let Some(description) = meta.description.as_deref() {
        let g = ui_kit::galley(ui, description, theme::regular(size::BODY_APP), with_alpha(t.text, 0.88), text_w, 4);
        y -= g.size().y;
        ui.painter().galley(pos2(left, y), g, t.text);
        y -= 14.0;
    }

    // Facts: year · runtime · age rating · rating.
    let mut facts: Vec<String> = Vec::new();
    if let Some(year) = meta.release_info.as_deref() {
        facts.push(year.to_owned());
    }
    if let Some(runtime) = meta.runtime.as_deref() {
        facts.push(runtime.to_owned());
    }
    if let Some(age) = meta.age_rating.as_deref() {
        facts.push(age.to_owned());
    }
    if let Some(status) = meta.status.as_deref().filter(|_| meta.is_series()) {
        facts.push(status.to_owned());
    }
    if !facts.is_empty() || meta.imdb_rating.is_some() {
        y -= 20.0;
        let mut x = left;
        if let Some(rating) = meta.imdb_rating.as_deref() {
            ui_kit::draw_icon(ui, pos2(x + 9.0, y + 10.0), icons::STAR, 20.0, t.warning);
            let r = ui_kit::draw_text(ui, pos2(x + 22.0, y + 10.0), Align2::LEFT_CENTER, rating, theme::bold(size::BODY_MD), t.warning);
            x = r.right() + 16.0;
        }
        ui_kit::draw_text(ui, pos2(x, y + 10.0), Align2::LEFT_CENTER, &facts.join("   ·   "), theme::semibold(size::BODY_SM), t.text_secondary);
        y -= 10.0;
    }
    if !meta.genres.is_empty() {
        y -= 20.0;
        ui_kit::draw_text(ui, pos2(left, y + 10.0), Align2::LEFT_CENTER, &meta.genres.iter().take(5).cloned().collect::<Vec<_>>().join("  ·  "), theme::regular(size::BODY_SM), t.text_muted);
        y -= 14.0;
    }

    // Title: the wordmark if there is one.
    let logo = meta.logo.as_deref().map(|url| cx.svc.images.get(url, 640.0 * ui.ctx().pixels_per_point()));
    match logo {
        Some(Image::Ready { texture, age }) => {
            let tex = texture.size_vec2();
            let scale = (text_w / tex.x).min(130.0 / tex.y);
            let logo_size = tex * scale;
            let alpha = (age.as_secs_f32() / 0.3).min(1.0);
            let logo_rect = Rect::from_min_size(pos2(left, y - logo_size.y - 8.0), logo_size);
            egui::Image::new((texture.id(), logo_size)).tint(Color32::from_white_alpha((alpha * 255.0) as u8)).paint_at(ui, logo_rect);
        }
        _ => {
            let g = ui_kit::galley(ui, &meta.name, theme::bold(size::PAGE_DISPLAY + 8.0), Color32::WHITE, text_w, 2);
            y -= g.size().y + 6.0;
            ui.painter().galley(pos2(left, y), g, Color32::WHITE);
        }
    }

    back_button(cx, ui, rect.left_top() + vec2(20.0, 20.0));
}

pub fn back_button(cx: &mut Cx, ui: &mut Ui, pos: egui::Pos2) {
    let rect = Rect::from_min_size(pos, vec2(46.0, 46.0));
    let response = ui.interact(rect, egui::Id::unique("back-button"), Sense::click());
    let hover = animate(ui, response.id.with("hover"), response.hovered(), theme::motion::FAST);
    ui.painter().circle_filled(rect.center(), 23.0, Color32::from_black_alpha((120.0 + 60.0 * hover) as u8));
    ui.painter().circle_stroke(rect.center(), 23.0, egui::Stroke::new(1.0, with_alpha(Color32::WHITE, 0.16 + 0.1 * hover)));
    ui_kit::draw_icon(ui, rect.center(), icons::ARROW_BACK, 24.0, Color32::WHITE);
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    if response.on_hover_text("Back").clicked() {
        cx.actions.push(Action::Back);
    }
}

struct PlayTarget {
    label: String,
    video: Option<Video>,
}

/// What the main button plays: a film, or the episode the viewer is at (in progress, else the one after the last
/// finished, else the first).
fn play_target(cx: &Cx, meta: &MetaDetails) -> PlayTarget {
    if !meta.is_series() || meta.videos.is_empty() {
        return match cx.state.progress.get(&meta.id).filter(|p| p.is_resumable()) {
            Some(p) => PlayTarget { label: format!("Resume · {}m left", p.remaining_minutes()), video: None },
            None => PlayTarget { label: "Play".into(), video: None },
        };
    }
    let ordered = watch_order(meta);
    let resumable = ordered.iter().rev().find(|v| cx.state.progress.get(&v.id).is_some_and(|p| p.is_resumable()));
    if let Some(video) = resumable {
        return PlayTarget { label: format!("Resume {}", video.code().unwrap_or_default()), video: Some((*video).clone()) };
    }
    let last_finished = ordered.iter().rposition(|v| cx.state.progress.is_watched(&v.id));
    let next = match last_finished {
        Some(index) => ordered.get(index + 1).or(ordered.first()),
        None => ordered.first(),
    };
    match next {
        Some(video) => PlayTarget { label: format!("Play {}", video.code().unwrap_or_default()), video: Some((*video).clone()) },
        None => PlayTarget { label: "Play".into(), video: None },
    }
}

fn episodes(cx: &mut Cx, ui: &mut Ui, meta: &MetaDetails, season: Option<i64>, pad: f32) {
    let t = theme::get();
    let seasons = meta.seasons();
    let Some(current) = season.or_else(|| seasons.first().copied()) else { return };
    ui.horizontal(|ui| {
        ui.add_space(pad);
        ui_kit::section_header(ui, "Episodes", None);
    });
    ui.add_space(8.0);
    if seasons.len() > 1 {
        ui_kit::paging_row(ui, egui::Id::unique("season-scroll"), pad, 16.0, |ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            for s in &seasons {
                let label = if *s == 0 { "Specials".to_owned() } else { format!("Season {s}") };
                if ui_kit::chip(ui, &label, *s == current).clicked() {
                    if let Some(details) = cx.state.details.as_mut() {
                        details.season = Some(*s);
                    }
                }
            }
        });
        ui.add_space(14.0);
    }

    let width = ui.available_width();
    let row_w = (width - pad * 2.0).min(1100.0);
    let thumb_w = 228.0_f32.min(row_w * 0.28);
    let thumb_h = thumb_w / 1.77;
    for video in meta.episodes_of(current) {
        ui.horizontal(|ui| {
            ui.add_space(pad);
            let (rect, response) = ui.allocate_exact_size(vec2(row_w, thumb_h + 24.0), Sense::click());
            let hover = animate(ui, response.id.with("hover"), response.hovered(), theme::motion::FAST);
            ui.painter().rect_filled(rect, radius_of(radius::POSTER + 4.0), with_alpha(Color32::WHITE, 0.035 + 0.05 * hover));
            let thumb = Rect::from_min_size(rect.min + vec2(12.0, 12.0), vec2(thumb_w, thumb_h));
            ui_kit::paint_cover_url(ui, &cx.svc.images, video.thumbnail.as_deref().or(meta.background.as_deref()), thumb, radius::POSTER - 2.0);
            let progress = cx.state.progress.get(&video.id);
            if let Some(p) = progress.filter(|p| p.fraction() > 0.01) {
                let bar = Rect::from_min_size(pos2(thumb.left() + 8.0, thumb.bottom() - 10.0), vec2(thumb.width() - 16.0, 4.0));
                ui.painter().rect_filled(bar, radius_of(2.0), with_alpha(Color32::WHITE, 0.25));
                ui.painter().rect_filled(Rect::from_min_size(bar.min, vec2(bar.width() * p.fraction() as f32, 4.0)), radius_of(2.0), t.accent);
            }
            // Play glyph on hover.
            if hover > 0.0 {
                ui.painter().circle_filled(thumb.center(), 24.0, Color32::from_black_alpha((150.0 * hover) as u8));
                ui_kit::draw_icon(ui, thumb.center(), icons::PLAY, 30.0, with_alpha(Color32::WHITE, hover));
            }
            let text_left = thumb.right() + 18.0;
            let text_w = rect.right() - text_left - 16.0;
            let number = video.episode.map(|e| format!("{e}.")).unwrap_or_default();
            let title = format!("{number} {}", video.title);
            let mut meta_line: Vec<String> = Vec::new();
            if let Some(date) = video.released.as_deref().and_then(short_date) {
                meta_line.push(date);
            }
            if let Some(minutes) = video.runtime_minutes {
                meta_line.push(format!("{minutes} min"));
            }
            if let Some(rating) = video.rating {
                meta_line.push(format!("{rating:.1}/10"));
            }
            let facts = ui_kit::draw_text(ui, pos2(rect.right() - 16.0, rect.top() + 28.0), Align2::RIGHT_CENTER, &meta_line.join("  ·  "), theme::regular(size::LABEL_SM), t.text_muted);
            // The title gets what the date, runtime and rating leave of the line.
            let title_w = (facts.left() - text_left - 16.0).max(60.0);
            let g = ui_kit::galley(ui, title.trim(), theme::semibold(size::BODY_LG), t.text, title_w, 1);
            ui.painter().galley(pos2(text_left, rect.top() + 16.0), g, t.text);
            if let Some(overview) = video.overview.as_deref() {
                let rows = if thumb_h > 110.0 { 3 } else { 2 };
                let g = ui_kit::galley(ui, overview, theme::regular(size::BODY_SM), t.text_secondary, text_w, rows);
                ui.painter().galley(pos2(text_left, rect.top() + 46.0), g, t.text_secondary);
            }
            if cx.state.progress.is_watched(&video.id) {
                ui_kit::draw_icon(ui, pos2(thumb.right() - 18.0, thumb.top() + 18.0), icons::CHECK, 20.0, Color32::WHITE);
            }
            if response.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            if response.clicked() {
                cx.actions.push(Action::OpenStreams { video: Some(video.clone()) });
            }
            ui_kit::glass_popup(egui::Popup::context_menu(&response), |ui| {
                ui.set_min_width(250.0);
                episode_menu(cx, ui, meta, video);
            });
        });
        ui.add_space(8.0);
    }
}

/// The episodes of a series in the order they are watched: real seasons first to last, the specials left out.
fn watch_order(meta: &MetaDetails) -> Vec<&Video> {
    meta.seasons().into_iter().filter(|s| *s > 0).flat_map(|s| meta.episodes_of(s)).collect()
}

/// What a right click on an episode offers.
fn episode_menu(cx: &mut Cx, ui: &mut Ui, meta: &MetaDetails, video: &Video) {
    let order = watch_order(meta);
    let position = order.iter().position(|v| v.id == video.id);
    let watched = cx.state.progress.is_watched(&video.id);
    let unwatched_before: Vec<&Video> = position.map(|p| order[..p].iter().copied().filter(|v| !cx.state.progress.is_watched(&v.id)).collect()).unwrap_or_default();
    let title = format!("{}{}", video.code().map(|c| format!("{c}  ")).unwrap_or_default(), video.title);
    let heading = ui_kit::galley(ui, &title, theme::bold(size::BODY_SM), theme::get().text_muted, 250.0, 1);
    let (row, _) = ui.allocate_exact_size(vec2(ui.available_width().max(250.0), heading.size().y + 12.0), Sense::hover());
    ui.painter().galley(row.min + vec2(14.0, 4.0), heading, theme::get().text_muted);

    if ui_kit::menu_item(ui, Some(icons::PLAY), "Play", false).clicked() {
        cx.actions.push(Action::OpenStreams { video: Some(video.clone()) });
    }
    if watched {
        if ui_kit::menu_item(ui, Some(icons::VISIBILITY_OFF), "Mark as not watched", false).clicked() {
            cx.state.progress.remove(&video.id);
            cx.state.save_progress();
        }
    } else if ui_kit::menu_item(ui, Some(icons::DONE_ALL), "Mark as watched", false).clicked() {
        mark_watched(cx, meta, &order, &[video]);
    }
    if !unwatched_before.is_empty() && ui_kit::menu_item(ui, Some(icons::TASK_ALT), "Mark the episodes before it as watched", false).clicked() {
        mark_watched(cx, meta, &order, &unwatched_before);
    }
}

/// Records `videos` as watched to the end. Their times are a second apart in the order given, so the last is the newest
/// and Continue Watching offers the episode after it.
fn mark_watched(cx: &mut Cx, meta: &MetaDetails, order: &[&Video], videos: &[&Video]) {
    let now = unix_seconds();
    for (i, video) in videos.iter().enumerate() {
        let duration = video.runtime_minutes.map(|m| m as f64 * 60.0).filter(|d| *d > 0.0).unwrap_or(45.0 * 60.0);
        let next = order.iter().position(|v| v.id == video.id).and_then(|p| order.get(p + 1)).map(|v| NextEpisode {
            video_id: v.id.clone(),
            season: v.season,
            episode: v.episode,
            title: Some(v.title.clone()).filter(|t| !t.is_empty()),
        });
        cx.state.progress.record(Progress {
            kind: meta.kind.clone(),
            id: meta.id.clone(),
            video_id: video.id.clone(),
            season: video.season,
            episode: video.episode,
            episode_title: Some(video.title.clone()).filter(|t| !t.is_empty()),
            name: meta.name.clone(),
            poster: meta.poster.clone(),
            background: meta.background.clone(),
            position: duration,
            duration,
            updated_at: now.saturating_sub((videos.len() - 1 - i) as u64),
            next,
        });
    }
    cx.state.save_progress();
}

/// `2024-03-09T00:00:00.000Z` as `Mar 9, 2024`.
fn short_date(raw: &str) -> Option<String> {
    let date = raw.get(..10)?;
    let mut parts = date.split('-');
    let (year, month, day): (i32, usize, u32) = (parts.next()?.parse().ok()?, parts.next()?.parse().ok()?, parts.next()?.parse().ok()?);
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    Some(format!("{} {day}, {year}", MONTHS.get(month.checked_sub(1)?)?))
}

fn cast(cx: &mut Cx, ui: &mut Ui, meta: &MetaDetails, pad: f32) {
    let t = theme::get();
    ui.horizontal(|ui| {
        ui.add_space(pad);
        ui_kit::section_header(ui, "Cast", None);
    });
    ui.add_space(10.0);
    ui_kit::paging_row(ui, egui::Id::unique("cast-scroll"), pad, 48.0, |ui| {
        ui.spacing_mut().item_spacing.x = 18.0;
        for person in meta.cast.iter().take(24) {
            let (rect, _) = ui.allocate_exact_size(vec2(104.0, 150.0), Sense::hover());
            let photo = Rect::from_center_size(pos2(rect.center().x, rect.top() + 48.0), vec2(96.0, 96.0));
            if person.photo.is_some() {
                ui_kit::paint_cover_url_with(ui, &cx.svc.images, person.photo.as_deref(), photo, CornerRadius::same(255));
            } else {
                ui.painter().circle_filled(photo.center(), 48.0, t.card);
                let initials: String = person.name.split_whitespace().filter_map(|w| w.chars().next()).take(2).collect();
                ui_kit::draw_text(ui, photo.center(), Align2::CENTER_CENTER, &initials, theme::bold(size::TITLE_MD), t.text_secondary);
            }
            let name = ui_kit::galley(ui, &person.name, theme::semibold(size::LABEL_SM + 1.0), t.text, rect.width(), 2);
            ui.painter().galley(pos2(rect.center().x - name.size().x / 2.0, photo.bottom() + 8.0), name.clone(), t.text);
            if let Some(role) = person.role.as_deref() {
                let role = ui_kit::galley(ui, role, theme::regular(size::LABEL_XS), t.text_muted, rect.width(), 1);
                ui.painter().galley(pos2(rect.center().x - role.size().x / 2.0, photo.bottom() + 12.0 + name.size().y), role, t.text_muted);
            }
        }
    });
}

fn credits(ui: &mut Ui, meta: &MetaDetails, pad: f32) {
    let t = theme::get();
    let mut rows: Vec<(&str, String)> = Vec::new();
    if !meta.directors.is_empty() {
        rows.push(("Director", meta.directors.join(", ")));
    }
    if !meta.writers.is_empty() {
        rows.push(("Writer", meta.writers.join(", ")));
    }
    if let Some(country) = &meta.country {
        rows.push(("Country", country.clone()));
    }
    if let Some(language) = &meta.language {
        rows.push(("Language", language.clone()));
    }
    if let Some(awards) = &meta.awards {
        rows.push(("Awards", awards.clone()));
    }
    if rows.is_empty() {
        return;
    }
    ui.horizontal(|ui| {
        ui.add_space(pad);
        ui_kit::section_header(ui, "Details", None);
    });
    ui.add_space(8.0);
    let width = (ui.available_width() - pad * 2.0).min(900.0);
    for (label, value) in rows {
        ui.horizontal(|ui| {
            ui.add_space(pad);
            let g_label = ui_kit::galley(ui, label, theme::semibold(size::BODY_SM), t.text_muted, 140.0, 1);
            let g_value = ui_kit::galley(ui, &value, theme::regular(size::BODY_MD), t.text, width - 160.0, 3);
            let (row, _) = ui.allocate_exact_size(vec2(width, g_value.size().y + 12.0), Sense::hover());
            ui.painter().galley(row.min + vec2(0.0, 2.0), g_label, t.text_muted);
            ui.painter().galley(row.min + vec2(160.0, 0.0), g_value, t.text);
        });
    }
}
