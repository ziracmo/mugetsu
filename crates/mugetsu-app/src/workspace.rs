//! Window content: a tab per repository. Only the active tab is rendered
//! (background tabs keep their data but cost nothing to draw), and a 1 s
//! scheduler drives each tab's fetch policy.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, TitleBar, WindowExt as _, h_flex, v_flex};
use gpui_kit::{
    Animation, AnimationExt as _, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, PathPromptOptions, Render, SharedString, StatefulInteractiveElement as _, Styled as _,
    Subscription, Task, Window, div, prelude::FluentBuilder as _, px,
};
use mugetsu_git::Repo;

use crate::repo_view::{RepoEvent, RepoView};
use crate::session::Session;
use crate::theme;

gpui_kit::actions!(workspace, [OpenRepository, CloseTab, NextTab, PreviousTab]);

pub fn init(cx: &mut gpui_kit::App) {
    use gpui_kit::KeyBinding;
    cx.bind_keys([
        KeyBinding::new("secondary-o", OpenRepository, Some("Workspace")),
        KeyBinding::new("secondary-w", CloseTab, Some("Workspace")),
        KeyBinding::new("ctrl-tab", NextTab, Some("Workspace")),
        KeyBinding::new("ctrl-shift-tab", PreviousTab, Some("Workspace")),
        KeyBinding::new("secondary-shift-]", NextTab, Some("Workspace")),
        KeyBinding::new("secondary-shift-[", PreviousTab, Some("Workspace")),
    ]);
}

struct Tab {
    view: Entity<RepoView>,
    _sub: Subscription,
}

pub struct Workspace {
    tabs: Vec<Tab>,
    active: usize,
    window_active: bool,
    persist: bool,
    _scheduler: Task<()>,
    _activation: Subscription,
}

impl Workspace {
    pub fn new(paths: Vec<PathBuf>, active: usize, persist: bool, window: &mut Window, cx: &mut Context<Self>) -> Self {
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
            window_active: true,
            persist,
            _scheduler: scheduler,
            _activation: activation,
        };
        for path in paths {
            if let Err(e) = this.open(path, window, cx) {
                window.push_notification(gpui_kit::component::notification::Notification::error(e.to_string()), cx);
            }
        }
        this.activate(active.min(this.tabs.len().saturating_sub(1)), cx);
        this
    }

    pub fn tabs(&self) -> Vec<Entity<RepoView>> {
        self.tabs.iter().map(|t| t.view.clone()).collect()
    }

    pub fn active(&self) -> Option<&Entity<RepoView>> {
        self.tabs.get(self.active).map(|t| &t.view)
    }

    pub fn active_index(&self) -> usize {
        self.active
    }

    /// Opens a repository (or focuses its tab if already open).
    pub fn open(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) -> anyhow::Result<()> {
        let repo = Repo::open(&path)?;
        if let Some(i) = self.tabs.iter().position(|t| t.view.read(cx).repo().root() == repo.root()) {
            self.activate(i, cx);
            return Ok(());
        }
        let view = cx.new(|cx| RepoView::new(repo, window, cx));
        let sub = cx.subscribe_in(&view, window, |this, _, event: &RepoEvent, window, cx| match event {
            RepoEvent::UpdatesChanged => cx.notify(),
            RepoEvent::OpenWorktree(path) => {
                if let Err(e) = this.open(path.clone(), window, cx) {
                    window.push_notification(gpui_kit::component::notification::Notification::error(e.to_string()), cx);
                }
            }
        });
        self.tabs.push(Tab { view, _sub: sub });
        self.activate(self.tabs.len() - 1, cx);
        Ok(())
    }

    pub fn activate(&mut self, index: usize, cx: &mut Context<Self>) {
        if index >= self.tabs.len() {
            return;
        }
        self.active = index;
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
            tab.view.update(cx, |v, cx| v.set_focused(window_active && i == active, cx));
        }
    }

    fn tick(&mut self, cx: &mut Context<Self>) {
        let now = Instant::now();
        let (active, window_active) = (self.active, self.window_active);
        for (i, tab) in self.tabs.iter().enumerate() {
            tab.view.update(cx, |v, cx| v.tick(window_active && i == active, now, cx));
        }
    }

    fn save(&self, cx: &Context<Self>) {
        if !self.persist {
            return;
        }
        Session {
            repos: self.tabs.iter().map(|t| t.view.read(cx).repo().root().to_path_buf()).collect(),
            active: self.active,
        }
        .save();
    }

    fn prompt_open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: true,
            prompt: Some("Ouvrir".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = paths.await {
                let _ = this.update_in(cx, |this, window, cx| {
                    for p in paths {
                        if let Err(e) = this.open(p, window, cx) {
                            window.push_notification(gpui_kit::component::notification::Notification::error(e.to_string()), cx);
                        }
                    }
                });
            }
        })
        .detach();
    }

    fn render_tab(&self, index: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let view = self.tabs[index].view.read(cx);
        let active = index == self.active;
        let name = view.name();
        let updates = view.has_updates();
        let fetching = view.is_fetching();
        let path = view.repo().root().display().to_string();
        h_flex()
            .id(SharedString::from(format!("tab-{path}")))
            .h(px(30.))
            .px_3()
            .gap_2()
            .flex_none()
            .rounded_t_md()
            .cursor_pointer()
            .text_sm()
            .border_b_2()
            .when(active, |s| s.bg(cx.theme().background).border_color(theme::cyan()).text_color(cx.theme().foreground))
            .when(!active, |s| {
                s.border_color(gpui_kit::transparent_black()).text_color(cx.theme().muted_foreground).hover(|s| s.bg(theme::raised()))
            })
            .child(Icon::new(Lucide::FolderGit2).xsmall().text_color(if active { theme::cyan() } else { theme::muted() }))
            .child(name)
            // Remote changes found by a background fetch: pulsing neon dot.
            .when(updates, |s| {
                s.child(
                    div()
                        .id("updates-dot")
                        .size(px(7.))
                        .rounded_full()
                        .bg(theme::magenta())
                        .with_animation(
                            "updates-pulse",
                            Animation::new(Duration::from_millis(1200)).repeat().with_easing(gpui_kit::pulsating_between(0.35, 1.)),
                            |el, t| el.opacity(t),
                        ),
                )
            })
            .when(fetching && active, |s| {
                s.child(div().size(px(5.)).rounded_full().bg(theme::cyan()).with_animation(
                    "fetch-blink",
                    Animation::new(Duration::from_millis(700)).repeat().with_easing(gpui_kit::pulsating_between(0.2, 1.)),
                    |el, t| el.opacity(t),
                ))
            })
            .child(
                div()
                    .id(SharedString::from(format!("close-{path}")))
                    .rounded_sm()
                    .text_color(theme::muted())
                    .hover(|s| s.text_color(theme::red()).bg(theme::raised()))
                    .child(Icon::new(Lucide::X).xsmall())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.close(index, cx);
                    })),
            )
            .tooltip(move |window, cx| Tooltip::new(path.clone()).build(window, cx))
            .on_click(cx.listener(move |this, _, _, cx| this.activate(index, cx)))
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tabs: Vec<_> = (0..self.tabs.len()).map(|i| self.render_tab(i, cx).into_any_element()).collect();
        let body = match self.active() {
            Some(view) => view.clone().into_any_element(),
            None => v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap_4()
                .child(div().text_3xl().font_weight(gpui_kit::FontWeight::BOLD).text_color(theme::cyan()).child("月 Mugetsu"))
                .child(div().text_color(cx.theme().muted_foreground).child("Ouvre un dépôt Git pour commencer"))
                .child(
                    Button::new("open-first").primary().icon(Icon::new(Lucide::FolderOpen)).label("Ouvrir un dépôt…")
                        .on_click(cx.listener(|this, _, window, cx| this.prompt_open(window, cx))),
                )
                .into_any_element(),
        };
        let _ = window;
        v_flex()
            .id("workspace")
            .key_context("Workspace")
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .on_action(cx.listener(|this, _: &OpenRepository, window, cx| this.prompt_open(window, cx)))
            .on_action(cx.listener(|this, _: &CloseTab, _, cx| this.close(this.active, cx)))
            .on_action(cx.listener(|this, _: &NextTab, _, cx| {
                if !this.tabs.is_empty() {
                    this.activate((this.active + 1) % this.tabs.len(), cx)
                }
            }))
            .on_action(cx.listener(|this, _: &PreviousTab, _, cx| {
                if !this.tabs.is_empty() {
                    this.activate((this.active + this.tabs.len() - 1) % this.tabs.len(), cx)
                }
            }))
            .child(
                TitleBar::new().child(
                    h_flex()
                        .size_full()
                        .gap_1()
                        .items_end()
                        .children(tabs)
                        .child(
                            Button::new("open").ghost().xsmall().icon(Icon::new(Lucide::Plus)).tooltip("Ouvrir un dépôt (⌘O)")
                                .on_click(cx.listener(|this, _, window, cx| this.prompt_open(window, cx))),
                        ),
                ),
            )
            .child(div().flex_1().min_h_0().child(body))
    }
}
