use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::time::Duration;

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::diff::Diff;
use gpui_kit::component::menu::ContextMenuExt as _;
use gpui_kit::component::scroll::Scrollbar;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    Animation, AnimationExt as _, AnyElement, ClipboardItem, Context, InteractiveElement as _,
    IntoElement, ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _,
    Window, div, prelude::FluentBuilder as _, px, uniform_list,
};
use mugetsu_git::RefLabel;

use super::menus;
use super::{Mode, RepoView, SelectNewer, SelectOlder};
use crate::fmt;
use crate::graph_view::{self, NodeStyle};
use crate::i18n::tr;
use crate::theme;

pub const ROW_HEIGHT: f32 = 26.;

/// What the history needs to draw ref chips, captured once per frame.
struct ChipContext {
    /// Local branch → its upstream (`origin/x`).
    upstreams: HashMap<String, String>,
    github: bool,
    view: gpui_kit::WeakEntity<RepoView>,
    /// ⌘F query, to ring the chips whose name matches.
    search: String,
    /// Branches checked out in another worktree.
    worktrees: HashMap<String, std::path::PathBuf>,
    /// Branch just pushed (name, sequence).
    pushed: Option<(String, usize)>,
    /// Branch just deleted (name, sequence).
    slashed: Option<(String, usize)>,
}

/// Rings a chip whose name matches the ⌘F query.
fn mark_match(el: AnyElement, name: &str, ctx: &ChipContext) -> AnyElement {
    if ctx.search.is_empty() || super::search::highlight_ranges(name, &ctx.search).is_empty() {
        return el;
    }
    div()
        .flex_none()
        .rounded_md()
        .border_1()
        .border_color(theme::amber())
        .shadow_sm()
        .child(el)
        .into_any_element()
}

/// Double-click on a branch chip checks it out (with a dialog when needed).
fn checkout_on_double_click(
    el: gpui_kit::Div,
    name: &str,
    is_remote: bool,
    ctx: &ChipContext,
) -> AnyElement {
    let (view, branch) = (ctx.view.clone(), name.to_string());
    el.id(SharedString::from(format!("chip-{name}")))
        .test_support()
        .cursor_pointer()
        .on_click(move |ev, window, cx| {
            if ev.click_count() >= 2 {
                cx.stop_propagation();
                let branch = branch.clone();
                let _ = view.update(cx, |this, cx| {
                    this.request_checkout(branch, is_remote, window, cx)
                });
            }
        })
        .into_any_element()
}

fn chip(
    text: String,
    color: gpui_kit::Hsla,
    icon: Lucide,
    head: bool,
    pushed: Option<Lucide>,
    worktree: bool,
) -> gpui_kit::Div {
    let base = h_flex()
        .flex_none()
        .max_w(px(240.))
        .h(px(18.))
        .rounded_md()
        .overflow_hidden()
        .border_1()
        .text_xs();
    let base = if head {
        // The checked-out branch: solid accent, white, glowing edge.
        base.border_color(theme::alpha(gpui_kit::white(), 0.55))
            .bg(theme::accent())
            .text_color(gpui_kit::white())
            .font_weight(gpui_kit::FontWeight::BOLD)
            .shadow_md()
    } else {
        base.border_color(theme::alpha(color, 0.45))
            .bg(theme::alpha(color, 0.12))
            .text_color(color)
    };
    base.map(|s| match pushed {
        // Pushed and identical on the remote: dark icon cell, one chip for both.
        Some(remote_icon) => s.child(
            div()
                .h_full()
                .px_1()
                .flex()
                .items_center()
                .bg(gpui_kit::rgba(0x000000b3))
                .child(Icon::new(remote_icon).xsmall().text_color(if head {
                    gpui_kit::white()
                } else {
                    color
                })),
        ),
        None => s.child(div().pl_1p5().child(Icon::new(icon).xsmall())),
    })
    // Checked out in another worktree: same dark cell, worktree icon.
    .when(worktree, |s| {
        s.child(
            div()
                .h_full()
                .px_1()
                .flex()
                .items_center()
                .bg(gpui_kit::rgba(0x000000b3))
                .child(Icon::new(Lucide::FolderTree).xsmall().text_color(if head {
                    gpui_kit::white()
                } else {
                    color
                })),
        )
    })
    .child(div().px_1().truncate().child(text))
}

fn chips(labels: &[RefLabel], ctx: &ChipContext) -> Vec<AnyElement> {
    let remotes: HashSet<&str> = labels
        .iter()
        .filter_map(|l| match l {
            RefLabel::Remote(n) => Some(n.as_str()),
            _ => None,
        })
        .collect();
    let mut merged = HashSet::new();
    let mut out = Vec::new();
    for label in labels {
        // A deleted branch's chip is replaced by its slash animation.
        let mut return_slash: Option<AnyElement> = None;
        let element = match label {
            RefLabel::Branch { name, head } => {
                let synced = ctx
                    .upstreams
                    .get(name)
                    .filter(|u| remotes.contains(u.as_str()));
                if let Some(u) = synced {
                    merged.insert(u.clone());
                }
                let color = if *head {
                    theme::accent()
                } else {
                    theme::lime()
                };
                let remote_icon = if ctx.github {
                    Lucide::Github
                } else {
                    Lucide::Cloud
                };
                let el = checkout_on_double_click(
                    chip(
                        name.clone(),
                        color,
                        if *head {
                            Lucide::Check
                        } else {
                            Lucide::GitBranch
                        },
                        *head,
                        synced.map(|_| remote_icon),
                        ctx.worktrees.contains_key(name),
                    ),
                    name,
                    false,
                    ctx,
                );
                if let Some((_, seq)) = ctx.slashed.as_ref().filter(|(b, _)| b == name) {
                    let half = || chip(name.clone(), color, Lucide::GitBranch, *head, None, false);
                    return_slash = Some(super::shimmer::slash(half, *seq));
                }
                match &ctx.pushed {
                    Some((b, seq)) if b == name => super::shimmer::pushed(el, *seq),
                    _ => el,
                }
            }
            RefLabel::Remote(_) => continue,
            RefLabel::Tag(name) => chip(
                name.clone(),
                theme::amber(),
                Lucide::Tag,
                false,
                None,
                false,
            )
            .into_any_element(),
            RefLabel::DetachedHead => chip(
                "HEAD".into(),
                theme::magenta(),
                Lucide::CircleDot,
                false,
                None,
                false,
            )
            .into_any_element(),
        };
        let element = match super::search::label_name(label) {
            Some(name) => mark_match(element, name, ctx),
            None => element,
        };
        let element = return_slash.take().unwrap_or(element);
        out.push(element);
    }
    // Remote branches not merged into a local chip (ahead/behind or remote-only).
    for label in labels {
        if let RefLabel::Remote(name) = label
            && !merged.contains(name)
        {
            let el = checkout_on_double_click(
                chip(
                    name.clone(),
                    theme::violet(),
                    Lucide::Cloud,
                    false,
                    None,
                    false,
                ),
                name,
                true,
                ctx,
            )
            .into_any_element();
            out.push(mark_match(el, name, ctx));
        }
    }
    out
}

impl RepoView {
    fn render_wip_banner(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let n = self.data.uncommitted();
        (n > 0).then(|| {
            h_flex()
                .id("wip")
                .test_support()
                .h(px(28.))
                .flex_none()
                .px_3()
                .gap_2()
                .cursor_pointer()
                .border_b_1()
                .border_color(cx.theme().border)
                .bg(theme::alpha(theme::amber(), 0.06))
                .hover(|s| s.bg(theme::alpha(theme::amber(), 0.12)))
                .child(
                    div()
                        .size(px(8.))
                        .rounded_full()
                        .bg(theme::amber())
                        .with_animation(
                            "wip-pulse",
                            Animation::new(Duration::from_millis(1400))
                                .repeat()
                                .with_easing(gpui_kit::pulsating_between(0.35, 1.)),
                            |el, t| el.opacity(t),
                        ),
                )
                .child(div().text_sm().text_color(theme::amber()).child(trf!(
                    "{n} fichier(s) modifié(s) non commité(s)",
                    "{n} uncommitted changed file(s)"
                )))
                .child(div().flex_1())
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(tr("Voir les changements →", "View changes →")),
                )
                .on_click(cx.listener(|this, _, _, cx| this.set_mode(Mode::Changes, cx)))
                .into_any_element()
        })
    }

    pub(super) fn render_history(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let data = self.data.clone();
        let selected = self.selected;
        let head = data.refs.head;
        let entity = cx.entity().downgrade();
        let now = fmt::now();
        let focused = self.history_focus.is_focused(window);
        let menu_ctx = self.menu_context(cx);
        let branches: Rc<Vec<_>> = Rc::new(
            data.refs
                .branches
                .iter()
                .map(|b| self.branch_menu_info(b))
                .collect(),
        );
        let remote_prs: Rc<Vec<_>> = Rc::new(
            data.refs
                .remote_branches
                .iter()
                .map(|b| {
                    (
                        b.name.clone(),
                        menus::found_pr(self.pr_for(&b.name, None, true)),
                    )
                })
                .collect(),
        );

        let search = if self.search_open {
            self.search_input.read(cx).value().trim().to_string()
        } else {
            String::new()
        };
        let highlight = gpui_kit::HighlightStyle {
            background_color: Some(theme::alpha(theme::amber(), 0.4)),
            color: Some(gpui_kit::white()),
            ..Default::default()
        };
        let chip_ctx = Rc::new(ChipContext {
            upstreams: data
                .refs
                .branches
                .iter()
                .filter_map(|b| Some((b.name.clone(), b.upstream.clone()?)))
                .collect(),
            github: data.github.is_some(),
            view: cx.entity().downgrade(),
            search: search.clone(),
            worktrees: self.other_worktrees(),
            pushed: self.pushed.clone(),
            slashed: self.slashed.clone(),
        });
        let hits = Rc::new(self.search_hits.clone());
        let shimmer = self.shimmer.clone();
        let celebrate = self.celebrate;
        let list = if data.history.is_empty() {
            v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap_3()
                .text_color(cx.theme().muted_foreground)
                .when(self.loading, |s| {
                    s.child(Spinner::new().color(theme::accent()))
                        .child(tr("Lecture de l'historique…", "Reading history…"))
                })
                .when(!self.loading, |s| s.child(tr("Aucun commit", "No commits")))
                .into_any_element()
        } else {
            uniform_list("history", data.history.len(), move |range, _, cx| {
                // The graph column adapts to the lanes actually visible.
                let rows: Vec<_> = range.clone().map(|ix| data.graph.row(ix)).collect();
                let graph_width =
                    graph_view::column_width(rows.iter().map(|r| r.width).max().unwrap_or(1));
                range
                    .zip(rows)
                    .map(|(ix, row)| {
                        let commit = &data.history.commits[ix];
                        let is_selected = selected == Some(commit.id);
                        let style = NodeStyle {
                            is_head: head == Some(commit.id),
                            is_merge: commit.parents.len() > 1,
                            selected: is_selected,
                        };
                        let labels = data
                            .decorations
                            .get(&commit.id)
                            .map(|l| chips(l, &chip_ctx))
                            .unwrap_or_default();
                        let is_hit = hits.contains(&ix);
                        let id = commit.id;
                        let entity = entity.clone();
                        let is_merge = commit.parents.len() > 1;
                        let ref_labels = data
                            .decorations
                            .get(&commit.id)
                            .cloned()
                            .unwrap_or_default();
                        let (menu_ctx, branches, remote_prs) =
                            (menu_ctx.clone(), branches.clone(), remote_prs.clone());
                        h_flex()
                            .id(SharedString::from(commit.id.to_string()))
                            .test_support()
                            .h(px(ROW_HEIGHT))
                            .w_full()
                            .pr_3()
                            .gap_2()
                            .text_sm()
                            .cursor_pointer()
                            .hover(|s| s.bg(theme::raised()))
                            .when(is_hit && !is_selected, |s| {
                                s.bg(theme::alpha(theme::amber(), 0.08))
                            })
                            .when(is_selected, |s| {
                                s.bg(theme::alpha(
                                    theme::accent(),
                                    if focused { 0.16 } else { 0.1 },
                                ))
                            })
                            .when_some(
                                celebrate.filter(|(c, _)| *c == id).map(|(_, seq)| seq),
                                |s, seq| {
                                    s.relative()
                                        .overflow_hidden()
                                        .child(super::shimmer::checkout(seq))
                                },
                            )
                            .when_some(
                                shimmer
                                    .as_ref()
                                    .filter(|(ids, _)| ids.contains(&id))
                                    .map(|(_, seq)| *seq),
                                |s, seq| {
                                    s.relative()
                                        .overflow_hidden()
                                        .child(super::shimmer::sweep(seq))
                                },
                            )
                            .child(
                                div()
                                    .flex_none()
                                    .w(graph_width)
                                    .h_full()
                                    .child(graph_view::graph_cell(row, style)),
                            )
                            .child(
                                h_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .gap_1p5()
                                    .children(labels)
                                    .child(
                                        div()
                                            .min_w_0()
                                            .truncate()
                                            .when(commit.parents.len() > 1, |s| {
                                                s.text_color(cx.theme().muted_foreground)
                                            })
                                            .child({
                                                let ranges = if is_hit {
                                                    super::search::highlight_ranges(
                                                        &commit.summary,
                                                        &search,
                                                    )
                                                } else {
                                                    Vec::new()
                                                };
                                                gpui_kit::StyledText::new(commit.summary.clone())
                                                    .with_highlights(
                                                        ranges.into_iter().map(|r| (r, highlight)),
                                                    )
                                            }),
                                    ),
                            )
                            .child(
                                h_flex()
                                    .flex_none()
                                    .w(px(150.))
                                    .gap_1p5()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(crate::avatars::avatar(
                                        &commit.email,
                                        &commit.author,
                                        16.,
                                        cx,
                                    ))
                                    .child(div().min_w_0().truncate().child(commit.author.clone())),
                            )
                            .child(
                                div()
                                    .flex_none()
                                    .w(px(56.))
                                    .text_xs()
                                    .text_right()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(fmt::relative_time(commit.time, now)),
                            )
                            .child(
                                div()
                                    .flex_none()
                                    .w(px(64.))
                                    .text_xs()
                                    .font_family(cx.theme().mono_font_family.clone())
                                    .text_color(theme::alpha(theme::accent(), 0.7))
                                    .child(commit.id.to_hex_with_len(7).to_string()),
                            )
                            .on_click(move |_, window, cx| {
                                let _ = entity.update(cx, |this, cx| {
                                    this.history_focus.focus(window, cx);
                                    this.select_commit(id, false, cx);
                                });
                            })
                            .context_menu(move |m, window, cx| {
                                menus::commit_menu(
                                    m,
                                    &menu_ctx,
                                    id,
                                    is_merge,
                                    &ref_labels,
                                    &branches,
                                    &remote_prs,
                                    window,
                                    cx,
                                )
                            })
                            .into_any_element()
                    })
                    .collect()
            })
            .size_full()
            .track_scroll(&self.history_scroll)
            .into_any_element()
        };

        v_flex()
            .size_full()
            .min_h_0()
            .children(self.render_wip_banner(cx))
            .child(
                div()
                    .id("history-list")
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .track_focus(&self.history_focus)
                    .key_context("History")
                    .on_action(
                        cx.listener(|this, _: &SelectNewer, _, cx| this.move_selection(-1, cx)),
                    )
                    .on_action(
                        cx.listener(|this, _: &SelectOlder, _, cx| this.move_selection(1, cx)),
                    )
                    .child(list)
                    .child(Scrollbar::vertical(&self.history_scroll)),
            )
    }

    pub(super) fn render_commit_details(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(commit) = self
            .selected
            .and_then(|id| self.data.history.row_of(&id))
            .map(|r| self.data.history.commits[r].clone())
        else {
            return v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .text_color(cx.theme().muted_foreground)
                .child(tr("Sélectionne un commit", "Select a commit"))
                .into_any_element();
        };
        let sha = commit.id.to_string();
        let message = if self.commit_message.is_empty() {
            SharedString::from(commit.summary.clone())
        } else {
            self.commit_message.clone()
        };
        let (subject, body) = message
            .split_once('\n')
            .map_or((message.to_string(), String::new()), |(s, b)| {
                (s.to_string(), b.trim().to_string())
            });
        v_flex()
            .size_full()
            .min_h_0()
            .border_t_1()
            .border_color(cx.theme().border)
            .child(
                v_flex()
                    .flex_none()
                    .px_4()
                    .py_2()
                    .gap_1()
                    .bg(theme::panel())
                    .border_b_1()
                    .border_color(theme::hairline())
                    .child(
                        div()
                            .text_base()
                            .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                            .child(subject),
                    )
                    // The description stays hidden until asked for.
                    .when(!body.is_empty(), |s| {
                        let lines = body.lines().count();
                        let expanded = self.body_expanded;
                        s.child(
                            h_flex()
                                .id("commit-body-toggle")
                                .test_support()
                                .gap_1()
                                .text_xs()
                                .cursor_pointer()
                                .text_color(theme::muted())
                                .hover(|s| s.text_color(cx.theme().foreground))
                                .child(
                                    Icon::new(if expanded {
                                        Lucide::ChevronDown
                                    } else {
                                        Lucide::ChevronRight
                                    })
                                    .xsmall(),
                                )
                                .child(if expanded {
                                    tr("Masquer la description", "Hide description").to_string()
                                } else {
                                    trf!("Description ({} lignes)", "Description ({} lines)", lines)
                                })
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.body_expanded = !this.body_expanded;
                                    cx.notify();
                                })),
                        )
                        .when(expanded, |s| {
                            s.child(
                                div()
                                    .id("commit-body")
                                    .test_support()
                                    .max_h(px(220.))
                                    .overflow_y_scroll()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(body.clone()),
                            )
                        })
                    })
                    .child(
                        h_flex()
                            .gap_3()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(
                                h_flex()
                                    .gap_1p5()
                                    .child(crate::avatars::avatar(
                                        &commit.email,
                                        &commit.author,
                                        20.,
                                        cx,
                                    ))
                                    .child(format!("{} <{}>", commit.author, commit.email)),
                            )
                            .child(fmt::absolute_time(commit.time))
                            .child(self.render_copy_sha(sha, &commit, cx))
                            .children(self.render_commit_link(cx))
                            .child(div().flex_1())
                            .child(trf!("{} fichier(s)", "{} file(s)", self.commit_files)),
                    ),
            )
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .when(self.commit_files > 0, |s| {
                        s.child(self.render_commit_files(cx))
                    })
                    .child(
                        div()
                            .relative()
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .p_2()
                            .child(
                                Diff::new(&self.commit_diff)
                                    .line_number(true)
                                    .when_some(self.file_link_renderer(cx), |d, render| {
                                        d.render_header_suffix(render)
                                    }),
                            )
                            .children(self.render_lines_link(cx)),
                    ),
            )
            .into_any_element()
    }

    /// The short SHA; a click copies the full one, with a "Copied" burst.
    fn render_copy_sha(
        &self,
        sha: String,
        commit: &mugetsu_git::Commit,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let copied = self.copied;
        h_flex()
            .id("copy-sha")
            .test_support()
            .relative()
            .gap_1()
            .cursor_pointer()
            .font_family(cx.theme().mono_font_family.clone())
            .text_color(theme::accent())
            .hover(|s| s.text_color(theme::magenta()))
            .map(|s| match copied {
                Some(seq) => s
                    .text_color(theme::lime())
                    .child(Icon::new(Lucide::Check).xsmall())
                    .child(
                        div()
                            .font_family(cx.theme().font_family.clone())
                            .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                            .child(tr("Copié !", "Copied!"))
                            .with_animation(
                                ("copied", seq),
                                Animation::new(Duration::from_millis(260))
                                    .with_easing(gpui_kit::ease_out_quint()),
                                |el, t| el.opacity(t).ml(px(6. * (1. - t))),
                            ),
                    )
                    .child(super::particles::burst(seq)),
                None => s
                    .child(Icon::new(Lucide::Copy).xsmall())
                    .child(commit.id.to_hex_with_len(10).to_string()),
            })
            .on_click(cx.listener(move |this, _, _, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(sha.clone()));
                this.celebrate_seq += 1;
                let seq = this.celebrate_seq;
                this.copied = Some(seq);
                cx.notify();
                cx.spawn(async move |this, cx| {
                    cx.background_executor()
                        .timer(Duration::from_millis(1300))
                        .await;
                    let _ = this.update(cx, |this, cx| {
                        if this.copied == Some(seq) {
                            this.copied = None;
                            cx.notify();
                        }
                    });
                })
                .detach();
            }))
            .into_any_element()
    }
}
