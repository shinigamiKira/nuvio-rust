//! The small custom-painted pieces every screen is built from: text that truncates, icon buttons with hover feedback,
//! pills, chips, cover-cropped images and the poster cards.

use std::sync::Arc;

use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontId, Galley, Id, Pos2, Rect, Response, Sense, Shape, Stroke, StrokeKind, Ui, Vec2,
    emath::easing, epaint::text::LayoutJob, pos2, vec2,
};
use nuvio_core::meta::{MetaPreview, PosterShape};

use crate::images::{Image, Images};
use crate::theme::{self, radius, size, space};

pub const HOVER_SECS: f32 = theme::motion::FAST;

/// `0..1` that follows `target` over `secs`, easing out. Drives every hover and press effect.
pub fn animate(ui: &Ui, id: Id, target: bool, secs: f32) -> f32 {
    ui.ctx().animate_bool_with_time_and_easing(id, target, secs, easing::cubic_out)
}

pub fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    a.lerp_to_gamma(b, t.clamp(0.0, 1.0))
}

pub fn with_alpha(color: Color32, alpha: f32) -> Color32 {
    color.gamma_multiply(alpha.clamp(0.0, 1.0))
}

pub fn radius_of(value: f32) -> CornerRadius {
    CornerRadius::same(value.min(255.0).round() as u8)
}

/// Lays out `text` in at most `rows` rows of `width`, cutting the last one with an ellipsis.
pub fn galley(ui: &Ui, text: &str, font: FontId, color: Color32, width: f32, rows: usize) -> Arc<Galley> {
    galley_ex(ui, text, font, color, width, rows, rows == 1)
}

/// `break_anywhere` lets a word with no spaces (a release file name) wrap at the edge instead of jumping to the next row.
pub fn galley_ex(ui: &Ui, text: &str, font: FontId, color: Color32, width: f32, rows: usize, break_anywhere: bool) -> Arc<Galley> {
    let mut job = LayoutJob::simple(text.to_owned(), font, color, width.max(1.0));
    job.wrap.max_rows = rows;
    job.wrap.break_anywhere = break_anywhere;
    job.wrap.overflow_character = Some('…');
    ui.painter().layout_job(job)
}

pub fn draw_text(ui: &Ui, pos: Pos2, anchor: Align2, text: &str, font: FontId, color: Color32) -> Rect {
    ui.painter().text(pos, anchor, text, font, color)
}

pub fn draw_icon(ui: &Ui, center: Pos2, icon: char, px: f32, color: Color32) {
    ui.painter().text(center, Align2::CENTER_CENTER, icon.to_string(), theme::icon_font(px), color);
}

/// A round icon button that lights up under the pointer. `tooltip` says what it does.
pub fn icon_button(ui: &mut Ui, icon: char, px: f32, tooltip: &str) -> Response {
    let t = theme::get();
    let hit = px + 16.0;
    let (rect, response) = ui.allocate_exact_size(vec2(hit, hit), Sense::click());
    let hover = animate(ui, response.id.with("hover"), response.hovered(), HOVER_SECS);
    if hover > 0.0 {
        ui.painter().circle_filled(rect.center(), hit / 2.0, with_alpha(Color32::WHITE, 0.12 * hover));
    }
    draw_icon(ui, rect.center(), icon, px, lerp_color(t.text_secondary, t.text, hover));
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    response.on_hover_text(tooltip)
}

#[derive(Clone, Copy, PartialEq)]
pub enum PillStyle {
    /// The accent colour, for the one main action of a screen.
    Primary,
    /// A translucent fill for secondary actions.
    Soft,
}

/// A rounded button with an optional leading icon.
pub fn pill_button(ui: &mut Ui, icon: Option<char>, label: &str, style: PillStyle) -> Response {
    let t = theme::get();
    let font = theme::semibold(size::BODY_MD);
    let text_galley = galley(ui, label, font, Color32::WHITE, 400.0, 1);
    let icon_w = if icon.is_some() { 26.0 } else { 0.0 };
    let pad_x = 22.0;
    let desired = vec2(text_galley.size().x + icon_w + pad_x * 2.0, 42.0);
    let (rect, response) = ui.allocate_exact_size(desired, Sense::click());
    let hover = animate(ui, response.id.with("hover"), response.hovered(), HOVER_SECS);
    let press = animate(ui, response.id.with("press"), response.is_pointer_button_down_on(), 0.08);
    let (fill, stroke, fg) = match style {
        PillStyle::Primary => (lerp_color(t.accent, Color32::WHITE, 0.18 * hover), Color32::TRANSPARENT, t.on_accent),
        PillStyle::Soft => (with_alpha(Color32::WHITE, 0.10 + 0.08 * hover), Color32::TRANSPARENT, t.text),
    };
    let rect = rect.shrink(press * 1.5);
    ui.painter().rect_filled(rect, radius_of(radius::CHIP), fill);
    if stroke != Color32::TRANSPARENT {
        ui.painter().rect_stroke(rect, radius_of(radius::CHIP), Stroke::new(1.0, stroke), StrokeKind::Inside);
    }
    let mut x = rect.left() + pad_x;
    if let Some(icon) = icon {
        draw_icon(ui, pos2(x + 10.0, rect.center().y), icon, 22.0, fg);
        x += icon_w;
    }
    let galley = galley(ui, label, theme::semibold(size::BODY_MD), fg, 400.0, 1);
    ui.painter().galley(pos2(x, rect.center().y - galley.size().y / 2.0), galley, fg);
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    response
}

/// A filter or tab chip. Returns the response; the caller flips its own state on click.
pub fn chip(ui: &mut Ui, label: &str, selected: bool) -> Response {
    let t = theme::get();
    let text = galley(ui, label, theme::semibold(size::BODY_SM), Color32::WHITE, 320.0, 1);
    let desired = vec2(text.size().x + 28.0, 32.0);
    let (rect, response) = ui.allocate_exact_size(desired, Sense::click());
    let hover = animate(ui, response.id.with("hover"), response.hovered(), HOVER_SECS);
    let on = animate(ui, response.id.with("on"), selected, theme::motion::NORMAL);
    let fill = lerp_color(with_alpha(Color32::WHITE, 0.07 + 0.06 * hover), t.accent, on);
    let fg = lerp_color(t.text_secondary.gamma_multiply(1.0), t.on_accent, on);
    ui.painter().rect_filled(rect, radius_of(radius::CHIP), fill);
    ui.painter().galley_with_override_text_color(pos2(rect.left() + 14.0, rect.center().y - text.size().y / 2.0), text, fg);
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    response
}

/// Paints `image` over `rect`, cropped to fill it, fading in when it arrives. While it loads a soft placeholder shows.
pub fn paint_cover(ui: &Ui, rect: Rect, image: &Image, corner: f32) {
    paint_cover_with(ui, rect, image, radius_of(corner));
}

/// Like `paint_cover`, with a radius per corner (the home hero rounds only its bottom edge).
pub fn paint_cover_with(ui: &Ui, rect: Rect, image: &Image, corner: CornerRadius) {
    paint_cover_alpha(ui, rect, image, corner, 1.0);
}

/// `alpha` fades the whole picture (a crossfade between two backdrops), on top of the fade-in of a fresh image.
pub fn paint_cover_alpha(ui: &Ui, rect: Rect, image: &Image, corner: CornerRadius, alpha: f32) {
    let t = theme::get();
    match image {
        Image::Ready { texture, age } => {
            let tex = texture.size_vec2();
            let (aspect, target) = (tex.x / tex.y, rect.width() / rect.height());
            let uv = if aspect > target {
                let w = target / aspect;
                Rect::from_min_max(pos2((1.0 - w) / 2.0, 0.0), pos2((1.0 + w) / 2.0, 1.0))
            } else {
                let h = aspect / target;
                Rect::from_min_max(pos2(0.0, (1.0 - h) / 2.0), pos2(1.0, (1.0 + h) / 2.0))
            };
            // The placeholder stays underneath so the corners are right while the image is still transparent.
            let fade = (age.as_secs_f32() / 0.28).min(1.0) * alpha;
            if fade < 1.0 && alpha >= 1.0 {
                ui.painter().rect_filled(rect, corner, t.card);
            }
            if fade < 1.0 {
                ui.ctx().request_repaint();
            }
            egui::Image::new((texture.id(), rect.size()))
                .corner_radius(corner)
                .uv(uv)
                .tint(Color32::from_white_alpha((fade * 255.0) as u8))
                .paint_at(ui, rect);
        }
        Image::Loading => {
            let pulse = 0.5 + 0.5 * (ui.input(|i| i.time) as f32 * 3.0).sin();
            ui.painter().rect_filled(rect, corner, lerp_color(t.card, t.surface, pulse * 0.6));
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(33));
        }
        Image::Failed => {
            ui.painter().rect_filled(rect, corner, t.card);
        }
    }
}

pub fn paint_cover_url(ui: &Ui, images: &Images, url: Option<&str>, rect: Rect, corner: f32) {
    paint_cover_url_with(ui, images, url, rect, radius_of(corner));
}

pub fn paint_cover_url_with(ui: &Ui, images: &Images, url: Option<&str>, rect: Rect, corner: CornerRadius) {
    match url {
        Some(url) => {
            let px = rect.width() * ui.ctx().pixels_per_point();
            paint_cover_with(ui, rect, &images.get(url, px), corner);
        }
        None => {
            let t = theme::get();
            ui.painter().rect_filled(rect, corner, t.card);
        }
    }
}

/// Vertical gradient (top alpha to bottom alpha) of `color` over `rect`, for text legibility on artwork.
pub fn vertical_gradient(ui: &Ui, rect: Rect, top: Color32, bottom: Color32) {
    let mut mesh = egui::Mesh::default();
    let uv = pos2(0.0, 0.0);
    let white = egui::TextureId::default();
    mesh.texture_id = white;
    let base = mesh.vertices.len() as u32;
    for (pos, color) in [(rect.left_top(), top), (rect.right_top(), top), (rect.right_bottom(), bottom), (rect.left_bottom(), bottom)] {
        mesh.vertices.push(egui::epaint::Vertex { pos, uv, color });
    }
    mesh.indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    ui.painter().add(Shape::mesh(mesh));
}

/// Horizontal gradient (left alpha to right alpha).
pub fn horizontal_gradient(ui: &Ui, rect: Rect, left: Color32, right: Color32) {
    let mut mesh = egui::Mesh::default();
    let uv = pos2(0.0, 0.0);
    let base = mesh.vertices.len() as u32;
    for (pos, color) in [(rect.left_top(), left), (rect.right_top(), right), (rect.right_bottom(), right), (rect.left_bottom(), left)] {
        mesh.vertices.push(egui::epaint::Vertex { pos, uv, color });
    }
    mesh.indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    ui.painter().add(Shape::mesh(mesh));
}

/// A frosted panel: a dark translucent fill, a hairline that catches light along the top edge, and a soft shadow.
pub fn glass_panel(ui: &Ui, rect: Rect, corner: f32, strength: f32) {
    let shadow = egui::Shadow { offset: [0, 10], blur: 30, spread: 0, color: Color32::from_black_alpha((110.0 * strength) as u8) };
    ui.painter().add(shadow.as_shape(rect, radius_of(corner)));
    // The blurred picture of what is behind the panel, then a dark wash over it for legibility.
    crate::glass::backdrop(ui, rect, corner, strength);
    ui.painter().rect_filled(rect, radius_of(corner), Color32::from_rgba_unmultiplied(14, 14, 18, (128.0 * strength) as u8));
    // The lit top edge: a bright hairline fading out down the sides.
    ui.painter().rect_stroke(rect, radius_of(corner), Stroke::new(1.0, Color32::from_white_alpha((34.0 * strength) as u8)), StrokeKind::Inside);
    let span = (rect.width() - corner * 2.0).max(0.0);
    let half = span / 2.0;
    let left = Rect::from_min_size(rect.min + vec2(corner, 0.5), vec2(half, 1.0));
    let right = Rect::from_min_size(left.right_top(), vec2(half, 1.0));
    let lit = Color32::from_white_alpha((62.0 * strength) as u8);
    horizontal_gradient(ui, left, Color32::from_white_alpha(0), lit);
    horizontal_gradient(ui, right, lit, Color32::from_white_alpha(0));
}

/// Poster, square or landscape dimensions for a card `width` wide.
pub fn card_size(shape: PosterShape, width: f32) -> Vec2 {
    match shape {
        PosterShape::Poster => vec2(width, width * 1.5),
        PosterShape::Square => vec2(width, width),
        PosterShape::Landscape => vec2(width, width / 1.77),
    }
}

pub struct PosterCard<'a> {
    pub images: &'a Images,
    pub item: &'a MetaPreview,
    pub width: f32,
    pub shape: PosterShape,
    pub show_title: bool,
}

impl PosterCard<'_> {
    /// Draws one card: the artwork with a lift and a ring under the pointer, the title below.
    pub fn show(self, ui: &mut Ui) -> Response {
        let t = theme::get();
        let art = card_size(self.shape, self.width);
        let title_h = if self.show_title { 44.0 } else { 0.0 };
        let (rect, response) = ui.allocate_exact_size(vec2(self.width, art.y + title_h), Sense::click());
        let hover = animate(ui, response.id.with("hover"), response.hovered(), theme::motion::NORMAL);
        let press = animate(ui, response.id.with("press"), response.is_pointer_button_down_on(), 0.08);

        let art_rect = Rect::from_min_size(rect.min, art);
        let grown = art_rect.expand2(art_rect.size() * (0.035 * hover - 0.02 * press) * 0.5);
        if ui.is_rect_visible(grown) {
            if hover > 0.01 {
                let shadow = egui::Shadow { offset: [0, 8], blur: 22, spread: 0, color: Color32::from_black_alpha((120.0 * hover) as u8) };
                ui.painter().add(shadow.as_shape(grown, radius_of(radius::POSTER)));
            }
            let url = match self.shape {
                PosterShape::Landscape => self.item.background.as_deref().or(self.item.poster.as_deref()),
                _ => self.item.poster.as_deref().or(self.item.background.as_deref()),
            };
            paint_cover_url(ui, self.images, url, grown, radius::POSTER);
            if hover > 0.0 {
                ui.painter().rect_stroke(grown, radius_of(radius::POSTER), Stroke::new(2.0, with_alpha(t.accent, hover)), StrokeKind::Inside);
            }
            if url.is_none() {
                let g = galley(ui, &self.item.name, theme::semibold(size::BODY_MD), t.text_secondary, art.x - 20.0, 4);
                ui.painter().galley(grown.center() - g.size() / 2.0, g, t.text_secondary);
            }
            if self.show_title {
                let name = galley(ui, &self.item.name, theme::semibold(size::BODY_MD), lerp_color(t.text, Color32::WHITE, hover), self.width, 1);
                ui.painter().galley(pos2(rect.left(), art_rect.bottom() + 10.0), name, t.text);
                if let Some(info) = self.item.release_info.as_deref() {
                    let sub = galley(ui, info, theme::regular(size::LABEL_SM), t.text_muted, self.width, 1);
                    ui.painter().galley(pos2(rect.left(), art_rect.bottom() + 10.0 + 19.0), sub, t.text_muted);
                }
            }
        }
        if response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            if let Some(backdrop) = self.item.background.as_deref() {
                self.images.prefetch(backdrop, ui.ctx().content_rect().width() * ui.ctx().pixels_per_point());
            }
        }
        response
    }
}

/// A section title with an optional "see all" arrow on the right, as on the home shelves.
pub fn section_header(ui: &mut Ui, title: &str, subtitle: Option<&str>) -> Rect {
    let t = theme::get();
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(width, 34.0), Sense::hover());
    ui.painter().text(rect.left_center() + vec2(0.0, -1.0), Align2::LEFT_CENTER, title, theme::bold(size::TITLE_SM + 1.0), t.text);
    if let Some(subtitle) = subtitle {
        let title_w = ui.painter().layout_no_wrap(title.to_owned(), theme::bold(size::TITLE_SM + 1.0), t.text).size().x;
        ui.painter().text(
            rect.left_center() + vec2(title_w + 12.0, 2.0),
            Align2::LEFT_CENTER,
            subtitle,
            theme::regular(size::LABEL_SM),
            t.text_muted,
        );
    }
    rect
}

pub fn spinner(ui: &Ui, center: Pos2, radius_px: f32, color: Color32) {
    let time = ui.input(|i| i.time) as f32;
    let start = time * 4.5;
    let sweep = 1.6 + 1.1 * (time * 2.2).sin();
    let points: Vec<Pos2> = (0..=40)
        .map(|i| {
            let angle = start + sweep * i as f32 / 40.0;
            center + vec2(angle.cos(), angle.sin()) * radius_px
        })
        .collect();
    ui.painter().add(Shape::line(points, Stroke::new(3.0, color)));
    ui.ctx().request_repaint();
}

pub fn page_padding(width: f32) -> f32 {
    if width >= 1500.0 { space::S56 } else if width >= 1100.0 { space::S40 } else { space::S32 }
}

/// A switch with a label on its left and a description underneath. Returns whether it was flipped.
pub fn switch_row(ui: &mut Ui, title: &str, description: Option<&str>, value: &mut bool) -> bool {
    let t = theme::get();
    let width = ui.available_width();
    let desc = description.map(|d| galley(ui, d, theme::regular(size::BODY_SM), t.text_muted, width - 90.0, 3));
    let height = 20.0 + 22.0 + desc.as_ref().map(|g| g.size().y + 4.0).unwrap_or(0.0);
    let (rect, response) = ui.allocate_exact_size(vec2(width, height), Sense::click());
    let on = animate(ui, response.id.with("on"), *value, theme::motion::NORMAL);
    ui.painter().text(rect.left_top() + vec2(0.0, 10.0), Align2::LEFT_TOP, title, theme::semibold(size::BODY_LG), t.text);
    if let Some(desc) = desc {
        ui.painter().galley(rect.left_top() + vec2(0.0, 34.0), desc, t.text_muted);
    }
    let track = Rect::from_center_size(pos2(rect.right() - 26.0, rect.top() + 22.0), vec2(48.0, 28.0));
    ui.painter().rect_filled(track, radius_of(14.0), lerp_color(with_alpha(Color32::WHITE, 0.16), t.accent, on));
    let knob_x = track.left() + 14.0 + on * 20.0;
    ui.painter().circle_filled(pos2(knob_x, track.center().y), 11.0, lerp_color(Color32::from_gray(210), t.on_accent, on));
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    if response.clicked() {
        *value = !*value;
        return true;
    }
    false
}

/// A single line text field in the app's style. `hint` shows while it is empty.
pub fn text_field(ui: &mut Ui, text: &mut String, hint: &str, width: f32) -> Response {
    let t = theme::get();
    let (rect, _) = ui.allocate_exact_size(vec2(width, 44.0), Sense::hover());
    let id = ui.make_persistent_id(("text-field", hint));
    let focused = ui.memory(|m| m.has_focus(id));
    let focus = animate(ui, id.with("focus"), focused, theme::motion::FAST);
    ui.painter().rect_filled(rect, radius_of(radius::CHIP), with_alpha(Color32::WHITE, 0.07));
    ui.painter().rect_stroke(rect, radius_of(radius::CHIP), Stroke::new(1.5, lerp_color(with_alpha(Color32::WHITE, 0.10), t.accent, focus)), StrokeKind::Inside);
    let inner = rect.shrink2(vec2(18.0, 6.0));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(inner).layout(egui::Layout::left_to_right(egui::Align::Center)));
    child.add(
        egui::TextEdit::singleline(text)
            .id(id)
            .hint_text(egui::RichText::new(hint).color(t.text_muted))
            .font(theme::regular(size::BODY_LG))
            .text_color(t.text)
            .frame(egui::Frame::NONE)
            .desired_width(inner.width()),
    )
}
