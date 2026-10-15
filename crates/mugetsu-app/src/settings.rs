//! User preferences (language, accent colour), saved as JSON and applied live.

use std::path::PathBuf;
use std::sync::RwLock;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Lang {
    #[default]
    Fr,
    En,
}

/// Accent colours offered in the settings window (0xRRGGBB).
pub const ACCENTS: [(u32, &str, &str); 8] = [
    (0xe5534b, "Rouge", "Red"),
    (0xe8853d, "Orange", "Orange"),
    (0xd9b26f, "Ambre", "Amber"),
    (0x8ac46e, "Vert", "Green"),
    (0x6cc7dc, "Cyan", "Cyan"),
    (0x5b8def, "Bleu", "Blue"),
    (0x9d8fe8, "Violet", "Purple"),
    (0xd96fa8, "Rose", "Pink"),
];

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub lang: Lang,
    pub accent: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            lang: system_lang(),
            accent: ACCENTS[0].0,
        }
    }
}

fn system_lang() -> Lang {
    match sys_locale::get_locale() {
        Some(l) if !l.to_lowercase().starts_with("fr") => Lang::En,
        _ => Lang::Fr,
    }
}

static CURRENT: RwLock<Option<Settings>> = RwLock::new(None);

fn file() -> Option<PathBuf> {
    Some(dirs::config_dir()?.join("Mugetsu").join("settings.json"))
}

impl Settings {
    pub fn load() -> Self {
        file()
            .and_then(|f| std::fs::read(f).ok())
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    fn save(&self) {
        let Some(f) = file() else { return };
        if let Some(dir) = f.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(json) = serde_json::to_vec_pretty(self) {
            let _ = std::fs::write(f, json);
        }
    }
}

/// Current settings (defaults until [`install`] runs).
pub fn get() -> Settings {
    CURRENT
        .read()
        .ok()
        .and_then(|s| s.clone())
        .unwrap_or_else(|| Settings {
            lang: Lang::Fr,
            accent: ACCENTS[0].0,
        })
}

/// Makes `settings` current: language, accent colour, every window redrawn.
pub fn install(settings: Settings, cx: &mut gpui_kit::App) {
    crate::i18n::set_english(settings.lang == Lang::En);
    if let Ok(mut current) = CURRENT.write() {
        *current = Some(settings);
    }
    crate::theme::apply(cx);
    #[cfg(not(test))]
    crate::app_menu::set(cx);
    cx.refresh_windows();
}

/// Changes and persists the settings.
pub fn update(cx: &mut gpui_kit::App, f: impl FnOnce(&mut Settings)) {
    let mut settings = get();
    f(&mut settings);
    settings.save();
    install(settings, cx);
}
