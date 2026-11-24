//! What is on screen while a stream opens: the artwork of the title, dimmed, behind its wordmark (or its name when it has
//! none), which breathes slowly, with what is happening underneath. The Kotlin build draws the same thing in
//! `OpeningOverlay.kt`; the numbers below are the ones it uses.

use eframe::egui::{self, Align, Color32, Id, LayerId, Order, Rect, Sense, Ui, emath::TSTransform, epaint::text::LayoutJob, pos2, vec2};

use crate::icons;
use crate::images::{Image, Images};
use crate::theme::{self, size};
use crate::ui_kit::{self, with_alpha};

/// The wordmark or title is invisible for this long, then fades in over `CONTENT_FADE`.
const CONTENT_DELAY: f32 = 0.4;
const CONTENT_FADE: f32 = 0.7;
/// One way of the breathing: 1.0 to 1.04 times its size in two seconds, and back.
const PULSE_SECS: f32 = 2.0;
const PULSE_GROWTH: f32 = 0.04;
/// The picture's fade-in, as everywhere else in the app.
const ARTWORK_FADE: f32 = 0.28;

pub struct Opening<'a> {
    pub artwork: Option<&'a str>,
    pub logo: Option<&'a str>,
    pub title: &'a str,
    pub message: Option<&'a str>,
}

/// Where the content breathes between: 1.0 at the start and every `2 * PULSE_SECS` after, `1 + PULSE_GROWTH` in between.
fn pulse(elapsed: f32) -> f32 {
    let phase = (elapsed % (2.0 * PULSE_SECS)) / PULSE_SECS;
    1.0 + PULSE_GROWTH * if phase < 1.0 { phase } else { 2.0 - phase }
}

/// Draws the overlay over `rect` at `alpha` (it fades out when the picture starts). `elapsed` is how many seconds it has
/// been showing. Returns whether the back button was pressed.
pub fn show(ui: &mut Ui, rect: Rect, images: &Images, opening: &Opening, elapsed: f32, alpha: f32) -> bool {
    let ppp = ui.ctx().pixels_per_point();
    ui.painter().rect_filled(rect, 0.0, with_alpha(Color32::BLACK, alpha));

    // The artwork, cropped to its top right corner, under a wash that darkens towards the bottom.
    if let Some(url) = opening.artwork {
        if let Image::Ready { texture, age } = images.get(url, rect.width() * ppp) {
            let fade = (age.as_secs_f32() / ARTWORK_FADE).min(1.0) * alpha;
            if age.as_secs_f32() < ARTWORK_FADE {
                crate::services::animate_on(ui.ctx());
            }
            let uv = ui_kit::cover_uv(texture.size_vec2(), rect.size(), vec2(1.0, 0.0));
            egui::Image::new((texture.id(), rect.size())).uv(uv).tint(Color32::from_white_alpha((fade * 255.0) as u8)).paint_at(ui, rect);
            let wash = |opacity: f32| Color32::from_black_alpha((255.0 * opacity * fade) as u8);
            let stops = [(0.0, 0.3), (0.35, 0.6), (0.7, 0.8), (1.0, 0.9)];
            for pair in stops.windows(2) {
                let band = Rect::from_min_max(pos2(rect.left(), rect.top() + rect.height() * pair[0].0), pos2(rect.right(), rect.top() + rect.height() * pair[1].0));
                ui_kit::vertical_gradient(ui, band, wash(pair[0].1), wash(pair[1].1));
            }
        }
    }

    // Back, top right.
    let button = Rect::from_min_size(pos2(rect.right() - 64.0, rect.top() + 20.0), vec2(44.0, 44.0));
    let response = ui.interact(button, Id::unique("opening-back"), Sense::click());
    let hover = ui_kit::animate(ui, response.id.with("hover"), response.hovered(), theme::motion::FAST);
    ui.painter().circle_filled(button.center(), 22.0, with_alpha(Color32::BLACK, (0.3 + 0.15 * hover) * alpha));
    ui_kit::draw_icon(ui, button.center(), icons::ARROW_BACK, 24.0, with_alpha(Color32::WHITE, alpha));
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }

    // The wordmark, or the name in its place. Both sit in the middle of the window, and both breathe.
    let content_alpha = ((elapsed - CONTENT_DELAY) / CONTENT_FADE).clamp(0.0, 1.0) * alpha;
    let centre = rect.center();
    let logo_box = vec2(320f32.min(rect.width() - 48.0), 180f32.min(rect.height() * 0.4));
    let layer = LayerId::new(Order::Foreground, Id::unique("opening-content"));
    let painter = ui.ctx().layer_painter(layer);
    let logo = opening.logo.filter(|url| !url.is_empty()).map(|url| images.get(url, logo_box.x * ppp));
    let content_height = match logo {
        Some(Image::Ready { texture, .. }) => {
            let tex = texture.size_vec2();
            let size = tex * (logo_box.x / tex.x).min(logo_box.y / tex.y);
            let target = Rect::from_center_size(centre, size);
            painter.image(texture.id(), target, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), with_alpha(Color32::WHITE, content_alpha));
            logo_box.y
        }
        // Still on its way: the space is kept so the status line does not jump when it arrives.
        Some(Image::Loading) => logo_box.y,
        _ if !opening.title.is_empty() => {
            let font_size = if rect.width() < 600.0 { 30.0 } else { 42.0 };
            let mut job = LayoutJob::simple(opening.title.to_owned(), theme::bold(font_size), with_alpha(Color32::WHITE, content_alpha), (rect.width() - 96.0).max(1.0));
            job.wrap.max_rows = 2;
            job.halign = Align::Center;
            let galley = painter.layout_job(job);
            let height = galley.size().y;
            // With a centred layout the galley's origin is the middle of its rows.
            painter.galley(pos2(centre.x, centre.y - height / 2.0), galley, Color32::WHITE);
            height
        }
        _ => {
            ui_kit::spinner(ui, centre, 20.0, with_alpha(Color32::WHITE, alpha));
            54.0
        }
    };
    let scale = pulse(elapsed);
    ui.ctx().transform_layer_shapes(layer, TSTransform::new(centre.to_vec2() * (1.0 - scale), scale));

    // What is going on, under the content. It does not breathe.
    if let Some(message) = opening.message.filter(|m| !m.is_empty()) {
        let y = centre.y + content_height / 2.0 + 16.0 + 20.0;
        ui_kit::draw_text(ui, pos2(centre.x, y), egui::Align2::CENTER_CENTER, message, theme::semibold(size::LABEL_SM + 1.0), with_alpha(Color32::WHITE, 0.72 * alpha));
    }

    // The breathing never rests while the overlay is up.
    crate::services::animate_on(ui.ctx());
    response.clicked()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn breathes_out_and_back() {
        assert_eq!(pulse(0.0), 1.0);
        assert!((pulse(PULSE_SECS) - (1.0 + PULSE_GROWTH)).abs() < 1e-6);
        assert!((pulse(PULSE_SECS * 2.0) - 1.0).abs() < 1e-6);
        assert!((pulse(PULSE_SECS * 3.0) - (1.0 + PULSE_GROWTH)).abs() < 1e-6);
        let halfway = pulse(PULSE_SECS * 1.5);
        assert!(halfway > 1.0 && halfway < 1.0 + PULSE_GROWTH);
    }
}
