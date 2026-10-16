//! Compact history: one row per local branch with how many commits it has
//! on top of the trunk; hovering a row lists those commits. Toggled from the
//! toolbar, with a staggered slide between the two views. The header filters
//! the rows by author and orders them by date or size.

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
use crate::settings::CompactSort;
use crate::{fmt, theme};

/// Commits listed in a row's hover card.
const PREVIEW: usize = 12;
/// Walks stop counting past this many commits.
const CAP: usize = 999;
/// Avatars drawn on a row.
const AUTHORS: usize = 3;
/// The trunk's authors are taken from its most recent commits only.
const TRUNK_AUTHORS_WINDOW: usize = 50;
/// Most active authors offered as filters, besides the user.
const TOP_AUTHORS: usize = 5;

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
    /// Who wrote the branch's own commits (email, name), most commits first,
    /// at most `AUTHORS`; `more_authors` counts the others.
    pub authors: Vec<(String, String)>,
    pub more_authors: usize,
    /// Every author of those commits, by lowercase email.
    pub emails: Vec<String>,
}

/// The compact view's data: one summary per branch, and who writes them.
#[derive(Clone, Debug, Default)]
pub struct Summary {
    pub rows: Vec<BranchSummary>,
    /// Authors of the branches' own commits (email, name, commits), most
    /// commits first; the trunk is left out.
    pub authors: Vec<(String, String, usize)>,
}

/// Distinct authors of `rows` (lowercase email, name, commits), most commits
/// first, ties by the first one met.
fn tally(history: &History, rows: impl IntoIterator<Item = usize>) -> Vec<(String, String, usize)> {
    let mut seen: Vec<(String, String, usize)> = Vec::new();
    for r in rows {
        let c = &history.commits[r];
        let key = c.email.trim().to_lowercase();
        match seen.iter_mut().find(|(e, _, _)| *e == key) {
            Some(entry) => entry.2 += 1,
            None => seen.push((key, c.author.clone(), 1)),
        }
    }
    // Stable: equal counts keep their first (newest) appearance order.
    seen.sort_by_key(|a| std::cmp::Reverse(a.2));
    seen
}

/// Distinct authors of `rows` (by email), most commits first, ties by the
/// most recent one: the first `AUTHORS`, how many others, and every email.
fn authors(history: &History, rows: &[usize]) -> (Vec<(String, String)>, usize, Vec<String>) {
    let seen = tally(history, rows.iter().copied());
    let more = seen.len().saturating_sub(AUTHORS);
    let emails = seen.iter().map(|(e, _, _)| e.clone()).collect();
    let top = seen
        .into_iter()
        .take(AUTHORS)
        .map(|(e, n, _)| (e, n))
        .collect();
    (top, more, emails)
}

/// Branches and the authors of their own commits.
pub fn summary(history: &History, refs: &RefsSnapshot) -> Summary {
    let rows = summarize(history, refs);
    let mut own: Vec<usize> = rows
        .iter()
        .filter(|r| !r.is_trunk)
        .flat_map(|r| r.commits.iter().copied())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    own.sort_unstable();
    let authors = tally(history, own);
    Summary { rows, authors }
}

/// Rows to show: the trunk always, the others only when `author` wrote some
/// of their own commits; by date as summarized, or most commits first.
pub fn arrange<'a>(
    rows: &'a [BranchSummary],
    author: Option<&str>,
    sort: CompactSort,
) -> Vec<&'a BranchSummary> {
    let mut out: Vec<&BranchSummary> = rows
        .iter()
        .filter(|r| r.is_trunk || author.is_none_or(|a| r.emails.iter().any(|e| e == a)))
        .collect();
    if sort == CompactSort::Size {
        out.sort_by_key(|r| {
            (
                !r.is_trunk,
                std::cmp::Reverse(r.count),
                std::cmp::Reverse(r.time),
            )
        });
    }
    out
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
            let window = if is_trunk { TRUNK_AUTHORS_WINDOW } else { CAP };
            let (authors, more_authors, emails) =
                authors(history, &commits[..commits.len().min(window)]);
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
                authors,
                more_authors,
                emails,
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
        self.replay_compact(cx);
    }

    /// Rows slide in again.
    fn replay_compact(&mut self, cx: &mut Context<Self>) {
        self.celebrate_seq += 1;
        self.compact_seq = self.celebrate_seq;
        cx.notify();
    }

    /// Moves the commit details between below the history and beside it.
    pub fn toggle_details_side(&mut self, cx: &mut Context<Self>) {
        self.details_side = !self.details_side;
        let side = self.details_side;
        // Tests run in parallel and share the settings: keep them out of it.
        if !cfg!(test) {
            crate::settings::update(cx, |s| s.details_side = side);
        }
        cx.notify();
    }

    /// Shows only `email`'s branches, or every branch when it already was.
    pub fn filter_compact_author(&mut self, email: String, cx: &mut Context<Self>) {
        self.compact_author = if self.compact_author.as_ref() == Some(&email) {
            None
        } else {
            Some(email)
        };
        self.replay_compact(cx);
    }

    pub fn set_compact_sort(&mut self, sort: CompactSort, cx: &mut Context<Self>) {
        if self.compact_sort == sort {
            return;
        }
        self.compact_sort = sort;
        if !cfg!(test) {
            crate::settings::remember(|s| s.compact_sort = sort);
        }
        self.replay_compact(cx);
    }

    /// Who commits in this repository (lowercase email, name), read once.
    fn compact_me(&mut self) -> Option<(String, String)> {
        self.compact_me
            .get_or_insert_with(|| {
                let id = self.repo.identity();
                let email = id.email.trim().to_lowercase();
                (!email.is_empty()).then_some((email, id.name))
            })
            .clone()
    }

    /// Branch summaries for the loaded data, recomputed when it changes.
    fn compact_rows(&mut self) -> Arc<Summary> {
        if let Some((data, rows)) = &self.compact_cache
            && Arc::ptr_eq(data, &self.data)
        {
            return rows.clone();
        }
        let rows = Arc::new(summary(&self.data.history, &self.data.refs));
        self.compact_cache = Some((self.data.clone(), rows.clone()));
        rows
    }

    /// Filters on the right of the header: the user, the most active
    /// authors, then the order.
    fn compact_filters(&mut self, summary: &Summary, cx: &mut Context<Self>) -> AnyElement {
        let me = self.compact_me();
        let commits_of = |email: &str| {
            summary
                .authors
                .iter()
                .find(|a| a.0 == email)
                .map_or(0, |a| a.2)
        };
        let mut people: Vec<(String, String, usize, bool)> = Vec::new();
        if let Some((email, name)) = &me {
            people.push((email.clone(), name.clone(), commits_of(email), true));
        }
        people.extend(
            summary
                .authors
                .iter()
                .filter(|a| me.as_ref().is_none_or(|m| m.0 != a.0))
                .take(TOP_AUTHORS)
                .map(|(e, n, c)| (e.clone(), n.clone(), *c, false)),
        );
        let others = people.iter().any(|p| !p.3);
        let filter = self.compact_author.clone();
        let sort = self.compact_sort;
        let mut chips = Vec::new();
        for (email, name, count, is_me) in people {
            let on = filter.as_deref() == Some(email.as_str());
            let dim = filter.is_some() && !on;
            let tip = match (is_me, on) {
                (_, true) => tr("Toutes les branches", "All branches").to_string(),
                (true, false) => trf!(
                    "Mes branches · {count} commits",
                    "My branches · {count} commits"
                ),
                (false, false) => trf!(
                    "Branches de {name} · {count} commits",
                    "{name}'s branches · {count} commits"
                ),
            };
            let face = crate::avatars::avatar(&email, &name, 20., cx);
            chips.push(
                div()
                    .id(SharedString::from(format!("compact-author-{email}")))
                    .test_support()
                    .flex_none()
                    .rounded_full()
                    .border_2()
                    .cursor_pointer()
                    .border_color(if on {
                        theme::accent()
                    } else {
                        gpui_kit::transparent_black()
                    })
                    .when(dim, |s| s.opacity(0.35).hover(|s| s.opacity(1.)))
                    .child(face)
                    .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.filter_compact_author(email.clone(), cx)
                    }))
                    .into_any_element(),
            );
            if is_me && others {
                chips.push(
                    div()
                        .flex_none()
                        .w(px(1.))
                        .h(px(14.))
                        .mx_1()
                        .bg(theme::hairline())
                        .into_any_element(),
                );
            }
        }
        let button = |id: &'static str, label: &'static str, value: CompactSort| {
            let on = sort == value;
            h_flex()
                .id(id)
                .test_support()
                .h(px(22.))
                .px_2()
                .rounded_md()
                .cursor_pointer()
                .when(on, |s| {
                    s.bg(theme::alpha(theme::accent(), 0.18))
                        .text_color(theme::accent())
                })
                .when(!on, |s| s.hover(|s| s.bg(theme::raised())))
                .child(label)
                .on_click(cx.listener(move |this, _, _, cx| this.set_compact_sort(value, cx)))
        };
        h_flex()
            .flex_none()
            .gap_1()
            .children(chips)
            .child(div().w(px(10.)))
            .child(
                h_flex()
                    .p(px(2.))
                    .gap_0p5()
                    .rounded_lg()
                    .bg(theme::alpha(gpui_kit::white(), 0.04))
                    .child(button(
                        "compact-sort-date",
                        tr("Date", "Date"),
                        CompactSort::Date,
                    ))
                    .child(button(
                        "compact-sort-size",
                        tr("Taille", "Size"),
                        CompactSort::Size,
                    )),
            )
            .into_any_element()
    }

    pub(super) fn render_compact(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let summary = self.compact_rows();
        let filters = self.compact_filters(&summary, cx);
        let rows = arrange(
            &summary.rows,
            self.compact_author.as_deref(),
            self.compact_sort,
        );
        let nobody = self.compact_author.is_some() && rows.iter().all(|r| r.is_trunk);
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
            let faces = author_faces(&b.authors, b.more_authors, cx);
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
                .child(faces)
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
                    .child(Icon::new(Lucide::Rows3).xsmall().flex_none())
                    .child(div().flex_1().min_w_0().truncate().child(tr(
                        "Vue compacte · survol pour les commits, double-clic pour l'historique",
                        "Compact view · hover for commits, double-click for the history",
                    )))
                    .child(filters),
            )
            .children(items)
            .when(nobody, |s| {
                s.child(
                    div()
                        .px_3()
                        .py_2()
                        .text_xs()
                        .text_color(theme::muted())
                        .child(tr(
                            "Aucune branche de cet auteur.",
                            "No branches by this author.",
                        )),
                )
            })
            .into_any_element()
    }
}

/// Up to three overlapping author pictures, plus “+n” for the others.
fn author_faces(authors: &[(String, String)], more: usize, cx: &mut gpui_kit::App) -> AnyElement {
    const SIZE: f32 = 20.;
    h_flex()
        .w(px(84.))
        .flex_none()
        .justify_end()
        .children(authors.iter().enumerate().map(|(i, (email, name))| {
            div()
                .flex_none()
                .rounded_full()
                .border_2()
                .border_color(theme::panel())
                .when(i > 0, |s| s.ml(px(-7.)))
                .child(crate::avatars::avatar(email, name, SIZE, cx))
        }))
        .when(more > 0, |s| {
            s.child(
                div()
                    .pl_1()
                    .text_xs()
                    .text_color(theme::muted())
                    .child(format!("+{more}")),
            )
        })
        .into_any_element()
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

    #[test]
    fn filters_by_author_and_sorts_by_size() {
        let row =
            |name: &str, count: usize, time: i64, trunk: bool, emails: &[&str]| BranchSummary {
                name: name.into(),
                tip: id(1),
                is_head: false,
                is_trunk: trunk,
                remote: false,
                count,
                ahead: 0,
                behind: 0,
                commits: Vec::new(),
                time,
                authors: Vec::new(),
                more_authors: 0,
                emails: emails.iter().map(|e| e.to_string()).collect(),
            };
        let rows = vec![
            row("main", 50, 9, true, &["bob@x.dev"]),
            row("new", 1, 8, false, &["ann@x.dev"]),
            row("big", 7, 5, false, &["ann@x.dev", "bob@x.dev"]),
            row("mid", 3, 2, false, &["bob@x.dev"]),
        ];
        let names = |v: Vec<&BranchSummary>| v.iter().map(|r| r.name.clone()).collect::<Vec<_>>();
        assert_eq!(
            names(arrange(&rows, None, CompactSort::Date)),
            ["main", "new", "big", "mid"]
        );
        assert_eq!(
            names(arrange(&rows, None, CompactSort::Size)),
            ["main", "big", "mid", "new"]
        );
        assert_eq!(
            names(arrange(&rows, Some("ann@x.dev"), CompactSort::Size)),
            ["main", "big", "new"]
        );
    }

    #[test]
    fn lists_at_most_three_authors_most_commits_first() {
        let by = |n: u8, who: &str| Commit {
            author: who.to_uppercase(),
            email: format!("{who}@x.dev"),
            ..commit(n, if n > 1 { &[0] } else { &[] })
        };
        let commits = vec![
            by(6, "ann"),
            by(5, "bob"),
            by(4, "cid"),
            by(3, "bob"),
            by(2, "dan"),
            by(1, "cid"),
        ];
        let index = commits.iter().enumerate().map(|(i, c)| (c.id, i)).collect();
        let history = History { commits, index };
        let (top, more, all) = authors(&history, &[0, 1, 2, 3, 4, 5]);
        let emails: Vec<&str> = top.iter().map(|(e, _)| e.as_str()).collect();
        assert_eq!(emails, ["bob@x.dev", "cid@x.dev", "ann@x.dev"]);
        assert_eq!(top[0].1, "BOB");
        assert_eq!(more, 1);
        assert_eq!(all.len(), 4);
    }
}
