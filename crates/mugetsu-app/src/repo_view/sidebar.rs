use std::path::PathBuf;
use std::rc::Rc;

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::input::Input;
use gpui_kit::component::menu::ContextMenuExt as _;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, ClickEvent, Context, InteractiveElement as _, IntoElement, ParentElement as _,
    SharedString, StatefulInteractiveElement as _, Styled as _, Window, div,
    prelude::FluentBuilder as _, px,
};
use mugetsu_git::{Branch, RefsSnapshot};

use super::menus::{self, BranchMenu, MenuBuilder, MenuContext};
use super::toolbar::pr_chip;
use super::{RepoEvent, RepoView};
use crate::branch_tree::{self, NodeKind, TreeNode};
use crate::i18n::tr;
use crate::theme;

type Click = Box<dyn Fn(&ClickEvent, &mut Window, &mut gpui_kit::App)>;

const INDENT: f32 = 14.;

/// Every foldable section key (repository and PR tabs).
pub const SECTION_KEYS: [&str; 9] = [
    "branches",
    "remotes",
    "worktrees",
    "tags",
    "stashes",
    "pr-open",
    "pr-drafts",
    "pr-stacks",
    "pr-merged",
];

struct Row {
    /// For the context-menu state (no tooltip over an open menu).
    view: gpui_kit::WeakEntity<RepoView>,
    /// A context menu is open: no tooltip.
    quiet: bool,
    id: SharedString,
    depth: usize,
    icon: Lucide,
    icon_color: gpui_kit::Hsla,
    label: String,
    selected: bool,
    bold: bool,
    tooltip: Option<String>,
    suffix: Vec<AnyElement>,
    on_click: Click,
    menu: Option<MenuBuilder>,
}

fn render_row(row: Row) -> AnyElement {
    let tooltip = row.tooltip.filter(|_| !row.quiet);
    let (down_view, hover_view) = (row.view.clone(), row.view);
    let el = h_flex()
        .id(row.id)
        .test_support()
        .h(px(24.))
        .pl(px(14. + INDENT * row.depth as f32))
        .pr_2()
        .mx_1()
        .gap_2()
        .min_w_0()
        .rounded_md()
        .text_sm()
        .cursor_pointer()
        .relative()
        .hover(|s| s.bg(theme::raised()))
        .when(row.selected && !row.bold, |s| {
            s.bg(theme::alpha(theme::accent(), 0.14))
                .border_1()
                .border_color(theme::alpha(theme::accent(), 0.25))
        })
        // Where we are (checked-out branch / worktree): filled accent pill.
        .when(row.bold, |s| {
            s.bg(theme::alpha(theme::accent(), 0.3))
                .border_1()
                .border_color(theme::alpha(theme::accent(), 0.7))
                .shadow_sm()
                .text_color(gpui_kit::white())
        })
        .child(Icon::new(row.icon).xsmall().text_color(if row.bold {
            gpui_kit::white()
        } else {
            row.icon_color
        }))
        .child(
            div().relative().flex_1().min_w_0().child(
                div()
                    .truncate()
                    .when(row.bold, |s| s.font_weight(gpui_kit::FontWeight::BOLD))
                    .child(row.label),
            ),
        )
        .children(row.suffix)
        .when_some(tooltip, |s, tip| {
            s.tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
        })
        .on_click(row.on_click)
        // Right-click opens the menu: keep the tooltip out of its way.
        .on_mouse_down(gpui_kit::MouseButton::Right, move |_, _, cx| {
            let _ = down_view.update(cx, |this, cx| {
                this.menu_open = true;
                cx.notify();
            });
        })
        .on_hover(move |hovered, _, cx| {
            if !*hovered {
                let _ = hover_view.update(cx, |this, cx| {
                    if this.menu_open {
                        this.menu_open = false;
                        cx.notify();
                    }
                });
            }
        });
    match row.menu {
        Some(menu) => el
            .context_menu(move |m, window, cx| menu(m, window, cx))
            .into_any_element(),
        None => el.into_any_element(),
    }
}

/// A sidebar section header being dragged; also its drag preview.
#[derive(Clone)]
struct SectionDrag {
    key: &'static str,
    title: &'static str,
}

impl gpui_kit::Render for SectionDrag {
    fn render(&mut self, _: &mut gpui_kit::Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .px_2()
            .py_1()
            .rounded_md()
            .text_sm()
            .font_weight(gpui_kit::FontWeight::SEMIBOLD)
            .bg(theme::popover())
            .text_color(gpui_kit::white())
            .border_1()
            .border_color(theme::alpha(theme::accent(), 0.6))
            .shadow_lg()
            .child(self.title)
    }
}

/// Repository tab: which PRs show a chip on branches (all / mine / to review).
fn pr_chips_button() -> AnyElement {
    use crate::settings::PrFilter;
    use gpui_kit::base::Selectable as _;
    use gpui_kit::component::button::{Button, ButtonVariants as _};
    use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
    let current = crate::settings::get().pr_chips;
    Button::new("pr-chips-config")
        .ghost()
        .small()
        .icon(Icon::new(Lucide::GitPullRequest))
        .when(current != PrFilter::All, |b| b.selected(true))
        .tooltip(tr("Infos de PR affichées", "PR info shown"))
        .dropdown_menu(move |menu, _, _| {
            let item = |label: &'static str, filter: PrFilter| {
                PopupMenuItem::new(label)
                    .checked(current == filter)
                    .on_click(move |_, _, cx| crate::settings::update(cx, |s| s.pr_chips = filter))
            };
            menu.label(tr("Chips de PR sur les branches", "PR chips on branches"))
                .item(item(tr("Toutes les PRs", "Every PR"), PrFilter::All))
                .item(item(tr("Mes PRs", "My PRs"), PrFilter::Mine))
                .item(item(
                    tr("Celles à reviewer", "PRs I review"),
                    PrFilter::Review,
                ))
        })
        .into_any_element()
}

/// `origin` when present, else the first remote.
pub(super) fn default_remote(refs: &RefsSnapshot) -> Option<String> {
    refs.remotes
        .iter()
        .find(|r| r.name == "origin")
        .or(refs.remotes.first())
        .map(|r| r.name.clone())
}

impl RepoView {
    pub(super) fn menu_context(&self, cx: &Context<Self>) -> MenuContext {
        MenuContext {
            view: cx.entity().downgrade(),
            root: self.repo.root().to_path_buf(),
            github: self.data.github.clone(),
            default_remote: default_remote(&self.data.refs),
            head_branch: self.data.refs.head_branch.clone(),
            head: self.data.refs.head,
            rebasing: self.data.operation.is_some(),
        }
    }

    pub(super) fn branch_menu_info(&self, b: &Branch) -> BranchMenu {
        BranchMenu {
            name: b.name.clone(),
            upstream: b.upstream.clone(),
            ahead: b.ahead,
            behind: b.behind,
            is_head: b.is_head,
            gone: b.upstream_gone,
            pr: menus::found_pr(self.pr_for(&b.name, b.upstream.as_deref(), false)),
        }
    }

    pub(super) fn sidebar_query(&self, cx: &gpui_kit::App) -> String {
        self.sidebar_search.read(cx).value().trim().to_string()
    }

    /// Folder rows are open when toggled open, or by default when they hold HEAD.
    fn folder_open(&self, key: &str, default: bool, searching: bool) -> bool {
        searching || self.folders.get(key).copied().unwrap_or(default)
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn section(
        &self,
        key: &'static str,
        title: &'static str,
        icon: Lucide,
        count: usize,
        rows: Vec<AnyElement>,
        searching: bool,
        extra: Option<AnyElement>,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if searching && rows.is_empty() {
            return None;
        }
        let collapsed = !searching && self.collapsed.contains(key);
        Some(
            v_flex()
                .w_full()
                .child(
                    h_flex()
                        .id(SharedString::from(format!("section-{key}")))
                        .h(px(28.))
                        .px_2()
                        .gap_1p5()
                        .cursor_pointer()
                        .text_sm()
                        .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                        .text_color(theme::alpha(gpui_kit::white(), 0.82))
                        .hover(|s| s.text_color(gpui_kit::white()))
                        .child(
                            Icon::new(if collapsed {
                                Lucide::ChevronRight
                            } else {
                                Lucide::ChevronDown
                            })
                            .xsmall(),
                        )
                        .child(Icon::new(icon).xsmall())
                        .child(div().flex_1().child(title))
                        .children(extra)
                        .child(
                            div()
                                .px_1p5()
                                .rounded_md()
                                .text_xs()
                                .bg(theme::raised())
                                .child(count.to_string()),
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if !this.collapsed.remove(key) {
                                this.collapsed.insert(key);
                            }
                            let folded: Vec<String> =
                                this.collapsed.iter().map(|k| k.to_string()).collect();
                            crate::settings::remember(|s| s.collapsed_sections = folded);
                            cx.notify();
                        }))
                        // Drag a header onto another to reorder the sections.
                        .when(crate::settings::SECTIONS.contains(&key), |s| {
                            s.on_drag(SectionDrag { key, title }, |drag, _, _, cx| {
                                gpui_kit::AppContext::new(cx, |_| drag.clone())
                            })
                            .drag_over::<SectionDrag>(|s, _, _, _| {
                                s.bg(theme::alpha(theme::accent(), 0.18)).rounded_md()
                            })
                            .on_drop(cx.listener(
                                move |_, drag: &SectionDrag, _, cx| {
                                    let from = drag.key;
                                    crate::settings::update(cx, |s| s.move_section(from, key));
                                },
                            ))
                        }),
                )
                .when(!collapsed, |s| s.children(rows))
                .into_any_element(),
        )
    }

    /// Renders a folder tree; `leaf` renders one item at a depth.
    #[allow(clippy::too_many_arguments)]
    fn tree_rows<T>(
        &self,
        scope: &str,
        nodes: &[TreeNode<T>],
        depth: usize,
        head_path: Option<&str>,
        searching: bool,
        out: &mut Vec<AnyElement>,
        leaf: &mut dyn FnMut(&TreeNode<T>, &T, usize) -> AnyElement,
        cx: &mut Context<Self>,
    ) {
        for node in nodes {
            match &node.kind {
                NodeKind::Leaf(value) => out.push(leaf(node, value, depth)),
                NodeKind::Folder(children) => {
                    let key = format!("{scope}:{}", node.path);
                    let holds_head =
                        head_path.is_some_and(|h| h.starts_with(&format!("{}/", node.path)));
                    let open = self.folder_open(&key, holds_head, searching);
                    out.push(
                        h_flex()
                            .id(SharedString::from(format!("folder:{key}")))
                            .test_support()
                            .h(px(24.))
                            .pl(px(2. + INDENT * depth as f32))
                            .pr_2()
                            .mx_1()
                            .gap_1()
                            .rounded_md()
                            .text_sm()
                            .cursor_pointer()
                            .hover(|s| s.bg(theme::raised()))
                            .child(
                                Icon::new(if open {
                                    Lucide::ChevronDown
                                } else {
                                    Lucide::ChevronRight
                                })
                                .xsmall()
                                .text_color(theme::muted()),
                            )
                            .child(
                                Icon::new(if open {
                                    Lucide::FolderOpen
                                } else {
                                    Lucide::Folder
                                })
                                .xsmall()
                                .text_color(theme::alpha(theme::accent(), 0.7)),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .child(node.label.clone()),
                            )
                            .when(!open, |s| {
                                s.child(
                                    div()
                                        .text_xs()
                                        .text_color(theme::muted())
                                        .child(node.leaf_count().to_string()),
                                )
                            })
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.folders.insert(key.clone(), !open);
                                cx.notify();
                            }))
                            .into_any_element(),
                    );
                    if open {
                        self.tree_rows(
                            scope,
                            children,
                            depth + 1,
                            head_path,
                            searching,
                            out,
                            leaf,
                            cx,
                        );
                    }
                }
            }
        }
    }

    fn branch_rows(&self, query: &str, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let data = self.data.clone();
        let entity = cx.entity().downgrade();
        let ctx = self.menu_context(cx);
        let searching = !query.is_empty();
        let pushed_color = theme::alpha(cx.theme().foreground, 0.55);
        let stacks = super::pr_panel::stack_positions(&self.pr_list);
        let tree = branch_tree::build(
            data.refs
                .branches
                .iter()
                .filter(|b| branch_tree::matches(&b.name, query))
                .map(|b| (b.name.clone(), b)),
        );
        let mut rows = Vec::new();
        let mut leaf = |node: &TreeNode<&Branch>, b: &&Branch, depth: usize| {
            let b: &Branch = b;
            let id = SharedString::from(format!("branch:{}", b.name));
            let pushed = b.upstream.is_some() && !b.upstream_gone;
            let mut suffix = Vec::new();
            if b.ahead > 0 {
                suffix.push(
                    div()
                        .text_xs()
                        .text_color(theme::lime())
                        .child(format!("{}↑", b.ahead))
                        .into_any_element(),
                );
            }
            if b.behind > 0 {
                suffix.push(
                    div()
                        .text_xs()
                        .text_color(theme::amber())
                        .child(format!("{}↓", b.behind))
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
            if let Some(&(pos, n)) = stacks.get(&b.name) {
                suffix.push(super::pr_panel::stack_badge(pos, n));
            }
            suffix.extend(pr_chip(
                SharedString::from(format!("pr:{}", b.name)),
                self.pr_for(&b.name, b.upstream.as_deref(), false),
            ));
            let info = self.branch_menu_info(b);
            let menu_ctx = ctx.clone();
            let (entity, name, upstream, target, row_id) = (
                entity.clone(),
                b.name.clone(),
                b.upstream.clone(),
                b.target,
                id.clone(),
            );
            render_row(Row {
                view: entity.clone(),
                quiet: self.menu_open,
                id: id.clone(),
                depth,
                icon: if b.is_head {
                    Lucide::Check
                } else if pushed {
                    Lucide::Github
                } else {
                    Lucide::GitBranch
                },
                icon_color: if b.is_head {
                    theme::accent()
                } else if pushed {
                    pushed_color
                } else {
                    theme::muted()
                },
                label: node.label.clone(),
                selected: self.selected_ref.as_ref() == Some(&id),
                bold: b.is_head,
                tooltip: Some(match &b.upstream {
                    Some(u) if !b.upstream_gone => {
                        trf!("{}\nPoussée sur {u}", "{}\nPushed to {u}", b.name)
                    }
                    Some(u) => trf!(
                        "{}\n{u} supprimée du remote",
                        "{}\n{u} deleted on the remote",
                        b.name
                    ),
                    None => trf!("{}\nPas encore poussée", "{}\nNot pushed yet", b.name),
                }),
                suffix,
                on_click: Box::new(move |ev, window, cx| {
                    let double = ev.click_count() >= 2;
                    let _ = entity.update(cx, |this, cx| {
                        this.selected_ref = Some(row_id.clone());
                        this.select_commit(target, true, cx);
                        this.lookup_pr(&name, upstream.as_deref(), false, cx);
                        if double {
                            this.request_checkout(name.clone(), false, window, cx);
                        }
                    });
                }),
                menu: Some(Rc::new(move |m, _, _| {
                    menus::local_branch_menu(m, &menu_ctx, &info)
                })),
            })
        };
        self.tree_rows(
            "local",
            &tree,
            0,
            data.refs.head_branch.as_deref(),
            searching,
            &mut rows,
            &mut leaf,
            cx,
        );
        rows
    }

    fn remote_rows(&self, query: &str, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let data = self.data.clone();
        let entity = cx.entity().downgrade();
        let ctx = self.menu_context(cx);
        let searching = !query.is_empty();
        let mut rows = Vec::new();
        for remote in &data.refs.remotes {
            let prefix = format!("{}/", remote.name);
            let tree = branch_tree::build(
                data.refs
                    .remote_branches
                    .iter()
                    .filter(|b| b.name.starts_with(&prefix) && !b.name.ends_with("/HEAD"))
                    .filter(|b| branch_tree::matches(&b.name, query))
                    .map(|b| (b.name[prefix.len()..].to_string(), b)),
            );
            if searching && tree.is_empty() {
                continue;
            }
            let key = format!("remote:{}", remote.name);
            let open = self.folder_open(&key, true, searching);
            let url = remote.url.clone();
            rows.push(
                h_flex()
                    .id(SharedString::from(key.clone()))
                    .h(px(24.))
                    .pl(px(2.))
                    .mx_1()
                    .gap_1()
                    .rounded_md()
                    .text_sm()
                    .cursor_pointer()
                    .text_color(theme::violet())
                    .hover(|s| s.bg(theme::raised()))
                    .child(
                        Icon::new(if open {
                            Lucide::ChevronDown
                        } else {
                            Lucide::ChevronRight
                        })
                        .xsmall(),
                    )
                    .child(
                        Icon::new(if url.contains("github.com") {
                            Lucide::Github
                        } else {
                            Lucide::Cloud
                        })
                        .xsmall(),
                    )
                    .child(div().truncate().child(remote.name.clone()))
                    .tooltip(move |window, cx| Tooltip::new(url.clone()).build(window, cx))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.folders.insert(key.clone(), !open);
                        cx.notify();
                    }))
                    .into_any_element(),
            );
            if !open {
                continue;
            }
            let mut leaf = |node: &TreeNode<&Branch>, b: &&Branch, depth: usize| {
                let b: &Branch = b;
                let id = SharedString::from(format!("remote:{}", b.name));
                let lookup = self.pr_for(&b.name, None, true);
                let pr = menus::found_pr(lookup);
                let suffix = pr_chip(SharedString::from(format!("rpr:{}", b.name)), lookup)
                    .into_iter()
                    .collect();
                let menu_ctx = ctx.clone();
                let (entity, name, target, row_id) =
                    (entity.clone(), b.name.clone(), b.target, id.clone());
                let menu_name = name.clone();
                render_row(Row {
                    view: entity.clone(),
                    quiet: self.menu_open,
                    id: id.clone(),
                    depth,
                    icon: Lucide::GitBranch,
                    icon_color: theme::alpha(theme::violet(), 0.7),
                    label: node.label.clone(),
                    selected: self.selected_ref.as_ref() == Some(&id),
                    bold: false,
                    tooltip: Some(b.name.clone()),
                    suffix,
                    on_click: Box::new(move |ev, window, cx| {
                        let double = ev.click_count() >= 2;
                        let _ = entity.update(cx, |this, cx| {
                            this.selected_ref = Some(row_id.clone());
                            this.select_commit(target, true, cx);
                            this.lookup_pr(&name, None, true, cx);
                            if double {
                                this.request_checkout(name.clone(), true, window, cx);
                            }
                        });
                    }),
                    menu: Some(Rc::new(move |m, _, _| {
                        menus::remote_branch_menu(m, &menu_ctx, &menu_name, pr.as_ref())
                    })),
                })
            };
            self.tree_rows(
                &format!("rb:{}", remote.name),
                &tree,
                1,
                None,
                searching,
                &mut rows,
                &mut leaf,
                cx,
            );
        }
        rows
    }

    fn tag_rows(&self, query: &str, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let entity = cx.entity().downgrade();
        let ctx = self.menu_context(cx);
        self.data
            .refs
            .tags
            .iter()
            .rev()
            .filter(|t| branch_tree::matches(&t.name, query))
            .map(|t| {
                let id = SharedString::from(format!("tag:{}", t.name));
                let (entity, target, row_id) = (entity.clone(), t.target, id.clone());
                let (menu_ctx, tag) = (ctx.clone(), t.name.clone());
                render_row(Row {
                    view: entity.clone(),
                    quiet: self.menu_open,
                    id: id.clone(),
                    depth: 0,
                    icon: Lucide::Tag,
                    icon_color: theme::amber(),
                    label: t.name.clone(),
                    selected: self.selected_ref.as_ref() == Some(&id),
                    bold: false,
                    tooltip: None,
                    suffix: vec![],
                    on_click: Box::new(move |_, _, cx| {
                        let _ = entity.update(cx, |this, cx| {
                            this.selected_ref = Some(row_id.clone());
                            this.select_commit(target, true, cx);
                        });
                    }),
                    menu: Some(Rc::new(move |m, _, _| menus::tag_menu(m, &menu_ctx, &tag))),
                })
            })
            .collect()
    }

    fn worktree_rows(&self, query: &str, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let entity = cx.entity().downgrade();
        self.data
            .refs
            .worktrees
            .iter()
            .filter(|w| {
                branch_tree::matches(
                    &format!("{} {}", w.branch.as_deref().unwrap_or_default(), w.path),
                    query,
                )
            })
            .map(|w| {
                let id = SharedString::from(format!("worktree:{}", w.path));
                let entity = entity.clone();
                let path = PathBuf::from(&w.path);
                let label = w
                    .branch
                    .clone()
                    .unwrap_or_else(|| tr("(détaché)", "(detached)").into());
                let is_current = path == self.repo.root();
                render_row(Row {
                    view: entity.clone(),
                    quiet: self.menu_open,
                    id,
                    depth: 0,
                    icon: Lucide::FolderGit2,
                    icon_color: if is_current {
                        theme::accent()
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
                            tr(
                                "\nCliquer pour ouvrir dans un onglet",
                                "\nClick to open in a tab",
                            )
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
                    menu: None,
                })
            })
            .collect()
    }

    fn stash_rows(&self, query: &str, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let entity = cx.entity().downgrade();
        self.data
            .refs
            .stashes
            .iter()
            .filter(|s| branch_tree::matches(&s.message, query))
            .map(|s| {
                let id = SharedString::from(format!("stash:{}", s.index));
                let (entity, target, row_id) = (entity.clone(), s.id, id.clone());
                render_row(Row {
                    view: entity.clone(),
                    quiet: self.menu_open,
                    id: id.clone(),
                    depth: 0,
                    icon: Lucide::Archive,
                    icon_color: theme::magenta(),
                    label: s.message.clone(),
                    selected: self.selected_ref.as_ref() == Some(&id),
                    bold: false,
                    tooltip: Some(format!("stash@{{{}}}", s.index)),
                    suffix: vec![],
                    on_click: Box::new(move |_, _, cx| {
                        let _ = entity.update(cx, |this, cx| {
                            this.selected_ref = Some(row_id.clone());
                            this.select_commit(target, true, cx);
                        });
                    }),
                    menu: None,
                })
            })
            .collect()
    }

    /// Trash button in the Stashes header: drops every stash after asking.
    fn clear_stashes_button(&self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .id("clear-stashes")
            .test_support()
            .flex_none()
            .p_0p5()
            .rounded_sm()
            .text_color(theme::muted())
            .hover(|s| s.bg(theme::raised()).text_color(theme::red()))
            .child(Icon::new(Lucide::Trash).xsmall())
            .tooltip(|window, cx| {
                Tooltip::new(tr("Supprimer tous les stashes…", "Clear all stashes…"))
                    .build(window, cx)
            })
            .on_click(cx.listener(|this, _, window, cx| {
                cx.stop_propagation();
                let n = this.data.refs.stashes.len();
                this.confirm(
                    trf!("Supprimer les {n} stashes ?", "Clear all {n} stashes?"),
                    tr(
                        "Tous les stashes sont supprimés définitivement.",
                        "Every stash is deleted for good.",
                    )
                    .into(),
                    tr("Tout supprimer", "Clear all"),
                    |this, window, cx| this.run_op("Stash clear", |r| r.stash_clear(), window, cx),
                    window,
                    cx,
                );
            }))
            .into_any_element()
    }

    pub(super) fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let query = self.sidebar_query(cx);
        let searching = !query.is_empty();
        let refs = &self.data.refs;
        let (nb, nr, nt, nw, ns) = (
            refs.branches.len(),
            refs.remote_branches.len(),
            refs.tags.len(),
            refs.worktrees.len(),
            refs.stashes.len(),
        );
        let branches = self.branch_rows(&query, cx);
        let remotes = self.remote_rows(&query, cx);
        let worktrees = self.worktree_rows(&query, cx);
        let tags = self.tag_rows(&query, cx);
        let stashes = self.stash_rows(&query, cx);
        let mut sections = vec![
            self.section(
                "branches",
                "Branches",
                Lucide::GitBranch,
                nb,
                branches,
                searching,
                None,
                cx,
            ),
            self.section(
                "remotes",
                "Remotes",
                Lucide::Cloud,
                nr,
                remotes,
                searching,
                None,
                cx,
            ),
            self.section(
                "worktrees",
                "Worktrees",
                Lucide::FolderTree,
                nw,
                worktrees,
                searching,
                None,
                cx,
            ),
            self.section("tags", "Tags", Lucide::Tags, nt, tags, searching, None, cx),
            self.section(
                "stashes",
                "Stashes",
                Lucide::Archive,
                ns,
                stashes,
                searching,
                (ns > 0).then(|| self.clear_stashes_button(cx)),
                cx,
            ),
        ];
        let order = crate::settings::get().sections();
        let keys = ["branches", "remotes", "worktrees", "tags", "stashes"];
        let mut sections: Vec<(usize, Option<AnyElement>)> = sections
            .drain(..)
            .enumerate()
            .map(|(i, el)| (order.iter().position(|k| *k == keys[i]).unwrap_or(i), el))
            .collect();
        sections.sort_by_key(|(rank, _)| *rank);
        let sections: Vec<AnyElement> = match self.sidebar_tab {
            super::SidebarTab::Repo => sections.into_iter().filter_map(|(_, el)| el).collect(),
            super::SidebarTab::Prs => self.pr_sections(&query, cx),
        };
        let empty = searching && sections.is_empty();
        v_flex()
            .id("sidebar")
            .size_full()
            // Pure window glass: the blur shows through the sidebar.
            .child(self.render_sidebar_tabs(cx))
            .child(
                h_flex()
                    .flex_none()
                    .p_2()
                    .gap_1()
                    .child(
                        div().flex_1().min_w_0().child(
                            Input::new(&self.sidebar_search)
                                .id("sidebar-search")
                                .small()
                                .cleanable(true)
                                .prefix(
                                    Icon::new(Lucide::Search)
                                        .xsmall()
                                        .text_color(theme::muted()),
                                ),
                        ),
                    )
                    .when(self.sidebar_tab == super::SidebarTab::Repo, |s| {
                        s.child(pr_chips_button())
                    }),
            )
            .child(
                v_flex()
                    .id("sidebar-scroll")
                    .flex_1()
                    .min_h_0()
                    .pb_2()
                    .overflow_y_scroll()
                    .children(sections)
                    .when(empty, |s| {
                        s.child(div().p_4().text_sm().text_color(theme::muted()).child(trf!(
                            "Rien ne correspond à « {query} »",
                            "Nothing matches “{query}”"
                        )))
                    }),
            )
    }
}
