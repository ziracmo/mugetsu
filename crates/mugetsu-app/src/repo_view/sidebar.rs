use std::path::PathBuf;

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, ClickEvent, Context, InteractiveElement as _, IntoElement, ParentElement as _,
    SharedString, StatefulInteractiveElement as _, Styled as _, Window, div,
    prelude::FluentBuilder as _, px,
};

use super::toolbar::pr_chip;
use super::{RepoEvent, RepoView};
use crate::theme;

type Click = Box<dyn Fn(&ClickEvent, &mut Window, &mut gpui_kit::App)>;

struct Row {
    id: SharedString,
    icon: Lucide,
    icon_color: gpui_kit::Hsla,
    label: String,
    selected: bool,
    bold: bool,
    tooltip: Option<String>,
    suffix: Vec<AnyElement>,
    on_click: Click,
}

fn render_row(row: Row) -> AnyElement {
    let tooltip = row.tooltip;
    h_flex()
        .id(row.id)
        .test_support()
        .h(px(24.))
        .pl_4()
        .pr_2()
        .gap_2()
        .w_full()
        .min_w_0()
        .text_sm()
        .cursor_pointer()
        .border_l_2()
        .border_color(gpui_kit::transparent_black())
        .hover(|s| s.bg(theme::raised()))
        .when(row.selected, |s| {
            s.bg(theme::alpha(theme::cyan(), 0.12))
                .border_color(theme::cyan())
        })
        .child(Icon::new(row.icon).xsmall().text_color(row.icon_color))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .when(row.bold, |s| {
                    s.font_weight(gpui_kit::FontWeight::SEMIBOLD)
                        .text_color(theme::cyan())
                })
                .child(row.label),
        )
        .children(row.suffix)
        .when_some(tooltip, |s, tip| {
            s.tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
        })
        .on_click(row.on_click)
        .into_any_element()
}

impl RepoView {
    fn section(
        &self,
        key: &'static str,
        title: &'static str,
        icon: Lucide,
        count: usize,
        rows: Vec<AnyElement>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let collapsed = self.collapsed.contains(key);
        v_flex()
            .w_full()
            .child(
                h_flex()
                    .id(SharedString::from(format!("section-{key}")))
                    .h(px(28.))
                    .px_2()
                    .gap_1p5()
                    .cursor_pointer()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .hover(|s| s.text_color(cx.theme().foreground))
                    .child(
                        Icon::new(if collapsed {
                            Lucide::ChevronRight
                        } else {
                            Lucide::ChevronDown
                        })
                        .xsmall(),
                    )
                    .child(Icon::new(icon).xsmall())
                    .child(div().flex_1().child(title.to_uppercase()))
                    .child(
                        div()
                            .px_1p5()
                            .rounded_sm()
                            .bg(theme::raised())
                            .child(count.to_string()),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if !this.collapsed.remove(key) {
                            this.collapsed.insert(key);
                        }
                        cx.notify();
                    })),
            )
            .when(!collapsed, |s| s.children(rows))
            .into_any_element()
    }

    pub(super) fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let data = self.data.clone();
        let entity = cx.entity().downgrade();
        let selected_ref = self.selected_ref.clone();

        // Local branches: click selects + looks up PR, double-click checks out.
        let mut branches: Vec<_> = data.refs.branches.iter().collect();
        branches.sort_by_key(|b| (!b.is_head, b.name.clone()));
        let branch_rows = branches
            .into_iter()
            .map(|b| {
                let id = SharedString::from(format!("branch:{}", b.name));
                let mut suffix = Vec::new();
                if b.ahead > 0 {
                    suffix.push(
                        div()
                            .text_xs()
                            .text_color(theme::lime())
                            .child(format!("↑{}", b.ahead))
                            .into_any_element(),
                    );
                }
                if b.behind > 0 {
                    suffix.push(
                        div()
                            .text_xs()
                            .text_color(theme::amber())
                            .child(format!("↓{}", b.behind))
                            .into_any_element(),
                    );
                }
                if b.upstream_gone {
                    suffix.push(
                        div()
                            .text_xs()
                            .text_color(theme::muted())
                            .child("gone")
                            .into_any_element(),
                    );
                }
                suffix.extend(pr_chip(
                    SharedString::from(format!("pr:{}", b.name)),
                    self.pr_for(&b.name, b.upstream.as_deref(), false),
                ));
                let (entity, name, upstream, target, row_id) = (
                    entity.clone(),
                    b.name.clone(),
                    b.upstream.clone(),
                    b.target,
                    id.clone(),
                );
                render_row(Row {
                    id: id.clone(),
                    icon: if b.is_head {
                        Lucide::CircleDot
                    } else {
                        Lucide::GitBranch
                    },
                    icon_color: if b.is_head {
                        theme::cyan()
                    } else {
                        theme::muted()
                    },
                    label: b.name.clone(),
                    selected: selected_ref.as_ref() == Some(&id),
                    bold: b.is_head,
                    tooltip: b.upstream.as_ref().map(|u| format!("suit {u}")),
                    suffix,
                    on_click: Box::new(move |ev, window, cx| {
                        let double = ev.click_count() >= 2;
                        let _ = entity.update(cx, |this, cx| {
                            this.selected_ref = Some(row_id.clone());
                            this.select_commit(target, true, cx);
                            this.lookup_pr(&name, upstream.as_deref(), false, cx);
                            if double {
                                let name = name.clone();
                                this.run_op("Checkout", move |r| r.checkout(&name), window, cx);
                            }
                        });
                    }),
                })
            })
            .collect::<Vec<_>>();

        // Remotes with their branches.
        let mut remote_rows = Vec::new();
        for remote in &data.refs.remotes {
            remote_rows.push(
                h_flex()
                    .h(px(22.))
                    .pl_4()
                    .gap_2()
                    .text_xs()
                    .text_color(theme::violet())
                    .child(Icon::new(Lucide::Cloud).xsmall())
                    .child(div().truncate().child(remote.name.clone()))
                    .into_any_element(),
            );
            let prefix = format!("{}/", remote.name);
            for b in data
                .refs
                .remote_branches
                .iter()
                .filter(|b| b.name.starts_with(&prefix))
            {
                let id = SharedString::from(format!("remote:{}", b.name));
                let (entity, name, target, row_id) =
                    (entity.clone(), b.name.clone(), b.target, id.clone());
                let suffix = pr_chip(
                    SharedString::from(format!("rpr:{}", b.name)),
                    self.pr_for(&b.name, None, true),
                )
                .into_iter()
                .collect();
                remote_rows.push(render_row(Row {
                    id: id.clone(),
                    icon: Lucide::GitBranch,
                    icon_color: theme::alpha(theme::violet(), 0.7),
                    label: b.name[prefix.len()..].to_string(),
                    selected: selected_ref.as_ref() == Some(&id),
                    bold: false,
                    tooltip: Some(remote.url.clone()),
                    suffix,
                    on_click: Box::new(move |ev, window, cx| {
                        let double = ev.click_count() >= 2;
                        let _ = entity.update(cx, |this, cx| {
                            this.selected_ref = Some(row_id.clone());
                            this.select_commit(target, true, cx);
                            this.lookup_pr(&name, None, true, cx);
                            if double {
                                let name = name.clone();
                                this.run_op(
                                    "Checkout",
                                    move |r| r.checkout_remote(&name),
                                    window,
                                    cx,
                                );
                            }
                        });
                    }),
                }));
            }
        }

        let tag_rows = data
            .refs
            .tags
            .iter()
            .rev()
            .map(|t| {
                let id = SharedString::from(format!("tag:{}", t.name));
                let (entity, target, row_id) = (entity.clone(), t.target, id.clone());
                render_row(Row {
                    id: id.clone(),
                    icon: Lucide::Tag,
                    icon_color: theme::amber(),
                    label: t.name.clone(),
                    selected: selected_ref.as_ref() == Some(&id),
                    bold: false,
                    tooltip: None,
                    suffix: vec![],
                    on_click: Box::new(move |_, _, cx| {
                        let _ = entity.update(cx, |this, cx| {
                            this.selected_ref = Some(row_id.clone());
                            this.select_commit(target, true, cx);
                        });
                    }),
                })
            })
            .collect::<Vec<_>>();

        let worktree_rows = data
            .refs
            .worktrees
            .iter()
            .map(|w| {
                let id = SharedString::from(format!("worktree:{}", w.path));
                let entity = entity.clone();
                let path = PathBuf::from(&w.path);
                let label = w.branch.clone().unwrap_or_else(|| "(détaché)".into());
                let is_current = path == self.repo.root();
                render_row(Row {
                    id,
                    icon: Lucide::FolderGit2,
                    icon_color: if is_current {
                        theme::cyan()
                    } else {
                        theme::lime()
                    },
                    label: format!(
                        "{label}  ·  {}",
                        path.file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_default()
                    ),
                    selected: false,
                    bold: is_current,
                    tooltip: Some(format!(
                        "{}{}",
                        w.path,
                        if is_current {
                            ""
                        } else {
                            "\nCliquer pour ouvrir dans un onglet"
                        }
                    )),
                    suffix: if w.locked {
                        vec![Icon::new(Lucide::Lock).xsmall().into_any_element()]
                    } else {
                        vec![]
                    },
                    on_click: Box::new(move |_, _, cx| {
                        let path = path.clone();
                        let _ = entity.update(cx, |_, cx| {
                            if !is_current {
                                cx.emit(RepoEvent::OpenWorktree(path));
                            }
                        });
                    }),
                })
            })
            .collect::<Vec<_>>();

        let stash_rows = data
            .refs
            .stashes
            .iter()
            .map(|s| {
                let id = SharedString::from(format!("stash:{}", s.index));
                let (entity, target, row_id) = (entity.clone(), s.id, id.clone());
                render_row(Row {
                    id: id.clone(),
                    icon: Lucide::Archive,
                    icon_color: theme::magenta(),
                    label: s.message.clone(),
                    selected: selected_ref.as_ref() == Some(&id),
                    bold: false,
                    tooltip: Some(format!("stash@{{{}}}", s.index)),
                    suffix: vec![],
                    on_click: Box::new(move |_, _, cx| {
                        let _ = entity.update(cx, |this, cx| {
                            this.selected_ref = Some(row_id.clone());
                            this.select_commit(target, true, cx);
                        });
                    }),
                })
            })
            .collect::<Vec<_>>();

        let (nb, nr, nt, nw, ns) = (
            data.refs.branches.len(),
            data.refs.remote_branches.len(),
            data.refs.tags.len(),
            data.refs.worktrees.len(),
            data.refs.stashes.len(),
        );
        div()
            .id("sidebar")
            .size_full()
            .bg(theme::panel())
            .border_r_1()
            .border_color(cx.theme().border)
            .child(
                v_flex()
                    .id("sidebar-scroll")
                    .size_full()
                    .py_1()
                    .overflow_y_scroll()
                    .child(self.section(
                        "branches",
                        "Branches",
                        Lucide::GitBranch,
                        nb,
                        branch_rows,
                        cx,
                    ))
                    .child(self.section("remotes", "Remotes", Lucide::Cloud, nr, remote_rows, cx))
                    .child(self.section(
                        "worktrees",
                        "Worktrees",
                        Lucide::FolderTree,
                        nw,
                        worktree_rows,
                        cx,
                    ))
                    .child(self.section("tags", "Tags", Lucide::Tags, nt, tag_rows, cx))
                    .child(self.section("stashes", "Stashes", Lucide::Archive, ns, stash_rows, cx)),
            )
    }
}
