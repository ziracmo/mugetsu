//! "Cyber dark" glass look: a translucent blurred base with soft accents, a dense
//! layout and a colourful graph.

use std::rc::Rc;

use gpui_kit::component::{Theme, ThemeConfig, ThemeMode, ThemeRegistry};
use gpui_kit::{App, Hsla, Rgba, rgb};

const THEME_JSON: &str = include_str!("../assets/theme.json");

/// The user's accent colour (settings, red by default).
pub fn accent() -> Hsla {
    rgb(crate::settings::get().accent).into()
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
/// Light frosting over the window glass (headers, sidebar).
pub fn panel() -> Hsla {
    alpha(rgb(0xffffff).into(), 0.025)
}
/// Floating content card: darker frosted glass above the blurred window.
pub fn card() -> Hsla {
    alpha(rgb(0x141416).into(), 0.9)
}
/// Opaque surface of dialogs and sheets (readable over any wallpaper).
pub fn sheet() -> Hsla {
    rgb(0x1f1f22).into()
}
/// Opaque surface of popovers, menus and tooltips.
pub fn popover() -> Hsla {
    rgb(0x262629).into()
}
/// Bright top edge of glass surfaces (light catching the rim).
pub fn glass_edge() -> Hsla {
    alpha(rgb(0xffffff).into(), 0.13)
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
    rgb(0x8a8a8a).into()
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

/// (Re)loads the theme with the accent colour from the settings.
pub fn apply(cx: &mut App) {
    let accent = crate::settings::get().accent;
    let hex = |a: u8| format!("#{accent:06x}{a:02x}");
    let mut set: serde_json::Value =
        serde_json::from_str(THEME_JSON).expect("theme.json is valid JSON");
    let colors = &mut set["themes"][0]["colors"];
    for (key, a) in [
        ("primary.background", 0xd9),
        ("primary.hover.background", 0xff),
        ("primary.active.background", 0xb3),
        ("caret", 0xff),
        ("ring", 0x80),
        ("selection.background", 0x40),
        ("list.active.background", 0x24),
        ("list.active.border", 0x80),
        ("link.foreground", 0xff),
        ("link.hover.foreground", 0xff),
        ("link.active.foreground", 0xff),
        ("progress_bar.background", 0xff),
        ("drag_border", 0xff),
        ("drop_target.background", 0x22),
        ("sidebar.primary.background", 0xff),
        ("scrollbar.thumb.hover.background", 0x80),
    ] {
        colors[key] = serde_json::Value::String(hex(a));
    }
    colors["primary.foreground"] = "#ffffff".into();
    set["themes"][0]["highlight"]["editor.active_line_number"] = hex(0xff).into();
    let config: ThemeConfig = serde_json::from_value(set["themes"][0].clone())
        .expect("theme.json matches the theme schema");
    let config = Rc::new(config);
    let _ = ThemeRegistry::global_mut(cx).load_themes_from_str(&set.to_string());
    Theme::update(cx, |theme| {
        theme.dark_theme = config.clone();
        theme.mode = ThemeMode::Dark;
    });
    Theme::change(ThemeMode::Dark, None, cx);
}

/// Installs the theme with the current settings.
#[cfg(test)]
pub fn init(cx: &mut App) {
    apply(cx);
}

/// Inside of graph nodes (opaque so lines don't show through).
pub fn node_fill() -> Hsla {
    rgb(0x161618).into()
}
