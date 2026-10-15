//! Sidebar "PRs" tab: open PRs (flagging those awaiting my review), drafts,
//! GitHub stacks and recently merged PRs. Also the stack position shown next
//! to branches.

use std::collections::HashMap;

use chrono::{DateTime, Duration, Utc};
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::menu::{ContextMenuExt as _, PopupMenuItem};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{Icon, Sizable as _, h_flex};
use gpui_kit::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled as _, div, prelude::FluentBuilder as _, px,
};
use mugetsu_forge::{PrState, PullRequest};

use super::toolbar::{check_look, pr_card, pr_color};
use super::{RepoView, SidebarTab};
use crate::i18n::tr;
use crate::theme;

/// Head branch → (position from the bottom, 1-based; stack size).
pub fn stack_positions(prs: &[PullRequest]) -> HashMap<String, (usize, usize)> {
    let mut out = HashMap::new();
    for stack in mugetsu_forge::stacks(prs) {
        let n = stack.len();
        for (i, pr) in stack.into_iter().enumerate() {
            out.insert(pr.head, (i + 1, n));
        }
    }
    out
}

pub fn merged_within(pr: &PullRequest, now: DateTime<Utc>, days: u32) -> bool {
    pr.state == PrState::Merged
        && pr
            .merged_at
            .as_deref()
            .and_then(|t| DateTime::parse_from_rfc3339(t).ok())
            .is_some_and(|t| now - t.with_timezone(&Utc) <= Duration::days(days.into()))
}

pub fn awaits(pr: &PullRequest, me: Option<&str>) -> bool {
    me.is_some_and(|me| pr.requested_reviewers.iter().any(|r| r == me))
}

fn matches(pr: &PullRequest, query: &str) -> bool {
    let hay = format!("#{} {} {} {}", pr.number, pr.title, pr.head, pr.author).to_lowercase();
    query
        .split_whitespace()
        .all(|w| hay.contains(&w.to_lowercase()))
}

/// Small "Layers n/total" badge for a branch in a stack.
pub fn stack_badge(pos: usize, total: usize) -> AnyElement {
    h_flex()
        .flex_none()
        .gap_0p5()
        .px_1()
        .rounded_sm()
        .text_xs()
        .text_color(theme::violet())
        .bg(theme::alpha(theme::violet(), 0.14))
        .child(Icon::new(Lucide::Layers).xsmall())
        .child(format!("{pos}/{total}"))
        .into_any_element()
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PrFilter {
    #[default]
    All,
    /// Authored by me.
    Mine,
    /// My review is requested.
    Review,
}

pub fn keep(pr: &PullRequest, filter: PrFilter, me: Option<&str>) -> bool {
    match filter {
        PrFilter::All => true,
        PrFilter::Mine => me.is_some_and(|me| pr.author == me),
        PrFilter::Review => awaits(pr, me),
    }
}

impl RepoView {
    /// Tip of a PR's head branch: the local branch, else the remote one.
    fn pr_tip(&self, head: &str) -> Option<gix::ObjectId> {
        let refs = &self.data.refs;
        let remote = super::sidebar::default_remote(refs).map(|r| format!("{r}/{head}"));
        refs.branches
            .iter()
            .find(|b| b.name == head)
            .or_else(|| {
                refs.remote_branches
                    .iter()
                    .find(|b| Some(&b.name) == remote.as_ref())
            })
            .map(|b| b.target)
    }

    /// "Show branch": back to the repository tab, tip selected and shimmering.
    pub fn show_pr_branch(&mut self, head: &str, cx: &mut Context<Self>) {
        self.pr_menu_open = false;
        let Some(tip) = self.pr_tip(head) else {
            return;
        };
        self.sidebar_tab = SidebarTab::Repo;
        if self.data.refs.branches.iter().any(|b| b.name == head) {
            self.selected_ref = Some(SharedString::from(format!("branch:{head}")));
        }
        self.select_commit(tip, true, cx);
        self.flash_commits(std::collections::HashSet::from([tip]), cx);
    }

    /// Quick filter above the PR sections.
    fn render_pr_filter(&self, cx: &mut Context<Self>) -> AnyElement {
        let chip = |id: &'static str, label: &'static str, filter: PrFilter| {
            let active = self.pr_filter == filter;
            div()
                .id(id)
                .test_support()
                .px_2()
                .h(px(20.))
                .flex()
                .items_center()
                .rounded_full()
                .text_xs()
                .cursor_pointer()
                .border_1()
                .when(active, |s| {
                    s.bg(theme::alpha(theme::accent(), 0.25))
                        .border_color(theme::alpha(theme::accent(), 0.7))
                        .text_color(gpui_kit::white())
                })
                .when(!active, |s| {
                    s.border_color(theme::hairline())
                        .text_color(theme::muted())
                        .hover(|s| s.text_color(gpui_kit::white()))
                })
                .child(label)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.pr_filter = filter;
                    cx.notify();
                }))
        };
        h_flex()
            .px_2()
            .pb_1()
            .gap_1()
            .child(chip("pr-filter-all", tr("Toutes", "All"), PrFilter::All))
            .child(chip(
                "pr-filter-mine",
                tr("Les miennes", "Mine"),
                PrFilter::Mine,
            ))
            .child(chip(
                "pr-filter-review",
                tr("À reviewer", "To review"),
                PrFilter::Review,
            ))
            .into_any_element()
    }

    /// Tabs at the top of the sidebar.
    pub(super) fn render_sidebar_tabs(&self, cx: &mut Context<Self>) -> AnyElement {
        let waiting = self
            .pr_list
            .iter()
            .filter(|p| p.state == PrState::Open && awaits(p, self.pr_me.as_deref()))
            .count();
        let tab = |id: &'static str, label: &'static str, which: SidebarTab, badge: usize| {
            let active = self.sidebar_tab == which;
            h_flex()
                .id(id)
                .test_support()
                .flex_1()
                .h(px(24.))
                .gap_1()
                .justify_center()
                .rounded_md()
                .text_sm()
                .cursor_pointer()
                .when(active, |s| {
                    s.bg(theme::alpha(gpui_kit::white(), 0.14))
                        .border_1()
                        .border_color(theme::glass_edge())
                        .shadow_sm()
                        .text_color(gpui_kit::white())
                })
                .when(!active, |s| {
                    s.text_color(theme::muted())
                        .hover(|s| s.text_color(gpui_kit::white()))
                })
                .child(label)
                .when(badge > 0, |s| {
                    s.child(
                        div()
                            .flex_none()
                            .h(px(16.))
                            .min_w(px(16.))
                            .px_1()
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_full()
                            .text_xs()
                            .line_height(px(16.))
                            .bg(theme::accent())
                            .text_color(gpui_kit::white())
                            .child(badge.to_string()),
                    )
                })
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.sidebar_tab = which;
                    let prs = which == SidebarTab::Prs;
                    crate::settings::remember(|s| s.sidebar_prs = prs);
                    cx.notify();
                }))
        };
        h_flex()
            .mx_2()
            .mt_2()
            .p(px(2.))
            .gap_0p5()
            .rounded_lg()
            .bg(theme::alpha(gpui_kit::white(), 0.05))
            .border_1()
            .border_color(theme::hairline())
            .child(tab(
                "sidebar-tab-repo",
                tr("Dépôt", "Repository"),
                SidebarTab::Repo,
                0,
            ))
            .child(tab("sidebar-tab-prs", "PRs", SidebarTab::Prs, waiting))
            .into_any_element()
    }

    fn pr_row(
        &self,
        pr: &PullRequest,
        stack: Option<(usize, usize)>,
        depth: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let icon = match pr.state {
            PrState::Open => Lucide::GitPullRequest,
            PrState::Draft => Lucide::GitPullRequestDraft,
            PrState::Merged => Lucide::GitMerge,
            PrState::Closed => Lucide::GitPullRequestClosed,
        };
        let waiting = pr.state == PrState::Open && awaits(pr, self.pr_me.as_deref());
        let card = pr.clone();
        let open_url = pr.url.clone();
        let head = pr.head.clone();
        let show_head = pr.head.clone();
        let view = cx.entity().downgrade();
        let can_checkout = pr.same_repo && pr.state != PrState::Merged;
        let remote = super::sidebar::default_remote(&self.data.refs);
        let checkout_ref = remote.as_ref().map(|r| format!("{r}/{}", pr.head));
        h_flex()
            .id(SharedString::from(format!("pr-row-{}", pr.number)))
            .test_support()
            .h(px(24.))
            .pl(px(14. + 14. * depth as f32))
            .pr_2()
            .mx_1()
            .gap_1p5()
            .min_w_0()
            .rounded_md()
            .text_sm()
            .cursor_pointer()
            .hover(|s| s.bg(theme::raised()))
            .child(Icon::new(icon).xsmall().text_color(pr_color(pr.state)))
            .child(
                div()
                    .flex_none()
                    .text_xs()
                    .text_color(theme::muted())
                    .child(format!("#{}", pr.number)),
            )
            .child(div().flex_1().min_w_0().truncate().child(pr.title.clone()))
            .when_some(stack, |s, (pos, n)| s.child(stack_badge(pos, n)))
            .when(waiting, |s| {
                s.child(Icon::new(Lucide::Eye).xsmall().text_color(theme::accent()))
            })
            .when_some(pr.status.as_ref().and_then(|st| st.checks), |s, c| {
                let (icon, color) = check_look(c);
                s.child(Icon::new(icon).xsmall().text_color(color))
            })
            .when_some(
                pr.status.as_ref().filter(|st| st.unresolved > 0),
                |s, st| {
                    s.child(
                        div()
                            .text_xs()
                            .text_color(theme::muted())
                            .child(format!("💬{}", st.unresolved)),
                    )
                },
            )
            // No info card over the context menu.
            .when(!self.pr_menu_open, |s| {
                s.tooltip(move |window, cx| {
                    let card = card.clone();
                    Tooltip::element(move |_, _| pr_card(&card)).build(window, cx)
                })
            })
            .on_mouse_down(
                gpui_kit::MouseButton::Right,
                cx.listener(|this, _, _, cx| {
                    this.pr_menu_open = true;
                    cx.notify();
                }),
            )
            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                if !*hovered && this.pr_menu_open {
                    this.pr_menu_open = false;
                    cx.notify();
                }
            }))
            .on_click(
                cx.listener(move |this, ev: &gpui_kit::ClickEvent, window, cx| {
                    // Select the branch tip (local, else remote), double-click checks out.
                    let remote_name = remote.as_ref().map(|r| format!("{r}/{head}"));
                    if let Some(tip) = this.pr_tip(&head) {
                        this.select_commit(tip, true, cx);
                    }
                    if ev.click_count() >= 2
                        && can_checkout
                        && let Some(r) = remote_name
                    {
                        this.request_checkout(r, true, window, cx);
                    }
                }),
            )
            .context_menu(move |m, _, _| {
                let url = open_url.clone();
                let (show_view, show_head) = (view.clone(), show_head.clone());
                let m = m
                    .item(
                        PopupMenuItem::new(tr("Montrer la branche", "Show branch"))
                            .icon(Icon::new(Lucide::Eye))
                            .on_click(move |_, _, cx| {
                                let _ = show_view
                                    .update(cx, |this, cx| this.show_pr_branch(&show_head, cx));
                            }),
                    )
                    .item(
                        PopupMenuItem::new(tr("Ouvrir sur GitHub", "Open on GitHub"))
                            .icon(Icon::new(Lucide::ExternalLink))
                            .on_click(move |_, _, cx| cx.open_url(&url)),
                    );
                match checkout_ref.clone().filter(|_| can_checkout) {
                    Some(r) => {
                        let view = view.clone();
                        m.item(
                            PopupMenuItem::new(tr("Checkout…", "Checkout…"))
                                .icon(Icon::new(Lucide::GitBranch))
                                .on_click(move |_, window, cx| {
                                    let _ = view.update(cx, |this, cx| {
                                        this.request_checkout(r.clone(), true, window, cx)
                                    });
                                }),
                        )
                    }
                    None => m,
                }
            })
            .into_any_element()
    }

    /// Sections of the PRs tab.
    pub(super) fn pr_sections(&self, query: &str, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let searching = !query.is_empty();
        if self.data.github.is_none() {
            return vec![
                div()
                    .p_4()
                    .text_sm()
                    .text_color(theme::muted())
                    .child(tr(
                        "Pas de remote GitHub pour ce dépôt.",
                        "No GitHub remote for this repository.",
                    ))
                    .into_any_element(),
            ];
        }
        let filter = self.pr_filter;
        let prs: Vec<&PullRequest> = self
            .pr_list
            .iter()
            .filter(|p| matches(p, query) && keep(p, filter, self.pr_me.as_deref()))
            .collect();
        let stacks = stack_positions(&self.pr_list);
        let me = self.pr_me.as_deref();
        let mut open: Vec<&PullRequest> = prs
            .iter()
            .copied()
            .filter(|p| p.state == PrState::Open)
            .collect();
        // Reviews I owe first.
        open.sort_by_key(|p| (!awaits(p, me), std::cmp::Reverse(p.number)));
        let drafts: Vec<&PullRequest> = prs
            .iter()
            .copied()
            .filter(|p| p.state == PrState::Draft)
            .collect();
        let days = crate::settings::get().merged_days;
        let now = Utc::now();
        let merged: Vec<&PullRequest> = prs
            .iter()
            .copied()
            .filter(|p| merged_within(p, now, days))
            .collect();

        let rows = |list: &[&PullRequest], this: &Self, cx: &mut Context<Self>| {
            list.iter()
                .map(|p| this.pr_row(p, stacks.get(&p.head).copied(), 0, cx))
                .collect::<Vec<_>>()
        };
        let open_rows = rows(&open, self, cx);
        let draft_rows = rows(&drafts, self, cx);
        let merged_rows = rows(&merged, self, cx);
        let mut stack_rows = Vec::new();
        let mut stack_count = 0;
        for (i, stack) in mugetsu_forge::stacks(&self.pr_list).iter().enumerate() {
            if !stack
                .iter()
                .any(|p| matches(p, query) && keep(p, filter, self.pr_me.as_deref()))
            {
                continue;
            }
            stack_count += 1;
            let base = stack.first().map(|p| p.base.clone()).unwrap_or_default();
            stack_rows.push(
                h_flex()
                    .id(("stack", i))
                    .h(px(22.))
                    .pl(px(14.))
                    .gap_1p5()
                    .text_xs()
                    .text_color(theme::muted())
                    .child(
                        Icon::new(Lucide::Layers)
                            .xsmall()
                            .text_color(theme::violet()),
                    )
                    .child(format!("{base} ← {}", stack.len()))
                    .into_any_element(),
            );
            // Top of the stack first, like GitHub's stack view.
            for (pos, pr) in stack.iter().enumerate().rev() {
                stack_rows.push(self.pr_row(pr, Some((pos + 1, stack.len())), 1, cx));
            }
        }
        let merged_title: &'static str = tr("Mergées récemment", "Recently merged");
        let filter_bar = self.render_pr_filter(cx);
        [
            Some(filter_bar),
            self.section(
                "pr-open",
                tr("Ouvertes", "Open"),
                Lucide::GitPullRequest,
                open.len(),
                open_rows,
                searching,
                None,
                cx,
            ),
            self.section(
                "pr-drafts",
                tr("Brouillons", "Drafts"),
                Lucide::GitPullRequestDraft,
                drafts.len(),
                draft_rows,
                searching,
                None,
                cx,
            ),
            self.section(
                "pr-stacks",
                "Stacks",
                Lucide::Layers,
                stack_count,
                stack_rows,
                searching,
                None,
                cx,
            ),
            self.section(
                "pr-merged",
                merged_title,
                Lucide::GitMerge,
                merged.len(),
                merged_rows,
                searching,
                Some(
                    div()
                        .text_xs()
                        .text_color(theme::muted())
                        .child(trf!("{days} j", "{days} d"))
                        .into_any_element(),
                ),
                cx,
            ),
        ]
        .into_iter()
        .flatten()
        .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pr(number: u64, head: &str, base: &str, state: PrState) -> PullRequest {
        PullRequest {
            number,
            title: format!("PR {number}"),
            url: String::new(),
            state,
            status: None,
            head: head.into(),
            base: base.into(),
            same_repo: true,
            author: "a".into(),
            requested_reviewers: vec!["me".into()],
            updated_at: String::new(),
            merged_at: None,
        }
    }

    #[test]
    fn positions_reviews_and_merge_age() {
        let prs = [
            pr(1, "a", "main", PrState::Open),
            pr(2, "b", "a", PrState::Draft),
            pr(3, "c", "main", PrState::Open),
        ];
        let pos = stack_positions(&prs);
        assert_eq!(pos.get("a"), Some(&(1, 2)));
        assert_eq!(pos.get("b"), Some(&(2, 2)));
        assert_eq!(pos.get("c"), None);
        assert!(awaits(&prs[0], Some("me")));
        assert!(!awaits(&prs[0], Some("you")));

        let now = DateTime::parse_from_rfc3339("2026-10-15T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let mut merged = pr(4, "d", "main", PrState::Merged);
        merged.merged_at = Some("2026-10-10T12:00:00Z".into());
        assert!(merged_within(&merged, now, 7));
        assert!(!merged_within(&merged, now, 3));
        assert!(matches(&merged, "#4"));
    }
}
