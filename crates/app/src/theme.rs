//! Colours, fonts and spacing. The values are the ones the Nuvio mobile app defines in `ThemeColors.kt` and
//! `Tokens.kt`, so a screen here reads like the same app.

use std::sync::{Arc, OnceLock, RwLock};

use eframe::egui::{self, Color32, FontData, FontDefinitions, FontFamily, FontId};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Palette {
    pub id: &'static str,
    pub name: &'static str,
    pub secondary: Color32,
    pub secondary_variant: Color32,
    pub on_secondary: Color32,
    pub focus_ring: Color32,
    pub focus_background: Color32,
    pub background: Color32,
    pub background_elevated: Color32,
    pub background_card: Color32,
}

const fn rgb(hex: u32) -> Color32 {
    Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

const DARK_TEXT: Color32 = rgb(0x111111);

#[allow(clippy::too_many_arguments)]
const fn palette(
    id: &'static str,
    name: &'static str,
    secondary: u32,
    secondary_variant: u32,
    on_secondary: Color32,
    focus_ring: u32,
    focus_background: u32,
    background: u32,
    background_elevated: u32,
    background_card: u32,
) -> Palette {
    Palette {
        id,
        name,
        secondary: rgb(secondary),
        secondary_variant: rgb(secondary_variant),
        on_secondary,
        focus_ring: rgb(focus_ring),
        focus_background: rgb(focus_background),
        background: rgb(background),
        background_elevated: rgb(background_elevated),
        background_card: rgb(background_card),
    }
}

pub const PALETTES: [Palette; 12] = [
    palette("white", "White", 0xF5F5F5, 0xE0E0E0, DARK_TEXT, 0xFFFFFF, 0x303030, 0x0D0D0D, 0x1A1A1A, 0x222222),
    palette("amber", "Amber", 0xFB8C00, 0xEF6C00, Color32::WHITE, 0xFFA726, 0x3D2D1A, 0x0F0D0D, 0x1E1A1A, 0x24201A),
    palette("gold", "Gold", 0xE8A91C, 0x9A6200, DARK_TEXT, 0xFFD45C, 0x3D2D1A, 0x0F0E0B, 0x1D1A14, 0x262116),
    palette("jade", "Jade", 0x22D37C, 0x0BBF9A, DARK_TEXT, 0x7BF08D, 0x153A2C, 0x0B0F0D, 0x141D18, 0x16251D),
    palette("rose-gold", "Rose Gold", 0xEC70A9, 0xB75AFF, DARK_TEXT, 0xFFB37A, 0x442037, 0x100C0F, 0x1F161D, 0x281A24),
    palette("arctic-blue", "Arctic Blue", 0x3185F5, 0x4D55E8, Color32::WHITE, 0x4DE3FF, 0x172844, 0x0B0E14, 0x141A24, 0x161E2A),
    palette("graphite", "Graphite", 0xAAB2BE, 0x687381, DARK_TEXT, 0xF3F5F7, 0x30343A, 0x0C0D0F, 0x17191D, 0x20242A),
    palette("crimson", "Crimson", 0xE53935, 0xC62828, Color32::WHITE, 0xFF5252, 0x3D1A1A, 0x0D0D0D, 0x1A1A1A, 0x241A1A),
    palette("ocean", "Ocean", 0x1E88E5, 0x1565C0, Color32::WHITE, 0x42A5F5, 0x1A2D3D, 0x0D0D0F, 0x1A1A1E, 0x1A1F24),
    palette("violet", "Violet", 0x8E24AA, 0x6A1B9A, Color32::WHITE, 0xAB47BC, 0x2D1A3D, 0x0D0D0F, 0x1A1A1E, 0x1F1A24),
    palette("emerald", "Emerald", 0x43A047, 0x2E7D32, Color32::WHITE, 0x66BB6A, 0x1A3D1E, 0x0D0D0D, 0x1A1A1A, 0x1A241A),
    palette("rose", "Rose", 0xD81B60, 0xC2185B, Color32::WHITE, 0xEC407A, 0x3D1A2D, 0x0D0D0D, 0x1A1A1A, 0x241A1F),
];

pub fn palette_by_id(id: &str) -> Palette {
    PALETTES.iter().copied().find(|p| p.id == id).unwrap_or(PALETTES[0])
}

/// What every widget reads its colours from.
#[derive(Debug, Clone)]
pub struct Theme {
    pub palette: Palette,
    pub amoled: bool,
    pub background: Color32,
    pub surface: Color32,
    pub card: Color32,
    pub accent: Color32,
    pub on_accent: Color32,
    pub text: Color32,
    pub text_secondary: Color32,
    pub text_muted: Color32,
    pub border: Color32,
    pub border_subtle: Color32,
    pub danger: Color32,
    pub success: Color32,
    pub warning: Color32,
    pub info: Color32,
}

impl Theme {
    pub fn new(palette: Palette, amoled: bool) -> Self {
        Self {
            palette,
            amoled,
            background: if amoled { Color32::BLACK } else { palette.background },
            surface: palette.background_elevated,
            card: palette.background_card,
            accent: palette.secondary,
            on_accent: palette.on_secondary,
            text: rgb(0xF5F7F8),
            text_secondary: rgb(0xB8BEC5),
            text_muted: rgb(0x969CA3),
            border: rgb(0x252A2A),
            border_subtle: Color32::from_rgba_unmultiplied(0x25, 0x2A, 0x2A, 140),
            danger: rgb(0xE36A8A),
            success: rgb(0x66BB6A),
            warning: rgb(0xFFC857),
            info: rgb(0x42A5F5),
        }
    }
}

fn slot() -> &'static RwLock<Arc<Theme>> {
    static THEME: OnceLock<RwLock<Arc<Theme>>> = OnceLock::new();
    THEME.get_or_init(|| RwLock::new(Arc::new(Theme::new(PALETTES[0], false))))
}

/// The active theme. Cheap: an `Arc` clone.
pub fn get() -> Arc<Theme> {
    slot().read().map(|t| t.clone()).unwrap_or_else(|e| e.into_inner().clone())
}

pub fn set(theme: Theme) {
    match slot().write() {
        Ok(mut guard) => *guard = Arc::new(theme),
        Err(poisoned) => *poisoned.into_inner() = Arc::new(theme),
    }
}

/// Spacing scale from `NuvioTokens.Space`, in points.
pub mod space {
    pub const S4: f32 = 4.0;
    pub const S6: f32 = 6.0;
    pub const S8: f32 = 8.0;
    pub const S10: f32 = 10.0;
    pub const S12: f32 = 12.0;
    pub const S14: f32 = 14.0;
    pub const S16: f32 = 16.0;
    pub const S20: f32 = 20.0;
    pub const S24: f32 = 24.0;
    pub const S32: f32 = 32.0;
    pub const S40: f32 = 40.0;
    pub const S56: f32 = 56.0;
}

/// Corner radii from `NuvioTokens.Radius`.
pub mod radius {
    pub const POSTER: f32 = 12.0;
    pub const CARD: f32 = 24.0;
    pub const BUTTON: f32 = 16.0;
    pub const CHIP: f32 = 999.0;
}

/// Type scale from `NuvioTokens.Type`, in points.
pub mod size {
    pub const LABEL_XS: f32 = 11.0;
    pub const LABEL_SM: f32 = 12.0;
    pub const BODY_SM: f32 = 13.0;
    pub const BODY_MD: f32 = 14.0;
    pub const BODY_APP: f32 = 15.0;
    pub const BODY_LG: f32 = 16.0;
    pub const TITLE_SM: f32 = 18.0;
    pub const TITLE_MD: f32 = 22.0;
    pub const HEADLINE: f32 = 26.0;
    pub const TITLE_LG: f32 = 28.0;
    pub const DISPLAY_SM: f32 = 32.0;
    pub const PAGE_DISPLAY: f32 = 38.0;
    pub const DISPLAY_MD: f32 = 48.0;
}

/// Motion durations from `NuvioTokens.Motion`, in seconds.
pub mod motion {
    pub const FAST: f32 = 0.15;
    pub const NORMAL: f32 = 0.22;
    pub const SLOW: f32 = 0.40;
    pub const CINEMATIC: f32 = 0.70;
}

pub fn semibold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("semibold".into()))
}

pub fn bold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("bold".into()))
}

pub fn regular(size: f32) -> FontId {
    FontId::new(size, FontFamily::Proportional)
}

pub fn icon_font(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("icons".into()))
}

pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::empty();
    let faces: [(&str, &'static [u8]); 4] = [
        ("jb-regular", include_bytes!("../../../assets/fonts/jetbrains_sans_regular.ttf")),
        ("jb-semibold", include_bytes!("../../../assets/fonts/jetbrains_sans_semibold.ttf")),
        ("jb-bold", include_bytes!("../../../assets/fonts/jetbrains_sans_bold.ttf")),
        ("icons", include_bytes!("../../../assets/fonts/MaterialIconsRound-Regular.otf")),
    ];
    for (name, bytes) in faces {
        fonts.font_data.insert(name.to_owned(), Arc::new(FontData::from_static(bytes)));
    }
    let chain = |first: &str| vec![first.to_owned(), "jb-regular".to_owned()];
    fonts.families.insert(FontFamily::Proportional, vec!["jb-regular".to_owned()]);
    fonts.families.insert(FontFamily::Monospace, vec!["jb-regular".to_owned()]);
    fonts.families.insert(FontFamily::Name("semibold".into()), chain("jb-semibold"));
    fonts.families.insert(FontFamily::Name("bold".into()), chain("jb-bold"));
    fonts.families.insert(FontFamily::Name("icons".into()), vec!["icons".to_owned()]);
    ctx.set_fonts(fonts);
}

/// Visuals for the widgets egui draws itself (text edits, scroll bars, tooltips). Most of Nuvio is custom painted.
pub fn apply(ctx: &egui::Context, theme: &Theme) {
    set(theme.clone());
    ctx.set_theme(egui::ThemePreference::Dark);
    ctx.global_style_mut(|style| {
        let v = &mut style.visuals;
        v.dark_mode = true;
        v.panel_fill = theme.background;
        v.window_fill = theme.surface;
        v.extreme_bg_color = theme.card;
        v.faint_bg_color = theme.surface;
        v.window_stroke = egui::Stroke::new(1.0, theme.border);
        v.override_text_color = Some(theme.text);
        v.selection.bg_fill = theme.accent.gamma_multiply(0.45);
        v.selection.stroke = egui::Stroke::new(1.0, theme.accent);
        v.hyperlink_color = theme.accent;
        v.window_shadow = egui::Shadow::NONE;
        v.popup_shadow = egui::Shadow { offset: [0, 8], blur: 24, spread: 0, color: Color32::from_black_alpha(140) };
        v.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, theme.border);
        v.widgets.inactive.weak_bg_fill = theme.card;
        v.widgets.inactive.bg_fill = theme.card;
        v.widgets.hovered.weak_bg_fill = theme.card.gamma_multiply(1.25);
        v.widgets.active.weak_bg_fill = theme.card.gamma_multiply(1.5);
        // Hinting keeps the stems of small text on whole pixels; sub-pixel positions made it look soft.
        v.text_options.font_hinting = true;
        v.text_options.subpixel_binning = false;
        style.spacing.item_spacing = egui::vec2(space::S8, space::S8);
        style.interaction.tooltip_delay = 0.25;
        style.interaction.show_tooltips_only_when_still = false;
        style.spacing.scroll = egui::style::ScrollStyle::floating();
    });
}
