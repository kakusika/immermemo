//! Theme resolution and push pipeline for immermemo.
//! Resolves Material Design 3 (M3) Dynamic Color tokens using `origiri-theme`
//! and pushes them to Slint's `ThemeColors` global via `origiri-mobile`.

use origiri_mobile::push_mobile_color_tokens;
use origiri_theme::palette::RgbColor;
use origiri_theme::{MaterialSettings, MaterialVariant, ResolvedTheme, ThemeSettings, resolve};
use slint::ComponentHandle;

use crate::{App, Theme, ThemeColors};
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

pub fn apply_theme(app: &App, app_data: &AppData) {
    let theme_choice = app_data.load_theme().unwrap_or(0);
    let color_source = app_data.load_color_source().unwrap_or(0);
    let color_seed = app_data
        .load_color_seed()
        .unwrap_or_else(|| "#6750A4".to_string());
    let palette_style = app_data.load_palette_style().unwrap_or(0);
    let contrast_level = app_data.load_contrast_level().unwrap_or(0);

    // 0: System, 1: Light, 2: Dark
    let is_dark = match theme_choice {
        1 => false,
        2 => true,
        _ => {
            #[cfg(target_os = "android")]
            {
                crate::android::system_ui::is_system_in_dark_theme()
            }
            #[cfg(not(target_os = "android"))]
            {
                false
            }
        }
    };

    let resolved = resolve_app_theme(
        theme_choice,
        color_source,
        &color_seed,
        palette_style,
        contrast_level,
        is_dark,
    );

    let to_slint_color = |r: origiri_theme::Rgba| slint::Color::from_argb_u8(r.a, r.r, r.g, r.b);
    let theme_colors = app.global::<ThemeColors>();
    let theme_flag = app.global::<Theme>();
    push_mobile_color_tokens!(theme_colors, theme_flag, resolved.colors, to_slint_color);
}
