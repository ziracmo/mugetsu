//! User preferences (language, accent colour), saved as JSON and applied live.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

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

/// Compact view order: most recent tip first, or most commits first.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CompactSort {
    #[default]
    Date,
    Size,
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
    /// The first-launch eclipse has played.
    pub intro_seen: bool,
    /// History shows one row per branch instead of every commit.
    pub history_compact: bool,
    /// History shows the commit details beside the list instead of below it.
    pub details_side: bool,
    /// The Changes view lists files as a folder tree instead of a flat list.
    pub changes_tree: bool,
    /// Dropping a branch chip on another asks before rebasing.
    pub confirm_drag_rebase: bool,
    /// How the compact view orders branches.
    pub compact_sort: CompactSort,
    /// AI tool launched by a PR's "Review with AI".
    pub ai_tool: AiTool,
    /// Shell command run in a terminal for that review; see [`crate::ai_review`].
    pub ai_command: String,
    /// Where that command runs; see [`crate::ai_review::terminals`].
    #[serde(deserialize_with = "or_default")]
    pub ai_terminal: AiTerminal,
    /// System notifications about pull requests (all off by default).
    pub notifications: Notifications,
    /// GitLab personal access token (else `GITLAB_TOKEN` or the `glab` login).
    pub gitlab_token: String,
    /// Self-hosted GitLab hosts, besides gitlab.com and `gitlab.*` ones.
    pub gitlab_hosts: Vec<String>,
    /// Bitbucket `user:app-password` or access token (else `BITBUCKET_TOKEN`).
    pub bitbucket_token: String,
    /// Branch deletes done at once, without asking (local ones forced).
    pub force_delete: ForceDelete,
    /// Last answer to "also delete the local branch" when deleting a remote one.
    pub delete_local_too: bool,
    /// Workspace folder: where repositories are cloned (asked when unset).
    pub clone_dir: Option<PathBuf>,
    /// Ask where to clone every time, even with a workspace folder.
    pub clone_ask: bool,
    /// Clone over HTTPS (`Some(true)`) or SSH; `None` picks SSH when a key exists.
    pub clone_https: Option<bool>,
    /// Offer the weekly recap on Friday afternoons.
    pub recap_reminder: bool,
    /// ISO week the recap was last offered (`2026-W42`).
    pub recap_offered: String,
}

/// Which branch deletes skip the confirmation. A local one is then forced
/// (`git branch -D`), merged or not, like Fork does.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ForceDelete {
    pub local: bool,
    pub remote: bool,
}

/// Which pull request events raise a system notification.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Notifications {
    /// CI finished (passed or failed).
    pub ci: bool,
    /// My review is requested.
    pub review: bool,
    /// PR merged.
    pub merged: bool,
    /// CI and merge events of other people's PRs too, not only mine.
    pub others: bool,
}

impl Notifications {
    pub fn any(self) -> bool {
        self.ci || self.review || self.merged
    }
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

/// A value from a newer version, unknown here, reads as the default instead
/// of failing the whole file.
fn or_default<'de, D, T>(d: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::de::DeserializeOwned + Default,
{
    let value = serde_json::Value::deserialize(d)?;
    Ok(serde_json::from_value(value).unwrap_or_default())
}

/// Where a PR's "Review with AI" runs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AiTerminal {
    /// Terminal.app, Windows Terminal (else PowerShell), or the Linux default;
    /// also what an unknown saved value falls back to.
    #[default]
    System,
    Iterm,
    Ghostty,
    Wezterm,
    Kitty,
    Alacritty,
    Powershell,
    Cmd,
    /// The Claude desktop app, as a new Claude Code session.
    Desktop,
}

impl AiTerminal {
    pub fn label(self) -> &'static str {
        match self {
            AiTerminal::System if cfg!(target_os = "macos") => "Terminal",
            AiTerminal::System if cfg!(windows) => "Windows Terminal",
            AiTerminal::System => crate::i18n::tr("Terminal par défaut", "Default terminal"),
            AiTerminal::Iterm => "iTerm2",
            AiTerminal::Ghostty => "Ghostty",
            AiTerminal::Wezterm => "WezTerm",
            AiTerminal::Kitty => "kitty",
            AiTerminal::Alacritty => "Alacritty",
            AiTerminal::Powershell => "PowerShell",
            AiTerminal::Cmd => "cmd",
            AiTerminal::Desktop => "Claude Desktop",
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
            intro_seen: false,
            history_compact: false,
            details_side: false,
            changes_tree: false,
            confirm_drag_rebase: true,
            compact_sort: CompactSort::default(),
            ai_tool: AiTool::Claude,
            ai_command: AiTool::Claude.preset().unwrap_or_default().to_string(),
            ai_terminal: AiTerminal::System,
            notifications: Notifications::default(),
            gitlab_token: String::new(),
            gitlab_hosts: Vec::new(),
            bitbucket_token: String::new(),
            force_delete: ForceDelete::default(),
            delete_local_too: false,
            clone_dir: None,
            clone_ask: false,
            clone_https: None,
            recap_reminder: true,
            recap_offered: String::new(),
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

    /// Folders scanned for repositories: the repository folders and the
    /// workspace folder.
    pub fn scan_folders(&self) -> Vec<PathBuf> {
        let mut out = self.repo_folders.clone();
        if let Some(dir) = &self.clone_dir
            && !out.contains(dir)
        {
            out.push(dir.clone());
        }
        out
    }

    /// Clones go over HTTPS: the user's choice, else when no SSH key is found.
    pub fn clone_over_https(&self) -> bool {
        self.clone_https.unwrap_or_else(|| {
            let keys = dirs::home_dir().and_then(|h| std::fs::read_dir(h.join(".ssh")).ok());
            !keys.is_some_and(|mut rd| {
                rd.any(|e| {
                    e.is_ok_and(|e| {
                        let name = e.file_name().to_string_lossy().to_string();
                        name.starts_with("id_") && name.ends_with(".pub")
                    })
                })
            })
        })
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

// Shared, not cloned: views read it many times per frame (`theme::accent()` per row).
static CURRENT: RwLock<Option<Arc<Settings>>> = RwLock::new(None);

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
pub fn get() -> Arc<Settings> {
    CURRENT
        .read()
        .ok()
        .and_then(|s| s.clone())
        .unwrap_or_else(|| {
            Arc::new(Settings {
                lang: Lang::Fr,
                ..Settings::default()
            })
        })
}

/// Makes `settings` current: language, accent colour, every window redrawn.
pub fn install(settings: Settings, cx: &mut gpui_kit::App) {
    crate::i18n::set_english(settings.lang == Lang::En);
    if let Ok(mut current) = CURRENT.write() {
        *current = Some(Arc::new(settings));
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
    let mut settings = Settings::clone(&get());
    f(&mut settings);
    settings.save();
    install(settings, cx);
}

/// Persists a UI memory (layout, folded sections) without redrawing the app.
pub fn remember(f: impl FnOnce(&mut Settings)) {
    let mut settings = Settings::clone(&get());
    f(&mut settings);
    settings.save();
    if let Ok(mut current) = CURRENT.write() {
        *current = Some(Arc::new(settings));
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
        assert!(!s.notifications.any(), "notifications start switched off");
        assert_eq!(s.ai_terminal, AiTerminal::System);
        let s: Settings = serde_json::from_str(r#"{"ai_terminal":"hyper"}"#).unwrap();
        assert_eq!(s.ai_terminal, AiTerminal::System, "unknown terminal");
    }
}
