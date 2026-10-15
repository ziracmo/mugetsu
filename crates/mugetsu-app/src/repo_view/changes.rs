use gpui_kit::assets::IconName as Lucide;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::diff::{Diff, DiffFile as UiDiffFile};
use gpui_kit::component::input::Textarea;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, Sizable as _, WindowExt as _, h_flex, v_flex,
};
use gpui_kit::{
    AnyElement, AppContext as _, Context, InteractiveElement as _, IntoElement, ParentElement as _,
    SharedString, StatefulInteractiveElement as _, Styled as _, Window, div,
    prelude::FluentBuilder as _, px,
};
use mugetsu_git::{Change, FileStatus};

use super::{RepoView, SelectedChange};
use crate::i18n::tr;
use crate::theme;

fn change_color(c: Change) -> gpui_kit::Hsla {
    match c {
        Change::Added | Change::Untracked => theme::lime(),
        Change::Deleted => theme::red(),
        Change::Conflicted => theme::magenta(),
        Change::Renamed | Change::Copied => theme::violet(),
        _ => theme::amber(),
    }
}

impl RepoView {
    pub(super) fn select_first_change(&mut self, cx: &mut Context<Self>) {
        let first = self
            .data
            .status
            .files
            .iter()
            .find(|f| f.is_unstaged())
            .map(|f| SelectedChange {
                path: f.path.clone(),
                staged: false,
            })
            .or_else(|| {
                self.data.status.staged().next().map(|f| SelectedChange {
                    path: f.path.clone(),
                    staged: true,
                })
            });
        match first {
            Some(sel) => self.load_change(sel, cx),
            None => {
                self.selected_change = None;
                self.change_hunks = None;
                self.change_diff.update(cx, |d, cx| d.set_files([], cx));
            }
        }
    }

    pub(super) fn load_change(&mut self, sel: SelectedChange, cx: &mut Context<Self>) {
        self.selected_change = Some(sel.clone());
        let untracked = self
            .data
            .status
            .files
            .iter()
            .any(|f| f.path == sel.path && f.worktree == Change::Untracked);
        let repo = self.repo.clone();
        let wanted = sel.clone();
        self._detail = cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let diff = if untracked && !sel.staged {
                        repo.diff_untracked(&sel.path)?
                    } else {
                        repo.diff_worktree(sel.staged, Some(&sel.path))?
                    };
                    let patch: String = diff
                        .files
                        .iter()
                        .flat_map(|f| {
                            (0..f.hunks.len().max(1)).filter_map(move |i| {
                                if f.hunks.is_empty() {
                                    Some(f.header.join("\n") + "\n")
                                } else {
                                    f.hunk_patch(i)
                                }
                            })
                        })
                        .collect();
                    let ui = UiDiffFile::parse(&patch).unwrap_or_default();
                    anyhow::Ok((diff, ui))
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.selected_change.as_ref() != Some(&wanted) {
                    return;
                }
                if let Ok((diff, ui)) = result {
                    this.change_hunks = Some(diff);
                    this.change_diff.update(cx, |d, cx| d.set_files(ui, cx));
                }
                cx.notify();
            });
        });
        cx.notify();
    }

    fn file_row(&self, f: &FileStatus, staged: bool, cx: &mut Context<Self>) -> AnyElement {
        let change = if staged { f.index } else { f.worktree };
        let color = change_color(change);
        let sel = SelectedChange {
            path: f.path.clone(),
            staged,
        };
        let is_selected = self.selected_change.as_ref() == Some(&sel);
        let (dir, name) = f
            .path
            .rsplit_once('/')
            .map_or(("", f.path.as_str()), |(d, n)| (d, n));
        let path = f.path.clone();
        let id = SharedString::from(format!(
            "{}:{}",
            if staged { "staged" } else { "unstaged" },
            f.path
        ));
        h_flex()
            .id(id)
            .test_support()
            .h(px(24.))
            .px_3()
            .gap_2()
            .w_full()
            .min_w_0()
            .text_sm()
            .cursor_pointer()
            .border_l_2()
            .border_color(gpui_kit::transparent_black())
            .hover(|s| s.bg(theme::raised()))
            .when(is_selected, |s| {
                s.bg(theme::alpha(theme::accent(), 0.12))
                    .border_color(theme::accent())
            })
            .child(
                div()
                    .w(px(14.))
                    .flex_none()
                    .text_xs()
                    .text_color(color)
                    .font_weight(gpui_kit::FontWeight::BOLD)
                    .child(change.letter()),
            )
            .child(
                div()
                    .flex_none()
                    .max_w(px(260.))
                    .truncate()
                    .child(name.to_string()),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(dir.to_string()),
            )
            .child(
                Button::new(SharedString::from(format!("toggle:{staged}:{}", f.path)))
                    .ghost()
                    .xsmall()
                    .icon(Icon::new(if staged { Lucide::Minus } else { Lucide::Plus }))
                    .tooltip(if staged {
                        tr("Retirer de l'index", "Unstage")
                    } else {
                        tr("Ajouter à l'index", "Stage")
                    })
                    .on_click(cx.listener({
                        let path = path.clone();
                        move |this, _, window, cx| {
                            let path = path.clone();
                            this.run_op(
                                if staged { "Unstage" } else { "Stage" },
                                move |r| {
                                    if staged {
                                        r.unstage(&[&path])
                                    } else {
                                        r.stage(&[&path])
                                    }
                                },
                                window,
                                cx,
                            );
                        }
                    })),
            )
            .on_click(
                cx.listener(move |this, ev: &gpui_kit::ClickEvent, window, cx| {
                    if ev.click_count() >= 2 {
                        let path = path.clone();
                        this.run_op(
                            "Stage",
                            move |r| {
                                if staged {
                                    r.unstage(&[&path])
                                } else {
                                    r.stage(&[&path])
                                }
                            },
                            window,
                            cx,
                        );
                    } else {
                        this.load_change(sel.clone(), cx);
                    }
                }),
            )
            .into_any_element()
    }

    fn list_header(
        &self,
        title: &str,
        count: usize,
        action: Option<AnyElement>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        h_flex()
            .h(px(30.))
            .flex_none()
            .px_3()
            .gap_2()
            .bg(theme::panel())
            .border_b_1()
            .border_color(cx.theme().border)
            .text_xs()
            .text_color(cx.theme().muted_foreground)
            .child(title.to_uppercase())
            .child(
                div()
                    .px_1p5()
                    .rounded_sm()
                    .bg(theme::raised())
                    .child(count.to_string()),
            )
            .child(div().flex_1())
            .children(action)
            .into_any_element()
    }

    fn render_hunk_bar(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let sel = self.selected_change.clone()?;
        let diff = self.change_hunks.as_ref()?;
        let file = diff.files.first()?.clone();
        let untracked = self
            .data
            .status
            .files
            .iter()
            .any(|f| f.path == sel.path && f.worktree == Change::Untracked);
        if file.hunks.len() < 2 || untracked {
            return None;
        }
        let staged = sel.staged;
        Some(
            h_flex()
                .flex_none()
                .flex_wrap()
                .gap_1()
                .px_3()
                .py_1()
                .border_b_1()
                .border_color(cx.theme().border)
                .children(file.hunks.iter().enumerate().map(|(i, h)| {
                    let file = file.clone();
                    Button::new(SharedString::from(format!("hunk-{i}")))
                        .xsmall()
                        .outline()
                        .label(format!("{} hunk {}", if staged { "−" } else { "+" }, i + 1))
                        .tooltip(h.header.clone())
                        .on_click(cx.listener(move |this, _, window, cx| {
                            let file = file.clone();
                            this.run_op(
                                "Hunk",
                                move |r| r.apply_hunk(&file, i, staged),
                                window,
                                cx,
                            );
                        }))
                })),
        )
        .map(|e| e.into_any_element())
    }

    pub(super) fn render_changes(
        &self,
        _window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let status = self.data.status.clone();
        let unstaged: Vec<_> = status.unstaged().cloned().collect();
        let staged: Vec<_> = status.staged().cloned().collect();
        let can_commit = (!staged.is_empty() || self.amend) && self.busy.is_none();
        let unstaged_rows: Vec<_> = unstaged
            .iter()
            .map(|f| self.file_row(f, false, cx))
            .collect();
        let staged_rows: Vec<_> = staged.iter().map(|f| self.file_row(f, true, cx)).collect();

        let left = v_flex()
            .w(px(380.))
            .flex_none()
            .h_full()
            .border_r_1()
            .border_color(cx.theme().border)
            .child(self.list_header(
                tr("Non indexés", "Unstaged"),
                unstaged.len(),
                (!unstaged.is_empty()).then(|| {
                    Button::new("stage-all")
                        .xsmall()
                        .ghost()
                        .label(tr("Tout indexer", "Stage all"))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.run_op("Stage all", |r| r.stage_all(), window, cx)
                        }))
                        .into_any_element()
                }),
                cx,
            ))
            .child(
                div()
                    .id("unstaged")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .children(unstaged_rows),
            )
            .child(self.list_header(
                tr("Indexés", "Staged"),
                staged.len(),
                (!staged.is_empty()).then(|| {
                    let paths: Vec<String> = staged.iter().map(|f| f.path.clone()).collect();
                    Button::new("unstage-all")
                        .xsmall()
                        .ghost()
                        .label(tr("Tout retirer", "Unstage all"))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            let paths = paths.clone();
                            this.run_op(
                                "Unstage all",
                                move |r| {
                                    r.unstage(&paths.iter().map(String::as_str).collect::<Vec<_>>())
                                },
                                window,
                                cx,
                            )
                        }))
                        .into_any_element()
                }),
                cx,
            ))
            .child(
                div()
                    .id("staged")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .children(staged_rows),
            )
            .child(
                v_flex()
                    .flex_none()
                    .p_3()
                    .gap_2()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .bg(theme::panel())
                    .child(
                        div()
                            .id("commit-message")
                            .test_support()
                            .child(Textarea::new(&self.message)),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                Checkbox::new("amend")
                                    .label("Amend")
                                    .checked(self.amend)
                                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                        this.amend = *checked;
                                        cx.notify();
                                    })),
                            )
                            .child(div().flex_1())
                            .child(
                                Button::new("commit")
                                    .primary()
                                    .small()
                                    .icon(Icon::new(Lucide::GitCommitHorizontal))
                                    .label(if staged.is_empty() {
                                        "Commit".to_string()
                                    } else {
                                        trf!(
                                            "Commit {} fichier(s)",
                                            "Commit {} file(s)",
                                            staged.len()
                                        )
                                    })
                                    .disabled(!can_commit)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.commit(false, window, cx)
                                    })),
                            ),
                    ),
            );

        let right = v_flex()
            .flex_1()
            .min_w_0()
            .h_full()
            .children(self.render_hunk_bar(cx))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .p_2()
                    .child(if self.selected_change.is_some() {
                        Diff::new(&self.change_diff)
                            .line_number(true)
                            .into_any_element()
                    } else {
                        v_flex()
                            .size_full()
                            .items_center()
                            .justify_center()
                            .gap_2()
                            .text_color(cx.theme().muted_foreground)
                            .child(
                                Icon::new(Lucide::CircleCheck)
                                    .large()
                                    .text_color(theme::lime()),
                            )
                            .child(tr("Copie de travail propre", "Working copy clean"))
                            .into_any_element()
                    }),
            );

        h_flex().size_full().min_h_0().child(left).child(right)
    }

    /// Commits the staged changes; `push` also pushes HEAD afterwards (⇧⌘↩).
    pub(super) fn commit(&mut self, push: bool, window: &mut Window, cx: &mut Context<Self>) {
        let message = self.message.read(cx).value().to_string();
        if message.trim().is_empty() {
            window.push_notification(
                gpui_kit::component::notification::Notification::warning(tr(
                    "Message de commit vide",
                    "Empty commit message",
                )),
                cx,
            );
            return;
        }
        let amend = self.amend;
        self.run_op_then(
            "Commit",
            move |r| r.commit(&message, amend),
            move |this, window, cx| {
                this.message.update(cx, |m, cx| m.set_value("", window, cx));
                this.amend = false;
                if push {
                    // run_op_then reloads after this callback; queue the push behind it.
                    let entity = cx.entity().downgrade();
                    window.defer(cx, move |window, cx| {
                        let _ = entity.update(cx, |this, cx| this.push_head(window, cx));
                    });
                }
            },
            window,
            cx,
        );
    }
}
