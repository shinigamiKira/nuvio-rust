//! The small custom-painted pieces every screen is built from: text that truncates, icon buttons with hover feedback,
//! pills, chips, cover-cropped images and the poster cards.

use std::sync::Arc;

use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontId, Galley, Id, Pos2, Rect, Response, Sense, Shape, Stroke, StrokeKind, Ui, Vec2,
    emath::{GuiRounding, easing},
    epaint::text::LayoutJob,
    pos2, vec2,
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
    paint_cover_anchored(ui, rect, image, corner, alpha, vec2(0.5, 0.5));
}

/// Like `paint_cover_alpha`, with `anchor` saying which part of a picture that is cropped survives (see `cover_uv`).
pub fn paint_cover_anchored(ui: &Ui, rect: Rect, image: &Image, corner: CornerRadius, alpha: f32, anchor: Vec2) {
    let t = theme::get();
    // On whole pixels, a picture prepared at the size it is drawn at lands texel for pixel; between them the sampler blends
    // neighbours, which is blur.
    let rect = rect.round_to_pixels(ui.ctx().pixels_per_point());
    match image {
        Image::Ready { texture, age } => {
            let uv = cover_uv(texture.size_vec2(), rect.size(), anchor);
            // The placeholder stays underneath so the corners are right while the image is still transparent.
            let fade = (age.as_secs_f32() / 0.28).min(1.0) * alpha;
            if fade < 1.0 && alpha >= 1.0 {
                ui.painter().rect_filled(rect, corner, t.card);
            }
            if fade < 1.0 {
                crate::services::animate_on(ui.ctx());
            }
            egui::Image::new((texture.id(), rect.size()))
                .corner_radius(corner)
                .uv(uv)
                .tint(Color32::from_white_alpha((fade * 255.0) as u8))
                .paint_at(ui, rect);
        }
        Image::Loading => {
            let pulse = 0.5 + 0.5 * (ui.input(|i| i.time) as f32 * 3.0).sin();
            // Over another picture (the hero's crossfade) the placeholder fades in with the rest instead of blanking it.
            ui.painter().rect_filled(rect, corner, with_alpha(lerp_color(t.card, t.surface, pulse * 0.6), alpha));
            // At 12 frames a second the breathing of a whole shelf of placeholders reads as flicker.
            crate::services::animate_on(ui.ctx());
        }
        Image::Failed => {
            ui.painter().rect_filled(rect, corner, with_alpha(t.card, alpha));
        }
    }
}

/// The part of a picture of `texture` size that fills a rectangle of `target` size without stretching. `anchor` says
/// which part survives the crop: (0.5, 0.5) keeps the middle, (1, 0) the top right corner.
pub fn cover_uv(texture: Vec2, target: Vec2, anchor: Vec2) -> Rect {
    let (aspect, wanted) = (texture.x / texture.y, target.x / target.y);
    if aspect > wanted {
        let w = wanted / aspect;
        let x = (1.0 - w) * anchor.x;
        Rect::from_min_max(pos2(x, 0.0), pos2(x + w, 1.0))
    } else {
        let h = aspect / wanted;
        let y = (1.0 - h) * anchor.y;
        Rect::from_min_max(pos2(0.0, y), pos2(1.0, y + h))
    }
}

/// A backdrop shown whole, never cropped. A picture that is not as wide as `rect` is wide (a poster, a portrait still, or a
/// 16:9 picture in a window that is wider than that) is drawn at the height of `rect` against its right edge, its left edge
/// fading out; the space left of it is the same picture, shrunk to a few pixels and stretched, which blurs it. A picture that
/// is nearly the shape of `rect` fills it. `alpha` fades the whole thing (a crossfade between two backdrops).
pub fn paint_backdrop_whole(ui: &Ui, images: &Images, url: &str, rect: Rect, corner: CornerRadius, alpha: f32) {
    let ppp = ui.ctx().pixels_per_point();
    let rect = rect.round_to_pixels(ppp);
    // The small copy comes first and fast: it tells the shape of the picture and is the filler.
    let small = images.get(url, 96.0);
    let Image::Ready { texture: small_texture, age } = &small else {
        paint_cover_anchored(ui, rect, &small, corner, alpha, vec2(0.5, 0.2));
        return;
    };
    let aspect = small_texture.size_vec2().x / small_texture.size_vec2().y;
    if aspect >= rect.aspect_ratio() * 0.96 {
        paint_cover_anchored(ui, rect, &images.get(url, rect.width() * ppp), corner, alpha, vec2(0.5, 0.2));
        return;
    }
    let fade = (age.as_secs_f32() / 0.28).min(1.0) * alpha;
    if age.as_secs_f32() < 0.28 {
        crate::services::animate_on(ui.ctx());
    }
    // The filler: a few pixels of the picture, enlarged by the sampler, under a dark wash.
    paint_cover_anchored(ui, rect, &small, corner, alpha, vec2(0.5, 0.5));
    ui.painter().rect_filled(rect, corner, Color32::from_black_alpha((140.0 * alpha) as u8));
    // The picture itself, prepared at the size it is drawn at.
    let width = (rect.height() * aspect).min(rect.width());
    let shown = Rect::from_min_max(pos2(rect.right() - width, rect.top()), rect.max).round_to_pixels(ppp);
    let Image::Ready { texture, .. } = images.get(url, shown.width() * ppp) else { return };
    let feather = (shown.width() * 0.14).min(180.0);
    let mut mesh = egui::Mesh::with_texture(texture.id());
    const STEPS: usize = 8;
    for i in 0..=STEPS {
        let f = i as f32 / STEPS as f32;
        let x = shown.left() + feather * f;
        let smooth = f * f * (3.0 - 2.0 * f);
        let color = Color32::from_white_alpha((255.0 * fade * smooth) as u8);
        let u = (x - shown.left()) / shown.width();
        mesh.vertices.push(egui::epaint::Vertex { pos: pos2(x, shown.top()), uv: pos2(u, 0.0), color });
        mesh.vertices.push(egui::epaint::Vertex { pos: pos2(x, shown.bottom()), uv: pos2(u, 1.0), color });
    }
    // The part past the feather is one more strip, at full strength.
    let color = Color32::from_white_alpha((255.0 * fade) as u8);
    mesh.vertices.push(egui::epaint::Vertex { pos: pos2(shown.right(), shown.top()), uv: pos2(1.0, 0.0), color });
    mesh.vertices.push(egui::epaint::Vertex { pos: pos2(shown.right(), shown.bottom()), uv: pos2(1.0, 1.0), color });
    for i in 0..=STEPS as u32 {
        let b = i * 2;
        mesh.indices.extend_from_slice(&[b, b + 1, b + 3, b, b + 3, b + 2]);
    }
    ui.painter().add(Shape::mesh(mesh));
}

pub fn paint_cover_url(ui: &Ui, images: &Images, url: Option<&str>, rect: Rect, corner: f32) {
    paint_cover_url_with(ui, images, url, rect, radius_of(corner));
}

pub fn paint_cover_url_with(ui: &Ui, images: &Images, url: Option<&str>, rect: Rect, corner: CornerRadius) {
    paint_cover_url_sized(ui, images, url, rect, rect.width(), corner);
}

/// Like `paint_cover_url_with`, with the size the picture is prepared for given apart from the rectangle it is drawn in: a
/// card that grows under the pointer keeps the picture it has instead of asking for a bigger one.
pub fn paint_cover_url_sized(ui: &Ui, images: &Images, url: Option<&str>, rect: Rect, base_width: f32, corner: CornerRadius) {
    match url {
        Some(url) => {
            let px = base_width * ui.ctx().pixels_per_point();
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

/// The shapes of a glass panel, bottom to top: a soft shadow, the glass itself (the picture behind it, bent and blurred, with
/// a rim of light; see `glass.rs`), a faint dark tint, and a hairline. `strength` fades the whole panel.
pub fn glass_shapes(ctx: &egui::Context, rect: Rect, corner: f32, strength: f32) -> Vec<Shape> {
    let radius = radius_of(corner);
    let shadow = egui::Shadow { offset: [0, 14], blur: 36, spread: 0, color: Color32::from_black_alpha((120.0 * strength) as u8) };
    vec![
        shadow.as_shape(rect, radius).into(),
        crate::glass::backdrop(ctx.pixels_per_point(), rect, corner, strength),
        Shape::rect_filled(rect, radius, Color32::from_rgba_unmultiplied(10, 10, 14, (34.0 * strength) as u8)),
        Shape::rect_stroke(rect, radius, Stroke::new(1.0, Color32::from_white_alpha((38.0 * strength) as u8)), StrokeKind::Inside),
    ]
}

/// A glass panel at `rect`, behind whatever is painted next.
pub fn glass_panel(ui: &Ui, rect: Rect, corner: f32, strength: f32) {
    for shape in glass_shapes(ui.ctx(), rect, corner, strength) {
        ui.painter().add(shape);
    }
}

/// Puts `add_contents` on glass that is as big as they turn out to be: the panel is painted behind them once their size is
/// known.
pub fn glass_frame<R>(ui: &mut Ui, corner: f32, margin: f32, add_contents: impl FnOnce(&mut Ui) -> R) -> R {
    let slot = ui.painter().add(Shape::Noop);
    let inner = egui::Frame::NONE.inner_margin(margin).show(ui, add_contents);
    let shapes = glass_shapes(ui.ctx(), inner.response.rect, corner, 1.0);
    ui.painter().set(slot, Shape::Vec(shapes));
    inner.inner
}

/// A popup menu (or context menu) drawn on glass.
pub fn glass_popup<R>(popup: egui::Popup<'_>, add_contents: impl FnOnce(&mut Ui) -> R) -> Option<egui::InnerResponse<R>> {
    popup.frame(egui::Frame::NONE).show(|ui| glass_frame(ui, 18.0, 8.0, add_contents))
}

/// One row of a menu: an icon (or nothing), a label, and a lit background under the pointer.
pub fn menu_item(ui: &mut Ui, icon: Option<char>, label: &str, selected: bool) -> Response {
    let t = theme::get();
    let width = ui.available_width().max(200.0);
    let (rect, response) = ui.allocate_exact_size(vec2(width, 38.0), Sense::click());
    let hover = animate(ui, response.id.with("h"), response.hovered(), HOVER_SECS);
    ui.painter().rect_filled(rect, radius_of(11.0), with_alpha(Color32::WHITE, 0.10 * hover));
    let colour = if selected { t.accent } else { t.text };
    let mut x = rect.left() + 14.0;
    if let Some(icon) = icon {
        draw_icon(ui, pos2(x + 10.0, rect.center().y), icon, 20.0, colour);
        x += 32.0;
    }
    let text = galley(ui, label, theme::semibold(size::BODY_SM), colour, rect.right() - x - 44.0, 1);
    ui.painter().galley(pos2(x, rect.center().y - text.size().y / 2.0), text, colour);
    if selected {
        draw_icon(ui, pos2(rect.right() - 20.0, rect.center().y), crate::icons::CHECK, 20.0, t.accent);
    }
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    response
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
            paint_cover_url_sized(ui, self.images, url, grown, art.x, radius_of(radius::POSTER));
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

/// Where a paging row is scrolled to.
pub struct RowView {
    pub offset: f32,
    pub view_w: f32,
    pub content_w: f32,
}

/// A row that scrolls sideways: by dragging, with Shift and the wheel, or with the arrows that appear on its two sides while
/// the pointer is over it (they page by most of its width, easing there). `add_contents` lays the cards out left to right;
/// `pad` is the page's side margin, kept before the first and after the last. `arrow_y` is how far below the top of the row
/// the arrows sit.
pub fn paging_row(ui: &mut Ui, scroll_id: Id, pad: f32, arrow_y: f32, add_contents: impl FnOnce(&mut Ui)) -> RowView {
    let target_id = scroll_id.with("target");
    let mut area = egui::ScrollArea::horizontal().id_salt(scroll_id).auto_shrink([false, true]).scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden);
    // Arrow clicks animate the offset; everything else (drag, touchpad) is left to the scroll area.
    let wanted: Option<f32> = ui.ctx().data(|d| d.get_temp(target_id));
    if let Some(target) = wanted {
        let eased = ui.ctx().animate_value_with_time(scroll_id.with("anim"), target, theme::motion::SLOW);
        area = area.horizontal_scroll_offset(eased);
        if (eased - target).abs() < 0.5 {
            ui.ctx().data_mut(|d| d.remove::<f32>(target_id));
        }
    }
    let output = area.show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 14.0;
            ui.add_space(pad);
            add_contents(ui);
            ui.add_space(pad);
        });
    });
    let view = RowView { offset: output.state.clamped_offset().x, view_w: output.inner_rect.width(), content_w: output.content_size.x };

    let over = ui.rect_contains_pointer(output.inner_rect);
    let hover = animate(ui, scroll_id.with("arrows"), over, theme::motion::FAST);
    if hover > 0.01 {
        let page = view.view_w * 0.8;
        let y = output.inner_rect.top() + arrow_y;
        for (side, icon, delta) in [(0, crate::icons::CHEVRON_LEFT, -page), (1, crate::icons::CHEVRON_RIGHT, page)] {
            let can = if side == 0 { view.offset > 1.0 } else { view.offset + view.view_w < view.content_w - 1.0 };
            if !can {
                continue;
            }
            let x = if side == 0 { output.inner_rect.left() + pad.min(30.0) } else { output.inner_rect.right() - pad.min(30.0) };
            let button = Rect::from_center_size(pos2(x, y), vec2(44.0, 44.0));
            let r = ui.interact(button, scroll_id.with(("arrow", side)), Sense::click());
            let h = animate(ui, r.id.with("h"), r.hovered(), theme::motion::FAST);
            ui.painter().circle_filled(button.center(), 22.0, Color32::from_black_alpha((150.0 + 60.0 * h) as u8).gamma_multiply(hover));
            ui.painter().circle_stroke(button.center(), 22.0, Stroke::new(1.0, with_alpha(Color32::WHITE, 0.18 * hover)));
            draw_icon(ui, button.center(), icon, 26.0, with_alpha(Color32::WHITE, hover));
            if r.clicked() {
                let target = (view.offset + delta).clamp(0.0, (view.content_w - view.view_w).max(0.0));
                ui.ctx().data_mut(|d| d.insert_temp(target_id, target));
                // Start the animation from where the row is now, not from where it last stopped.
                ui.ctx().animate_value_with_time(scroll_id.with("anim"), view.offset, 0.0);
                crate::services::animate_on(ui.ctx());
            }
            if r.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
        }
    }
    view
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
    crate::services::animate_on(ui.ctx());
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
    text_field_ex(ui, text, hint, width, false)
}

/// A text field that shows dots instead of what is typed.
pub fn password_field(ui: &mut Ui, text: &mut String, hint: &str, width: f32) -> Response {
    text_field_ex(ui, text, hint, width, true)
}

fn text_field_ex(ui: &mut Ui, text: &mut String, hint: &str, width: f32, password: bool) -> Response {
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
            .password(password)
            .frame(egui::Frame::NONE)
            .desired_width(inner.width()),
    )
}
