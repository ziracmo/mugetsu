//! Drag a branch chip onto another to rebase it there, after a small
//! confirmation popover (which can be told not to ask again).

use std::time::Duration;

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    Animation, AnimationExt as _, AnyElement, Context, Hsla, InteractiveElement as _, IntoElement,
    ParentElement as _, Pixels, Point, Render, StatefulInteractiveElement as _, Styled as _,
    Window, anchored, deferred, div, point, prelude::FluentBuilder as _, px,
};

use super::RepoView;
use crate::i18n::tr;
use crate::theme;

/// A local branch chip being dragged.
#[derive(Clone)]
pub(super) struct BranchDrag {
    pub name: String,
    pub color: Hsla,
    pub head: bool,
    /// Where the chip was grabbed, from its top-left corner.
    pub offset: Point<Pixels>,
}

impl Render for BranchDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let chip = h_flex()
            .h(px(20.))
            .px_1p5()
            .gap_1()
            .rounded_md()
            .border_1()
            .text_xs()
            .shadow_lg()
            .map(|s| {
                if self.head {
                    s.bg(theme::accent())
                        .border_color(theme::alpha(gpui_kit::white(), 0.55))
                        .text_color(gpui_kit::black())
                        .font_weight(gpui_kit::FontWeight::BOLD)
                } else {
                    s.bg(theme::popover())
                        .border_color(theme::alpha(self.color, 0.8))
                        .text_color(self.color)
                }
            })
            .child(Icon::new(Lucide::GitBranch).xsmall())
            .child(self.name.clone());
        // Carried below-right of the pointer, so the chip or row it is over
        // stays visible. Picked up, it struggles a little: a quick jitter.
        div()
            .pl(self.offset.x + px(16.))
            .pt(self.offset.y + px(14.))
            .child(chip.with_animation(
                "branch-drag-shake",
                Animation::new(Duration::from_millis(520)).repeat(),
                |el, t| {
                    let a = t * std::f32::consts::TAU;
                    let x = 2.6 * (3. * a).sin() * (0.6 + 0.4 * a.cos().abs());
                    let y = 1.1 * (2. * a + 0.7).sin();
                    el.ml(px(x)).mt(px(y))
                },
            ))
    }
}

/// Rebase waiting for the popover's answer.
pub(super) struct PendingRebase {
    branch: String,
    onto: String,
    at: Point<Pixels>,
    dont_ask: bool,
}

impl RepoView {
    /// `branch` was dropped on the chip of `onto`.
    pub(super) fn drop_branch(
        &mut self,
        branch: String,
        onto: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if branch == onto {
            return;
        }
        if crate::settings::get().confirm_drag_rebase {
            self.pending_rebase = Some(PendingRebase {
                branch,
                onto,
                at: window.mouse_position(),
                dont_ask: false,
            });
            cx.notify();
        } else {
            self.rebase_branch_onto(branch, onto, window, cx);
        }
    }

    fn rebase_branch_onto(
        &mut self,
        branch: String,
        onto: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.run_op_then(
            "Rebase",
            move |r| {
                r.rebase_branch(&branch, &onto)
                    .and_then(super::conflicts::stopped_on_conflicts)
            },
            |this, _, _| this.rebased(),
            window,
            cx,
        );
    }

    fn answer_rebase(&mut self, go: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(p) = self.pending_rebase.take() else {
            return;
        };
        cx.notify();
        if !go {
            return;
        }
        if p.dont_ask {
            crate::settings::update(cx, |s| s.confirm_drag_rebase = false);
        }
        self.rebase_branch_onto(p.branch, p.onto, window, cx);
    }

    /// The confirmation popover, next to where the chip was dropped.
    pub(super) fn render_rebase_popover(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let p = self.pending_rebase.as_ref()?;
        let name = |s: &str| {
            div()
                .px_1()
                .rounded_sm()
                .bg(theme::alpha(theme::accent(), 0.16))
                .text_color(gpui_kit::white())
                .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                .child(s.to_string())
        };
        let checked = p.dont_ask;
        let card = v_flex()
            .id("drag-rebase")
            .test_support()
            .w(px(320.))
            .p_3()
            .gap_2p5()
            .rounded_lg()
            .border_1()
            .border_color(theme::alpha(theme::accent(), 0.5))
            .bg(theme::popover())
            .shadow_xl()
            .text_sm()
            .occlude()
            .on_mouse_down_out(
                cx.listener(|this, _, window, cx| this.answer_rebase(false, window, cx)),
            )
            .child(
                h_flex()
                    .gap_1p5()
                    .flex_wrap()
                    .child(
                        Icon::new(Lucide::GitBranch)
                            .xsmall()
                            .text_color(theme::accent()),
                    )
                    .child(tr("Rebaser", "Rebase"))
                    .child(name(&p.branch))
                    .child(tr("sur", "onto"))
                    .child(name(&p.onto))
                    .child("?"),
            )
            .child(div().text_xs().text_color(theme::muted()).child(trf!(
                "Les commits de '{}' sont rejoués au-dessus de '{}'.",
                "The commits of '{}' are replayed on top of '{}'.",
                p.branch,
                p.onto
            )))
            .child(
                h_flex()
                    .id("drag-rebase-dont-ask")
                    .test_support()
                    .gap_2()
                    .cursor_pointer()
                    .text_xs()
                    .child(
                        div()
                            .size(px(14.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(4.))
                            .border_1()
                            .map(|s| {
                                if checked {
                                    s.bg(theme::accent()).border_color(theme::accent()).child(
                                        Icon::new(Lucide::Check)
                                            .xsmall()
                                            .text_color(gpui_kit::white()),
                                    )
                                } else {
                                    s.border_color(theme::alpha(gpui_kit::white(), 0.25))
                                        .bg(theme::alpha(gpui_kit::white(), 0.04))
                                }
                            }),
                    )
                    .child(tr("Ne plus demander", "Don't ask again"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        if let Some(p) = this.pending_rebase.as_mut() {
                            p.dont_ask = !p.dont_ask;
                            cx.notify();
                        }
                    })),
            )
            .child(
                h_flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("drag-rebase-cancel")
                            .ghost()
                            .small()
                            .label(tr("Annuler", "Cancel"))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.answer_rebase(false, window, cx)
                            })),
                    )
                    .child(
                        Button::new("drag-rebase-ok")
                            .primary()
                            .small()
                            .label(tr("Rebaser", "Rebase"))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.answer_rebase(true, window, cx)
                            })),
                    ),
            )
            .with_animation(
                (
                    "drag-rebase-in",
                    p.at.x.to_f64() as usize ^ p.at.y.to_f64() as usize,
                ),
                Animation::new(Duration::from_millis(160)).with_easing(gpui_kit::ease_out_quint()),
                |el, t| el.opacity(t).mt(px(6. * (1. - t))),
            );
        Some(
            deferred(
                anchored()
                    .position(p.at + point(px(8.), px(8.)))
                    .snap_to_window_with_margin(px(8.))
                    .child(card),
            )
            .with_priority(1)
            .into_any_element(),
        )
    }
}
