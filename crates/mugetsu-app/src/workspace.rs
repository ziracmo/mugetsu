//! Window content: a tab per repository. Only the active tab is rendered
//! (background tabs keep their data but cost nothing to draw), and a 1 s
//! scheduler drives each tab's fetch policy.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{ContextMenuExt as _, DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{
    ActiveTheme as _, Icon, Sizable as _, TitleBar, WindowExt as _, h_flex, v_flex,
};
use gpui_kit::{
    Animation, AnimationExt as _, AppContext as _, Context, Entity, InteractiveElement as _,
    IntoElement, ParentElement as _, PathPromptOptions, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, Task, Window, div,
    prelude::FluentBuilder as _, px,
};
use mugetsu_git::Repo;

use crate::i18n::tr;
use crate::palette::{Palette, PaletteEvent, SearchBranches, TogglePalette};
use crate::repo_view::{RepoEvent, RepoView};
use crate::session::{RepoSet, Session};
use crate::theme;

gpui_kit::actions!(
    workspace,
    [
        OpenRepository,
        CloseTab,
        NextTab,
        PreviousTab,
        FetchAll,
        ReloadApp
    ]
);

pub fn init(cx: &mut gpui_kit::App) {
    use gpui_kit::KeyBinding;
    cx.bind_keys([
        KeyBinding::new("secondary-o", OpenRepository, Some("Workspace")),
        KeyBinding::new("secondary-t", OpenRepository, Some("Workspace")),
        KeyBinding::new("secondary-w", CloseTab, Some("Workspace")),
        KeyBinding::new("ctrl-tab", NextTab, Some("Workspace")),
        KeyBinding::new("ctrl-shift-tab", PreviousTab, Some("Workspace")),
        KeyBinding::new("secondary-shift-]", NextTab, Some("Workspace")),
        KeyBinding::new("secondary-shift-[", PreviousTab, Some("Workspace")),
        KeyBinding::new("ctrl-r", FetchAll, Some("Workspace")),
        KeyBinding::new("ctrl-shift-r", ReloadApp, Some("Workspace")),
    ]);
}

struct Tab {
    view: Entity<RepoView>,
    _sub: Subscription,
}

pub struct Workspace {
    tabs: Vec<Tab>,
    active: usize,
    /// Named repository lists (workspaces); `current` is the open one,
    /// its `repos` kept in sync with the tabs.
    sets: Vec<RepoSet>,
    current: usize,
    window_active: bool,
    persist: bool,
    /// The opening eclipse playing over the window, keyed so a reload
    /// replays it from the start.
    intro: Option<u64>,
    intro_seq: u64,
    _intro: Task<()>,
    _scheduler: Task<()>,
    _activation: Subscription,
    /// Focus the active tab on the next frame (after a switch, the focused
    /// element may belong to a tab no longer drawn: keys would go nowhere).
    refocus: bool,
    /// ⌃Tab switcher: highlighted tab while the modifier is held.
    switcher: Option<usize>,
    /// ⌘K palette while shown, and where focus goes back afterwards.
    palette: Option<(Entity<Palette>, Subscription)>,
    palette_return: Option<gpui_kit::FocusHandle>,
}

/// Height of the title bar holding the tabs.
pub const TITLE_HEIGHT: f32 = 44.;

impl Workspace {
    pub fn new(
        session: Session,
        persist: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let scheduler = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                if this.update(cx, |this, cx| this.tick(cx)).is_err() {
                    break;
                }
            }
        });
        let activation = cx.observe_window_activation(window, |this, window, cx| {
            this.window_active = window.is_window_active();
            this.sync_focus(cx);
        });
        let mut this = Self {
            tabs: Vec::new(),
            active: 0,
            sets: session.workspaces,
            current: session.current,
            window_active: true,
            persist,
            intro: None,
            intro_seq: 0,
            _intro: Task::ready(()),
            _scheduler: scheduler,
            _activation: activation,
            refocus: true,
            switcher: None,
            palette: None,
            palette_return: None,
        };
        if persist && !cfg!(test) && !crate::settings::get().intro_seen {
            crate::settings::remember(|s| s.intro_seen = true);
            this.play_intro(cx);
        }
        this.open_set(window, cx);
        this
    }

    /// Opens the tabs of the current workspace.
    fn open_set(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let set = self.sets[self.current].clone();
        for path in set.repos {
            if let Err(e) = self.open(path, window, cx) {
                window.push_notification(
                    gpui_kit::component::notification::Notification::error(e.to_string()),
                    cx,
                );
            }
        }
        self.activate(set.active.min(self.tabs.len().saturating_sub(1)), cx);
        cx.notify();
    }

    /// Copies the open tabs into the current workspace.
    fn sync_set(&mut self, cx: &Context<Self>) {
        let repos = self
            .tabs
            .iter()
            .map(|t| t.view.read(cx).repo().root().to_path_buf())
            .collect();
        let set = &mut self.sets[self.current];
        set.repos = repos;
        set.active = self.active;
    }

    pub fn switch_workspace(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if index >= self.sets.len() || index == self.current {
            return;
        }
        self.sync_set(cx);
        self.tabs.clear();
        self.active = 0;
        self.current = index;
        self.open_set(window, cx);
        self.save(cx);
    }

    pub fn new_workspace(&mut self, name: String, window: &mut Window, cx: &mut Context<Self>) {
        self.sets.push(RepoSet {
            name,
            repos: Vec::new(),
            active: 0,
        });
        self.switch_workspace(self.sets.len() - 1, window, cx);
    }

    pub fn rename_workspace(&mut self, name: String, cx: &mut Context<Self>) {
        self.sets[self.current].name = name;
        self.save(cx);
        cx.notify();
    }

    /// Deletes the current workspace (never the last one) and opens the first.
    pub fn delete_workspace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.sets.len() < 2 {
            return;
        }
        self.sets.remove(self.current);
        self.tabs.clear();
        self.active = 0;
        self.current = 0;
        self.open_set(window, cx);
        self.save(cx);
    }

    #[cfg(test)]
    pub fn workspace_names(&self) -> (Vec<String>, usize) {
        (
            self.sets.iter().map(|s| s.name.clone()).collect(),
            self.current,
        )
    }

    #[cfg(test)]
    pub fn tabs(&self) -> Vec<Entity<RepoView>> {
        self.tabs.iter().map(|t| t.view.clone()).collect()
    }

    pub fn active(&self) -> Option<&Entity<RepoView>> {
        self.tabs.get(self.active).map(|t| &t.view)
    }

    #[cfg(test)]
    pub fn active_index(&self) -> usize {
        self.active
    }

    /// Opens a repository (or focuses its tab if already open).
    pub fn open(
        &mut self,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> anyhow::Result<()> {
        let repo = Repo::open(&path)?;
        if let Some(i) = self
            .tabs
            .iter()
            .position(|t| t.view.read(cx).repo().root() == repo.root())
        {
            self.activate(i, cx);
            return Ok(());
        }
        let repo_root = repo.root().to_path_buf();
        let view = cx.new(|cx| RepoView::new(repo, window, cx));
        let sub =
            cx.subscribe_in(
                &view,
                window,
                |this, _, event: &RepoEvent, window, cx| match event {
                    RepoEvent::UpdatesChanged => cx.notify(),
                    RepoEvent::OpenWorktree(path) => {
                        if let Err(e) = this.open(path.clone(), window, cx) {
                            window.push_notification(
                                gpui_kit::component::notification::Notification::error(
                                    e.to_string(),
                                ),
                                cx,
                            );
                        }
                    }
                },
            );
        self.tabs.push(Tab { view, _sub: sub });
        if self.persist {
            let root = repo_root.clone();
            cx.defer(move |cx| crate::settings::update(cx, |s| s.add_recent(root)));
        }
        self.activate(self.tabs.len() - 1, cx);
        Ok(())
    }

    pub fn activate(&mut self, index: usize, cx: &mut Context<Self>) {
        if index >= self.tabs.len() {
            return;
        }
        self.active = index;
        self.refocus = true;
        self.sync_focus(cx);
        self.save(cx);
        cx.notify();
    }

    pub fn close(&mut self, index: usize, cx: &mut Context<Self>) {
        if index >= self.tabs.len() {
            return;
        }
        self.tabs.remove(index);
        if self.active >= self.tabs.len() {
            self.active = self.tabs.len().saturating_sub(1);
        } else if index < self.active {
            self.active -= 1;
        }
        self.sync_focus(cx);
        self.save(cx);
        cx.notify();
    }

    fn sync_focus(&mut self, cx: &mut Context<Self>) {
        let (active, window_active) = (self.active, self.window_active);
        for (i, tab) in self.tabs.iter().enumerate() {
            tab.view
                .update(cx, |v, cx| v.set_focused(window_active && i == active, cx));
        }
    }

    fn tick(&mut self, cx: &mut Context<Self>) {
        let now = Instant::now();
        let (active, window_active) = (self.active, self.window_active);
        for (i, tab) in self.tabs.iter().enumerate() {
            tab.view
                .update(cx, |v, cx| v.tick(window_active && i == active, now, cx));
        }
    }

    fn save(&mut self, cx: &Context<Self>) {
        self.sync_set(cx);
        if self.persist {
            Session::new(self.sets.clone(), self.current).save();
        }
    }

    /// ⌘K: shows the command palette, or closes it. ⌘P (`branches_only`):
    /// the same, narrowed to the branches.
    pub fn toggle_palette(
        &mut self,
        branches_only: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some((palette, _)) = &self.palette {
            palette.update(cx, |p, cx| p.dismiss(cx));
            return;
        }
        self.palette_return = window.focused(cx);
        let entries = crate::palette::entries(self, window, cx);
        let palette = cx.new(|cx| Palette::new(entries, branches_only, window, cx));
        let sub = cx.subscribe_in(
            &palette,
            window,
            |this, palette, event: &PaletteEvent, window, cx| match event {
                PaletteEvent::Run(run) => {
                    this.restore_focus(window, cx);
                    run(this, window, cx);
                }
                PaletteEvent::Dismissed => this.restore_focus(window, cx),
                PaletteEvent::Closed => {
                    if this.palette.as_ref().is_some_and(|(p, _)| p == palette) {
                        this.palette = None;
                        cx.notify();
                    }
                }
            },
        );
        palette.update(cx, |p, cx| p.focus_input(window, cx));
        self.palette = Some((palette, sub));
        cx.notify();
    }

    fn restore_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let handle = self
            .palette_return
            .take()
            .or_else(|| self.active().map(|v| v.read(cx).focus_target()));
        match handle {
            Some(h) => h.focus(window, cx),
            None => window.blur(cx),
        }
    }

    /// Runs `action` where the user was before opening the palette (the
    /// active tab), so repository shortcuts' actions reach it.
    pub fn dispatch(
        &mut self,
        action: Box<dyn gpui_kit::Action>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if window.focused(cx).is_none()
            && let Some(view) = self.active()
        {
            view.read(cx).focus_target().focus(window, cx);
        }
        window.dispatch_action(action, cx);
    }

    #[cfg(test)]
    pub fn palette(&self) -> Option<Entity<Palette>> {
        self.palette.as_ref().map(|(p, _)| p.clone())
    }

    /// Roots of the open tabs, in order.
    pub fn tab_roots(&self, cx: &gpui_kit::App) -> Vec<PathBuf> {
        self.tabs
            .iter()
            .map(|t| t.view.read(cx).repo().root().to_path_buf())
            .collect()
    }

    /// Workspace names and the current one.
    pub fn workspace_list(&self) -> (Vec<String>, usize) {
        (
            self.sets.iter().map(|s| s.name.clone()).collect(),
            self.current,
        )
    }

    /// Opens `path`, or shows why it can't be.
    pub fn open_or_report(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if let Err(e) = self.open(path, window, cx) {
            window.push_notification(
                gpui_kit::component::notification::Notification::error(e.to_string()),
                cx,
            );
        }
    }

    /// ⌘O: the repository picker when there is something to offer, else the
    /// native folder dialog.
    fn prompt_open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let settings = crate::settings::get();
        if settings.recent.is_empty() && settings.repo_folders.is_empty() {
            self.browse(window, cx);
            return;
        }
        let (pick_ws, browse_ws) = (cx.entity().downgrade(), cx.entity().downgrade());
        crate::open_repo::open(
            window,
            cx,
            move |path, window, cx| {
                let _ = pick_ws.update(cx, |ws, cx| ws.open_or_report(path, window, cx));
            },
            move |window, cx| {
                let _ = browse_ws.update(cx, |ws, cx| ws.browse(window, cx));
            },
        );
    }

    fn browse(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: true,
            prompt: Some(tr("Ouvrir", "Open").into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = paths.await {
                let _ = this.update_in(cx, |this, window, cx| {
                    for p in paths {
                        if let Err(e) = this.open(p, window, cx) {
                            window.push_notification(
                                gpui_kit::component::notification::Notification::error(
                                    e.to_string(),
                                ),
                                cx,
                            );
                        }
                    }
                });
            }
        })
        .detach();
    }

    /// Title-bar menu listing the workspaces, plus new / rename / delete.
    fn render_workspace_menu(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let this = cx.entity().downgrade();
        let names: Vec<String> = self.sets.iter().map(|s| s.name.clone()).collect();
        let current = self.current;
        Button::new("workspace-menu")
            .ghost()
            .xsmall()
            .icon(Icon::new(Lucide::LayoutDashboard))
            .label(names[current].clone())
            .tooltip(tr("Workspaces", "Workspaces"))
            .dropdown_menu(move |mut menu, _, _| {
                let act = |f: fn(&mut Workspace, &mut Window, &mut Context<Workspace>)| {
                    let this = this.clone();
                    move |_: &gpui_kit::ClickEvent, window: &mut Window, cx: &mut gpui_kit::App| {
                        let _ = this.update(cx, |ws, cx| f(ws, window, cx));
                    }
                };
                for (i, name) in names.iter().enumerate() {
                    let this = this.clone();
                    menu = menu.item(
                        PopupMenuItem::new(name.clone())
                            .checked(i == current)
                            .on_click(move |_, window, cx| {
                                let _ = this.update(cx, |ws, cx| ws.switch_workspace(i, window, cx));
                            }),
                    );
                }
                menu.separator()
                    .item(
                        PopupMenuItem::new(tr("Nouveau workspace…", "New Workspace…"))
                            .icon(Icon::new(Lucide::Plus))
                            .on_click(act(|_, window, cx| {
                                crate::dialogs::prompt(
                                    tr("Nouveau workspace", "New Workspace").into(),
                                    tr("Nom du workspace", "Workspace name"),
                                    String::new(),
                                    tr("Créer", "Create"),
                                    |ws: &mut Workspace, name, window, cx| {
                                        ws.new_workspace(name, window, cx)
                                    },
                                    window,
                                    cx,
                                );
                            })),
                    )
                    .item(
                        PopupMenuItem::new(tr("Renommer…", "Rename…"))
                            .icon(Icon::new(Lucide::Pencil))
                            .on_click(act(|ws, window, cx| {
                                let name = ws.sets[ws.current].name.clone();
                                crate::dialogs::prompt(
                                    trf!("Renommer '{name}'", "Rename '{name}'"),
                                    tr("Nom du workspace", "Workspace name"),
                                    name,
                                    tr("Renommer", "Rename"),
                                    |ws: &mut Workspace, name, _, cx| ws.rename_workspace(name, cx),
                                    window,
                                    cx,
                                );
                            })),
                    )
                    .item(
                        PopupMenuItem::new(tr("Supprimer…", "Delete…"))
                            .icon(Icon::new(Lucide::Trash))
                            .disabled(names.len() < 2)
                            .on_click(act(|ws, window, cx| {
                                let name = ws.sets[ws.current].name.clone();
                                crate::dialogs::confirm(
                                    trf!(
                                        "Supprimer le workspace '{name}' ?",
                                        "Delete workspace '{name}'?"
                                    ),
                                    tr(
                                        "Les dépôts restent sur le disque ; seule la liste est supprimée.",
                                        "Repositories stay on disk; only the list is deleted.",
                                    )
                                    .into(),
                                    tr("Supprimer", "Delete"),
                                    |ws: &mut Workspace, window, cx| ws.delete_workspace(window, cx),
                                    window,
                                    cx,
                                );
                            })),
                    )
            })
    }

    /// Safari-like tab: equal widths, frosted pill for the active one,
    /// close button on hover, repository + current branch.
    fn render_tab(&self, index: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let view = self.tabs[index].view.read(cx);
        let active = index == self.active;
        let name = view.name();
        let branch = view.head_label();
        let updates = view.has_updates();
        let fetching = view.is_fetching();
        let root = view.repo().root().to_path_buf();
        let path = root.display().to_string();
        let dirty = view.uncommitted();
        let emoji = crate::settings::get().repo_icons.get(&root).cloned();
        let group = SharedString::from(format!("tab-group-{index}"));
        // A thin divider between two inactive neighbours, like macOS tabs.
        let divider = index > 0 && !active && index != self.active + 1;
        h_flex()
            .id(SharedString::from(format!("tab-{name}")))
            .test_support()
            .group(group.clone())
            .relative()
            .h(px(28.))
            .flex_1()
            .min_w(px(110.))
            .max_w(px(260.))
            .px_2()
            .gap_2()
            .items_center()
            .rounded_lg()
            .cursor_pointer()
            .text_sm()
            .when(active, |s| {
                s.bg(theme::alpha(gpui_kit::white(), 0.11))
                    .border_1()
                    .border_color(theme::glass_edge())
                    .shadow_md()
                    .text_color(gpui_kit::white())
            })
            .when(!active, |s| {
                s.text_color(theme::muted())
                    .hover(|s| s.bg(theme::alpha(gpui_kit::white(), 0.05)))
            })
            .when(divider, |s| {
                s.child(
                    div()
                        .absolute()
                        .left(px(-2.))
                        .top(px(8.))
                        .h(px(14.))
                        .w(px(1.))
                        .bg(theme::hairline()),
                )
            })
            // Close button: always on the active tab, on hover elsewhere.
            .child(
                div()
                    .id(SharedString::from(format!("close-{name}")))
                    .test_support()
                    .flex_none()
                    .size(px(16.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_sm()
                    .when(!active, |s| {
                        s.opacity(0.).group_hover(group.clone(), |s| s.opacity(1.))
                    })
                    .text_color(theme::muted())
                    .hover(|s| s.bg(theme::raised()).text_color(gpui_kit::white()))
                    .child(Icon::new(Lucide::X).xsmall())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.close(index, cx);
                    })),
            )
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .justify_center()
                    .gap_1p5()
                    .items_center()
                    .when_some(emoji, |s, e| {
                        s.child(
                            div()
                                .flex_none()
                                .h(px(16.))
                                .flex()
                                .items_center()
                                .line_height(px(16.))
                                .child(crate::repo_icon::view(&e, 15.)),
                        )
                    })
                    // Name and branch share a baseline (different font sizes).
                    .child(
                        h_flex()
                            .min_w_0()
                            .items_baseline()
                            .gap_1p5()
                            .line_height(px(16.))
                            .child(
                                div()
                                    .flex_none()
                                    .truncate()
                                    .font_weight(if active {
                                        gpui_kit::FontWeight::SEMIBOLD
                                    } else {
                                        gpui_kit::FontWeight::NORMAL
                                    })
                                    .child(name.clone()),
                            )
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .text_xs()
                                    .text_color(theme::muted())
                                    .child(branch),
                            ),
                    )
                    // Uncommitted changes in this repository.
                    .when(dirty > 0, |s| {
                        s.child(
                            Icon::new(Lucide::Pencil)
                                .xsmall()
                                .flex_none()
                                .text_color(theme::amber()),
                        )
                    }),
            )
            .child(
                div()
                    .flex_none()
                    .w(px(16.))
                    .flex()
                    .justify_center()
                    // Remote changes found by a background fetch: pulsing dot.
                    .when(updates, |s| {
                        s.child(
                            div()
                                .id(SharedString::from(format!("updates-{name}")))
                                .test_support()
                                .size(px(7.))
                                .rounded_full()
                                .bg(theme::magenta())
                                .with_animation(
                                    "updates-pulse",
                                    Animation::new(Duration::from_millis(1200))
                                        .repeat()
                                        .with_easing(gpui_kit::pulsating_between(0.35, 1.)),
                                    |el, t| el.opacity(t),
                                ),
                        )
                    })
                    .when(fetching && active && !updates, |s| {
                        s.child(
                            div()
                                .size(px(5.))
                                .rounded_full()
                                .bg(theme::accent())
                                .with_animation(
                                    "fetch-blink",
                                    Animation::new(Duration::from_millis(700))
                                        .repeat()
                                        .with_easing(gpui_kit::pulsating_between(0.2, 1.)),
                                    |el, t| el.opacity(t),
                                ),
                        )
                    }),
            )
            .tooltip(move |window, cx| {
                let tip = if dirty > 0 {
                    format!(
                        "{path}\n{}",
                        trf!(
                            "{dirty} fichier(s) non commité(s)",
                            "{dirty} uncommitted file(s)"
                        )
                    )
                } else {
                    path.clone()
                };
                Tooltip::new(tip).build(window, cx)
            })
            .on_click(cx.listener(move |this, _, _, cx| this.activate(index, cx)))
            .context_menu(move |menu, _, _| icon_menu(menu, root.clone()))
    }
}

/// Right-click on a tab.
fn icon_menu(
    menu: gpui_kit::component::menu::PopupMenu,
    root: PathBuf,
) -> gpui_kit::component::menu::PopupMenu {
    menu.item(
        PopupMenuItem::new(tr("Choisir une icône…", "Choose Icon…"))
            .icon(Icon::new(Lucide::Palette))
            .on_click(move |_, window, cx| crate::repo_icon::open_picker(root.clone(), window, cx)),
    )
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tabs: Vec<_> = (0..self.tabs.len())
            .map(|i| self.render_tab(i, cx).into_any_element())
            .collect();
        let body = match self.active() {
            Some(view) => view.clone().into_any_element(),
            None => {
                let open = cx.entity().downgrade();
                let recent = cx.entity().downgrade();
                crate::welcome::empty_state(
                    &crate::settings::get().recent,
                    std::rc::Rc::new(move |window, cx| {
                        let _ = open.update(cx, |this, cx| this.prompt_open(window, cx));
                    }),
                    std::rc::Rc::new(move |path, window, cx| {
                        let _ = recent.update(cx, |this, cx| this.open_or_report(path, window, cx));
                    }),
                )
            }
        };
        if std::mem::take(&mut self.refocus)
            && let Some(view) = self.active()
        {
            let handle = view.read(cx).history_focus_handle();
            handle.focus(window, cx);
        }
        let switcher = self.render_switcher(cx);
        v_flex()
            .id("workspace")
            .relative()
            .key_context("Workspace")
            .size_full()
            .text_color(cx.theme().foreground)
            .on_action(
                cx.listener(|this, _: &OpenRepository, window, cx| this.prompt_open(window, cx)),
            )
            .on_action(cx.listener(|this, _: &CloseTab, _, cx| this.close(this.active, cx)))
            .on_action(cx.listener(|this, _: &FetchAll, _, cx| this.fetch_all(cx)))
            .on_action(cx.listener(|this, _: &ReloadApp, _, cx| this.reload_app(cx)))
            .on_action(cx.listener(|this, _: &TogglePalette, window, cx| {
                this.toggle_palette(false, window, cx)
            }))
            .on_action(cx.listener(|this, _: &SearchBranches, window, cx| {
                this.toggle_palette(true, window, cx)
            }))
            .on_action(
                cx.listener(|this, _: &NextTab, window, cx| this.step_switcher(true, window, cx)),
            )
            .on_action(cx.listener(|this, _: &PreviousTab, window, cx| {
                this.step_switcher(false, window, cx)
            }))
            // Releasing the modifier picks the highlighted tab, like Dia / ⌘Tab.
            .on_modifiers_changed(cx.listener(
                |this, ev: &gpui_kit::ModifiersChangedEvent, _, cx| {
                    if this.switcher.is_some() && !ev.modifiers.control && !ev.modifiers.platform {
                        this.commit_switcher(cx);
                    }
                },
            ))
            .child(
                TitleBar::new().h(px(TITLE_HEIGHT)).child(
                    h_flex()
                        .size_full()
                        .pl_1()
                        .pr_3()
                        .py_2()
                        .gap_1p5()
                        .items_center()
                        .child(self.render_workspace_menu(cx))
                        .child(
                            div()
                                .flex_none()
                                .w(px(1.))
                                .h(px(16.))
                                .mx_1()
                                .bg(theme::hairline()),
                        )
                        .children(tabs)
                        .child(
                            Button::new("open")
                                .ghost()
                                .xsmall()
                                .icon(Icon::new(Lucide::Plus))
                                .tooltip(tr("Ouvrir un dépôt (⌘O)", "Open repository (⌘O)"))
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.prompt_open(window, cx)),
                                ),
                        ),
                ),
            )
            .child(div().flex_1().min_h_0().child(body))
            .children(switcher)
            .children(self.intro.map(crate::welcome::eclipse))
            .children(self.palette.as_ref().map(|(p, _)| p.clone()))
    }
}

impl Workspace {
    /// Plays the opening eclipse over the window, from the start.
    pub fn play_intro(&mut self, cx: &mut Context<Self>) {
        self.intro_seq += 1;
        let seq = self.intro_seq;
        self.intro = Some(seq);
        self._intro = cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(crate::welcome::ECLIPSE)
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.intro == Some(seq) {
                    this.intro = None;
                    cx.notify();
                }
            });
        });
        cx.notify();
    }

    /// ⌃R: fetches every repository open in the window, each button
    /// showing its own outcome.
    pub fn fetch_all(&mut self, cx: &mut Context<Self>) {
        for tab in &self.tabs {
            tab.view.update(cx, |view, cx| view.fetch_now(cx));
        }
    }

    /// ⌃⇧R: reloads every repository and replays the opening.
    pub fn reload_app(&mut self, cx: &mut Context<Self>) {
        for tab in &self.tabs {
            tab.view.update(cx, |view, cx| view.reload(cx));
        }
        self.play_intro(cx);
    }

    pub fn intro(&self) -> Option<u64> {
        self.intro
    }

    /// ⌃Tab / ⌃⇧Tab: opens the switcher or moves its highlight.
    fn step_switcher(&mut self, forward: bool, window: &Window, cx: &mut Context<Self>) {
        let n = self.tabs.len();
        if n < 2 {
            return;
        }
        let from = self.switcher.unwrap_or(self.active);
        let next = if forward {
            (from + 1) % n
        } else {
            (from + n - 1) % n
        };
        // No modifier held (menu, test…): switch at once.
        let held = window.modifiers();
        if !held.control && !held.platform {
            self.switcher = None;
            self.activate(next, cx);
            return;
        }
        self.switcher = Some(next);
        cx.notify();
    }

    fn commit_switcher(&mut self, cx: &mut Context<Self>) {
        if let Some(index) = self.switcher.take() {
            self.activate(index, cx);
        }
        cx.notify();
    }

    /// Dia-like overlay: a card per open repository, the highlighted one framed.
    fn render_switcher(&self, cx: &mut Context<Self>) -> Option<gpui_kit::AnyElement> {
        let selected = self.switcher?;
        let icons = crate::settings::get().repo_icons;
        let cards = self.tabs.iter().enumerate().map(|(i, tab)| {
            let view = tab.view.read(cx);
            let name = view.name();
            let branch = view.head_label();
            let dirty = view.uncommitted();
            let root = view.repo().root().to_path_buf();
            let emoji = icons.get(&root).cloned();
            let active = i == selected;
            v_flex()
                .id(("switch-card", i))
                .test_support()
                .w(px(170.))
                .p_1p5()
                .gap_1p5()
                .rounded_xl()
                .cursor_pointer()
                .when(active, |s| s.bg(theme::alpha(gpui_kit::white(), 0.14)))
                .hover(|s| s.bg(theme::alpha(gpui_kit::white(), 0.08)))
                .child(
                    v_flex()
                        .h(px(92.))
                        .items_center()
                        .justify_center()
                        .gap_1()
                        .rounded_lg()
                        .border_1()
                        .border_color(if active {
                            theme::alpha(theme::accent(), 0.8)
                        } else {
                            theme::hairline()
                        })
                        .bg(theme::sheet())
                        .child(match &emoji {
                            Some(e) => crate::repo_icon::view(e, 34.),
                            None => div()
                                .text_3xl()
                                .child(
                                    name.chars()
                                        .next()
                                        .unwrap_or('?')
                                        .to_uppercase()
                                        .to_string(),
                                )
                                .into_any_element(),
                        })
                        .child(
                            h_flex()
                                .gap_1()
                                .text_xs()
                                .text_color(theme::muted())
                                .child(Icon::new(Lucide::GitBranch).xsmall())
                                .child(div().max_w(px(130.)).truncate().child(branch))
                                .when(dirty > 0, |s| {
                                    s.child(
                                        Icon::new(Lucide::Pencil)
                                            .xsmall()
                                            .text_color(theme::amber()),
                                    )
                                }),
                        ),
                )
                .child(
                    div()
                        .px_1()
                        .text_sm()
                        .truncate()
                        .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                        .text_color(if active {
                            gpui_kit::white()
                        } else {
                            theme::muted()
                        })
                        .child(name),
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.switcher = Some(i);
                    this.commit_switcher(cx);
                }))
        });
        Some(
            div()
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .child(
                    h_flex()
                        .id("tab-switcher")
                        .test_support()
                        .max_w(px(1100.))
                        .flex_wrap()
                        .justify_center()
                        .p_3()
                        .gap_2()
                        .rounded_2xl()
                        .bg(theme::popover())
                        .border_1()
                        .border_color(theme::glass_edge())
                        .shadow_2xl()
                        .children(cards),
                )
                .into_any_element(),
        )
    }
}
