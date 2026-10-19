//! ⌘F / Ctrl+F: find commits by message, author, branch / tag name or SHA,
//! and step through matches; matched text is highlighted in the history.

use std::collections::HashMap;
use std::ops::Range;

use gix::ObjectId;
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::Input;
use gpui_kit::component::{Icon, Sizable as _, h_flex};
use gpui_kit::{
    AnyElement, Context, Focusable as _, InteractiveElement as _, IntoElement, ParentElement as _,
    Styled as _, Window, div, px,
};
use mugetsu_git::{History, RefLabel};

use super::{CloseSearch, RepoView};
use crate::i18n::tr;
use crate::theme;

/// Name of a ref label, if it has one.
pub fn label_name(label: &RefLabel) -> Option<&str> {
    match label {
        RefLabel::Branch { name, .. } | RefLabel::Remote(name) | RefLabel::Tag(name) => Some(name),
        RefLabel::Stash(_) | RefLabel::DetachedHead => None,
    }
}

/// Byte ranges of `text` matching any word of `query` (case-insensitive),
/// sorted and merged.
pub fn highlight_ranges(text: &str, query: &str) -> Vec<Range<usize>> {
    let lower = text.to_lowercase();
    // Lowercasing can shift byte offsets (rare non-ASCII cases): no highlight then.
    if lower.len() != text.len() {
        return Vec::new();
    }
    let mut ranges: Vec<Range<usize>> = Vec::new();
    for word in query.split_whitespace().map(str::to_lowercase) {
        let mut from = 0;
        while let Some(at) = lower[from..].find(&word) {
            let start = from + at;
            let end = start + word.len();
            if text.is_char_boundary(start) && text.is_char_boundary(end) {
                ranges.push(start..end);
            }
            from = end.max(start + 1);
            while from < lower.len() && !lower.is_char_boundary(from) {
                from += 1;
            }
        }
    }
    ranges.sort_by_key(|r| r.start);
    let mut merged: Vec<Range<usize>> = Vec::new();
    for r in ranges {
        match merged.last_mut() {
            Some(last) if r.start <= last.end => last.end = last.end.max(r.end),
            _ => merged.push(r),
        }
    }
    merged
}

/// Rows whose summary, author or ref names contain every word of `query`
/// (case-insensitive), or whose SHA starts with it.
pub fn find_commits(
    history: &History,
    labels: &HashMap<ObjectId, Vec<RefLabel>>,
    query: &str,
) -> Vec<usize> {
    let query = query.trim();
    if query.is_empty() {
        return Vec::new();
    }
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    let sha = (query.len() >= 4 && query.chars().all(|c| c.is_ascii_hexdigit()))
        .then(|| query.to_ascii_lowercase());
    history
        .commits
        .iter()
        .enumerate()
        .filter(|(_, c)| {
            if let Some(sha) = &sha
                && c.id.to_string().starts_with(sha.as_str())
            {
                return true;
            }
            let mut hay = format!("{}\n{}", c.summary, c.author);
            for name in labels
                .get(&c.id)
                .into_iter()
                .flatten()
                .filter_map(label_name)
            {
                hay.push('\n');
                hay.push_str(name);
            }
            let hay = hay.to_lowercase();
            words.iter().all(|w| hay.contains(w.as_str()))
        })
        .map(|(i, _)| i)
        .collect()
}

impl RepoView {
    pub(super) fn open_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search_open = true;
        self.search_input.focus_handle(cx).focus(window, cx);
        self.run_search(cx);
        cx.notify();
    }

    pub(super) fn close_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search_open = false;
        self.search_matches.clear();
        self.search_hits.clear();
        self.history_focus.focus(window, cx);
        cx.notify();
    }

    /// Recomputes matches and jumps to the first one at or after the selection.
    pub(super) fn run_search(&mut self, cx: &mut Context<Self>) {
        let query = self.search_input.read(cx).value().to_string();
        self.search_matches = find_commits(&self.data.history, &self.data.decorations, &query);
        self.search_hits = self.search_matches.iter().copied().collect();
        let current = self
            .selected
            .and_then(|id| self.data.history.row_of(&id))
            .unwrap_or(0);
        self.search_pos = self
            .search_matches
            .iter()
            .position(|&r| r >= current)
            .unwrap_or(0);
        self.show_match(cx);
    }

    pub(super) fn step_search(&mut self, backwards: bool, cx: &mut Context<Self>) {
        let n = self.search_matches.len();
        if n == 0 {
            return;
        }
        self.search_pos = if backwards {
            (self.search_pos + n - 1) % n
        } else {
            (self.search_pos + 1) % n
        };
        self.show_match(cx);
    }

    fn show_match(&mut self, cx: &mut Context<Self>) {
        if let Some(&row) = self.search_matches.get(self.search_pos) {
            let id = self.data.history.commits[row].id;
            self.select_commit(id, true, cx);
        }
        cx.notify();
    }

    pub(super) fn render_search(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.search_open {
            return None;
        }
        let n = self.search_matches.len();
        let has_query = !self.search_input.read(cx).value().trim().is_empty();
        let counter = match (has_query, n) {
            (false, _) => String::new(),
            (true, 0) => tr("Aucun résultat", "No results").to_string(),
            (true, n) => format!("{}/{}", self.search_pos + 1, n),
        };
        Some(
            h_flex()
                .id("commit-search")
                .test_support()
                .key_context("CommitSearch")
                .on_action(
                    cx.listener(|this, _: &CloseSearch, window, cx| this.close_search(window, cx)),
                )
                .absolute()
                .top_2()
                .right_4()
                .w(px(420.))
                .p_1p5()
                .gap_1()
                .rounded_xl()
                .border_1()
                .border_color(theme::glass_edge())
                .bg(theme::popover())
                .shadow_lg()
                .child(
                    div().flex_1().child(
                        Input::new(&self.search_input)
                            .id("commit-search-input")
                            .small()
                            .appearance(false)
                            .prefix(
                                Icon::new(Lucide::Search)
                                    .xsmall()
                                    .text_color(theme::muted()),
                            ),
                    ),
                )
                .child(
                    div()
                        .flex_none()
                        .text_xs()
                        .text_color(if has_query && n == 0 {
                            theme::red()
                        } else {
                            theme::muted()
                        })
                        .child(counter),
                )
                .child(
                    Button::new("search-prev")
                        .ghost()
                        .xsmall()
                        .icon(Icon::new(Lucide::ChevronUp))
                        .tooltip(tr("Précédent (⇧↩)", "Previous (⇧↩)"))
                        .on_click(cx.listener(|this, _, _, cx| this.step_search(true, cx))),
                )
                .child(
                    Button::new("search-next")
                        .ghost()
                        .xsmall()
                        .icon(Icon::new(Lucide::ChevronDown))
                        .tooltip(tr("Suivant (↩)", "Next (↩)"))
                        .on_click(cx.listener(|this, _, _, cx| this.step_search(false, cx))),
                )
                .child(
                    Button::new("search-close")
                        .ghost()
                        .xsmall()
                        .icon(Icon::new(Lucide::X))
                        .tooltip(tr("Fermer (échap)", "Close (esc)"))
                        .on_click(cx.listener(|this, _, window, cx| this.close_search(window, cx))),
                )
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mugetsu_git::Commit;

    fn history() -> History {
        let commit = |hex: &str, summary: &str, author: &str, time| Commit {
            id: gix::ObjectId::from_hex(hex.as_bytes()).unwrap(),
            parents: Default::default(),
            author: author.into(),
            email: String::new(),
            time,
            summary: summary.into(),
        };
        History::from_commits(vec![
            commit(
                "aaaa000000000000000000000000000000000000",
                "feat(search): add ⌘F",
                "Alex",
                3,
            ),
            commit(
                "bbbb000000000000000000000000000000000000",
                "fix: crash",
                "Sam",
                2,
            ),
            commit(
                "cafe000000000000000000000000000000000000",
                "docs: readme",
                "Alex",
                1,
            ),
        ])
    }

    #[test]
    fn matches_message_author_and_sha() {
        let h = history();
        let mut labels = HashMap::new();
        labels.insert(
            h.commits[1].id,
            vec![RefLabel::Branch {
                name: "feature/login".into(),
                head: false,
            }],
        );
        let find = |q| find_commits(&h, &labels, q);
        assert_eq!(find("FEAT search"), [0]);
        assert_eq!(find("alex"), [0, 2]);
        assert_eq!(find("cafe"), [2], "SHA prefix");
        assert_eq!(find("bbbb00"), [1]);
        assert_eq!(find("login"), [1], "branch name");
        assert!(find("  ").is_empty());
        assert!(find("nothing").is_empty());
    }

    #[test]
    fn highlights_every_word() {
        assert_eq!(highlight_ranges("Fix the fix", "FIX"), [0..3, 8..11]);
        assert_eq!(highlight_ranges("abcdef", "bc cd"), vec![1..4]);
        assert!(highlight_ranges("abc", "").is_empty());
    }
}
