//! Theme resolution and push pipeline for immermemo.
//! Resolves Material Design 3 (M3) Dynamic Color tokens using `origiri-theme`
//! and pushes them to Slint's `ThemeColors` global via `origiri-mobile`.

use origiri_mobile::push_mobile_color_tokens;
use origiri_theme::palette::RgbColor;
use origiri_theme::{MaterialSettings, MaterialVariant, ResolvedTheme, ThemeSettings, resolve};
use slint::ComponentHandle;

use crate::{App, SettingsAdapter, Theme, ThemeColors};
use immermemo_core::AppData;

pub fn resolve_app_theme(
    _theme_choice: i32,
    color_source: i32,
    color_seed: &str,
    palette_style: i32,
    contrast_choice: i32,
    is_dark: bool,
) -> ResolvedTheme {
    let (seed_rgb, contrast_val) = if color_source == 0 {
        // System wallpaper & contrast
        #[cfg(target_os = "android")]
        let seed = crate::android::system_ui::get_wallpaper_seed_color()
            .map(|argb| {
                RgbColor::new(
                    ((argb >> 16) & 0xff) as u8,
                    ((argb >> 8) & 0xff) as u8,
                    (argb & 0xff) as u8,
                )
            })
            .unwrap_or_else(|| RgbColor::new(0x67, 0x50, 0xa4));

        #[cfg(target_os = "android")]
        let contrast = crate::android::system_ui::get_system_contrast();

        #[cfg(not(target_os = "android"))]
        let seed = RgbColor::new(0x67, 0x50, 0xa4);
        #[cfg(not(target_os = "android"))]
        let contrast = 0.0;

        (seed, contrast)
    } else {
        let seed =
            RgbColor::from_hex(color_seed).unwrap_or_else(|| RgbColor::new(0x67, 0x50, 0xa4));
        let contrast = match contrast_choice {
            1 => 0.5,
            2 => 1.0,
            _ => 0.0,
        };
        (seed, contrast)
    };

    let variant = match palette_style {
        1 => MaterialVariant::Vivid,
        2 => MaterialVariant::Expressive,
        3 => MaterialVariant::FruitSalad,
        4 => MaterialVariant::Rainbow,
        5 => MaterialVariant::Spritz,
        6 => MaterialVariant::Monochrome,
        _ => MaterialVariant::TonalSpot,
    };

    let settings = ThemeSettings::material(MaterialSettings {
        seed: seed_rgb,
        variant,
        is_dark,
        contrast: contrast_val,
    });

    resolve(&settings)
}

/// The seed color used until the user picks one. (`settings.slint`'s color
/// chips and `SettingsAdapter.color-seed` repeat the literal -- Slint cannot
/// import a Rust constant.)
pub const DEFAULT_COLOR_SEED: &str = "#6750A4";

/// The saved appearance preferences: loaded once from [`AppData`], shown in
/// the UI in one place ([`show`](Self::show)).
pub struct Appearance {
    /// 0: small, 1: medium (default), 2: large.
    pub font_size_choice: i32,
    /// 0: follow the system, 1: light, 2: dark.
    pub theme_choice: i32,
    /// 0: system wallpaper / dynamic color, 1: custom seed color.
    pub color_source: i32,
    pub color_seed: String,
    pub palette_style: i32,
    pub contrast_level: i32,
    pub status_bar_choice: i32,
}

impl Appearance {
    pub fn load(app_data: &AppData) -> Self {
        Self {
            font_size_choice: app_data.load_font_size().unwrap_or(1),
            theme_choice: app_data.load_theme().unwrap_or(0),
            color_source: app_data.load_color_source().unwrap_or(0),
            color_seed: app_data
                .load_color_seed()
                .unwrap_or_else(|| DEFAULT_COLOR_SEED.to_owned()),
            palette_style: app_data.load_palette_style().unwrap_or(0),
            contrast_level: app_data.load_contrast_level().unwrap_or(0),
            status_bar_choice: app_data.load_status_bar_choice().unwrap_or(0),
        }
    }

    /// Reflects every preference in the UI: the Settings controls, the
    /// editor's font size, and the resolved color tokens.
    pub fn show(&self, app: &App) {
        let settings = app.global::<SettingsAdapter>();
        settings.set_font_size_choice(self.font_size_choice);
        settings.set_theme_choice(self.theme_choice);
        settings.set_color_source(self.color_source);
        settings.set_color_seed(self.color_seed.as_str().into());
        settings.set_palette_style(self.palette_style);
        settings.set_contrast_level(self.contrast_level);
        settings.set_status_bar_choice(self.status_bar_choice);
        app.set_editor_font_size(editor_font_px(self.font_size_choice));
        self.push_colors(app);
    }

    /// Resolves the Material color tokens for these preferences and pushes
    /// them to the Slint theme globals.
    pub fn push_colors(&self, app: &App) {
        let is_dark = match self.theme_choice {
            1 => false,
            2 => true,
            _ => system_is_dark(),
        };
        let resolved = resolve_app_theme(
            self.theme_choice,
            self.color_source,
            &self.color_seed,
            self.palette_style,
            self.contrast_level,
            is_dark,
        );
        let to_slint_color =
            |r: origiri_theme::Rgba| slint::Color::from_argb_u8(r.a, r.r, r.g, r.b);
        let theme_colors = app.global::<ThemeColors>();
        let theme_flag = app.global::<Theme>();
        push_mobile_color_tokens!(theme_colors, theme_flag, resolved.colors, to_slint_color);
    }
}

/// The editor's font size in logical pixels for a `font_size_choice`.
pub fn editor_font_px(choice: i32) -> f32 {
    match choice {
        0 => 14.0,
        2 => 20.0,
        _ => 17.0,
    }
}

fn system_is_dark() -> bool {
    #[cfg(target_os = "android")]
    {
        crate::android::system_ui::is_system_in_dark_theme()
    }
    #[cfg(not(target_os = "android"))]
    {
        false
    }
}

/// Re-resolves and pushes the color tokens from what is saved right now --
/// after a single color preference changed.
pub fn apply_theme(app: &App, app_data: &AppData) {
    Appearance::load(app_data).push_colors(app);
}
