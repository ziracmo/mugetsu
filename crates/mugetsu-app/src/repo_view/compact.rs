//! Compact history: one row per local branch with how many commits it has
//! on top of the trunk; hovering a row lists those commits. Toggled from the
//! toolbar, with a staggered slide between the two views.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use gix::ObjectId;
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    Animation, AnimationExt as _, AnyElement, Context, InteractiveElement as _, IntoElement,
    ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _, div,
    prelude::FluentBuilder as _, px,
};
use mugetsu_git::{History, RefsSnapshot};

use super::RepoView;
use crate::i18n::tr;
use crate::{fmt, theme};

/// Commits listed in a row's hover card.
const PREVIEW: usize = 12;
/// Walks stop counting past this many commits.
const CAP: usize = 999;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BranchSummary {
    pub name: String,
    pub tip: ObjectId,
    pub is_head: bool,
    pub is_trunk: bool,
    /// Remote branch with no local branch tracking it.
    pub remote: bool,
    /// Commits counted (for the trunk, its whole loaded history).
    pub count: usize,
    pub ahead: u32,
    pub behind: u32,
    /// Commits only on this branch (for the trunk: its own history), newest
    /// first, as history rows; at most `CAP`.
    pub commits: Vec<usize>,
    /// Committer time of the tip.
    pub time: i64,
}

/// The trunk tip: the remote's main / master, else a local one, else HEAD.
pub fn trunk(refs: &RefsSnapshot) -> Option<(String, ObjectId)> {
    let find = |list: &[mugetsu_git::Branch], name: &str| {
        list.iter()
            .find(|b| b.name == name)
            .map(|b| (b.name.clone(), b.target))
    };
    ["origin/main", "origin/master"]
        .iter()
        .find_map(|n| find(&refs.remote_branches, n))
        .or_else(|| {
            ["main", "master", "develop"]
                .iter()
                .find_map(|n| find(&refs.branches, n))
        })
        .or_else(|| refs.head.map(|h| ("HEAD".into(), h)))
}

/// History rows reachable from `tip`, newest first, not crossing `stop`.
fn walk(history: &History, tip: ObjectId, stop: &HashSet<usize>, cap: usize) -> Vec<usize> {
    match history.index.get(&tip) {
        Some(&row) => walk_rows(history, row, stop, cap),
        None => Vec::new(),
    }
}

/// [`walk`] from a history row.
pub(super) fn walk_rows(
    history: &History,
    start: usize,
    stop: &HashSet<usize>,
    cap: usize,
) -> Vec<usize> {
    let mut seen = HashSet::new();
    let mut stack = vec![start];
    let mut out = Vec::new();
    while let Some(row) = stack.pop() {
        if stop.contains(&row) || !seen.insert(row) {
            continue;
        }
        out.push(row);
        if out.len() >= cap {
            break;
        }
        for p in &history.commits[row].parents {
            if let Some(&r) = history.index.get(p) {
                stack.push(r);
            }
        }
    }
    // Rows are in display order: newest first.
    out.sort_unstable();
    out
}

/// One summary per local branch: HEAD first, then most recent first.
pub fn summarize(history: &History, refs: &RefsSnapshot) -> Vec<BranchSummary> {
    let trunk = trunk(refs);
    let trunk_rows: HashSet<usize> = trunk
        .as_ref()
        .map(|(_, tip)| walk(history, *tip, &HashSet::new(), usize::MAX))
        .unwrap_or_default()
        .into_iter()
        .collect();
    let trunk_name = trunk.as_ref().map(|(n, _)| n.as_str());
    let tracked: HashSet<&str> = refs
        .branches
        .iter()
        .filter_map(|b| b.upstream.as_deref())
        .collect();
    let remotes = refs
        .remote_branches
        .iter()
        .filter(|b| !b.name.ends_with("/HEAD") && !tracked.contains(b.name.as_str()));
    let mut out: Vec<BranchSummary> = refs
        .branches
        .iter()
        .map(|b| (b, false))
        .chain(remotes.map(|b| (b, true)))
        .map(|(b, remote)| {
            let is_trunk = trunk_name.is_some_and(|t| {
                t == b.name || (!remote && t.split_once('/').is_some_and(|(_, n)| n == b.name))
            });
            let commits = if is_trunk {
                walk(history, b.target, &HashSet::new(), CAP)
            } else {
                walk(history, b.target, &trunk_rows, CAP)
            };
            BranchSummary {
                name: b.name.clone(),
                tip: b.target,
                is_head: b.is_head,
                is_trunk,
                remote,
                count: if is_trunk {
                    trunk_rows.len()
                } else {
                    commits.len()
                },
                ahead: b.ahead,
                behind: b.behind,
                commits,
                time: history
                    .index
                    .get(&b.target)
                    .map(|&r| history.commits[r].time)
                    .unwrap_or_default(),
            }
        })
        // A remote branch with nothing of its own is noise.
        .filter(|s| !s.remote || s.is_trunk || s.count > 0)
        .collect();
    out.sort_by_key(|s| (!s.is_head, !s.is_trunk, s.remote, std::cmp::Reverse(s.time)));
    out
}

impl RepoView {
    /// Flips between the full history and the compact branch list.
    pub fn toggle_compact(&mut self, cx: &mut Context<Self>) {
        self.compact = !self.compact;
        let compact = self.compact;
        // Tests run in parallel and share the settings: keep them out of it.
        if !cfg!(test) {
            crate::settings::remember(|s| s.history_compact = compact);
        }
        self.celebrate_seq += 1;
        self.compact_seq = self.celebrate_seq;
        cx.notify();
    }

    /// Branch summaries for the loaded data, recomputed when it changes.
    fn compact_rows(&mut self) -> Arc<Vec<BranchSummary>> {
        if let Some((data, rows)) = &self.compact_cache
            && Arc::ptr_eq(data, &self.data)
        {
            return rows.clone();
        }
        let rows = Arc::new(summarize(&self.data.history, &self.data.refs));
        self.compact_cache = Some((self.data.clone(), rows.clone()));
        rows
    }

    pub(super) fn render_compact(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let rows = self.compact_rows();
        let data = self.data.clone();
        let seq = self.compact_seq;
        let now = fmt::now();
        let selected = self.selected;
        let max = rows
            .iter()
            .filter(|r| !r.is_trunk)
            .map(|r| r.count)
            .max()
            .unwrap_or(1)
            .max(1);
        let items = rows.iter().enumerate().map(|(i, b)| {
            let n = b.count;
            let count = if n >= CAP && !b.is_trunk {
                format!("{CAP}+")
            } else {
                n.to_string()
            };
            let label = if b.is_trunk {
                trf!("{count} commits", "{count} commits")
            } else {
                trf!("+{count}", "+{count}")
            };
            let color = if b.is_head {
                theme::accent()
            } else if b.is_trunk {
                theme::violet()
            } else if b.remote {
                theme::magenta()
            } else {
                theme::lime()
            };
            let fill = if b.is_trunk {
                1.
            } else {
                n as f32 / max as f32
            };
            let min_fill = if n == 0 { 0. } else { 0.02 };
            let tip = b.tip;
            let active = selected == Some(tip);
            let preview: Vec<(String, String, String)> = b
                .commits
                .iter()
                .take(PREVIEW)
                .map(|&r| {
                    let c = &data.history.commits[r];
                    (
                        c.short_id(),
                        c.summary.clone(),
                        fmt::relative_time(c.time, now),
                    )
                })
                .collect();
            let more = n.saturating_sub(PREVIEW);
            let remote = b.remote;
            let title = b.name.clone();
            let delay = i.min(14) as f32 * 28.;
            h_flex()
                .id(SharedString::from(format!("compact-{}", b.name)))
                .test_support()
                .w_full()
                .h(px(36.))
                .px_3()
                .gap_3()
                .rounded_lg()
                .cursor_pointer()
                .when(active, |s| s.bg(theme::alpha(theme::accent(), 0.16)))
                .when(!active, |s| s.hover(|s| s.bg(theme::raised())))
                .child(
                    div()
                        .size(px(8.))
                        .flex_none()
                        .rounded_full()
                        .bg(color)
                        .when(b.is_head, |s| {
                            s.shadow(vec![gpui_kit::BoxShadow {
                                color: theme::alpha(color, 0.6),
                                offset: gpui_kit::point(px(0.), px(0.)),
                                blur_radius: px(6.),
                                spread_radius: px(1.),
                                inset: false,
                            }])
                        }),
                )
                .child(
                    h_flex()
                        .w(px(260.))
                        .flex_none()
                        .min_w_0()
                        .gap_1p5()
                        .when(remote, |s| {
                            s.child(Icon::new(Lucide::Cloud).xsmall().text_color(theme::muted()))
                        })
                        .child(
                            div()
                                .min_w_0()
                                .truncate()
                                .text_sm()
                                .when(b.is_head, |s| s.font_weight(gpui_kit::FontWeight::SEMIBOLD))
                                .child(b.name.clone()),
                        )
                        .when(b.ahead > 0, |s| {
                            s.child(
                                div()
                                    .text_xs()
                                    .text_color(theme::lime())
                                    .child(format!("↑{}", b.ahead)),
                            )
                        })
                        .when(b.behind > 0, |s| {
                            s.child(
                                div()
                                    .text_xs()
                                    .text_color(theme::amber())
                                    .child(format!("↓{}", b.behind)),
                            )
                        }),
                )
                .child(
                    // How much the branch carries, relative to the biggest one.
                    div()
                        .flex_1()
                        .h(px(6.))
                        .rounded_full()
                        .bg(theme::alpha(gpui_kit::white(), 0.05))
                        .child(
                            div()
                                .h_full()
                                .rounded_full()
                                .bg(gpui_kit::linear_gradient(
                                    90.,
                                    gpui_kit::linear_color_stop(theme::alpha(color, 0.25), 0.),
                                    gpui_kit::linear_color_stop(color, 1.),
                                ))
                                .with_animation(
                                    ("compact-bar", seq * 4096 + i),
                                    Animation::new(Duration::from_millis(520 + delay as u64)),
                                    move |el, t| {
                                        let k = ((t * (520. + delay) - delay) / 520.).clamp(0., 1.);
                                        let k = 1. - (1. - k).powi(3);
                                        el.w(gpui_kit::relative(fill.max(min_fill) * k))
                                    },
                                ),
                        ),
                )
                .child(
                    div()
                        .w(px(90.))
                        .flex_none()
                        .text_right()
                        .text_xs()
                        .text_color(if n == 0 { theme::muted() } else { color })
                        .child(label),
                )
                .child(
                    div()
                        .w(px(70.))
                        .flex_none()
                        .text_right()
                        .text_xs()
                        .text_color(theme::muted())
                        .child(fmt::relative_time(b.time, now)),
                )
                .tooltip_show_delay(super::pr_tip::DELAY)
                .tooltip(move |window, cx| {
                    let (title, preview) = (title.clone(), preview.clone());
                    Tooltip::element(move |_, _| commits_card(&title, &preview, more))
                        .build(window, cx)
                })
                .on_click(cx.listener(move |this, ev: &gpui_kit::ClickEvent, _, cx| {
                    this.select_commit(tip, false, cx);
                    // Double-click: back to the full history on that tip.
                    if ev.click_count() >= 2 {
                        this.toggle_compact(cx);
                        this.select_commit(tip, true, cx);
                    }
                }))
                .with_animation(
                    ("compact-row", seq * 4096 + i),
                    Animation::new(Duration::from_millis(320 + delay as u64)),
                    move |el, t| {
                        let k = ((t * (320. + delay) - delay) / 320.).clamp(0., 1.);
                        let k = 1. - (1. - k).powi(3);
                        el.opacity(k).mt(px(10. * (1. - k)))
                    },
                )
        });
        v_flex()
            .id("compact-history")
            .test_support()
            .size_full()
            .overflow_y_scroll()
            .p_2()
            .gap_0p5()
            .child(
                h_flex()
                    .px_3()
                    .pb_1()
                    .gap_2()
                    .text_xs()
                    .text_color(theme::muted())
                    .child(Icon::new(Lucide::Rows3).xsmall())
                    .child(tr(
                        "Vue compacte · survol pour les commits, double-clic pour l'historique",
                        "Compact view · hover for commits, double-click for the history",
                    )),
            )
            .children(items)
            .into_any_element()
    }
}

/// Hover card: the branch's own commits.
fn commits_card(title: &str, commits: &[(String, String, String)], more: usize) -> AnyElement {
    v_flex()
        .w(px(420.))
        .py_1()
        .gap_1()
        .text_xs()
        .child(
            div()
                .text_sm()
                .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                .pb_1()
                .child(title.to_string()),
        )
        .when(commits.is_empty(), |s| {
            s.child(div().text_color(theme::muted()).child(tr(
                "Aucun commit propre à cette branche.",
                "No commits of its own.",
            )))
        })
        .children(commits.iter().map(|(sha, summary, when)| {
            h_flex()
                .gap_2()
                .child(
                    div()
                        .flex_none()
                        .text_color(theme::accent())
                        .child(sha.clone()),
                )
                .child(div().flex_1().min_w_0().truncate().child(summary.clone()))
                .child(
                    div()
                        .flex_none()
                        .text_color(theme::muted())
                        .child(when.clone()),
                )
        }))
        .when(more > 0, |s| {
            s.child(
                div()
                    .text_color(theme::muted())
                    .child(trf!("… et {more} de plus", "… and {more} more")),
            )
        })
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use mugetsu_git::{Branch, Commit};

    fn id(n: u8) -> ObjectId {
        ObjectId::from_bytes_or_panic(&[n; 20])
    }

    fn commit(n: u8, parents: &[u8]) -> Commit {
        Commit {
            id: id(n),
            parents: parents.iter().map(|p| id(*p)).collect(),
            author: String::new(),
            email: String::new(),
            time: n as i64,
            summary: format!("c{n}"),
        }
    }

    fn branch(name: &str, tip: u8, head: bool) -> Branch {
        Branch {
            name: name.into(),
            target: id(tip),
            upstream: None,
            ahead: 0,
            behind: 0,
            is_head: head,
            upstream_gone: false,
        }
    }

    #[test]
    fn counts_the_commits_each_branch_adds_to_the_trunk() {
        // main: 1 ← 2 ← 3; topic: 2 ← 4 ← 5; merged: 3 (already in main).
        let commits = vec![
            commit(5, &[4]),
            commit(4, &[2]),
            commit(3, &[2]),
            commit(2, &[1]),
            commit(1, &[]),
        ];
        let index = commits.iter().enumerate().map(|(i, c)| (c.id, i)).collect();
        let history = History { commits, index };
        let refs = RefsSnapshot {
            head: Some(id(5)),
            branches: vec![
                branch("main", 3, false),
                branch("topic", 5, true),
                branch("old", 3, false),
            ],
            ..Default::default()
        };
        let rows = summarize(&history, &refs);
        let shown: Vec<(&str, usize)> = rows.iter().map(|r| (r.name.as_str(), r.count)).collect();
        assert_eq!(shown, [("topic", 2), ("main", 3), ("old", 0)]);
        assert_eq!(rows[0].commits, vec![0, 1]);
    }
}
