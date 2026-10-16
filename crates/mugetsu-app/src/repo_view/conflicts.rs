//! Built-in conflict resolution: the operation banner (continue, skip,
//! abort) and a three-pane resolver, ours | result | theirs, aligned chunk by
//! chunk. A conflicting chunk is settled by clicking a side (or both, or
//! neither); the result can also be edited by hand before marking the file
//! resolved.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Textarea, TextareaState};
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    Animation, AnimationExt as _, AnyElement, App, AppContext as _, Context, Entity, Hsla,
    InteractiveElement as _, IntoElement, ParentElement as _, ScrollHandle, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window, div, prelude::FluentBuilder as _, px,
    relative,
};
use mugetsu_git::{Chunk, ConflictFile, Operation, Resolution};

use super::{Mode, RepoView, SelectedChange};
use crate::i18n::tr;
use crate::theme;

/// Clean runs longer than this fold to their first and last lines.
const FOLD_OVER: usize = 10;
const FOLD_KEEP: usize = 3;

/// The resolver's state for one conflicted file.
pub(super) struct ConflictEditor {
    pub path: String,
    /// `None` while the versions load.
    file: Option<Arc<ConflictFile>>,
    error: Option<String>,
    /// One per conflicting chunk, in order.
    choices: Vec<Option<Resolution>>,
    /// Clean chunks unfolded by the user (chunk index).
    unfolded: HashSet<usize>,
    /// Hand-editing the result instead of picking sides.
    editing: Option<Entity<TextareaState>>,
    /// Last pick (conflict index, sequence), for its entrance animation.
    picked: Option<(usize, usize)>,
    seq: usize,
    scroll: ScrollHandle,
}

impl ConflictEditor {
    fn new(path: String) -> Self {
        Self {
            path,
            file: None,
            error: None,
            choices: Vec::new(),
            unfolded: HashSet::new(),
            editing: None,
            picked: None,
            seq: 0,
            scroll: ScrollHandle::new(),
        }
    }

    fn resolved(&self) -> usize {
        self.choices.iter().filter(|c| c.is_some()).count()
    }

    fn all_resolved(&self) -> bool {
        self.choices.iter().all(Option::is_some)
    }

    /// Chunk index of the n-th conflict.
    fn chunk_of(&self, conflict: usize) -> Option<usize> {
        let file = self.file.as_ref()?;
        file.chunks
            .iter()
            .enumerate()
            .filter(|(_, c)| matches!(c, Chunk::Conflict(_)))
            .nth(conflict)
            .map(|(i, _)| i)
    }
}

/// `Ok(false)` from a step that stopped on (new) conflicts becomes an error,
/// which [`RepoView::run_op`] reports as conflicts to resolve.
pub(super) fn stopped_on_conflicts(done: bool) -> anyhow::Result<()> {
    if done {
        Ok(())
    } else {
        anyhow::bail!("conflicts")
    }
}

/// Ours: the lane cyan; theirs: magenta. Result: the accent.
fn ours_color() -> Hsla {
    theme::lane_color(0)
}
fn theirs_color() -> Hsla {
    theme::magenta()
}

fn side_color(r: Resolution, first: bool) -> Hsla {
    match (r, first) {
        (Resolution::Ours | Resolution::OursThenTheirs, true)
        | (Resolution::TheirsThenOurs, false) => ours_color(),
        _ => theirs_color(),
    }
}

/// Display text of a line: no line ending, tabs as spaces, never empty (so
/// the row keeps its height).
fn shown(line: &str) -> String {
    let l = line.trim_end_matches(['\n', '\r']).replace('\t', "    ");
    if l.is_empty() { " ".into() } else { l }
}

/// Lines with a gutter of numbers starting at `first`.
fn code(lines: &[&str], first: usize, cx: &App) -> AnyElement {
    v_flex()
        .w_full()
        .py(px(2.))
        .font_family(cx.theme().mono_font_family.clone())
        .text_xs()
        .children(lines.iter().enumerate().map(|(i, l)| {
            h_flex()
                .items_start()
                .w_full()
                .min_w_0()
                .child(
                    div()
                        .w(px(34.))
                        .flex_none()
                        .pr_2()
                        .text_right()
                        .text_color(theme::alpha(theme::muted(), 0.6))
                        .child((first + i).to_string()),
                )
                .child(div().flex_1().min_w_0().child(shown(l)))
        }))
        .into_any_element()
}

impl RepoView {
    /// Shows the resolver: on `path`, or on the first conflicted file. Entry
    /// point for anything that leaves the repository with conflicts (merge,
    /// rebase, cherry-pick, stash pop…).
    pub fn open_conflicts(&mut self, path: Option<String>, cx: &mut Context<Self>) {
        let path = path.or_else(|| self.data.conflicted().next().map(str::to_string));
        self.mode = Mode::Changes;
        if let Some(path) = path {
            self.load_change(
                SelectedChange {
                    path,
                    staged: false,
                },
                cx,
            );
        }
        cx.notify();
    }

    /// File open in the resolver, once its versions are loaded.
    #[cfg(test)]
    pub fn conflict_loaded(&self) -> Option<&str> {
        self.conflict
            .as_ref()
            .filter(|c| c.file.is_some())
            .map(|c| c.path.as_str())
    }

    pub(super) fn is_conflicted(&self, path: &str) -> bool {
        self.data.conflicted().any(|p| p == path)
    }

    /// Loads the three versions of `path` into the resolver.
    pub(super) fn load_conflict(&mut self, path: String, cx: &mut Context<Self>) {
        if self.conflict.as_ref().is_some_and(|c| c.path == path) {
            // Same file (reload): keep the picks made so far.
            cx.notify();
            return;
        }
        self.conflict = Some(ConflictEditor::new(path.clone()));
        let repo = self.repo.clone();
        self._detail = cx.spawn(async move |this, cx| {
            let wanted = path.clone();
            let result = cx
                .background_spawn(async move { repo.conflict_file(&path) })
                .await;
            let _ = this.update(cx, |this, cx| {
                let Some(editor) = this.conflict.as_mut().filter(|c| c.path == wanted) else {
                    return;
                };
                match result {
                    Ok(file) => {
                        editor.choices = vec![None; file.conflict_count()];
                        editor.file = Some(Arc::new(file));
                    }
                    Err(e) => editor.error = Some(format!("{e:#}")),
                }
                cx.notify();
            });
        });
        cx.notify();
    }

    fn pick(&mut self, conflict: usize, r: Option<Resolution>, cx: &mut Context<Self>) {
        let Some(editor) = self.conflict.as_mut() else {
            return;
        };
        if let Some(slot) = editor.choices.get_mut(conflict) {
            *slot = r;
            editor.seq += 1;
            editor.picked = r.map(|_| (conflict, editor.seq));
        }
        cx.notify();
    }

    fn pick_all(&mut self, r: Resolution, cx: &mut Context<Self>) {
        if let Some(editor) = self.conflict.as_mut() {
            editor.seq += 1;
            editor.picked = None;
            editor.choices.iter_mut().for_each(|c| *c = Some(r));
        }
        cx.notify();
    }

    /// Scrolls to the next (or previous) unresolved conflict after the one
    /// last picked.
    fn jump(&mut self, forward: bool, cx: &mut Context<Self>) {
        let Some(editor) = self.conflict.as_ref() else {
            return;
        };
        let n = editor.choices.len();
        if n == 0 {
            return;
        }
        let from = editor
            .picked
            .map_or(if forward { n - 1 } else { 0 }, |(i, _)| i);
        let next = (1..=n)
            .map(|k| {
                if forward {
                    (from + k) % n
                } else {
                    (from + n - k % n) % n
                }
            })
            .find(|&i| editor.choices[i].is_none())
            .unwrap_or(if forward {
                (from + 1) % n
            } else {
                (from + n - 1) % n
            });
        if let Some(chunk) = editor.chunk_of(next) {
            editor.scroll.scroll_to_item(chunk);
        }
        if let Some(editor) = self.conflict.as_mut() {
            editor.picked = Some((next, 0));
        }
        cx.notify();
    }

    fn toggle_editing(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = self.conflict.as_mut() else {
            return;
        };
        if editor.editing.take().is_some() {
            cx.notify();
            return;
        }
        let Some(file) = editor.file.clone() else {
            return;
        };
        let text = file.render(&editor.choices);
        editor.editing = Some(cx.new(|cx| {
            let mut state = TextareaState::new(window, cx);
            state.set_value(text, window, cx);
            state
        }));
        cx.notify();
    }

    /// Writes the result and stages it; the next conflicted file opens.
    fn mark_resolved(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = self.conflict.as_ref() else {
            return;
        };
        let content = match (&editor.editing, &editor.file) {
            (Some(text), _) => text.read(cx).value().to_string(),
            (None, Some(file)) if editor.all_resolved() => file.render(&editor.choices),
            _ => return,
        };
        let path = editor.path.clone();
        self.run_op_then(
            tr("Résoudre", "Resolve"),
            move |r| r.resolve_with(&path, &content),
            |this, _, cx| this.after_resolve(cx),
            window,
            cx,
        );
    }

    /// Keeps one side's whole file (deleted or binary files).
    fn take_side(&mut self, ours: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = self.conflict.as_ref().map(|c| c.path.clone()) else {
            return;
        };
        self.run_op_then(
            tr("Résoudre", "Resolve"),
            move |r| r.resolve_side(&path, ours),
            |this, _, cx| this.after_resolve(cx),
            window,
            cx,
        );
    }

    fn after_resolve(&mut self, cx: &mut Context<Self>) {
        self.conflict = None;
        self.selected_change = None;
        self.celebrate_seq += 1;
        self.resolved_flash = Some(self.celebrate_seq);
        cx.notify();
    }

    fn continue_operation(
        &mut self,
        op: Operation,
        skip: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let label = if skip {
            tr("Passer", "Skip")
        } else {
            tr("Continuer", "Continue")
        };
        self.run_op_then(
            label,
            move |r| {
                if skip {
                    r.skip_operation(op)
                } else {
                    r.continue_operation(op)
                }
                .and_then(stopped_on_conflicts)
            },
            move |this, _, _| {
                if op == Operation::Rebase {
                    this.rebased();
                }
            },
            window,
            cx,
        );
    }

    fn abort_operation(
        &mut self,
        op: Option<Operation>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let title = match op {
            Some(op) => trf!("Annuler le {} ?", "Abort the {}?", op.name().to_lowercase()),
            None => tr("Annuler la résolution ?", "Abort the resolution?").to_string(),
        };
        self.confirm(
            title,
            tr(
                "Le dépôt revient à son état d'avant. Les résolutions faites sont perdues.",
                "The repository goes back to where it was. Resolutions made so far are lost.",
            )
            .into(),
            tr("Annuler l'opération", "Abort"),
            move |this, window, cx| {
                this.conflict = None;
                this.run_op("Abort", move |r| r.abort_operation(op), window, cx)
            },
            window,
            cx,
        );
    }

    // ---- Rendering -------------------------------------------------------

    /// Banner over the file lists while an operation or conflicts are pending.
    pub(super) fn render_operation_banner(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let state = self.data.operation.clone();
        let left = self.data.conflicted().count();
        if state.is_none() && left == 0 {
            return None;
        }
        let op = state.as_ref().map(|s| s.op);
        let title = match &state {
            Some(s) => match s.step {
                Some((n, total)) => format!("{} {n}/{total}", s.op.name()),
                None => s.op.name().to_string(),
            },
            None => tr("Conflits", "Conflicts").to_string(),
        };
        let detail = match &state {
            Some(s) => format!("{} ← {}", s.ours, s.theirs),
            None => tr(
                "Les modifications appliquées entrent en conflit",
                "Applied changes conflict",
            )
            .to_string(),
        };
        let done = left == 0;
        let color = if done {
            theme::lime()
        } else {
            theme::magenta()
        };
        let seq = self.resolved_flash.unwrap_or(0);
        Some(
            v_flex()
                .id("operation-banner")
                .flex_none()
                .relative()
                .overflow_hidden()
                .gap_2()
                .p_3()
                .border_b_1()
                .border_color(cx.theme().border)
                .bg(theme::alpha(color, 0.07))
                .child(
                    div()
                        .absolute()
                        .left_0()
                        .top_0()
                        .bottom_0()
                        .w(px(2.))
                        .bg(color),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .min_w_0()
                        .child(
                            Icon::new(if done {
                                Lucide::CircleCheck
                            } else {
                                Lucide::GitMergeConflict
                            })
                            .small()
                            .text_color(color),
                        )
                        .child(
                            div()
                                .text_sm()
                                .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                                .child(title),
                        )
                        .child(div().flex_1())
                        .child(
                            div()
                                .px_1p5()
                                .rounded_sm()
                                .text_xs()
                                .bg(theme::alpha(color, 0.16))
                                .text_color(color)
                                .child(if done {
                                    tr("tout est résolu", "all resolved").to_string()
                                } else {
                                    trf!("{} à résoudre", "{} to resolve", left)
                                })
                                .with_animation(
                                    ("resolved-pill", seq),
                                    Animation::new(Duration::from_millis(500))
                                        .with_easing(gpui_kit::ease_out_quint()),
                                    |el, t| {
                                        let pop = 1. + 0.25 * (1. - t) * (t * 6.).min(1.);
                                        el.opacity(0.4 + 0.6 * t).mt(px(-2. * (pop - 1.) * 4.))
                                    },
                                ),
                        ),
                )
                .child(
                    div()
                        .text_xs()
                        .truncate()
                        .text_color(cx.theme().muted_foreground)
                        .child(detail),
                )
                .child(
                    h_flex()
                        .gap_1()
                        .when_some(op, |row, op| {
                            row.child(
                                Button::new("op-continue")
                                    .xsmall()
                                    .primary()
                                    .icon(Icon::new(Lucide::Check))
                                    .label(tr("Continuer", "Continue"))
                                    .disabled(!done || self.busy.is_some())
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.continue_operation(op, false, window, cx)
                                    })),
                            )
                            .when(op.can_skip(), |row| {
                                row.child(
                                    Button::new("op-skip")
                                        .xsmall()
                                        .outline()
                                        .icon(Icon::new(Lucide::SkipForward))
                                        .label(tr("Passer ce commit", "Skip commit"))
                                        .disabled(self.busy.is_some())
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.continue_operation(op, true, window, cx)
                                        })),
                                )
                            })
                        })
                        .child(div().flex_1())
                        .child(
                            Button::new("op-abort")
                                .xsmall()
                                .ghost()
                                .icon(Icon::new(Lucide::X))
                                .label(tr("Annuler", "Abort"))
                                .disabled(self.busy.is_some() || (op.is_none() && done))
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.abort_operation(op, window, cx)
                                })),
                        ),
                )
                .with_animation(
                    "operation-banner-in",
                    Animation::new(Duration::from_millis(320))
                        .with_easing(gpui_kit::ease_out_quint()),
                    |el, t| el.opacity(t).mt(px(-10. * (1. - t))),
                )
                .into_any_element(),
        )
    }

    /// Sides' names: the operation's, else HEAD and the stash.
    fn side_labels(&self) -> (String, String) {
        match &self.data.operation {
            Some(s) => (s.ours.clone(), s.theirs.clone()),
            None => ("HEAD".into(), "stash".into()),
        }
    }

    pub(super) fn render_conflict(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(editor) = self.conflict.as_ref() else {
            return div().into_any_element();
        };
        if let Some(err) = &editor.error {
            return v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap_2()
                .text_sm()
                .text_color(theme::red())
                .child(err.clone())
                .into_any_element();
        }
        let Some(file) = editor.file.clone() else {
            return div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .text_color(cx.theme().muted_foreground)
                .child(
                    Icon::new(Lucide::GitMergeConflict)
                        .large()
                        .text_color(theme::magenta())
                        .with_animation(
                            "conflict-loading",
                            Animation::new(Duration::from_millis(900))
                                .repeat()
                                .with_easing(gpui_kit::pulsating_between(0.3, 1.)),
                            |el, t| el.opacity(t),
                        ),
                )
                .into_any_element();
        };
        if file.whole_file_only() {
            return self.render_whole_file(&file, cx);
        }
        let (ours_label, theirs_label) = self.side_labels();
        let column_title = |label: &str, name: String, color: Hsla| {
            h_flex()
                .flex_1()
                .min_w_0()
                .gap_1p5()
                .px_3()
                .child(div().size(px(7.)).rounded_full().bg(color))
                .child(
                    div()
                        .flex_none()
                        .text_color(color)
                        .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                        .child(label.to_uppercase()),
                )
                .child(div().min_w_0().truncate().child(name))
        };
        let columns = h_flex()
            .flex_none()
            .h(px(26.))
            .text_xs()
            .text_color(cx.theme().muted_foreground)
            .border_b_1()
            .border_color(cx.theme().border)
            .bg(theme::panel())
            .child(column_title("Ours", ours_label, ours_color()))
            .child(column_title(
                tr("Résultat", "Result"),
                editor.path.rsplit('/').next().unwrap_or("").to_string(),
                theme::accent(),
            ))
            .child(column_title("Theirs", theirs_label, theirs_color()));

        let body = if let Some(text) = &editor.editing {
            div()
                .flex_1()
                .min_h_0()
                .p_2()
                .font_family(cx.theme().mono_font_family.clone())
                .child(Textarea::new(text).h_full())
                .into_any_element()
        } else {
            let rows = self.conflict_rows(editor, &file, cx);
            div()
                .id("conflict-rows")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .track_scroll(&editor.scroll)
                .children(rows)
                .into_any_element()
        };

        v_flex()
            .size_full()
            .min_h_0()
            .child(self.conflict_header(editor, cx))
            .when(editor.editing.is_none(), |el| el.child(columns))
            .child(body)
            .into_any_element()
    }

    fn conflict_header(&self, editor: &ConflictEditor, cx: &mut Context<Self>) -> AnyElement {
        let total = editor.choices.len();
        let resolved = editor.resolved();
        let ratio = if total == 0 {
            1.
        } else {
            resolved as f32 / total as f32
        };
        let complete = editor.all_resolved() || editor.editing.is_some();
        let (dir, name) = editor
            .path
            .rsplit_once('/')
            .map_or(("", editor.path.as_str()), |(d, n)| (d, n));
        let bar_color = if editor.all_resolved() {
            theme::lime()
        } else {
            theme::accent()
        };
        h_flex()
            .flex_none()
            .h(px(40.))
            .px_3()
            .gap_2()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                Icon::new(Lucide::GitMergeConflict)
                    .small()
                    .text_color(theme::magenta()),
            )
            .child(
                div()
                    .text_sm()
                    .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                    .child(name.to_string()),
            )
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(dir.to_string()),
            )
            .child(
                h_flex()
                    .flex_none()
                    .gap_1p5()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(trf!(
                        "{resolved}/{total} résolus",
                        "{resolved}/{total} resolved"
                    ))
                    .child(
                        div()
                            .w(px(64.))
                            .h(px(4.))
                            .rounded_full()
                            .bg(theme::raised())
                            .overflow_hidden()
                            .child(
                                div()
                                    .h_full()
                                    .rounded_full()
                                    .bg(bar_color)
                                    .w(relative(ratio))
                                    .with_animation(
                                        ("conflict-progress", editor.seq),
                                        Animation::new(Duration::from_millis(420))
                                            .with_easing(gpui_kit::ease_out_quint()),
                                        move |el, t| {
                                            el.shadow(vec![gpui_kit::BoxShadow {
                                                color: theme::alpha(bar_color, 0.7 * (1. - t)),
                                                offset: gpui_kit::point(px(0.), px(0.)),
                                                blur_radius: px(2. + 8. * t),
                                                spread_radius: px(0.),
                                                inset: false,
                                            }])
                                        },
                                    ),
                            ),
                    ),
            )
            .child(div().flex_1())
            .when(editor.editing.is_none() && total > 1, |el| {
                el.child(
                    Button::new("conflict-prev")
                        .xsmall()
                        .ghost()
                        .icon(Icon::new(Lucide::ChevronUp))
                        .tooltip(tr("Conflit précédent", "Previous conflict"))
                        .on_click(cx.listener(|this, _, _, cx| this.jump(false, cx))),
                )
                .child(
                    Button::new("conflict-next")
                        .xsmall()
                        .ghost()
                        .icon(Icon::new(Lucide::ChevronDown))
                        .tooltip(tr("Conflit suivant", "Next conflict"))
                        .on_click(cx.listener(|this, _, _, cx| this.jump(true, cx))),
                )
            })
            .when(editor.editing.is_none(), |el| {
                el.child(
                    Button::new("all-ours")
                        .xsmall()
                        .outline()
                        .label(tr("Tout ours", "All ours"))
                        .on_click(
                            cx.listener(|this, _, _, cx| this.pick_all(Resolution::Ours, cx)),
                        ),
                )
                .child(
                    Button::new("all-theirs")
                        .xsmall()
                        .outline()
                        .label(tr("Tout theirs", "All theirs"))
                        .on_click(
                            cx.listener(|this, _, _, cx| this.pick_all(Resolution::Theirs, cx)),
                        ),
                )
            })
            .child(
                Button::new("conflict-edit")
                    .xsmall()
                    .ghost()
                    .icon(Icon::new(if editor.editing.is_some() {
                        Lucide::Columns3
                    } else {
                        Lucide::Pencil
                    }))
                    .tooltip(if editor.editing.is_some() {
                        tr("Revenir aux 3 panneaux", "Back to 3 panes")
                    } else {
                        tr("Éditer le résultat à la main", "Edit the result by hand")
                    })
                    .on_click(cx.listener(|this, _, window, cx| this.toggle_editing(window, cx))),
            )
            .child(
                Button::new("conflict-resolve")
                    .xsmall()
                    .primary()
                    .icon(Icon::new(Lucide::Check))
                    .label(tr("Marquer résolu", "Mark resolved"))
                    .disabled(!complete || self.busy.is_some())
                    .on_click(cx.listener(|this, _, window, cx| this.mark_resolved(window, cx))),
            )
            .into_any_element()
    }

    /// One row per chunk: the three columns stay aligned while scrolling.
    fn conflict_rows(
        &self,
        editor: &ConflictEditor,
        file: &ConflictFile,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let (mut n_ours, mut n_result, mut n_theirs) = (1, 1, 1);
        let mut conflict = 0;
        let mut rows = Vec::with_capacity(file.chunks.len());
        for (i, chunk) in file.chunks.iter().enumerate() {
            match chunk {
                Chunk::Clean(lines) => {
                    rows.push(self.clean_row(
                        i,
                        lines,
                        (n_ours, n_result, n_theirs),
                        editor.unfolded.contains(&i),
                        cx,
                    ));
                    n_ours += lines.len();
                    n_result += lines.len();
                    n_theirs += lines.len();
                }
                Chunk::Conflict(c) => {
                    let choice = editor.choices.get(conflict).copied().flatten();
                    let ours: Vec<&str> = c.ours.iter().map(String::as_str).collect();
                    let theirs: Vec<&str> = c.theirs.iter().map(String::as_str).collect();
                    let result = choice.map(|r| c.lines(r));
                    let fresh = editor
                        .picked
                        .filter(|(k, s)| *k == conflict && *s > 0)
                        .map(|(_, s)| s);
                    let focused = editor.picked.is_some_and(|(k, _)| k == conflict);
                    rows.push(
                        h_flex()
                            .id(("conflict", conflict))
                            .items_stretch()
                            .w_full()
                            .my_1()
                            .child(self.side_cell(conflict, &ours, n_ours, true, choice, cx))
                            .child(self.result_cell(
                                conflict,
                                c,
                                choice,
                                result.as_deref(),
                                n_result,
                                fresh,
                                focused,
                                cx,
                            ))
                            .child(self.side_cell(conflict, &theirs, n_theirs, false, choice, cx))
                            .into_any_element(),
                    );
                    n_ours += ours.len();
                    n_theirs += theirs.len();
                    n_result += result.map_or(0, |r| r.len());
                    conflict += 1;
                }
            }
        }
        rows
    }

    fn clean_row(
        &self,
        index: usize,
        lines: &[String],
        (n_ours, n_result, n_theirs): (usize, usize, usize),
        unfolded: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let all: Vec<&str> = lines.iter().map(String::as_str).collect();
        let fold = !unfolded && all.len() > FOLD_OVER;
        let columns = |slice: &[&str], offset: usize, cx: &App| {
            h_flex()
                .w_full()
                .items_start()
                .text_color(cx.theme().muted_foreground)
                .child(div().flex_1().min_w_0().opacity(0.65).child(code(
                    slice,
                    n_ours + offset,
                    cx,
                )))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .border_x_1()
                        .border_color(cx.theme().border)
                        .text_color(cx.theme().foreground)
                        .child(code(slice, n_result + offset, cx)),
                )
                .child(div().flex_1().min_w_0().opacity(0.65).child(code(
                    slice,
                    n_theirs + offset,
                    cx,
                )))
        };
        if !fold {
            return v_flex()
                .id(("clean", index))
                .w_full()
                .child(columns(&all, 0, cx))
                .into_any_element();
        }
        let hidden = all.len() - 2 * FOLD_KEEP;
        let tail = all.len() - FOLD_KEEP;
        v_flex()
            .id(("clean", index))
            .w_full()
            .child(columns(&all[..FOLD_KEEP], 0, cx))
            .child(
                h_flex()
                    .id(("unfold", index))
                    .w_full()
                    .h(px(20.))
                    .justify_center()
                    .gap_1()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .bg(theme::panel())
                    .cursor_pointer()
                    .hover(|s| s.bg(theme::raised()).text_color(cx.theme().foreground))
                    .child(Icon::new(Lucide::ChevronsUpDown).xsmall())
                    .child(trf!("{} lignes identiques", "{} unchanged lines", hidden))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(editor) = this.conflict.as_mut() {
                            editor.unfolded.insert(index);
                        }
                        cx.notify();
                    })),
            )
            .child(columns(&all[tail..], tail, cx))
            .into_any_element()
    }

    /// Ours or theirs for one conflict: click to take it.
    fn side_cell(
        &self,
        conflict: usize,
        lines: &[&str],
        first: usize,
        ours: bool,
        choice: Option<Resolution>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let color = if ours { ours_color() } else { theirs_color() };
        let taken = match choice {
            Some(Resolution::Ours) => ours,
            Some(Resolution::Theirs) => !ours,
            Some(Resolution::OursThenTheirs | Resolution::TheirsThenOurs) => true,
            _ => false,
        };
        let dropped = choice.is_some() && !taken;
        let pick = if ours {
            Resolution::Ours
        } else {
            Resolution::Theirs
        };
        let tag = h_flex()
            .absolute()
            .top(px(3.))
            .when(ours, |s| s.right(px(6.)))
            .when(!ours, |s| s.left(px(6.)))
            .gap_1()
            .px_1p5()
            .rounded_sm()
            .text_xs()
            .bg(theme::alpha(color, if taken { 0.3 } else { 0.16 }))
            .text_color(color)
            .when(!ours, |s| s.child(Icon::new(Lucide::ArrowLeft).xsmall()))
            .child(if taken {
                tr("pris", "taken")
            } else {
                tr("prendre", "take")
            })
            .when(ours, |s| s.child(Icon::new(Lucide::ArrowRight).xsmall()));
        div()
            .id(SharedString::from(format!(
                "{}:{conflict}",
                if ours { "ours" } else { "theirs" }
            )))
            .test_support()
            .relative()
            .flex_1()
            .min_w_0()
            .min_h(px(26.))
            .cursor_pointer()
            .bg(theme::alpha(color, if taken { 0.13 } else { 0.06 }))
            .border_y_1()
            .border_color(theme::alpha(color, 0.25))
            .when(ours, |s| s.border_l_2())
            .when(!ours, |s| s.border_r_2())
            .when(dropped, |s| s.opacity(0.45))
            .hover(|s| s.bg(theme::alpha(color, 0.18)))
            .child(if lines.is_empty() {
                div()
                    .px_3()
                    .py_1()
                    .text_xs()
                    .italic()
                    .text_color(cx.theme().muted_foreground)
                    .child(tr("(lignes supprimées)", "(lines deleted)"))
                    .into_any_element()
            } else {
                code(lines, first, cx)
            })
            .child(tag)
            .on_click(cx.listener(move |this, ev: &gpui_kit::ClickEvent, _, cx| {
                // ⌥-click: this side after the other one, keeping both.
                let r = if ev.modifiers().alt {
                    if ours {
                        Resolution::TheirsThenOurs
                    } else {
                        Resolution::OursThenTheirs
                    }
                } else {
                    pick
                };
                this.pick(conflict, Some(r), cx)
            }))
            .into_any_element()
    }

    #[allow(clippy::too_many_arguments)]
    fn result_cell(
        &self,
        conflict: usize,
        c: &mugetsu_git::Conflict,
        choice: Option<Resolution>,
        result: Option<&[&str]>,
        first: usize,
        fresh: Option<usize>,
        focused: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let accent = theme::accent();
        let cell = div()
            .relative()
            .flex_1()
            .min_w_0()
            .min_h(px(26.))
            .mx_1()
            .rounded_md()
            .overflow_hidden();
        let (Some(r), Some(lines)) = (choice, result) else {
            // Unresolved: a pulsing frame with the ways out.
            let choice_button = |id: &'static str, label: &'static str, r: Option<Resolution>| {
                Button::new((id, conflict))
                    .xsmall()
                    .ghost()
                    .label(label)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.pick(conflict, r.or(Some(Resolution::Neither)), cx)
                    }))
            };
            let both_label = tr("Les deux", "Both");
            return cell
                .border_1()
                .border_color(theme::alpha(accent, 0.5))
                .bg(theme::alpha(accent, 0.05))
                .when(focused, |s| s.border_color(accent))
                .child(
                    v_flex()
                        .size_full()
                        .items_center()
                        .justify_center()
                        .gap_1()
                        .py_2()
                        .child(
                            h_flex()
                                .gap_1()
                                .text_xs()
                                .text_color(accent)
                                .child(Icon::new(Lucide::GitMergeConflict).xsmall())
                                .child(trf!(
                                    "{} ligne(s) contre {}",
                                    "{} line(s) vs {}",
                                    c.ours.len(),
                                    c.theirs.len()
                                )),
                        )
                        .child(
                            h_flex()
                                .gap_0p5()
                                .flex_wrap()
                                .justify_center()
                                .child(choice_button("res-ours", "← Ours", Some(Resolution::Ours)))
                                .child(choice_button(
                                    "res-both",
                                    both_label,
                                    Some(Resolution::OursThenTheirs),
                                ))
                                .child(choice_button(
                                    "res-theirs",
                                    "Theirs →",
                                    Some(Resolution::Theirs),
                                ))
                                .child(choice_button(
                                    "res-none",
                                    tr("Aucun", "Neither"),
                                    Some(Resolution::Neither),
                                )),
                        ),
                )
                .with_animation(
                    ("unresolved", conflict),
                    Animation::new(Duration::from_millis(1600))
                        .repeat()
                        .with_easing(gpui_kit::pulsating_between(0.35, 1.)),
                    move |el, t| el.border_color(theme::alpha(accent, 0.25 + 0.6 * t)),
                )
                .into_any_element();
        };

        // Lines tinted by the side they come from.
        let split = match r {
            Resolution::OursThenTheirs => c.ours.len(),
            Resolution::TheirsThenOurs => c.theirs.len(),
            _ => lines.len(),
        };
        let first_color = side_color(r, true);
        let second_color = side_color(r, false);
        let part = |slice: &[&str], start: usize, color: Hsla, cx: &App| {
            div()
                .w_full()
                .bg(theme::alpha(color, 0.1))
                .border_l_2()
                .border_color(theme::alpha(color, 0.7))
                .child(code(slice, start, cx))
        };
        let content = if lines.is_empty() {
            div()
                .px_3()
                .py_1()
                .text_xs()
                .italic()
                .text_color(cx.theme().muted_foreground)
                .child(tr("(rien de gardé)", "(nothing kept)"))
                .into_any_element()
        } else {
            v_flex()
                .w_full()
                .when(split > 0, |el| {
                    el.child(part(&lines[..split], first, first_color, cx))
                })
                .when(split < lines.len(), |el| {
                    el.child(part(&lines[split..], first + split, second_color, cx))
                })
                .into_any_element()
        };
        let undo = Button::new(("res-undo", conflict))
            .xsmall()
            .ghost()
            .icon(Icon::new(Lucide::Undo2))
            .tooltip(tr("Annuler ce choix", "Undo this pick"))
            .on_click(cx.listener(move |this, _, _, cx| this.pick(conflict, None, cx)));
        let from_left = matches!(r, Resolution::Ours | Resolution::OursThenTheirs);
        let cell = cell
            .border_1()
            .border_color(theme::alpha(theme::lime(), 0.35))
            .bg(theme::alpha(theme::lime(), 0.03))
            .child(content)
            .child(div().absolute().top(px(1.)).right(px(1.)).child(undo));
        match fresh {
            // Just picked: the lines slide in from their side with a glow.
            Some(seq) => cell
                .with_animation(
                    ("picked", seq),
                    Animation::new(Duration::from_millis(460))
                        .with_easing(gpui_kit::ease_out_quint()),
                    move |el, t| {
                        let dx = if from_left { -18. } else { 18. } * (1. - t);
                        el.opacity(0.2 + 0.8 * t)
                            .left(px(dx))
                            .shadow(vec![gpui_kit::BoxShadow {
                                color: theme::alpha(first_color, 0.6 * (1. - t)),
                                offset: gpui_kit::point(px(0.), px(0.)),
                                blur_radius: px(4. + 16. * t),
                                spread_radius: px(0.),
                                inset: false,
                            }])
                    },
                )
                .into_any_element(),
            None => cell.into_any_element(),
        }
    }

    /// Deleted on one side, or binary: keep one whole version.
    fn render_whole_file(&self, file: &ConflictFile, cx: &mut Context<Self>) -> AnyElement {
        let (ours_label, theirs_label) = self.side_labels();
        let binary = file.ours.is_some() && file.theirs.is_some();
        let describe = |v: &Option<Vec<u8>>| match v {
            None => tr("supprimé", "deleted").to_string(),
            Some(b) => trf!("modifié · {} octets", "modified · {} bytes", b.len()),
        };
        let side = |id: &'static str, ours: bool, label: String, state: String, color: Hsla| {
            v_flex()
                .id(id)
                .test_support()
                .w(px(240.))
                .gap_1()
                .p_4()
                .rounded_lg()
                .cursor_pointer()
                .border_1()
                .border_color(theme::alpha(color, 0.4))
                .bg(theme::alpha(color, 0.08))
                .hover(|s| s.bg(theme::alpha(color, 0.16)).border_color(color))
                .child(
                    div()
                        .text_xs()
                        .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                        .text_color(color)
                        .child(if ours { "OURS" } else { "THEIRS" }),
                )
                .child(div().text_sm().truncate().child(label))
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(state),
                )
                .child(
                    div()
                        .pt_2()
                        .text_xs()
                        .text_color(color)
                        .child(tr("Garder cette version", "Keep this version")),
                )
                .on_click(cx.listener(move |this, _, window, cx| this.take_side(ours, window, cx)))
        };
        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap_4()
            .child(
                Icon::new(Lucide::GitMergeConflict)
                    .large()
                    .text_color(theme::magenta()),
            )
            .child(div().text_sm().child(file.path.clone()))
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(if binary {
                        tr(
                            "Fichier binaire : choisissez la version à garder.",
                            "Binary file: pick the version to keep.",
                        )
                    } else {
                        tr(
                            "Supprimé d'un côté, modifié de l'autre.",
                            "Deleted on one side, modified on the other.",
                        )
                    }),
            )
            .child(
                h_flex()
                    .gap_3()
                    .child(side(
                        "keep-ours",
                        true,
                        ours_label,
                        describe(&file.ours),
                        ours_color(),
                    ))
                    .child(side(
                        "keep-theirs",
                        false,
                        theirs_label,
                        describe(&file.theirs),
                        theirs_color(),
                    )),
            )
            .with_animation(
                "whole-file-in",
                Animation::new(Duration::from_millis(360)).with_easing(gpui_kit::ease_out_quint()),
                |el, t| el.opacity(t).mt(px(12. * (1. - t))),
            )
            .into_any_element()
    }
}
