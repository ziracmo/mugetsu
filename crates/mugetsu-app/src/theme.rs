//! "Cyber dark" glass look: a translucent blurred base with soft accents, inspired by Fork's
//! density and GitKraken's colourful graph.

use std::rc::Rc;

use gpui_kit::component::{Theme, ThemeConfig, ThemeMode, ThemeRegistry};
use gpui_kit::{App, Hsla, Rgba, rgb};

const THEME_JSON: &str = include_str!("../assets/theme.json");

pub fn cyan() -> Hsla {
    rgb(0x6cc7dc).into()
}
pub fn magenta() -> Hsla {
    rgb(0xc48ad6).into()
}
pub fn lime() -> Hsla {
    rgb(0x9ccc7a).into()
}
pub fn amber() -> Hsla {
    rgb(0xd9b26f).into()
}
pub fn red() -> Hsla {
    rgb(0xe07a86).into()
}
pub fn violet() -> Hsla {
    rgb(0x9d8fe8).into()
}
/// Frosted surface: translucent so the window blur shows through.
pub fn panel() -> Hsla {
    alpha(rgb(0x121826).into(), 0.55)
}
/// Hover / raised overlay, works on any surface.
pub fn raised() -> Hsla {
    alpha(rgb(0xffffff).into(), 0.06)
}
/// Hairline used for glass edges.
pub fn hairline() -> Hsla {
    alpha(rgb(0xffffff).into(), 0.08)
}
pub fn muted() -> Hsla {
    rgb(0x7d8798).into()
}

/// Graph lane palette, cycled by lane color index.
const LANES: [u32; 10] = [
    0x6cc7dc, 0xc48ad6, 0x9ccc7a, 0xd9b26f, 0x9d8fe8, 0x6fa8dc, 0xd68fb0, 0x6cc4b0, 0xd98a8a,
    0xcfc27a,
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
