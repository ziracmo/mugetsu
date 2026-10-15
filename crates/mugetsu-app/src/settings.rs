//! User preferences (language, accent colour), saved as JSON and applied live.

use std::collections::HashMap;
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
    (0xf87171, "Rouge", "Red"),
    (0xe8853d, "Orange", "Orange"),
    (0xd9b26f, "Ambre", "Amber"),
    (0x8ac46e, "Vert", "Green"),
    (0x6cc7dc, "Cyan", "Cyan"),
    (0x5b8def, "Bleu", "Blue"),
    (0x9d8fe8, "Violet", "Purple"),
    (0xd96fa8, "Rose", "Pink"),
];

/// Former default red, replaced by a lighter one.
const OLD_RED: u32 = 0xe5534b;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CommitSort {
    #[default]
    Topological,
    Date,
}

impl From<CommitSort> for mugetsu_git::CommitOrder {
    fn from(s: CommitSort) -> Self {
        match s {
            CommitSort::Topological => Self::Topological,
            CommitSort::Date => Self::Date,
        }
    }
}

/// Sidebar sections, in their default order (users can drag them around).
pub const SECTIONS: [&str; 5] = ["branches", "remotes", "worktrees", "tags", "stashes"];

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub lang: Lang,
    pub accent: u32,
    pub commit_order: CommitSort,
    /// Folders holding repositories, offered when opening one.
    pub repo_folders: Vec<PathBuf>,
    /// Emoji shown next to a repository's name, by repository path.
    pub repo_icons: HashMap<PathBuf, String>,
    /// Sidebar section keys in display order.
    pub sidebar_order: Vec<String>,
    /// Recently opened repositories, newest first.
    pub recent: Vec<PathBuf>,
    /// PRs tab: merged PRs younger than this many days are listed.
    pub merged_days: u32,
    /// Every repository tab shares the sidebar width and open sidebar tab.
    pub shared_layout: bool,
    /// Sidebar width in pixels (shared layout), once resized.
    pub sidebar_width: Option<u32>,
    /// Sidebar shows the PRs tab (shared layout).
    pub sidebar_prs: bool,
    /// Keys of the folded sidebar sections.
    pub collapsed_sections: Vec<String>,
    /// Last PRs tab filter.
    pub pr_filter: PrFilter,
    /// Which PRs get a chip on branches (repository tab).
    pub pr_chips: PrFilter,
    /// History shows one row per branch instead of every commit.
    pub history_compact: bool,
    /// AI tool launched by a PR's "Review with AI".
    pub ai_tool: AiTool,
    /// Shell command run in a terminal for that review; see [`crate::ai_review`].
    pub ai_command: String,
}

/// AI coding tool a PR review is started with.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AiTool {
    #[default]
    Claude,
    Codex,
    Custom,
}

impl AiTool {
    pub const ALL: [AiTool; 3] = [AiTool::Claude, AiTool::Codex, AiTool::Custom];

    pub fn label(self) -> &'static str {
        match self {
            AiTool::Claude => "Claude Code",
            AiTool::Codex => "Codex",
            AiTool::Custom => crate::i18n::tr("Autre", "Other"),
        }
    }

    /// The command picked with the tool; `Custom` keeps the user's own.
    pub fn preset(self) -> Option<&'static str> {
        match self {
            AiTool::Claude => Some("claude \"/code-review {url}\""),
            AiTool::Codex => Some("codex \"Review the pull request {url} (branch {branch})\""),
            AiTool::Custom => None,
        }
    }
}

/// PRs tab quick filter, shared by every repository.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PrFilter {
    #[default]
    All,
    /// Authored by me.
    Mine,
    /// My review is requested.
    Review,
}

/// Choices offered for [`Settings::merged_days`].
pub const MERGED_DAYS: [u32; 4] = [3, 7, 14, 30];

impl Default for Settings {
    fn default() -> Self {
        Self {
            lang: system_lang(),
            accent: ACCENTS[0].0,
            commit_order: CommitSort::default(),
            repo_folders: Vec::new(),
            repo_icons: HashMap::new(),
            sidebar_order: SECTIONS.iter().map(|s| s.to_string()).collect(),
            recent: Vec::new(),
            merged_days: 7,
            shared_layout: true,
            sidebar_width: None,
            sidebar_prs: false,
            collapsed_sections: vec!["tags".into()],
            pr_filter: PrFilter::All,
            pr_chips: PrFilter::All,
            history_compact: false,
            ai_tool: AiTool::Claude,
            ai_command: AiTool::Claude.preset().unwrap_or_default().to_string(),
        }
    }
}

impl Settings {
    /// Sidebar sections in display order: saved order first, unknown keys
    /// dropped, missing ones appended.
    pub fn sections(&self) -> Vec<&'static str> {
        let mut out: Vec<&'static str> = self
            .sidebar_order
            .iter()
            .filter_map(|k| SECTIONS.iter().copied().find(|s| s == k))
            .collect();
        for s in SECTIONS {
            if !out.contains(&s) {
                out.push(s);
            }
        }
        out
    }

    /// Moves section `key` onto section `target`'s place.
    pub fn move_section(&mut self, key: &str, target: &str) {
        let mut order = self.sections();
        let Some(from) = order.iter().position(|s| *s == key) else {
            return;
        };
        let item = order.remove(from);
        let Some(to) = order.iter().position(|s| *s == target) else {
            order.insert(from, item);
            return;
        };
        // Dragged down: lands after the target; dragged up: before it.
        order.insert(if from > to { to } else { to + 1 }, item);
        self.sidebar_order = order.iter().map(|s| s.to_string()).collect();
    }

    pub fn add_recent(&mut self, path: PathBuf) {
        self.recent.retain(|p| *p != path);
        self.recent.insert(0, path);
        self.recent.truncate(12);
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
            .and_then(|b| serde_json::from_slice::<Settings>(&b).ok())
            .map(|mut s| {
                if s.accent == OLD_RED {
                    s.accent = ACCENTS[0].0;
                }
                s
            })
            .unwrap_or_default()
    }

    fn save(&self) {
        if cfg!(test) {
            return;
        }
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
            ..Settings::default()
        })
}

/// Makes `settings` current: language, accent colour, every window redrawn.
pub fn install(settings: Settings, cx: &mut gpui_kit::App) {
    crate::i18n::set_english(settings.lang == Lang::En);
    if let Ok(mut current) = CURRENT.write() {
        *current = Some(settings);
    }
    crate::theme::apply(cx);
    let revision = cx.try_global::<Revision>().map_or(0, |r| r.0) + 1;
    cx.set_global(Revision(revision));
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

/// Persists a UI memory (layout, folded sections) without redrawing the app.
pub fn remember(f: impl FnOnce(&mut Settings)) {
    let mut settings = get();
    f(&mut settings);
    settings.save();
    if let Ok(mut current) = CURRENT.write() {
        *current = Some(settings);
    }
}

/// Bumped on every settings change; views observe it to react.
pub struct Revision(pub u64);
impl gpui_kit::Global for Revision {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sections_can_be_reordered() {
        let mut s = Settings::default();
        assert_eq!(s.sections(), SECTIONS);
        s.move_section("tags", "branches");
        assert_eq!(
            s.sections(),
            ["tags", "branches", "remotes", "worktrees", "stashes"]
        );
        s.move_section("tags", "stashes");
        assert_eq!(
            s.sections(),
            ["branches", "remotes", "worktrees", "stashes", "tags"]
        );
        s.move_section("tags", "remotes");
        assert_eq!(
            s.sections(),
            ["branches", "tags", "remotes", "worktrees", "stashes"]
        );
        s.sidebar_order = vec!["stashes".into(), "bogus".into()];
        assert_eq!(
            s.sections(),
            ["stashes", "branches", "remotes", "worktrees", "tags"]
        );
    }

    #[test]
    fn old_settings_files_still_load() {
        let s: Settings = serde_json::from_str(r#"{"lang":"en","accent":123}"#).unwrap();
        assert_eq!(s.lang, Lang::En);
        assert_eq!(s.commit_order, CommitSort::Topological);
        assert_eq!(s.sections().len(), 5);
    }
}
