//! "Cyber dark" look: a near-black base with neon accents, inspired by Fork's
//! density and GitKraken's colourful graph.

use std::rc::Rc;

use gpui_kit::component::{Theme, ThemeConfig, ThemeMode, ThemeRegistry};
use gpui_kit::{App, Hsla, Rgba, rgb};

const THEME_JSON: &str = include_str!("../assets/theme.json");

pub fn cyan() -> Hsla {
    rgb(0x00e5ff).into()
}
pub fn magenta() -> Hsla {
    rgb(0xff2bd6).into()
}
pub fn lime() -> Hsla {
    rgb(0xa3ff12).into()
}
pub fn amber() -> Hsla {
    rgb(0xffb020).into()
}
pub fn red() -> Hsla {
    rgb(0xff4d6d).into()
}
pub fn violet() -> Hsla {
    rgb(0x8b5cf6).into()
}
pub fn panel() -> Hsla {
    rgb(0x0d121a).into()
}
pub fn raised() -> Hsla {
    rgb(0x121925).into()
}
pub fn muted() -> Hsla {
    rgb(0x64748b).into()
}

/// Graph lane palette, cycled by lane color index.
const LANES: [u32; 10] = [
    0x00e5ff, 0xff2bd6, 0xa3ff12, 0xffb020, 0x8b5cf6, 0x38bdf8, 0xf472b6, 0x2dd4bf, 0xfb7185,
    0xfacc15,
];

pub fn lane_color(index: u16) -> Hsla {
    let c: Rgba = rgb(LANES[index as usize % LANES.len()]);
    c.into()
}

/// Same color with a different alpha.
pub fn alpha(color: Hsla, a: f32) -> Hsla {
    Hsla { a, ..color }
}

pub fn init(cx: &mut App) {
    let set: serde_json::Value =
        serde_json::from_str(THEME_JSON).expect("theme.json is valid JSON");
    let config: ThemeConfig = serde_json::from_value(set["themes"][0].clone())
        .expect("theme.json matches the theme schema");
    let config = Rc::new(config);
    let _ = ThemeRegistry::global_mut(cx).load_themes_from_str(THEME_JSON);
    Theme::update(cx, |theme| {
        theme.dark_theme = config.clone();
        theme.mode = ThemeMode::Dark;
    });
    Theme::change(ThemeMode::Dark, None, cx);
}
