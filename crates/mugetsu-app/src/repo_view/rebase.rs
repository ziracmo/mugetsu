//! Visual interactive rebase: a sheet listing the commits after a base, to
//! reorder by drag and drop and to pick, reword, squash, fix up or drop. A stop
//! on conflicts is handed to the conflict resolver and its operation banner.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use gix::ObjectId;
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::dialog::{DialogAction, DialogClose, DialogFooter};
use gpui_kit::component::input::{Textarea, TextareaState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::{
    Animation, AnimationExt as _, AnyElement, AppContext as _, Context, Entity, Hsla,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window, div, prelude::FluentBuilder as _, px,
};
use mugetsu_git::{RebaseAction, RebaseCommit, RebaseStep};

use super::RepoView;
use crate::i18n::tr;
use crate::theme;

fn action_color(action: RebaseAction) -> Hsla {
    match action {
        RebaseAction::Pick => theme::lime(),
        RebaseAction::Reword => theme::amber(),
        RebaseAction::Squash => theme::magenta(),
        RebaseAction::Fixup => theme::violet(),
        RebaseAction::Drop => theme::red(),
    }
}

fn action_icon(action: RebaseAction) -> Lucide {
    match action {
        RebaseAction::Pick => Lucide::Check,
        RebaseAction::Reword => Lucide::Pencil,
        RebaseAction::Squash => Lucide::Combine,
        RebaseAction::Fixup => Lucide::ArrowDownToLine,
        RebaseAction::Drop => Lucide::Trash,
    }
}

fn action_name(action: RebaseAction) -> &'static str {
    match action {
        RebaseAction::Pick => "pick",
        RebaseAction::Reword => "reword",
        RebaseAction::Squash => "squash",
        RebaseAction::Fixup => "fixup",
        RebaseAction::Drop => "drop",
    }
}

fn action_label(action: RebaseAction) -> &'static str {
    match action {
        RebaseAction::Pick => tr("Garder", "Pick"),
        RebaseAction::Reword => tr("Renommer", "Reword"),
        RebaseAction::Squash => tr("Fusionner", "Squash"),
        RebaseAction::Fixup => "Fixup",
        RebaseAction::Drop => tr("Supprimer", "Drop"),
    }
}

/// Eases `t` (0..1 over the whole animation) for an item starting `delay` in.
fn staggered(t: f32, delay: f32) -> f32 {
    let k = ((t - delay) / (1. - delay)).clamp(0., 1.);
    1. - (1. - k).powi(3)
}

struct Row {
    /// Index in `RebaseForm::commits`.
    commit: usize,
    action: RebaseAction,
    /// Message editor, created the first time the row is reworded.
    message: Option<Entity<TextareaState>>,
    /// Bumped when the action changes (restarts its pop).
    changed: usize,
}

/// A row being dragged; also its floating preview.
#[derive(Clone)]
struct RowDrag {
    from: usize,
    short: SharedString,
    summary: SharedString,
    color: Hsla,
}

impl Render for RowDrag {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .w(px(420.))
            .h(px(36.))
            .px_3()
            .gap_2()
            .rounded_lg()
            .bg(theme::popover())
            .border_1()
            .border_color(theme::alpha(theme::accent(), 0.8))
            .shadow_xl()
            .text_sm()
            .child(
                Icon::new(Lucide::GripVertical)
                    .xsmall()
                    .text_color(theme::accent()),
            )
            .child(div().size(px(8.)).rounded_full().bg(self.color))
            .child(
                div()
                    .text_xs()
                    .font_family(cx.theme().mono_font_family.clone())
                    .text_color(theme::muted())
                    .child(self.short.clone()),
            )
            .child(div().flex_1().truncate().child(self.summary.clone()))
            .with_animation(
                "rebase-lift",
                Animation::new(Duration::from_millis(180)).with_easing(gpui_kit::ease_out_quint()),
                |el, t| el.opacity(0.6 + 0.4 * t).mt(px(4. * (1. - t))),
            )
    }
}

pub struct RebaseForm {
    branch: String,
    base: String,
    commits: Vec<RebaseCommit>,
    /// Newest first, like the history.
    rows: Vec<Row>,
    /// Row being dragged (set by the drag preview constructor).
    dragging: Rc<Cell<Option<usize>>>,
    /// Commit just dropped in place, with its sequence (settle glow).
    settled: Option<(usize, usize)>,
    seq: usize,
}

impl RebaseForm {
    fn new(branch: String, base: String, commits: Vec<RebaseCommit>) -> Self {
        let rows = (0..commits.len())
            .rev()
            .map(|commit| Row {
                commit,
                action: RebaseAction::Pick,
                message: None,
                changed: 0,
            })
            .collect();
        Self {
            branch,
            base,
            commits,
            rows,
            dragging: Rc::default(),
            settled: None,
            seq: 0,
        }
    }

    fn move_row(&mut self, from: usize, to: usize, cx: &mut Context<Self>) {
        self.dragging.set(None);
        if from == to || from >= self.rows.len() || to >= self.rows.len() {
            cx.notify();
            return;
        }
        let row = self.rows.remove(from);
        let commit = row.commit;
        self.rows.insert(to, row);
        self.seq += 1;
        self.settled = Some((commit, self.seq));
        cx.notify();
    }

    fn set_action(
        &mut self,
        index: usize,
        action: RebaseAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let message = self.commits[self.rows[index].commit].message.clone();
        let row = &mut self.rows[index];
        if row.action == action {
            return;
        }
        row.action = action;
        self.seq += 1;
        row.changed = self.seq;
        if action == RebaseAction::Reword {
            let editor = row.message.get_or_insert_with(|| {
                cx.new(|cx| {
                    let mut state = TextareaState::new(window, cx).default_value(message);
                    state.set_auto_grow(2, 6, cx);
                    state
                })
            });
            editor.update(cx, |e, cx| e.focus(window, cx));
        }
        cx.notify();
    }

    /// The plan, oldest first.
    fn steps(&self, cx: &gpui_kit::App) -> Vec<RebaseStep> {
        self.rows
            .iter()
            .rev()
            .map(|row| RebaseStep {
                id: self.commits[row.commit].id.clone(),
                action: row.action,
                message: (row.action == RebaseAction::Reword)
                    .then(|| row.message.as_ref().map(|m| m.read(cx).value().to_string()))
                    .flatten(),
            })
            .collect()
    }

    /// Nothing moved and every commit is picked.
    fn is_noop(&self) -> bool {
        self.rows
            .iter()
            .rev()
            .enumerate()
            .all(|(i, r)| r.commit == i && r.action == RebaseAction::Pick)
    }

    /// Why the plan can't run, if it can't.
    fn problem(&self) -> Option<&'static str> {
        let oldest_kept = self
            .rows
            .iter()
            .rev()
            .find(|r| r.action != RebaseAction::Drop)?;
        oldest_kept.action.melds().then(|| {
            tr(
                "Le commit le plus ancien gardé ne peut pas être fusionné : il n'a rien en dessous.",
                "The oldest kept commit can't be melded: there is nothing below it.",
            )
        })
    }

    /// Commits left once squashes, fixups and drops are applied.
    fn result_count(&self) -> usize {
        self.rows
            .iter()
            .filter(|r| r.action != RebaseAction::Drop && !r.action.melds())
            .count()
    }

    /// The commit a melding row lands in: the next kept row below it.
    fn meld_target(&self, index: usize) -> Option<&RebaseCommit> {
        self.rows[index + 1..]
            .iter()
            .find(|r| r.action != RebaseAction::Drop && !r.action.melds())
            .map(|r| &self.commits[r.commit])
    }

    fn render_actions(&self, index: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let row = &self.rows[index];
        let current = row.action;
        let changed = row.changed;
        h_flex()
            .flex_none()
            .gap_0p5()
            .p(px(2.))
            .rounded_md()
            .bg(theme::alpha(gpui_kit::white(), 0.04))
            .children(RebaseAction::ALL.into_iter().map(|action| {
                let selected = action == current;
                let color = action_color(action);
                div()
                    .id(SharedString::from(format!(
                        "rebase-{}-{index}",
                        action_name(action)
                    )))
                    .test_support()
                    .size(px(22.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(5.))
                    .cursor_pointer()
                    .border_1()
                    .map(|s| {
                        if selected {
                            s.bg(theme::alpha(color, 0.22))
                                .border_color(theme::alpha(color, 0.6))
                        } else {
                            s.border_color(gpui_kit::transparent_black())
                                .hover(|s| s.bg(theme::raised()))
                        }
                    })
                    .child(
                        Icon::new(action_icon(action))
                            .xsmall()
                            .text_color(if selected {
                                color
                            } else {
                                theme::alpha(gpui_kit::white(), 0.38)
                            }),
                    )
                    .tooltip(move |window, cx| {
                        gpui_kit::component::tooltip::Tooltip::new(action_label(action))
                            .build(window, cx)
                    })
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        this.set_action(index, action, window, cx)
                    }))
                    .map(|s| {
                        if !(selected && changed > 0) {
                            return s.into_any_element();
                        }
                        s.with_animation(
                            SharedString::from(format!("rebase-pop-{index}-{changed}")),
                            Animation::new(Duration::from_millis(420))
                                .with_easing(gpui_kit::ease_out_quint()),
                            move |el, t| {
                                el.bg(theme::alpha(color, 0.6 - 0.38 * t)).shadow(vec![
                                    gpui_kit::BoxShadow {
                                        color: theme::alpha(color, 0.55 * (1. - t)),
                                        offset: gpui_kit::point(px(0.), px(0.)),
                                        blur_radius: px(10. * (1. - t)),
                                        spread_radius: px(2. * (1. - t)),
                                        inset: false,
                                    },
                                ])
                            },
                        )
                        .into_any_element()
                    })
                    .into_any_element()
            }))
    }

    fn render_row(
        &self,
        index: usize,
        dragging: Option<usize>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let row = &self.rows[index];
        let commit = &self.commits[row.commit];
        let color = action_color(row.action);
        let dropped = row.action == RebaseAction::Drop;
        let melds = row.action.melds();
        let lifted = dragging == Some(index);
        let total = self.commits.len().max(1) as f32;
        let drag = RowDrag {
            from: index,
            short: commit.short_id().to_string().into(),
            summary: commit.summary().to_string().into(),
            color,
        };
        let flag = self.dragging.clone();
        let header = h_flex()
            .h(px(34.))
            .px_2()
            .gap_2()
            .child(
                div()
                    .id(SharedString::from(format!("rebase-grip-{index}")))
                    .flex_none()
                    .cursor_grab()
                    .child(
                        Icon::new(Lucide::GripVertical)
                            .xsmall()
                            .text_color(theme::alpha(gpui_kit::white(), 0.35)),
                    ),
            )
            .child(self.render_actions(index, cx))
            .when(melds, |s| {
                s.child(Icon::new(Lucide::CornerLeftDown).xsmall().text_color(color))
            })
            .child(
                div()
                    .flex_none()
                    .text_xs()
                    .font_family(cx.theme().mono_font_family.clone())
                    .text_color(theme::muted())
                    .child(commit.short_id().to_string()),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_sm()
                    .when(dropped, |s| s.line_through().text_color(theme::muted()))
                    .child(commit.summary().to_string()),
            )
            .when_some(
                melds.then(|| self.meld_target(index)).flatten(),
                |s, target| {
                    s.child(
                        div()
                            .flex_none()
                            .px_1p5()
                            .rounded_md()
                            .text_xs()
                            .text_color(color)
                            .bg(theme::alpha(color, 0.12))
                            .child(trf!("dans {}", "into {}", target.short_id())),
                    )
                },
            )
            .child(
                div()
                    .flex_none()
                    .max_w(px(110.))
                    .truncate()
                    .text_xs()
                    .text_color(theme::muted())
                    .child(commit.author.clone()),
            );
        let editor = (row.action == RebaseAction::Reword)
            .then(|| row.message.clone())
            .flatten();
        let settle = self
            .settled
            .filter(|(c, _)| *c == row.commit)
            .map(|(_, seq)| seq);
        let enter_delay = (row.commit as f32 / total).min(1.) * 0.45;
        v_flex()
            .id(SharedString::from(format!("rebase-row-{index}")))
            .test_support()
            .rounded_lg()
            .border_1()
            .border_color(theme::hairline())
            .bg(theme::alpha(gpui_kit::white(), 0.03))
            .when(melds, |s| {
                // Hangs off the commit it melds into.
                s.ml(px(22.))
                    .border_l_2()
                    .border_color(theme::alpha(color, 0.55))
            })
            .when(dropped, |s| {
                s.bg(theme::alpha(theme::red(), 0.05))
                    .border_color(theme::alpha(theme::red(), 0.25))
            })
            .when(lifted, |s| {
                s.opacity(0.35)
                    .border_dashed()
                    .border_color(theme::alpha(theme::accent(), 0.7))
            })
            .on_drag(drag, move |drag, _, _, cx| {
                flag.set(Some(drag.from));
                cx.new(|_| drag.clone())
            })
            .drag_over::<RowDrag>(move |s, drag, _, _| {
                if drag.from == index {
                    return s;
                }
                s.bg(theme::alpha(theme::accent(), 0.16))
                    .border_color(theme::accent())
            })
            .on_drop(
                cx.listener(move |this, drag: &RowDrag, _, cx| this.move_row(drag.from, index, cx)),
            )
            .child(header)
            .when_some(editor, |s, editor| {
                s.child(
                    div()
                        .px_2()
                        .pb_2()
                        .pl(px(38.))
                        .child(Textarea::new(&editor).small()),
                )
            })
            .map(|el| match settle {
                Some(seq) => el
                    .with_animation(
                        ("rebase-settle", seq),
                        Animation::new(Duration::from_millis(700))
                            .with_easing(gpui_kit::ease_out_quint()),
                        |el, t| {
                            el.bg(theme::alpha(theme::accent(), 0.28 * (1. - t)))
                                .border_color(theme::alpha(theme::accent(), 1. - 0.9 * t))
                                .shadow(vec![gpui_kit::BoxShadow {
                                    color: theme::alpha(theme::accent(), 0.45 * (1. - t)),
                                    offset: gpui_kit::point(px(0.), px(0.)),
                                    blur_radius: px(16. * (1. - t)),
                                    spread_radius: px(0.),
                                    inset: false,
                                }])
                        },
                    )
                    .into_any_element(),
                None => el
                    .with_animation(
                        ("rebase-in", row.commit),
                        Animation::new(Duration::from_millis(420 + 18 * total.min(20.) as u64)),
                        move |el, t| {
                            let k = staggered(t, enter_delay);
                            el.opacity(k).mt(px(10. * (1. - k)))
                        },
                    )
                    .into_any_element(),
            })
    }

    fn render_summary(&self) -> impl IntoElement {
        let before = self.commits.len();
        let after = self.result_count();
        let count = |action: RebaseAction| self.rows.iter().filter(|r| r.action == action).count();
        let moved = self
            .rows
            .iter()
            .rev()
            .enumerate()
            .filter(|(i, r)| r.commit != *i)
            .count();
        let mut chips: Vec<AnyElement> = Vec::new();
        if moved > 0 {
            chips.push(summary_chip(
                Lucide::ArrowDownUp,
                theme::accent(),
                trf!("{moved} déplacé(s)", "{moved} moved"),
            ));
        }
        for action in [
            RebaseAction::Reword,
            RebaseAction::Squash,
            RebaseAction::Fixup,
            RebaseAction::Drop,
        ] {
            let n = count(action);
            if n > 0 {
                chips.push(summary_chip(
                    action_icon(action),
                    action_color(action),
                    format!("{n} {}", action_label(action).to_lowercase()),
                ));
            }
        }
        v_flex()
            .gap_2()
            .child(
                h_flex()
                    .gap_2()
                    .flex_wrap()
                    .text_sm()
                    .child(
                        h_flex()
                            .gap_1p5()
                            .child(
                                div()
                                    .text_color(theme::muted())
                                    .child(trf!("{before} commit(s)", "{before} commit(s)")),
                            )
                            .child(
                                Icon::new(Lucide::ArrowRight)
                                    .xsmall()
                                    .text_color(theme::muted()),
                            )
                            .child(
                                div()
                                    .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                                    .child(after.to_string()),
                            ),
                    )
                    .children(chips)
                    .when(self.is_noop(), |s| {
                        s.child(div().text_color(theme::muted()).child(tr(
                            "· rien ne change pour l'instant",
                            "· nothing changes yet",
                        )))
                    }),
            )
            .when_some(self.problem(), |s, problem| {
                s.child(
                    h_flex()
                        .gap_2()
                        .text_sm()
                        .text_color(theme::red())
                        .child(Icon::new(Lucide::TriangleAlert).xsmall())
                        .child(problem),
                )
            })
    }
}

fn summary_chip(icon: Lucide, color: Hsla, text: String) -> AnyElement {
    h_flex()
        .gap_1()
        .px_1p5()
        .h(px(20.))
        .rounded_md()
        .text_xs()
        .text_color(color)
        .bg(theme::alpha(color, 0.12))
        .border_1()
        .border_color(theme::alpha(color, 0.35))
        .child(Icon::new(icon).xsmall())
        .child(text)
        .into_any_element()
}

fn pill(name: &str, icon: Lucide, color: Hsla) -> impl IntoElement {
    h_flex()
        .flex_none()
        .max_w(px(220.))
        .gap_1()
        .px_2()
        .h(px(22.))
        .rounded_md()
        .border_1()
        .border_color(theme::alpha(color, 0.5))
        .bg(theme::alpha(color, 0.14))
        .text_xs()
        .text_color(color)
        .child(Icon::new(icon).xsmall())
        .child(div().truncate().child(name.to_string()))
}

impl Render for RebaseForm {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // A drag that ended outside any row leaves no lifted row behind.
        let dragging = if cx.has_active_drag() {
            self.dragging.get()
        } else {
            None
        };
        let rows: Vec<AnyElement> = (0..self.rows.len())
            .map(|i| self.render_row(i, dragging, cx))
            .collect();
        v_flex()
            .id("rebase-form")
            .gap_4()
            .child(
                h_flex()
                    .gap_3()
                    .child(
                        div()
                            .flex_none()
                            .size(px(44.))
                            .rounded_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .bg(theme::alpha(theme::accent(), 0.16))
                            .border_1()
                            .border_color(theme::alpha(theme::accent(), 0.45))
                            .child(
                                Icon::new(Lucide::ListOrdered)
                                    .size(px(20.))
                                    .text_color(theme::accent()),
                            )
                            .with_animation(
                                "rebase-badge",
                                Animation::new(Duration::from_millis(520))
                                    .with_easing(gpui_kit::ease_out_quint()),
                                |el, t| {
                                    el.shadow(vec![gpui_kit::BoxShadow {
                                        color: theme::alpha(theme::accent(), 0.5 * (1. - t)),
                                        offset: gpui_kit::point(px(0.), px(0.)),
                                        blur_radius: px(18. * (1. - t)),
                                        spread_radius: px(6. * (1. - t)),
                                        inset: false,
                                    }])
                                },
                            ),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .gap_1()
                            .child(
                                div()
                                    .text_lg()
                                    .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                                    .child(tr("Rebase interactif", "Interactive rebase")),
                            )
                            .child(
                                h_flex()
                                    .gap_1p5()
                                    .text_xs()
                                    .text_color(theme::muted())
                                    .child(pill(&self.branch, Lucide::GitBranch, theme::accent()))
                                    .child(tr("rejoué sur", "replayed onto"))
                                    .child(pill(
                                        &self.base,
                                        Lucide::GitCommitHorizontal,
                                        theme::violet(),
                                    )),
                            ),
                    ),
            )
            .child(
                h_flex()
                    .gap_3()
                    .text_xs()
                    .text_color(theme::muted())
                    .child(tr(
                        "Glisse pour réordonner, le plus récent en haut.",
                        "Drag to reorder, newest on top.",
                    ))
                    .child(div().flex_1())
                    .children(RebaseAction::ALL.into_iter().map(|a| {
                        h_flex()
                            .gap_1()
                            .child(
                                Icon::new(action_icon(a))
                                    .xsmall()
                                    .text_color(action_color(a)),
                            )
                            .child(action_label(a))
                    })),
            )
            .child(
                v_flex()
                    .id("rebase-rows")
                    .gap_1()
                    .max_h(px(420.))
                    .overflow_y_scroll()
                    .children(rows),
            )
            .child(self.render_summary())
    }
}

impl RepoView {
    /// Loads the commits after `base` on HEAD, then opens the rebase sheet.
    pub fn open_rebase(&mut self, base: ObjectId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(branch) = self.data.refs.head_branch.clone() else {
            return;
        };
        let repo = self.repo.clone();
        let sha = base.to_string();
        cx.spawn_in(window, async move |this, cx| {
            let plan = cx
                .background_spawn(async move { repo.rebase_plan(&sha) })
                .await;
            let _ = this.update_in(cx, |this, window, cx| match plan {
                Err(e) => window.push_notification(
                    crate::toast::error(tr("Rebase : échec", "Rebase failed"), &e, None),
                    cx,
                ),
                Ok(commits) if commits.is_empty() => window.push_notification(
                    Notification::info(tr("Rien à rebaser", "Nothing to rebase")),
                    cx,
                ),
                Ok(commits) => this.show_rebase_sheet(branch, base, commits, window, cx),
            });
        })
        .detach();
    }

    fn show_rebase_sheet(
        &mut self,
        branch: String,
        base: ObjectId,
        commits: Vec<RebaseCommit>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let short = base.to_hex_with_len(7).to_string();
        let form = cx.new(|_| RebaseForm::new(branch, short, commits));
        let view = cx.entity().downgrade();
        let base = base.to_string();
        window.open_dialog(cx, move |dialog, window, _| {
            let top = ((window.viewport_size().height - px(640.)) / 2.).max(px(40.));
            let (form, view, base) = (form.clone(), view.clone(), base.clone());
            dialog
                .bg(theme::sheet())
                .w(px(720.))
                .margin_top(top)
                .child(form.clone())
                .footer(
                    DialogFooter::new()
                        .child(
                            DialogClose::new()
                                .trigger(|b| b.ghost().label(tr("Annuler", "Cancel"))),
                        )
                        .child(
                            DialogAction::new().child(
                                Button::new("rebase-ok")
                                    .primary()
                                    .icon(Icon::new(Lucide::ListOrdered))
                                    .label(tr("Rebaser", "Rebase")),
                            ),
                        ),
                )
                .on_ok(move |_, window, cx| {
                    let f = form.read(cx);
                    if let Some(problem) = f.problem() {
                        window.push_notification(Notification::warning(problem), cx);
                        return false;
                    }
                    if f.is_noop() {
                        return true;
                    }
                    let steps = f.steps(cx);
                    let base = base.clone();
                    let _ = view.update(cx, |this, cx| this.run_rebase(base, steps, window, cx));
                    true
                })
        });
    }

    /// Runs the rebase in the background. A stop on conflicts fails the
    /// operation, so `run_op` reports it and the resolver opens on reload.
    pub(crate) fn run_rebase(
        &mut self,
        base: String,
        steps: Vec<RebaseStep>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.run_op(
            "Rebase",
            move |r| {
                if r.rebase_interactive(&base, &steps)? {
                    Ok(())
                } else {
                    anyhow::bail!("stopped on conflicts")
                }
            },
            window,
            cx,
        );
    }
}
