//! Catch-up: rebases the local branches lagging behind their target (the
//! base of their PR, else the trunk) when they replay without a conflict.
//!
//! A scan after every reload (so after every fetch) replays them in memory
//! (see `mugetsu_git::catch_up`); a badge on the Branches section counts the
//! clean ones. Its popover lists them, keyboard first (↑↓, P pick, S skip,
//! ⌘↵ rebase), then shows the run row by row and how it ended.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::Duration;

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{Disableable as _, Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    Animation, AnimationExt as _, AnyElement, App, AppContext as _, BoxShadow, Context,
    EventEmitter, FocusHandle, Focusable, Hsla, InteractiveElement as _, IntoElement, KeyBinding,
    ParentElement as _, Pixels, Point, Render, ScrollHandle, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, Window, anchored, deferred, div,
    linear_color_stop, linear_gradient, point, prelude::FluentBuilder as _, px, relative,
};
use mugetsu_forge::PrState;
use mugetsu_git::{CatchUp, CatchUpRequest, Moved, Verdict};

use super::RepoView;
use crate::i18n::tr;
use crate::theme;

gpui_kit::actions!(catch_up, [Prev, Next, Pick, Skip, Toggle, Submit, Dismiss]);

const CONTEXT: &str = "CatchUp";

pub fn init(cx: &mut App) {
    let ctx = Some(CONTEXT);
    cx.bind_keys([
        KeyBinding::new("up", Prev, ctx),
        KeyBinding::new("k", Prev, ctx),
        KeyBinding::new("down", Next, ctx),
        KeyBinding::new("j", Next, ctx),
        KeyBinding::new("p", Pick, ctx),
        KeyBinding::new("s", Skip, ctx),
        KeyBinding::new("space", Toggle, ctx),
        KeyBinding::new("secondary-enter", Submit, ctx),
        KeyBinding::new("escape", Dismiss, ctx),
    ]);
}

// ---- Panel ------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
enum RowState {
    /// Listed, picked or not (`CatchUpPanel::picked`).
    Idle,
    Waiting,
    Rebasing,
    Pushing,
    Done {
        pushed: bool,
    },
    /// Nothing left to do when its turn came.
    Current,
    Failed(String),
    Skipped,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Choosing,
    Running,
    Finished,
}

pub enum CatchUpEvent {
    /// The picked branches, parents first.
    Submit(Vec<CatchUp>),
    Close,
}

pub struct CatchUpPanel {
    /// Branches that replay cleanly, stacks parent first.
    items: Vec<CatchUp>,
    /// Those that don't, with why.
    blocked: Vec<CatchUp>,
    picked: Vec<bool>,
    states: Vec<RowState>,
    /// Per row: bumped when it changes, restarts its pop.
    bumps: Vec<usize>,
    selected: usize,
    /// Bumped on each change (selection, pick), ids of the animations.
    seq: usize,
    phase: Phase,
    show_blocked: bool,
    focus: FocusHandle,
    scroll: ScrollHandle,
}

impl EventEmitter<CatchUpEvent> for CatchUpPanel {}

impl Focusable for CatchUpPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl CatchUpPanel {
    fn new(scan: &[CatchUp], cx: &mut Context<Self>) -> Self {
        let (items, blocked): (Vec<CatchUp>, Vec<CatchUp>) =
            scan.iter().cloned().partition(|c| c.verdict.is_clean());
        let n = items.len();
        Self {
            items,
            blocked,
            picked: vec![true; n],
            states: vec![RowState::Idle; n],
            bumps: vec![0; n],
            selected: 0,
            seq: 0,
            phase: Phase::Choosing,
            show_blocked: false,
            focus: cx.focus_handle(),
            scroll: ScrollHandle::new(),
        }
    }

    fn picked_count(&self) -> usize {
        self.picked.iter().filter(|p| **p).count()
    }

    fn step(&mut self, delta: isize, cx: &mut Context<Self>) {
        if self.phase != Phase::Choosing || self.items.is_empty() {
            return;
        }
        let n = self.items.len() as isize;
        self.selected = (self.selected as isize + delta).rem_euclid(n) as usize;
        self.scroll.scroll_to_item(self.selected);
        self.seq += 1;
        cx.notify();
    }

    /// Picks or skips row `i`. A stack holds together: picking a branch
    /// picks the branches it rests on, skipping one skips those resting on it.
    fn set_pick(&mut self, i: usize, pick: bool, cx: &mut Context<Self>) {
        if self.phase != Phase::Choosing || i >= self.items.len() {
            return;
        }
        self.seq += 1;
        let mut todo = vec![i];
        while let Some(j) = todo.pop() {
            if self.picked[j] == pick && j != i {
                continue;
            }
            if self.picked[j] != pick {
                self.picked[j] = pick;
                self.bumps[j] = self.seq;
            }
            let item = &self.items[j];
            for (k, other) in self.items.iter().enumerate() {
                let linked = if pick {
                    other.branch == item.target
                } else {
                    other.target == item.branch
                };
                if linked && self.picked[k] != pick {
                    todo.push(k);
                }
            }
        }
        cx.notify();
    }

    fn pick(&mut self, pick: bool, cx: &mut Context<Self>) {
        self.set_pick(self.selected, pick, cx);
        // Moves on, like a review: one key per branch.
        if self.selected + 1 < self.items.len() {
            self.step(1, cx);
        }
    }

    fn submit(&mut self, cx: &mut Context<Self>) {
        if self.phase != Phase::Choosing || self.picked_count() == 0 {
            return;
        }
        let picks = (0..self.items.len())
            .filter(|i| self.picked[*i])
            .map(|i| self.items[i].clone())
            .collect();
        cx.emit(CatchUpEvent::Submit(picks));
    }

    fn dismiss(&mut self, cx: &mut Context<Self>) {
        if self.phase != Phase::Running {
            cx.emit(CatchUpEvent::Close);
        }
    }

    fn start(&mut self, cx: &mut Context<Self>) {
        self.phase = Phase::Running;
        self.seq += 1;
        for i in 0..self.items.len() {
            self.states[i] = if self.picked[i] {
                RowState::Waiting
            } else {
                RowState::Skipped
            };
        }
        cx.notify();
    }

    fn set_state(&mut self, branch: &str, state: RowState, cx: &mut Context<Self>) {
        if let Some(i) = self.items.iter().position(|c| c.branch == branch) {
            self.seq += 1;
            self.states[i] = state;
            self.bumps[i] = self.seq;
            cx.notify();
        }
    }

    fn finish(&mut self, cx: &mut Context<Self>) {
        self.phase = Phase::Finished;
        self.seq += 1;
        cx.notify();
    }

    /// (rebased, pushed, failed) once run.
    fn tally(&self) -> (usize, usize, usize) {
        self.states
            .iter()
            .fold((0, 0, 0), |(ok, pushed, ko), s| match s {
                RowState::Done { pushed: true } => (ok + 1, pushed + 1, ko),
                RowState::Done { pushed: false } | RowState::Current => (ok + 1, pushed, ko),
                RowState::Failed(_) => (ok, pushed, ko + 1),
                _ => (ok, pushed, ko),
            })
    }

    fn succeeded(&self) -> bool {
        self.phase == Phase::Finished && self.tally().2 == 0
    }
}

// ---- Rendering --------------------------------------------------------------

/// easeOutBack: overshoots a little before settling.
fn back(t: f32) -> f32 {
    let (c1, c3) = (1.70158, 2.70158);
    1. + c3 * (t - 1.).powi(3) + c1 * (t - 1.).powi(2)
}

fn stagger(t: f32, i: usize) -> f32 {
    let delay = (i as f32 * 0.06).min(0.6);
    let k = ((t - delay) / (1. - delay)).clamp(0., 1.);
    1. - (1. - k).powi(3)
}

fn keycap(text: &'static str) -> impl IntoElement {
    div()
        .min_w(px(18.))
        .h(px(18.))
        .px_1()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(5.))
        .border_1()
        .border_color(theme::hairline())
        .bg(theme::alpha(gpui_kit::white(), 0.05))
        .text_xs()
        .text_color(theme::alpha(gpui_kit::white(), 0.8))
        .child(text)
}

fn hint(keys: &[&'static str], label: &'static str) -> impl IntoElement {
    h_flex()
        .gap_1()
        .children(keys.iter().map(|k| keycap(k)))
        .child(div().text_color(theme::muted()).child(label))
}

fn pill(text: &str, color: Hsla) -> impl IntoElement {
    div()
        .flex_none()
        .max_w(px(150.))
        .truncate()
        .px_1p5()
        .rounded_md()
        .text_xs()
        .bg(theme::alpha(color, 0.14))
        .text_color(color)
        .child(text.to_string())
}

/// Why a push failed, the branch being back where it was.
fn push_failure(error: &str) -> String {
    if error.contains("stale info") || error.contains("fetch first") {
        tr(
            "Quelqu'un a poussé sur cette branche depuis le dernier fetch : elle est remise \
             en place, rien n'est écrasé. Pull puis réessaie.",
            "Someone pushed to this branch since the last fetch: it is put back, nothing is \
             overwritten. Pull, then try again.",
        )
        .into()
    } else {
        trf!(
            "Push refusé, branche remise en place : {error}",
            "Push refused, branch put back: {error}"
        )
    }
}

/// Why a branch is left out, in a few words.
fn blocked_reason(v: &Verdict) -> String {
    match v {
        Verdict::Clean => String::new(),
        Verdict::Conflict { commit, paths, .. } => trf!(
            "Conflit ({commit}) sur {}",
            "Conflict ({commit}) on {}",
            paths.join(", ")
        ),
        Verdict::Dirty { .. } => tr(
            "Modifications non commitées dans son worktree",
            "Uncommitted changes in its worktree",
        )
        .into(),
        Verdict::RemoteAhead { upstream } => trf!(
            "{upstream} a des commits en plus : pull d'abord",
            "{upstream} has extra commits: pull first"
        ),
        Verdict::Merges => tr("Contient des commits de merge", "Contains merge commits").into(),
        Verdict::ParentBlocked { parent } => trf!(
            "Repose sur {parent}, bloquée",
            "Rests on {parent}, which is blocked"
        ),
    }
}

impl CatchUpPanel {
    fn render_glyph(&self, i: usize) -> AnyElement {
        let seq = self.bumps[i];
        let picked = self.picked[i];
        let (icon, color) = match &self.states[i] {
            RowState::Idle if picked => (Lucide::CircleCheck, theme::lime()),
            RowState::Idle | RowState::Skipped => (Lucide::CircleMinus, theme::muted()),
            RowState::Waiting => (Lucide::CircleDashed, theme::muted()),
            RowState::Rebasing => (Lucide::LoaderCircle, theme::accent()),
            RowState::Pushing => (Lucide::ArrowUpFromLine, theme::violet()),
            RowState::Done { .. } => (Lucide::CircleCheckBig, theme::lime()),
            RowState::Current => (Lucide::CircleCheck, theme::muted()),
            RowState::Failed(_) => (Lucide::CircleX, theme::red()),
        };
        let glyph = Icon::new(icon).small().text_color(color);
        let glyph: AnyElement = match self.states[i] {
            RowState::Rebasing => glyph
                .with_animation(
                    ("catch-up-spin", i),
                    Animation::new(Duration::from_millis(800)).repeat(),
                    |icon, t| icon.rotate(gpui_kit::radians(t * std::f32::consts::TAU)),
                )
                .into_any_element(),
            RowState::Pushing => div()
                .relative()
                .child(glyph)
                .with_animation(
                    ("catch-up-push", i),
                    Animation::new(Duration::from_millis(700))
                        .repeat()
                        .with_easing(gpui_kit::ease_in_out),
                    |el, t| el.top(px(-2. * (t * std::f32::consts::PI).sin())),
                )
                .into_any_element(),
            _ => glyph.into_any_element(),
        };
        // Pops when it changes: a pick, a skip, a step of the run.
        div()
            .flex_none()
            .size(px(20.))
            .flex()
            .items_center()
            .justify_center()
            .rounded_full()
            .child(glyph)
            .with_animation(
                ("catch-up-pop", seq * 1024 + i),
                Animation::new(Duration::from_millis(340)),
                move |el, t| {
                    let k = if seq == 0 { 1. } else { back(t) };
                    el.bg(theme::alpha(color, 0.25 * (1. - t).max(0.)))
                        .opacity(0.4 + 0.6 * k.min(1.))
                        .ml(px(2. * (1. - k)))
                },
            )
            .into_any_element()
    }

    fn render_row(&self, i: usize, cx: &mut Context<Self>) -> AnyElement {
        let item = &self.items[i];
        let choosing = self.phase == Phase::Choosing;
        let selected = choosing && i == self.selected;
        let picked = self.picked[i];
        let faded = matches!(self.states[i], RowState::Skipped) || (choosing && !picked);
        let stacked = self.items.iter().any(|o| o.branch == item.target);
        let mut meta: Vec<String> = vec![trf!(
            "{} commit(s) rejoué(s)",
            "{} commit(s) replayed",
            item.commits
        )];
        if let Some(u) = &item.upstream {
            meta.push(trf!(
                "force-push {}/{}",
                "force-push {}/{}",
                u.remote,
                u.branch
            ));
        }
        if item.worktree.is_some() {
            meta.push(tr("checkout suivi", "checkout follows").into());
        }
        let failure = match &self.states[i] {
            RowState::Failed(e) => Some(e.clone()),
            _ => None,
        };
        let status = match &self.states[i] {
            RowState::Done { pushed: true } => Some(tr("rebasée · poussée", "rebased · pushed")),
            RowState::Done { pushed: false } => Some(tr("rebasée", "rebased")),
            RowState::Current => Some(tr("déjà à jour", "already up to date")),
            RowState::Skipped => Some(tr("passée", "skipped")),
            _ => None,
        };
        let id = SharedString::from(format!("catch-up:{}", item.branch));
        let seq = self.seq;
        let row = v_flex()
            .id(id)
            .test_support()
            .relative()
            .px_2()
            .py_1p5()
            .gap_0p5()
            .rounded_lg()
            .border_1()
            .border_color(gpui_kit::transparent_black())
            .when(choosing, |s| {
                s.cursor_pointer()
                    .hover(|s| s.bg(theme::alpha(gpui_kit::white(), 0.04)))
            })
            .when(selected, |s| {
                s.border_color(theme::alpha(theme::accent(), 0.55))
                    .bg(theme::alpha(theme::accent(), 0.12))
            })
            .when(failure.is_some(), |s| {
                s.border_color(theme::alpha(theme::red(), 0.4))
                    .bg(theme::alpha(theme::red(), 0.07))
            })
            .child(
                h_flex()
                    .gap_2()
                    .text_sm()
                    .when(stacked, |s| {
                        s.pl_3().child(
                            Icon::new(Lucide::CornerDownRight)
                                .xsmall()
                                .text_color(theme::muted()),
                        )
                    })
                    .child(self.render_glyph(i))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                            .text_color(if faded {
                                theme::muted()
                            } else {
                                theme::alpha(gpui_kit::white(), 0.93)
                            })
                            .when(faded, |s| s.line_through())
                            .child(item.branch.clone()),
                    )
                    .when(item.behind > 0, |s| {
                        s.child(
                            div()
                                .flex_none()
                                .text_xs()
                                .text_color(theme::amber())
                                .child(format!("↓{}", item.behind)),
                        )
                    })
                    .child(
                        Icon::new(Lucide::ArrowRight)
                            .xsmall()
                            .text_color(theme::muted()),
                    )
                    .child(pill(
                        &item.target,
                        if stacked {
                            theme::magenta()
                        } else {
                            theme::violet()
                        },
                    )),
            )
            .child(
                h_flex()
                    .pl(px(if stacked { 40. } else { 28. }))
                    .gap_2()
                    .text_xs()
                    .text_color(theme::muted())
                    .child(div().flex_1().min_w_0().truncate().child(meta.join(" · ")))
                    .children(status.map(|s| {
                        div()
                            .flex_none()
                            .text_color(match &self.states[i] {
                                RowState::Done { .. } => theme::lime(),
                                _ => theme::muted(),
                            })
                            .child(s)
                    })),
            )
            .children(failure.map(|e| {
                div()
                    .pl(px(28.))
                    .text_xs()
                    .text_color(theme::alpha(theme::red(), 0.9))
                    .child(e)
                    .with_animation(
                        ("catch-up-why", seq * 64 + i),
                        Animation::new(Duration::from_millis(360))
                            .with_easing(gpui_kit::ease_out_quint()),
                        |el, t| el.opacity(t).mt(px(-6. * (1. - t))),
                    )
            }))
            .on_click(cx.listener(move |this, _, _, cx| {
                if this.phase != Phase::Choosing {
                    return;
                }
                this.selected = i;
                let pick = !this.picked[i];
                this.set_pick(i, pick, cx);
            }));
        // Rows land one after the other when the popover opens.
        row.with_animation(
            ("catch-up-row-in", i),
            Animation::new(Duration::from_millis(420 + 40 * i.min(10) as u64)),
            move |el, t| {
                let k = stagger(t, i);
                el.opacity(k).ml(px(10. * (1. - k)))
            },
        )
        .into_any_element()
    }

    fn render_header(&self) -> AnyElement {
        let (icon, color, title, detail): (Lucide, Hsla, String, String) = match self.phase {
            Phase::Choosing => (
                Lucide::GitPullRequestArrow,
                theme::accent(),
                tr("Rebaser sur leur cible", "Rebase onto their target").into(),
                trf!(
                    "{} branche(s) sans conflit avec les nouveaux changements",
                    "{} branch(es) free of conflicts with the new changes",
                    self.items.len()
                ),
            ),
            Phase::Running => (
                Lucide::RefreshCw,
                theme::accent(),
                tr("Rebase en cours…", "Rebasing…").into(),
                trf!(
                    "{}/{} branche(s)",
                    "{}/{} branch(es)",
                    self.states
                        .iter()
                        .filter(|s| matches!(s, RowState::Done { .. } | RowState::Current))
                        .count(),
                    self.picked_count()
                ),
            ),
            Phase::Finished => {
                let (ok, pushed, ko) = self.tally();
                if ko == 0 {
                    (
                        Lucide::Check,
                        theme::lime(),
                        tr("Tout s'est bien passé", "All went well").into(),
                        trf!(
                            "{ok} branche(s) rebasée(s), {pushed} poussée(s)",
                            "{ok} branch(es) rebased, {pushed} pushed"
                        ),
                    )
                } else if ok == 0 {
                    (
                        Lucide::X,
                        theme::red(),
                        tr("Rien n'a pu être rebasé", "Nothing could be rebased").into(),
                        tr(
                            "Les branches sont restées où elles étaient, voir pourquoi ci-dessous.",
                            "The branches stayed where they were, see why below.",
                        )
                        .into(),
                    )
                } else {
                    (
                        Lucide::TriangleAlert,
                        theme::amber(),
                        trf!("{ko} échec(s)", "{ko} failure(s)"),
                        trf!(
                            "{ok} rebasée(s), {pushed} poussée(s) · les autres n'ont pas bougé",
                            "{ok} rebased, {pushed} pushed · the others did not move"
                        ),
                    )
                }
            }
        };
        let seq = self.seq;
        let finished = self.phase == Phase::Finished;
        let failed = finished && !self.succeeded();
        let badge = div()
            .relative()
            .flex_none()
            .size(px(40.))
            .flex()
            .items_center()
            .justify_center()
            .rounded_full()
            .bg(theme::alpha(color, if finished { 0.9 } else { 0.16 }))
            .border_1()
            .border_color(theme::alpha(color, 0.5))
            .child(Icon::new(icon).size(px(20.)).text_color(if finished {
                gpui_kit::black()
            } else {
                color
            }));
        let badge: AnyElement = match self.phase {
            Phase::Finished if !failed => div()
                .relative()
                .child(badge.with_animation(
                    ("catch-up-win", seq),
                    Animation::new(Duration::from_millis(900)),
                    move |el, t| {
                        let k = back((t * 1.6).min(1.));
                        let ring = (t * 1.2).min(1.);
                        el.opacity(k.min(1.))
                            .mt(px(6. * (1. - k)))
                            .shadow(vec![BoxShadow {
                                color: theme::alpha(color, 0.55 * (1. - ring)),
                                offset: point(px(0.), px(0.)),
                                blur_radius: px(4. + 18. * ring),
                                spread_radius: px(14. * ring),
                                inset: false,
                            }])
                    },
                ))
                .child(super::particles::burst(seq))
                .into_any_element(),
            Phase::Finished => badge
                .with_animation(
                    ("catch-up-fail", seq),
                    Animation::new(Duration::from_millis(600)),
                    |el, t| {
                        let shake = (t * std::f32::consts::TAU * 3.).sin() * 6. * (1. - t);
                        el.ml(px(shake))
                    },
                )
                .into_any_element(),
            Phase::Running => badge
                .with_animation(
                    ("catch-up-run", seq / 1024),
                    Animation::new(Duration::from_millis(1400)).repeat(),
                    move |el, t| {
                        let glow = 0.5 - 0.5 * (t * std::f32::consts::TAU).cos();
                        el.shadow(vec![BoxShadow {
                            color: theme::alpha(color, 0.45 * glow),
                            offset: point(px(0.), px(0.)),
                            blur_radius: px(14.),
                            spread_radius: px(3. * glow),
                            inset: false,
                        }])
                    },
                )
                .into_any_element(),
            Phase::Choosing => badge.into_any_element(),
        };
        h_flex()
            .gap_3()
            .child(badge)
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .text_base()
                            .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                            .truncate()
                            .child(title),
                    )
                    .child(div().text_xs().text_color(theme::muted()).child(detail)),
            )
            .with_animation(
                ("catch-up-head", self.phase as usize),
                Animation::new(Duration::from_millis(320)).with_easing(gpui_kit::ease_out_quint()),
                |el, t| el.opacity(t),
            )
            .into_any_element()
    }

    fn render_progress(&self) -> Option<AnyElement> {
        if self.phase != Phase::Running {
            return None;
        }
        let total = self.picked_count().max(1) as f32;
        let done = self
            .states
            .iter()
            .filter(|s| {
                matches!(
                    s,
                    RowState::Done { .. } | RowState::Current | RowState::Failed(_)
                )
            })
            .count() as f32;
        let pushing = self.states.contains(&RowState::Pushing);
        let fraction = if pushing {
            0.9
        } else {
            (done + 0.5) / (total + 1.)
        };
        Some(
            div()
                .w_full()
                .h(px(4.))
                .rounded_full()
                .bg(theme::alpha(gpui_kit::white(), 0.06))
                .overflow_hidden()
                .relative()
                .child(
                    div()
                        .h_full()
                        .w(relative(fraction))
                        .rounded_full()
                        .bg(linear_gradient(
                            90.,
                            linear_color_stop(theme::alpha(theme::accent(), 0.4), 0.),
                            linear_color_stop(theme::accent(), 1.),
                        )),
                )
                .child(
                    div()
                        .absolute()
                        .top_0()
                        .h_full()
                        .w(relative(0.25))
                        .bg(linear_gradient(
                            90.,
                            linear_color_stop(theme::alpha(gpui_kit::white(), 0.), 0.),
                            linear_color_stop(theme::alpha(gpui_kit::white(), 0.35), 1.),
                        ))
                        .with_animation(
                            "catch-up-sweep",
                            Animation::new(Duration::from_millis(1100))
                                .repeat()
                                .with_easing(gpui_kit::ease_in_out),
                            |el, t| el.left(relative(-0.25 + 1.25 * t)),
                        ),
                )
                .into_any_element(),
        )
    }

    fn render_blocked(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.blocked.is_empty() {
            return None;
        }
        let open = self.show_blocked;
        let n = self.blocked.len();
        Some(
            v_flex()
                .gap_1()
                .child(
                    h_flex()
                        .id("catch-up-blocked")
                        .test_support()
                        .gap_1p5()
                        .cursor_pointer()
                        .text_xs()
                        .text_color(theme::muted())
                        .hover(|s| s.text_color(gpui_kit::white()))
                        .child(
                            Icon::new(if open {
                                Lucide::ChevronDown
                            } else {
                                Lucide::ChevronRight
                            })
                            .xsmall(),
                        )
                        .child(trf!(
                            "{n} autre(s) en retard, pas rebasable(s) sans aide",
                            "{n} other(s) behind, not rebasable without help"
                        ))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.show_blocked = !this.show_blocked;
                            cx.notify();
                        })),
                )
                .when(open, |s| {
                    s.children(self.blocked.iter().enumerate().map(|(i, c)| {
                        let color = match c.verdict {
                            Verdict::Conflict { .. } => theme::red(),
                            _ => theme::amber(),
                        };
                        h_flex()
                            .gap_2()
                            .pl_1()
                            .text_xs()
                            .child(
                                Icon::new(match c.verdict {
                                    Verdict::Conflict { .. } => Lucide::TriangleAlert,
                                    _ => Lucide::Ban,
                                })
                                .xsmall()
                                .text_color(color),
                            )
                            .child(
                                div()
                                    .flex_none()
                                    .max_w(px(140.))
                                    .truncate()
                                    .text_color(theme::alpha(gpui_kit::white(), 0.8))
                                    .child(c.branch.clone()),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_color(theme::muted())
                                    .child(blocked_reason(&c.verdict)),
                            )
                            .with_animation(
                                ("catch-up-blocked-in", i),
                                Animation::new(Duration::from_millis(260 + 30 * i as u64)),
                                move |el, t| {
                                    let k = stagger(t, i);
                                    el.opacity(k).mt(px(-4. * (1. - k)))
                                },
                            )
                    }))
                })
                .into_any_element(),
        )
    }

    fn render_footer(&self, cx: &mut Context<Self>) -> AnyElement {
        match self.phase {
            Phase::Choosing => {
                let n = self.picked_count();
                v_flex()
                    .gap_2()
                    .child(
                        h_flex()
                            .flex_wrap()
                            .gap_x_3()
                            .gap_y_1()
                            .text_xs()
                            .child(hint(&["↑", "↓"], tr("naviguer", "move")))
                            .child(hint(&["P"], tr("garder", "pick")))
                            .child(hint(&["S"], tr("passer", "skip")))
                            .child(hint(
                                &[
                                    if cfg!(target_os = "macos") {
                                        "⌘"
                                    } else {
                                        "Ctrl"
                                    },
                                    "↵",
                                ],
                                tr("rebaser", "rebase"),
                            )),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .justify_end()
                            .child(
                                Button::new("catch-up-cancel")
                                    .ghost()
                                    .small()
                                    .label(tr("Annuler", "Cancel"))
                                    .on_click(cx.listener(|this, _, _, cx| this.dismiss(cx))),
                            )
                            .child(
                                Button::new("catch-up-ok")
                                    .primary()
                                    .small()
                                    .icon(Icon::new(Lucide::GitPullRequestArrow))
                                    .label(trf!("Rebaser {n}", "Rebase {n}"))
                                    .disabled(n == 0)
                                    .on_click(cx.listener(|this, _, _, cx| this.submit(cx))),
                            ),
                    )
                    .into_any_element()
            }
            Phase::Running => div()
                .text_xs()
                .text_color(theme::muted())
                .child(tr(
                    "Rebase hors checkout, puis push avec lease : un push refusé remet la \
                     branche en place.",
                    "Rebased without checkout, then pushed with lease: a refused push puts \
                     the branch back.",
                ))
                .into_any_element(),
            Phase::Finished => h_flex()
                .justify_end()
                .child(
                    Button::new("catch-up-close")
                        .small()
                        .map(|b| {
                            if self.succeeded() {
                                b.ghost()
                            } else {
                                b.primary()
                            }
                        })
                        .label(tr("Fermer", "Close"))
                        .on_click(cx.listener(|this, _, _, cx| this.dismiss(cx))),
                )
                .into_any_element(),
        }
    }
}

impl Render for CatchUpPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rows: Vec<AnyElement> = (0..self.items.len())
            .map(|i| self.render_row(i, cx))
            .collect();
        let accent = match self.phase {
            Phase::Finished if self.succeeded() => theme::lime(),
            Phase::Finished => theme::red(),
            _ => theme::accent(),
        };
        v_flex()
            .id("catch-up")
            .test_support()
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &Prev, _, cx| this.step(-1, cx)))
            .on_action(cx.listener(|this, _: &Next, _, cx| this.step(1, cx)))
            .on_action(cx.listener(|this, _: &Pick, _, cx| this.pick(true, cx)))
            .on_action(cx.listener(|this, _: &Skip, _, cx| this.pick(false, cx)))
            .on_action(cx.listener(|this, _: &Toggle, _, cx| {
                let pick = !this.picked.get(this.selected).copied().unwrap_or(true);
                this.set_pick(this.selected, pick, cx)
            }))
            .on_action(cx.listener(|this, _: &Submit, _, cx| this.submit(cx)))
            .on_action(cx.listener(|this, _: &Dismiss, _, cx| this.dismiss(cx)))
            .w(px(440.))
            .p_3()
            .gap_3()
            .rounded_xl()
            .border_1()
            .border_color(theme::alpha(accent, 0.45))
            .bg(theme::popover())
            .shadow_xl()
            .occlude()
            .on_mouse_down_out(cx.listener(|this, _, _, cx| this.dismiss(cx)))
            .child(self.render_header())
            .children(self.render_progress())
            .child(
                v_flex()
                    .id("catch-up-rows")
                    .gap_1()
                    .max_h(px(360.))
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .children(rows),
            )
            .children(self.render_blocked(cx))
            .child(self.render_footer(cx))
            .with_animation(
                "catch-up-in",
                Animation::new(Duration::from_millis(260)).with_easing(gpui_kit::ease_out_quint()),
                |el, t| el.opacity(t).mt(px(8. * (1. - t))),
            )
    }
}

// ---- Repository view --------------------------------------------------------

/// The open popover, where it was opened, and its events.
pub(super) struct Open {
    panel: gpui_kit::Entity<CatchUpPanel>,
    at: Point<Pixels>,
    _events: Subscription,
}

impl RepoView {
    /// What to catch up: every local branch onto the base of its open PR,
    /// else onto the trunk. Branches serving as a base without a PR of their
    /// own (develop, release…) and the trunk itself are left alone.
    fn catch_up_requests(&self) -> Vec<CatchUpRequest> {
        let refs = &self.data.refs;
        let Some((trunk, _)) = super::compact::trunk(refs).filter(|(t, _)| t != "HEAD") else {
            return Vec::new();
        };
        let remote = super::sidebar::default_remote(refs);
        let trunk_name = remote
            .as_deref()
            .and_then(|r| trunk.strip_prefix(&format!("{r}/")))
            .unwrap_or(&trunk)
            .to_string();
        let open: Vec<&mugetsu_forge::PullRequest> = self
            .pr_list
            .iter()
            .filter(|p| p.same_repo && matches!(p.state, PrState::Open | PrState::Draft))
            .collect();
        let pr_of = |b: &str| open.iter().find(|p| p.head == b);
        let has_local = |b: &str| refs.branches.iter().any(|x| x.name == b);
        let has_remote = |r: &str| refs.remote_branches.iter().any(|x| x.name == r);
        refs.branches
            .iter()
            .filter(|b| !b.upstream_gone && b.name != trunk_name && b.name != trunk)
            .filter_map(|b| {
                let target = match pr_of(&b.name) {
                    Some(pr)
                        if pr.base != trunk_name
                            && has_local(&pr.base)
                            && pr_of(&pr.base).is_some() =>
                    {
                        pr.base.clone()
                    }
                    Some(pr) => {
                        let r = format!("{}/{}", remote.as_deref()?, pr.base);
                        has_remote(&r).then_some(r)?
                    }
                    None if open.iter().any(|p| p.base == b.name) => return None,
                    None => trunk.clone(),
                };
                (b.upstream.as_deref() != Some(target.as_str())).then(|| CatchUpRequest {
                    branch: b.name.clone(),
                    target,
                })
            })
            .collect()
    }

    /// Replays the lagging branches in the background when the refs or the
    /// PRs changed (so after every fetch).
    pub(super) fn scan_catch_up(&mut self, cx: &mut Context<Self>) {
        let requests = self.catch_up_requests();
        let mut h = DefaultHasher::new();
        self.data.refs.fingerprint.hash(&mut h);
        for r in &requests {
            (&r.branch, &r.target).hash(&mut h);
        }
        let key = h.finish();
        if self.catch_up_key == Some(key) {
            return;
        }
        self.catch_up_key = Some(key);
        if requests.is_empty() {
            self.catch_up = Arc::default();
            self._catch_up_scan = gpui_kit::Task::ready(());
            return;
        }
        let repo = self.repo.clone();
        self._catch_up_scan = cx.spawn(async move |this, cx| {
            let scan = cx
                .background_spawn(async move { repo.catch_up_scan(&requests) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.catch_up = Arc::new(scan.unwrap_or_default());
                cx.notify();
            });
        });
    }

    /// Branches the badge offers.
    pub fn catch_up_ready(&self) -> usize {
        self.catch_up
            .iter()
            .filter(|c| c.verdict.is_clean())
            .count()
    }

    /// Badge of the Branches section: how many branches catch up cleanly.
    pub(super) fn catch_up_badge(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let n = self.catch_up_ready();
        if n == 0 {
            return None;
        }
        let color = theme::accent();
        Some(
            h_flex()
                .id("catch-up-badge")
                .test_support()
                .flex_none()
                .gap_1()
                .px_1p5()
                .h(px(18.))
                .rounded_full()
                .text_xs()
                .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                .bg(theme::alpha(color, 0.18))
                .border_1()
                .border_color(theme::alpha(color, 0.5))
                .text_color(color)
                .hover(|s| s.bg(theme::alpha(color, 0.3)))
                .child(Icon::new(Lucide::GitPullRequestArrow).xsmall())
                .child(n.to_string())
                .tooltip(move |window, cx| {
                    Tooltip::new(trf!(
                        "{n} branche(s) rebasable(s) sur leur cible sans conflit",
                        "{n} branch(es) can be rebased onto their target without conflict"
                    ))
                    .build(window, cx)
                })
                .on_click(cx.listener(|this, _, window, cx| {
                    cx.stop_propagation();
                    this.open_catch_up(window, cx);
                }))
                .with_animation(
                    ("catch-up-badge-in", n),
                    Animation::new(Duration::from_millis(1600)),
                    move |el, t| {
                        // Arrives with a pop, then one soft pulse.
                        let k = back((t * 3.).min(1.));
                        let pulse = ((t - 0.3) / 0.7).clamp(0., 1.);
                        el.opacity(k.min(1.)).shadow(vec![BoxShadow {
                            color: theme::alpha(
                                color,
                                0.5 * (1. - pulse) * (pulse > 0.) as u8 as f32,
                            ),
                            offset: point(px(0.), px(0.)),
                            blur_radius: px(10. * pulse),
                            spread_radius: px(6. * pulse),
                            inset: false,
                        }])
                    },
                )
                .into_any_element(),
        )
    }

    pub fn open_catch_up(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy.is_some() || self.catch_up_ready() == 0 {
            return;
        }
        let scan = self.catch_up.clone();
        let panel = cx.new(|cx| CatchUpPanel::new(&scan, cx));
        let events = cx.subscribe_in(
            &panel,
            window,
            |this, _, event: &CatchUpEvent, window, cx| match event {
                CatchUpEvent::Submit(picks) => this.run_catch_up(picks.clone(), window, cx),
                CatchUpEvent::Close => this.close_catch_up(window, cx),
            },
        );
        panel.update(cx, |p, cx| p.focus.focus(window, cx));
        self.catch_up_open = Some(Open {
            panel,
            at: window.mouse_position(),
            _events: events,
        });
        cx.notify();
    }

    fn close_catch_up(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.catch_up_open.take().is_some() {
            self.root_focus.focus(window, cx);
            cx.notify();
        }
    }

    fn with_catch_up(
        &self,
        cx: &mut Context<Self>,
        f: impl FnOnce(&mut CatchUpPanel, &mut Context<CatchUpPanel>),
    ) {
        if let Some(open) = &self.catch_up_open {
            open.panel.update(cx, f);
        }
    }

    /// Rebases the picked branches one by one (the popover follows), then
    /// force-pushes those that were pushed.
    fn run_catch_up(&mut self, picks: Vec<CatchUp>, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy.is_some() {
            return;
        }
        self.busy = Some("Rebase");
        self.with_catch_up(cx, |p, cx| p.start(cx));
        let repo = self.repo.clone();
        let set = |this: &mut Self, branch: &str, state: RowState, cx: &mut Context<Self>| {
            this.with_catch_up(cx, |p, cx| p.set_state(branch, state, cx));
        };
        self._op = cx.spawn_in(window, async move |this, cx| {
            let mut moved: Vec<Moved> = Vec::new();
            let mut failed: Vec<String> = Vec::new();
            for item in &picks {
                let _ = this.update(cx, |this, cx| {
                    set(this, &item.branch, RowState::Rebasing, cx)
                });
                let state = if failed.contains(&item.target) {
                    RowState::Failed(trf!(
                        "Pas rebasée : {} a échoué juste avant",
                        "Not rebased: {} failed just before",
                        item.target
                    ))
                } else {
                    let (r, it, mv) = (repo.clone(), item.clone(), moved.clone());
                    match cx
                        .background_spawn(async move { r.catch_up_one(&it, &mv) })
                        .await
                    {
                        Ok(Some(m)) => {
                            let pushed = m.upstream.is_some();
                            moved.push(m);
                            if pushed {
                                RowState::Pushing
                            } else {
                                RowState::Done { pushed: false }
                            }
                        }
                        Ok(None) => RowState::Current,
                        Err(e) => RowState::Failed(format!("{e:#}")),
                    }
                };
                if matches!(state, RowState::Failed(_)) {
                    failed.push(item.branch.clone());
                }
                let _ = this.update(cx, |this, cx| set(this, &item.branch, state, cx));
            }
            let r = repo.clone();
            let mv = moved.clone();
            let pushes = cx
                .background_spawn(async move { r.catch_up_push(&mv) })
                .await;
            let _ = this.update(cx, |this, cx| {
                for (branch, result) in pushes {
                    let state = match result {
                        Ok(()) => RowState::Done { pushed: true },
                        Err(e) => RowState::Failed(push_failure(&format!("{e:#}"))),
                    };
                    set(this, &branch, state, cx);
                }
                this.busy = None;
                this.with_catch_up(cx, |p, cx| p.finish(cx));
                this.reload(cx);
                // PR heads moved: refresh their state.
                this.prs = Default::default();
                this.refresh_prs(true, cx);
            });
            // A clean run closes by itself once its celebration played.
            cx.background_executor()
                .timer(Duration::from_millis(2600))
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                let done = this
                    .catch_up_open
                    .as_ref()
                    .is_some_and(|o| o.panel.read(cx).succeeded());
                if done {
                    this.close_catch_up(window, cx);
                }
            });
        });
        cx.notify();
    }

    /// The popover, below-right of where the badge was clicked.
    pub(super) fn render_catch_up(&self) -> Option<AnyElement> {
        let open = self.catch_up_open.as_ref()?;
        Some(
            deferred(
                anchored()
                    .position(open.at + point(px(-12.), px(14.)))
                    .snap_to_window_with_margin(px(8.))
                    .child(open.panel.clone()),
            )
            .with_priority(1)
            .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(branch: &str, target: &str) -> CatchUp {
        CatchUp {
            branch: branch.into(),
            target: target.into(),
            verdict: Verdict::Clean,
            behind: 1,
            commits: 1,
            upstream: None,
            tip: gix::ObjectId::null(gix::hash::Kind::Sha1),
            worktree: None,
        }
    }

    #[gpui_kit::test]
    fn a_stack_is_picked_and_skipped_together(cx: &mut gpui_kit::TestAppContext) {
        let scan = vec![
            item("a", "origin/main"),
            item("b", "a"),
            item("c", "b"),
            item("x", "origin/main"),
        ];
        let panel = cx.new(|cx| CatchUpPanel::new(&scan, cx));
        panel.update(cx, |p, cx| {
            p.set_pick(1, false, cx);
            assert_eq!(
                p.picked,
                [true, false, false, true],
                "b and what rests on it"
            );
            p.set_pick(2, true, cx);
            assert_eq!(p.picked, [true, true, true, true], "c and what it rests on");
            p.set_pick(0, false, cx);
            assert_eq!(p.picked, [false, false, false, true]);
        });
    }

    #[gpui_kit::test]
    fn picking_moves_to_the_next_row(cx: &mut gpui_kit::TestAppContext) {
        let scan = vec![item("a", "origin/main"), item("x", "origin/main")];
        let panel = cx.new(|cx| CatchUpPanel::new(&scan, cx));
        panel.update(cx, |p, cx| {
            p.pick(false, cx);
            assert_eq!((p.picked.clone(), p.selected), (vec![false, true], 1));
            p.step(1, cx);
            assert_eq!(p.selected, 0, "wraps around");
        });
    }
}
