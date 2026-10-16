//! Built-in conflict resolution: the operation banner (continue, skip,
//! abort) and a three-pane resolver, ours | result | theirs, aligned chunk by
//! chunk. A conflict is settled by taking a side (or both, or neither), or by
//! dragging blocks and single lines into its result, where kept pieces can be
//! reordered or dragged back out. ⌥↑ / ⌥↓ jump from conflict to conflict and
//! ⌥← / ⌥→ take a side; the result can also be edited by hand before marking
//! the file resolved.

use std::collections::HashSet;
use std::ops::Range;
use std::sync::Arc;
use std::time::Duration;

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Textarea, TextareaState};
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    Animation, AnimationExt as _, AnyElement, App, AppContext as _, Context, Entity, Hsla,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, ScrollHandle, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window, div, prelude::FluentBuilder as _, px,
    relative,
};
use mugetsu_git::{Chunk, Conflict, ConflictFile, Operation, Resolution};

use super::{Mode, RepoView, SelectedChange};
use crate::i18n::tr;
use crate::theme;

/// Clean runs longer than this fold to their first and last lines.
const FOLD_OVER: usize = 10;
const FOLD_KEEP: usize = 3;
/// Height of the bar over each side of a conflict.
const BAR_H: f32 = 26.;
/// Estimated height of a code line, for the overview ruler.
const LINE_H: f32 = 16.;
const RULER_W: f32 = 12.;

/// One side of a conflict.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Side {
    Ours,
    Theirs,
}

impl Side {
    fn lines(self, c: &Conflict) -> &[String] {
        match self {
            Self::Ours => &c.ours,
            Self::Theirs => &c.theirs,
        }
    }

    fn color(self) -> Hsla {
        match self {
            Self::Ours => ours_color(),
            Self::Theirs => theirs_color(),
        }
    }

    fn id(self) -> &'static str {
        match self {
            Self::Ours => "ours",
            Self::Theirs => "theirs",
        }
    }
}

/// A run of lines from one side, kept in a conflict's result.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Piece {
    side: Side,
    lines: Range<usize>,
}

impl Piece {
    fn whole(side: Side, c: &Conflict) -> Self {
        Self {
            side,
            lines: 0..side.lines(c).len(),
        }
    }

    fn text<'a>(&self, c: &'a Conflict) -> &'a [String] {
        &self.side.lines(c)[self.lines.clone()]
    }
}

/// The pieces a one-click resolution keeps.
fn pieces_for(r: Resolution, c: &Conflict) -> Vec<Piece> {
    let (a, b) = match r {
        Resolution::Ours => (Some(Side::Ours), None),
        Resolution::Theirs => (Some(Side::Theirs), None),
        Resolution::OursThenTheirs => (Some(Side::Ours), Some(Side::Theirs)),
        Resolution::TheirsThenOurs => (Some(Side::Theirs), Some(Side::Ours)),
        Resolution::Neither => (None, None),
    };
    let mut pieces: Vec<Piece> = a.into_iter().chain(b).map(|s| Piece::whole(s, c)).collect();
    normalize(&mut pieces);
    pieces
}

/// Drops empty pieces and joins neighbours that continue each other (lines
/// dragged one after the other).
fn normalize(pieces: &mut Vec<Piece>) {
    pieces.retain(|p| !p.lines.is_empty());
    let mut i = 1;
    while i < pieces.len() {
        if pieces[i - 1].side == pieces[i].side && pieces[i - 1].lines.end == pieces[i].lines.start
        {
            let end = pieces.remove(i).lines.end;
            pieces[i - 1].lines.end = end;
        } else {
            i += 1;
        }
    }
}

/// Puts `piece` before the kept piece `before` (at the end without one).
fn insert_piece(pieces: &mut Vec<Piece>, piece: Piece, before: Option<usize>) {
    let at = before.unwrap_or(pieces.len()).min(pieces.len());
    pieces.insert(at, piece);
    normalize(pieces);
}

/// Moves the kept piece `from` before `before` (to the end without one).
fn move_piece(pieces: &mut Vec<Piece>, from: usize, before: Option<usize>) {
    if from >= pieces.len() {
        return;
    }
    let piece = pieces.remove(from);
    let before = before.map(|b| if b > from { b - 1 } else { b });
    insert_piece(pieces, piece, before);
}

/// Whether line `line` of `side` is in the result.
fn kept(pieces: &[Piece], side: Side, line: usize) -> bool {
    pieces
        .iter()
        .any(|p| p.side == side && p.lines.contains(&line))
}

/// What is being dragged towards (or out of) a conflict's result.
#[derive(Clone, Debug)]
enum Dragged {
    /// Lines of one side.
    Lines(Piece),
    /// A piece already in the result (index): reorder, or drop out to remove.
    Kept(usize),
}

#[derive(Clone)]
struct CodeDrag {
    conflict: usize,
    what: Dragged,
    color: Hsla,
    first: SharedString,
    count: usize,
}

impl Render for CodeDrag {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .w(px(320.))
            .h(px(30.))
            .px_2()
            .gap_2()
            .rounded_md()
            .bg(theme::popover())
            .border_1()
            .border_color(self.color)
            .shadow_xl()
            .child(
                Icon::new(Lucide::GripVertical)
                    .xsmall()
                    .text_color(self.color),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .font_family(cx.theme().mono_font_family.clone())
                    .text_xs()
                    .child(self.first.clone()),
            )
            .when(self.count > 1, |el| {
                el.child(
                    div()
                        .flex_none()
                        .px_1p5()
                        .rounded_sm()
                        .text_xs()
                        .bg(theme::alpha(self.color, 0.2))
                        .text_color(self.color)
                        .child(trf!("{} lignes", "{} lines", self.count)),
                )
            })
            .with_animation(
                "code-lift",
                Animation::new(Duration::from_millis(160)).with_easing(gpui_kit::ease_out_quint()),
                |el, t| el.opacity(0.6 + 0.4 * t),
            )
    }
}

/// The resolver's state for one conflicted file.
pub(super) struct ConflictEditor {
    pub path: String,
    /// `None` while the versions load.
    file: Option<Arc<ConflictFile>>,
    error: Option<String>,
    /// What each conflict keeps, in order; `None` while unresolved.
    choices: Vec<Option<Vec<Piece>>>,
    /// The conflict the navigation and the shortcuts act on.
    current: usize,
    /// Clean chunks unfolded by the user (chunk index).
    unfolded: HashSet<usize>,
    /// Hand-editing the result instead of picking sides.
    editing: Option<Entity<TextareaState>>,
    /// Last change (conflict index, sequence), for its entrance animation.
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
            current: 0,
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

    fn conflict(&self, n: usize) -> Option<&Conflict> {
        self.file.as_ref()?.conflicts().nth(n)
    }

    /// The merged text with the current picks.
    fn render(&self) -> Option<String> {
        let file = self.file.as_ref()?;
        Some(file.render_with(|i, c| {
            self.choices.get(i)?.as_ref().map(|pieces| {
                pieces
                    .iter()
                    .flat_map(|p| p.text(c).iter().map(String::as_str))
                    .collect()
            })
        }))
    }

    /// Makes `conflict` current and brings it into view, with a few lines of
    /// context above it.
    fn focus(&mut self, conflict: usize) {
        self.current = conflict;
        if let Some(chunk) = self.chunk_of(conflict) {
            self.scroll.scroll_to_top_of_item(chunk.saturating_sub(1));
        }
    }

    /// The next unresolved conflict after the current one, wrapping.
    fn next_unresolved(&self) -> Option<usize> {
        let n = self.choices.len();
        (1..=n)
            .map(|k| (self.current + k) % n)
            .find(|&i| self.choices[i].is_none())
    }

    fn changed(&mut self, conflict: usize) {
        self.seq += 1;
        self.picked = Some((conflict, self.seq));
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

/// The resolver's own opaque ground: nothing of the window shows through it.
fn ground() -> Hsla {
    Hsla {
        a: 1.,
        ..theme::card()
    }
}

/// Opaque bars (file header, column titles, conflict bars).
fn bar() -> Hsla {
    Hsla {
        a: 1.,
        ..theme::sheet()
    }
}

/// Display text of a line: no line ending, tabs as spaces, never empty (so
/// the row keeps its height).
fn shown(line: &str) -> String {
    let l = line.trim_end_matches(['\n', '\r']).replace('\t', "    ");
    if l.is_empty() { " ".into() } else { l }
}

/// One code line with its number in the gutter.
fn code_line(number: usize, text: &str, cx: &App) -> gpui_kit::Div {
    h_flex()
        .items_start()
        .w_full()
        .min_w_0()
        .child(
            div()
                .w(px(38.))
                .flex_none()
                .pr_2()
                .text_right()
                .text_color(theme::alpha(theme::muted(), 0.7))
                .child(number.to_string()),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_color(cx.theme().foreground)
                .child(shown(text)),
        )
}

/// Lines with a gutter of numbers starting at `first`.
fn code(lines: &[&str], first: usize, cx: &App) -> AnyElement {
    v_flex()
        .w_full()
        .py(px(2.))
        .font_family(cx.theme().mono_font_family.clone())
        .text_xs()
        .children(
            lines
                .iter()
                .enumerate()
                .map(|(i, l)| code_line(first + i, l, cx)),
        )
        .into_any_element()
}

/// Muted italic placeholder inside a cell.
fn placeholder(text: &'static str, cx: &App) -> AnyElement {
    div()
        .px_3()
        .py_1p5()
        .text_xs()
        .italic()
        .text_color(cx.theme().muted_foreground)
        .child(text)
        .into_any_element()
}

/// A small keycap for the shortcut hints.
fn keycap(k: String) -> AnyElement {
    div()
        .px_1()
        .rounded_sm()
        .border_1()
        .border_color(theme::hairline())
        .bg(theme::raised())
        .child(k)
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

    /// Replaces what `conflict` keeps (`None`: unresolved again). With
    /// `advance`, a resolved conflict hands over to the next unresolved one.
    fn set_choice(
        &mut self,
        conflict: usize,
        pieces: Option<Vec<Piece>>,
        advance: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(editor) = self.conflict.as_mut() else {
            return;
        };
        let Some(slot) = editor.choices.get_mut(conflict) else {
            return;
        };
        let resolved = pieces.is_some();
        *slot = pieces;
        editor.current = conflict;
        if resolved {
            editor.changed(conflict);
        } else {
            editor.picked = None;
        }
        if advance
            && resolved
            && let Some(next) = editor.next_unresolved()
        {
            editor.focus(next);
        }
        cx.notify();
    }

    fn pick(&mut self, conflict: usize, r: Resolution, cx: &mut Context<Self>) {
        let pieces = self
            .conflict
            .as_ref()
            .and_then(|e| e.conflict(conflict))
            .map(|c| pieces_for(r, c));
        if let Some(pieces) = pieces {
            self.set_choice(conflict, Some(pieces), true, cx);
        }
    }

    /// Adds a whole side after what the result already keeps.
    fn append_side(&mut self, conflict: usize, side: Side, cx: &mut Context<Self>) {
        let Some(editor) = self.conflict.as_ref() else {
            return;
        };
        let Some(c) = editor.conflict(conflict) else {
            return;
        };
        let mut pieces = editor
            .choices
            .get(conflict)
            .cloned()
            .flatten()
            .unwrap_or_default();
        insert_piece(&mut pieces, Piece::whole(side, c), None);
        self.set_choice(conflict, Some(pieces), false, cx);
    }

    fn pick_all(&mut self, r: Resolution, cx: &mut Context<Self>) {
        if let Some(editor) = self.conflict.as_mut()
            && let Some(file) = editor.file.clone()
        {
            editor.seq += 1;
            editor.picked = None;
            editor.choices = file.conflicts().map(|c| Some(pieces_for(r, c))).collect();
        }
        cx.notify();
    }

    /// Dropped into `conflict`'s result, before the kept piece `before` (at
    /// the end without one).
    fn drop_code(
        &mut self,
        conflict: usize,
        drag: &CodeDrag,
        before: Option<usize>,
        cx: &mut Context<Self>,
    ) {
        if drag.conflict != conflict {
            return;
        }
        let Some(editor) = self.conflict.as_ref() else {
            return;
        };
        let mut pieces = editor
            .choices
            .get(conflict)
            .cloned()
            .flatten()
            .unwrap_or_default();
        match &drag.what {
            Dragged::Lines(piece) => insert_piece(&mut pieces, piece.clone(), before),
            Dragged::Kept(from) => move_piece(&mut pieces, *from, before),
        }
        self.set_choice(conflict, Some(pieces), false, cx);
    }

    /// Takes a kept piece out of the result; the last one out leaves the
    /// conflict unresolved.
    fn remove_piece(&mut self, conflict: usize, index: usize, cx: &mut Context<Self>) {
        let Some(mut pieces) = self
            .conflict
            .as_ref()
            .and_then(|e| e.choices.get(conflict).cloned().flatten())
        else {
            return;
        };
        if index < pieces.len() {
            pieces.remove(index);
        }
        let choice = (!pieces.is_empty()).then_some(pieces);
        self.set_choice(conflict, choice, false, cx);
    }

    /// Goes to the next (or previous) conflict, wrapping.
    pub(super) fn conflict_jump(&mut self, forward: bool, cx: &mut Context<Self>) {
        let Some(editor) = self.conflict.as_mut().filter(|e| e.editing.is_none()) else {
            return;
        };
        let n = editor.choices.len();
        if n == 0 {
            return;
        }
        let next = if forward {
            (editor.current + 1) % n
        } else {
            (editor.current + n - 1) % n
        };
        editor.focus(next);
        cx.notify();
    }

    fn conflict_goto(&mut self, conflict: usize, cx: &mut Context<Self>) {
        if let Some(editor) = self.conflict.as_mut() {
            editor.focus(conflict);
        }
        cx.notify();
    }

    /// Takes one side for the current conflict (keyboard).
    pub(super) fn conflict_take(&mut self, ours: bool, cx: &mut Context<Self>) {
        let Some(current) = self
            .conflict
            .as_ref()
            .filter(|e| e.editing.is_none() && !e.choices.is_empty())
            .map(|e| e.current)
        else {
            return;
        };
        let r = if ours {
            Resolution::Ours
        } else {
            Resolution::Theirs
        };
        self.pick(current, r, cx);
    }

    fn toggle_editing(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = self.conflict.as_mut() else {
            return;
        };
        if editor.editing.take().is_some() {
            cx.notify();
            return;
        }
        let Some(text) = editor.render() else {
            return;
        };
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
        let content = match &editor.editing {
            Some(text) => text.read(cx).value().to_string(),
            None if editor.all_resolved() => match editor.render() {
                Some(text) => text,
                None => return,
            },
            None => return,
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
                .h_full()
                .gap_1p5()
                .px_3()
                .child(div().flex_none().size(px(7.)).rounded_full().bg(color))
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
            .h(px(28.))
            .text_xs()
            .text_color(cx.theme().muted_foreground)
            .border_b_1()
            .border_color(cx.theme().border)
            .bg(bar())
            .child(column_title("Ours", ours_label, ours_color()))
            .child(
                column_title(
                    tr("Résultat", "Result"),
                    editor.path.rsplit('/').next().unwrap_or("").to_string(),
                    theme::accent(),
                )
                .border_x_1()
                .border_color(cx.theme().border),
            )
            .child(column_title("Theirs", theirs_label, theirs_color()))
            .child(
                div()
                    .flex_none()
                    .w(px(RULER_W))
                    .h_full()
                    .border_l_1()
                    .border_color(cx.theme().border),
            );

        let body = if let Some(text) = &editor.editing {
            div()
                .flex_1()
                .min_h_0()
                .p_2()
                .font_family(cx.theme().mono_font_family.clone())
                .child(Textarea::new(text).h_full())
                .into_any_element()
        } else {
            let (rows, marks, total) = self.conflict_rows(editor, &file, cx);
            h_flex()
                .flex_1()
                .min_h_0()
                .items_stretch()
                .child(
                    div()
                        .id("conflict-rows")
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .overflow_y_scroll()
                        .track_scroll(&editor.scroll)
                        .children(rows),
                )
                .child(self.conflict_ruler(editor, marks, total, cx))
                .into_any_element()
        };

        v_flex()
            .size_full()
            .min_h_0()
            .bg(ground())
            .child(self.conflict_header(editor, cx))
            .when(editor.editing.is_none(), |el| el.child(columns))
            .child(body)
            .when(editor.editing.is_none(), |el| el.child(conflict_hints(cx)))
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
        let alt = if cfg!(target_os = "macos") {
            "⌥"
        } else {
            "Alt+"
        };
        let navigator = h_flex()
            .flex_none()
            .gap_0p5()
            .px_0p5()
            .rounded_md()
            .border_1()
            .border_color(theme::hairline())
            .bg(theme::raised())
            .child(
                Button::new("conflict-prev")
                    .xsmall()
                    .ghost()
                    .icon(Icon::new(Lucide::ChevronUp))
                    .tooltip(format!(
                        "{} ({alt}↑)",
                        tr("Conflit précédent", "Previous conflict")
                    ))
                    .on_click(cx.listener(|this, _, _, cx| this.conflict_jump(false, cx))),
            )
            .child(
                div()
                    .px_1()
                    .text_xs()
                    .font_weight(gpui_kit::FontWeight::MEDIUM)
                    .child(trf!(
                        "Conflit {} sur {}",
                        "Conflict {} of {}",
                        editor.current + 1,
                        total
                    )),
            )
            .child(
                Button::new("conflict-next")
                    .xsmall()
                    .ghost()
                    .icon(Icon::new(Lucide::ChevronDown))
                    .tooltip(format!(
                        "{} ({alt}↓)",
                        tr("Conflit suivant", "Next conflict")
                    ))
                    .on_click(cx.listener(|this, _, _, cx| this.conflict_jump(true, cx))),
            );
        h_flex()
            .flex_none()
            .h(px(44.))
            .px_3()
            .gap_2()
            .bg(bar())
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                Icon::new(Lucide::GitMergeConflict)
                    .small()
                    .text_color(theme::magenta()),
            )
            .child(
                div()
                    .flex_none()
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
            .child(div().flex_1())
            .when(editor.editing.is_none() && total > 0, |el| {
                el.child(navigator)
            })
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
            .when(editor.editing.is_none(), |el| {
                el.child(
                    Button::new("all-ours")
                        .xsmall()
                        .ghost()
                        .label(tr("Tout ours", "All ours"))
                        .on_click(
                            cx.listener(|this, _, _, cx| this.pick_all(Resolution::Ours, cx)),
                        ),
                )
                .child(
                    Button::new("all-theirs")
                        .xsmall()
                        .ghost()
                        .label(tr("Tout theirs", "All theirs"))
                        .on_click(
                            cx.listener(|this, _, _, cx| this.pick_all(Resolution::Theirs, cx)),
                        ),
                )
            })
            .child(
                Button::new("conflict-edit")
                    .xsmall()
                    .outline()
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

    /// One row per chunk, so the three columns stay aligned while scrolling;
    /// plus where each conflict sits (start, height) for the ruler, and the
    /// total height, in estimated pixels.
    fn conflict_rows(
        &self,
        editor: &ConflictEditor,
        file: &ConflictFile,
        cx: &mut Context<Self>,
    ) -> (Vec<AnyElement>, Vec<(usize, f32, f32)>, f32) {
        let count = editor.choices.len();
        let dragging = cx.has_active_drag();
        let (mut n_ours, mut n_result, mut n_theirs) = (1, 1, 1);
        let mut conflict = 0;
        let mut rows = Vec::with_capacity(file.chunks.len());
        let mut marks = Vec::with_capacity(count);
        let mut y = 0.;
        for (i, chunk) in file.chunks.iter().enumerate() {
            match chunk {
                Chunk::Clean(lines) => {
                    let unfolded = editor.unfolded.contains(&i);
                    rows.push(self.clean_row(i, lines, (n_ours, n_result, n_theirs), unfolded, cx));
                    let shown = if unfolded || lines.len() <= FOLD_OVER {
                        lines.len()
                    } else {
                        2 * FOLD_KEEP + 1
                    };
                    y += LINE_H * shown as f32;
                    n_ours += lines.len();
                    n_result += lines.len();
                    n_theirs += lines.len();
                }
                Chunk::Conflict(c) => {
                    let pieces = editor.choices.get(conflict).cloned().flatten();
                    let kept_lines: usize = pieces.iter().flatten().map(|p| p.lines.len()).sum();
                    let fresh = editor
                        .picked
                        .filter(|(k, _)| *k == conflict)
                        .map(|(_, s)| s);
                    let current = editor.current == conflict;
                    rows.push(
                        h_flex()
                            .id(("conflict", conflict))
                            .items_stretch()
                            .w_full()
                            .my_1()
                            .border_y_1()
                            .border_color(if current {
                                theme::alpha(theme::accent(), 0.7)
                            } else {
                                cx.theme().border
                            })
                            .child(self.side_cell(
                                conflict,
                                c,
                                Side::Ours,
                                n_ours,
                                pieces.as_deref(),
                                cx,
                            ))
                            .child(self.result_cell(
                                conflict,
                                count,
                                c,
                                pieces.as_deref(),
                                n_result,
                                fresh,
                                current,
                                dragging,
                                cx,
                            ))
                            .child(self.side_cell(
                                conflict,
                                c,
                                Side::Theirs,
                                n_theirs,
                                pieces.as_deref(),
                                cx,
                            ))
                            .into_any_element(),
                    );
                    let tallest = c.ours.len().max(c.theirs.len()).max(kept_lines).max(2);
                    let height = BAR_H + LINE_H * tallest as f32 + 8.;
                    marks.push((conflict, y, height));
                    y += height;
                    n_ours += c.ours.len();
                    n_theirs += c.theirs.len();
                    n_result += kept_lines;
                    conflict += 1;
                }
            }
        }
        (rows, marks, y.max(1.))
    }

    /// The overview strip: where each conflict sits in the file, coloured by
    /// state; a click jumps there.
    fn conflict_ruler(
        &self,
        editor: &ConflictEditor,
        marks: Vec<(usize, f32, f32)>,
        total: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .relative()
            .flex_none()
            .w(px(RULER_W))
            .h_full()
            .border_l_1()
            .border_color(cx.theme().border)
            .bg(bar())
            .children(marks.into_iter().map(|(k, start, height)| {
                let resolved = editor.choices.get(k).is_some_and(Option::is_some);
                let current = editor.current == k;
                let color = if resolved {
                    theme::lime()
                } else {
                    theme::accent()
                };
                div()
                    .id(("conflict-mark", k))
                    .absolute()
                    .left(px(3.))
                    .right(px(3.))
                    .top(relative((start / total).min(0.99)))
                    .h(relative(height / total))
                    .min_h(px(4.))
                    .rounded_sm()
                    .cursor_pointer()
                    .bg(theme::alpha(color, if current { 1. } else { 0.55 }))
                    .hover(|s| s.bg(color))
                    .on_click(cx.listener(move |this, _, _, cx| this.conflict_goto(k, cx)))
            }))
            .into_any_element()
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
                .child(div().flex_1().min_w_0().opacity(0.55).child(code(
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
                        .opacity(0.85)
                        .child(code(slice, n_result + offset, cx)),
                )
                .child(div().flex_1().min_w_0().opacity(0.55).child(code(
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
                    .h(px(LINE_H))
                    .justify_center()
                    .gap_1()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .bg(bar())
                    .border_y_1()
                    .border_color(cx.theme().border)
                    .cursor_pointer()
                    .hover(|s| s.text_color(cx.theme().foreground))
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

    /// Ours or theirs for one conflict: a bar to take the side (or drag the
    /// whole block), then its lines, each draggable on its own. Kept pieces
    /// dragged back here leave the result.
    fn side_cell(
        &self,
        conflict: usize,
        c: &Conflict,
        side: Side,
        first: usize,
        pieces: Option<&[Piece]>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let color = side.color();
        let ours = side == Side::Ours;
        let lines = side.lines(c);
        let whole = 0..lines.len();
        let taken = pieces.is_some_and(|p| {
            !lines.is_empty() && p.iter().any(|p| p.side == side && p.lines == whole)
        });
        let resolved = pieces.is_some();
        let drag_of = |range: Range<usize>| CodeDrag {
            conflict,
            color,
            first: lines
                .get(range.start)
                .map(|l| shown(l).trim().to_string())
                .unwrap_or_default()
                .into(),
            count: range.len(),
            what: Dragged::Lines(Piece { side, lines: range }),
        };

        let take = Button::new(SharedString::from(format!("{}:{conflict}", side.id())))
            .xsmall()
            .map(|b| if taken { b.primary() } else { b.outline() })
            .icon(Icon::new(if taken {
                Lucide::Check
            } else if ours {
                Lucide::ArrowRight
            } else {
                Lucide::ArrowLeft
            }))
            .label(if taken {
                tr("Pris", "Taken")
            } else {
                tr("Prendre", "Take")
            })
            .tooltip(if ours {
                tr("Garder ours pour ce conflit", "Keep ours for this conflict")
            } else {
                tr(
                    "Garder theirs pour ce conflit",
                    "Keep theirs for this conflict",
                )
            })
            .on_click(cx.listener(move |this, ev: &gpui_kit::ClickEvent, _, cx| {
                // ⌥-click: this side after what is already kept.
                if ev.modifiers().alt {
                    this.append_side(conflict, side, cx)
                } else {
                    let r = if ours {
                        Resolution::Ours
                    } else {
                        Resolution::Theirs
                    };
                    this.pick(conflict, r, cx)
                }
            }));
        let append = Button::new(SharedString::from(format!(
            "{}-append:{conflict}",
            side.id()
        )))
        .xsmall()
        .ghost()
        .icon(Icon::new(Lucide::Plus))
        .tooltip(tr("Ajouter à la suite du résultat", "Append to the result"))
        .on_click(cx.listener(move |this, _, _, cx| this.append_side(conflict, side, cx)));

        let header = h_flex()
            .id(SharedString::from(format!("{}-bar:{conflict}", side.id())))
            .flex_none()
            .h(px(BAR_H))
            .px_2()
            .gap_1p5()
            .text_xs()
            .bg(theme::alpha(color, 0.12))
            .border_b_1()
            .border_color(theme::alpha(color, 0.3))
            .when(!lines.is_empty(), |el| {
                el.child(
                    Icon::new(Lucide::GripVertical)
                        .xsmall()
                        .text_color(cx.theme().muted_foreground),
                )
            })
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_color(color)
                    .font_weight(gpui_kit::FontWeight::MEDIUM)
                    .child(if lines.is_empty() {
                        tr("rien de ce côté", "nothing on this side").to_string()
                    } else {
                        trf!("{} ligne(s)", "{} line(s)", lines.len())
                    }),
            )
            .child(div().flex_1())
            .when(resolved && !taken && !lines.is_empty(), |el| {
                el.child(append)
            })
            .child(take)
            .when(!lines.is_empty(), |el| {
                el.cursor_grab()
                    .on_drag(drag_of(whole.clone()), |drag, _, _, cx| {
                        cx.new(|_| drag.clone())
                    })
            });

        let body = if lines.is_empty() {
            placeholder(tr("(lignes supprimées)", "(lines deleted)"), cx)
        } else {
            v_flex()
                .w_full()
                .py(px(2.))
                .font_family(cx.theme().mono_font_family.clone())
                .text_xs()
                .children(lines.iter().enumerate().map(|(i, l)| {
                    let in_result = pieces.is_some_and(|p| kept(p, side, i));
                    code_line(first + i, l, cx)
                        .id(SharedString::from(format!(
                            "{}-line:{conflict}:{i}",
                            side.id()
                        )))
                        .cursor_grab()
                        .when(in_result, |s| s.bg(theme::alpha(color, 0.14)))
                        .when(resolved && !in_result, |s| s.opacity(0.45))
                        .hover(|s| s.bg(theme::alpha(color, 0.24)))
                        .on_drag(drag_of(i..i + 1), |drag, _, _, cx| cx.new(|_| drag.clone()))
                }))
                .into_any_element()
        };

        v_flex()
            .id(SharedString::from(format!("{}-cell:{conflict}", side.id())))
            .test_support()
            .flex_1()
            .min_w_0()
            .bg(theme::alpha(color, if taken { 0.08 } else { 0.04 }))
            .when(ours, |s| s.border_l_2())
            .when(!ours, |s| s.border_r_2())
            .border_color(theme::alpha(color, 0.6))
            .child(header)
            .child(body)
            .drag_over::<CodeDrag>(move |s, drag, _, _| {
                if drag.conflict == conflict && matches!(drag.what, Dragged::Kept(_)) {
                    s.bg(theme::alpha(theme::red(), 0.12))
                } else {
                    s
                }
            })
            .on_drop(cx.listener(move |this, drag: &CodeDrag, _, cx| {
                if let Dragged::Kept(k) = drag.what
                    && drag.conflict == conflict
                {
                    this.remove_piece(conflict, k, cx)
                }
            }))
            .into_any_element()
    }

    /// What one conflict keeps: a bar with the one-click choices, then the
    /// kept pieces (reorderable, removable), or a drop zone while unresolved.
    #[allow(clippy::too_many_arguments)]
    fn result_cell(
        &self,
        conflict: usize,
        count: usize,
        c: &Conflict,
        pieces: Option<&[Piece]>,
        first: usize,
        fresh: Option<usize>,
        current: bool,
        dragging: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let accent = theme::accent();
        let resolved = pieces.is_some();
        let state_color = if resolved { theme::lime() } else { accent };
        let choice = |id: &'static str, label: &'static str, r: Resolution| {
            Button::new((id, conflict))
                .xsmall()
                .ghost()
                .label(label)
                .on_click(cx.listener(move |this, _, _, cx| this.pick(conflict, r, cx)))
        };
        let header = h_flex()
            .flex_none()
            .h(px(BAR_H))
            .px_2()
            .gap_1()
            .overflow_hidden()
            .text_xs()
            .bg(theme::alpha(state_color, if current { 0.2 } else { 0.1 }))
            .border_b_1()
            .border_color(theme::alpha(state_color, 0.35))
            .child(
                Icon::new(if resolved {
                    Lucide::CircleCheck
                } else {
                    Lucide::GitMergeConflict
                })
                .xsmall()
                .text_color(state_color),
            )
            .child(
                div()
                    .flex_none()
                    .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                    .text_color(state_color)
                    .child(format!("{}/{count}", conflict + 1)),
            )
            .child(div().flex_1())
            .child(choice("res-ours", "Ours", Resolution::Ours))
            .child(choice(
                "res-both",
                tr("Les deux", "Both"),
                Resolution::OursThenTheirs,
            ))
            .child(choice("res-theirs", "Theirs", Resolution::Theirs))
            .child(choice(
                "res-none",
                tr("Aucun", "Neither"),
                Resolution::Neither,
            ))
            .when(resolved, |el| {
                el.child(
                    Button::new(("res-undo", conflict))
                        .xsmall()
                        .ghost()
                        .icon(Icon::new(Lucide::Undo2))
                        .tooltip(tr("Annuler ce choix", "Undo this pick"))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.set_choice(conflict, None, false, cx)
                        })),
                )
            });

        let body = match pieces {
            None => v_flex()
                .m_2()
                .py_3()
                .px_2()
                .items_center()
                .justify_center()
                .gap_1()
                .rounded_md()
                .border_1()
                .border_dashed()
                .border_color(theme::alpha(accent, if dragging { 0.9 } else { 0.45 }))
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(
                    Icon::new(Lucide::GripVertical)
                        .small()
                        .text_color(theme::alpha(accent, 0.8)),
                )
                .child(tr(
                    "Glissez des lignes ici, ou choisissez un côté",
                    "Drag lines here, or pick a side",
                ))
                .child(
                    div()
                        .text_color(theme::alpha(theme::muted(), 0.8))
                        .child(trf!(
                            "{} ligne(s) contre {}",
                            "{} line(s) vs {}",
                            c.ours.len(),
                            c.theirs.len()
                        )),
                )
                .into_any_element(),
            Some([]) => placeholder(tr("(rien de gardé)", "(nothing kept)"), cx),
            Some(pieces) => {
                let mut n = first;
                v_flex()
                    .w_full()
                    .py(px(2.))
                    .gap(px(1.))
                    .children(pieces.iter().enumerate().map(|(k, p)| {
                        let color = p.side.color();
                        let text: Vec<&str> = p.text(c).iter().map(String::as_str).collect();
                        let start = n;
                        n += text.len();
                        let drag = CodeDrag {
                            conflict,
                            color,
                            first: text
                                .first()
                                .map(|l| shown(l).trim().to_string())
                                .unwrap_or_default()
                                .into(),
                            count: text.len(),
                            what: Dragged::Kept(k),
                        };
                        h_flex()
                            .id(SharedString::from(format!("kept:{conflict}:{k}")))
                            .items_stretch()
                            .w_full()
                            .bg(theme::alpha(color, 0.1))
                            .cursor_grab()
                            .hover(|s| s.bg(theme::alpha(color, 0.17)))
                            .child(div().flex_none().w(px(3.)).bg(color))
                            .child(div().flex_1().min_w_0().child(code(&text, start, cx)))
                            .child(
                                div().flex_none().w(px(24.)).flex().justify_center().child(
                                    Button::new(SharedString::from(format!(
                                        "kept-remove:{conflict}:{k}"
                                    )))
                                    .xsmall()
                                    .ghost()
                                    .icon(Icon::new(Lucide::X))
                                    .tooltip(tr("Retirer du résultat", "Remove from the result"))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.remove_piece(conflict, k, cx)
                                    })),
                                ),
                            )
                            .on_drag(drag, |drag, _, _, cx| cx.new(|_| drag.clone()))
                            .drag_over::<CodeDrag>(move |s, drag, _, _| {
                                if drag.conflict == conflict {
                                    s.border_t_2().border_color(accent)
                                } else {
                                    s
                                }
                            })
                            .on_drop(cx.listener(move |this, drag: &CodeDrag, _, cx| {
                                this.drop_code(conflict, drag, Some(k), cx)
                            }))
                    }))
                    .when(dragging, |el| {
                        el.child(
                            h_flex()
                                .mx_2()
                                .my_1()
                                .h(px(20.))
                                .justify_center()
                                .rounded_sm()
                                .border_1()
                                .border_dashed()
                                .border_color(theme::alpha(accent, 0.6))
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(tr("déposer à la fin", "drop at the end")),
                        )
                    })
                    .into_any_element()
            }
        };
        let body = match fresh {
            // Just changed: the kept lines fade in with a short glow.
            Some(seq) => div()
                .w_full()
                .child(body)
                .with_animation(
                    ("picked", seq),
                    Animation::new(Duration::from_millis(420))
                        .with_easing(gpui_kit::ease_out_quint()),
                    move |el, t| {
                        el.opacity(0.3 + 0.7 * t)
                            .bg(theme::alpha(accent, 0.12 * (1. - t)))
                    },
                )
                .into_any_element(),
            None => body,
        };

        v_flex()
            .id(SharedString::from(format!("result-cell:{conflict}")))
            .flex_1()
            .min_w_0()
            .border_x_1()
            .border_color(cx.theme().border)
            .bg(ground())
            .child(header)
            .child(body)
            .drag_over::<CodeDrag>(move |s, drag, _, _| {
                if drag.conflict == conflict {
                    s.bg(theme::alpha(accent, 0.08))
                } else {
                    s
                }
            })
            .on_drop(cx.listener(move |this, drag: &CodeDrag, _, cx| {
                this.drop_code(conflict, drag, None, cx)
            }))
            .into_any_element()
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

/// Shortcut hints under the resolver.
fn conflict_hints(cx: &App) -> AnyElement {
    let alt = if cfg!(target_os = "macos") {
        "⌥"
    } else {
        "Alt+"
    };
    let hint = |keys: Vec<String>, text: &'static str| {
        h_flex()
            .flex_none()
            .gap_1()
            .children(keys.into_iter().map(keycap))
            .child(text)
    };
    h_flex()
        .flex_none()
        .h(px(26.))
        .px_3()
        .gap_4()
        .overflow_hidden()
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .bg(bar())
        .border_t_1()
        .border_color(cx.theme().border)
        .child(
            h_flex()
                .flex_none()
                .gap_1()
                .child(Icon::new(Lucide::GripVertical).xsmall())
                .child(tr(
                    "Glissez un bloc ou une ligne vers le résultat",
                    "Drag a block or a line into the result",
                )),
        )
        .child(hint(
            vec![format!("{alt}↑"), format!("{alt}↓")],
            tr("conflit précédent / suivant", "previous / next conflict"),
        ))
        .child(hint(
            vec![format!("{alt}←"), format!("{alt}→")],
            tr("prendre ours / theirs", "take ours / theirs"),
        ))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn piece(side: Side, lines: Range<usize>) -> Piece {
        Piece { side, lines }
    }

    #[test]
    fn dragged_lines_join_their_neighbours() {
        let mut pieces = vec![piece(Side::Ours, 0..1)];
        insert_piece(&mut pieces, piece(Side::Ours, 1..2), None);
        assert_eq!(pieces, vec![piece(Side::Ours, 0..2)]);
        insert_piece(&mut pieces, piece(Side::Theirs, 3..4), Some(0));
        assert_eq!(
            pieces,
            vec![piece(Side::Theirs, 3..4), piece(Side::Ours, 0..2)]
        );
        assert!(kept(&pieces, Side::Ours, 1));
        assert!(!kept(&pieces, Side::Theirs, 0));
    }

    #[test]
    fn kept_pieces_reorder() {
        let mut pieces = vec![
            piece(Side::Ours, 0..1),
            piece(Side::Theirs, 0..2),
            piece(Side::Ours, 4..5),
        ];
        move_piece(&mut pieces, 0, None);
        assert_eq!(
            pieces,
            vec![
                piece(Side::Theirs, 0..2),
                piece(Side::Ours, 4..5),
                piece(Side::Ours, 0..1),
            ]
        );
        move_piece(&mut pieces, 2, Some(0));
        assert_eq!(
            pieces,
            vec![
                piece(Side::Ours, 0..1),
                piece(Side::Theirs, 0..2),
                piece(Side::Ours, 4..5),
            ]
        );
        // Before the piece right after it: no move.
        move_piece(&mut pieces, 0, Some(1));
        assert_eq!(pieces[0], piece(Side::Ours, 0..1));
    }

    #[test]
    fn one_click_resolutions_become_pieces() {
        let c = Conflict {
            ours: vec!["a\n".into()],
            base: vec![],
            theirs: vec![],
        };
        assert_eq!(
            pieces_for(Resolution::TheirsThenOurs, &c),
            vec![piece(Side::Ours, 0..1)]
        );
        assert!(pieces_for(Resolution::Neither, &c).is_empty());
    }
}
