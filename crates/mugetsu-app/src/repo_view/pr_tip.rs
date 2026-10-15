//! Hover card of a PR, shown almost at once: a short shimmering skeleton,
//! then the card is revealed by a fiery liquid front sweeping left to right
//! and shedding sparks.

use std::time::Duration;

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::{
    Animation, AnimationExt as _, AnyElement, AnyView, App, IntoElement, ParentElement as _,
    Styled as _, Window, div, linear_color_stop, linear_gradient, px, relative,
};
use mugetsu_forge::PullRequest;

use super::toolbar::pr_card;
use crate::theme;

/// Hover time before the card (in its loading state) appears.
pub const DELAY: Duration = Duration::from_millis(70);

/// Whole entrance: skeleton, then the sweep, then the last sparks fade.
const TOTAL_MS: f32 = 480.;
/// The sweep itself: ~100 ms.
const SWEEP: (f32, f32) = (0.5, 0.71);
const SPARKS: usize = 22;

/// The tooltip view for `pr`.
pub fn view(pr: &PullRequest, window: &mut Window, cx: &mut App) -> AnyView {
    let pr = pr.clone();
    Tooltip::element(move |_, cx| card(&pr, cx)).build(window, cx)
}

/// 0..1 progress of the front across the card at animation time `t`.
fn front(t: f32) -> f32 {
    let k = ((t - SWEEP.0) / (SWEEP.1 - SWEEP.0)).clamp(0., 1.);
    // Fast start, liquid settle.
    1. - (1. - k).powi(2)
}

fn card(pr: &PullRequest, cx: &mut App) -> AnyElement {
    let cover_bg = cx.theme().tokens.popover;
    let content = pr_card(pr, cx);
    let bar = |w: f32, h: f32| {
        div()
            .w(relative(w))
            .h(px(h))
            .rounded_md()
            .bg(theme::alpha(gpui_kit::white(), 0.08))
    };
    let skeleton = v_flex()
        .absolute()
        .inset_0()
        .py_1()
        .gap_2()
        .child(
            h_flex()
                .gap_2()
                .child(bar(0.22, 16.))
                .child(bar(0.1, 16.))
                .child(div().flex_1())
                .child(
                    div()
                        .size(px(16.))
                        .rounded_full()
                        .bg(theme::alpha(gpui_kit::white(), 0.08)),
                ),
        )
        .child(bar(0.85, 14.))
        .child(bar(0.55, 12.))
        .child(bar(0.7, 12.));
    let cover = div()
        .absolute()
        .top_0()
        .bottom_0()
        .right_0()
        .overflow_hidden()
        .bg(cover_bg)
        .child(skeleton.with_animation(
            "pr-tip-skeleton",
            Animation::new(Duration::from_millis(700)).repeat(),
            |el, t| el.opacity(0.55 + 0.45 * (t * std::f32::consts::TAU).cos().abs()),
        ));
    let fire = div()
        .absolute()
        .top_0()
        .bottom_0()
        .w(px(46.))
        .bg(linear_gradient(
            90.,
            linear_color_stop(theme::alpha(theme::amber(), 0.), 0.),
            linear_color_stop(theme::alpha(theme::accent(), 0.85), 1.),
        ))
        .rounded_r_lg();
    let sparks = (0..SPARKS).map(|i| {
        // Born along the sweep, each drifts right and up and burns out.
        let born = SWEEP.0 + (SWEEP.1 - SWEEP.0) * i as f32 / SPARKS as f32;
        let row = ((i * 7) % SPARKS) as f32 / SPARKS as f32;
        let size = 2. + (i % 4) as f32 * 0.8;
        let drift = 18. + (i % 4) as f32 * 9.;
        let color = if i % 3 == 0 {
            theme::amber()
        } else {
            theme::accent()
        };
        div()
            .absolute()
            .size(px(size))
            .rounded_full()
            .bg(color)
            .with_animation(
                ("pr-tip-spark", i),
                Animation::new(Duration::from_millis(TOTAL_MS as u64)),
                move |el, t| {
                    let age = ((t - born) / 0.35).clamp(0., 1.);
                    let alive = t >= born && age < 1.;
                    el.left(relative(front(born + 0.02).min(0.98)))
                        .ml(px(drift * age))
                        .top(relative(row))
                        .mt(px(-10. * age))
                        .opacity(if alive { (1. - age) * 0.95 } else { 0. })
                },
            )
    });
    div()
        .relative()
        .overflow_hidden()
        .child(content)
        .child(cover.with_animation(
            "pr-tip-cover",
            Animation::new(Duration::from_millis(TOTAL_MS as u64)),
            |el, t| {
                let k = front(t);
                el.left(relative(k)).opacity(if k >= 1. { 0. } else { 1. })
            },
        ))
        .child(fire.with_animation(
            "pr-tip-fire",
            Animation::new(Duration::from_millis(TOTAL_MS as u64)),
            |el, t| {
                let k = front(t);
                let on = t >= SWEEP.0 && k < 1.;
                el.left(relative(k))
                    .ml(px(-46.))
                    .opacity(if on { 1. } else { 0. })
            },
        ))
        .children(sparks)
        .into_any_element()
}
